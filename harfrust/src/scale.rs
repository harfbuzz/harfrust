use crate::GlyphExtents;

// libm used for f32::floor() and f32::ceil()
#[cfg(not(feature = "std"))]
#[allow(unused_imports)]
use core_maths::CoreFloat as _;

#[derive(Copy, Clone)]
/// How font units become the units a caller asked for.
///
/// Shaping applies this to everything it reports, from
/// [`crate::ShaperFont::set_scale`].
/// A caller asking a font about one glyph rather
/// than about a run needs the same conversion, and needs it to be the same
/// one, so it is spelled once here -- down to the rounding, which follows
/// HarfBuzz's.
#[derive(Debug)]
pub struct Scale {
    x_mult: i64,
    y_mult: i64,
    x_multf: f32,
    y_multf: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Self {
            x_mult: 1 << 16,
            y_mult: 1 << 16,
            x_multf: 1.0,
            y_multf: 1.0,
        }
    }
}

// Various conversions between f32 and i32
#[allow(clippy::cast_precision_loss)]
impl Scale {
    /// The conversion from `upem` font units into `scale`, or the identity
    /// when there is no scale to apply or the face has no units to convert.
    pub fn new(scale: Option<(i32, i32)>, upem: i32) -> Self {
        let (Some((x_scale, y_scale)), true) = (scale, upem != 0) else {
            // When scale is not configured, or upem is zero, return results
            // in font units.
            return Self::default();
        };
        let [x_mult, y_mult] = [x_scale, y_scale].map(|s| Self::mult_from_scale(s, upem));
        let upem = upem as f32;
        Self {
            x_mult,
            y_mult,
            x_multf: x_scale as f32 / upem,
            y_multf: y_scale as f32 / upem,
        }
    }

    /// A horizontal distance in font units, in the units asked for.
    #[inline(always)]
    pub fn scale_x(&self, x: i32) -> i32 {
        Self::scale_by_mult(x, self.x_mult)
    }

    /// A vertical distance in font units, in the units asked for.
    #[inline(always)]
    pub fn scale_y(&self, y: i32) -> i32 {
        Self::scale_by_mult(y, self.y_mult)
    }

    /// Scales a fractional (font-unit) value, matching HarfBuzz's `em_scalef`
    /// (`roundf(v * scale / upem)`).
    #[inline(always)]
    pub(crate) fn scale_x_f(&self, x: f32) -> i32 {
        (x * self.x_multf).round() as i32
    }

    #[inline(always)]
    pub(crate) fn scale_y_f(&self, y: f32) -> i32 {
        (y * self.y_multf).round() as i32
    }

    /// Scales glyph extents using HarfBuzz's corner-based float arithmetic:
    /// floor the origin corners and ceil the far corners before deriving the
    /// final width/height.
    /// hb_font_t::scale_glyph_extents: <https://github.com/harfbuzz/harfbuzz/blob/88adc6437ef561486a5adf1822410297ef4a852b/src/hb-font.hh#L201>'
    pub fn scale_extents(&self, mut extents: GlyphExtents) -> GlyphExtents {
        let x1 = extents.x_bearing as f32 * self.x_multf;
        let y1 = extents.y_bearing as f32 * self.y_multf;
        let x2 = (i64::from(extents.x_bearing) + i64::from(extents.width)) as f32 * self.x_multf;
        let y2 = (i64::from(extents.y_bearing) + i64::from(extents.height)) as f32 * self.y_multf;
        let rx1 = x1.floor();
        let ry1 = y1.floor();
        let rx2 = x2.ceil();
        let ry2 = y2.ceil();
        extents.x_bearing = rx1 as i32;
        extents.y_bearing = ry1 as i32;
        extents.width = (f64::from(rx2) - f64::from(rx1)) as i32;
        extents.height = (f64::from(ry2) - f64::from(ry1)) as i32;
        extents
    }

    #[inline(always)]
    fn mult_from_scale(scale: i32, upem: i32) -> i64 {
        if scale < 0 {
            -((-(scale as i64)) << 16) / upem as i64
        } else {
            ((scale as i64) << 16) / upem as i64
        }
    }

    #[inline(always)]
    fn scale_by_mult(value: i32, mult: i64) -> i32 {
        ((i64::from(value) * mult + 32768) >> 16) as i32
    }
}
