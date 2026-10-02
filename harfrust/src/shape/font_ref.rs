use read_fonts::ps::cff::charset::Charset;
use read_fonts::tables::{
    cff::Cff,
    cmap::{Cmap, Cmap14, CmapSubtable, MapVariant},
    glyf::Glyf,
    gvar::Gvar,
    hmtx::{Hmtx, LongMetric},
    hvar::Hvar,
    loca::Loca,
    mvar::Mvar,
    post::Post,
    vmtx::Vmtx,
    vorg::Vorg,
    vvar::Vvar,
};
use read_fonts::types::{BoundingBox, F2Dot14, Fixed, GlyphId, Point};
use read_fonts::{FontRef, TableProvider};
use smallvec::SmallVec;

use crate::aat::AatData;
use crate::face::{BasicFontMetrics, FontKind, Scale, Shaper};
use crate::ot::{LayoutTable, OtData};
use crate::shape::font::LayoutCache;
use crate::tables::{legacy_symbol_font_page, SelectedCmapSubtable, TableRanges};
use crate::{GlyphExtents, GlyphInfo, GlyphPosition, Tag};
use crate::{NormalizedCoord, ShapePlanKey, Variation};

pub(crate) struct LegacyFont<'a> {
    pub(crate) glyph_metrics: &'a GlyphMetrics<'a>,
    pub(crate) charmap: &'a Charmap<'a>,
}

/// Data required for shaping with a single font.
pub struct ShaperData {
    table_ranges: TableRanges,
    cache: LayoutCache,
}

impl ShaperData {
    /// Creates new cached shaper data for the given font.
    pub fn new(font: &FontRef) -> Self {
        let apply_trak = font.trak().is_ok() && font.stat().is_ok();
        let cache = LayoutCache::new(font, apply_trak);
        let table_ranges = TableRanges::new(font);
        Self {
            table_ranges,
            cache,
        }
    }

    /// Returns a builder for constructing a new shaper with the given
    /// font.
    pub fn shaper<'a>(&'a self, font: &FontRef<'a>) -> ShaperBuilder<'a> {
        ShaperBuilder {
            data: self,
            font: font.clone(),
            instance: None,
        }
    }
}

// Maximum number of coordinates to store inline before spilling to the
// heap.
//
// Any value between 5 and 11 yields a SmallVec footprint of 32 bytes.
const MAX_INLINE_COORDS: usize = 11;

/// An instance of a variable font.
#[derive(Clone, Default, Debug)]
pub struct ShaperInstance {
    coords: SmallVec<[F2Dot14; MAX_INLINE_COORDS]>,
    pub(crate) feature_variations: [Option<u32>; 2],
    // TODO: this is a good place to hang variation specific caches
}

impl ShaperInstance {
    /// Creates a new shaper instance for the given font from the specified
    /// list of variation settings.
    ///
    /// The setting values are in user space and the order is insignificant.
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
    ///
    /// The sequence of coordinates is expected to be in axis order.
    pub fn from_coords(font: &FontRef, coords: impl IntoIterator<Item = NormalizedCoord>) -> Self {
        let mut this = Self::default();
        this.set_coords(font, coords);
        this
    }

    /// Creates a new shaper instance for the given font using the variation
    /// position from the named instance at the specified index.
    pub fn from_named_instance(font: &FontRef, index: usize) -> Self {
        let mut this = Self::default();
        this.set_named_instance(font, index);
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
                    .map(Into::into)
                    .map(|var| (var.tag, Fixed::from_f64(var.value as _))),
                self.coords.as_mut_slice(),
            );
            self.check_default();
            self.set_feature_variations(font);
        }
    }

    /// Resets the instance for the given font and normalized coordinates.
    pub fn set_coords(&mut self, font: &FontRef, coords: impl IntoIterator<Item = F2Dot14>) {
        self.coords.clear();
        if let Ok(fvar) = font.fvar() {
            let count = fvar.axis_count() as usize;
            self.coords.reserve(count);
            self.coords.extend(coords.into_iter().take(count));
            self.check_default();
            self.set_feature_variations(font);
        }
    }

    /// Resets the instance for the given font using the variation
    /// position from the named instance at the specified index.
    pub fn set_named_instance(&mut self, font: &FontRef, index: usize) {
        self.coords.clear();
        if let Ok(fvar) = font.fvar() {
            if let Ok((axes, instance)) = fvar
                .axis_instance_arrays()
                .and_then(|arrays| Ok((arrays.axes(), arrays.instances().get(index)?)))
            {
                self.set_variations(
                    font,
                    axes.iter()
                        .zip(instance.coordinates)
                        .map(|(axis, coord)| (axis.axis_tag(), coord.get().to_f32())),
                );
            }
        }
    }

    fn set_feature_variations(&mut self, font: &FontRef) {
        self.feature_variations = [None; 2];
        if self.coords.is_empty() {
            return;
        }
        self.feature_variations[0] = font
            .gsub()
            .ok()
            .and_then(|t| LayoutTable::Gsub(t).feature_variation_index(&self.coords));
        self.feature_variations[1] = font
            .gpos()
            .ok()
            .and_then(|t| LayoutTable::Gpos(t).feature_variation_index(&self.coords));
    }

    fn check_default(&mut self) {
        if self.coords.iter().all(|coord| *coord == F2Dot14::ZERO) {
            self.coords.clear();
        }
    }
}

/// Builder type for constructing a [`Shaper`](crate::Shaper).
pub struct ShaperBuilder<'a> {
    data: &'a ShaperData,
    font: FontRef<'a>,
    instance: Option<&'a ShaperInstance>,
}

impl<'a> ShaperBuilder<'a> {
    /// Sets an optional instance for the shaper.
    ///
    /// This defines the variable font configuration.
    pub fn instance(mut self, instance: Option<&'a ShaperInstance>) -> Self {
        self.instance = instance;
        self
    }

    /// Builds the shaper with the current configuration.
    pub fn build(self) -> Shaper<'a> {
        let font = self.font;
        let units_per_em = self.data.table_ranges.units_per_em;
        let charmap = Charmap::new(&font, &self.data.table_ranges);
        let glyph_metrics = GlyphMetrics::new(&font, &self.data.table_ranges);
        let (coords, feature_variations) = self
            .instance
            .map(|instance| (instance.coords(), instance.feature_variations))
            .unwrap_or_default();
        let ot_data = OtData::new(
            &font,
            &self.data.cache.ot,
            &self.data.table_ranges,
            coords,
            feature_variations,
        );
        let aat_data = AatData::new(&font, &self.data.cache.aat, &self.data.table_ranges);
        let font_data = FontRefData {
            font,
            glyph_metrics,
            charmap,
        };
        let glyph_metrics = Some(font_data.glyph_metrics.clone());
        let charmap = Some(font_data.charmap.clone());
        let font = FontKind::FontRef(font_data);
        Shaper {
            font,
            units_per_em,
            cmap_cache: &self.data.cache.cmap,
            glyph_metrics,
            charmap,
            ot_data,
            aat_data,
            apply_trak: self.data.cache.apply_trak,
        }
    }
}

#[derive(Clone)]
pub struct FontRefData<'a> {
    pub(crate) font: FontRef<'a>,
    pub(crate) glyph_metrics: GlyphMetrics<'a>,
    pub(crate) charmap: Charmap<'a>,
}

impl ShapePlanKey<'_> {
    /// Sets the instance to use for this shape plan key.
    pub fn instance(mut self, instance: Option<&ShaperInstance>) -> Self {
        self.feature_variations = instance
            .map(|instance| instance.feature_variations)
            .unwrap_or_default();
        self
    }
}

#[derive(Clone)]
pub struct Charmap<'a> {
    subtable: Option<(SelectedCmapSubtable, CmapSubtable<'a>)>,
    vs_subtable: Option<Cmap14<'a>>,
}

impl<'a> Charmap<'a> {
    pub fn new(font: &FontRef<'a>, table_ranges: &TableRanges) -> Self {
        if let Some(cmap) = table_ranges.cmap.resolve_table::<Cmap>(font) {
            let data = cmap.offset_data();
            let records = cmap.encoding_records();
            let subtable = table_ranges
                .cmap_subtable
                .and_then(|s| Some((s, records.get(s.index as usize)?.subtable(data).ok()?)));
            let vs_subtable = table_ranges
                .cmap_vs_subtable
                .and_then(|index| records.get(index as usize))
                .and_then(|rec| rec.subtable(data).ok())
                .and_then(|subtable| match subtable {
                    CmapSubtable::Format14(table) => Some(table),
                    _ => None,
                });
            Self {
                subtable,
                vs_subtable,
            }
        } else {
            Self {
                subtable: None,
                vs_subtable: None,
            }
        }
    }

    pub fn from_tables(font: &impl TableProvider<'a>) -> Self {
        if let Ok(cmap) = font.cmap() {
            let subtable = if let Some((index, record, subtable)) = cmap.best_subtable() {
                Some((
                    SelectedCmapSubtable {
                        index,
                        is_mac_roman: record.is_mac_roman(),
                        is_symbol: record.is_symbol(),
                        symbol_font_page: legacy_symbol_font_page(font.os2().ok().as_ref()),
                    },
                    subtable,
                ))
            } else {
                None
            };
            Self {
                subtable,
                vs_subtable: cmap.uvs_subtable().map(|(_, subtable)| subtable),
            }
        } else {
            Self {
                subtable: None,
                vs_subtable: None,
            }
        }
    }

    pub fn map(&self, mut c: u32) -> Option<GlyphId> {
        let subtable = self.subtable.as_ref()?;
        if subtable.0.is_mac_roman && c > 0x7F {
            c = unicode_to_macroman(c);
        }
        let result = subtable.1.map_codepoint(c);
        if result.is_none() && subtable.0.is_symbol {
            let mapped = match subtable.0.symbol_font_page {
                0xB200 => arabic_pua_map(c, true),
                0xB300 => arabic_pua_map(c, false),
                0 if c <= 0x00FF => 0xF000 + c,
                _ => 0,
            };
            if mapped != 0 {
                return subtable.1.map_codepoint(mapped);
            }
        }
        result
    }

    pub fn map_variant(&self, c: u32, vs: u32) -> Option<GlyphId> {
        let subtable = self.vs_subtable.as_ref()?;
        match subtable.map_variant(c, vs)? {
            MapVariant::UseDefault => self.map(c),
            MapVariant::Variant(gid) => Some(gid),
        }
    }
}

fn arabic_pua_map(c: u32, simplified: bool) -> u32 {
    let Ok(c) = usize::try_from(c) else {
        return 0;
    };
    let mapped = if simplified {
        crate::ot::shaper::arabic_pua::arabic_pua_simp_map(c)
    } else {
        crate::ot::shaper::arabic_pua::arabic_pua_trad_map(c)
    };
    u32::from(mapped)
}

#[rustfmt::skip]
static UNICODE_TO_MACROMAN: &[u16] = &[
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1,
    0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8,
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC,
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF,
    0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211,
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8,
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153,
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1,
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

fn unicode_to_macroman(c: u32) -> u32 {
    let u = c as u16;
    let Some(index) = UNICODE_TO_MACROMAN.iter().position(|m| *m == u) else {
        return 0;
    };
    (0x80 + index) as u32
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn maps_legacy_arabic_symbol_fonts() {
        let simplified = FontRef::new(include_bytes!(
            "../../tests/fonts/in-house/SimpArabicTest.ttf"
        ))
        .unwrap();
        let simplified_ranges = TableRanges::new(&simplified);
        assert_eq!(
            simplified_ranges.cmap_subtable.unwrap().symbol_font_page,
            0xB200
        );
        assert_eq!(
            Charmap::new(&simplified, &simplified_ranges).map(0x0627),
            Some(GlyphId::new(45))
        );

        let traditional = FontRef::new(include_bytes!(
            "../../tests/fonts/in-house/TradArabicTest.ttf"
        ))
        .unwrap();
        let traditional_ranges = TableRanges::new(&traditional);
        assert_eq!(
            traditional_ranges.cmap_subtable.unwrap().symbol_font_page,
            0xB300
        );
        assert_eq!(
            Charmap::new(&traditional, &traditional_ranges).map(0x0627),
            Some(GlyphId::new(65))
        );
    }
}

#[derive(Clone, Default)]
pub struct GlyphMetrics<'a> {
    hmtx: Option<Hmtx<'a>>,
    h_metrics: &'a [LongMetric],
    hvar: Option<Hvar<'a>>,
    vmtx: Option<Vmtx<'a>>,
    vvar: Option<Vvar<'a>>,
    vorg: Option<Vorg<'a>>,
    glyf: Option<GlyfTables<'a>>,
    mvar: Option<Mvar<'a>>,
    num_glyphs: u32,
    upem: u16,
    ascent: i16,
    descent: i16,
}

#[derive(Clone)]
struct GlyfTables<'a> {
    loca: Loca<'a>,
    glyf: Glyf<'a>,
    gvar: Option<Gvar<'a>>,
}

impl<'a> GlyphMetrics<'a> {
    pub(crate) fn new(font: &FontRef<'a>, table_ranges: &TableRanges) -> Self {
        let num_glyphs = table_ranges.num_glyphs;
        let upem = table_ranges.units_per_em;
        let hmtx = table_ranges
            .hmtx
            .resolve_data(font)
            .and_then(|data| Hmtx::read(data, table_ranges.num_h_metrics).ok());
        let h_metrics = hmtx
            .as_ref()
            .map(|hmtx| hmtx.h_metrics())
            .unwrap_or_default();
        let hvar = table_ranges.hvar.resolve_table(font);
        let vmtx = table_ranges
            .vmtx
            .resolve_data(font)
            .and_then(|data| Vmtx::read(data, table_ranges.num_v_metrics).ok());
        let vvar = table_ranges.vvar.resolve_table(font);
        let vorg = table_ranges.vorg.resolve_table(font);
        let loca = table_ranges
            .loca
            .resolve_data(font)
            .and_then(|data| Loca::read(data, table_ranges.loca_long).ok());
        let glyf = table_ranges.glyf.resolve_table(font);
        let glyf = if let Some((loca, glyf)) = loca.zip(glyf) {
            let gvar = table_ranges.gvar.resolve_table(font);
            Some(GlyfTables { loca, glyf, gvar })
        } else {
            None
        };
        let mvar = table_ranges.mvar.resolve_table(font);
        let ascent = table_ranges.ascent;
        let descent = table_ranges.descent;
        Self {
            hmtx,
            h_metrics,
            hvar,
            vmtx,
            vvar,
            vorg,
            glyf,
            mvar,
            num_glyphs,
            upem,
            ascent,
            descent,
        }
    }

    pub(crate) fn from_tables(font: &impl TableProvider<'a>, metrics: &BasicFontMetrics) -> Self {
        let hmtx = font.hmtx().ok();
        let h_metrics = hmtx
            .as_ref()
            .map(|hmtx| hmtx.h_metrics())
            .unwrap_or_default();
        let hvar = font.hvar().ok();
        let vmtx = font.vmtx().ok();
        let vvar = font.vvar().ok();
        let vorg = font.vorg().ok();
        let loca = font.loca(None).ok();
        let glyf = font.glyf().ok();
        let glyf = if let Some((loca, glyf)) = loca.zip(glyf) {
            let gvar = font.gvar().ok();
            Some(GlyfTables { loca, glyf, gvar })
        } else {
            None
        };
        let mvar = font.mvar().ok();
        Self {
            hmtx,
            h_metrics,
            hvar,
            vmtx,
            vvar,
            vorg,
            glyf,
            mvar,
            num_glyphs: metrics.num_glyphs,
            upem: metrics.units_per_em,
            ascent: metrics.ascent,
            descent: metrics.descent,
        }
    }

    /// The horizontal advance before any variation is applied.
    ///
    /// A face with no horizontal metrics gives every glyph the same default,
    /// and past the last glyph the face has there is no advance to give --
    /// which is what a malformed cmap pointing beyond the glyph count asks
    /// for. HarfBuzz answers both the same way.
    fn plain_advance_width(&self, gid: GlyphId) -> i32 {
        if self.h_metrics.is_empty() {
            return self.upem as i32 / 2;
        }
        if gid.to_u32() >= self.num_glyphs {
            return 0;
        }
        self.h_metrics
            .get(gid.to_u32() as usize)
            .or_else(|| self.h_metrics.last())
            .map_or(0, |metric| metric.advance() as i32)
    }

    pub(crate) fn advance_width(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let mut advance = self.plain_advance_width(gid);
        if !coords.is_empty() {
            if let Some(hvar) = self.hvar.as_ref() {
                advance = advance.saturating_add(
                    hvar.advance_width_delta(gid, coords)
                        .unwrap_or_default()
                        .to_i32(),
                );
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                advance = advance
                    .saturating_add(deltas[1].x.to_i32().saturating_sub(deltas[0].x.to_i32()));
            }
        }
        Some(advance)
    }

    pub(crate) fn populate_advance_widths(
        &self,
        infos: &[GlyphInfo],
        pos: &mut [GlyphPosition],
        coords: &[F2Dot14],
        scale: Scale,
    ) {
        for (info, pos) in infos.iter().zip(pos.iter_mut()) {
            pos.x_advance = self.plain_advance_width(GlyphId::from(info.glyph_id));
        }
        if !coords.is_empty() {
            if let Some(hvar) = self.hvar.as_ref() {
                for (info, pos) in infos.iter().zip(pos.iter_mut()) {
                    pos.x_advance = pos.x_advance.saturating_add(
                        hvar.advance_width_delta(info.as_glyph(), coords)
                            .unwrap_or_default()
                            .to_i32(),
                    );
                }
            } else {
                for (info, pos) in infos.iter().zip(pos.iter_mut()) {
                    if let Some(deltas) = self.phantom_deltas(info.as_glyph(), coords) {
                        pos.x_advance = pos.x_advance.saturating_add(
                            deltas[1].x.to_i32().saturating_sub(deltas[0].x.to_i32()),
                        );
                    }
                }
            }
        }
        for pos in pos.iter_mut() {
            pos.x_advance = scale.scale_x(pos.x_advance);
        }
    }

    pub(crate) fn _left_side_bearing(
        &self,
        gid: impl Into<GlyphId>,
        coords: &[F2Dot14],
    ) -> Option<i32> {
        let gid = gid.into();
        let mut bearing = if let Some(hmtx) = self.hmtx.as_ref() {
            hmtx.side_bearing(gid).unwrap_or_default() as i32
        } else {
            let extents = self.bounds(gid, coords)?;
            return Some(extents.x_min);
        };
        if !coords.is_empty() {
            if let Some(hvar) = self.hvar.as_ref() {
                bearing = bearing
                    .saturating_add(hvar.lsb_delta(gid, coords).unwrap_or_default().to_i32());
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                bearing = bearing.saturating_add(deltas[0].x.to_i32());
            }
        }
        Some(bearing)
    }

    /// The vertical advance before any variation is applied, or `None` when
    /// the face carries no vertical metrics to read one from.
    ///
    /// As with the horizontal advance, past the last glyph the face has
    /// there is no advance to give.
    fn plain_advance_height(&self, gid: GlyphId) -> Option<i32> {
        let vmtx = self.vmtx.as_ref()?;
        if gid.to_u32() >= self.num_glyphs {
            return Some(0);
        }
        Some(vmtx.advance(gid)? as i32)
    }

    pub(crate) fn advance_height(
        &self,
        gid: impl Into<GlyphId>,
        coords: &[F2Dot14],
    ) -> Option<i32> {
        let gid = gid.into();
        let Some(mut advance) = self.plain_advance_height(gid) else {
            // No vertical metrics at all: every glyph is as tall as the face.
            return Some(self.ascent as i32 - self.descent as i32);
        };
        if !coords.is_empty() {
            if let Some(vvar) = self.vvar.as_ref() {
                advance = advance.saturating_add(
                    vvar.advance_height_delta(gid, coords)
                        .unwrap_or_default()
                        .to_i32(),
                );
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                advance = advance
                    .saturating_add(deltas[3].y.to_i32().saturating_sub(deltas[2].y.to_i32()));
            }
        }
        Some(advance)
    }

    pub(crate) fn top_side_bearing(
        &self,
        gid: impl Into<GlyphId>,
        coords: &[F2Dot14],
    ) -> Option<i32> {
        let gid = gid.into();
        let vmtx = self.vmtx.as_ref()?;
        let mut bearing = vmtx.side_bearing(gid).unwrap_or_default() as i32;
        if !coords.is_empty() {
            if let Some(vvar) = self.vvar.as_ref() {
                bearing = bearing
                    .saturating_add(vvar.tsb_delta(gid, coords).unwrap_or_default().to_i32());
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                bearing = bearing.saturating_add(deltas[3].y.to_i32());
            }
        }
        Some(bearing)
    }

    pub(crate) fn v_origin(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let origin = if let Some(vorg) = self.vorg.as_ref() {
            let mut origin = vorg.vertical_origin_y(gid) as i32;
            if !coords.is_empty() {
                if let Some(vvar) = self.vvar.as_ref() {
                    origin = origin
                        .saturating_add(vvar.v_org_delta(gid, coords).unwrap_or_default().to_i32());
                }
            }
            origin
        } else if let Some(extents) = self.bounds(gid, coords) {
            let origin = if self.vmtx.is_some() {
                let mut origin = Some(extents.y_max);
                let tsb = self.top_side_bearing(gid, coords);
                if let Some(tsb) = tsb {
                    origin = Some(origin.unwrap().saturating_add(tsb));
                } else {
                    origin = None;
                }
                if origin.is_some() && !coords.is_empty() {
                    if let Some(vvar) = self.vvar.as_ref() {
                        origin = Some(origin.unwrap().saturating_add(
                            vvar.v_org_delta(gid, coords).unwrap_or_default().to_i32(),
                        ));
                    }
                }
                origin
            } else {
                None
            };

            if let Some(origin) = origin {
                origin
            } else {
                let mut advance = self.ascent as i32 - self.descent as i32;
                if let Some(mvar) = self.mvar.as_ref() {
                    advance = advance
                        .saturating_add(
                            mvar.metric_delta(Tag::new(b"hasc"), coords)
                                .unwrap_or_default()
                                .to_i32(),
                        )
                        .saturating_sub(
                            mvar.metric_delta(Tag::new(b"hdsc"), coords)
                                .unwrap_or_default()
                                .to_i32(),
                        );
                }
                let height = extents.y_max.saturating_sub(extents.y_min);
                let diff = advance.saturating_sub(height);
                extents.y_max.saturating_add(diff >> 1)
            }
        } else {
            let mut ascent = self.ascent as i32;
            if let Some(mvar) = self.mvar.as_ref() {
                ascent = ascent.saturating_add(
                    mvar.metric_delta(Tag::new(b"hasc"), coords)
                        .unwrap_or_default()
                        .to_i32(),
                );
            }
            ascent
        };
        Some(origin)
    }

    fn bounds(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<BoundingBox<i32>> {
        let gid = gid.into();
        let glyf = self.glyf.as_ref()?;
        let glyph = glyf.loca.get_glyf(gid, &glyf.glyf).ok()?;
        let Some(glyph) = glyph else {
            // Return empty extents for empty glyph
            return Some(BoundingBox::default());
        };
        if !coords.is_empty() {
            return None; // TODO https://github.com/harfbuzz/harfrust/pull/52#issuecomment-2878117808
        }
        Some(BoundingBox {
            x_min: glyph.x_min() as i32,
            y_min: glyph.y_min() as i32,
            x_max: glyph.x_max() as i32,
            y_max: glyph.y_max() as i32,
        })
    }

    pub(crate) fn extents(
        &self,
        gid: impl Into<GlyphId>,
        coords: &[F2Dot14],
    ) -> Option<GlyphExtents> {
        let gid = gid.into();
        let glyf = self.glyf.as_ref()?;
        let glyph = glyf.loca.get_glyf(gid, &glyf.glyf).ok()?;
        let Some(glyph) = glyph else {
            // Return empty extents for empty glyph
            return Some(GlyphExtents::default());
        };
        if !coords.is_empty() {
            return None; // TODO https://github.com/harfbuzz/harfrust/pull/52#issuecomment-2878117808
        }
        let (x_min, x_max) = (glyph.x_min() as i32, glyph.x_max() as i32);
        let (y_min, y_max) = (glyph.y_min() as i32, glyph.y_max() as i32);
        // Undocumented rasterizer behaviour, which HarfBuzz matches: the glyph
        // is shifted left by (lsb - xMin), so the left edge of the ink sits at
        // the left side bearing rather than at the box the glyph carries. A
        // face whose hmtx does not reach this glyph keeps the box's own edge.
        let x_bearing = self
            .hmtx
            .as_ref()
            .and_then(|hmtx| hmtx.side_bearing(gid))
            .map_or_else(|| x_min.min(x_max), |lsb| lsb as i32);
        // The box is not assumed to be the right way round: a face may store
        // it either way, and the extents are the same either way.
        Some(GlyphExtents {
            x_bearing,
            y_bearing: y_min.max(y_max),
            width: x_min.max(x_max) - x_min.min(x_max),
            height: y_min.min(y_max) - y_min.max(y_max),
        })
    }

    fn phantom_deltas(&self, gid: GlyphId, coords: &[F2Dot14]) -> Option<[Point<Fixed>; 4]> {
        let glyf = self.glyf.as_ref()?;
        let gvar = glyf.gvar.as_ref()?;
        gvar.phantom_point_deltas(&glyf.glyf, &glyf.loca, coords, gid)
            .ok()?
    }
}

/// The names a face gives its glyphs.
#[derive(Clone)]
pub enum GlyphNames<'a> {
    /// The face names no glyphs.
    None,
    /// Names from the CFF charset.
    Cff(Cff<'a>, Charset<'a>),
    /// Names from the `post` table.
    Post(Post<'a>),
}

impl<'a> GlyphNames<'a> {
    /// The names in a font, from whichever table carries them.
    pub fn new(font: &FontKind<'a>) -> Self {
        match font {
            FontKind::FontRef(font) => Self::from_tables(&font.font),
            FontKind::FontInstance(instance, _) => Self::from_tables(&instance.tables()),
        }
    }

    pub(crate) fn from_tables(font: &impl TableProvider<'a>) -> Self {
        if let Some((cff, charset)) = font
            .cff()
            .ok()
            .and_then(|cff| Some((cff.clone(), cff.charset(0).ok()??)))
        {
            Self::Cff(cff, charset)
        } else if let Ok(post) = font.post() {
            Self::Post(post)
        } else {
            Self::None
        }
    }

    /// The name of one glyph, or `None` when the face does not name it.
    ///
    /// The name borrows from the face rather than from this lookup, so it
    /// can be held after the lookup is done with.
    pub fn get(&self, glyph_id: u32) -> Option<&'a str> {
        let name = match self {
            Self::Cff(cff, charset) => {
                let sid = charset.string_id(glyph_id.into()).ok()?;
                core::str::from_utf8(cff.string(sid)?).ok()
            }
            Self::Post(post) => {
                let gid: u16 = glyph_id.try_into().ok()?;
                post.glyph_name(gid.into())
            }
            Self::None => None,
        }?;
        (!name.is_empty()).then_some(name)
    }
}
