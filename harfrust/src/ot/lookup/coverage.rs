//! Coverage and class definition tables, reduced to the arrays they search.

use crate::budget::{Budget, Exhausted};
use crate::ot::layout::MAX_CONTEXT_LENGTH;
use crate::set_digest::SetDigest;
use alloc::boxed::Box;
use core::cmp::Ordering;
use read_fonts::tables::layout::{self, ClassRangeRecord, CoverageTable, RangeRecord};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16, Offset16};
use read_fonts::{FontData, ReadError, ResolveOffset};

/// A coverage per context glyph, parsed eagerly only for short contexts.
///
/// Arrays longer than [`MAX_CONTEXT_LENGTH`] are read on demand. Overlapping
/// subtables can otherwise multiply a small font into large allocations and
/// parsing work.
pub(crate) enum CoverageArray<'a> {
    Cached(Box<[Coverage<'a>]>),
    Uncached {
        data: FontData<'a>,
        offsets: &'a [BigEndian<Offset16>],
    },
}

impl Default for CoverageArray<'_> {
    fn default() -> Self {
        Self::Cached(Box::default())
    }
}

impl<'a> CoverageArray<'a> {
    pub fn new(
        data: FontData<'a>,
        offsets: &'a [BigEndian<Offset16>],
        budget: &Budget,
    ) -> Result<Self, Exhausted> {
        if offsets.len() > MAX_CONTEXT_LENGTH {
            return Ok(Self::Uncached { data, offsets });
        }
        budget.work(offsets.len())?;
        budget.alloc::<Coverage>(offsets.len())?;
        Ok(Self::Cached(
            offsets
                .iter()
                .map(|offset| Coverage::new(offset.get().resolve(data)))
                .collect(),
        ))
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Cached(coverages) => coverages.len(),
            Self::Uncached { offsets, .. } => offsets.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Missing or unreadable coverages cover no glyphs, in either form.
    #[inline]
    pub fn get(&self, index: usize) -> Coverage<'a> {
        match self {
            Self::Cached(coverages) => coverages.get(index).copied().unwrap_or_default(),
            Self::Uncached { data, offsets } => {
                offsets.get(index).map_or(Coverage::None, |offset| {
                    Coverage::new(offset.get().resolve(*data))
                })
            }
        }
    }
}

/// The position of `gid` among sorted ranges of glyphs.
#[inline]
fn find_range<T>(
    ranges: &[T],
    gid: GlyphId16,
    bounds: impl Fn(&T) -> (GlyphId16, GlyphId16),
) -> Option<&T> {
    ranges
        .binary_search_by(|range| {
            let (start, end) = bounds(range);
            if end < gid {
                Ordering::Less
            } else if start > gid {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .ok()
        .map(|index| &ranges[index])
}

/// A coverage table: the glyphs it covers, each with a coverage index.
#[derive(Clone, Copy, Default)]
pub(crate) enum Coverage<'a> {
    /// A null or unreadable table, which covers nothing.
    #[default]
    None,
    /// Sorted glyphs, each covered at its position.
    Glyphs(&'a [BigEndian<GlyphId16>]),
    /// Sorted glyph ranges, each with the index of its first glyph.
    Ranges(&'a [RangeRecord]),
}

impl<'a> Coverage<'a> {
    pub fn new(table: Result<CoverageTable<'a>, ReadError>) -> Self {
        match table {
            Ok(CoverageTable::Format1(table)) => Self::Glyphs(table.glyph_array()),
            Ok(CoverageTable::Format2(table)) => Self::Ranges(table.range_records()),
            Err(_) => Self::None,
        }
    }

    /// A coverage that must be present, or `None` if it can't be read.
    pub fn required(table: Result<CoverageTable<'a>, ReadError>) -> Option<Self> {
        Some(Self::new(table)).filter(|coverage| !coverage.is_none())
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The coverage, or `None` if the table is missing.
    pub fn present(&self) -> Option<&Self> {
        (!self.is_none()).then_some(self)
    }

    /// The coverage index of `gid`, if it is covered.
    #[inline(always)]
    pub fn get(&self, gid: impl Into<GlyphId>) -> Option<u16> {
        let gid = GlyphId16::try_from(gid.into()).ok()?;
        match self {
            Self::None => None,
            Self::Glyphs(glyphs) => glyphs
                .binary_search(&gid.into())
                .ok()
                .map(|index| index as u16),
            Self::Ranges(ranges) => {
                let range = find_range(ranges, gid, |r| (r.start_glyph_id(), r.end_glyph_id()))?;
                // Subtract first so the index can't overflow on the way.
                range
                    .start_coverage_index()
                    .checked_add(gid.to_u16() - range.start_glyph_id().to_u16())
            }
        }
    }

    pub fn covers(&self, gid: impl Into<GlyphId>) -> bool {
        self.get(gid).is_some()
    }

    /// The glyphs covered, or every glyph if the table is missing.
    pub fn digest(&self, budget: &Budget) -> Result<SetDigest, Exhausted> {
        let mut digest = SetDigest::new();
        match self {
            Self::None => return Ok(SetDigest::full()),
            Self::Glyphs(glyphs) => {
                budget.work(glyphs.len())?;
                for glyph in *glyphs {
                    digest.add(glyph.get().into());
                }
            }
            Self::Ranges(ranges) => {
                budget.work(ranges.len())?;
                for range in *ranges {
                    digest.add_range(range.start_glyph_id().into(), range.end_glyph_id().into());
                }
            }
        }
        Ok(digest)
    }
}

/// A class definition table: the class of each glyph, 0 if unlisted.
#[derive(Clone, Copy, Default)]
pub(crate) enum ClassDef<'a> {
    /// A null or unreadable table, which puts every glyph in class 0.
    #[default]
    None,
    /// The classes of consecutive glyphs from `start`.
    Array {
        start: u16,
        classes: &'a [BigEndian<u16>],
    },
    /// Sorted glyph ranges, each with a class.
    Ranges(&'a [ClassRangeRecord]),
}

impl<'a> ClassDef<'a> {
    pub fn new(table: Result<layout::ClassDef<'a>, ReadError>) -> Self {
        match table {
            Ok(layout::ClassDef::Format1(table)) => Self::Array {
                start: table.start_glyph_id().to_u16(),
                classes: table.class_value_array(),
            },
            Ok(layout::ClassDef::Format2(table)) => Self::Ranges(table.class_range_records()),
            Err(_) => Self::None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The class of `gid`.
    #[inline(always)]
    pub fn get(&self, gid: impl Into<GlyphId>) -> u16 {
        let gid = gid.into();
        match self {
            Self::None => 0,
            Self::Array { start, classes } => gid
                .to_u32()
                .checked_sub(u32::from(*start))
                .and_then(|index| classes.get(index as usize))
                .map_or(0, |class| class.get()),
            Self::Ranges(ranges) => GlyphId16::try_from(gid)
                .ok()
                .and_then(|gid| find_range(ranges, gid, |r| (r.start_glyph_id(), r.end_glyph_id())))
                .map_or(0, |range| range.class()),
        }
    }

    /// The cost of looking up a glyph, which decides which subtable gets
    /// the glyph-info class cache.
    pub fn cost(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Array { .. } => 1,
            Self::Ranges(ranges) => u32::BITS - (ranges.len() as u32).leading_zeros(),
        }
    }

    /// Adds the glyphs of `class` to `digest`. Class 0 holds every glyph not
    /// listed, so it adds all of them.
    pub fn add_class_to(
        &self,
        digest: &mut SetDigest,
        class: u16,
        budget: &Budget,
    ) -> Result<(), Exhausted> {
        if class == 0 {
            *digest = SetDigest::full();
            return Ok(());
        }
        match self {
            Self::None => {}
            Self::Array { start, classes } => {
                budget.work(classes.len())?;
                for (index, value) in classes.iter().enumerate() {
                    if value.get() == class {
                        digest.add(u32::from(start.saturating_add(index as u16)));
                    }
                }
            }
            Self::Ranges(ranges) => {
                budget.work(ranges.len())?;
                for range in *ranges {
                    if range.class() == class {
                        budget.work(
                            usize::from(range.end_glyph_id().to_u16())
                                .saturating_sub(usize::from(range.start_glyph_id().to_u16()))
                                + 1,
                        )?;
                        for glyph in range.start_glyph_id().to_u16()..=range.end_glyph_id().to_u16()
                        {
                            digest.add(u32::from(glyph));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{vec, vec::Vec};

    #[test]
    fn class_range_expansion_is_charged() {
        use read_fonts::FontRead;
        let words: [u16; 5] = [2, 1, 0, u16::MAX, 1];
        let bytes: Vec<_> = words.into_iter().flat_map(u16::to_be_bytes).collect();
        let class = ClassDef::new(layout::ClassDef::read(FontData::new(&bytes)));
        let mut digest = SetDigest::new();
        assert_eq!(
            class.add_class_to(&mut digest, 1, &Budget::new(10, 0)),
            Err(Exhausted)
        );
        assert!(!digest.may_have(0));
        class
            .add_class_to(&mut digest, 1, &Budget::new(65_537, 0))
            .unwrap();
        assert!(digest.may_have(u32::from(u16::MAX)));
    }

    #[test]
    fn coverage_arrays_keep_long_contexts_without_parsing_every_offset() {
        // Two format 1 coverages, following two padding bytes.
        let data = FontData::new(&[0, 0, 0, 1, 0, 1, 0, 42, 0, 1, 0, 1, 0, 43]);
        for count in [MAX_CONTEXT_LENGTH, MAX_CONTEXT_LENGTH + 1, 65_535] {
            // Many entries can alias the same coverage in a tiny font.
            let mut offsets = vec![Offset16::new(2).into(); count];
            offsets[count - 1] = Offset16::new(8).into();
            let coverages =
                CoverageArray::new(data, &offsets, &Budget::for_table_bytes(data.len())).unwrap();
            assert_eq!(coverages.len(), count);
            assert!(!coverages.is_empty());
            assert_eq!(coverages.get(0).get(42u32), Some(0));
            assert_eq!(coverages.get(count - 1).get(43u32), Some(0));
            assert!(!coverages.get(count - 1).covers(42u32));
            assert!(coverages.get(count).is_none());
            assert_eq!(
                matches!(coverages, CoverageArray::Cached(_)),
                count <= MAX_CONTEXT_LENGTH,
            );
        }
    }

    #[test]
    fn cached_and_uncached_arrays_handle_unreadable_coverages_the_same_way() {
        let data = FontData::new(&[0, 0, 0, 1, 0, 1, 0, 42]);
        for count in [MAX_CONTEXT_LENGTH, MAX_CONTEXT_LENGTH + 1] {
            let mut offsets = vec![Offset16::new(2).into(); count];
            // Null, out of bounds, and an unsupported coverage format.
            for (index, offset) in [0, u16::MAX, 3].into_iter().enumerate() {
                offsets[index] = Offset16::new(offset).into();
            }
            let coverages =
                CoverageArray::new(data, &offsets, &Budget::for_table_bytes(data.len())).unwrap();
            for index in [0, 1, 2, count] {
                let coverage = coverages.get(index);
                assert!(coverage.is_none());
                assert!(!coverage.covers(42u32));
                assert!(coverage.digest(&Budget::new(0, 0)).unwrap().may_have(42));
            }
            assert_eq!(coverages.get(3).get(42u32), Some(0));
        }
    }

    #[test]
    fn empty_coverage_arrays_cover_nothing() {
        for coverages in [
            CoverageArray::default(),
            CoverageArray::new(FontData::new(&[]), &[], &Budget::new(0, 0)).unwrap(),
        ] {
            assert!(coverages.is_empty());
            assert_eq!(coverages.len(), 0);
            assert!(coverages.get(0).is_none());
        }
    }
}
