//! Driving the compiled lookups over a buffer.
//!
//! Two levels, and both are format-agnostic: what makes this file short is
//! that every decision about *which* format runs was already made when the
//! lookup was compiled.
//!
//! * [`apply_at`] tries the subtables that can start on the glyph under the
//!   cursor, in the order they must be tried.
//! * [`apply_forward`] walks the buffer.
//!
//! `apply_forward` deliberately has the shape of this crate's own
//! `ot_layout::apply_forward` -- scan for a candidate, try to apply, advance if
//! it did not. That is so the two can be compared position for position while
//! the formats are filled in. It is not where the compiled form pays off: the
//! shaper this came from replaces the scan with a bitmap intersection built
//! from the lookup's compiled reach, its pair key and the buffer's transposed
//! feature masks, which is a later piece of work and a bigger one.

use super::lookup::{Apply, CompiledLookup};
use crate::hb::ot_layout_gsubgpos::OT::check_glyph_property;

/// Try this lookup at the cursor, reporting whether a subtable applied.
///
/// Where HarfBuzz walks every subtable and asks each to re-read its coverage
/// out of the font, this asks a compiled set -- and when the lookup has enough
/// subtables to be worth an index, only the ones that can start on this glyph
/// are considered at all.
///
/// The gate is asked for exactly what the subtable will use: ranking a bit
/// costs a load and a popcount over the test that answers membership, and
/// several formats never read the number. The result is handed to the format
/// so it does not recompute it.
pub fn apply_at(ctx: &mut Apply, lookup: &CompiledLookup) -> Option<()> {
    let glyph = ctx.glyph();
    // With an index, only the subtables that can start on this glyph, in the
    // order they must be. Without one, all of them.
    let row = lookup.dispatch.as_ref().map(|d| d.row(glyph));
    let count = row.map_or(lookup.subtables.len(), <[u16]>::len);
    for k in 0..count {
        let at = match row {
            Some(r) => r[k] as usize,
            None => k,
        };
        let Some(sub) = lookup.subtables.get(at) else {
            continue;
        };
        let Some(index) = sub.gate(glyph) else {
            continue;
        };
        // An indirect call, not a match. The format was settled when the
        // subtable was compiled.
        if (sub.apply)(ctx, lookup, sub, index).is_some() {
            return Some(());
        }
    }
    None
}

/// One forward pass over the buffer.
///
/// Reports whether anything applied. The caller owns the output buffer
/// discipline -- `clear_output` before, `sync` after, for a table that is not
/// applied in place.
pub fn apply_forward(ctx: &mut Apply, lookup: &CompiledLookup) -> bool {
    let mut applied = false;
    while ctx.host.buffer.successful {
        // Scan to the next position this lookup could touch. Three tests, and
        // the first is the compiled reach rather than a parse of the font.
        let idx = ctx.host.buffer.idx;
        let mut j = idx;
        while j < ctx.host.buffer.len {
            let info = &ctx.host.buffer.info[j];
            if lookup.reach.contains(info.glyph_id)
                && (info.mask & ctx.host.lookup_mask()) != 0
                && check_glyph_property(ctx.host.face, info, ctx.host.lookup_props)
            {
                break;
            }
            j += 1;
        }
        if j > idx {
            ctx.host.buffer.next_glyphs(j - idx);
        }
        if ctx.host.buffer.idx >= ctx.host.buffer.len {
            break;
        }

        // A format that applied has already advanced the cursor past what it
        // consumed; one that did not leaves it to us.
        if apply_at(ctx, lookup).is_some() {
            applied = true;
        } else {
            ctx.host.buffer.next_glyph();
        }
    }
    applied
}

/// One descending pass over the buffer, for a reverse lookup.
///
/// Reverse chaining substitution is the only thing that runs this way, and the
/// direction is the format rather than an optimisation: a fraction font
/// substitutes a digit for its numerator form when what follows is the
/// fraction slash *or another numerator*, so each position's rule can only
/// match because the one to its right was already decided. Running forwards,
/// the chain could not propagate.
///
/// Simpler than the forward pass, and every difference follows from that. No
/// output buffer, because the substitution is in place. No cursor advance,
/// because a one-glyph input consumes nothing a later position wanted -- the
/// format leaves the cursor alone and this owns it.
pub fn apply_backward(ctx: &mut Apply, lookup: &CompiledLookup) -> bool {
    let mut applied = false;
    // Read out of the context once: these cannot change under a reverse
    // lookup, which neither recurses nor alters the buffer's length.
    let face = ctx.host.face;
    let lookup_mask = ctx.host.lookup_mask();
    let lookup_props = ctx.host.lookup_props;

    loop {
        let idx = ctx.host.buffer.idx;
        let candidate = ctx.host.buffer.info[..=idx].iter().rposition(|info| {
            lookup.reach.contains(info.glyph_id)
                && (info.mask & lookup_mask) != 0
                && check_glyph_property(face, info, lookup_props)
        });
        let Some(at) = candidate else {
            ctx.host.buffer.idx = 0;
            break;
        };

        ctx.host.buffer.idx = at;
        applied |= apply_at(ctx, lookup).is_some();

        if at == 0 {
            break;
        }
        ctx.host.buffer.idx = at - 1;
    }
    applied
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::hb::buffer::Buffer;
    use crate::hb::common::Direction;
    use crate::hb::face::Scale;
    use crate::hb::ot::compile::lookup::{Program, SubtableKind};
    use crate::hb::ot::compile::set::GlyphSet;
    use crate::hb::ot::compile::{compile_gpos_program, compile_gsub_program};
    use crate::hb::ot_layout::{apply_synthesized_subst_lookup, TableIndex};
    use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
    use crate::BufferFlags;
    use crate::{FontRef, ShaperData};
    use read_fonts::tables::gpos::{
        ExtensionSubtable as PosExtension, Gpos, PairPos, PositionLookup,
    };
    use read_fonts::tables::gsub::{
        ExtensionSubtable, Gsub, LigatureSubstFormat1, SubstitutionLookup,
    };
    use read_fonts::types::GlyphId;
    use read_fonts::TableProvider;

    /// Longest buffer a case builds, so a lookup with a huge coverage does not
    /// turn one case into a benchmark.
    const MAX_GLYPHS: usize = 48;
    /// Cap on coverage entries and ligatures taken per subtable, same reason.
    const MAX_CASES: usize = 8;

    /// One case: the font it came from and the buffer to run over.
    struct Case {
        name: String,
        data: Vec<u8>,
        index: u16,
        glyphs: Vec<u32>,
        /// Whether this lookup runs from the end of the buffer towards the
        /// start. Tracked so the descending pass can be shown to have done
        /// something rather than assumed to.
        reverse: bool,
    }

    /// What a run left behind: glyphs, clusters, and the flag masks.
    ///
    /// The last of those is the half that is invisible when it is wrong -- it
    /// shows up as a caller breaking a line where it may not, or reusing a run
    /// it should have reshaped -- so it is compared from the first format
    /// rather than added once something has already gone quiet.
    type Outcome = (Vec<u32>, Vec<u32>, Vec<u32>);

    /// Whether every subtable of this lookup is a format implemented so far.
    /// A lookup with a stub in it would "differ" for a reason that says
    /// nothing about dispatch.
    fn all_implemented(program: &Program, index: u16, table: &[u8]) -> bool {
        let Some(lookup) = program.get(index, table) else {
            return false;
        };
        !lookup.subtables.is_empty()
            && lookup.subtables.iter().all(|s| {
                matches!(
                    s.kind,
                    SubtableKind::SingleDelta { .. }
                        | SubtableKind::SingleList { .. }
                        | SubtableKind::Alternate { .. }
                        | SubtableKind::Ligature { .. }
                        | SubtableKind::Multiple { .. }
                        | SubtableKind::ReverseChain { .. }
                )
            })
    }

    /// Buffers worth running a lookup over.
    ///
    /// The first is the glyphs it can act on plus one it cannot, which catches
    /// a pass that substitutes nothing and one that substitutes
    /// indiscriminately. That is enough for a 1->1 format, but it would almost
    /// never *form* a ligature -- so a ligature subtable also contributes one
    /// buffer per ligature, built from the components the font itself names.
    /// Without those, the ligature half of this would only be checking that
    /// nothing happens.
    fn probes(gsub: &Gsub, program: &Program, index: u16, table: &[u8]) -> Vec<Vec<u32>> {
        let lookup = program.get(index, table).unwrap();
        let mut reach = lookup.reach.to_vec();
        reach.truncate(MAX_GLYPHS - 1);
        reach.push(0);
        let mut out = vec![reach];

        // A reverse chain only fires when the glyphs either side are in its
        // backtrack and lookahead coverages, so build one buffer that is: one
        // glyph from each set, the backtrack laid out in buffer order.
        for sub in &lookup.subtables {
            let SubtableKind::ReverseChain {
                backtrack,
                lookahead,
                ..
            } = &sub.kind
            else {
                continue;
            };
            let Some(&covered) = sub.cov.to_vec().first() else {
                continue;
            };
            let one = |set: &GlyphSet| set.to_vec().first().copied();
            let Some(before) = backtrack
                .iter()
                .rev()
                .map(|s| one(s))
                .collect::<Option<Vec<u32>>>()
            else {
                continue;
            };
            let Some(after) = lookahead
                .iter()
                .map(|s| one(s))
                .collect::<Option<Vec<u32>>>()
            else {
                continue;
            };
            let mut seq = before;
            seq.push(covered);
            seq.extend(after);
            out.push(seq);
        }

        for sub in ligature_subtables(gsub, index) {
            let Ok(cov) = sub.coverage() else { continue };
            let sets = sub.ligature_sets();
            for (i, first) in cov.iter().enumerate().take(MAX_CASES) {
                let Ok(set) = sets.get(i) else { continue };
                for lig in set
                    .ligatures()
                    .iter()
                    .filter_map(Result::ok)
                    .take(MAX_CASES)
                {
                    let mut seq = vec![first.to_u32()];
                    seq.extend(lig.component_glyph_ids().iter().map(|c| c.get().to_u32()));
                    // A trailing glyph, so a ligature that runs to the end of
                    // the buffer is not the only shape under test.
                    seq.push(0);
                    out.push(seq);
                }
            }
        }
        out
    }

    /// Ligature subtables of one lookup, through an extension or not.
    fn ligature_subtables<'a>(gsub: &Gsub<'a>, index: u16) -> Vec<LigatureSubstFormat1<'a>> {
        let Ok(list) = gsub.lookup_list() else {
            return Vec::new();
        };
        let Ok(lookup) = list.lookups().get(index as usize) else {
            return Vec::new();
        };
        match lookup {
            SubstitutionLookup::Ligature(l) => {
                l.subtables().iter().filter_map(Result::ok).collect()
            }
            SubstitutionLookup::Extension(l) => l
                .subtables()
                .iter()
                .filter_map(Result::ok)
                .filter_map(|e| match e {
                    ExtensionSubtable::Ligature(e) => e.extension().ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Every case in this crate's font corpus, for the formats implemented.
    fn cases() -> Vec<Case> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");
        let mut out = Vec::new();
        for entry in walk(dir) {
            let Ok(data) = std::fs::read(&entry) else {
                continue;
            };
            let Ok(font) = FontRef::new(&data) else {
                continue;
            };
            let Ok(gsub) = font.gsub() else { continue };
            let Some(table) = font
                .table_data(read_fonts::types::Tag::new(b"GSUB"))
                .map(|d| d.as_bytes().to_vec())
            else {
                continue;
            };
            let name = entry.file_name().unwrap().to_string_lossy().into_owned();
            let program = compile_gsub_program(&gsub);
            for index in 0..program.len() as u16 {
                if !all_implemented(&program, index, &table) {
                    continue;
                }
                let reverse = program.get(index, &table).is_some_and(|l| {
                    l.subtables
                        .iter()
                        .any(|s| matches!(s.kind, SubtableKind::ReverseChain { .. }))
                });
                for glyphs in probes(&gsub, &program, index, &table) {
                    if glyphs.len() < 2 {
                        continue;
                    }
                    out.push(Case {
                        name: name.clone(),
                        data: data.clone(),
                        index,
                        glyphs,
                        reverse,
                    });
                }
            }
        }
        out
    }

    /// Apply one GSUB lookup by one path or the other and report what is left.
    fn run(case: &Case, mine: bool, concat: bool) -> Option<Outcome> {
        let font = FontRef::new(&case.data).ok()?;
        let shaper_data = ShaperData::new(&font);
        let face = shaper_data.shaper(&font).build();
        let (table, info) = face
            .ot_tables
            .table_data_and_lookup(TableIndex::GSUB, case.index)?;
        if !info.is_subst {
            return None;
        }
        let reverse = info.is_reverse();

        let mut buffer = Buffer::new();
        for (i, &g) in case.glyphs.iter().enumerate() {
            buffer.push(g, i as u32);
        }
        buffer.reset_masks(1);
        if concat {
            buffer.flags |= BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        }
        // The vars GSUB writes through: ligature ids and glyph props. Shaping
        // allocates these around the substitution stage, and setting glyph
        // props below asserts they are there.
        buffer.allocate_gsubgpos_vars();
        crate::hb::ot_layout::hb_ot_layout_substitute_start(&face, &mut buffer);

        let mut ctx =
            hb_ot_apply_context_t::new(TableIndex::GSUB, &face, Scale::default(), &mut buffer);
        ctx.lookup_index = case.index;
        ctx.set_lookup_mask(1);

        ctx.lookup_props = info.props();
        ctx.update_matchers();

        match (mine, reverse) {
            (true, false) => {
                let gsub = font.gsub().ok()?;
                let program = compile_gsub_program(&gsub);
                let compiled = program.get(case.index, table)?;
                ctx.buffer.clear_output();
                ctx.buffer.idx = 0;
                let mut apply = Apply {
                    host: &mut ctx,
                    table,
                    program: &program,
                };
                apply_forward(&mut apply, compiled);
                ctx.buffer.sync();
            }
            (true, true) => {
                let gsub = font.gsub().ok()?;
                let program = compile_gsub_program(&gsub);
                let compiled = program.get(case.index, table)?;
                ctx.buffer.idx = ctx.buffer.len - 1;
                let mut apply = Apply {
                    host: &mut ctx,
                    table,
                    program: &program,
                };
                apply_backward(&mut apply, compiled);
            }
            (false, false) => apply_synthesized_subst_lookup(&mut ctx, info, table),
            // This crate has no entry point that applies one reverse lookup,
            // so the loop is written out here. It is the same loop both sides
            // run, which is the point: what this compares is the format, not
            // the pass around it.
            (false, true) => {
                ctx.buffer.idx = ctx.buffer.len - 1;
                let face = ctx.face;
                let mask = ctx.lookup_mask();
                let props = ctx.lookup_props;
                loop {
                    let idx = ctx.buffer.idx;
                    let candidate = ctx.buffer.info[..=idx].iter().rposition(|i| {
                        info.digest().may_have(i.glyph_id)
                            && (i.mask & mask) != 0
                            && check_glyph_property(face, i, props)
                    });
                    let Some(at) = candidate else {
                        ctx.buffer.idx = 0;
                        break;
                    };
                    ctx.buffer.idx = at;
                    info.apply(&mut ctx, table, false);
                    if at == 0 {
                        break;
                    }
                    ctx.buffer.idx = at - 1;
                }
            }
        }

        let n = buffer.len;
        Some((
            buffer.info[..n].iter().map(|i| i.glyph_id).collect(),
            buffer.info[..n].iter().map(|i| i.cluster).collect(),
            buffer.info[..n].iter().map(|i| i.mask).collect(),
        ))
    }

    /// The compiled path against this crate's own, on every case, with the
    /// buffer configured the way a caller gets it by default.
    #[test]
    fn substitution_agrees_with_the_shaper_it_was_lifted_into() {
        let cases = cases();
        let mut checked = 0usize;
        let mut effective = 0usize;
        let mut grew = 0usize;
        let mut shrank = 0usize;
        let mut reversed = 0usize;
        let mut failures = Vec::new();

        for case in &cases {
            let (Some(want), Some(got)) = (run(case, false, false), run(case, true, false)) else {
                continue;
            };
            checked += 1;
            match want.0.len().cmp(&case.glyphs.len()) {
                core::cmp::Ordering::Greater => grew += 1,
                core::cmp::Ordering::Less => shrank += 1,
                core::cmp::Ordering::Equal => {}
            }
            // Did the lookup do anything at all? Two paths that both leave the
            // buffer alone prove only that neither crashed.
            let changed =
                want.0 != case.glyphs || want.1.iter().enumerate().any(|(i, &c)| c != i as u32);
            if changed {
                effective += 1;
                if case.reverse {
                    reversed += 1;
                }
            }
            if got != want {
                failures.push(format!(
                    "{}: lookup {} on {:?}\n    want {want:?}\n    got  {got:?}",
                    case.name, case.index, case.glyphs
                ));
            }
        }

        assert!(checked > 0, "no lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            effective * 4 > checked,
            "only {effective} of {checked} cases changed anything; the probes \
             are not exercising these formats"
        );
        // Both directions of length change have to be reached, or a whole
        // path is untested: ligature is the only format here that shortens
        // the buffer, and multiple substitution the only one that lengthens
        // it.
        assert!(
            shrank > 0,
            "no case shortened the buffer; ligature untested"
        );
        assert!(
            grew > 0,
            "no case lengthened the buffer; the splice untested"
        );
        assert!(
            reversed > 0,
            "no reverse lookup changed anything; the descending pass is untested"
        );
        println!(
            "{checked} cases agree, {effective} of them changing glyphs or \
             clusters, {grew} lengthening the buffer, {shrank} shortening it \
             and {reversed} substituting on the descending pass"
        );
    }

    /// The same cases with the caller asking for unsafe-to-concat, where the
    /// agreement is deliberately weaker.
    ///
    /// Worth its own test because those flags are off by default -- the sweep
    /// above cannot see them at all, since `Buffer::unsafe_to_concat` returns
    /// immediately unless the buffer asked -- and worth its own assertion
    /// because the compiled path is knowingly *more precise* here. Its pair key
    /// is an exact set where this crate consults a three-word digest, so it
    /// rejects some second glyphs the digest lets through, and it is the loop
    /// past that point which records a hazard. See [`super::super::gsub`].
    ///
    /// So glyphs and clusters must still match exactly, and the flags we set
    /// must be a *subset* of theirs. A superset would mean inventing hazards;
    /// anything but a subset would mean the difference is not the one this
    /// documents.
    #[test]
    fn concat_hazards_are_a_subset_of_the_ones_this_crate_marks() {
        let cases = cases();
        let mut checked = 0usize;
        let mut fewer = 0usize;
        let mut failures = Vec::new();

        for case in &cases {
            let (Some(want), Some(got)) = (run(case, false, true), run(case, true, true)) else {
                continue;
            };
            checked += 1;
            if got.0 != want.0 || got.1 != want.1 {
                failures.push(format!(
                    "{}: lookup {} on {:?}: glyphs or clusters differ\n    \
                     want {want:?}\n    got  {got:?}",
                    case.name, case.index, case.glyphs
                ));
                continue;
            }
            if got.2 == want.2 {
                continue;
            }
            if got
                .2
                .iter()
                .zip(&want.2)
                .all(|(&ours, &theirs)| ours & !theirs == 0)
            {
                fewer += 1;
            } else {
                failures.push(format!(
                    "{}: lookup {} on {:?}: we mark a flag they do not\n    \
                     want {:?}\n    got  {:?}",
                    case.name, case.index, case.glyphs, want.2, got.2
                ));
            }
        }

        assert!(checked > 0, "no lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases disagree beyond concat hazards:\n{}",
            failures.len(),
            failures.join("\n")
        );
        println!(
            "{checked} cases agree on glyphs and clusters; {fewer} mark fewer \
             concat hazards than this crate does"
        );
    }

    // ---- positioning ----------------------------------------------------
    //
    // The same shape of differential, but the answer is positions rather than
    // glyphs, so it needs its own probes and its own comparison. Pair
    // positioning in particular will not fire on arbitrary glyphs: the probes
    // below are built from the pairs and classes the font itself names.

    /// Advance and offset per position, which is the whole of what GPOS does.
    type Positions = Vec<(i32, i32, i32, i32)>;

    /// Whether every subtable of this GPOS lookup is a format implemented so
    /// far.
    fn gpos_implemented(program: &Program, index: u16, table: &[u8]) -> bool {
        let Some(lookup) = program.get(index, table) else {
            return false;
        };
        !lookup.subtables.is_empty()
            && lookup.subtables.iter().all(|s| {
                matches!(
                    s.kind,
                    SubtableKind::SinglePos { .. }
                        | SubtableKind::PairPos1 { .. }
                        | SubtableKind::PairPos2 { .. }
                )
            })
    }

    /// Buffers worth running a positioning lookup over.
    ///
    /// Single positioning is happy with any covered glyph, but a pair format
    /// needs two glyphs that actually pair. Format 1 names its pairs outright,
    /// so those are read straight out of the font. Format 2 names classes, so
    /// glyphs are sampled one per class -- pairing a covered glyph with one
    /// representative of each second class reaches a spread of matrix cells,
    /// including the mostly-zero ones the row summaries exist to skip.
    fn gpos_probes(gpos: &Gpos, program: &Program, index: u16, table: &[u8]) -> Vec<Vec<u32>> {
        let lookup = program.get(index, table).unwrap();
        let mut reach = lookup.reach.to_vec();
        reach.truncate(MAX_GLYPHS - 1);
        reach.push(0);
        let mut out = vec![reach];

        for sub in pair_subtables(gpos, index) {
            match sub {
                PairPos::Format1(t) => {
                    let Ok(cov) = t.coverage() else { continue };
                    let sets = t.pair_sets();
                    for (i, first) in cov.iter().enumerate().take(MAX_CASES) {
                        let Ok(set) = sets.get(i) else { continue };
                        for rec in set
                            .pair_value_records()
                            .iter()
                            .filter_map(Result::ok)
                            .take(MAX_CASES)
                        {
                            out.push(vec![first.to_u32(), rec.second_glyph.get().to_u32()]);
                        }
                    }
                }
                PairPos::Format2(t) => {
                    let Ok(cov) = t.coverage() else { continue };
                    let Ok(class2) = t.class_def2() else { continue };
                    // One glyph per second class, found by probing. There is no
                    // reverse index from class to glyph in the format.
                    let mut seen = alloc::collections::BTreeMap::new();
                    for gid in 0..2048u32 {
                        let class = class2.get(GlyphId::from(gid));
                        if class < t.class2_count() {
                            seen.entry(class).or_insert(gid);
                        }
                    }
                    for first in cov.iter().take(MAX_CASES) {
                        for second in seen.values().take(MAX_CASES) {
                            out.push(vec![first.to_u32(), *second]);
                        }
                    }
                }
            }
        }
        out
    }

    /// Pair subtables of one lookup, through an extension or not.
    fn pair_subtables<'a>(gpos: &Gpos<'a>, index: u16) -> Vec<PairPos<'a>> {
        let Ok(list) = gpos.lookup_list() else {
            return Vec::new();
        };
        let Ok(lookup) = list.lookups().get(index as usize) else {
            return Vec::new();
        };
        match lookup {
            PositionLookup::Pair(l) => l.subtables().iter().filter_map(Result::ok).collect(),
            PositionLookup::Extension(l) => l
                .subtables()
                .iter()
                .filter_map(Result::ok)
                .filter_map(|e| match e {
                    PosExtension::Pair(e) => e.extension().ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn gpos_cases() -> Vec<Case> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");
        let mut out = Vec::new();
        for entry in walk(dir) {
            let Ok(data) = std::fs::read(&entry) else {
                continue;
            };
            let Ok(font) = FontRef::new(&data) else {
                continue;
            };
            let Ok(gpos) = font.gpos() else { continue };
            let Some(table) = font
                .table_data(read_fonts::types::Tag::new(b"GPOS"))
                .map(|d| d.as_bytes().to_vec())
            else {
                continue;
            };
            let name = entry.file_name().unwrap().to_string_lossy().into_owned();
            let program = compile_gpos_program(&gpos);
            for index in 0..program.len() as u16 {
                if !gpos_implemented(&program, index, &table) {
                    continue;
                }
                for glyphs in gpos_probes(&gpos, &program, index, &table) {
                    if glyphs.len() < 2 {
                        continue;
                    }
                    out.push(Case {
                        name: name.clone(),
                        data: data.clone(),
                        index,
                        glyphs,
                        reverse: false,
                    });
                }
            }
        }
        out
    }

    /// Apply one GPOS lookup by one path or the other and report the positions
    /// and flags it left.
    fn run_gpos(case: &Case, mine: bool, concat: bool) -> Option<(Positions, Vec<u32>)> {
        let font = FontRef::new(&case.data).ok()?;
        let shaper_data = ShaperData::new(&font);
        let face = shaper_data.shaper(&font).build();
        let (table, info) = face
            .ot_tables
            .table_data_and_lookup(TableIndex::GPOS, case.index)?;
        if info.is_subst || info.is_reverse() {
            return None;
        }

        let mut buffer = Buffer::new();
        for (i, &g) in case.glyphs.iter().enumerate() {
            buffer.push(g, i as u32);
        }
        buffer.reset_masks(1);
        if concat {
            buffer.flags |= BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        }
        // Positioning reads the direction -- an advance is horizontal or it is
        // not -- so pin it rather than leave it at whatever a fresh buffer has.
        buffer.direction = Direction::LeftToRight;
        buffer.allocate_gsubgpos_vars();
        crate::hb::ot_layout::hb_ot_layout_substitute_start(&face, &mut buffer);
        // Sizes `pos` and zeroes it, so every case starts from no adjustment
        // and what is compared is exactly what the lookup contributed.
        buffer.clear_positions();

        let mut ctx =
            hb_ot_apply_context_t::new(TableIndex::GPOS, &face, Scale::default(), &mut buffer);
        ctx.lookup_index = case.index;
        ctx.set_lookup_mask(1);
        ctx.lookup_props = info.props();
        ctx.update_matchers();
        ctx.buffer.idx = 0;

        if mine {
            let gpos = font.gpos().ok()?;
            let program = compile_gpos_program(&gpos);
            let compiled = program.get(case.index, table)?;
            let mut apply = Apply {
                host: &mut ctx,
                table,
                program: &program,
            };
            // GPOS applies in place, so no output buffer to clear or sync.
            apply_forward(&mut apply, compiled);
        } else {
            // This crate has no entry point that applies a single GPOS lookup,
            // so the pass is written out. It is the same pass both sides run,
            // which is the point: what this compares is the format.
            let face = ctx.face;
            let mask = ctx.lookup_mask();
            let props = ctx.lookup_props;
            while ctx.buffer.successful {
                let idx = ctx.buffer.idx;
                let mut j = idx;
                while j < ctx.buffer.len {
                    let i = &ctx.buffer.info[j];
                    if info.digest().may_have(i.glyph_id)
                        && (i.mask & mask) != 0
                        && check_glyph_property(face, i, props)
                    {
                        break;
                    }
                    j += 1;
                }
                if j > idx {
                    ctx.buffer.next_glyphs(j - idx);
                }
                if ctx.buffer.idx >= ctx.buffer.len {
                    break;
                }
                if info.apply(&mut ctx, table, false).is_none() {
                    ctx.buffer.next_glyph();
                }
            }
        }

        let n = buffer.len;
        Some((
            buffer.pos[..n]
                .iter()
                .map(|p| (p.x_advance, p.y_advance, p.x_offset, p.y_offset))
                .collect(),
            buffer.info[..n].iter().map(|i| i.mask).collect(),
        ))
    }

    /// The compiled positioning path against this crate's own.
    #[test]
    fn positioning_agrees_with_the_shaper_it_was_lifted_into() {
        let cases = gpos_cases();
        let mut checked = 0usize;
        let mut moved = 0usize;
        let mut failures = Vec::new();

        for case in &cases {
            let (Some(want), Some(got)) =
                (run_gpos(case, false, false), run_gpos(case, true, false))
            else {
                continue;
            };
            checked += 1;
            // A lookup that adjusted nothing proves only that neither path
            // crashed, so require that a real share of them did something.
            if want
                .0
                .iter()
                .any(|&(a, b, c, d)| a != 0 || b != 0 || c != 0 || d != 0)
            {
                moved += 1;
            }
            if got != want {
                failures.push(format!(
                    "{}: lookup {} on {:?}\n    want {want:?}\n    got  {got:?}",
                    case.name, case.index, case.glyphs
                ));
            }
        }

        assert!(checked > 0, "no positioning lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            moved * 8 > checked,
            "only {moved} of {checked} cases adjusted a position; the probes \
             are not exercising these formats"
        );
        println!("{checked} positioning cases agree, {moved} of them adjusting a position");
    }

    /// The same cases with concat hazards requested.
    ///
    /// Unlike ligature, nothing here is knowingly more precise than this
    /// crate: the pair-set summaries reject only where the search would have
    /// failed, and the row summaries stand in for a record of all zeros, which
    /// this crate applies and finds inert. So the flags must match exactly,
    /// and that is asserted rather than weakened.
    #[test]
    fn positioning_flags_agree_when_concat_hazards_are_requested() {
        let cases = gpos_cases();
        let mut checked = 0usize;
        let mut flagged = 0usize;
        let mut failures = Vec::new();

        for case in &cases {
            let (Some(want), Some(got)) = (run_gpos(case, false, true), run_gpos(case, true, true))
            else {
                continue;
            };
            checked += 1;
            if want.1.iter().any(|&m| m != 1) {
                flagged += 1;
            }
            if got != want {
                failures.push(format!(
                    "{}: lookup {} on {:?}\n    want {want:?}\n    got  {got:?}",
                    case.name, case.index, case.glyphs
                ));
            }
        }

        assert!(checked > 0, "no positioning lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            flagged > 0,
            "no case set a flag; the hazard logic is untested"
        );
        println!("{checked} positioning cases agree exactly, {flagged} of them setting a flag");
    }

    fn walk(dir: &str) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .is_some_and(|x| x == "ttf" || x == "otf" || x == "ttc")
                {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }
}
