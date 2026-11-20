use crate::{Feature, ShapePlan};

/// A shaping engine that [`shape`](crate::shape) can use.
///
/// The OpenType shaper is the default. The WebAssembly shaper runs a font's
/// own module from its `Wasm` table, as HarfBuzz does, and needs the `wasm`
/// feature.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum Shaper {
    /// The OpenType shaper.
    Ot,
    /// The font's embedded WebAssembly module.
    ///
    /// Only fonts with a readable `Wasm` table can be shaped this way. A font
    /// without one is passed over when the shaper list is consulted.
    #[cfg(feature = "wasm")]
    Wasm,
}

const DEFAULT_SHAPERS: &[Shaper] = &[Shaper::Ot];

/// Optional settings for a shaping operation.
///
/// By default, shaping creates a plan, applies the font's default features,
/// has no point size for the `trak` table, and shapes with the OpenType
/// shaper.
pub struct ShapeOptions<'a> {
    pub(crate) plan: Option<&'a ShapePlan>,
    pub(crate) point_size: Option<f32>,
    pub(crate) features: &'a [Feature],
    pub(crate) shapers: &'a [Shaper],
    pub(crate) max_glyphs: Option<usize>,
    pub(crate) max_operations: Option<u32>,
}

impl Default for ShapeOptions<'_> {
    fn default() -> Self {
        Self {
            plan: None,
            point_size: None,
            features: &[],
            shapers: DEFAULT_SHAPERS,
            max_glyphs: None,
            max_operations: None,
        }
    }
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
    ///
    /// A plan records the shaper it was built for, and that shaper is used
    /// when the plan is supplied; [`shapers`](Self::shapers) is not consulted.
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

    /// Sets the shapers to try, in the order given.
    ///
    /// The first shaper that can shape the font is used. A shaper that cannot
    /// shape it is passed over, so `&[Shaper::Wasm, Shaper::Ot]` runs a font's
    /// module when it has one and leaves the rest to OpenType.
    ///
    /// Defaults to `&[Shaper::Ot]`. Ignored when a [`plan`](Self::plan) is
    /// supplied.
    pub fn shapers(mut self, shapers: &'a [Shaper]) -> Self {
        self.shapers = shapers;
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
