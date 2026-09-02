use alloc::boxed::Box;
use core::any::Any;
use smallvec::SmallVec;

use crate::hb::common::HB_FEATURE_GLOBAL_END;
use crate::hb::common::HB_FEATURE_GLOBAL_START;
use crate::ShaperInstance;

use super::aat::map::*;
use super::ot_map::*;
use super::ot_shape::*;
use super::ot_shaper::*;
use super::{hb_font_t, hb_mask_t, Direction, Feature, Language, Script};

/// A reusable plan for shaping a text buffer.
pub struct hb_ot_shape_plan_t {
    pub(crate) direction: Direction,
    pub(crate) script: Option<Script>,
    pub(crate) language: Option<Language>,
    pub(crate) shaper: &'static hb_ot_shaper_t,
    pub(crate) ot_map: hb_ot_map_t,
    pub(crate) aat_map: AatMap,
    pub(crate) data: Option<Box<dyn Any + Send + Sync>>,

    pub(crate) frac_mask: hb_mask_t,
    pub(crate) numr_mask: hb_mask_t,
    pub(crate) dnom_mask: hb_mask_t,
    pub(crate) rtlm_mask: hb_mask_t,
    pub(crate) kern_mask: hb_mask_t,

    pub(crate) requested_kerning: bool,
    pub(crate) has_frac: bool,
    pub(crate) has_vert: bool,
    pub(crate) has_gpos_mark: bool,
    pub(crate) zero_marks: bool,
    pub(crate) fallback_glyph_classes: bool,
    pub(crate) fallback_mark_positioning: bool,
    pub(crate) adjust_mark_positioning_when_zeroing: bool,

    pub(crate) apply_gpos: bool,
    pub(crate) apply_fallback_kern: bool,
    pub(crate) apply_kern: bool,
    pub(crate) apply_kerx: bool,
    pub(crate) apply_morx: bool,
    pub(crate) apply_trak: bool,

    pub(crate) user_features: SmallVec<[Feature; 4]>,
}

pub trait AnyFont {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&hb_font_t>) -> R;
}

impl AnyFont for hb_font_t<'_> {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&hb_font_t>) -> R,
    {
        f(Some(self))
    }
}

impl AnyFont for crate::font::FontInstance {
    fn with_font<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&hb_font_t>) -> R,
    {
        let hb_font = hb_font_t::from_font(self);
        f(hb_font.as_ref())
    }
}

impl hb_ot_shape_plan_t {
    /// Returns a plan that can be used for shaping any buffer with the
    /// provided properties.
    pub fn new(
        font: &impl AnyFont,
        direction: Direction,
        script: Option<Script>,
        language: Option<&Language>,
        user_features: &[Feature],
    ) -> Self {
        assert_ne!(
            direction,
            Direction::Invalid,
            "Direction must not be Invalid"
        );
        font.with_font(|font| {
            let font = font.expect("font should be available for shaping");
            let mut planner = hb_ot_shape_planner_t::new(font, direction, script, language);
            planner.collect_features(user_features);
            planner.compile(user_features)
        })
    }

    pub(crate) fn data<T: 'static>(&self) -> &T {
        self.data.as_ref().unwrap().downcast_ref().unwrap()
    }

    /// The direction of the text.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// The script of the text.
    pub fn script(&self) -> Option<Script> {
        self.script
    }

    /// The language of the text.
    pub fn language(&self) -> Option<&Language> {
        self.language.as_ref()
    }
}

/// A key used for selecting a shape plan.
pub struct ShapePlanKey<'a> {
    script: Option<Script>,
    direction: Direction,
    language: Option<&'a Language>,
    feature_variations: [Option<u32>; 2],
    features: &'a [Feature],
}

impl<'a> ShapePlanKey<'a> {
    /// Creates a new shape plan key with the given script and direction.
    pub fn new(script: Option<Script>, direction: Direction) -> Self {
        Self {
            script,
            direction,
            language: None,
            feature_variations: [None; 2],
            features: &[],
        }
    }

    /// Sets the language to use for this shape plan key.
    pub fn language(mut self, language: Option<&'a Language>) -> Self {
        self.language = language;
        self
    }

    /// Sets the instance to use for this shape plan key.
    pub fn instance(mut self, instance: Option<&ShaperInstance>) -> Self {
        self.feature_variations = instance
            .map(|instance| instance.feature_variations)
            .unwrap_or_default();
        self
    }

    /// Sets the features to use for this shape plan key.
    pub fn features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// Returns true if this key is a match for the given shape plan.
    pub fn matches(&self, plan: &hb_ot_shape_plan_t) -> bool {
        self.script == plan.script
            && self.direction == plan.direction
            && self.language == plan.language.as_ref()
            && self.feature_variations == *plan.ot_map.feature_variations()
            && features_equivalent(self.features, &plan.user_features)
    }
}

fn features_equivalent(features_a: &[Feature], features_b: &[Feature]) -> bool {
    if features_a.len() != features_b.len() {
        return false;
    }
    for (a, b) in features_a.iter().zip(features_b) {
        if a.tag != b.tag
            || a.value != b.value
            || (a.start == HB_FEATURE_GLOBAL_START && a.end == HB_FEATURE_GLOBAL_END)
                != (b.start == HB_FEATURE_GLOBAL_START && b.end == HB_FEATURE_GLOBAL_END)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::hb_ot_shape_plan_t;

    #[test]
    fn test_shape_plan_is_send_and_sync() {
        fn ensure_send_and_sync<T: Send + Sync>() {}
        ensure_send_and_sync::<hb_ot_shape_plan_t>();
    }

    /// Gated on `std` because `#[hegel::test]`'s generated code uses the std
    /// prelude.
    #[cfg(all(test, feature = "std"))]
    mod properties {
        use crate::hb::common::{TagExt, HB_FEATURE_GLOBAL_END, HB_FEATURE_GLOBAL_START};
        use crate::{
            script, Direction, Feature, FontRef, Language, Script, ShapePlan, ShapePlanKey,
            ShaperData, Tag,
        };
        use hegel::generators::{self, Generator};

        const DIRECTIONS: &[Direction] = &[
            Direction::LeftToRight,
            Direction::RightToLeft,
            Direction::TopToBottom,
            Direction::BottomToTop,
        ];

        const SCRIPTS: &[Script] = &[
            script::LATIN,
            script::ARABIC,
            script::DEVANAGARI,
            script::HAN,
            script::UNKNOWN,
        ];

        const LANGUAGES: &[&str] = &["en", "ar", "hi", "zh-cn"];

        /// The properties a plan is built for and a key is matched against.
        #[derive(Debug, Clone)]
        struct PlanProperties {
            direction: Direction,
            script: Option<Script>,
            language: Option<Language>,
            features: Vec<Feature>,
        }

        fn draw_properties(tc: &hegel::TestCase) -> PlanProperties {
            let properties = PlanProperties {
                // `ShapePlan::new` asserts on `Direction::Invalid`.
                direction: tc.draw_silent(generators::sampled_from(DIRECTIONS.to_vec())),
                script: tc.draw_silent(generators::optional(generators::sampled_from(
                    SCRIPTS.to_vec(),
                ))),
                language: tc
                    .draw_silent(generators::optional(generators::sampled_from(
                        LANGUAGES.to_vec(),
                    )))
                    .and_then(Language::new),
                features: draw_features(tc),
            };
            tc.note(&format!("{properties:?}"));
            properties
        }

        fn draw_features(tc: &hegel::TestCase) -> Vec<Feature> {
            let n = tc.draw_silent(generators::integers::<usize>().max_value(3));
            (0..n)
                .map(|_| {
                    let bytes: [u8; 4] = tc.draw_silent(generators::arrays(
                        generators::integers::<u8>().min_value(b'a').max_value(b'z'),
                    ));
                    let tag = Tag::from_bytes_lossy(&bytes);
                    let value = tc.draw_silent(generators::integers::<u32>().max_value(3));
                    let (start, end) = draw_range(tc);
                    Feature {
                        tag,
                        value,
                        start,
                        end,
                    }
                })
                .collect()
        }

        fn draw_range(tc: &hegel::TestCase) -> (u32, u32) {
            if tc.draw_silent(generators::booleans()) {
                (HB_FEATURE_GLOBAL_START, HB_FEATURE_GLOBAL_END)
            } else {
                let start = tc.draw_silent(generators::integers::<u32>().max_value(8));
                (
                    start,
                    start + tc.draw_silent(generators::integers::<u32>().max_value(8)),
                )
            }
        }

        fn font() -> FontRef<'static> {
            FontRef::new(include_bytes!("../../benches/fonts/Roboto-Regular.ttf")).unwrap()
        }

        fn key(properties: &PlanProperties) -> ShapePlanKey<'_> {
            ShapePlanKey::new(properties.script, properties.direction)
                .language(properties.language.as_ref())
                .features(&properties.features)
        }

        fn plan(font: &FontRef, data: &ShaperData, properties: &PlanProperties) -> ShapePlan {
            let shaper = data.shaper(font).build();
            ShapePlan::new(
                &shaper,
                properties.direction,
                properties.script,
                properties.language.as_ref(),
                &properties.features,
            )
        }

        /// Property: a key matches the plan built from the same properties.
        ///
        /// That is what a plan cache is for: `ShapePlanKey::matches` decides
        /// whether a cached plan can be reused, so it has to accept the plan
        /// its own properties produced.
        #[hegel::test]
        fn a_key_matches_the_plan_built_from_the_same_properties(tc: hegel::TestCase) {
            let font = font();
            let data = ShaperData::new(&font);
            let properties = draw_properties(&tc);
            assert!(key(&properties).matches(&plan(&font, &data, &properties)));
        }

        /// Property: a key with a different direction, script or language does
        /// not match.
        ///
        /// Reusing a plan across those would shape with the wrong lookups;
        /// `Shaper::shape_buffer` refuses a plan whose direction or script
        /// disagrees with the buffer for the same reason.
        #[hegel::test]
        fn a_key_with_different_segment_properties_does_not_match(tc: hegel::TestCase) {
            let font = font();
            let data = ShaperData::new(&font);
            let properties = draw_properties(&tc);
            let mut other = properties.clone();
            match tc.draw(generators::integers::<u8>().max_value(2)) {
                0 => {
                    other.direction = tc.draw(
                        generators::sampled_from(
                            DIRECTIONS
                                .iter()
                                .copied()
                                .filter(|d| *d != properties.direction)
                                .collect::<Vec<_>>(),
                        )
                        .print_as_debug(),
                    );
                }
                1 => {
                    other.script = tc.draw(
                        generators::sampled_from(
                            SCRIPTS
                                .iter()
                                .copied()
                                .map(Some)
                                .chain([None])
                                .filter(|s| *s != properties.script)
                                .collect::<Vec<_>>(),
                        )
                        .print_as_debug(),
                    );
                }
                _ => {
                    other.language = tc.draw(
                        generators::sampled_from(
                            LANGUAGES
                                .iter()
                                .filter_map(Language::new)
                                .map(Some)
                                .chain([None])
                                .filter(|l| *l != properties.language)
                                .collect::<Vec<_>>(),
                        )
                        .print_as_debug(),
                    );
                }
            }
            assert!(!key(&other).matches(&plan(&font, &data, &properties)));
        }

        /// Property: where a non-global feature applies does not affect
        /// whether a key matches.
        ///
        /// `features_equivalent` compares tags and values but only asks of the
        /// range whether it is global, matching HarfBuzz's
        /// `hb_shape_plan_key_t::user_features_match`. Non-global features are
        /// applied per cluster at shaping time rather than compiled into the
        /// plan, so two runs differing only in where they apply can share one.
        #[hegel::test]
        fn a_non_global_features_range_does_not_affect_matching(tc: hegel::TestCase) {
            let font = font();
            let data = ShaperData::new(&font);
            let properties = draw_properties(&tc);
            let mut other = properties.clone();
            for feature in &mut other.features {
                if feature.start != HB_FEATURE_GLOBAL_START || feature.end != HB_FEATURE_GLOBAL_END
                {
                    (feature.start, feature.end) = draw_range(&tc);
                    // Keep it non-global, or the key stops being equivalent.
                    if feature.start == HB_FEATURE_GLOBAL_START
                        && feature.end == HB_FEATURE_GLOBAL_END
                    {
                        feature.end = HB_FEATURE_GLOBAL_END - 1;
                    }
                }
            }
            assert!(key(&other).matches(&plan(&font, &data, &properties)));
        }
    }
}
