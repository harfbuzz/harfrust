use crate::{Feature, ShapePlan};

/// Optional settings for a shaping operation.
///
/// By default, shaping creates a plan, applies the font's default features,
/// and has no point size for the `trak` table.
#[derive(Default)]
pub struct ShapeOptions<'a> {
    pub(crate) plan: Option<&'a ShapePlan>,
    pub(crate) point_size: Option<f32>,
    pub(crate) features: &'a [Feature],
    pub(crate) max_glyphs: Option<usize>,
    pub(crate) max_operations: Option<u32>,
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

    /// Sets the point size used by the `trak` tracking table.
    pub fn point_size(mut self, point_size: Option<f32>) -> Self {
        self.point_size = point_size;
        self
    }

    /// Sets user features to apply during shaping.
    pub fn features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// Limits the number of input, intermediate, and output glyph slots used
    /// by this call. A limit can only lower the shaper's built-in allowance.
    ///
    /// With either limit set, an incomplete result is reported as
    /// [`ShapeError::Incomplete`](crate::ShapeError::Incomplete).
    pub fn max_glyphs(mut self, limit: Option<usize>) -> Self {
        self.max_glyphs = limit;
        self
    }

    /// Limits the shaper's existing work counter for this call. Its units
    /// depend on the shaping implementation. Plan construction and elapsed
    /// time are outside this counter.
    ///
    /// With either limit set, an incomplete result is reported as
    /// [`ShapeError::Incomplete`](crate::ShapeError::Incomplete).
    pub fn max_operations(mut self, limit: Option<u32>) -> Self {
        self.max_operations = limit;
        self
    }
}
