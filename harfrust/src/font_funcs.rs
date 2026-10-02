use core::mem::size_of;
use core::ptr;

use read_fonts::types::F2Dot14;
use read_fonts::types::GlyphId;

use crate::face::BasicFontMetrics;
use crate::shape::font_ref::{Charmap, GlyphMetrics, LegacyFont};
use crate::shape::CharmapCache;
use crate::shape::{Advances, NominalGlyphs, ShaperFont};

use super::buffer::{GlyphInfo, GlyphPosition};
use super::face::GlyphExtents;

/// Raw C-style view over a batch of glyph ids and advance widths.
#[derive(Clone, Copy, Debug)]
pub struct RawAdvances {
    /// Number of batch entries.
    pub len: usize,
    /// Pointer to glyph ids (read-only).
    pub gids: *const u32,
    /// Pointer to horizontal advances (writable).
    ///
    /// See "Metrics scaling" in the [FontFuncs] for details
    /// on what value this method should return.
    pub advances: *mut i32,
    /// Byte stride between successive glyph ids.
    pub gid_stride: isize,
    /// Byte stride between successive advances.
    pub advance_stride: isize,
}

impl Advances<'_> {
    /// Returns a raw C-style view over this batch.
    pub fn into_raw(self) -> RawAdvances {
        if self.infos.is_empty() {
            return RawAdvances {
                len: 0,
                gids: ptr::null(),
                advances: ptr::null_mut(),
                gid_stride: size_of::<GlyphInfo>() as isize,
                advance_stride: size_of::<GlyphPosition>() as isize,
            };
        }

        RawAdvances {
            len: self.infos.len(),
            // `glyph_id` is the first field in `GlyphInfo`.
            gids: self.infos.as_ptr().cast::<u32>(),
            // `x_advance` is the first field in `GlyphPosition`.
            advances: self.positions.as_mut_ptr().cast::<i32>(),
            gid_stride: size_of::<GlyphInfo>() as isize,
            advance_stride: size_of::<GlyphPosition>() as isize,
        }
    }
}

/// Raw C-style view over a batch of codepoints and output glyph ids.
#[derive(Clone, Copy, Debug)]
pub struct RawNominalGlyphs {
    /// Number of batch entries.
    pub len: usize,
    /// Pointer to codepoints (read-only).
    pub codepoints: *const u32,
    /// Pointer to output glyph ids (writable).
    pub glyphs: *mut u32,
    /// Byte stride between successive codepoints.
    pub codepoint_stride: isize,
    /// Byte stride between successive glyphs.
    pub glyph_stride: isize,
}

impl NominalGlyphs<'_> {
    /// Byte offset of the output glyph id within a batch entry.
    const GLYPH_OFFSET: usize = core::mem::offset_of!(GlyphInfo, vars)
        + (GlyphInfo::NORMALIZER_GLYPH_INDEX_VAR.var_index as usize - 1) * size_of::<u32>();

    /// Returns a raw C-style view over this batch.
    pub fn into_raw(self) -> RawNominalGlyphs {
        if self.infos.is_empty() {
            return RawNominalGlyphs {
                len: 0,
                codepoints: ptr::null(),
                glyphs: ptr::null_mut(),
                codepoint_stride: size_of::<GlyphInfo>() as isize,
                glyph_stride: size_of::<GlyphInfo>() as isize,
            };
        }

        let base = self.infos.as_mut_ptr();
        RawNominalGlyphs {
            len: self.infos.len(),
            // `glyph_id` is the first field in `GlyphInfo` and holds the
            // codepoint before mapping.
            codepoints: base.cast::<u32>().cast_const(),
            // The normalizer glyph-index var.
            glyphs: base.wrapping_byte_add(Self::GLYPH_OFFSET).cast::<u32>(),
            codepoint_stride: size_of::<GlyphInfo>() as isize,
            glyph_stride: size_of::<GlyphInfo>() as isize,
        }
    }
}

struct InstanceFont<'a> {
    instance: &'a crate::font::FontInstance,
    basic_metrics: BasicFontMetrics,
    prepared_glyph_metrics: Option<&'a GlyphMetrics<'a>>,
    prepared_charmap: Option<&'a Charmap<'a>>,
    glyph_metrics: core::cell::OnceCell<GlyphMetrics<'a>>,
    charmap: core::cell::OnceCell<Charmap<'a>>,
}

enum BuiltinSource<'a> {
    Legacy(LegacyFont<'a>),
    Instance(InstanceFont<'a>),
}

impl<'a> BuiltinSource<'a> {
    fn charmap(&self) -> &Charmap<'a> {
        match self {
            Self::Legacy(font) => font.charmap,
            Self::Instance(font) => font.prepared_charmap.unwrap_or_else(|| {
                font.charmap
                    .get_or_init(|| Charmap::from_tables(&font.instance.tables()))
            }),
        }
    }

    fn glyph_metrics(&self) -> &GlyphMetrics<'a> {
        match self {
            Self::Legacy(font) => font.glyph_metrics,
            Self::Instance(font) => font.prepared_glyph_metrics.unwrap_or_else(|| {
                font.glyph_metrics.get_or_init(|| {
                    GlyphMetrics::from_tables(&font.instance.tables(), &font.basic_metrics)
                })
            }),
        }
    }
}

/// Default implementations backed by font tables.
pub struct BuiltinFontFuncs<'a> {
    source: BuiltinSource<'a>,
    coords: &'a [F2Dot14],
    units_per_em: u16,
    pub(crate) cmap_cache: &'a CharmapCache,
}

impl<'a> BuiltinFontFuncs<'a> {
    pub(crate) fn from_legacy(
        glyph_metrics: &'a GlyphMetrics<'a>,
        charmap: &'a Charmap<'a>,
        coords: &'a [F2Dot14],
        units_per_em: u16,
        cmap_cache: &'a CharmapCache,
    ) -> Self {
        Self {
            source: BuiltinSource::Legacy(LegacyFont {
                glyph_metrics,
                charmap,
            }),
            coords,
            units_per_em,
            cmap_cache,
        }
    }

    pub(crate) fn from_instance(
        instance: &'a crate::font::FontInstance,
        basic_metrics: BasicFontMetrics,
        prepared_glyph_metrics: Option<&'a GlyphMetrics<'a>>,
        prepared_charmap: Option<&'a Charmap<'a>>,
        coords: &'a [F2Dot14],
        units_per_em: u16,
        cmap_cache: &'a CharmapCache,
    ) -> Self {
        Self {
            source: BuiltinSource::Instance(InstanceFont {
                instance,
                basic_metrics,
                prepared_glyph_metrics,
                prepared_charmap,
                glyph_metrics: core::cell::OnceCell::new(),
                charmap: core::cell::OnceCell::new(),
            }),
            coords,
            units_per_em,
            cmap_cache,
        }
    }

    pub(crate) fn coords(&self) -> &[F2Dot14] {
        self.coords
    }

    fn charmap(&self) -> &Charmap<'a> {
        self.source.charmap()
    }

    pub(crate) fn glyph_metrics(&self) -> &GlyphMetrics<'a> {
        self.source.glyph_metrics()
    }

    /// Maps a Unicode scalar value to a nominal glyph.
    pub fn nominal_glyph(&self, c: u32) -> Option<GlyphId> {
        // A cmap entry pointing at .notdef says the font has no glyph for
        // the character, rather than that its glyph is .notdef. HarfBuzz
        // reads it the same way, and the difference shows once a caller
        // asks for a not-found glyph of its own.
        self.charmap().map(c).filter(|glyph| glyph.to_u32() != 0)
    }

    /// Maps a Unicode scalar value and variation selector to a glyph.
    pub fn variant_glyph(&self, c: u32, vs: u32) -> Option<GlyphId> {
        // As in `nominal_glyph`: .notdef is not a glyph the font has.
        self.charmap()
            .map_variant(c, vs)
            .filter(|glyph| glyph.to_u32() != 0)
    }

    /// Returns the horizontal advance for a glyph.
    pub fn advance_width(&self, glyph: GlyphId) -> i32 {
        self.glyph_metrics()
            .advance_width(glyph, self.coords())
            .unwrap_or_default()
    }

    /// Returns the vertical advance for a glyph.
    pub fn advance_height(&self, glyph: GlyphId) -> i32 {
        self.glyph_metrics()
            .advance_height(glyph, self.coords())
            .unwrap_or(self.units_per_em as i32)
            .saturating_neg()
    }

    /// Returns the vertical origin for a glyph.
    pub fn vertical_origin(&self, glyph: GlyphId) -> (i32, i32) {
        let v_origin_y = self
            .glyph_metrics()
            .v_origin(glyph, self.coords())
            .unwrap_or_default();
        (self.advance_width(glyph) / 2, v_origin_y)
    }

    /// Returns extents for a glyph if available.
    pub fn extents(&self, glyph: GlyphId) -> Option<GlyphExtents> {
        self.glyph_metrics().extents(glyph, self.coords())
    }

    /// Populates horizontal advances for all entries in the batch.
    pub fn populate_advance_widths(&self, batch: Advances<'_>) {
        for (glyph, advance) in batch {
            *advance = self.advance_width(glyph);
        }
    }

    /// Maps a run of codepoints to nominal glyphs, stopping at the first
    /// codepoint the font has no glyph for. Returns the number of
    /// consecutive codepoints mapped.
    pub fn populate_nominal_glyphs(&self, batch: NominalGlyphs<'_>) -> usize {
        let mut done = 0;
        for (codepoint, glyph) in batch {
            match self.nominal_glyph(codepoint) {
                Some(gid) => *glyph = gid,
                None => break,
            }
            done += 1;
        }
        done
    }
}

/// Customizable font callback surface.
///
/// # Metrics scaling
///
/// All font metrics returned by these callbacks must be consistent with the
/// scale factor configured on [`ShaperFont`] or through
/// [`ShapeOptions::scale`](crate::ShapeOptions::scale).
///
/// If no scale is set, values must be in unscaled font units (i.e. the same
/// coordinate space as the font's `units_per_em`). If a scale is set —
/// for example `font_size * 64` for FreeType-style 26.6 — then all returned
/// values must already be in that scaled coordinate space.
pub trait FontFuncs {
    /// Nominal character-to-glyph mapping callback.
    fn nominal_glyph(&self, font: &ShaperFont, c: u32) -> Option<GlyphId> {
        font.default_nominal_glyph(c)
    }

    /// Batch nominal character-to-glyph mapping callback.
    ///
    /// Maps a run of codepoints to glyphs, stopping at the first
    /// codepoint the font has no glyph for. Returns the number of
    /// consecutive codepoints mapped.
    fn nominal_glyphs(&self, font: &ShaperFont, glyphs: NominalGlyphs<'_>) -> usize {
        let mut done = 0;
        for (codepoint, glyph) in glyphs {
            match self.nominal_glyph(font, codepoint) {
                Some(gid) => *glyph = gid,
                None => break,
            }
            done += 1;
        }
        done
    }

    /// Variation-selector mapping callback.
    fn variant_glyph(&self, font: &ShaperFont, c: u32, vs: u32) -> Option<GlyphId> {
        font.default_variant_glyph(c, vs)
    }

    /// Horizontal advance callback.
    ///
    /// See "Metrics scaling" in the [trait-level docs](FontFuncs) for details
    /// on what value this method should return.
    fn h_advance(&self, font: &ShaperFont, glyph: GlyphId) -> i32 {
        font.default_h_advance(glyph)
    }

    /// Batch horizontal-advance callback.
    ///
    /// See "Metrics scaling" in the [trait-level docs](FontFuncs) for details
    /// on what value this method should return.
    fn h_advances(&self, font: &ShaperFont, advances: Advances<'_>) {
        for (glyph, advance) in advances {
            *advance = self.h_advance(font, glyph);
        }
    }

    /// Vertical advance callback.
    ///
    /// See "Metrics scaling" in the [trait-level docs](FontFuncs) for details
    /// on what value this method should return.
    fn v_advance(&self, font: &ShaperFont, glyph: GlyphId) -> i32 {
        font.default_v_advance(glyph)
    }

    /// Vertical origin callback.
    ///
    /// Returns the (x, y) coordinates of the vertical origin for the given glyph.
    ///
    /// See "Metrics scaling" in the [trait-level docs](FontFuncs) for details
    /// on what values this method should return.
    fn v_origin(&self, font: &ShaperFont, glyph: GlyphId) -> (i32, i32) {
        font.default_v_origin(glyph)
    }

    /// Glyph extents callback.
    ///
    /// See "Metrics scaling" in the [trait-level docs](FontFuncs) for details
    /// on what values this method should return.
    fn glyph_extents(&self, font: &ShaperFont, glyph: GlyphId) -> Option<GlyphExtents> {
        font.default_glyph_extents(glyph)
    }
}
