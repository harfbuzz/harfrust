//! Positioning subtables.

use super::coverage::{ClassDef, Coverage};
use super::{glyph_class_cached, Offsets, SubtableDigests};
use crate::budget::{Budget, Exhausted};
use crate::buffer::{Buffer, GlyphPosition, HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT};
use crate::hint::{Likely, LikelyOk};
use crate::ot::apply::{ApplyContext, MappingCache, MatchResult, SkippingIterator};
use crate::ot::gpos::attach_type;
use crate::ot::lookup_flags;
use crate::set_digest::SetDigest;
use crate::Direction;
use alloc::{boxed::Box, vec::Vec};
use read_fonts::tables::gpos::{
    AnchorTable, BaseArray, ClassValueRecords, CursivePosFormat1, EntryExitRecord, LigatureArray,
    Mark2Array, MarkArray, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1,
    PairPosFormat1, PairPosFormat2, PairSet, SinglePosFormat1, SinglePosFormat2, ValueFormat,
    ValueRecord,
};
use read_fonts::tables::variations::DeltaSetIndex;
use read_fonts::types::{BigEndian, GlyphId};
use read_fonts::FontData;

/// Adds `record` to the position of glyph `idx`, returning whether it
/// changed anything. A truncated record changes nothing.
#[inline]
fn apply_value(ctx: &mut ApplyContext, idx: usize, record: &ValueRecord) -> bool {
    let Some(bytes) = record.bytes() else {
        return false;
    };
    // Most kerning adjusts only the advance.
    if record.format() == ValueFormat::X_ADVANCE {
        let &[hi, lo] = bytes else {
            return false;
        };
        let value = i16::from_be_bytes([hi, lo]);
        if !ctx.buffer.direction.is_horizontal() {
            return false;
        }
        let x_advance = ctx.scale.scale_x(i32::from(value));
        let Some(pos) = ctx.buffer.pos.get_mut(idx).likely() else {
            return false;
        };
        pos.x_advance = pos.x_advance.saturating_add(x_advance);
        return value != 0;
    }
    apply_value_fields(ctx, idx, bytes, record.format(), record.offset_data())
}

/// Applies a value record of any format. Device offsets in it are relative
/// to `base`.
// The cursor is advanced past the last field too, for uniformity.
#[allow(unused_assignments)]
#[inline(never)]
fn apply_value_fields(
    ctx: &mut ApplyContext,
    idx: usize,
    bytes: &[u8],
    format: ValueFormat,
    base: FontData,
) -> bool {
    let scale = ctx.scale;
    let is_horizontal = ctx.buffer.direction.is_horizontal();
    let Some(pos) = ctx.buffer.pos.get_mut(idx).likely() else {
        return false;
    };
    let mut fields = bytes
        .chunks_exact(2)
        .map(|field| i16::from_be_bytes([field[0], field[1]]));
    let mut worked = false;
    // Reads the next value, which counts only if it applies in this
    // direction.
    macro_rules! value {
        ($applies:expr) => {{
            let value = fields.next().unwrap_or(0);
            let applies = $applies;
            worked |= applies && value != 0;
            applies.then_some(i32::from(value))
        }};
    }
    if format.contains(ValueFormat::X_PLACEMENT) {
        if let Some(value) = value!(true) {
            pos.x_offset = pos.x_offset.saturating_add(scale.scale_x(value));
        }
    }
    if format.contains(ValueFormat::Y_PLACEMENT) {
        if let Some(value) = value!(true) {
            pos.y_offset = pos.y_offset.saturating_add(scale.scale_y(value));
        }
    }
    if format.contains(ValueFormat::X_ADVANCE) {
        if let Some(value) = value!(is_horizontal) {
            pos.x_advance = pos.x_advance.saturating_add(scale.scale_x(value));
        }
    }
    if format.contains(ValueFormat::Y_ADVANCE) {
        if let Some(value) = value!(!is_horizontal) {
            pos.y_advance = pos.y_advance.saturating_sub(scale.scale_y(value));
        }
    }
    if !format.intersects(ValueFormat::ANY_DEVICE_OR_VARIDX) {
        return worked;
    }
    let Some(store) = ctx.layout.ot.var_store() else {
        return worked;
    };
    let coords = ctx.layout.ot.coords;
    // Reads the next device offset and resolves its delta, if it applies in
    // this direction. Only variation indices contribute; device tables are
    // for hinting, which shaping doesn't do.
    let mut delta = |applies: bool, worked: &mut bool| -> Option<f32> {
        let device = fields.next().unwrap_or(0) as u16 as usize;
        if !applies {
            return None;
        }
        const VARIATION_INDEX_FORMAT: u16 = 0x8000;
        if device == 0 || base.read_at::<u16>(device + 4).ok() != Some(VARIATION_INDEX_FORMAT) {
            return Some(0.0);
        }
        let outer = base.read_at::<u16>(device).ok()?;
        let inner = base.read_at::<u16>(device + 2).ok()?;
        // Keep the delta fractional so it is rounded once, when scaled.
        let value = store
            .compute_delta(DeltaSetIndex { outer, inner }, coords)
            .unwrap_or_default()
            .to_f64() as f32;
        *worked |= value != 0.0;
        Some(value)
    };
    if format.contains(ValueFormat::X_PLACEMENT_DEVICE) {
        if let Some(value) = delta(true, &mut worked) {
            pos.x_offset = pos.x_offset.saturating_add(scale.scale_x_f(value));
        }
    }
    if format.contains(ValueFormat::Y_PLACEMENT_DEVICE) {
        if let Some(value) = delta(true, &mut worked) {
            pos.y_offset = pos.y_offset.saturating_add(scale.scale_y_f(value));
        }
    }
    if format.contains(ValueFormat::X_ADVANCE_DEVICE) {
        if let Some(value) = delta(is_horizontal, &mut worked) {
            pos.x_advance = pos.x_advance.saturating_add(scale.scale_x_f(value));
        }
    }
    if format.contains(ValueFormat::Y_ADVANCE_DEVICE) {
        if let Some(value) = delta(!is_horizontal, &mut worked) {
            pos.y_advance = pos.y_advance.saturating_sub(scale.scale_y_f(value));
        }
    }
    worked
}

/// Single adjustment by one value record.
pub(crate) struct SinglePos1<'a> {
    value: ValueRecord<'a>,
}

impl<'a> SinglePos1<'a> {
    pub fn new(table: &SinglePosFormat1<'a>) -> Option<Self> {
        Some(Self {
            value: ValueRecord::new(
                table.offset_data(),
                table.value_record_byte_range().start,
                table.value_format(),
            ),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, _index: u16) -> Option<()> {
        message!(ctx, "positioning glyph at {}", ctx.buffer.idx);
        apply_value(ctx, ctx.buffer.idx, &self.value);
        message!(ctx, "positioned glyph at {}", ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl<'a> SubtableDigests<'a> for SinglePos1<'a> {}

/// Single adjustment by a value record per covered glyph.
pub(crate) struct SinglePos2<'a> {
    /// The first value record.
    values: ValueRecord<'a>,
}

impl<'a> SinglePos2<'a> {
    pub fn new(table: &SinglePosFormat2<'a>) -> Option<Self> {
        Some(Self {
            values: ValueRecord::new(
                table.offset_data(),
                table.value_records_byte_range().start,
                table.value_format(),
            ),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let index = usize::from(index);
        let values = &self.values;
        let value = ValueRecord::new(
            values.offset_data(),
            values.offset() + values.byte_len() * index,
            values.format(),
        );
        message!(ctx, "positioning glyph at {}", ctx.buffer.idx);
        apply_value(ctx, ctx.buffer.idx, &value);
        message!(ctx, "positioned glyph at {}", ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl<'a> SubtableDigests<'a> for SinglePos2<'a> {}

/// The glyph that a pair starting at the current glyph would pair with.
///
/// It depends only on the buffer and the lookup, so the subtables of a pair
/// lookup share one search per position.
#[derive(Clone, Copy)]
pub(crate) enum SecondGlyph {
    Unknown,
    /// No glyph follows; a match would depend on the glyphs up to
    /// `unsafe_to`.
    Missing {
        unsafe_to: usize,
    },
    Found {
        index: usize,
        buf_idx: usize,
        glyph: GlyphId,
    },
    Failed,
}

impl SecondGlyph {
    /// Returns the second glyph's index in the buffer, the index to resume
    /// from after the pair, and the glyph, searching the first time. Without
    /// one, marks the buffer unsafe to concatenate as far as the search went.
    #[inline(always)]
    fn get(&mut self, ctx: &mut ApplyContext) -> Option<(usize, usize, GlyphId)> {
        if let Self::Unknown = self {
            *self = Self::search(ctx);
        }
        self.resolve(ctx)
    }

    /// The second glyph, if the search found one.
    #[inline(always)]
    fn resolve(self, ctx: &mut ApplyContext) -> Option<(usize, usize, GlyphId)> {
        match self {
            Self::Found {
                index,
                buf_idx,
                glyph,
            } => Some((index, buf_idx, glyph)),
            Self::Missing { unsafe_to } => {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
                None
            }
            Self::Unknown | Self::Failed => None,
        }
    }

    #[inline(always)]
    fn search(ctx: &mut ApplyContext) -> Self {
        let mut iter = SkippingIterator::new(ctx, false);
        if iter.reset(iter.buffer.idx).is_none() {
            iter.buffer.index_failed();
        }
        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            return Self::Missing { unsafe_to };
        }
        let index = iter.index();
        let buf_idx = iter.buf_idx;
        match iter.buffer.info.get(index).likely() {
            Some(info) => Self::Found {
                index,
                buf_idx,
                glyph: info.as_glyph(),
            },
            None => Self::Failed,
        }
    }
}

/// Applies the two value records of a pair at `second_index`, then moves
/// past the pair.
#[inline(always)]
fn apply_pair(
    ctx: &mut ApplyContext,
    second_index: usize,
    mut buf_idx: usize,
    value1: &ValueRecord,
    value2: &ValueRecord,
) -> Option<()> {
    message!(
        ctx,
        "try kerning glyphs at {},{second_index}",
        ctx.buffer.idx
    );
    let has_value2 = !value2.is_empty();
    let worked1 = !value1.is_empty() && apply_value(ctx, ctx.buffer.idx, value1);
    let worked2 = has_value2 && apply_value(ctx, second_index, value2);
    if worked1 || worked2 {
        message!(ctx, "kerned glyphs at {},{second_index}", ctx.buffer.idx);
        ctx.buffer
            .unsafe_to_break(Some(ctx.buffer.idx), Some(second_index + 1));
    } else {
        // Even a zero-valued record is a hazard: a different second glyph
        // could select a non-zero one.
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
    }
    message!(
        ctx,
        "tried kerning glyphs at {},{second_index}",
        ctx.buffer.idx
    );
    if has_value2 {
        buf_idx += 1;
        // https://github.com/harfbuzz/harfbuzz/issues/3824
        // https://github.com/harfbuzz/harfbuzz/issues/3888#issuecomment-1326781116
        ctx.buffer
            .unsafe_to_break(Some(ctx.buffer.idx), Some(buf_idx + 1));
    }
    ctx.buffer.idx = buf_idx;
    Some(())
}

/// Pair adjustment by glyph pairs.
pub(crate) struct PairPos1<'a> {
    pair_sets: Offsets<'a>,
    /// The second glyphs of each pair set.
    pair_set_digests: Box<[SetDigest]>,
    format1: ValueFormat,
    format2: ValueFormat,
}

impl<'a> PairPos1<'a> {
    pub fn new(table: &PairPosFormat1<'a>, budget: &Budget) -> Result<Self, Exhausted> {
        let sets = table.pair_sets();
        budget.work(sets.len())?;
        budget.alloc::<SetDigest>(sets.len())?;
        let mut digests = Vec::with_capacity(sets.len());
        for set in sets.iter() {
            digests.push(match set {
                Ok(set) => pair_set_digest(&set, budget)?,
                Err(_) => SetDigest::full(),
            });
        }
        let pair_set_digests = digests.into_boxed_slice();
        Ok(Self {
            pair_sets: Offsets::new(table.offset_data(), table.pair_set_offsets()),
            pair_set_digests,
            format1: table.value_format1(),
            format2: table.value_format2(),
        })
    }

    /// Applies, sharing the search for the second glyph with the lookup's
    /// other subtables.
    #[inline]
    pub fn apply(
        &self,
        ctx: &mut ApplyContext,
        index: u16,
        second: &mut SecondGlyph,
    ) -> Option<()> {
        let index = usize::from(index);
        let (second_index, buf_idx, second) = second.get(ctx)?;
        if self
            .pair_set_digests
            .get(index)
            .is_some_and(|digest| !digest.may_have(second.to_u32()))
        {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
            return None;
        }

        // Binary search the pair set's records, which start with the second
        // glyph, directly in the bytes.
        let data = self.pair_sets.data(index)?;
        let bytes = data.as_bytes();
        let format1_len = self.format1.record_byte_len();
        let record_len = 2 + format1_len + self.format2.record_byte_len();
        let (mut lo, mut hi) = (0, data.read_at::<u16>(0).likely_ok()? as usize);
        while lo < hi {
            // The bounds are 16-bit, so the plain midpoint can't overflow.
            #[allow(clippy::manual_midpoint)]
            let mid = (lo + hi) / 2;
            let offset = 2 + mid * record_len;
            let &[hi_byte, lo_byte] = bytes.get(offset..offset + 2)? else {
                return None;
            };
            let glyph = GlyphId::from(u16::from_be_bytes([hi_byte, lo_byte]));
            if glyph < second {
                lo = mid + 1;
            } else if glyph > second {
                hi = mid;
            } else {
                let value1 = ValueRecord::new(data, offset + 2, self.format1);
                let value2 = ValueRecord::new(data, offset + 2 + format1_len, self.format2);
                return apply_pair(ctx, second_index, buf_idx, &value1, &value2);
            }
        }
        // The same hazard the digest reports, found the long way.
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
        None
    }

    /// The second glyphs of all pairs.
    pub fn seconds(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        budget.work(self.pair_set_digests.len())?;
        let mut seconds = SetDigest::new();
        for digest in &self.pair_set_digests {
            seconds.union(digest);
        }
        Ok(seconds)
    }
}

impl<'a> SubtableDigests<'a> for PairPos1<'a> {
    const CACHES_COVERAGE: bool = true;

    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        self.seconds(budget)
    }
}

fn pair_set_digest(set: &PairSet, budget: &Budget) -> Result<SetDigest, Exhausted> {
    let records = set.pair_value_records();
    budget.work(records.len())?;
    let mut digest = SetDigest::new();
    for record in records.iter() {
        let Ok(record) = record else {
            return Ok(SetDigest::full());
        };
        digest.add(record.second_glyph().to_u32());
    }
    Ok(digest)
}

/// Pair adjustment by glyph classes.
pub(crate) struct PairPos2<'a> {
    class_def1: ClassDef<'a>,
    class_def2: ClassDef<'a>,
    caches: Option<Box<PairPos2Caches>>,
    /// The matrix of value records, by first and second class.
    records: ClassValueRecords<'a>,
}

/// Caches of both classes of a glyph.
struct PairPos2Caches {
    class1: MappingCache,
    class2: MappingCache,
}

impl<'a> PairPos2<'a> {
    pub fn new(
        table: &PairPosFormat2<'a>,
        with_caches: bool,
        budget: &Budget,
    ) -> Result<Option<Self>, Exhausted> {
        if class_matrix_range(table).is_none() {
            return Ok(None);
        }
        if with_caches {
            budget.alloc::<PairPos2Caches>(1)?;
        }
        Ok(Some(Self {
            class_def1: ClassDef::new(table.class_def1()),
            class_def2: ClassDef::new(table.class_def2()),
            caches: with_caches.then(|| {
                Box::new(PairPos2Caches {
                    class1: MappingCache::new(),
                    class2: MappingCache::new(),
                })
            }),
            records: table.class_value_records(),
        }))
    }

    /// Applies, sharing the search for the second glyph with the lookup's
    /// other subtables.
    #[inline]
    pub fn apply(
        &self,
        ctx: &mut ApplyContext,
        _index: u16,
        second: &mut SecondGlyph,
    ) -> Option<()> {
        let first = ctx.buffer.cur(0)?.as_glyph();
        let caches = self.caches.as_deref();
        let (second_index, buf_idx, second) = second.get(ctx)?;
        let class1 = |gid| self.class_def1.get(gid);
        let class2 = |gid| self.class_def2.get(gid);
        let (class1, class2) = match caches {
            Some(caches) => (
                glyph_class_cached(class1, first, &caches.class1),
                glyph_class_cached(class2, second, &caches.class2),
            ),
            None => (class1(first), class2(second)),
        };
        let Some([value1, value2]) = self.records.get(class1, class2) else {
            // A class outside the matrix matches nothing, as in HarfBuzz.
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
            return None;
        };
        apply_pair(ctx, second_index, buf_idx, &value1, &value2)
    }
}

impl<'a> SubtableDigests<'a> for PairPos2<'a> {
    const CACHES_COVERAGE: bool = true;
}

/// The byte range of a class pair subtable's value matrix, or `None` if it
/// does not fit in the subtable.
///
/// The extent is checked here because the generated range helpers add the
/// matrix size to its start unchecked, which overflows on 32-bit targets.
fn class_matrix_range(table: &PairPosFormat2) -> Option<core::ops::Range<usize>> {
    let record_len =
        table.value_format1().record_byte_len() + table.value_format2().record_byte_len();
    let len = usize::from(table.class1_count())
        .checked_mul(usize::from(table.class2_count()))?
        .checked_mul(record_len)?;
    let start = table.class2_count_byte_range().end;
    let end = start.checked_add(len)?;
    (end <= table.offset_data().len()).then_some(start..end)
}

/// Whether a pair subtable only adjusts the advance of its first glyph,
/// which is how kerning is stored.
fn is_kerning(format1: ValueFormat, format2: ValueFormat) -> bool {
    format1 == ValueFormat::X_ADVANCE && format2.is_empty()
}

/// Adds a kerning `value` to the current glyph's advance, then moves on to
/// the second glyph of the pair.
#[inline(always)]
fn apply_kerning(
    ctx: &mut ApplyContext,
    second_index: usize,
    buf_idx: usize,
    value: i16,
) -> Option<()> {
    message!(
        ctx,
        "try kerning glyphs at {},{second_index}",
        ctx.buffer.idx
    );
    let x_advance = ctx.scale.scale_x(i32::from(value));
    let worked = value != 0
        && ctx.buffer.direction.is_horizontal()
        && ctx
            .buffer
            .pos
            .get_mut(ctx.buffer.idx)
            .likely()
            .map(|pos| pos.x_advance = pos.x_advance.saturating_add(x_advance))
            .is_some();
    if worked {
        message!(ctx, "kerned glyphs at {},{second_index}", ctx.buffer.idx);
        ctx.buffer
            .unsafe_to_break(Some(ctx.buffer.idx), Some(second_index + 1));
    } else {
        // Even a zero kern is a hazard: a different second glyph could
        // select a non-zero one.
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
    }
    message!(
        ctx,
        "tried kerning glyphs at {},{second_index}",
        ctx.buffer.idx
    );
    ctx.buffer.idx = buf_idx;
    Some(())
}

/// Kerning by glyph pairs: pair adjustment format 1 whose records hold only
/// an advance for the first glyph.
pub(crate) struct PairKern1<'a> {
    pair_sets: Offsets<'a>,
    /// The second glyphs of each pair set.
    pair_set_digests: Box<[SetDigest]>,
}

impl<'a> PairKern1<'a> {
    /// Returns `None` unless the subtable holds only kerning.
    pub fn new(table: &PairPosFormat1<'a>, budget: &Budget) -> Result<Option<Self>, Exhausted> {
        if !is_kerning(table.value_format1(), table.value_format2()) {
            return Ok(None);
        }
        let pairs = PairPos1::new(table, budget)?;
        Ok(Some(Self {
            pair_sets: pairs.pair_sets,
            pair_set_digests: pairs.pair_set_digests,
        }))
    }

    #[inline]
    pub fn apply(
        &self,
        ctx: &mut ApplyContext,
        index: u16,
        second: &mut SecondGlyph,
    ) -> Option<()> {
        let index = usize::from(index);
        let (second_index, buf_idx, second) = second.get(ctx)?;
        if self
            .pair_set_digests
            .get(index)
            .is_some_and(|digest| !digest.may_have(second.to_u32()))
        {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
            return None;
        }
        // Each record is a second glyph in the high half and its kern in
        // the low half.
        let data = self.pair_sets.data(index)?;
        let count = data.read_at::<u16>(0).likely_ok()? as usize;
        let records: &[BigEndian<u32>] = data.read_array(2..2 + count * 4).ok()?;
        let second = second.to_u32();
        if let Ok(found) = records.binary_search_by_key(&second, |record| record.get() >> 16) {
            let value = records[found].get() as u16 as i16;
            return apply_kerning(ctx, second_index, buf_idx, value);
        }
        // The same hazard the digest reports, found the long way.
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
        None
    }
}

impl<'a> SubtableDigests<'a> for PairKern1<'a> {
    const CACHES_COVERAGE: bool = true;

    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        budget.work(self.pair_set_digests.len())?;
        let mut seconds = SetDigest::new();
        for digest in &self.pair_set_digests {
            seconds.union(digest);
        }
        Ok(seconds)
    }
}

/// Kerning by glyph classes: pair adjustment format 2 whose matrix holds
/// only an advance for the first glyph.
pub(crate) struct PairKern2<'a> {
    class_def1: ClassDef<'a>,
    class_def2: ClassDef<'a>,
    caches: Option<Box<PairPos2Caches>>,
    /// The kern of each first class and second class.
    matrix: &'a [BigEndian<i16>],
    class1_count: u16,
    class2_count: u16,
}

impl<'a> PairKern2<'a> {
    /// Returns `None` unless the subtable holds only kerning, in a matrix
    /// that can be read.
    pub fn new(
        table: &PairPosFormat2<'a>,
        with_caches: bool,
        budget: &Budget,
    ) -> Result<Option<Self>, Exhausted> {
        if !is_kerning(table.value_format1(), table.value_format2()) {
            return Ok(None);
        }
        let Some(range) = class_matrix_range(table) else {
            return Ok(None);
        };
        let Ok(matrix) = table.offset_data().read_array(range) else {
            return Ok(None);
        };
        let Some(pairs) = PairPos2::new(table, with_caches, budget)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            matrix,
            class_def1: pairs.class_def1,
            class_def2: pairs.class_def2,
            caches: pairs.caches,
            class1_count: table.class1_count(),
            class2_count: table.class2_count(),
        }))
    }

    #[inline]
    pub fn apply(
        &self,
        ctx: &mut ApplyContext,
        _index: u16,
        second: &mut SecondGlyph,
    ) -> Option<()> {
        let first = ctx.buffer.cur(0)?.as_glyph();
        let caches = self.caches.as_deref();
        let (second_index, buf_idx, second) = second.get(ctx)?;
        let class1 = |gid| self.class_def1.get(gid);
        let class2 = |gid| self.class_def2.get(gid);
        let (class1, class2) = match caches {
            Some(caches) => (
                glyph_class_cached(class1, first, &caches.class1),
                glyph_class_cached(class2, second, &caches.class2),
            ),
            None => (class1(first), class2(second)),
        };
        if class1 >= self.class1_count || class2 >= self.class2_count {
            // A class outside the matrix matches nothing, as in HarfBuzz.
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_index + 1));
            return None;
        }
        let index = usize::from(class1) * usize::from(self.class2_count) + usize::from(class2);
        let value = self.matrix.get(index)?.get();
        apply_kerning(ctx, second_index, buf_idx, value)
    }
}

impl<'a> SubtableDigests<'a> for PairKern2<'a> {
    const CACHES_COVERAGE: bool = true;
}

/// Cursive attachment.
pub(crate) struct CursivePos1<'a> {
    coverage: Coverage<'a>,
    data: FontData<'a>,
    records: &'a [EntryExitRecord],
}

impl<'a> CursivePos1<'a> {
    pub fn new(table: &CursivePosFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            data: table.offset_data(),
            records: table.entry_exit_record(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let index_this = usize::from(index);
        let entry_this = self
            .records
            .get(index_this)?
            .entry_anchor(self.data)?
            .ok()?;

        let mut iter = SkippingIterator::new(ctx, false);
        iter.reset_fast(iter.buffer.idx);
        let mut unsafe_from = 0;
        if !iter.prev(Some(&mut unsafe_from)) {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(unsafe_from), Some(ctx.buffer.idx + 1));
            return None;
        }
        let i = iter.index();
        let prev = iter.buffer.info.get(i).likely()?.as_glyph();
        let index_prev = self.coverage.get(prev)? as usize;
        let Some(exit_prev) = self
            .records
            .get(index_prev)
            .and_then(|record| record.exit_anchor(self.data).transpose().ok().flatten())
        else {
            iter.buffer
                .unsafe_to_concat_from_outbuffer(Some(i), Some(iter.buffer.idx + 1));
            return None;
        };

        let (exit_x, exit_y) = ctx.layout.ot.resolve_anchor(&exit_prev);
        let (entry_x, entry_y) = ctx.layout.ot.resolve_anchor(&entry_this);
        let (exit_x, exit_y) = (ctx.scale_x(exit_x), ctx.scale_y(exit_y));
        let (entry_x, entry_y) = (ctx.scale_x(entry_x), ctx.scale_y(entry_y));

        let direction = ctx.buffer.direction;
        let j = ctx.buffer.idx;
        ctx.buffer.unsafe_to_break(Some(i), Some(j + 1));

        message!(ctx, "cursive attaching glyph at {} to glyph at {}", i, j);
        let pos = &mut ctx.buffer.pos;
        // Both glyphs change together, so adjust copies and store them back.
        let mut prev_pos = *pos.get(i).likely()?;
        let mut this_pos = *pos.get(j).likely()?;
        match direction {
            Direction::LeftToRight => {
                prev_pos.x_advance = exit_x.saturating_add(prev_pos.x_offset);
                let d = entry_x.saturating_add(this_pos.x_offset);
                this_pos.x_advance = this_pos.x_advance.saturating_sub(d);
                this_pos.x_offset = this_pos.x_offset.saturating_sub(d);
            }
            Direction::RightToLeft => {
                let d = exit_x.saturating_add(prev_pos.x_offset);
                prev_pos.x_advance = prev_pos.x_advance.saturating_sub(d);
                prev_pos.x_offset = prev_pos.x_offset.saturating_sub(d);
                this_pos.x_advance = entry_x.saturating_add(this_pos.x_offset);
            }
            Direction::TopToBottom => {
                prev_pos.y_advance = exit_y.saturating_add(prev_pos.y_offset);
                let d = entry_y.saturating_add(this_pos.y_offset);
                this_pos.y_advance = this_pos.y_advance.saturating_sub(d);
                this_pos.y_offset = this_pos.y_offset.saturating_sub(d);
            }
            Direction::BottomToTop => {
                let d = exit_y.saturating_add(prev_pos.y_offset);
                prev_pos.y_advance = prev_pos.y_advance.saturating_sub(d);
                prev_pos.y_offset = prev_pos.y_offset.saturating_sub(d);
                this_pos.y_advance = entry_y;
            }
            Direction::Invalid => {}
        }
        *pos.get_mut(i).likely()? = prev_pos;
        *pos.get_mut(j).likely()? = this_pos;

        // Attach the child to its parent in the cross direction. The parent
        // stays on the baseline; right to left, the common Arabic case, the
        // later glyph is the parent.
        let (mut child, mut parent) = (i, j);
        let mut x_offset = entry_x.saturating_sub(exit_x);
        let mut y_offset = entry_y.saturating_sub(exit_y);
        // The low 16 bits of the props are the lookup flags.
        if ctx.lookup_props as u16 & lookup_flags::RIGHT_TO_LEFT == 0 {
            core::mem::swap(&mut child, &mut parent);
            x_offset = x_offset.saturating_neg();
            y_offset = y_offset.saturating_neg();
        }

        // If the child was attached elsewhere, reverse that chain so it
        // now hangs off the new parent.
        reverse_cursive_minor_offset(pos, child, direction, parent)?;

        let child_pos = pos.get_mut(child).likely()?;
        child_pos.set_attach_type(attach_type::CURSIVE);
        let chain = parent as isize - child as isize;
        child_pos.set_attach_chain(chain as i16);
        // A distance that doesn't fit the chain field leaves the glyph
        // unattached, as in HarfBuzz.
        if isize::from(child_pos.attach_chain()) != chain {
            child_pos.set_attach_chain(0);
        }
        if direction.is_horizontal() {
            child_pos.y_offset = y_offset;
        } else {
            child_pos.x_offset = x_offset;
        }
        let child_chain = child_pos.attach_chain();
        ctx.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;

        // If the parent was attached to the child, separate them.
        // https://github.com/harfbuzz/harfbuzz/issues/2469
        let parent_pos = ctx.buffer.pos.get_mut(parent).likely()?;
        if parent_pos.attach_chain() == -child_chain {
            parent_pos.set_attach_chain(0);
            if direction.is_horizontal() {
                parent_pos.y_offset = 0;
            } else {
                parent_pos.x_offset = 0;
            }
        }
        message!(ctx, "cursive attached glyph at {} to glyph at {}", i, j);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl<'a> SubtableDigests<'a> for CursivePos1<'a> {
    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        self.coverage.digest(budget)
    }
}

/// Reverses the cursive chain from glyph `i` so that it ends at
/// `new_parent`. Returns `None` only if `i` is outside `pos`.
fn reverse_cursive_minor_offset(
    pos: &mut [GlyphPosition],
    i: usize,
    direction: Direction,
    new_parent: usize,
) -> Option<()> {
    let this = pos.get_mut(i).likely()?;
    let chain = this.attach_chain();
    let attach_type = this.attach_type();
    if chain == 0 || attach_type & attach_type::CURSIVE == 0 {
        return Some(());
    }
    this.set_attach_chain(0);
    let j = (i as isize + isize::from(chain)) as usize;
    // A truncated chain can point outside the buffer, and the chain stops
    // at the new parent.
    if j >= pos.len() || j == new_parent {
        return Some(());
    }
    reverse_cursive_minor_offset(pos, j, direction, new_parent)?;
    // Read this glyph after the recursion, which a looping chain can
    // rewrite.
    let this = pos.get(i).likely()?;
    let offset = if direction.is_horizontal() {
        this.y_offset
    } else {
        this.x_offset
    };
    let next = pos.get_mut(j).likely()?;
    if direction.is_horizontal() {
        next.y_offset = offset.saturating_neg();
    } else {
        next.x_offset = offset.saturating_neg();
    }
    next.set_attach_chain(-chain);
    next.set_attach_type(attach_type);
    Some(())
}

/// Attaches the current glyph, a mark, to the glyph at `base_pos`.
fn attach_mark(
    ctx: &mut ApplyContext,
    base_anchor: &AnchorTable,
    mark_anchor: &AnchorTable,
    base_pos: usize,
) -> Option<()> {
    let (base_x, base_y) = ctx.layout.ot.resolve_anchor(base_anchor);
    let (mark_x, mark_y) = ctx.layout.ot.resolve_anchor(mark_anchor);
    let x_offset = ctx.scale_x(base_x - mark_x);
    let y_offset = ctx.scale_y(base_y - mark_y);
    // Resolve the base first so a bad index leaves the buffer untouched.
    let base_offset = cross_offset(&ctx.buffer.pos, base_pos, ctx.buffer.direction)?;
    ctx.buffer
        .unsafe_to_break(Some(base_pos), Some(ctx.buffer.idx + 1));
    let horizontal = ctx.buffer.direction.is_horizontal();
    let idx = ctx.buffer.idx;
    message!(
        ctx,
        "attaching mark glyph at {} to glyph at {}",
        idx,
        base_pos
    );
    // A distance that doesn't fit the chain field leaves the mark
    // unattached, as in HarfBuzz.
    let Ok(chain) = i16::try_from(base_pos as isize - idx as isize) else {
        ctx.buffer.cur_pos_mut()?.set_attach_chain(0);
        ctx.buffer.idx += 1;
        return Some(());
    };
    let pos = ctx.buffer.cur_pos_mut()?;
    pos.x_offset = x_offset;
    pos.y_offset = y_offset;
    if horizontal {
        pos.y_offset = pos.y_offset.saturating_add(base_offset);
    } else {
        pos.x_offset = pos.x_offset.saturating_add(base_offset);
    }
    pos.set_attach_type(attach_type::MARK);
    pos.set_attach_chain(chain);
    message!(
        ctx,
        "attached mark glyph at {} to glyph at {}",
        idx,
        base_pos
    );
    ctx.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
    ctx.buffer.idx += 1;
    Some(())
}

/// The cross-direction offset of the glyph at `index`, summed along its
/// cursive chain.
fn cross_offset(pos: &[GlyphPosition], mut index: usize, direction: Direction) -> Option<i32> {
    let cross = |p: &GlyphPosition| {
        if direction.is_horizontal() {
            p.y_offset
        } else {
            p.x_offset
        }
    };
    let mut this = pos.get(index).likely()?;
    let mut offset = cross(this);
    while this.attach_type() & attach_type::CURSIVE != 0 && this.attach_chain() != 0 {
        // Stop at a parent outside the buffer.
        let Some(parent) = index
            .checked_add_signed(isize::from(this.attach_chain()))
            .and_then(|parent| Some((parent, pos.get(parent)?)))
        else {
            break;
        };
        (index, this) = parent;
        offset = offset.saturating_add(cross(this));
    }
    Some(offset)
}

/// Searches back from the current glyph for the base a mark attaches to,
/// remembering the result for the marks that follow.
fn find_mark_base(
    ctx: &mut ApplyContext,
    accept: fn(&Buffer, usize) -> Option<bool>,
    coverage: &Coverage,
) -> Option<usize> {
    if ctx.last_base_until > ctx.buffer.idx as u32 {
        ctx.last_base_until = 0;
        ctx.last_base = -1;
    }
    let last_base_until = ctx.last_base_until;
    let mut last_base = ctx.last_base;
    // Search without skippy_iter.prev() to avoid O(n^2) behavior.
    let mut iter = SkippingIterator::new(ctx, false);
    iter.set_lookup_props(u32::from(lookup_flags::IGNORE_MARKS));
    let mut j = iter.buffer.idx;
    while j > last_base_until as usize {
        let mut matched = iter.match_at(j - 1);
        // https://github.com/harfbuzz/harfbuzz/issues/4124
        if matched == MatchResult::Match
            && accept(iter.buffer, j - 1) != Some(true)
            && iter
                .buffer
                .info
                .get(j - 1)
                .is_none_or(|info| coverage.get(info.as_glyph()).is_none())
        {
            matched = MatchResult::Skip;
        }
        if matched == MatchResult::Match {
            last_base = j as i32 - 1;
            break;
        }
        j -= 1;
    }
    ctx.last_base_until = ctx.buffer.idx as u32;
    ctx.last_base = last_base;
    if last_base == -1 {
        ctx.buffer
            .unsafe_to_concat_from_outbuffer(Some(0), Some(ctx.buffer.idx + 1));
        return None;
    }
    Some(last_base as usize)
}

/// Whether a mark may attach to the base at `idx`: only the first glyph of
/// a multiple substitution is a base, unless a mark breaks the sequence.
/// https://github.com/harfbuzz/harfbuzz/issues/740
/// https://github.com/harfbuzz/harfbuzz/issues/1020
fn accept_base(buffer: &Buffer, idx: usize) -> Option<bool> {
    let info = buffer.info.get(idx).likely()?;
    if !info.multiplied() || info.lig_comp() == 0 || idx == 0 {
        return Some(true);
    }
    let prev = buffer.info.get(idx - 1).likely()?;
    Some(
        prev.is_mark()
            || !prev.multiplied()
            || info.lig_id() != prev.lig_id()
            || info.lig_comp() != prev.lig_comp() + 1,
    )
}

/// Whether a mark may attach to the ligature at `idx`: the first glyph of
/// a multiple substitution may have been ligated with what precedes it.
/// https://github.com/harfbuzz/harfbuzz/issues/4969
fn accept_ligature(buffer: &Buffer, idx: usize) -> Option<bool> {
    let info = buffer.info.get(idx).likely()?;
    Some(!info.multiplied() || info.lig_comp() == 0)
}

/// The anchor of the mark at `index` and its class.
fn mark_anchor<'a>(marks: &MarkArray<'a>, index: usize) -> Option<(AnchorTable<'a>, usize)> {
    let record = marks.mark_records().get(index)?;
    let anchor = record.mark_anchor(marks.offset_data()).ok()?;
    Some((anchor, record.mark_class() as usize))
}

/// Mark-to-base attachment.
pub(crate) struct MarkBasePos1<'a> {
    base_coverage: Coverage<'a>,
    marks: Option<MarkArray<'a>>,
    bases: Option<BaseArray<'a>>,
}

impl<'a> MarkBasePos1<'a> {
    pub fn new(table: &MarkBasePosFormat1<'a>) -> Option<Self> {
        Some(Self {
            base_coverage: Coverage::new(table.base_coverage()),
            marks: table.mark_array().ok(),
            bases: table.base_array().ok(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let mark_index = usize::from(index);
        let base_coverage = self.base_coverage.present()?;
        let base_pos = find_mark_base(ctx, accept_base, base_coverage)?;
        // Checking the base's GDEF class is too strict, so it isn't done.
        let base = ctx.buffer.info.get(base_pos).likely()?.as_glyph();
        let Some(base_index) = base_coverage.get(base) else {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(base_pos), Some(ctx.buffer.idx + 1));
            return None;
        };
        let (mark_anchor, class) = mark_anchor(self.marks.as_ref()?, mark_index)?;
        let bases = self.bases.as_ref()?;
        let base_record = bases.base_records().get(base_index as usize).likely_ok()?;
        let base_anchor = base_record
            .base_anchors(bases.offset_data())
            .get(class)?
            .ok()?;
        attach_mark(ctx, &base_anchor, &mark_anchor, base_pos)
    }
}

impl<'a> SubtableDigests<'a> for MarkBasePos1<'a> {
    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        self.base_coverage.digest(budget)
    }
}

/// Mark-to-ligature attachment.
pub(crate) struct MarkLigPos1<'a> {
    ligature_coverage: Coverage<'a>,
    marks: Option<MarkArray<'a>>,
    ligatures: Option<LigatureArray<'a>>,
}

impl<'a> MarkLigPos1<'a> {
    pub fn new(table: &MarkLigPosFormat1<'a>) -> Option<Self> {
        Some(Self {
            ligature_coverage: Coverage::new(table.ligature_coverage()),
            marks: table.mark_array().ok(),
            ligatures: table.ligature_array().ok(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let mark_index = usize::from(index);
        let ligature_coverage = self.ligature_coverage.present()?;
        let lig_pos = find_mark_base(ctx, accept_ligature, ligature_coverage)?;
        // Checking the ligature's GDEF class is too strict, so it isn't done.
        let lig_info = ctx.buffer.info.get(lig_pos).likely()?;
        let lig_id = lig_info.lig_id();
        let Some(lig_index) = ligature_coverage.get(lig_info.as_glyph()) else {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(lig_pos), Some(ctx.buffer.idx + 1));
            return None;
        };
        let lig_attach = self
            .ligatures
            .as_ref()?
            .ligature_attaches()
            .get(lig_index as usize)
            .ok()?;
        let comp_count = lig_attach.component_count();
        if comp_count == 0 {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(lig_pos), Some(ctx.buffer.idx + 1));
            return None;
        }
        // A mark from the same ligature attaches to its own component;
        // otherwise it attaches to the last one.
        let mark_id = ctx.buffer.cur(0)?.lig_id();
        let mark_comp = u16::from(ctx.buffer.cur(0)?.lig_comp());
        let comp_index = if lig_id != 0 && lig_id == mark_id && mark_comp > 0 {
            mark_comp.min(comp_count)
        } else {
            comp_count
        } - 1;
        let (mark_anchor, class) = mark_anchor(self.marks.as_ref()?, mark_index)?;
        let component = lig_attach
            .component_records()
            .get(comp_index as usize)
            .ok()?;
        let base_anchor = component
            .ligature_anchors(lig_attach.offset_data())
            .get(class)?
            .ok()?;
        attach_mark(ctx, &base_anchor, &mark_anchor, lig_pos)
    }
}

impl<'a> SubtableDigests<'a> for MarkLigPos1<'a> {
    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        self.ligature_coverage.digest(budget)
    }
}

/// Mark-to-mark attachment.
pub(crate) struct MarkMarkPos1<'a> {
    mark2_coverage: Coverage<'a>,
    marks1: Option<MarkArray<'a>>,
    marks2: Option<Mark2Array<'a>>,
}

impl<'a> MarkMarkPos1<'a> {
    pub fn new(table: &MarkMarkPosFormat1<'a>) -> Option<Self> {
        Some(Self {
            mark2_coverage: Coverage::new(table.mark2_coverage()),
            marks1: table.mark1_array().ok(),
            marks2: table.mark2_array().ok(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let mark1_index = usize::from(index);
        // Search back for a mark, stopping at the first glyph that isn't one.
        let lookup_props = ctx.lookup_props;
        let mut iter = SkippingIterator::new(ctx, false);
        iter.reset_fast(iter.buffer.idx);
        iter.set_lookup_props(lookup_props & !u32::from(lookup_flags::IGNORE_FLAGS));
        let mut unsafe_from = 0;
        if !iter.prev(Some(&mut unsafe_from)) {
            iter.buffer
                .unsafe_to_concat_from_outbuffer(Some(unsafe_from), Some(iter.buffer.idx + 1));
            return None;
        }
        let mark2_pos = iter.index();
        let mark2 = *ctx.buffer.info.get(mark2_pos).likely()?;
        if !mark2.is_mark() {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(mark2_pos), Some(ctx.buffer.idx + 1));
            return None;
        }
        let cur = ctx.buffer.cur(0)?;
        let (id1, comp1) = (cur.lig_id(), cur.lig_comp());
        let (id2, comp2) = (mark2.lig_id(), mark2.lig_comp());
        let matches = if id1 == id2 {
            // Marks of the same base, or of the same ligature component.
            id1 == 0 || comp1 == comp2
        } else {
            // One of the marks may itself be a ligature.
            (id1 > 0 && comp1 == 0) || (id2 > 0 && comp2 == 0)
        };
        if !matches {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(mark2_pos), Some(ctx.buffer.idx + 1));
            return None;
        }
        let mark2_index = self.mark2_coverage.get(mark2.as_glyph())? as usize;
        let (mark1_anchor, class) = mark_anchor(self.marks1.as_ref()?, mark1_index)?;
        let marks2 = self.marks2.as_ref()?;
        let mark2_record = marks2.mark2_records().get(mark2_index).likely_ok()?;
        let mark2_anchor = mark2_record
            .mark2_anchors(marks2.offset_data())
            .get(class)?
            .ok()?;
        attach_mark(ctx, &mark2_anchor, &mark1_anchor, mark2_pos)
    }
}

impl<'a> SubtableDigests<'a> for MarkMarkPos1<'a> {
    fn digest_second(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        self.mark2_coverage.digest(budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_pair_sets_charge_every_record_visit() {
        // Three offsets alias one pair set of four records. Value records
        // are empty, so each pair record consists only of its second glyph.
        let words: [u16; 13] = [1, 0, 0, 0, 3, 16, 16, 16, 4, 10, 20, 30, 40];
        let data: Vec<_> = words.into_iter().flat_map(u16::to_be_bytes).collect();
        let table = PairPosFormat1::read(FontData::new(&data)).unwrap();
        assert!(matches!(
            PairPos1::new(&table, &Budget::new(14, 1024)),
            Err(Exhausted)
        ));
        assert!(PairPos1::new(&table, &Budget::new(15, 1024)).is_ok());
    }
    use alloc::vec;
    use read_fonts::FontRead;

    /// A kerning class pair subtable claiming a 65535 by 65535 matrix while
    /// holding one value, which overflows the matrix size on 32-bit targets.
    #[test]
    fn oversized_class_matrix_is_rejected() {
        let words: [u16; 14] = [
            2, 18, 4, 0, 24, 24, 65535, 65535,  // header
            0xffce, // the only value
            1, 1, 36, // coverage of glyph 36
            2, 0, // an empty class definition
        ];
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        let table = PairPosFormat2::read(FontData::new(&bytes)).unwrap();
        assert!(class_matrix_range(&table).is_none());
        assert!(PairKern2::new(&table, false, &Budget::new(0, 0))
            .unwrap()
            .is_none());
        assert!(PairPos2::new(&table, false, &Budget::new(0, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn reverse_cursive_minor_offset_ignores_out_of_range_chain() {
        // A truncated attach_chain can make `i + chain` point past the end of
        // the buffer; reverse_cursive_minor_offset must not index out of
        // bounds. See https://github.com/harfbuzz/harfrust/issues/410.
        let mut pos = vec![GlyphPosition::default(); 4];
        pos[0].set_attach_type(attach_type::CURSIVE);
        pos[0].set_attach_chain(30000); // 0 + 30000 is far past the buffer end

        reverse_cursive_minor_offset(&mut pos, 0, Direction::LeftToRight, 3);

        // The out-of-range link is cleared and no other glyph is touched.
        assert_eq!(pos[0].attach_chain(), 0);
    }

    #[test]
    fn reverse_cursive_minor_offset_saturates_negation() {
        let mut pos = vec![GlyphPosition::default(); 2];
        pos[0].set_attach_type(attach_type::CURSIVE);
        pos[0].set_attach_chain(1);
        pos[0].y_offset = i32::MIN;

        reverse_cursive_minor_offset(&mut pos, 0, Direction::LeftToRight, usize::MAX);

        assert_eq!(pos[1].y_offset, i32::MAX);
    }
}
