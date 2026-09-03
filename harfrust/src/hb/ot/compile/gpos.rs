//! Applying the positioning formats.
//!
//! Positioning never changes the glyph count, so every format here is
//! length-preserving and writes where it stands. Value records and anchors
//! stay in the font: a record is a handful of 16-bit fields whose presence a
//! format word describes, so its very size varies, and a class-based pair
//! table holds one per class pair -- thousands in a kerning subtable, of which
//! a shaping run touches a few dozen.
//!
//! Every one of these is a stub. See [`super::gsub`].

use super::lookup::{Apply, CompiledLookup, Subtable, SubtableKind};

/// Single positioning, both formats: one value record, either shared by the
/// whole coverage or indexed by the coverage rank.
pub fn at_single_pos(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::SinglePos { .. } = &sub.kind else {
        return None;
    };
    None
}

/// Pair positioning format 1: the second glyph is found by binary search in
/// the pair set for the first.
///
/// The compiled part is `seconds`, the union of every second glyph -- the pair
/// key, which lets a candidate be rejected on the glyph that follows it before
/// the font is touched at all.
pub fn at_pair1(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::PairPos1 { .. } = &sub.kind else {
        return None;
    };
    None
}

/// Pair positioning format 2: both glyphs resolve to classes and the record is
/// at the intersection, so the lookup is a matrix index rather than a search.
pub fn at_pair2(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::PairPos2 { .. } = &sub.kind else {
        return None;
    };
    None
}

/// Cursive attachment: the exit anchor of one glyph meets the entry anchor of
/// the next, and which of the two moves depends on the run's direction.
pub fn at_cursive(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::Cursive { .. } = &sub.kind else {
        return None;
    };
    None
}

/// Mark attachment, all three kinds at once.
///
/// Base, ligature and mark-to-mark differ in how the target is found and how
/// its anchors are laid out, not in what happens once it is -- so they share a
/// routine and carry an [`super::lookup::AttachTo`] to say which.
pub fn at_mark_to(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::MarkTo { .. } = &sub.kind else {
        return None;
    };
    None
}
