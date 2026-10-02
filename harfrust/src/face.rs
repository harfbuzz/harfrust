use super::aat::AatData;
use super::buffer::Buffer;
use super::ot::OtData;
use super::shape::shape_with_font;
use crate::shape::font_ref::FontRefData;
use crate::shape::font_ref::{Charmap, GlyphMetrics, GlyphNames};
use crate::shape::CharmapCache;
pub use crate::shape::{Advances, GlyphExtents, NominalGlyphs, Scale};
use crate::shape::{LayoutCache, LayoutData, ShapeOptions, ShaperFont};
use crate::tables::TableRanges;
use crate::{
    ContentType, Direction, Feature, GlyphBuffer, NormalizedCoord, ShapeError, ShapePlan,
    UnicodeBuffer,
};
use alloc::boxed::Box;

pub use super::font_funcs::{BuiltinFontFuncs, FontFuncs, RawAdvances, RawNominalGlyphs};

// The new-font path keeps only the basic metrics needed to construct a
// shaper, leaving the legacy table ranges in ShaperData.
struct InstanceCache {
    layout: LayoutCache,
    metrics: BasicFontMetrics,
}

impl InstanceCache {
    fn new(font: &crate::font::Font) -> Self {
        let tables = font.tables();
        let apply_trak = tables.trak_data().is_some() && tables.stat_data().is_some();
        let layout = LayoutCache::new(&tables, apply_trak);
        let table_ranges = TableRanges::from_tables(&tables);
        let metrics = BasicFontMetrics {
            units_per_em: table_ranges.units_per_em,
            num_glyphs: table_ranges.num_glyphs,
            ascent: table_ranges.ascent,
            descent: table_ranges.descent,
        };
        Self { layout, metrics }
    }
}

/// Shapes the buffer content using provided options.
///
/// Consumes the buffer. You can then run [`GlyphBuffer::clear`] to get the [`UnicodeBuffer`] back
/// without allocating a new one.
///
/// If a plan is provided, it is up to the caller to ensure that the shape plan matches the
/// properties of the provided buffer, otherwise the shaping result will likely be incorrect.
///
/// # Panics
///
/// Will panic when debugging assertions are enabled if the buffer and plan have mismatched
/// properties.
#[cfg(feature = "experimental_font_api")]
pub fn shape(
    font: &crate::font::FontInstance,
    buffer: UnicodeBuffer,
    mut options: ShapeOptions<'_>,
) -> GlyphBuffer {
    let shaper = Shaper::from_font(font);
    let Some(shaper) = shaper.as_ref() else {
        let mut buffer = buffer;
        buffer.clear();
        return GlyphBuffer(buffer.0);
    };
    // If the user didn't request an explicit scale but the font instance
    // has a size, set the scale to that size with 16 fractional bits.
    if options.scale.is_none() {
        if let Some(ppem) = font.size() {
            options = options.scale(Some((ppem * 65536.0) as i32));
        }
    }
    let mut buffer = buffer.0;
    // As above, this signature cannot report a failure.
    if let Err(err) = shaper.shape_buffer_inner(&mut buffer, options) {
        panic!("{err}");
    }
    GlyphBuffer(buffer)
}

// This will go away completely when we drop the old API.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum FontKind<'a> {
    FontRef(FontRefData<'a>),
    FontInstance(&'a crate::font::FontInstance, BasicFontMetrics),
}

#[derive(Copy, Clone, Debug)]
pub struct BasicFontMetrics {
    pub units_per_em: u16,
    pub num_glyphs: u32,
    pub ascent: i16,
    pub descent: i16,
}

/// A configured shaper.
#[derive(Clone)]
pub struct Shaper<'a> {
    pub(crate) font: FontKind<'a>,
    pub(crate) units_per_em: u16,
    pub(crate) cmap_cache: &'a CharmapCache,
    pub(crate) glyph_metrics: Option<GlyphMetrics<'a>>,
    pub(crate) charmap: Option<Charmap<'a>>,
    pub(crate) ot_data: OtData<'a>,
    pub(crate) aat_data: AatData<'a>,
    pub(crate) apply_trak: bool,
}

impl<'a> LayoutData<'a> {
    pub(crate) fn from_shaper(shaper: &'a Shaper<'a>) -> Self {
        Self {
            ot: &shaper.ot_data,
            aat: &shaper.aat_data,
            units_per_em: shaper.units_per_em,
            apply_trak: shaper.apply_trak,
        }
    }
}

pub trait AnyFont {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&Shaper>) -> R;
}

impl AnyFont for Shaper<'_> {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&Shaper>) -> R,
    {
        f(Some(self))
    }
}

impl AnyFont for crate::font::FontInstance {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&Shaper>) -> R,
    {
        let shaper = Shaper::from_font(self);
        f(shaper.as_ref())
    }
}

impl ShapePlan {
    /// Returns a plan that can be used for shaping any buffer with the
    /// provided properties.
    pub fn new(
        font: &impl AnyFont,
        direction: Direction,
        script: Option<crate::Script>,
        language: Option<&crate::Language>,
        user_features: &[Feature],
    ) -> Self {
        font.with_font(|font| {
            let font = font.expect("font should be available for shaping");
            Self::from_layout(
                LayoutData::from_shaper(font),
                direction,
                script,
                language,
                user_features,
            )
        })
    }
}

impl<'a> Shaper<'a> {
    /// The callbacks that read the font's own tables, which shaping falls
    /// back on when nothing else answers.
    ///
    /// These know about the legacy cmap subtables -- Macintosh Roman, and the
    /// Windows symbol encoding's private-use pages -- so a caller looking a
    /// glyph up outside shaping finds the same one shaping would.
    pub fn builtin_font_funcs(&'a self) -> BuiltinFontFuncs<'a> {
        match &self.font {
            FontKind::FontRef(font) => BuiltinFontFuncs::from_legacy(
                &font.glyph_metrics,
                &font.charmap,
                self.ot_data.coords,
                self.units_per_em,
                self.cmap_cache,
            ),
            FontKind::FontInstance(instance, metrics) => BuiltinFontFuncs::from_instance(
                instance,
                *metrics,
                self.glyph_metrics.as_ref(),
                self.charmap.as_ref(),
                self.ot_data.coords,
                self.units_per_em,
                self.cmap_cache,
            ),
        }
    }

    /// Builds a shaper for the font instance, reusing the instance's cached
    /// shaping data.
    ///
    /// The shaper borrows the instance; callers that shape repeatedly can
    /// build it once and reuse it across calls.
    #[cfg(feature = "experimental_font_api")]
    pub fn from_font_instance(font: &'a crate::font::FontInstance) -> Option<Self> {
        Self::from_font(font)
    }

    /// Preloads the table views used by the built-in font functions.
    ///
    /// This is an internal hook for bridges that cache a prepared shaper and
    /// know that shaping will use the built-in functions.
    #[doc(hidden)]
    #[cfg(feature = "experimental_font_api")]
    pub fn preload_builtin_font_data(&mut self) {
        let FontKind::FontInstance(instance, metrics) = &self.font else {
            return;
        };
        let tables = instance.tables();
        self.glyph_metrics = Some(GlyphMetrics::from_tables(&tables, metrics));
        self.charmap = Some(Charmap::from_tables(&tables));
    }

    pub(crate) fn from_font(font: &'a crate::font::FontInstance) -> Option<Self> {
        let tables = font.tables();
        let data = crate::font::_font_interop::_get_or_init_shaping_data(font, || {
            Box::new(InstanceCache::new(font))
        })
        .downcast_ref::<InstanceCache>()?;
        let cache = &data.layout;
        let metrics = data.metrics;
        let coords = font.normalized_coords();
        let feature_variations = if coords.is_empty() {
            [None; 2]
        } else {
            let feature_variations = font.feature_variations();
            [feature_variations.gsub(), feature_variations.gpos()]
        };
        let ot_data = OtData::from_tables(&tables, &cache.ot, coords, feature_variations);
        let aat_data = AatData::from_tables(&tables, &cache.aat);
        Some(Self {
            font: FontKind::FontInstance(font, metrics),
            units_per_em: metrics.units_per_em,
            cmap_cache: &cache.cmap,
            glyph_metrics: None,
            charmap: None,
            ot_data,
            aat_data,
            apply_trak: cache.apply_trak,
        })
    }

    /// Returns font's units per EM.
    #[inline]
    pub fn units_per_em(&self) -> i32 {
        self.units_per_em as i32
    }

    /// Returns the currently active normalized coordinates.
    pub fn coords(&self) -> &'a [NormalizedCoord] {
        self.ot_data.coords
    }

    /// Shapes the buffer content using provided options.
    ///
    /// Consumes the buffer. You can then run [`GlyphBuffer::clear`] to get the [`UnicodeBuffer`] back
    /// without allocating a new one.
    ///
    /// If a plan is provided, it is up to the caller to ensure that the shape plan matches the
    /// properties of the provided buffer, otherwise the shaping result will likely be incorrect.
    ///
    /// # Panics
    ///
    /// Will panic when debugging assertions are enabled if the buffer and plan have mismatched
    /// properties.    
    pub fn shape(&self, buffer: UnicodeBuffer, options: ShapeOptions<'_>) -> GlyphBuffer {
        let mut buffer = buffer.0;
        // This signature cannot report a failure, and every way shaping can
        // fail is a programming error, so panic.
        if let Err(err) = self.shape_buffer_inner(&mut buffer, options) {
            panic!("{err}");
        }
        GlyphBuffer(buffer)
    }

    /// Shapes a buffer in place using this prepared shaper.
    ///
    /// On success the buffer holds [`ContentType::Glyphs`]. If a plan
    /// is supplied through [`ShapeOptions::plan`] it must have been built for
    /// this buffer's direction and script.
    ///
    /// # Errors
    ///
    /// Returns a [`ShapeError`] when the buffer has already been shaped, has
    /// no direction and no plan, or does not match the supplied plan.
    #[cfg(feature = "experimental_font_api")]
    pub fn shape_buffer(
        &self,
        buffer: &mut Buffer,
        options: ShapeOptions<'_>,
    ) -> Result<(), ShapeError> {
        self.shape_buffer_inner(buffer, options)
    }

    pub(crate) fn shape_buffer_inner(
        &self,
        buffer: &mut Buffer,
        options: ShapeOptions<'_>,
    ) -> Result<(), ShapeError> {
        if buffer.content_type == Some(ContentType::Glyphs) {
            return Err(ShapeError::AlreadyShaped);
        }
        if let Some(plan) = options.plan {
            self.shape_with_plan(plan, buffer, options)
        } else {
            // Compiling a plan requires a direction, and asserts on its own if
            // it does not get one.
            if buffer.direction == Direction::Invalid {
                return Err(ShapeError::DirectionUnset);
            }
            let plan = ShapePlan::new(
                self,
                buffer.direction,
                buffer.script,
                buffer.language.as_ref(),
                options.features,
            );
            self.shape_with_plan(&plan, buffer, options)
        }
    }

    fn shape_with_plan(
        &self,
        plan: &ShapePlan,
        buffer: &mut Buffer,
        options: ShapeOptions<'_>,
    ) -> Result<(), ShapeError> {
        let scale = Scale::new(options.scale, self.units_per_em as i32);
        let font = ShaperFont::new(
            LayoutData::from_shaper(self),
            self.builtin_font_funcs(),
            scale,
            options.font_funcs,
        );
        shape_with_font(plan, &font, buffer, options.features, options.point_size)
    }

    /// The names the face gives its glyphs, from `post` or from the CFF
    /// charset, or nothing when it names none.
    pub fn glyph_names(&self) -> GlyphNames<'a> {
        GlyphNames::new(&self.font)
    }

    pub(crate) fn glyph_metrics(&self) -> GlyphMetrics<'a> {
        if let Some(metrics) = &self.glyph_metrics {
            return metrics.clone();
        }
        match &self.font {
            FontKind::FontRef(data) => data.glyph_metrics.clone(),
            FontKind::FontInstance(instance, metrics) => {
                GlyphMetrics::from_tables(&instance.tables(), metrics)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tag;
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
