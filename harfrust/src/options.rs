use crate::{Feature, ShapePlan};

/// Options which can be used to configure shaping.
#[derive(Default)]
pub struct ShapeOptions<'a> {
    pub(crate) plan: Option<&'a ShapePlan>,
    pub(crate) point_size: Option<f32>,
    pub(crate) features: &'a [Feature],
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
}
