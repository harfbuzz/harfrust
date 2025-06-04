#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;
use read_fonts::types::{F2Dot14, Fixed, GlyphId};
use read_fonts::{FontRef, TableProvider};
use smallvec::SmallVec;

use super::aat::AatTables;
use super::buffer::GlyphPropsFlags;
use super::charmap::{cache_t as cmap_cache_t, Charmap};
use super::glyph_metrics::GlyphMetrics;
use super::glyph_names::GlyphNames;
use super::ot::{LayoutTable, OtCache, OtTables};
use super::ot_layout::TableIndex;
use super::ot_shape::{hb_ot_shape_context_t, shape_internal};
use super::ot_shape_plan::hb_ot_shape_plan_t;
use crate::{script, Feature, GlyphBuffer, NormalizedCoord, UnicodeBuffer, Variation};

/// Data required for shaping with a single font.
pub struct ShaperData {
    ot_cache: OtCache,
    cmap_cache: cmap_cache_t,
}

impl ShaperData {
    /// Creates new cached shaper data for the given font.
    pub fn new(font: &FontRef) -> Self {
        let ot_cache = OtCache::new(font);
        let cmap_cache = cmap_cache_t::new();
        Self {
            ot_cache,
            cmap_cache,
        }
    }

    /// Returns a builder for constructing a new shaper with the given
    /// font.
    pub fn shaper<'a>(&'a self, font: &FontRef<'a>) -> ShaperBuilder<'a> {
        ShaperBuilder {
            data: self,
            font: font.clone(),
            instance: None,
            point_size: None,
        }
    }
}

/// An instance of a variable font.
#[derive(Clone, Default, Debug)]
pub struct ShaperInstance {
    coords: SmallVec<[F2Dot14; 8]>,
    // TODO: this is a good place to hang variation specific caches
}

impl ShaperInstance {
    /// Creates a new shaper instance for the given font from the specified
    /// list of variation settings.
    pub fn from_variations<V>(font: &FontRef, variations: V) -> Self
    where
        V: IntoIterator,
        V::Item: Into<Variation>,
    {
        let mut this = Self::default();
        this.set_variations(font, variations);
        this
    }

    /// Creates a new shaper instance for the given font from the specified
    /// set of normalized coordinates.
    pub fn from_coords(font: &FontRef, coords: impl Iterator<Item = NormalizedCoord>) -> Self {
        let mut this = Self::default();
        this.set_coords(font, coords);
        this
    }

    /// Returns the underlying set of normalized coordinates.
    pub fn coords(&self) -> &[F2Dot14] {
        &self.coords
    }

    /// Resets the instance for the given font and variation settings.
    pub fn set_variations<V>(&mut self, font: &FontRef, variations: V)
    where
        V: IntoIterator,
        V::Item: Into<Variation>,
    {
        self.coords.clear();
        if let Ok(fvar) = font.fvar() {
            self.coords
                .resize(fvar.axis_count() as usize, F2Dot14::ZERO);
            fvar.user_to_normalized(
                font.avar().ok().as_ref(),
                variations
                    .into_iter()
                    .map(|var| var.into())
                    .map(|var| (var.tag, Fixed::from_f64(var.value as _))),
                self.coords.as_mut_slice(),
            );
            self.check_default();
        }
    }

    /// Resets the instance for the given font and normalized coordinates.
    pub fn set_coords(&mut self, font: &FontRef, coords: impl Iterator<Item = F2Dot14>) {
        self.coords.clear();
        if let Ok(fvar) = font.fvar() {
            let count = fvar.axis_count() as usize;
            self.coords.reserve(count);
            self.coords.extend(coords.take(count));
            self.check_default();
        }
    }

    fn check_default(&mut self) {
        if self.coords.iter().all(|coord| *coord == F2Dot14::ZERO) {
            self.coords.clear();
        }
    }
}

/// Builder type for constructing a [`Shaper`].
pub struct ShaperBuilder<'a> {
    data: &'a ShaperData,
    font: FontRef<'a>,
    instance: Option<&'a ShaperInstance>,
    point_size: Option<f32>,
}

impl<'a> ShaperBuilder<'a> {
    /// Sets an optional instance for the shaper.
    ///
    /// This defines the variable font configuration.
    pub fn instance(mut self, instance: &'a ShaperInstance) -> Self {
        self.instance = Some(instance);
        self
    }

    /// Sets the point size for the shaper.
    ///
    /// This controls adjustments provided by the `trak` (tracking) table.
    pub fn point_size(mut self, size: f32) -> Self {
        self.point_size = Some(size);
        self
    }

    /// Builds the shaper with the current configuration.
    pub fn build(self) -> hb_font_t<'a> {
        let font = self.font;
        let units_per_em = font.head().map(|head| head.units_per_em()).unwrap_or(1000);
        let charmap = Charmap::new(&font, &self.data.cmap_cache);
        let glyph_metrics = GlyphMetrics::new(&font);
        let coords = self
            .instance
            .map(|instance| instance.coords())
            .unwrap_or_default();
        let ot_tables = OtTables::new(&font, &self.data.ot_cache, coords);
        let aat_tables = AatTables::new(&font);
        hb_font_t {
            font,
            units_per_em,
            pixels_per_em: None,
            points_per_em: self.point_size,
            charmap,
            glyph_metrics,
            ot_tables,
            aat_tables,
        }
    }
}

/// A configured shaper.
#[derive(Clone)]
pub struct hb_font_t<'a> {
    pub(crate) font: FontRef<'a>,
    pub(crate) units_per_em: u16,
    pixels_per_em: Option<(u16, u16)>,
    pub(crate) points_per_em: Option<f32>,
    charmap: Charmap<'a>,
    glyph_metrics: GlyphMetrics<'a>,
    pub(crate) ot_tables: OtTables<'a>,
    pub(crate) aat_tables: AatTables<'a>,
}

impl<'a> hb_font_t<'a> {
    /// Returns face’s units per EM.
    #[inline]
    pub fn units_per_em(&self) -> i32 {
        self.units_per_em as i32
    }

    /// Returns the currently active normalized coordinates.
    pub fn coords(&self) -> &'a [NormalizedCoord] {
        self.ot_tables.coords
    }

    /// Shapes the buffer content using provided font and features.
    ///
    /// Consumes the buffer. You can then run [`GlyphBuffer::clear`] to get the [`UnicodeBuffer`] back
    /// without allocating a new one.
    ///
    /// If you plan to shape multiple strings using the same [`Face`] prefer [`shape_with_plan`].
    /// This is because [`ShapePlan`] initialization is pretty slow and should preferably be called
    /// once for each [`Face`].
    pub fn shape(&self, features: &[Feature], mut buffer: UnicodeBuffer) -> GlyphBuffer {
        buffer.0.guess_segment_properties();
        let plan = hb_ot_shape_plan_t::new(
            self,
            buffer.0.direction,
            buffer.0.script,
            buffer.0.language.as_ref(),
            features,
        );
        self.shape_with_plan(&plan, buffer)
    }

    /// Shapes the buffer content using the provided font and plan.
    ///
    /// Consumes the buffer. You can then run [`GlyphBuffer::clear`] to get the [`UnicodeBuffer`] back
    /// without allocating a new one.
    ///
    /// It is up to the caller to ensure that the shape plan matches the properties of the provided
    /// buffer, otherwise the shaping result will likely be incorrect.
    ///
    /// # Panics
    ///
    /// Will panic when debugging assertions are enabled if the buffer and plan have mismatched
    /// properties.
    pub fn shape_with_plan(&self, plan: &hb_ot_shape_plan_t, buffer: UnicodeBuffer) -> GlyphBuffer {
        let mut buffer = buffer.0;
        buffer.guess_segment_properties();

        buffer.enter();

        debug_assert_eq!(buffer.direction, plan.direction);
        debug_assert_eq!(
            buffer.script.unwrap_or(script::UNKNOWN),
            plan.script.unwrap_or(script::UNKNOWN)
        );

        if buffer.len > 0 {
            // Save the original direction, we use it later.
            let target_direction = buffer.direction;
            shape_internal(&mut hb_ot_shape_context_t {
                plan,
                face: self,
                buffer: &mut buffer,
                target_direction,
            });
        }

        GlyphBuffer(buffer)
    }

    #[inline]
    pub(crate) fn pixels_per_em(&self) -> Option<(u16, u16)> {
        self.pixels_per_em
    }

    pub(crate) fn has_glyph(&self, c: u32) -> bool {
        self.get_nominal_glyph(c).is_some()
    }

    pub(crate) fn get_nominal_glyph(&self, c: u32) -> Option<GlyphId> {
        self.charmap.map(c)
    }

    pub(crate) fn get_nominal_variant_glyph(&self, c: char, vs: char) -> Option<GlyphId> {
        self.charmap.map_variant(c as u32, vs as u32)
    }

    pub(crate) fn glyph_h_advance(&self, glyph: GlyphId) -> i32 {
        self.glyph_metrics
            .advance_width(glyph, self.ot_tables.coords)
            .unwrap_or_default()
    }

    pub(crate) fn glyph_v_advance(&self, glyph: GlyphId) -> i32 {
        -self
            .glyph_metrics
            .advance_height(glyph, self.ot_tables.coords)
            .unwrap_or(self.units_per_em as i32)
    }

    pub(crate) fn glyph_h_origin(&self, glyph: GlyphId) -> i32 {
        self.glyph_h_advance(glyph) / 2
    }

    pub(crate) fn glyph_v_origin(&self, glyph: GlyphId) -> i32 {
        self.glyph_metrics
            .v_origin(glyph, self.ot_tables.coords)
            .unwrap_or_default()
    }

    pub(crate) fn glyph_extents(
        &self,
        glyph: GlyphId,
        glyph_extents: &mut hb_glyph_extents_t,
    ) -> bool {
        if let Some(extents) = self.glyph_metrics.extents(glyph, self.ot_tables.coords) {
            glyph_extents.x_bearing = extents.x_min;
            glyph_extents.y_bearing = extents.y_max;
            glyph_extents.width = extents.x_max - extents.x_min;
            glyph_extents.height = extents.y_min - extents.y_max;
            true
        } else {
            false
        }
    }

    pub(crate) fn glyph_names(&self) -> GlyphNames<'a> {
        GlyphNames::new(&self.font)
    }

    pub(crate) fn glyph_props(&self, glyph: GlyphId) -> u16 {
        let glyph = glyph.to_u32();
        match self.ot_tables.glyph_class(glyph) {
            1 => GlyphPropsFlags::BASE_GLYPH.bits(),
            2 => GlyphPropsFlags::LIGATURE.bits(),
            3 => {
                let class = self.ot_tables.glyph_mark_attachment_class(glyph);
                (class << 8) | GlyphPropsFlags::MARK.bits()
            }
            _ => 0,
        }
    }

    pub(crate) fn layout_table(&self, table_index: TableIndex) -> Option<LayoutTable<'a>> {
        match table_index {
            TableIndex::GSUB => self
                .ot_tables
                .gsub
                .as_ref()
                .map(|table| LayoutTable::Gsub(table.table.clone())),
            TableIndex::GPOS => self
                .ot_tables
                .gpos
                .as_ref()
                .map(|table| LayoutTable::Gpos(table.table.clone())),
        }
    }

    pub(crate) fn layout_tables(&self) -> impl Iterator<Item = (TableIndex, LayoutTable<'a>)> + '_ {
        TableIndex::iter().filter_map(move |idx| self.layout_table(idx).map(|table| (idx, table)))
    }
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct hb_glyph_extents_t {
    pub x_bearing: i32,
    pub y_bearing: i32,
    pub width: i32,
    pub height: i32,
}
