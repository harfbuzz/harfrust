use std::{fs, path::PathBuf};

use harfrust::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};

fn font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("text-rendering-tests")
        .join(name)
}

// This test uses `TestGPOSThree.ttf`, which has a GPOS table.
// It verifies that GPOS attachment offset propagation scales safely without integer overflow
// for extremely long grapheme clusters (as fixed in commit 6bccd74).
#[test]
fn issue_384_overly_long_grapheme_cluster_gpos_does_not_overflow() {
    let font_data = fs::read(font_path("TestGPOSThree.ttf")).expect("failed to read test font");
    let font = FontRef::new(&font_data).expect("failed to parse test font");
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();

    let mut text = String::with_capacity(35_002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 35_000));
    text.push('X');

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shaper.shape(buffer, ShapeOptions::new());
}

// Uses `Calculator-Regular.ttf` (no GPOS table) to verify fallback mark positioning.
// A large scale factor amplifies mark heights, triggering fallback integer overflows
// with a lower grapheme count of 5,000.
#[test]
fn issue_384_overly_long_grapheme_cluster_fallback_does_not_overflow() {
    let font_data =
        fs::read(font_path("Calculator-Regular.ttf")).expect("failed to read test font");
    let font = FontRef::new(&font_data).expect("failed to parse test font");
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();

    let mut text = String::with_capacity(5002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 5000));
    text.push('X');

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shaper.shape(buffer, ShapeOptions::new().scale(Some(10_000_000)));
}

#[test]
fn shaping_long_line_kern_does_not_overflow_glyph_data() {
    let font_data = fs::read(font_path("TestKERNOne.otf")).expect("failed to read test font");
    let font = FontRef::new(&font_data).expect("failed to parse test font");
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();

    let mut text = String::with_capacity(70_000);
    for _ in 0..35_000 {
        text.push('u');
        text.push('T');
    }

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shaper.shape(buffer, ShapeOptions::new());
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
