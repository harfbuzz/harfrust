use core::mem::size_of;
use core::ptr;
use core::slice;

use read_fonts::types::GlyphId;

use super::buffer::{hb_buffer_t, GlyphInfo, GlyphPosition};
use super::face::{hb_font_t, GlyphExtents};

/// Raw C-style view over a batch of glyph ids and advance widths.
#[derive(Clone, Copy, Debug)]
pub struct RawAdvanceWidthBatch {
    /// Number of batch entries.
    pub len: usize,
    /// Pointer to glyph ids (read-only).
    pub gids: *const u32,
    /// Pointer to horizontal advances (writable).
    pub advances: *mut i32,
    /// Byte stride between successive glyph ids.
    pub gid_stride: isize,
    /// Byte stride between successive advances.
    pub advance_stride: isize,
}

/// Safe batch view for glyph id / horizontal-advance updates.
pub struct AdvanceWidthBatch<'a> {
    infos: &'a [GlyphInfo],
    positions: &'a mut [GlyphPosition],
}

impl<'a> AdvanceWidthBatch<'a> {
    pub(crate) fn new(buffer: &'a mut hb_buffer_t) -> Self {
        let len = buffer.len;
        let infos = &buffer.info[..len];
        let positions = &mut buffer.pos[..len];
        Self { infos, positions }
    }

    /// Returns the number of entries in the batch.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    /// Returns true if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }

    /// Returns a raw C-style view over this batch.
    pub fn into_raw(self) -> RawAdvanceWidthBatch {
        if self.infos.is_empty() {
            return RawAdvanceWidthBatch {
                len: 0,
                gids: ptr::null(),
                advances: ptr::null_mut(),
                gid_stride: size_of::<GlyphInfo>() as isize,
                advance_stride: size_of::<GlyphPosition>() as isize,
            };
        }

        RawAdvanceWidthBatch {
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

pub struct AdvanceWidthBatchIter<'a> {
    infos: slice::Iter<'a, GlyphInfo>,
    positions: slice::IterMut<'a, GlyphPosition>,
}

impl<'a> Iterator for AdvanceWidthBatchIter<'a> {
    type Item = (GlyphId, &'a mut i32);

    fn next(&mut self) -> Option<Self::Item> {
        let info = self.infos.next()?;
        let pos = self.positions.next()?;
        Some((info.as_glyph(), &mut pos.x_advance))
    }
}

impl<'a> IntoIterator for AdvanceWidthBatch<'a> {
    type Item = (GlyphId, &'a mut i32);
    type IntoIter = AdvanceWidthBatchIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        AdvanceWidthBatchIter {
            infos: self.infos.iter(),
            positions: self.positions.iter_mut(),
        }
    }
}

/// Default implementations backed by font tables.
pub struct BuiltinFontFuncs<'a> {
    face: &'a hb_font_t<'a>,
}

impl<'a> BuiltinFontFuncs<'a> {
    pub(crate) fn new(face: &'a hb_font_t<'a>) -> Self {
        Self { face }
    }

    /// Maps a Unicode scalar value to a nominal glyph.
    pub fn nominal_glyph(&self, c: u32) -> Option<GlyphId> {
        self.face.get_nominal_glyph(c)
    }

    /// Maps a Unicode scalar value and variation selector to a glyph.
    pub fn variant_glyph(&self, c: u32, vs: u32) -> Option<GlyphId> {
        self.face.get_nominal_variant_glyph(c, vs)
    }

    /// Returns the horizontal advance for a glyph.
    pub fn advance_width(&self, glyph: GlyphId) -> i32 {
        self.face.glyph_h_advance(glyph)
    }

    /// Returns the vertical advance for a glyph.
    pub fn advance_height(&self, glyph: GlyphId) -> i32 {
        self.face.glyph_v_advance(glyph)
    }

    /// Returns the horizontal origin for a glyph.
    pub fn horizontal_origin(&self, glyph: GlyphId) -> i32 {
        self.advance_width(glyph) / 2
    }

    /// Returns the vertical origin for a glyph.
    pub fn vertical_origin(&self, glyph: GlyphId) -> i32 {
        self.face.glyph_v_origin(glyph)
    }

    /// Returns extents for a glyph if available.
    pub fn extents(&self, glyph: GlyphId) -> Option<GlyphExtents> {
        let mut extents = GlyphExtents::default();
        if self.face.glyph_extents(glyph, &mut extents) {
            Some(extents)
        } else {
            None
        }
    }

    /// Populates horizontal advances for all entries in the batch.
    pub fn populate_advance_widths(&self, batch: AdvanceWidthBatch<'_>) {
        for (glyph, advance) in batch {
            *advance = self.face.glyph_h_advance(glyph);
        }
    }
}

/// Customizable font callback surface.
pub trait FontFuncs {
    /// Nominal character-to-glyph mapping callback.
    fn nominal_glyph(&mut self, builtin: &BuiltinFontFuncs, c: u32) -> Option<GlyphId> {
        builtin.nominal_glyph(c)
    }

    /// Variation-selector mapping callback.
    fn variant_glyph(&mut self, builtin: &BuiltinFontFuncs, c: u32, vs: u32) -> Option<GlyphId> {
        builtin.variant_glyph(c, vs)
    }

    /// Horizontal advance callback.
    fn advance_width(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        builtin.advance_width(glyph)
    }

    /// Batch horizontal-advance callback.
    fn populate_advance_widths(
        &mut self,
        builtin: &BuiltinFontFuncs,
        batch: AdvanceWidthBatch<'_>,
    ) {
        for (glyph, advance) in batch {
            *advance = self.advance_width(builtin, glyph);
        }
    }

    /// Vertical advance callback.
    fn advance_height(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        builtin.advance_height(glyph)
    }

    /// Vertical origin callback.
    fn vertical_origin(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        builtin.vertical_origin(glyph)
    }

    /// Glyph extents callback.
    fn extents(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> Option<GlyphExtents> {
        builtin.extents(glyph)
    }
}

pub struct DummyFontFuncs;

impl FontFuncs for DummyFontFuncs {}

pub(crate) struct FontFuncsDispatch<'a, 'u> {
    builtin: BuiltinFontFuncs<'a>,
    funcs: &'u mut (dyn FontFuncs + 'u),
    has_custom_funcs: bool,
}

impl<'a, 'u> FontFuncsDispatch<'a, 'u> {
    pub(crate) fn new(
        face: &'a hb_font_t<'a>,
        funcs: &'u mut (dyn FontFuncs + 'u),
        has_custom_funcs: bool,
    ) -> Self {
        Self {
            builtin: BuiltinFontFuncs::new(face),
            funcs,
            has_custom_funcs,
        }
    }

    pub(crate) fn font(&self) -> &'a hb_font_t<'a> {
        self.builtin.face
    }

    #[inline(always)]
    pub(crate) fn nominal_glyph(&mut self, c: u32) -> Option<GlyphId> {
        let cache = self.builtin.face.cmap_cache;
        if let Some(gid) = cache.get(c) {
            Some(gid.into())
        } else if let Some(gid) = self.funcs.nominal_glyph(&self.builtin, c) {
            cache.set(c, gid.to_u32());
            Some(gid)
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) fn has_glyph(&mut self, c: u32) -> bool {
        self.nominal_glyph(c).is_some()
    }

    #[inline(always)]
    pub(crate) fn variant_glyph(&mut self, c: u32, vs: u32) -> Option<GlyphId> {
        self.funcs.variant_glyph(&self.builtin, c, vs)
    }

    #[inline(always)]
    pub(crate) fn advance_width(&mut self, glyph: GlyphId) -> i32 {
        self.funcs.advance_width(&self.builtin, glyph)
    }

    #[inline(always)]
    pub(crate) fn advance_height(&mut self, glyph: GlyphId) -> i32 {
        self.funcs.advance_height(&self.builtin, glyph)
    }

    #[inline(always)]
    pub(crate) fn horizontal_origin(&mut self, glyph: GlyphId) -> i32 {
        self.advance_width(glyph) / 2
    }

    #[inline(always)]
    pub(crate) fn vertical_origin(&mut self, glyph: GlyphId) -> i32 {
        self.funcs.vertical_origin(&self.builtin, glyph)
    }

    #[inline(always)]
    pub(crate) fn extents(&mut self, glyph: GlyphId) -> Option<GlyphExtents> {
        self.funcs.extents(&self.builtin, glyph)
    }

    pub(crate) fn populate_advance_widths(&mut self, batch: AdvanceWidthBatch<'_>) {
        self.funcs.populate_advance_widths(&self.builtin, batch);
    }

    pub(crate) fn has_custom_funcs(&self) -> bool {
        self.has_custom_funcs
    }
}
