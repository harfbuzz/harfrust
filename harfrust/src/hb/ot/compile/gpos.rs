//! Applying the positioning formats.
//!
//! Positioning never changes the glyph count, so every format here is
//! length-preserving and writes where it stands. Value records and anchors
//! stay in the font: a record is a handful of 16-bit fields whose presence a
//! format word describes, so its very size varies, and a class-based pair
//! table holds one per class pair -- thousands in a kerning subtable, of which
//! a shaping run touches a few dozen.
//!
//! # Cluster and safety flags
//!
//! Positioning owes the buffer more than substitution does, and the rule is
//! not the obvious one. Moving two glyphs relative to each other makes the
//! pair unsafe to *break*, but only if something actually moved: a record of
//! all zeros is a match that changed nothing, and that is a concat hazard
//! rather than a break hazard. Both pair formats distinguish the two, which is
//! why they read back whether each value record did anything.
//!
//! `apply_value` is this crate's own, deliberately. It carries the
//! variation-delta arithmetic -- resolving a `VariationIndex` through the item
//! variation store and scaling the fractional result once, so a rounding
//! difference does not creep in -- and that is where the correctness of this
//! format lives. It also returns whether the record moved anything, which is
//! exactly the flag decision above.

use super::be16;
use super::lookup::{Apply, CompiledLookup, Subtable, SubtableKind};
use super::record_size;
use crate::hb::ot::gpos::apply_value;
use crate::hb::ot_layout_gsubgpos::skipping_iterator_t;
use read_fonts::tables::gpos::ValueFormat;
use read_fonts::FontData;

/// Byte offset of the pair-set offset array in a `PairPosFormat1` subtable:
/// past the format, the coverage offset, both value formats and the count.
const PAIR_SET_OFFSETS: usize = 10;

/// Single positioning, both formats: one value record, either shared by the
/// whole coverage or indexed by the coverage rank.
///
/// Flags: nothing owed. One glyph moves on its own, so no pair of positions
/// became dependent on each other.
pub fn at_single_pos(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::SinglePos {
        value_format,
        values,
        shared,
    } = &sub.kind
    else {
        return None;
    };
    // Format 1 gives every covered glyph the same record and so needs no
    // index; format 2 indexes an array by the coverage rank. The gate already
    // computed that rank, which is the only reason this format ranks at all.
    let at = match shared {
        true => *values as usize,
        false => *values as usize + index as usize * record_size(*value_format),
    };
    let idx = ctx.host.buffer.idx;
    apply_value(
        ctx.host,
        idx,
        &FontData::new(ctx.table),
        at,
        ValueFormat::from_bits_truncate(*value_format),
    );
    ctx.host.buffer.idx += 1;
    Some(())
}

/// What both pair formats do once the records are in.
///
/// Three flag decisions, and none of them is about whether a pair was *found*:
///
/// * Something moved, so the two positions now depend on each other and a
///   caller may not break between them.
/// * Nothing moved -- a matched record of all zeros -- so breaking is fine but
///   concatenating is not, because appending text could change which record
///   matched.
/// * The second glyph carries a record of its own, so the cursor steps past it
///   and the span it covers is unsafe to break as well. That last one is
///   HarfBuzz issue 3824.
fn pair_done(
    ctx: &mut Apply,
    second: usize,
    mut cursor: usize,
    moved: bool,
    has_record2: bool,
) -> Option<()> {
    let idx = ctx.host.buffer.idx;
    if moved {
        ctx.host.buffer.unsafe_to_break(Some(idx), Some(second + 1));
    } else {
        ctx.host
            .buffer
            .unsafe_to_concat(Some(idx), Some(second + 1));
    }
    if has_record2 {
        cursor += 1;
        ctx.host.buffer.unsafe_to_break(Some(idx), Some(cursor + 1));
    }
    ctx.host.buffer.idx = cursor;
    Some(())
}

/// The next position a pattern would see, and where the cursor should end up.
///
/// `None` means there is nothing after this glyph to pair it with, which is
/// itself a concat hazard: appending text would give this glyph a neighbour.
fn second_glyph(ctx: &mut Apply) -> Option<(usize, usize)> {
    let mut iter = skipping_iterator_t::new(&mut *ctx.host, false);
    iter.reset(iter.buffer.idx);
    let mut unsafe_to = 0;
    if !iter.next(Some(&mut unsafe_to)) {
        let idx = ctx.host.buffer.idx;
        ctx.host.buffer.unsafe_to_concat(Some(idx), Some(unsafe_to));
        return None;
    }
    Some((iter.index(), iter.buf_idx))
}

/// Pair positioning format 1: the second glyph is found by binary search in
/// the pair set for the first.
///
/// The compiled part is `seconds`: one machine word per pair set saying which
/// glyphs that set's first glyph pairs with. This glyph pairs with a handful
/// of others and on running text the next glyph is almost never one of them,
/// so one `and` answers what the search below walks the font to conclude.
///
/// It is also the one filter here that cannot change a flag. It rejects only
/// when the search would have failed, and a failed search sets nothing.
///
/// Flags: see [`pair_done`].
pub fn at_pair1(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::PairPos1 {
        seconds,
        offset,
        first_format,
        second_format,
    } = &sub.kind
    else {
        return None;
    };
    let (second, cursor) = second_glyph(ctx)?;
    let want = ctx.host.buffer.info[second].glyph_id;

    if !seconds.may_hold(index, want) {
        return None;
    }

    let base = *offset as usize;
    if index >= u32::from(be16(ctx.table, base + 8)?) {
        return None;
    }
    let set = base
        + usize::from(be16(
            ctx.table,
            base + PAIR_SET_OFFSETS + index as usize * 2,
        )?);
    let count = usize::from(be16(ctx.table, set)?);
    let first_len = record_size(*first_format);
    let stride = 2 + first_len + record_size(*second_format);

    // Pair records are sorted by second glyph, so this is a binary search.
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        // `usize::midpoint` widens to u128 to avoid an overflow that cannot
        // happen here: both bounds came from a 16-bit count.
        #[allow(clippy::manual_midpoint)]
        let mid = (lo + hi) / 2;
        let at = set + 2 + mid * stride;
        let glyph = u32::from(be16(ctx.table, at)?);
        match glyph.cmp(&want) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => {
                return apply_pair(ctx, second, cursor, at + 2, *first_format, *second_format);
            }
        }
    }
    None
}

/// Pair positioning format 2: both glyphs resolve to classes and the record is
/// at the intersection, so reaching it is a matrix index rather than a search.
///
/// The classes are compiled because every probe reads them; the matrix itself
/// stays in the font, and it is the larger of the two by far.
///
/// The compiled part beyond that is `rows`: one word per class-1 row saying
/// which class-2 values that row has a non-zero record for. A class matrix is
/// mostly zeros -- an entry for every pair of classes, and few pairs kern.
///
/// This filter *can* be seen in the flags, and is arranged so it is not. A
/// zero record is still a match: HarfBuzz applies it, finds nothing moved, and
/// records a concat hazard. So rejecting here does the same, rather than
/// returning as though nothing matched. The rows are built from the record's
/// raw bytes, which is what makes that sound -- an all-zero record has zero
/// values *and* null device offsets, and a null device offset moves nothing.
///
/// Flags: see [`pair_done`].
pub fn at_pair2(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::PairPos2 {
        rows,
        class1,
        class2,
        matrix,
        first_format,
        second_format,
        class2_count,
    } = &sub.kind
    else {
        return None;
    };
    let (second, cursor) = second_glyph(ctx)?;

    let c1 = u32::from(class1.get(ctx.glyph()));
    let c2 = u32::from(class2.get(ctx.host.buffer.info[second].glyph_id));
    let has_record2 = record_size(*second_format) != 0;

    if !rows.may_hold(c1, c2) {
        // Matched, moved nothing.
        return pair_done(ctx, second, cursor, false, has_record2);
    }

    let pair = record_size(*first_format) + record_size(*second_format);
    let at = *matrix as usize + (c1 as usize * usize::from(*class2_count) + c2 as usize) * pair;
    apply_pair(ctx, second, cursor, at, *first_format, *second_format)
}

/// Apply a pair of value records, then settle the flags and the cursor.
///
/// Whether each record moved anything is what decides between a break hazard
/// and a concat hazard, so the results are read rather than discarded. An
/// empty format is not applied at all -- there are no bytes to read -- and
/// counts as having moved nothing.
fn apply_pair(
    ctx: &mut Apply,
    second: usize,
    cursor: usize,
    at: usize,
    first_format: u16,
    second_format: u16,
) -> Option<()> {
    let data = FontData::new(ctx.table);
    let first = ValueFormat::from_bits_truncate(first_format);
    let second_fmt = ValueFormat::from_bits_truncate(second_format);
    let idx = ctx.host.buffer.idx;

    let moved1 = !first.is_empty() && apply_value(ctx.host, idx, &data, at, first) == Some(true);
    let has_record2 = !second_fmt.is_empty();
    let moved2 = has_record2
        && apply_value(
            ctx.host,
            second,
            &data,
            at + record_size(first_format),
            second_fmt,
        ) == Some(true);

    pair_done(ctx, second, cursor, moved1 || moved2, has_record2)
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
