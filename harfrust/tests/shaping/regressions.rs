use std::{fs, path::PathBuf};

use harfrust::{font::Font, shape, Buffer, ShapeOptions, ShaperFont};

fn font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("text-rendering-tests")
        .join(name)
}

#[test]
fn hangul_tone_reordering_preserves_monotone_character_clusters() {
    assert_eq!(
        crate::shape(
            "tests/fonts/text-rendering-tests/FDArrayTest257.otf",
            "\u{AC00}\u{302E}",
            "--cluster-level=1 --show-flags --no-glyph-names",
        ),
        "[47=0+1000|1=0+1000]"
    );
}

// This test uses `TestGPOSThree.ttf`, which has a GPOS table.
// It verifies that GPOS attachment offset propagation scales safely without integer overflow
// for extremely long grapheme clusters (as fixed in commit 6bccd74).
#[test]
fn issue_384_overly_long_grapheme_cluster_gpos_does_not_overflow() {
    let font_data = fs::read(font_path("TestGPOSThree.ttf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let shaping_font = shaper;

    let mut text = String::with_capacity(35_002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 35_000));
    text.push('X');

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

// Uses `Calculator-Regular.ttf` (no GPOS table) to verify fallback mark positioning.
// A large scale factor amplifies mark heights, triggering fallback integer overflows
// with a lower grapheme count of 5,000.
#[test]
fn issue_384_overly_long_grapheme_cluster_fallback_does_not_overflow() {
    let font_data =
        fs::read(font_path("Calculator-Regular.ttf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let mut shaping_font = shaper;

    let mut text = String::with_capacity(5002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 5000));
    text.push('X');

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shaping_font.set_scale(10_000_000);
    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

// The glyph is 2143 units tall in a 2048-unit line. Its synthesized origin
// halves -95 toward negative infinity, matching HarfBuzz.
#[test]
fn synthesized_vertical_origin_floors_negative_half_height() {
    assert_eq!(
        crate::shape(
            "tests/fonts/text-rendering-tests/Calculator-Regular.ttf",
            "\u{03AE}",
            "--direction=ttb --no-glyph-names",
        ),
        "[825=0@-628,-1603+0,-2048]"
    );
}

#[test]
fn shaping_long_line_kern_does_not_overflow_glyph_data() {
    let font_data = fs::read(font_path("TestKERNOne.otf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let shaping_font = shaper;

    let mut text = String::with_capacity(70_000);
    for _ in 0..35_000 {
        text.push('u');
        text.push('T');
    }

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

/// A mark more than `i16::MAX` glyphs after its base cannot store that
/// distance in its attachment chain. HarfBuzz leaves such a mark unattached
/// instead of storing a truncated chain.
#[test]
fn mark_too_far_from_its_base_is_left_unattached() {
    let mut text = String::from("x");
    text.extend(std::iter::repeat_n('\u{0301}', 32_769));
    let output = crate::shape(
        "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
        &text,
        "--features=-mkmk --no-glyph-names --no-clusters",
    );
    let glyphs: Vec<&str> = output.trim_matches(['[', ']']).split('|').collect();
    assert_eq!(glyphs.len(), 32_770);
    assert_eq!(glyphs[32_768], "975@-369,0+0");
    assert_eq!(glyphs[32_769], "975+0");
}

// Sorting these marks temporarily puts the minimum cluster inside the run.
// It must remain exempt from flags applied to the other clusters.
#[test]
fn reordered_character_clusters_keep_the_minimum_boundary_safe() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
            "\u{030B}\u{0323}\u{031B}",
            "--cluster-level=2 --show-flags --no-glyph-names",
        ),
        "[986=2+0#1|987=1+0#1|984=0+0]"
    );
}

/// A zero-valued PairPosFormat1 record is still a concat hazard. In the test
/// font, changing `X` to `V` selects another record and changes `A`'s advance.
#[test]
fn pair_pos_format1_zero_record_is_unsafe_to_concat() {
    assert_eq!(
        crate::shape(
            "tests/fonts/pairpos1-zero-record.otf",
            "BAXB",
            "--features=test --no-glyph-names --no-positions --no-clusters \
             --show-flags --unsafe-to-concat",
        ),
        "[66|65#2|88#2|66]"
    );
}

/// A ligature set that cannot possibly match reports an unsafe-to-concat
/// hazard, whichever path reaches that conclusion.
///
/// `LigatureSet::apply` has two of them: a walk over the ligatures, and a fast
/// path that first asks a digest whether any ligature takes this second glyph
/// at all. The walk reports a hazard when a multi-component ligature wants a
/// different second glyph; the digest was returning without reporting one, so
/// the same font and the same text gave different flags depending on which
/// path ran. These two fonts are where that showed.
///
/// HarfBuzz 14.1.0 -- and its main branch, which has not touched these files
/// since -- reports no hazard here. Its own two paths disagree the same way,
/// and the walk is the one that looks right, so this follows the walk.
#[test]
fn ligature_digest_reports_the_same_concat_hazard_as_the_walk() {
    for font in [
        "tests/fonts/aots/gsub4_1_multiple_ligatures_f1.otf",
        "tests/fonts/aots/gsub4_1_multiple_ligatures_f2.otf",
    ] {
        assert_eq!(
            crate::shape(
                font,
                "\u{0012}\u{0015}",
                "--features=test --no-clusters --no-glyph-names --ned \
                 --show-flags --unsafe-to-concat",
            ),
            "[18#2|21@1500,0#2]",
            "{font}"
        );
    }
}

// Nikhahit moves ahead of every above-base mark before SARA AM, keeping each
// mark rather than repeating the first.
#[test]
fn thai_sara_am_after_two_above_base_marks() {
    assert_eq!(
        crate::shape(
            "tests/fonts/in-house/63a539a90a371ccf028dc2dcced9b63b07163be7.ttf",
            "\u{0E01}\u{0E48}\u{0E49}\u{0E33}",
            "",
        ),
        "[uni0E01=0+500|uni0E4D=0+0|uni0E48=0+0|uni0E49=0+0|uni0E32=0+500]"
    );
}

#[test]
fn tatweel_flag_production_cleans_singleton_cluster_flags() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
            "\u{0302}\u{0300}",
            "--cluster-level=2 --safe-to-insert-tatweel --show-flags --no-glyph-names",
        ),
        "[976=0+0|974=1@0,249+0#1]"
    );
}

#[test]
fn singleton_tatweel_opportunity_is_unsafe_to_break() {
    assert_eq!(
        crate::shape(
            "tests/fonts/in-house/73c3222a2992bac9067663888d2a1503774976bb.ttf",
            "\u{0628}\u{0628}",
            "--safe-to-insert-tatweel --show-flags --no-glyph-names",
        ),
        "[5=1+200#5|1=0+200]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_list_end() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-list-end.ttf", "aab", ""),
        "[a=0@50,0+611|a=1@40,0+601|b=2@31,0+646]"
    );
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-list-end.ttf",
            "aab",
            "--show-flags --unsafe-to-concat"
        ),
        "[a=0@50,0+611#2|a=1@40,0+601#3|b=2@31,0+646#3]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_negative_list() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-negative-list.ttf", "aab", ""),
        "[a=0@-50,0+511|a=1@-40,0+521|b=2@-31,0+584]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_value_index() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-value-index.ttf", "ab", ""),
        "[a=0+561|b=1@30,0+645]"
    );
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-value-index.ttf", "cb", ""),
        "[c=0+480|b=1@70,0+685]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_short_list() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-short-list.ttf", "aab", ""),
        "[a=0+561|a=1+561|b=2@31,0+646]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_retained_stack() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-retained-stack.ttf", "aab", ""),
        "[a=0@17,0+578|a=1@13,0+574|b=2@31,0+646]"
    );
}
