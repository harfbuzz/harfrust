use alloc::boxed::Box;
use core::any::Any;
use smallvec::SmallVec;

use crate::aat::map::AatMap;
use crate::once::Once;
use crate::ot::shaper::OtShaper;

use crate::ot::map::*;
use crate::LayoutData;
use crate::{
    font::Font, Direction, Feature, Language, Mask, Script, ShapePlanner, Shaper, ShaperFont,
};
use read_fonts::types::F2Dot14;

/// A reusable plan for shaping with one set of segment properties.
///
/// A plan records the shaper it was built for and the selected shaping
/// features and lookups. Reuse it with a compatible font, direction, script,
/// language, and feature set.
pub struct ShapePlan {
    pub(crate) direction: Direction,
    pub(crate) script: Option<Script>,
    pub(crate) language: Option<Language>,
    pub(crate) ot_shaper: &'static OtShaper,
    pub(crate) shaper: Shaper,
    pub(crate) ot_map: OtMap,
    pub(crate) aat_map: AatMap,
    pub(crate) data: Option<Box<dyn Any + Send + Sync>>,

    pub(crate) frac_mask: Mask,
    pub(crate) numr_mask: Mask,
    pub(crate) dnom_mask: Mask,
    pub(crate) rtlm_mask: Mask,
    pub(crate) kern_mask: Mask,

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

impl ShapePlan {
    /// Builds a plan for a font and segment properties.
    ///
    /// # Panics
    ///
    /// Panics if `direction` is [`Direction::Invalid`].
    pub fn new(
        font: &Font,
        direction: Direction,
        script: Option<Script>,
        language: Option<&Language>,
        user_features: &[Feature],
    ) -> Self {
        let shaping_font = ShaperFont::new(font);
        Self::from_layout(
            shaping_font.layout(),
            direction,
            script,
            language,
            user_features,
        )
    }

    pub(crate) fn from_layout(
        layout: LayoutData<'_>,
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
        let mut planner = ShapePlanner::new(layout, direction, script, language);
        planner.collect_features(user_features);
        planner.compile(user_features)
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

    /// The language supplied when this plan was built.
    pub fn language(&self) -> Option<&Language> {
        self.language.as_ref()
    }

    /// The shaper this plan was built for.
    pub fn shaper(&self) -> Shaper {
        self.shaper
    }
}

/// The properties used to match a reusable [`ShapePlan`].
pub struct ShapePlanKey<'a> {
    font: &'a Font,
    resolved: Once<ResolvedLanguages>,
    script: Option<Script>,
    direction: Direction,
    language: Option<&'a Language>,
    pub(crate) feature_variations: [Option<u32>; 2],
    /// The font's location, if lookup variations depend on it.
    lookup_coords: Option<&'a [F2Dot14]>,
    features: &'a [Feature],
}

impl<'a> ShapePlanKey<'a> {
    /// Creates a key for a font, script, and direction.
    pub fn new(font: &'a Font, script: Option<Script>, direction: Direction) -> Self {
        let variations = font.feature_variations();
        Self {
            font,
            resolved: Once::new(),
            script,
            direction,
            language: None,
            feature_variations: [variations.gsub, variations.gpos],
            lookup_coords: variations
                .has_lookup_variations
                .then(|| font.normalized_coords()),
            features: &[],
        }
    }

    /// Sets the language to use for this shape plan key.
    pub fn language(mut self, language: Option<&'a Language>) -> Self {
        self.language = language;
        self.resolved = Once::new();
        self
    }

    /// Sets the features to use for this shape plan key.
    pub fn features(mut self, features: &'a [Feature]) -> Self {
        self.features = features;
        self
    }

    /// Matches direction, script, and language against a plan for the same face.
    /// OpenType plans accept different language tags that select the same
    /// GSUB and GPOS language systems and AAT language selectors.
    pub fn matches_segment_properties(&self, plan: &ShapePlan) -> bool {
        if self.script != plan.script || self.direction != plan.direction {
            return false;
        }
        if self.language == plan.language() {
            return true;
        }
        if plan.shaper != Shaper::Ot {
            return false;
        }
        // A cache may compare this key to several plans. Resolve at most
        // once, and only when the language tags differ.
        let resolved = self.resolved.get_or_init(|| {
            let font = ShaperFont::new(self.font);
            ResolvedLanguages::new(font.layout(), self.direction, self.script, self.language)
        });
        *resolved == plan.ot_map.languages
    }

    /// Returns true if this key matches the given shape plan for the same face.
    pub fn matches(&self, plan: &ShapePlan) -> bool {
        self.matches_segment_properties(plan)
            && self.feature_variations == *plan.ot_map.feature_variations()
            && self.lookup_coords == plan.ot_map.lookup_coords()
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
            || (a.start == Feature::GLOBAL_START && a.end == Feature::GLOBAL_END)
                != (b.start == Feature::GLOBAL_START && b.end == Feature::GLOBAL_END)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{ShapePlan, ShapePlanKey};
    use crate::{Direction, Font, Language, Script};

    fn check_languages(data: &'static [u8], first: &str, alias: &str, other: &str) {
        let font = Font::new(data, 0).unwrap();
        let script = Some(Script::LATIN);
        let direction = Direction::LeftToRight;
        let first = Language::new(first);
        let alias = Language::new(alias);
        let other = Language::new(other);
        let plan = ShapePlan::new(&font, direction, script, first.as_ref(), &[]);
        let key = ShapePlanKey::new(&font, script, direction).language(alias.as_ref());
        assert!(key.matches(&plan));
        // Reusing the builder with another language must resolve it anew.
        assert!(!key.language(other.as_ref()).matches(&plan));
        assert_eq!(plan.language(), first.as_ref());
    }

    #[test]
    fn keys_match_resolved_scripts_and_languages() {
        let gsub =
            include_bytes!("../tests/fonts/in-house/bbc24004e776f348a0f72287d24b0124867ee750.ttf");
        check_languages(gsub, "en", "de", "tr");
        check_languages(gsub, "en", "en-GB", "en-x-hbsccyrl");
        // Explicitly finding DFLT differs from falling back to it.
        check_languages(gsub, "en-x-hbscdflt", "de-x-hbscdflt", "en-x-hbsczzzz");

        let gpos = include_bytes!("../tests/fonts/rb_custom/NotoSansSinhala.subset1.otf");
        check_languages(gpos, "en", "de", "ro");
    }

    #[test]
    fn keys_match_canonical_aat_language_prefixes() {
        let aat = include_bytes!("../tests/fonts/in-house/morx-language-canonicalization.ttf");
        check_languages(aat, "sr", "sr-Latn-RS", "zh-Hant");
        check_languages(aat, "zh-Hant", "zh-Hant-TW", "zh");
    }

    #[test]
    fn test_shape_plan_is_send_and_sync() {
        fn ensure_send_and_sync<T: Send + Sync>() {}
        ensure_send_and_sync::<ShapePlan>();
    }
}
