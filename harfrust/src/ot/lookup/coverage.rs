//! Coverage and class definition tables, reduced to the arrays they search.

use crate::set_digest::SetDigest;
use core::cmp::Ordering;
use read_fonts::tables::layout::{self, ClassRangeRecord, CoverageTable, RangeRecord};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16};
use read_fonts::ReadError;

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
    pub fn digest(&self) -> SetDigest {
        let mut digest = SetDigest::new();
        match self {
            Self::None => return SetDigest::full(),
            Self::Glyphs(glyphs) => {
                for glyph in *glyphs {
                    digest.add(glyph.get().into());
                }
            }
            Self::Ranges(ranges) => {
                for range in *ranges {
                    digest.add_range(range.start_glyph_id().into(), range.end_glyph_id().into());
                }
            }
        }
        digest
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
    pub fn add_class_to(&self, digest: &mut SetDigest, class: u16) {
        if class == 0 {
            *digest = SetDigest::full();
            return;
        }
        match self {
            Self::None => {}
            Self::Array { start, classes } => {
                for (index, value) in classes.iter().enumerate() {
                    if value.get() == class {
                        digest.add(u32::from(start.saturating_add(index as u16)));
                    }
                }
            }
            Self::Ranges(ranges) => {
                for range in *ranges {
                    if range.class() == class {
                        for glyph in range.start_glyph_id().to_u16()..=range.end_glyph_id().to_u16()
                        {
                            digest.add(u32::from(glyph));
                        }
                    }
                }
            }
        }
    }
}
