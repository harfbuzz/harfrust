/// An owned glyph name stored without allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphName {
    buf: [u8; 64],
    len: u8,
}

impl GlyphName {
    pub(crate) fn new(name: &str) -> Option<Self> {
        if name.is_empty() {
            return None;
        }
        let mut len = name.len().min(64);
        while !name.is_char_boundary(len) {
            len -= 1;
        }
        let mut buf = [0; 64];
        buf[..len].copy_from_slice(&name.as_bytes()[..len]);
        Some(Self {
            buf,
            len: len as u8,
        })
    }

    /// Returns the glyph name as a string.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or_default()
    }

    /// Returns the glyph name as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }
}

impl core::fmt::Display for GlyphName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

use read_fonts::ps::cff::charset::Charset;
use read_fonts::tables::{
    cff::Cff,
    cmap::{Cmap14, CmapSubtable, MapVariant},
    glyf::Glyf,
    gvar::Gvar,
    hmtx::{Hmtx, LongMetric},
    hvar::Hvar,
    loca::Loca,
    mvar::Mvar,
    os2::Os2,
    post::Post,
    vmtx::Vmtx,
    vorg::Vorg,
    vvar::Vvar,
};
use read_fonts::types::{BoundingBox, F2Dot14, Fixed, GlyphId, Point};
use read_fonts::TableProvider;

use crate::shaper_font::CharmapCache;
use crate::{GlyphExtents, GlyphInfo, GlyphPosition, Scale, Tag};

#[derive(Copy, Clone, Debug)]
pub(crate) struct BasicFontMetrics {
    pub units_per_em: u16,
    pub num_glyphs: u32,
    pub ascent: i16,
    pub descent: i16,
}

impl BasicFontMetrics {
    pub(crate) fn new<'a>(tables: &impl TableProvider<'a>) -> Self {
        let units_per_em = tables
            .head()
            .map(|head| head.units_per_em())
            .unwrap_or(1000);
        let num_glyphs = tables
            .maxp()
            .map(|maxp| maxp.num_glyphs() as u32)
            .unwrap_or_default();
        let os2 = tables.os2().ok();
        let hhea = tables.hhea().ok();
        let (ascent, descent) = horizontal_metrics(os2.as_ref(), hhea.as_ref(), units_per_em);
        Self {
            units_per_em,
            num_glyphs,
            ascent,
            descent,
        }
    }
}

#[derive(Copy, Clone)]
pub(crate) struct SelectedCmapSubtable {
    pub is_mac_roman: bool,
    pub is_symbol: bool,
    pub symbol_font_page: u16,
}

pub(crate) fn legacy_symbol_font_page(os2: Option<&Os2<'_>>) -> u16 {
    let Some(os2) = os2.filter(|os2| os2.version() == 0) else {
        return 0;
    };
    os2.offset_data()
        .read_at::<u16>(os2.fs_selection_byte_range().start)
        .unwrap_or_default()
        & 0xFF00
}

/// Selects horizontal ascender and descender as HarfBuzz does for legacy metrics.
fn horizontal_metrics(
    os2: Option<&Os2>,
    hhea: Option<&read_fonts::tables::hhea::Hhea>,
    upem: u16,
) -> (i16, i16) {
    let typo = os2.filter(|os2| {
        os2.fs_selection()
            .contains(read_fonts::tables::os2::SelectionFlags::USE_TYPO_METRICS)
    });
    let (ascent, descent) = match (typo, hhea) {
        (Some(os2), _) => (os2.s_typo_ascender(), os2.s_typo_descender()),
        (None, Some(hhea)) => (hhea.ascender().to_i16(), hhea.descender().to_i16()),
        (None, None) => {
            let ascent = (i32::from(upem) * 4 / 5) as i16;
            (ascent, ascent.saturating_sub(upem as i16))
        }
    };
    (ascent.saturating_abs(), -descent.saturating_abs())
}

#[cfg(feature = "std")]
type Lazy<T> = std::sync::OnceLock<T>;
#[cfg(not(feature = "std"))]
type Lazy<T> = core::cell::OnceCell<T>;

#[derive(Clone)]
struct InstanceFont<'a> {
    font: &'a crate::font::FontInstance,
    basic_metrics: BasicFontMetrics,
    glyph_metrics: Lazy<GlyphMetrics<'a>>,
    charmap: Lazy<Charmap<'a>>,
}

/// Default implementations backed by font tables.
#[derive(Clone)]
pub(crate) struct BuiltinFontFuncs<'a> {
    source: InstanceFont<'a>,
    coords: &'a [F2Dot14],
    units_per_em: u16,
    cmap_cache: Option<&'a CharmapCache>,
}

impl<'a> BuiltinFontFuncs<'a> {
    pub(crate) fn from_font(
        font: &'a crate::font::FontInstance,
        basic_metrics: BasicFontMetrics,
        coords: &'a [F2Dot14],
        units_per_em: u16,
        cmap_cache: Option<&'a CharmapCache>,
    ) -> Self {
        Self {
            source: InstanceFont {
                font,
                basic_metrics,
                glyph_metrics: Lazy::new(),
                charmap: Lazy::new(),
            },
            coords,
            units_per_em,
            cmap_cache,
        }
    }

    pub(crate) fn coords(&self) -> &[F2Dot14] {
        self.coords
    }

    fn charmap(&self) -> &Charmap<'a> {
        self.source
            .charmap
            .get_or_init(|| Charmap::from_tables(&self.source.font.tables()))
    }

    pub(crate) fn glyph_metrics(&self) -> &GlyphMetrics<'a> {
        self.source.glyph_metrics.get_or_init(|| {
            GlyphMetrics::from_tables(&self.source.font.tables(), &self.source.basic_metrics)
        })
    }

    pub(crate) fn cmap_cache(&self) -> Option<&CharmapCache> {
        self.cmap_cache
    }

    pub(crate) fn preload(&self) {
        let _ = self.charmap();
        let _ = self.glyph_metrics();
    }

    pub(crate) fn glyph_names(&self) -> GlyphNames<'a> {
        GlyphNames::from_tables(&self.source.font.tables())
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
}

#[derive(Clone)]
pub(crate) struct Charmap<'a> {
    subtable: Option<(SelectedCmapSubtable, CmapSubtable<'a>)>,
    vs_subtable: Option<Cmap14<'a>>,
}

impl<'a> Charmap<'a> {
    pub(crate) fn from_tables(font: &impl TableProvider<'a>) -> Self {
        if let Ok(cmap) = font.cmap() {
            let subtable = if let Some((_, record, subtable)) = cmap.best_subtable() {
                Some((
                    SelectedCmapSubtable {
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

    pub(crate) fn map(&self, mut c: u32) -> Option<GlyphId> {
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

    pub(crate) fn map_variant(&self, c: u32, vs: u32) -> Option<GlyphId> {
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
    use read_fonts::FontRef;

    #[test]
    fn maps_legacy_arabic_symbol_fonts() {
        let simplified =
            FontRef::new(include_bytes!("../tests/fonts/in-house/SimpArabicTest.ttf")).unwrap();
        assert_eq!(
            Charmap::from_tables(&simplified).map(0x0627),
            Some(GlyphId::new(45))
        );

        let traditional =
            FontRef::new(include_bytes!("../tests/fonts/in-house/TradArabicTest.ttf")).unwrap();
        assert_eq!(
            Charmap::from_tables(&traditional).map(0x0627),
            Some(GlyphId::new(65))
        );
    }
}

#[derive(Clone, Default)]
pub(crate) struct GlyphMetrics<'a> {
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
pub(crate) enum GlyphNames<'a> {
    /// The face names no glyphs.
    None,
    /// Names from the CFF charset.
    Cff(Cff<'a>, Charset<'a>),
    /// Names from the `post` table.
    Post(Post<'a>),
}

impl<'a> GlyphNames<'a> {
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
    pub(crate) fn get(&self, glyph_id: u32) -> Option<&'a str> {
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
