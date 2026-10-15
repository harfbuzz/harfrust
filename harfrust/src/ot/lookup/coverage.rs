//! Coverage and class definition tables, reduced to the arrays they search.

use crate::set_digest::SetDigest;
use core::cmp::Ordering;
use read_fonts::tables::layout::{
    self, ClassRangeRecord, ClassRangeRecord2, CoverageTable, RangeRecord, RangeRecord2,
};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16, GlyphId24, Uint24};
use read_fonts::ReadError;

/// The position of `gid` among sorted ranges of glyphs.
#[inline]
fn find_range<T, G: PartialOrd>(ranges: &[T], gid: G, bounds: impl Fn(&T) -> (G, G)) -> Option<&T> {
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
    /// Sorted 24-bit glyphs.
    Glyphs24(&'a [BigEndian<GlyphId24>]),
    /// Sorted 24-bit glyph ranges.
    Ranges24(&'a [RangeRecord2]),
}

impl<'a> Coverage<'a> {
    pub fn new(table: Result<CoverageTable<'a>, ReadError>) -> Self {
        match table {
            Ok(CoverageTable::Format1(table)) => Self::Glyphs(table.glyph_array()),
            Ok(CoverageTable::Format2(table)) => Self::Ranges(table.range_records()),
            Ok(CoverageTable::Format3(table)) => Self::Glyphs24(table.glyph_array()),
            Ok(CoverageTable::Format4(table)) => Self::Ranges24(table.range_records()),
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
    pub fn get(&self, gid: impl Into<GlyphId>) -> Option<u32> {
        let gid = gid.into();
        match self {
            Self::None => None,
            Self::Glyphs(glyphs) => {
                let gid = GlyphId16::try_from(gid).ok()?;
                glyphs
                    .binary_search(&gid.into())
                    .ok()
                    .map(|index| index as u32)
            }
            Self::Ranges(ranges) => {
                let gid = GlyphId16::try_from(gid).ok()?;
                let range = find_range(ranges, gid, |r| (r.start_glyph_id(), r.end_glyph_id()))?;
                // Subtract first so the index can't overflow on the way.
                range
                    .start_coverage_index()
                    .checked_add(gid.to_u16() - range.start_glyph_id().to_u16())
                    .map(u32::from)
            }
            Self::Glyphs24(_) | Self::Ranges24(_) => self.get_wide(gid),
        }
    }

    // The 24-bit formats share one call, which keeps the inlined lookups
    // of the original formats as small as they were.
    #[inline(never)]
    fn get_wide(&self, gid: GlyphId) -> Option<u32> {
        match self {
            Self::Glyphs24(glyphs) => glyphs24_index(glyphs, gid),
            Self::Ranges24(ranges) => ranges24_index(ranges, gid),
            _ => None,
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
            Self::Glyphs24(glyphs) => {
                for glyph in *glyphs {
                    digest.add(glyph.get().to_u32());
                }
            }
            Self::Ranges24(ranges) => {
                for range in *ranges {
                    digest.add_range(
                        range.start_glyph_id().to_u32(),
                        range.end_glyph_id().to_u32(),
                    );
                }
            }
        }
        digest
    }
}

fn glyphs24_index(glyphs: &[BigEndian<GlyphId24>], gid: GlyphId) -> Option<u32> {
    let gid = GlyphId24::checked_new(gid.to_u32())?;
    glyphs
        .binary_search(&gid.into())
        .ok()
        .map(|index| index as u32)
}

fn ranges24_index(ranges: &[RangeRecord2], gid: GlyphId) -> Option<u32> {
    let gid = gid.to_u32();
    let range = find_range(ranges, gid, |r| {
        (r.start_glyph_id().to_u32(), r.end_glyph_id().to_u32())
    })?;
    let index = range
        .start_coverage_index()
        .to_u32()
        .checked_add(gid - range.start_glyph_id().to_u32())?;
    (index <= Uint24::MAX.to_u32()).then_some(index)
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
    /// The 24-bit classes of consecutive glyphs from a 24-bit `start`.
    Array24 {
        start: u32,
        classes: &'a [BigEndian<Uint24>],
    },
    /// Sorted 24-bit glyph ranges.
    Ranges24(&'a [ClassRangeRecord2]),
}

impl<'a> ClassDef<'a> {
    pub fn new(table: Result<layout::ClassDef<'a>, ReadError>) -> Self {
        match table {
            Ok(layout::ClassDef::Format1(table)) => Self::Array {
                start: table.start_glyph_id().to_u16(),
                classes: table.class_value_array(),
            },
            Ok(layout::ClassDef::Format2(table)) => Self::Ranges(table.class_range_records()),
            Ok(layout::ClassDef::Format3(table)) => Self::Array24 {
                start: table.start_glyph_id().to_u32(),
                classes: table.class_value_array(),
            },
            Ok(layout::ClassDef::Format4(table)) => Self::Ranges24(table.class_range_records()),
            Err(_) => Self::None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The class of `gid`.
    #[inline(always)]
    pub fn get(&self, gid: impl Into<GlyphId>) -> u32 {
        let gid = gid.into();
        match self {
            Self::None => 0,
            Self::Array { start, classes } => gid
                .to_u32()
                .checked_sub(u32::from(*start))
                .and_then(|index| classes.get(index as usize))
                .map_or(0, |class| class.get().into()),
            Self::Ranges(ranges) => GlyphId16::try_from(gid)
                .ok()
                .and_then(|gid| find_range(ranges, gid, |r| (r.start_glyph_id(), r.end_glyph_id())))
                .map_or(0, |range| range.class().into()),
            Self::Array24 { .. } | Self::Ranges24(_) => self.get_wide(gid),
        }
    }

    #[inline(never)]
    fn get_wide(&self, gid: GlyphId) -> u32 {
        match self {
            Self::Array24 { start, classes } => array24_class(*start, classes, gid),
            Self::Ranges24(ranges) => ranges24_class(ranges, gid),
            _ => 0,
        }
    }

    /// The cost of looking up a glyph, which decides which subtable gets
    /// the glyph-info class cache.
    pub fn cost(&self) -> u32 {
        let bits = |len: usize| u32::BITS - (len as u32).leading_zeros();
        match self {
            Self::None => 0,
            Self::Array { .. } | Self::Array24 { .. } => 1,
            Self::Ranges(ranges) => bits(ranges.len()),
            Self::Ranges24(ranges) => bits(ranges.len()),
        }
    }

    /// Adds the glyphs of `class` to `digest`. Class 0 holds every glyph not
    /// listed, so it adds all of them.
    pub fn add_class_to(&self, digest: &mut SetDigest, class: u32) {
        if class == 0 {
            *digest = SetDigest::full();
            return;
        }
        match self {
            Self::None => {}
            Self::Array { start, classes } => {
                for (index, value) in classes.iter().enumerate() {
                    if u32::from(value.get()) == class {
                        digest.add(u32::from(start.saturating_add(index as u16)));
                    }
                }
            }
            Self::Ranges(ranges) => {
                for range in *ranges {
                    if u32::from(range.class()) == class {
                        for glyph in range.start_glyph_id().to_u16()..=range.end_glyph_id().to_u16()
                        {
                            digest.add(u32::from(glyph));
                        }
                    }
                }
            }
            Self::Array24 { start, classes } => {
                for (index, value) in classes.iter().enumerate() {
                    if value.get().to_u32() == class {
                        digest.add(start.saturating_add(index as u32));
                    }
                }
            }
            Self::Ranges24(ranges) => {
                for range in *ranges {
                    if u32::from(range.class()) == class {
                        for glyph in range.start_glyph_id().to_u32()..=range.end_glyph_id().to_u32()
                        {
                            digest.add(glyph);
                        }
                    }
                }
            }
        }
    }
}

fn array24_class(start: u32, classes: &[BigEndian<Uint24>], gid: GlyphId) -> u32 {
    gid.to_u32()
        .checked_sub(start)
        .and_then(|index| classes.get(index as usize))
        .map_or(0, |class| class.get().to_u32())
}

fn ranges24_class(ranges: &[ClassRangeRecord2], gid: GlyphId) -> u32 {
    let gid = gid.to_u32();
    find_range(ranges, gid, |r| {
        (r.start_glyph_id().to_u32(), r.end_glyph_id().to_u32())
    })
    .map_or(0, |range| range.class().into())
}
