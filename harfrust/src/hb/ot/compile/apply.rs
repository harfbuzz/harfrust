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

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::hb::buffer::Buffer;
    use crate::hb::face::Scale;
    use crate::hb::ot::compile::compile_gsub_program;
    use crate::hb::ot::compile::lookup::{Program, SubtableKind};
    use crate::hb::ot_layout::{apply_synthesized_subst_lookup, TableIndex};
    use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
    use crate::BufferFlags;
    use crate::{FontRef, ShaperData};
    use read_fonts::tables::gsub::{
        ExtensionSubtable, Gsub, LigatureSubstFormat1, SubstitutionLookup,
    };
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
                for glyphs in probes(&gsub, &program, index, &table) {
                    if glyphs.len() < 2 {
                        continue;
                    }
                    out.push(Case {
                        name: name.clone(),
                        data: data.clone(),
                        index,
                        glyphs,
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
        if !info.is_subst || info.is_reverse() {
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
        // The vars GSUB writes through: ligature ids and glyph props. Shaping
        // allocates these around the substitution stage, and setting glyph
        // props below asserts they are there.
        buffer.allocate_gsubgpos_vars();
        crate::hb::ot_layout::hb_ot_layout_substitute_start(&face, &mut buffer);

        let mut ctx =
            hb_ot_apply_context_t::new(TableIndex::GSUB, &face, Scale::default(), &mut buffer);
        ctx.lookup_index = case.index;
        ctx.set_lookup_mask(1);

        if mine {
            let gsub = font.gsub().ok()?;
            let program = compile_gsub_program(&gsub);
            let compiled = program.get(case.index, table)?;
            ctx.lookup_props = info.props();
            ctx.update_matchers();
            ctx.buffer.clear_output();
            ctx.buffer.idx = 0;
            let mut apply = Apply {
                host: &mut ctx,
                table,
                program: &program,
            };
            apply_forward(&mut apply, compiled);
            ctx.buffer.sync();
        } else {
            apply_synthesized_subst_lookup(&mut ctx, info, table);
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
            if want.0 != case.glyphs || want.1.iter().enumerate().any(|(i, &c)| c != i as u32) {
                effective += 1;
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
        println!(
            "{checked} cases agree, {effective} of them changing glyphs or \
             clusters, {grew} lengthening the buffer and {shrank} shortening it"
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
