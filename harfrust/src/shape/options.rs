use crate::font_funcs::FontFuncs;
use crate::{Feature, ShapePlan};

/// Options which can be used to configure shaping.
#[derive(Default)]
pub struct ShapeOptions<'a> {
    pub(crate) plan: Option<&'a ShapePlan>,
    pub(crate) scale: Option<(i32, i32)>,
    pub(crate) point_size: Option<f32>,
    pub(crate) features: &'a [Feature],
    pub(crate) font_funcs: Option<&'a (dyn FontFuncs + 'a)>,
}

impl<'a> ShapeOptions<'a> {
    /// Creates a default set of shape options ready for configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the plan to use for shaping.
    ///
    /// The shape plan must be compatible with the properties of the buffer
    /// passed to shaping.
    pub fn plan(mut self, plan: Option<&'a ShapePlan>) -> Self {
        self.plan = plan;
        self
    }

    /// Sets the scale factor to use during shaping.
    ///
    /// The font scale is a number related to, but not the same as, font size.
    /// Typically the client establishes a scale factor to be used between the
    /// two. For example, 64, or 256, which would be the fractional-precision
    /// part of the font scale. This is necessary because position and metric
    /// values are integer types and you need to leave room for fractional
    /// values in there.
    ///
    /// For example, to set the font size to 20, with 64 levels of fractional
    /// precision you would call provide a scale of `20 * 64`.
    ///
    /// In the example above, even what font size 20 means is up to you. It
    /// might be 20 pixels, or 20 points, or 20 millimeters. HarfRust does
    /// not care about that.
    ///
    /// The choice of scale is yours but needs to be consistent between what
    /// you set here, and what you expect as output as well as the values
    /// returned by [font functions](FontFuncs).
    ///
    /// This defaults to `None` which means that no scale is applied-- positions
    /// and metrics will be returned in font units.
    pub fn scale(mut self, scale: Option<i32>) -> Self {
        self.scale = scale.map(|s| (s, s));
        self
    }

    /// Sets separate x- and y-scale factors to use during shaping.
    ///
    /// Each axis uses the same semantics as [`scale`](Self::scale).
    pub fn scale_separate(mut self, scale: Option<(i32, i32)>) -> Self {
        self.scale = scale;
        self
    }

    /// Sets the size used for application of the tracking table.
    pub fn point_size(mut self, point_size: Option<f32>) -> Self {
        self.point_size = point_size;
        self
    }

    /// Sets the features to apply during shaping.
    pub fn features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// Sets optional font functions used for shaping.
    pub fn font_funcs(mut self, funcs: Option<&'a (dyn FontFuncs + 'a)>) -> Self {
        self.font_funcs = funcs;
        self
    }
}
