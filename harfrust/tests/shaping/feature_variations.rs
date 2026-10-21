use harfrust::{font::Font, Direction, ShapePlan, ShapePlanKey};

const FONT: &str = "tests/fonts/text-rendering-tests/AdobeVFPrototype-Subset.otf";

#[test]
fn default_location_selects_feature_variations() {
    for options in ["", "--variations=wght=1000", "--variations=wght=999"] {
        assert_eq!(
            crate::shape(FONT, "$", &format!("{options} --no-glyph-names")),
            "[2=0+560]",
            "{options}"
        );
    }
    assert_eq!(
        crate::shape(FONT, "$", "--variations=wght=500 --no-glyph-names"),
        "[1=0+510]"
    );
}

#[test]
fn default_feature_variations_match_the_shape_plan_key() {
    let font = Font::new(
        include_bytes!("../fonts/text-rendering-tests/AdobeVFPrototype-Subset.otf").to_vec(),
        0,
    )
    .unwrap();
    let plan = ShapePlan::new(&font, Direction::LeftToRight, None, None, &[]);
    assert!(ShapePlanKey::new(&font, None, Direction::LeftToRight).matches(&plan));

    let default = font
        .instance_builder()
        .variations([("wght", 1000.0)])
        .build();
    assert!(ShapePlanKey::new(&default, None, Direction::LeftToRight).matches(&plan));

    let nearby = font
        .instance_builder()
        .variations([("wght", 999.0)])
        .build();
    assert!(ShapePlanKey::new(&nearby, None, Direction::LeftToRight).matches(&plan));

    let outside = font
        .instance_builder()
        .variations([("wght", 500.0)])
        .build();
    assert!(!ShapePlanKey::new(&outside, None, Direction::LeftToRight).matches(&plan));
}

#[test]
fn default_location_selects_gpos_feature_variations() {
    let font = "tests/fonts/default-gpos-variations.ttf";
    for options in ["", "--variations=TEST=0", "--variations=TEST=0.125"] {
        assert_eq!(
            crate::shape(font, "A", &format!("{options} --no-glyph-names")),
            "[1=0+600]",
            "{options}"
        );
    }
    assert_eq!(
        crate::shape(font, "A", "--variations=TEST=0.5 --no-glyph-names"),
        "[1=0+510]"
    );
}
