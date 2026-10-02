//! The font data and callbacks visible to a shaping operation.

use core::slice;
use read_fonts::types::GlyphId;
use read_fonts::TableProvider;

#[cfg(feature = "experimental_font_api")]
use super::options::ShapeOptions;
use super::scale::Scale;
use crate::aat::{AatCache, AatData};
use crate::buffer::{Buffer, GlyphInfo, GlyphPosition};
use crate::face::Shaper;
use crate::font_funcs::{BuiltinFontFuncs, FontFuncs};
use crate::ot::{OtCache, OtData};
use crate::shape::CharmapCache;
#[cfg(feature = "experimental_font_api")]
use crate::ShapeError;

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
/// [`crate::font_funcs::FontFuncs::nominal_glyphs`].
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
pub struct ShaperFont<'a, 'f> {
    pub(crate) layout: LayoutData<'a>,
    builtin: BuiltinFontFuncs<'a>,
    pub(crate) scale: Scale,
    funcs: Option<&'f dyn FontFuncs>,
}

impl<'a, 'f> ShaperFont<'a, 'f> {
    pub(crate) fn new(
        layout: LayoutData<'a>,
        builtin: BuiltinFontFuncs<'a>,
        scale: Scale,
        funcs: Option<&'f dyn FontFuncs>,
    ) -> Self {
        Self {
            layout,
            builtin,
            scale,
            funcs,
        }
    }

    /// Borrows the prepared data of a shaper, initially using font units and
    /// the font's own query implementations.
    pub fn from_shaper(shaper: &'a Shaper<'a>) -> Self {
        Self::new(
            LayoutData::from_shaper(shaper),
            shaper.builtin_font_funcs(),
            Scale::default(),
            None,
        )
    }

    /// Sets the same scale on both axes.
    pub fn set_scale(&mut self, scale: i32) {
        self.set_scale_separate(scale, scale);
    }

    /// Returns a new instance with the same scale applied to both axes.
    pub fn with_scale(mut self, scale: i32) -> Self {
        self.set_scale(scale);
        self
    }

    /// Sets independent horizontal and vertical scales.
    pub fn set_scale_separate(&mut self, x_scale: i32, y_scale: i32) {
        self.scale = Scale::new(Some((x_scale, y_scale)), self.layout.units_per_em as i32);
    }

    /// Returns a new instance with independent horizontal and vertical scales applied.
    pub fn with_scale_separate(mut self, x_scale: i32, y_scale: i32) -> Self {
        self.set_scale_separate(x_scale, y_scale);
        self
    }

    /// Replaces the callbacks used for effective font queries.
    pub fn set_font_funcs(&mut self, funcs: Option<&'f dyn FontFuncs>) {
        self.funcs = funcs;
    }

    /// Returns a new instance with the specified font function callbacks.
    pub fn with_font_funcs(mut self, funcs: Option<&'f dyn FontFuncs>) -> Self {
        self.set_font_funcs(funcs);
        self
    }

    /// Returns the conversion from font units to the configured scale.
    pub fn scale(&self) -> Scale {
        self.scale
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
        if let Some(gid) = self.builtin.cmap_cache.get(codepoint) {
            Some(gid.into())
        } else if let Some(gid) = self.builtin.nominal_glyph(codepoint) {
            self.builtin.cmap_cache.set(codepoint, gid.to_u32());
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

#[cfg(feature = "experimental_font_api")]
impl Buffer {
    /// Shapes the buffer contents in place with the given font.
    ///
    /// This matches HarfBuzz's `hb_shape`. On success the buffer holds
    /// [`crate::ContentType::Glyphs`].
    ///
    /// If a plan is supplied through [`ShapeOptions::plan`] it must have been
    /// built for this buffer's direction and script, which is checked before
    /// anything is touched.
    ///
    /// # Errors
    ///
    /// [`ShapeError::AlreadyShaped`] if the buffer holds glyphs rather than
    /// text, [`ShapeError::DirectionUnset`] if it has no direction and no plan
    /// was supplied to give it one, [`ShapeError::UnusableFont`] if the font
    /// has nothing to shape with, [`ShapeError::DirectionMismatch`] or
    /// [`ShapeError::ScriptMismatch`] if the supplied plan was built for other
    /// properties.
    ///
    /// Each of these is a misuse of the API, caught before anything is
    /// touched, so a failure leaves the buffer exactly as it arrived. Running
    /// out of room is not among them; check
    /// [`allocation_successful`](Buffer::allocation_successful) for that.
    pub fn shape(
        &mut self,
        font: &crate::font::FontInstance,
        mut options: ShapeOptions<'_>,
    ) -> Result<(), ShapeError> {
        let shaper = Shaper::from_font(font);
        let Some(shaper) = shaper.as_ref() else {
            return Err(ShapeError::UnusableFont);
        };
        // If the user didn't request an explicit scale but the font instance
        // has a size, set the scale to that size with 16 fractional bits.
        if options.scale.is_none() {
            if let Some(ppem) = font.size() {
                options = options.scale(Some((ppem * 65536.0) as i32));
            }
        }
        shaper.shape_buffer_inner(self, options)
    }
}
