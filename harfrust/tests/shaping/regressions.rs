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
        "[a=0@42,0+603|a=1@17,0+578|b=2@44,0+659]"
    );
}

// The format 4 subtable runs backwards, attaching each fatha to its base.
// Disable format 1 kerning so this regression isolates attachment direction.
#[test]
fn kerx_process_direction_arabic_marks() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-backwards-arabic.ttf",
            "\u{0648}\u{064E}\u{0644}\u{064E}\u{062F}",
            "--features=-kern",
        ),
        "[uni062F.fina=4+474|uni064E=2@-10,-28+0|uni0644.init=2+212|uni064E=0@157,-211+0|uni0648=0+468]"
    );
}

#[test]
fn kerx_process_direction_ltr_marks() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-backwards-mark.ttf", "xb", ""),
        "[m=0@325,600+150|b=1+615]"
    );
}

#[test]
fn kerx_process_direction_ltr_kerning() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-backwards-kern.ttf", "ba", ""),
        "[b=0@30,0+645|a=1+561]"
    );
}

// Native CoreText reference positions were measured on macOS 15.8.2 and 26.6.2.
// Unlike Apple's written pop/reset rules, CoreText retains eight circular
// history slots, initially referring to the first glyph, and ignores Reset.
// Each action list walks the history independently, wrapping every eight values.
// See https://github.com/harfbuzz/harfbuzz/pull/6540.
#[test]
fn kerx_coretext_stack_leftover() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-stack-leftover.ttf", "abc", ""),
        "[a=0+561|b=1@30,0+645|c=2+480]"
    );
}

#[test]
fn kerx_coretext_stack_extra_value() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-stack-extra-value.ttf", "ab", ""),
        "[a=0@60,0+621|b=1@10,0+625]"
    );
}

#[test]
fn kerx_coretext_stack_double_push() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-stack-double-push.ttf", "ab", ""),
        "[a=0@30,0+591|b=1+615]"
    );
}

#[test]
fn kerx_coretext_stack_zero_first() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-stack-zero-first.ttf", "ab", ""),
        "[a=0@30,0+591|b=1+615]"
    );
}

#[test]
fn kerx_coretext_stack_zero_middle() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-stack-zero-middle.ttf",
            "abc",
            ""
        ),
        "[a=0@30,0+591|b=1+615|c=2@10,0+490]"
    );
}

#[test]
fn kerx_coretext_stack_negative_odd() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-stack-negative-odd.ttf",
            "aab",
            ""
        ),
        "[a=0@-50,0+511|a=1@-40,0+521|b=2@-31,0+584]"
    );
}

#[test]
fn kerx_coretext_stack_arabic_shape() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-stack-arabic-shape.ttf",
            "abcd",
            ""
        ),
        "[a=0@50,0+611|b=1+615|c=2@20,0+500|d=3+615]"
    );
}

#[test]
fn kerx_coretext_stream_20() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-stream-20.ttf", "abababababababababab", ""),
        "[a=0@10,0+571|b=1@10,0+625|a=2@10,0+571|b=3@10,0+625|a=4@10,0+571|b=5@10,0+625|a=6@10,0+571|b=7@10,0+625|a=8@10,0+571|b=9@10,0+625|a=10@10,0+571|b=11@10,0+625|a=12@10,0+571|b=13@10,0+625|a=14@10,0+571|b=15@10,0+625|a=16@10,0+571|b=17@10,0+625|a=18@10,0+571|b=19@10,0+625]"
    );
}

#[test]
fn kerx_coretext_reset_before_action() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-reset-before-action.ttf",
            "ab",
            ""
        ),
        "[a=0@30,0+591|b=1+615]"
    );
}

#[test]
fn kerx_coretext_reset_and_push() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-reset-and-push.ttf", "ab", ""),
        "[a=0@20,0+581|b=1@10,0+625]"
    );
}

#[test]
fn kerx_coretext_long_list_64() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-long-list-64.ttf", "a", ""),
        "[a=0@64,0+625]"
    );
}

#[test]
fn kerx_coretext_history_20() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-history-20.ttf", "abababababababababab", ""),
        "[a=0@10,0+571|b=1@10,0+625|a=2@10,0+571|b=3@10,0+625|a=4@10,0+571|b=5@10,0+625|a=6@10,0+571|b=7@10,0+625|a=8@10,0+571|b=9@10,0+625|a=10@10,0+571|b=11@10,0+625|a=12@34,0+595|b=13@32,0+647|a=14@30,0+591|b=15@49,0+664|a=16@46,0+607|b=17@43,0+658|a=18@40,0+601|b=19@37,0+652]"
    );
}

#[test]
fn kerx_coretext_history_wrap_and_reset() {
    let cases = [
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "a", "[a=0@241,0+802]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "ab", "[a=0@214,0+775|b=1@37,0+652]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "aba", "[a=0@184,0+745|b=1@40,0+655|a=2@37,0+598]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "abababa", "[a=0@56,0+617|b=1@30,0+645|a=2@49,0+610|b=3@46,0+661|a=4@43,0+604|b=5@40,0+655|a=6@37,0+598]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "abababab", "[a=0@34,0+595|b=1@32,0+647|a=2@30,0+591|b=3@49,0+664|a=4@46,0+607|b=5@43,0+658|a=6@40,0+601|b=7@37,0+652]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "ababababa", "[a=0@10,0+571|b=1@34,0+649|a=2@32,0+593|b=3@30,0+645|a=4@49,0+610|b=5@46,0+661|a=6@43,0+604|b=7@40,0+655|a=8@37,0+598]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "abababababababab", "[a=0@10,0+571|b=1@10,0+625|a=2@10,0+571|b=3@10,0+625|a=4@10,0+571|b=5@10,0+625|a=6@10,0+571|b=7@10,0+625|a=8@34,0+595|b=9@32,0+647|a=10@30,0+591|b=11@49,0+664|a=12@46,0+607|b=13@43,0+658|a=14@40,0+601|b=15@37,0+652]"),
        ("tests/fonts/rb_custom/kerx-history-20.ttf", "ababababababababa", "[a=0@10,0+571|b=1@10,0+625|a=2@10,0+571|b=3@10,0+625|a=4@10,0+571|b=5@10,0+625|a=6@10,0+571|b=7@10,0+625|a=8@10,0+571|b=9@34,0+649|a=10@32,0+593|b=11@30,0+645|a=12@49,0+610|b=13@46,0+661|a=14@43,0+604|b=15@40,0+655|a=16@37,0+598]"),
        ("tests/fonts/rb_custom/kerx-long-list-64.ttf", "ba", "[b=0@56,0+671|a=1@8,0+569]"),
        ("tests/fonts/rb_custom/kerx-long-list-64.ttf", "bba", "[b=0@56,0+671|b=1+615|a=2@8,0+569]"),
        ("tests/fonts/rb_custom/kerx-long-list-64.ttf", "aaa", "[a=0@168,0+729|a=1@16,0+577|a=2@8,0+569]"),
        ("tests/fonts/rb_custom/kerx-reset-and-push.ttf", "abab", "[a=0@20,0+581|b=1@10,0+625|a=2@20,0+581|b=3@10,0+625]"),
        ("tests/fonts/rb_custom/kerx-reset-and-push.ttf", "b", "[b=0@30,0+645]"),
        ("tests/fonts/rb_custom/kerx-reset-and-push.ttf", "abb", "[a=0@20,0+581|b=1@30,0+645|b=2@10,0+625]"),
        ("tests/fonts/rb_custom/kerx-reset-and-push.ttf", "abaaba", "[a=0@20,0+581|b=1@10,0+625|a=2+561|a=3@20,0+581|b=4@10,0+625|a=5+561]"),
        ("tests/fonts/rb_custom/kerx-reset-before-action.ttf", "ba", "[b=0+615|a=1@10,0+571]"),
        ("tests/fonts/rb_custom/kerx-reset-before-action.ttf", "abb", "[a=0@50,0+611|b=1+615|b=2+615]"),
        ("tests/fonts/rb_custom/kerx-coretext-arabic.ttf", "وَلَد", "[uni062F.fina=4+474|uni064E=2@-61,-28+-51|uni0644.init=2+212|uni064E=0@36,-211+-121|uni0648=0+468]"),
        ("tests/fonts/rb_custom/kerx-backwards-arabic.ttf", "وَلَد", "[uni062F.fina=4+474|uni064E=2@-61,-28+-51|uni0644.init=2+212|uni064E=0@36,-211+-121|uni0648=0+468]"),
        ("tests/fonts/rb_custom/kerx-reset-long-list.ttf", "aba", "[a=0@112,0+673|b=1@8,0+623|a=2@8,0+569]"),
        ("tests/fonts/rb_custom/kerx-reset-without-push-long-list.ttf", "baba", "[b=0@104,0+719|a=1@16,0+577|b=2+615|a=3@8,0+569]"),
        ("tests/fonts/rb_custom/kerx-reset-each-push-history.ttf", "abababababababababab", "[a=0@10,0+571|b=1@10,0+625|a=2@10,0+571|b=3@10,0+625|a=4@10,0+571|b=5@10,0+625|a=6@10,0+571|b=7@10,0+625|a=8@10,0+571|b=9@10,0+625|a=10@10,0+571|b=11@10,0+625|a=12@34,0+595|b=13@32,0+647|a=14@30,0+591|b=15@49,0+664|a=16@46,0+607|b=17@43,0+658|a=18@40,0+601|b=19@37,0+652]"),
    ];
    for (font, text, expected) in cases {
        assert_eq!(crate::shape(font, text, ""), expected, "{font}: {text:?}");
    }
}

// Removing a run of default ignorables after a right-to-left buffer has been
// reversed merges each deleted glyph backward into everything merged so far,
// so the work is quadratic in the run length. The extension walks of
// merge_clusters() must be charged to max_ops so the budget trips instead.
#[test]
fn merging_descending_clusters_is_charged_to_max_ops() {
    let font_data = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fonts/rb_custom/OpenSans.subset1.ttf"),
    )
    .expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaping_font = ShaperFont::new(&instance);

    let mut text = String::from("\u{05D0}");
    text.extend(std::iter::repeat_n('\u{00AD}', 20_000));

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();
    buffer.set_flags(harfrust::BufferFlags::REMOVE_DEFAULT_IGNORABLES);

    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
    assert!(!buffer.allocation_successful());
}
