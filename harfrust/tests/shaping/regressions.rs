use std::{fs, path::PathBuf};

use harfrust::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};

fn font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("text-rendering-tests")
        .join(name)
}

#[test]
fn issue_384_overly_long_grapheme_cluster_does_not_overflow() {
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
