use crate::{
    hb::{
        face::{BasicFontMetrics, Scale},
        tables::TableRanges,
    },
    GlyphExtents, GlyphInfo, GlyphPosition, Tag,
};
use read_fonts::{
    tables::{
        glyf::Glyf,
        gvar::Gvar,
        hmtx::{Hmtx, LongMetric},
        hvar::Hvar,
        loca::Loca,
        mvar::Mvar,
        vmtx::Vmtx,
        vorg::Vorg,
        vvar::Vvar,
    },
    types::{BoundingBox, F2Dot14, Fixed, GlyphId, Point},
    FontRef, TableProvider,
};

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

    pub(crate) fn advance_height(
        &self,
        gid: impl Into<GlyphId>,
        coords: &[F2Dot14],
    ) -> Option<i32> {
        let gid = gid.into();
        let Some(mut advance) = self
            .vmtx
            .as_ref()
            .and_then(|vmtx| vmtx.advance(gid))
            .map(|advance| advance as i32)
        else {
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
