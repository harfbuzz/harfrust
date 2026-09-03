//! Applying the substitution formats.
//!
//! One function per format, reached through the pointer a [`Subtable`] carries
//! rather than by matching on its kind. Each opens with the single branch that
//! extracts its own payload -- a check that cannot fail, since the pointer was
//! resolved from that very variant -- and is then sized for its own work
//! rather than for the widest arm of a shared dispatch.
//!
//! # Cluster and safety flags
//!
//! Every format that consumes more than one position owes the buffer a note
//! about it, and getting that wrong is invisible in the glyphs: it shows up
//! only as a caller breaking a line where it may not, or reusing a shaped run
//! it should have reshaped. So each routine states what it owes.
//!
//! A 1->1 substitution owes nothing -- one position in, the same position out,
//! and `replace_glyph` keeps the cluster. The formats that owe something are
//! ligature (merge the components' clusters), the contexts (see
//! [`super::contextual`]) and reverse chaining.

use super::be16;
use super::lookup::{Apply, CompiledLookup, Subtable, SubtableKind};
use crate::hb::ot_map::hb_ot_map_t;
use read_fonts::types::GlyphId;

/// Single substitution format 1: the substitute is `glyph + delta`, wrapped
/// within the 16-bit glyph space as the format requires.
///
/// Nothing to borrow, so the whole payload is the delta and applying touches
/// no font bytes at all. Membership-gated: the coverage index is not read.
///
/// Flags: nothing owed, 1->1.
pub fn at_single_delta(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::SingleDelta { delta } = &sub.kind else {
        return None;
    };
    let subst = (ctx.glyph() as i32 + delta) as u32 & 0xFFFF;
    ctx.host.replace_glyph(GlyphId::from(subst));
    Some(())
}

/// Single substitution format 2: one indexed big-endian read at the coverage
/// index, straight out of the font.
///
/// The array stayed where it was; what the compiler kept is its offset and its
/// length. This is the whole of "compile the index, borrow the payload" in one
/// function -- the coverage that decided we are here is compiled and exact,
/// and the answer it indexes is still in the font.
///
/// Flags: nothing owed, 1->1.
pub fn at_single_list(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::SingleList { subst } = &sub.kind else {
        return None;
    };
    let glyph = subst.get(ctx.table, index)?;
    ctx.host.replace_glyph(GlyphId::from(u32::from(glyph)));
    Some(())
}

/// Multiple substitution: one glyph becomes a sequence, so this is the only
/// format that can lengthen the buffer -- see [`super::lookup::LengthEffect`].
///
/// Flags: the components inherit the input's cluster, so nothing is owed
/// beyond what the buffer's own splice does.
pub fn at_multiple(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::Multiple {
        offset: _,
        effect: _,
    } = &sub.kind
    else {
        return None;
    };
    None
}

/// Ligature substitution: walk the ligature set for the coverage index in font
/// order and apply the first whose components all match.
///
/// The sets stay in the font. What was compiled is the coverage and the pair
/// key -- the set of glyphs that can appear as a *second* component -- which
/// the font does not contain and which is the discriminating half of the
/// filter: `ccmp` covers 72% of English text by first glyph and ligates none
/// of it, because the second component is always a combining mark.
///
/// Flags: **owes `merge_clusters` over the whole matched run.** The components
/// become one glyph, so a caller may not break between the characters they
/// came from.
pub fn at_ligature(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::Ligature { offset: _ } = &sub.kind else {
        return None;
    };
    None
}

/// Alternate substitution: the feature's value selects which alternate, so
/// this is the one format that reads the lookup mask for more than a test.
///
/// Flags: **owes the whole buffer `unsafe_to_break` when it randomises.** The
/// `rand` feature asks for a different alternate each time, which means the
/// answer depends on how many times this ran before -- so no part of the run
/// can be reshaped in isolation. HarfBuzz notes that it could be narrower and
/// does not bother, and neither does this.
pub fn at_alternate(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::Alternate { offset } = &sub.kind else {
        return None;
    };
    // The alternate sets stay in the font: one per covered glyph, and a run
    // reads at most one of them. Two indexed reads to reach ours.
    let base = *offset as usize;
    if index >= u32::from(be16(ctx.table, base + 4)?) {
        return None;
    }
    let set = base + usize::from(be16(ctx.table, base + 6 + index as usize * 2)?);
    let count = be16(ctx.table, set)?;

    // The feature's value picks the variant, and it rides in this lookup's own
    // bits of the mask, shifted down to a plain index. A value of one -- what
    // enabling a feature normally means -- selects the first alternate.
    let mask = ctx.host.lookup_mask();
    let mut value = (mask & ctx.host.buffer.cur(0).mask) >> mask.trailing_zeros();

    // The saturated value means `rand`: pick one at random rather than by
    // index. That makes the result depend on how many times this lookup has
    // already run, so no part of the run can be reshaped on its own -- hence
    // the whole buffer, not the matched position. HarfBuzz notes that it could
    // be narrower and does not bother, because narrowing it would mean
    // tracking the random state per position.
    if value == hb_ot_map_t::MAX_VALUE && ctx.host.random {
        if count == 0 {
            return None;
        }
        ctx.host
            .buffer
            .unsafe_to_break(Some(0), Some(ctx.host.buffer.len));
        value = ctx.host.random_number() % u32::from(count) + 1;
    }

    let alt = (value as usize).checked_sub(1)?;
    if alt >= usize::from(count) {
        return None;
    }
    let glyph = be16(ctx.table, set + 2 + alt * 2)?;
    ctx.host.replace_glyph(GlyphId::from(u32::from(glyph)));
    Some(())
}

/// Reverse chaining contextual single substitution.
///
/// A single input glyph with a context on both sides. The caller runs these
/// descending, from the end of the buffer towards the start, which is what
/// makes the format expressible: each position's rule depends on the decision
/// one to its right having already been made.
///
/// Flags: **owes `unsafe_to_break` over the context it matched**, backtrack and
/// lookahead included -- the substitution depended on glyphs either side, so
/// neither side can be cut away from it.
pub fn at_reverse_chain(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::ReverseChain {
        backtrack: _,
        lookahead: _,
        subst: _,
    } = &sub.kind
    else {
        return None;
    };
    None
}
