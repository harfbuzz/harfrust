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

pub use super::lookup::Apply;
use super::lookup::{CompiledLookup, Program};
use super::set::GlyphSet;

/// Settle which shape the reach has, once, and run the pass against it.
///
/// `GlyphSet::contains` is a match over five variants. Inlined into the scan
/// that match becomes the loop's own overhead, and it is worse than it looks:
/// the compiler builds a jump table, so every position pays a load of the
/// set's discriminant -- a different cache line from its words -- a load from
/// the table, and an indirect jump. A profile of English put a third of the
/// scan's time on those three instructions.
///
/// The variant cannot change under the pass, so it is settled here and the
/// scan is handed a closure with the test already chosen. Each arm gets its
/// own copy of the loop with a straight-line probe in it.
macro_rules! over_reach {
    ($set:expr, $run:ident, $ctx:expr, $lookup:expr) => {
        match $set {
            GlyphSet::Empty => false,
            GlyphSet::Range { first, len } => {
                let (first, len) = (*first, *len);
                $run($ctx, $lookup, move |g| g.wrapping_sub(first) < len)
            }
            GlyphSet::Bitmap { base, words } => {
                // One compare, not two. `checked_sub` and a bounds check on
                // the word ask the same question twice: a glyph is in range
                // exactly when its offset from the base is below the span the
                // words cover, and wrapping makes a glyph below the base land
                // above that. Both ends fall out of the one unsigned compare,
                // and the index that follows is then in range by construction.
                let (base, words) = (*base, &words[..]);
                let span = (words.len() as u32) << 6;
                $run($ctx, $lookup, move |g| {
                    let o = g.wrapping_sub(base);
                    o < span && (words[(o >> 6) as usize] >> (o & 63)) & 1 != 0
                })
            }
            other => {
                let other: &GlyphSet = other;
                $run($ctx, $lookup, move |g| other.contains(g))
            }
        }
    };
}

/// Counting, not guessing.
///
/// Enabled by the `compile-stats` feature and compiled out otherwise. Every
/// number here was a question that could be answered by reasoning, and more
/// than one of them was answered wrong by reasoning first.
#[cfg(feature = "compile-stats")]
pub mod stats {
    use core::sync::atomic::{AtomicU64, Ordering};

    macro_rules! counters {
        ($($name:ident),* $(,)?) => {
            $(pub static $name: AtomicU64 = AtomicU64::new(0);)*
            pub fn all() -> alloc::vec::Vec<(&'static str, u64)> {
                alloc::vec![$((stringify!($name), $name.load(Ordering::Relaxed))),*]
            }
        };
    }

    counters!(LOOKUPS, SCANNED, REACHED, MASKED, CANDIDATES, APPLIED, GATED, GATE_PASS,);

    #[inline]
    pub fn bump(c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }
}

/// Bump a counter when the feature is on, and vanish when it is not.
macro_rules! count {
    ($name:ident, $n:expr) => {
        #[cfg(feature = "compile-stats")]
        stats::bump(&stats::$name, $n);
    };
}
use crate::hb::ot_layout_gsubgpos::MatchPositions;
use crate::hb::ot_layout_gsubgpos::OT::check_glyph_property;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;

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
    count!(CANDIDATES, 1);
    let glyph = ctx.glyph();
    // One subtable is the common case and none of the machinery below earns
    // its place there: no order to choose, nothing to index, nothing to skip.
    // SourceSerif reaches ten lookups holding sixteen subtables between them,
    // and paying a dispatch probe and a bounds check per application is how a
    // faster format ends up a slower lookup.
    if let [sub] = &lookup.subtables[..] {
        count!(GATED, 1);
        let index = sub.gate(glyph)?;
        count!(GATE_PASS, 1);
        return (sub.apply)(ctx, lookup, sub, index);
    }
    // Without an index, every subtable in font order, each asking its own
    // coverage.
    //
    // The call below is indirect rather than a match on the format, which was
    // settled when the subtable was compiled. Within a hot loop its target is
    // stable per lookup, so the branch predictor gets it right nearly every
    // time, and matching the common formats in front of it only adds work.
    let Some(dispatch) = lookup.dispatch.as_deref() else {
        for sub in &lookup.subtables {
            count!(GATED, 1);
            let Some(index) = sub.gate(glyph) else {
                continue;
            };
            count!(GATE_PASS, 1);
            if (sub.apply)(ctx, lookup, sub, index).is_some() {
                return Some(());
            }
        }
        return None;
    };

    // With one, only the subtables that can start on this glyph, in the order
    // they must be. A row is built from the subtables' own coverages, so a
    // subtable named by one has already been shown to cover this glyph, and
    // only the formats that read the coverage *index* still have to ask.
    let row = dispatch.row(glyph);
    // A row naming one subtable is what an index is for, and it is what nearly
    // every row is: SourceSerif's kerning is five subtables and a glyph
    // reaches one of them. There is no order to walk and nothing to fall
    // through to, so take the same path a one-subtable lookup takes.
    if let [only] = row {
        if let Some(sub) = lookup.subtables.get(*only as usize) {
            let index = if sub.rank { sub.gate(glyph)? } else { 0 };
            return (sub.apply)(ctx, lookup, sub, index);
        }
    }
    for &at in row {
        let Some(sub) = lookup.subtables.get(at as usize) else {
            continue;
        };
        let index = if sub.rank {
            count!(GATED, 1);
            let Some(index) = sub.gate(glyph) else {
                continue;
            };
            count!(GATE_PASS, 1);
            index
        } else {
            0
        };
        if (sub.apply)(ctx, lookup, sub, index).is_some() {
            return Some(());
        }
    }
    None
}

/// Apply at the cursor, knowing the scan has already proved the glyph is in
/// this lookup's reach.
///
/// That knowledge is worth a probe. A lookup with one subtable has a reach
/// which *is* that subtable's coverage -- `reach` is built by unioning the
/// subtable coverages, and a union of one is the thing itself -- so a gate that
/// only asks about membership is asking a question the scan just answered.
/// Counting confirmed it: over a run of English every such gate admitted, fifty
/// million of them, without once rejecting.
///
/// A gate that reads the coverage *index* still has to run, since the index is
/// what the format indexes with. This only skips the ones that would return a
/// constant.
#[inline]
fn apply_at_reached(ctx: &mut Apply, lookup: &CompiledLookup) -> Option<()> {
    if let [sub] = &lookup.subtables[..] {
        if !sub.rank {
            count!(CANDIDATES, 1);
            return (sub.apply)(ctx, lookup, sub, 0);
        }
    }
    apply_at(ctx, lookup)
}

/// One forward pass over the buffer.
///
/// Reports whether anything applied. The caller owns the output buffer
/// discipline -- `clear_output` before, `sync` after, for a table that is not
/// applied in place.
pub fn apply_forward(ctx: &mut Apply, lookup: &CompiledLookup) -> bool {
    over_reach!(lookup.reach(), forward_over, ctx, lookup)
}

/// The forward pass, over a candidate test that is already resolved.
fn forward_over<F: Fn(u32) -> bool>(ctx: &mut Apply, lookup: &CompiledLookup, reach: F) -> bool {
    let mut applied = false;
    // Read out of the context once. A nested lookup saves and restores all
    // three, so they cannot change under this loop -- and hoisting them is what
    // lets the scan below borrow the buffer as a plain slice.
    let face = ctx.host.face;
    let lookup_mask = ctx.host.lookup_mask();
    let lookup_props = ctx.host.lookup_props;

    count!(LOOKUPS, 1);
    while ctx.host.buffer.successful {
        // Scan to the next position this lookup could touch. Three tests, and
        // the first is the compiled reach rather than a parse of the font.
        let idx = ctx.host.buffer.idx;
        let j = {
            let infos = &ctx.host.buffer.info[..ctx.host.buffer.len];
            let mut j = idx;
            while j < infos.len() {
                let info = &infos[j];
                count!(SCANNED, 1);
                if reach(info.glyph_id) {
                    count!(REACHED, 1);
                    if (info.mask & lookup_mask) != 0 {
                        count!(MASKED, 1);
                        if check_glyph_property(face, info, lookup_props) {
                            break;
                        }
                    }
                }
                j += 1;
            }
            j
        };
        if j > idx {
            ctx.host.buffer.next_glyphs(j - idx);
        }
        if ctx.host.buffer.idx >= ctx.host.buffer.len {
            break;
        }

        // A format that applied has already advanced the cursor past what it
        // consumed; one that did not leaves it to us.
        if apply_at_reached(ctx, lookup).is_some() {
            count!(APPLIED, 1);
            applied = true;
        } else {
            ctx.host.buffer.next_glyph();
        }
    }
    applied
}

/// Apply a nested lookup, staying on the compiled path.
///
/// The counterpart of this crate's own `hb_ot_apply_context_t::recurse`, and it
/// exists because a context that fell back to the font's path would be a hole
/// that everything reached through it fell into. On a script whose shaping is
/// mostly contexts -- which is what a cursive script is -- that would be nearly
/// every lookup that does any work, and a measurement taken with it would say
/// almost nothing about this path.
///
/// The state it saves and restores is the same state, and for the same reasons.
/// The nested lookup gets its own properties and its own matchers, and must not
/// be able to see or disturb the match positions of the context that invoked
/// it. The two budgets are shared and are what stop a font recursing forever.
pub fn recurse(
    host: &mut hb_ot_apply_context_t,
    table: &[u8],
    program: &Program,
    lookup_index: u16,
) -> Option<()> {
    if host.nesting_level_left == 0 {
        host.buffer.successful = false;
        return None;
    }
    host.buffer.max_ops -= 1;
    if host.buffer.max_ops < 0 {
        host.buffer.successful = false;
        return None;
    }

    host.nesting_level_left -= 1;
    let saved_props = host.lookup_props;
    let saved_index = host.lookup_index;
    // Moved out rather than cloned: the nested lookup never reads the caller's
    // positions, and the replacement has to keep the length at one or more,
    // since a single-position match writes slot zero without resizing.
    let saved_positions =
        core::mem::replace(&mut host.match_positions, MatchPositions::from_elem(0, 1));
    let saved_positions_len = host.match_positions_len;

    host.lookup_index = lookup_index;
    let applied = program.get(lookup_index, table).and_then(|nested| {
        host.lookup_props = nested.props;
        host.update_matchers();
        let mut ctx = Apply {
            host,
            table,
            program,
        };
        apply_at(&mut ctx, nested)
    });

    host.lookup_props = saved_props;
    host.lookup_index = saved_index;
    host.update_matchers();
    host.match_positions = saved_positions;
    host.match_positions_len = saved_positions_len;
    host.nesting_level_left += 1;
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
    over_reach!(lookup.reach(), backward_over, ctx, lookup)
}

fn backward_over<F: Fn(u32) -> bool>(ctx: &mut Apply, lookup: &CompiledLookup, reach: F) -> bool {
    let mut applied = false;
    // Read out of the context once: these cannot change under a reverse
    // lookup, which neither recurses nor alters the buffer's length.
    let face = ctx.host.face;
    let lookup_mask = ctx.host.lookup_mask();
    let lookup_props = ctx.host.lookup_props;

    loop {
        let idx = ctx.host.buffer.idx;
        let candidate = ctx.host.buffer.info[..=idx].iter().rposition(|info| {
            reach(info.glyph_id)
                && (info.mask & lookup_mask) != 0
                && check_glyph_property(face, info, lookup_props)
        });
        let Some(at) = candidate else {
            ctx.host.buffer.idx = 0;
            break;
        };

        ctx.host.buffer.idx = at;
        applied |= apply_at_reached(ctx, lookup).is_some();

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
    use crate::hb::ot::compile::be16;
    use crate::hb::ot::compile::lookup::{CompiledLookup, Program, SubtableKind};
    use crate::hb::ot::compile::set::{ClassMap, GlyphSet};
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
                        | SubtableKind::ChainCtx3 { .. }
                        | SubtableKind::Rules { .. }
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
        let mut reach = Vec::new();
        lookup.reach_into(&mut reach);
        reach.truncate(MAX_GLYPHS - 1);
        reach.push(0);
        let mut out = vec![reach];

        out.extend(context_probes(lookup));
        out.extend(rule_probes(table, lookup));

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

    /// One buffer per context subtable that satisfies its pattern.
    ///
    /// The context formats fire only when the glyphs around the gate are in the
    /// right coverages, which arbitrary glyphs are not, so the buffer has to be
    /// built to match: one glyph from each set, with the backtrack laid out in
    /// buffer order rather than the nearest-first order the format stores it
    /// in. A set with no glyphs in it means the pattern is unsatisfiable, and
    /// the case is dropped rather than tested as a near miss.
    fn context_probes(lookup: &CompiledLookup) -> Vec<Vec<u32>> {
        let one = |set: &GlyphSet| set.to_vec().first().copied();
        let mut out = Vec::new();
        for sub in &lookup.subtables {
            let (backtrack, middle, lookahead) = match &sub.kind {
                SubtableKind::ReverseChain {
                    backtrack,
                    lookahead,
                    ..
                } => (&backtrack[..], &[][..], &lookahead[..]),
                SubtableKind::ChainCtx3 {
                    backtrack,
                    input,
                    lookahead,
                    ..
                } => (&backtrack[..], &input[..], &lookahead[..]),
                _ => continue,
            };
            let Some(&gate) = sub.cov.to_vec().first() else {
                continue;
            };
            let Some(mut seq) = backtrack
                .iter()
                .rev()
                .map(|s| one(s))
                .collect::<Option<Vec<u32>>>()
            else {
                continue;
            };
            seq.push(gate);
            let Some(rest) = middle
                .iter()
                .chain(lookahead.iter())
                .map(|s| one(s))
                .collect::<Option<Vec<u32>>>()
            else {
                continue;
            };
            seq.extend(rest);
            out.push(seq);
        }
        out
    }

    /// One buffer per rule, built to satisfy it.
    ///
    /// Coverage glyphs in a row will enter a rule set and then match nothing in
    /// it, so without this the rule-based formats would be tested only for
    /// their rejection paths. A rule names its positions either as glyph ids or
    /// as classes, depending on the format, so the buffer is built by walking
    /// the rule's own value arrays out of the font and turning each value into
    /// a glyph that satisfies it -- for a class, by finding any glyph the
    /// compiled map puts in that class.
    ///
    /// Rules are variable-length and the two layouts differ in a way that is
    /// easy to miss: an unchained rule puts its lookup count second, right
    /// after the glyph count, while a chained one puts it last, after the
    /// lookahead. Reading one the other way round yields a rule that simply
    /// never matches, which is why this parses them explicitly rather than
    /// sharing a path.
    fn rule_probes(table: &[u8], lookup: &CompiledLookup) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        for sub in &lookup.subtables {
            let SubtableKind::Rules {
                input_classes,
                backtrack_classes,
                lookahead_classes,
                rule_sets,
                base,
                rule_set_count,
                chained,
                ..
            } = &sub.kind
            else {
                continue;
            };
            let covered = sub.cov.to_vec();
            // Any glyph in the given class, or the value itself when the format
            // compares glyph ids.
            let glyph_for = |map: Option<&ClassMap>, value: u32| -> Option<u32> {
                match map {
                    None => Some(value),
                    Some(m) => (0..4096u32).find(|&g| u32::from(m.get(g)) == value),
                }
            };

            for set_index in 0..u32::from(*rule_set_count) {
                // The gate glyph has to be one that selects this rule set:
                // the coverage entry at this rank for format 1, or any covered
                // glyph in this class for format 2.
                let Some(gate) = (match input_classes {
                    None => covered.get(set_index as usize).copied(),
                    Some(m) => covered
                        .iter()
                        .copied()
                        .find(|&g| u32::from(m.get(g)) == set_index),
                }) else {
                    continue;
                };
                let Some(offset) = be16(table, *rule_sets as usize + set_index as usize * 2) else {
                    continue;
                };
                if offset == 0 {
                    continue;
                }
                let set_at = *base as usize + usize::from(offset);
                let Some(count) = be16(table, set_at) else {
                    continue;
                };
                for r in 0..usize::from(count).min(MAX_CASES) {
                    let Some(rule_off) = be16(table, set_at + 2 + r * 2) else {
                        continue;
                    };
                    let mut at = set_at + usize::from(rule_off);

                    // Backtrack, nearest-first, so laying it out in buffer
                    // order means reversing it.
                    let mut before = Vec::new();
                    if *chained {
                        let Some(n) = be16(table, at) else { continue };
                        at += 2;
                        for k in 0..usize::from(n) {
                            let Some(v) = be16(table, at + k * 2) else {
                                continue;
                            };
                            before.push(glyph_for(backtrack_classes.as_deref(), u32::from(v)));
                        }
                        at += usize::from(n) * 2;
                    }

                    let Some(input_count) = be16(table, at) else {
                        continue;
                    };
                    at += 2;
                    if input_count == 0 {
                        continue;
                    }
                    // Unchained: the lookup count sits here, before the input.
                    if !*chained {
                        at += 2;
                    }
                    // The first input position is the covered glyph itself, so
                    // only the rest are listed.
                    let mut middle = Vec::new();
                    for k in 0..usize::from(input_count) - 1 {
                        let Some(v) = be16(table, at + k * 2) else {
                            continue;
                        };
                        middle.push(glyph_for(input_classes.as_deref(), u32::from(v)));
                    }
                    at += (usize::from(input_count) - 1) * 2;

                    let mut after = Vec::new();
                    if *chained {
                        let Some(n) = be16(table, at) else { continue };
                        at += 2;
                        for k in 0..usize::from(n) {
                            let Some(v) = be16(table, at + k * 2) else {
                                continue;
                            };
                            after.push(glyph_for(lookahead_classes.as_deref(), u32::from(v)));
                        }
                    }

                    // A value with no glyph in its class makes the rule
                    // unsatisfiable; drop the case rather than test a near miss.
                    let gate = Some(gate);
                    let parts = before
                        .iter()
                        .rev()
                        .chain(core::iter::once(&gate))
                        .chain(middle.iter())
                        .chain(after.iter());
                    if let Some(seq) = parts.copied().collect::<Option<Vec<u32>>>() {
                        if seq.len() >= 2 {
                            out.push(seq);
                        }
                    }
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
        // Shaping seeds these during the unicode-props pass; a buffer built
        // by hand has to do it itself, and every lookup here runs with mask 1.
        for info in &mut buffer.info[..case.glyphs.len()] {
            info.mask = 1;
        }
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

    /// The same cases with the caller asking for unsafe-to-concat.
    ///
    /// Worth its own test because those flags are off by default -- the sweep
    /// above cannot see them at all, since `Buffer::unsafe_to_concat` returns
    /// immediately unless the buffer asked for them.
    ///
    /// Every filter the compiled path adds rejects only where the walk it
    /// replaces would have reported a hazard, and reports the same one, so
    /// these must match exactly rather than merely closely.
    #[test]
    fn concat_hazards_match_the_ones_this_crate_marks() {
        let cases = cases();
        let mut checked = 0usize;
        let mut flagged = 0usize;
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
            if got.2 != want.2 {
                failures.push(format!(
                    "{}: lookup {} on {:?}: concat hazards differ\n    \
                     want {:?}\n    got  {:?}",
                    case.name, case.index, case.glyphs, want.2, got.2
                ));
            } else if got.2.iter().any(|&f| f != 0) {
                flagged += 1;
            }
        }

        assert!(checked > 0, "no lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases disagree on concat hazards:\n{}",
            failures.len(),
            failures.join("\n")
        );
        println!(
            "{checked} cases agree on glyphs, clusters and concat hazards, \
             {flagged} of them marking one"
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
                        | SubtableKind::ChainCtx3 { .. }
                        | SubtableKind::Rules { .. }
                        | SubtableKind::Cursive { .. }
                        | SubtableKind::MarkTo { .. }
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
        let mut reach = Vec::new();
        lookup.reach_into(&mut reach);
        reach.truncate(MAX_GLYPHS - 1);
        reach.push(0);
        let mut out = vec![reach];
        out.extend(context_probes(lookup));
        out.extend(rule_probes(table, lookup));

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
        // Shaping seeds these during the unicode-props pass; a buffer built
        // by hand has to do it itself, and every lookup here runs with mask 1.
        for info in &mut buffer.info[..case.glyphs.len()] {
            info.mask = 1;
        }
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
    /// The pair-set summaries reject only where the search would have missed,
    /// and report what a miss reports; the row summaries stand in for a record
    /// of all zeros, which HarfBuzz applies and finds inert. Neither is
    /// visible in the flags, so these must match exactly.
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
            if got.0 != want.0 {
                failures.push(format!(
                    "{}: lookup {} on {:?}: positions differ
    \n                     want {:?}
    got  {:?}",
                    case.name, case.index, case.glyphs, want.0, got.0
                ));
            } else if got.1 != want.1 {
                failures.push(format!(
                    "{}: lookup {} on {:?}: concat hazards differ
    \n                     want {:?}
    got  {:?}",
                    case.name, case.index, case.glyphs, want.1, got.1
                ));
            }
        }

        assert!(checked > 0, "no positioning lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} cases disagree:
{}",
            failures.len(),
            failures.join(
                "
"
            )
        );
        assert!(
            flagged > 0,
            "no case set a flag; the hazard logic is untested"
        );
        println!(
            "{checked} positioning cases agree on positions and concat hazards,              {flagged} of them marking one"
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
