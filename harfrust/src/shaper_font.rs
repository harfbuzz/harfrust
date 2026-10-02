//! The font data and callbacks visible to a shaping operation.

use alloc::boxed::Box;
use core::{mem::size_of, ops::Deref, ptr, slice};
use read_fonts::types::GlyphId;
use read_fonts::TableProvider;

use crate::aat::{AatCache, AatData, EMPTY_AAT_DATA};
use crate::buffer::{Buffer, GlyphInfo, GlyphPosition};
use crate::cache::Cache;
use crate::font_support::{BasicFontMetrics, BuiltinFontFuncs, GlyphName};
use crate::ot::{OtCache, OtData, EMPTY_OT_DATA};
use crate::scale::Scale;

pub(crate) type CharmapCache = Cache<21, 19, 256, 32>;

/// Glyph ink extents in font units.
///
/// This matches HarfBuzz's glyph extents layout and semantics.
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct GlyphExtents {
    /// Horizontal bearing from glyph origin to the left side of the ink box.
    pub x_bearing: i32,
    /// Vertical bearing from glyph origin to the top of the ink box.
    pub y_bearing: i32,
    /// Width of the glyph ink box.
    pub width: i32,
    /// Height of the glyph ink box.
    pub height: i32,
}

/// Safe batch view for glyph id and horizontal advance updates.
pub struct Advances<'a> {
    pub(crate) infos: &'a [GlyphInfo],
    pub(crate) positions: &'a mut [GlyphPosition],
}

impl<'a> Advances<'a> {
    pub(crate) fn new(buffer: &'a mut Buffer) -> Self {
        let len = buffer.len;
        Self {
            infos: &buffer.info[..len],
            positions: &mut buffer.pos[..len],
        }
    }

    /// Returns the number of entries in the batch.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    /// Returns true if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }
}

pub struct AdvancesIter<'a> {
    infos: slice::Iter<'a, GlyphInfo>,
    positions: slice::IterMut<'a, GlyphPosition>,
}

impl<'a> Iterator for AdvancesIter<'a> {
    type Item = (GlyphId, &'a mut i32);

    fn next(&mut self) -> Option<Self::Item> {
        let info = self.infos.next()?;
        let pos = self.positions.next()?;
        Some((info.as_glyph(), &mut pos.x_advance))
    }
}

impl<'a> IntoIterator for Advances<'a> {
    type Item = (GlyphId, &'a mut i32);
    type IntoIter = AdvancesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        AdvancesIter {
            infos: self.infos.iter(),
            positions: self.positions.iter_mut(),
        }
    }
}

/// Safe batch view for codepoint to nominal glyph mapping.
///
/// Glyph ids must be written for consecutive codepoints starting at the
/// first entry; mapping stops at the first codepoint the font has no
/// glyph for, and the number of glyphs written is returned from
/// [`crate::font::FontFuncs::nominal_glyphs`].
pub struct NominalGlyphs<'a> {
    pub(crate) infos: &'a mut [GlyphInfo],
}

impl<'a> NominalGlyphs<'a> {
    pub(crate) fn new(infos: &'a mut [GlyphInfo]) -> Self {
        Self { infos }
    }

    /// Returns the number of entries in the batch.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    /// Returns true if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }
}

pub struct NominalGlyphsIter<'a> {
    infos: slice::IterMut<'a, GlyphInfo>,
}

impl<'a> Iterator for NominalGlyphsIter<'a> {
    type Item = (u32, &'a mut GlyphId);

    fn next(&mut self) -> Option<Self::Item> {
        let info = self.infos.next()?;
        let codepoint = info.glyph_id;
        let var_index = GlyphInfo::NORMALIZER_GLYPH_INDEX_VAR.var_index as usize - 1;
        Some((codepoint, bytemuck::cast_mut(&mut info.vars[var_index])))
    }
}

impl<'a> IntoIterator for NominalGlyphs<'a> {
    type Item = (u32, &'a mut GlyphId);
    type IntoIter = NominalGlyphsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        NominalGlyphsIter {
            infos: self.infos.iter_mut(),
        }
    }
}

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

/// Font-wide layout data shared by shapers at different variation positions.
pub(crate) struct LayoutCache {
    pub ot: OtCache,
    pub aat: AatCache,
    pub cmap: CharmapCache,
    /// True if the font has both `trak` and `STAT` tables.
    pub apply_trak: bool,
}

impl LayoutCache {
    pub(crate) fn new<'a>(font: &impl TableProvider<'a>, apply_trak: bool) -> Self {
        let ot = OtCache::new(font);
        let aat = AatCache::new(font, &ot);
        Self {
            ot,
            aat,
            cmap: CharmapCache::new(),
            apply_trak,
        }
    }
}

struct FontCache {
    layout: LayoutCache,
    metrics: BasicFontMetrics,
}

impl FontCache {
    fn new(font: &crate::font::Font) -> Self {
        let tables = font.tables();
        let apply_trak = tables.trak_data().is_some() && tables.stat_data().is_some();
        Self {
            layout: LayoutCache::new(&tables, apply_trak),
            metrics: BasicFontMetrics::new(&tables),
        }
    }
}

/// Returns cached layout data, or `None` if the read-fonts interop slot
/// contains an unexpected type.
fn font_cache(font: &crate::font::FontInstance) -> Option<(&LayoutCache, BasicFontMetrics)> {
    let data = crate::font::_font_interop::_get_or_init_shaping_data(font, || {
        Box::new(FontCache::new(font.font()))
    });
    let cache = data.downcast_ref::<FontCache>()?;
    Some((&cache.layout, cache.metrics))
}

/// Prepared, read-only data for one font and variation position.
///
/// The table views borrow a long-lived `LayoutCache`.
#[derive(Clone, Copy)]
pub(crate) struct LayoutData<'a> {
    pub ot: &'a OtData<'a>,
    pub aat: &'a AatData<'a>,
    pub units_per_em: u16,
    pub apply_trak: bool,
}

/// Prepared font data, scale, and optional callbacks used by shaping.
///
/// Dereferences to the underlying font for direct font queries.
#[derive(Clone)]
pub struct ShaperFont<'a, 'f> {
    font: &'a crate::font::FontInstance,
    ot: OtData<'a>,
    aat: AatData<'a>,
    units_per_em: u16,
    apply_trak: bool,
    builtin: BuiltinFontFuncs<'a>,
    pub(crate) scale: Scale,
    funcs: Option<&'f dyn FontFuncs>,
}

impl Deref for ShaperFont<'_, '_> {
    type Target = crate::font::FontInstance;

    fn deref(&self) -> &Self::Target {
        self.font
    }
}

impl<'a, 'f> ShaperFont<'a, 'f> {
    /// Prepares a font for shaping, reusing its cached layout data.
    pub fn new(font: &'a crate::font::FontInstance) -> Self {
        let cached = font_cache(font);
        let metrics = cached.map_or_else(
            || BasicFontMetrics::new(&font.tables()),
            |(_, metrics)| metrics,
        );
        let (ot, aat, apply_trak, cmap_cache) = if let Some((cache, _)) = cached {
            let tables = font.tables();
            let coords = font.normalized_coords();
            let feature_variations = if coords.is_empty() {
                [None; 2]
            } else {
                let variations = font.feature_variations();
                [variations.gsub(), variations.gpos()]
            };
            (
                OtData::from_tables(&tables, &cache.ot, coords, feature_variations),
                AatData::from_tables(&tables, &cache.aat),
                cache.apply_trak,
                Some(&cache.cmap),
            )
        } else {
            (EMPTY_OT_DATA.clone(), EMPTY_AAT_DATA.clone(), false, None)
        };
        let scale = font
            .size()
            .map(|ppem| {
                let value = (ppem * 65536.0) as i32;
                Scale::new(Some((value, value)), metrics.units_per_em as i32)
            })
            .unwrap_or_default();
        Self {
            font,
            ot,
            aat,
            units_per_em: metrics.units_per_em,
            apply_trak,
            builtin: BuiltinFontFuncs::from_font(
                font,
                metrics,
                font.normalized_coords(),
                metrics.units_per_em,
                cmap_cache,
            ),
            scale,
            funcs: None,
        }
    }

    pub(crate) fn layout(&self) -> LayoutData<'_> {
        LayoutData {
            ot: &self.ot,
            aat: &self.aat,
            units_per_em: self.units_per_em,
            apply_trak: self.apply_trak,
        }
    }

    /// Returns font's units per EM.
    pub fn units_per_em(&self) -> i32 {
        self.units_per_em as i32
    }

    /// Returns the currently active normalized coordinates.
    pub fn coords(&self) -> &[read_fonts::types::F2Dot14] {
        self.font.normalized_coords()
    }

    /// Preloads the built-in charmap and glyph metrics for repeated queries.
    #[doc(hidden)]
    pub fn preload_builtin_font_data(&self) {
        self.builtin.preload();
    }

    /// Sets the same scale on both axes.
    pub fn set_scale(&mut self, scale: i32) {
        self.set_scale_separate(scale, scale);
    }

    /// Returns a new shaping font with the same scale applied to both axes.
    pub fn with_scale(mut self, scale: i32) -> Self {
        self.set_scale(scale);
        self
    }

    /// Sets independent horizontal and vertical scales.
    pub fn set_scale_separate(&mut self, x_scale: i32, y_scale: i32) {
        self.scale = Scale::new(Some((x_scale, y_scale)), self.units_per_em as i32);
    }

    /// Returns a new shaping font with independent horizontal and vertical scales applied.
    pub fn with_scale_separate(mut self, x_scale: i32, y_scale: i32) -> Self {
        self.set_scale_separate(x_scale, y_scale);
        self
    }

    /// Replaces the callbacks used for effective font queries.
    pub fn set_font_funcs(&mut self, funcs: Option<&'f dyn FontFuncs>) {
        self.funcs = funcs;
    }

    /// Returns a new shaping font with the specified font function callbacks.
    pub fn with_font_funcs(mut self, funcs: Option<&'f dyn FontFuncs>) -> Self {
        self.set_font_funcs(funcs);
        self
    }

    /// Returns the conversion from font units to the configured scale.
    pub fn scale(&self) -> Scale {
        self.scale
    }

    /// Returns the name stored for a glyph in the font, if any.
    pub fn glyph_name(&self, glyph: GlyphId) -> Option<GlyphName> {
        GlyphName::new(self.builtin.glyph_names().get(glyph.to_u32())?)
    }

    /// Maps a character through the configured callbacks.
    pub fn nominal_glyph(&self, codepoint: u32) -> Option<GlyphId> {
        self.funcs.map_or_else(
            || self.default_nominal_glyph(codepoint),
            |f| f.nominal_glyph(self, codepoint),
        )
    }

    /// Maps a character using the font's own charmap.
    pub fn default_nominal_glyph(&self, codepoint: u32) -> Option<GlyphId> {
        if let Some(gid) = self
            .builtin
            .cmap_cache()
            .and_then(|cache| cache.get(codepoint))
        {
            Some(gid.into())
        } else if let Some(gid) = self.builtin.nominal_glyph(codepoint) {
            if let Some(cache) = self.builtin.cmap_cache() {
                cache.set(codepoint, gid.to_u32());
            }
            Some(gid)
        } else {
            None
        }
    }

    pub(crate) fn has_glyph(&self, codepoint: u32) -> bool {
        self.nominal_glyph(codepoint).is_some()
    }

    /// Maps a batch of codepoints through the configured callbacks.
    pub fn nominal_glyphs(&self, glyphs: NominalGlyphs<'_>) -> usize {
        if let Some(funcs) = self.funcs {
            funcs.nominal_glyphs(self, glyphs)
        } else {
            self.default_nominal_glyphs(glyphs)
        }
    }

    /// Maps a batch of codepoints using the font's own charmap.
    pub fn default_nominal_glyphs(&self, glyphs: NominalGlyphs<'_>) -> usize {
        let mut done = 0;
        for (codepoint, glyph) in glyphs {
            match self.default_nominal_glyph(codepoint) {
                Some(gid) => *glyph = gid,
                None => break,
            }
            done += 1;
        }
        done
    }

    /// Maps a character and variation selector through the configured callbacks.
    pub fn variant_glyph(&self, codepoint: u32, selector: u32) -> Option<GlyphId> {
        self.funcs.map_or_else(
            || self.default_variant_glyph(codepoint, selector),
            |f| f.variant_glyph(self, codepoint, selector),
        )
    }

    /// Maps a variation sequence using the font's own charmap.
    pub fn default_variant_glyph(&self, codepoint: u32, selector: u32) -> Option<GlyphId> {
        self.builtin.variant_glyph(codepoint, selector)
    }

    /// Returns the effective horizontal advance.
    pub fn h_advance(&self, glyph: GlyphId) -> i32 {
        self.funcs.map_or_else(
            || self.default_h_advance(glyph),
            |f| f.h_advance(self, glyph),
        )
    }

    /// Returns the scaled horizontal advance from the font's own tables.
    pub fn default_h_advance(&self, glyph: GlyphId) -> i32 {
        self.scale.scale_x(self.builtin.advance_width(glyph))
    }

    /// Writes effective horizontal advances for a batch of glyphs.
    pub fn h_advances(&self, advances: Advances<'_>) {
        if let Some(funcs) = self.funcs {
            funcs.h_advances(self, advances);
        } else {
            self.default_h_advances(advances);
        }
    }

    /// Writes scaled horizontal advances from the font's own tables.
    pub fn default_h_advances(&self, advances: Advances<'_>) {
        self.builtin.glyph_metrics().populate_advance_widths(
            advances.infos,
            advances.positions,
            self.builtin.coords(),
            self.scale,
        );
    }

    /// Returns the effective vertical advance.
    pub fn v_advance(&self, glyph: GlyphId) -> i32 {
        self.funcs.map_or_else(
            || self.default_v_advance(glyph),
            |f| f.v_advance(self, glyph),
        )
    }

    /// Returns the scaled vertical advance from the font's own tables.
    pub fn default_v_advance(&self, glyph: GlyphId) -> i32 {
        self.scale.scale_y(self.builtin.advance_height(glyph))
    }

    /// Returns the effective vertical origin.
    pub fn v_origin(&self, glyph: GlyphId) -> (i32, i32) {
        self.funcs
            .map_or_else(|| self.default_v_origin(glyph), |f| f.v_origin(self, glyph))
    }

    /// Returns the scaled vertical origin from the font's own tables.
    pub fn default_v_origin(&self, glyph: GlyphId) -> (i32, i32) {
        let (x, y) = self.builtin.vertical_origin(glyph);
        (self.scale.scale_x(x), self.scale.scale_y(y))
    }

    /// Returns the effective glyph extents.
    pub fn glyph_extents(&self, glyph: GlyphId) -> Option<GlyphExtents> {
        self.funcs.map_or_else(
            || self.default_glyph_extents(glyph),
            |f| f.glyph_extents(self, glyph),
        )
    }

    /// Returns the scaled glyph extents from the font's own tables.
    pub fn default_glyph_extents(&self, glyph: GlyphId) -> Option<GlyphExtents> {
        self.builtin
            .extents(glyph)
            .map(|e| self.scale.scale_extents(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{aat::AatData, ot::OtData, Tag};
    use core::cell::Cell;
    use read_fonts::{FontData, FontRef, TableProvider};

    struct CountingProvider<'a> {
        font: FontRef<'a>,
        loads: Cell<usize>,
    }

    impl<'a> TableProvider<'a> for CountingProvider<'a> {
        fn data_for_tag(&self, tag: Tag) -> Option<FontData<'a>> {
            self.loads.set(self.loads.get() + 1);
            self.font.data_for_tag(tag)
        }
    }

    #[test]
    fn cached_table_disposition_skips_missing_tables() {
        let font = FontRef::new(include_bytes!("../benches/fonts/Roboto-Regular.ttf")).unwrap();
        let provider = CountingProvider {
            font,
            loads: Cell::new(0),
        };
        let cache = LayoutCache::new(&provider, false);

        provider.loads.set(0);
        let ot_data = OtData::from_tables(&provider, &cache.ot, &[], [None; 2]);
        let aat_data = AatData::from_tables(&provider, &cache.aat);

        assert!(ot_data.gsub.is_some());
        assert!(ot_data.gpos.is_some());
        assert!(ot_data.gdef.table.is_some());
        assert!(aat_data.morx.is_none());
        assert!(aat_data.mort.is_none());
        assert!(aat_data.ankr.is_none());
        assert!(aat_data.kern.is_none());
        assert!(aat_data.kerx.is_none());
        assert!(aat_data.trak.is_none());
        assert!(aat_data.feat.is_none());
        assert!(aat_data.ltag.is_none());
        assert_eq!(provider.loads.get(), 3);
    }

    #[test]
    fn extents_scale_from_corners_like_harfbuzz() {
        // HarfBuzz scales corners in floating point, floors the bearings,
        // ceils the far corners, and then derives width/height from them.
        let scale = Scale::new(Some((1500, 1500)), 1000);
        let extents = GlyphExtents {
            x_bearing: 1,
            y_bearing: 4,
            width: 3,
            height: -2,
        };
        let scaled = scale.scale_extents(extents);
        assert_eq!(scaled.x_bearing, 1);
        assert_eq!(scaled.y_bearing, 6);
        assert_eq!(scaled.width, 5);
        assert_eq!(scaled.height, -3);
    }

    #[test]
    fn full_range_extents_saturate() {
        let extents = GlyphExtents {
            x_bearing: i32::MAX,
            y_bearing: i32::MIN,
            width: i32::MAX,
            height: i32::MIN,
        };

        let scaled = Scale::default().scale_extents(extents);
        assert_eq!(scaled.x_bearing, i32::MAX);
        assert_eq!(scaled.y_bearing, i32::MIN);
        assert_eq!(scaled.width, i32::MAX);
        assert_eq!(scaled.height, i32::MIN);
    }
}

/// Customizable font callback surface.
///
/// # Metrics scaling
///
/// All font metrics returned by these callbacks must be consistent with the
/// scale factor configured on [`ShaperFont`].
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
