use crate::Tag;
use read_fonts::{
    tables::{
        glyf::Glyf, gvar::Gvar, hmtx::Hmtx, hvar::Hvar, loca::Loca, mvar::Mvar, vmtx::Vmtx,
        vorg::Vorg, vvar::Vvar,
    },
    types::{BoundingBox, F2Dot14, Fixed, GlyphId, Point},
    FontRef, TableProvider,
};

#[derive(Clone)]
pub(crate) struct GlyphMetrics<'a> {
    hmtx: Option<Hmtx<'a>>,
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
    pub fn new(font: &FontRef<'a>) -> Self {
        let num_glyphs = font
            .maxp()
            .map(|maxp| maxp.num_glyphs() as u32)
            .unwrap_or(0);
        let upem = font.head().map(|head| head.units_per_em()).unwrap_or(1024);
        let hmtx = font.hmtx().ok();
        let hvar = font.hvar().ok();
        let vmtx = font.vmtx().ok();
        let vvar = font.vvar().ok();
        let vorg = font.vorg().ok();
        let glyf = if let (Ok(glyf), Ok(loca)) = (font.glyf(), font.loca(None)) {
            Some(GlyfTables {
                glyf,
                loca,
                gvar: font.gvar().ok(),
            })
        } else {
            None
        };
        let mvar = font.mvar().ok();
        let (ascent, descent) = if let Ok(os2) = font.os2() {
            (os2.s_typo_ascender(), os2.s_typo_descender())
        } else if let Ok(hhea) = font.hhea() {
            (hhea.ascender().to_i16(), hhea.descender().to_i16())
        } else {
            (0, 0) // TODO
        };
        Self {
            hmtx,
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

    pub fn advance_width(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let Some(mut advance) = self
            .hmtx
            .as_ref()
            .and_then(|hmtx| hmtx.advance(gid))
            .map(|advance| advance as i32)
        else {
            return (gid.to_u32() < self.num_glyphs).then_some(self.upem as i32 / 2);
        };
        if !coords.is_empty() {
            if let Some(hvar) = self.hvar.as_ref() {
                advance += hvar
                    .advance_width_delta(gid, coords)
                    .unwrap_or_default()
                    .to_i32();
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                advance += deltas[1].x.to_i32() - deltas[0].x.to_i32();
            }
        }
        Some(advance)
    }

    pub fn _left_side_bearing(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let mut bearing = if let Some(hmtx) = self.hmtx.as_ref() {
            hmtx.side_bearing(gid).unwrap_or_default() as i32
        } else if let Some(extents) = self.extents(gid, coords) {
            return Some(extents.x_min);
        } else {
            return None;
        };
        if !coords.is_empty() {
            if let Some(hvar) = self.hvar.as_ref() {
                bearing += hvar.lsb_delta(gid, coords).unwrap_or_default().to_i32();
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                bearing += deltas[0].x.to_i32();
            }
        }
        Some(bearing)
    }

    pub fn advance_height(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
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
                advance += vvar
                    .advance_height_delta(gid, coords)
                    .unwrap_or_default()
                    .to_i32();
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                advance += deltas[3].y.to_i32() - deltas[2].y.to_i32();
            }
        }
        Some(advance)
    }

    pub fn top_side_bearing(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let mut bearing = if let Some(vmtx) = self.vmtx.as_ref() {
            vmtx.side_bearing(gid).unwrap_or_default() as i32
        } else {
            return None;
        };
        if !coords.is_empty() {
            if let Some(vvar) = self.vvar.as_ref() {
                bearing += vvar.tsb_delta(gid, coords).unwrap_or_default().to_i32();
            } else if let Some(deltas) = self.phantom_deltas(gid, coords) {
                bearing += deltas[3].y.to_i32();
            }
        }
        Some(bearing)
    }

    pub fn v_origin(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<i32> {
        let gid = gid.into();
        let origin = if let Some(vorg) = self.vorg.as_ref() {
            let mut origin = vorg.vertical_origin_y(gid) as i32;
            if !coords.is_empty() {
                if let Some(vvar) = self.vvar.as_ref() {
                    origin += vvar.v_org_delta(gid, coords).unwrap_or_default().to_i32();
                }
            }
            origin
        } else if let Some(extents) = self.extents(gid, coords) {
            let origin = if self.vmtx.is_some() {
                let mut origin = Some(extents.y_max);
                let tsb = self.top_side_bearing(gid, coords);
                if let Some(tsb) = tsb {
                    origin = Some(origin.unwrap() + tsb);
                } else {
                    origin = None;
                }
                if origin.is_some() && !coords.is_empty() {
                    if let Some(vvar) = self.vvar.as_ref() {
                        origin = Some(
                            origin.unwrap()
                                + vvar.v_org_delta(gid, coords).unwrap_or_default().to_i32(),
                        );
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
                    advance += mvar
                        .metric_delta(Tag::new(b"hasc"), coords)
                        .unwrap_or_default()
                        .to_i32()
                        - mvar
                            .metric_delta(Tag::new(b"hdsc"), coords)
                            .unwrap_or_default()
                            .to_i32();
                }
                let diff = advance - (extents.y_max - extents.y_min);
                extents.y_max + (diff >> 1)
            }
        } else {
            let mut ascent = self.ascent as i32;
            if let Some(mvar) = self.mvar.as_ref() {
                ascent += mvar
                    .metric_delta(Tag::new(b"hasc"), coords)
                    .unwrap_or_default()
                    .to_i32();
            }
            ascent
        };
        Some(origin)
    }

    pub fn extents(&self, gid: impl Into<GlyphId>, coords: &[F2Dot14]) -> Option<BoundingBox<i32>> {
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
            x_max: glyph.x_max() as i32,
            y_min: glyph.y_min() as i32,
            y_max: glyph.y_max() as i32,
        })
    }

    fn phantom_deltas(&self, gid: GlyphId, coords: &[F2Dot14]) -> Option<[Point<Fixed>; 4]> {
        let glyf = self.glyf.as_ref()?;
        let gvar = glyf.gvar.as_ref()?;
        gvar.phantom_point_deltas(&glyf.glyf, &glyf.loca, coords, gid)
            .ok()?
    }
}
