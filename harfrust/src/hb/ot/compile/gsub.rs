//! Applying the substitution formats.
//!
//! One function per format, reached through the pointer a [`Subtable`] carries
//! rather than by matching on its kind. Each opens with the single branch that
//! extracts its own payload -- a check that cannot fail, since the pointer was
//! resolved from that very variant -- and is then sized for its own work
//! rather than for the widest arm of a shared dispatch.
//!
//! Every one of these is a stub. The compiled form is in place and dispatches
//! correctly; what each format then does with the font bytes it borrowed is
//! still to be written against this crate's buffer.

use super::lookup::host::hb_ot_apply_context_t;
use super::lookup::{CompiledLookup, Subtable, SubtableKind};

/// Single substitution format 1: the substitute is `(glyph + delta)`, wrapped
/// within the 16-bit glyph space.
///
/// Nothing to borrow, so the whole payload is the delta. Membership-gated: the
/// coverage index is not read.
pub fn at_single_delta(
    _ctx: &mut hb_ot_apply_context_t,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::SingleDelta { delta: _ } = &sub.kind else {
        return None;
    };
    None
}

/// Single substitution format 2: one indexed big-endian read at the coverage
/// index, straight out of the font.
pub fn at_single_list(
    _ctx: &mut hb_ot_apply_context_t,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::SingleList { subst: _ } = &sub.kind else {
        return None;
    };
    None
}

/// Multiple substitution: one glyph becomes a sequence, so this is the only
/// format that can lengthen the buffer -- see [`super::lookup::LengthEffect`].
pub fn at_multiple(
    _ctx: &mut hb_ot_apply_context_t,
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
/// filter.
pub fn at_ligature(
    _ctx: &mut hb_ot_apply_context_t,
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
pub fn at_alternate(
    _ctx: &mut hb_ot_apply_context_t,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::Alternate { offset: _ } = &sub.kind else {
        return None;
    };
    None
}

/// Reverse chaining contextual single substitution.
///
/// A single input glyph with a context on both sides. The caller runs these
/// descending, from the end of the buffer towards the start, which is what
/// makes the format expressible: each position's rule depends on the decision
/// one to its right having already been made.
pub fn at_reverse_chain(
    _ctx: &mut hb_ot_apply_context_t,
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
