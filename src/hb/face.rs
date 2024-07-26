use bytemuck::{Pod, Zeroable};
#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;

use crate::hb::paint_extents::hb_paint_extents_context_t;
use ttf_parser::gdef::GlyphClass;
use ttf_parser::opentype_layout::LayoutTable;
use ttf_parser::{GlyphId, RgbaColor};

use super::buffer::GlyphPropsFlags;
use super::fonta;
use super::ot_layout::TableIndex;
use super::ot_layout_common::{PositioningTable, SubstitutionTable};
use crate::Variation;

/// A font face handle.
#[derive(Clone)]
pub struct hb_font_t<'a> {
    pub(crate) ttfp_face: ttf_parser::Face<'a>,
    pub(crate) font: fonta::Font<'a>,
    pub(crate) units_per_em: u16,
    pixels_per_em: Option<(u16, u16)>,
    pub(crate) points_per_em: Option<f32>,
    pub(crate) gsub: Option<SubstitutionTable<'a>>,
    pub(crate) gpos: Option<PositioningTable<'a>>,
}

impl<'a> AsRef<ttf_parser::Face<'a>> for hb_font_t<'a> {
    #[inline]
    fn as_ref(&self) -> &ttf_parser::Face<'a> {
        &self.ttfp_face
    }
}

impl<'a> AsMut<ttf_parser::Face<'a>> for hb_font_t<'a> {
    #[inline]
    fn as_mut(&mut self) -> &mut ttf_parser::Face<'a> {
        &mut self.ttfp_face
    }
}

impl<'a> core::ops::Deref for hb_font_t<'a> {
    type Target = ttf_parser::Face<'a>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.ttfp_face
    }
}

impl<'a> core::ops::DerefMut for hb_font_t<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ttfp_face
    }
}

impl<'a> hb_font_t<'a> {
    /// Creates a new `Face` from data.
    ///
    /// Data will be referenced, not owned.
    pub fn from_slice(data: &'a [u8], face_index: u32) -> Option<Self> {
        let face = ttf_parser::Face::parse(data, face_index).ok()?;
        let font = fonta::Font::new(data, face_index)?;
        Some(hb_font_t {
            font,
            units_per_em: face.units_per_em(),
            pixels_per_em: None,
            points_per_em: None,
            gsub: face.tables().gsub.map(SubstitutionTable::new),
            gpos: face.tables().gpos.map(PositioningTable::new),
            ttfp_face: face,
        })
    }

    /// Creates a new [`Face`] from [`ttf_parser::Face`].
    ///
    /// Data will be referenced, not owned.
    pub fn from_face(face: ttf_parser::Face<'a>) -> Self {
        let font = fonta::Font::new(face.raw_face().data, 0).unwrap();
        hb_font_t {
            font,
            units_per_em: face.units_per_em(),
            pixels_per_em: None,
            points_per_em: None,
            gsub: face.tables().gsub.map(SubstitutionTable::new),
            gpos: face.tables().gpos.map(PositioningTable::new),
            ttfp_face: face,
        }
    }

    // TODO: remove
    /// Returns face’s units per EM.
    #[inline]
    pub fn units_per_em(&self) -> i32 {
        self.units_per_em as i32
    }

    #[inline]
    pub(crate) fn pixels_per_em(&self) -> Option<(u16, u16)> {
        self.pixels_per_em
    }

    /// Sets pixels per EM.
    ///
    /// Used during raster glyphs processing and hinting.
    ///
    /// `None` by default.
    #[inline]
    pub fn set_pixels_per_em(&mut self, ppem: Option<(u16, u16)>) {
        self.pixels_per_em = ppem;
    }

    /// Sets point size per EM.
    ///
    /// Used for optical-sizing in Apple fonts.
    ///
    /// `None` by default.
    #[inline]
    pub fn set_points_per_em(&mut self, ptem: Option<f32>) {
        self.points_per_em = ptem;
    }

    /// Sets font variations.
    pub fn set_variations(&mut self, variations: &[Variation]) {
        for variation in variations {
            self.set_variation(variation.tag, variation.value);
        }
        self.font.set_coords(self.ttfp_face.variation_coordinates());
    }

    pub(crate) fn has_glyph(&self, c: u32) -> bool {
        self.get_nominal_glyph(c).is_some()
    }

    pub(crate) fn get_nominal_glyph(&self, c: u32) -> Option<GlyphId> {
        self.font
            .nominal_glyph(c)
            .map(|gid| GlyphId(gid.to_u32() as u16)) // TODO: remove as u16 when fully on read-fonts GlyphId
    }

    pub(crate) fn glyph_variation_index(&self, c: char, vs: char) -> Option<GlyphId> {
        self.font
            .nominal_variant_glyph(c as u32, vs as u32)
            .map(|gid| GlyphId(gid.to_u32() as u16)) // TODO: remove as u16 when fully on read-fonts GlyphId
    }

    pub(crate) fn glyph_h_advance(&self, glyph: GlyphId) -> i32 {
        self.glyph_advance(glyph, false) as i32
    }

    pub(crate) fn glyph_v_advance(&self, glyph: GlyphId) -> i32 {
        -(self.glyph_advance(glyph, true) as i32)
    }

    fn glyph_advance(&self, glyph: GlyphId, is_vertical: bool) -> u32 {
        let face = &self.ttfp_face;
        if face.is_variable()
            && face.has_non_default_variation_coordinates()
            && face.tables().hvar.is_none()
            && face.tables().vvar.is_none()
            && face.glyph_phantom_points(glyph).is_none()
        {
            return match face.glyph_bounding_box(glyph) {
                Some(bbox) => {
                    (if is_vertical {
                        bbox.y_max + bbox.y_min
                    } else {
                        bbox.x_max + bbox.x_min
                    }) as u32
                }
                None => 0,
            };
        }

        if is_vertical {
            if face.tables().vmtx.is_some() {
                return face.glyph_ver_advance(glyph).unwrap_or(0) as u32;
            } else {
                // TODO: Original code calls `h_extents_with_fallback`
                return (face.ascender() - face.descender()) as u32;
            }
        } else if !is_vertical && face.tables().hmtx.is_some() {
            face.glyph_hor_advance(glyph).unwrap_or(0) as u32
        } else {
            face.units_per_em() as u32
        }
    }

    pub(crate) fn glyph_h_origin(&self, glyph: GlyphId) -> i32 {
        self.glyph_h_advance(glyph) / 2
    }

    pub(crate) fn glyph_v_origin(&self, glyph: GlyphId) -> i32 {
        match self.ttfp_face.glyph_y_origin(glyph) {
            Some(y) => i32::from(y),
            None => {
                let mut extents = hb_glyph_extents_t::default();
                if self.glyph_extents(glyph, &mut extents) {
                    if self.ttfp_face.tables().vmtx.is_some() {
                        extents.y_bearing + self.glyph_side_bearing(glyph, true)
                    } else {
                        let advance = self.ttfp_face.ascender() - self.ttfp_face.descender();
                        let diff = advance as i32 - -extents.height;
                        return extents.y_bearing + (diff >> 1);
                    }
                } else {
                    // TODO: Original code calls `h_extents_with_fallback`
                    self.ttfp_face.ascender() as i32
                }
            }
        }
    }

    pub(crate) fn glyph_side_bearing(&self, glyph: GlyphId, is_vertical: bool) -> i32 {
        let face = &self.ttfp_face;
        if face.is_variable() && face.tables().hvar.is_none() && face.tables().vvar.is_none() {
            return match face.glyph_bounding_box(glyph) {
                Some(bbox) => (if is_vertical { bbox.x_min } else { bbox.y_min }) as i32,
                None => 0,
            };
        }

        if is_vertical {
            face.glyph_ver_side_bearing(glyph).unwrap_or(0) as i32
        } else {
            face.glyph_hor_side_bearing(glyph).unwrap_or(0) as i32
        }
    }

    pub(crate) fn glyph_extents(
        &self,
        glyph: GlyphId,
        glyph_extents: &mut hb_glyph_extents_t,
    ) -> bool {
        let pixels_per_em = match self.pixels_per_em {
            Some(ppem) => ppem.0,
            None => core::u16::MAX,
        };

        if let Some(img) = self.ttfp_face.glyph_raster_image(glyph, pixels_per_em) {
            // HarfBuzz also supports only PNG.
            if img.format == ttf_parser::RasterImageFormat::PNG {
                let scale = self.units_per_em as f32 / img.pixels_per_em as f32;
                glyph_extents.x_bearing = (f32::from(img.x) * scale).round() as i32;
                glyph_extents.y_bearing =
                    ((f32::from(img.y) + f32::from(img.height)) * scale).round() as i32;
                glyph_extents.width = (f32::from(img.width) * scale).round() as i32;
                glyph_extents.height = (-f32::from(img.height) * scale).round() as i32;
                return true;
            }
        // TODO: Add tests for this. We should use all glyphs from
        // https://github.com/googlefonts/color-fonts/blob/main/fonts/test_glyphs-glyf_colr_1_no_cliplist.ttf
        // and test their output against harfbuzz.
        } else if let Some(colr) = self.ttfp_face.tables().colr {
            if colr.is_simple() {
                return false;
            }

            if let Some(clip_box) = colr.clip_box(glyph, self.variation_coordinates()) {
                // Floor
                glyph_extents.x_bearing = (clip_box.x_min).round() as i32;
                glyph_extents.y_bearing = (clip_box.y_max).round() as i32;
                glyph_extents.width = (clip_box.x_max - clip_box.x_min).round() as i32;
                glyph_extents.height = (clip_box.y_min - clip_box.y_max).round() as i32;
                return true;
            }

            let mut extents_data = hb_paint_extents_context_t::new(&self.ttfp_face);
            let ret = colr
                .paint(
                    glyph,
                    0,
                    &mut extents_data,
                    self.variation_coordinates(),
                    RgbaColor::new(0, 0, 0, 0),
                )
                .is_some();

            let e = extents_data.get_extents();
            if e.is_void() {
                glyph_extents.x_bearing = 0;
                glyph_extents.y_bearing = 0;
                glyph_extents.width = 0;
                glyph_extents.height = 0;
            } else {
                glyph_extents.x_bearing = e.x_min as i32;
                glyph_extents.y_bearing = e.y_max as i32;
                glyph_extents.width = (e.x_max - e.x_min) as i32;
                glyph_extents.height = (e.y_min - e.y_max) as i32;
            }

            return ret;
        }

        let mut bbox = None;

        if let Some(glyf) = self.ttfp_face.tables().glyf {
            bbox = glyf.bbox(glyph);
        }

        // See https://github.com/RazrFalcon/harfruzz/pull/98#issuecomment-1948430785
        if self.ttfp_face.tables().glyf.is_some() && bbox.is_none() {
            // Empty glyph; zero extents.
            return true;
        }

        let Some(bbox) = bbox else {
            return false;
        };

        glyph_extents.x_bearing = i32::from(bbox.x_min);
        glyph_extents.y_bearing = i32::from(bbox.y_max);
        glyph_extents.width = i32::from(bbox.width());
        glyph_extents.height = i32::from(bbox.y_min - bbox.y_max);

        return true;
    }

    pub(crate) fn glyph_name(&self, glyph: GlyphId) -> Option<&str> {
        self.ttfp_face.glyph_name(glyph)
    }

    pub(crate) fn glyph_props(&self, glyph: GlyphId) -> u16 {
        let table = match self.tables().gdef {
            Some(v) => v,
            None => return 0,
        };

        match table.glyph_class(glyph) {
            Some(GlyphClass::Base) => GlyphPropsFlags::BASE_GLYPH.bits(),
            Some(GlyphClass::Ligature) => GlyphPropsFlags::LIGATURE.bits(),
            Some(GlyphClass::Mark) => {
                let class = table.glyph_mark_attachment_class(glyph);
                (class << 8) | GlyphPropsFlags::MARK.bits()
            }
            _ => 0,
        }
    }

    pub(crate) fn layout_table(&self, table_index: TableIndex) -> Option<&LayoutTable<'a>> {
        match table_index {
            TableIndex::GSUB => self.gsub.as_ref().map(|table| &table.inner),
            TableIndex::GPOS => self.gpos.as_ref().map(|table| &table.inner),
        }
    }

    pub(crate) fn layout_tables(
        &self,
    ) -> impl Iterator<Item = (TableIndex, &LayoutTable<'a>)> + '_ {
        TableIndex::iter().filter_map(move |idx| self.layout_table(idx).map(|table| (idx, table)))
    }
}

#[derive(Clone, Copy, Default, Zeroable, Pod)]
#[repr(C)]
pub struct hb_glyph_extents_t {
    pub x_bearing: i32,
    pub y_bearing: i32,
    pub width: i32,
    pub height: i32,
}
