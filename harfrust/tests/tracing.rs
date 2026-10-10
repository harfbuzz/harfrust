#![cfg(feature = "tracing")]

use harfrust::{
    font::Font, shape, Buffer, ContentType, Feature, SerializeFlags, ShapeOptions, ShaperFont,
};
use std::sync::{Arc, Mutex};

fn instance() -> Font {
    Font::new(
        include_bytes!("../benches/fonts/Roboto-Regular.ttf").to_vec(),
        0,
    )
    .unwrap()
    .instance_builder()
    .build()
}

fn buffer(text: &str) -> Buffer {
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer
}

#[test]
fn callback_observes_unicode_glyphs_and_positions() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut buffer = buffer("o\u{0301}ffice AV");
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    let mut calls = 0;
    buffer.set_message_function(move |buffer, font, message| {
        calls += 1;
        assert_eq!(buffer.glyph_infos().len(), buffer.len());
        assert!(
            buffer.glyph_positions().is_empty() || buffer.glyph_positions().len() == buffer.len()
        );
        let snapshot = buffer.serialize(Some(font), SerializeFlags::default());
        assert_ne!(snapshot, "");
        captured.lock().unwrap().push((
            calls,
            message.to_string(),
            buffer.content_type(),
            !buffer.glyph_positions().is_empty(),
            snapshot,
        ));
        true
    });
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    let messages = messages.lock().unwrap();
    assert_eq!(messages[0].1, "start decompose");
    assert_eq!(messages[0].4, "<U+006F=0|U+0301=0|U+0066=3|U+0066=4|U+0069=5|U+0063=6|U+0065=7|U+0020=8|U+0041=9|U+0056=10>");
    let normalization: Vec<_> = messages
        .iter()
        .filter(|(_, message, ..)| {
            message.ends_with("decompose")
                || message.ends_with("reorder")
                || message.ends_with("compose")
        })
        .map(|(_, message, ..)| message.as_str())
        .collect();
    assert_eq!(
        normalization,
        [
            "start decompose",
            "end decompose",
            "start reorder",
            "end reorder",
            "start compose",
            "end compose"
        ]
    );
    assert!(messages
        .iter()
        .any(|(_, message, content, positioned, snapshot)| {
            message == "start reorder"
                && *content == Some(ContentType::Unicode)
                && !positioned
                && snapshot.contains("U+")
        }));
    assert!(messages.iter().any(|(_, message, content, positioned, _)| {
        message.starts_with("start table GSUB")
            && *content == Some(ContentType::Glyphs)
            && !positioned
    }));
    assert!(messages.iter().any(|(_, message, _, positioned, _)| {
        message.starts_with("end table GPOS") && *positioned
    }));
    assert_eq!(messages.last().unwrap().0, messages.len());
}

#[test]
fn rejecting_gsub_matches_disabling_ligatures() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut skipped = buffer("ffi");
    skipped.set_message_function(|_, _, message| !message.starts_with("start table GSUB"));
    shape(&font, &mut skipped, ShapeOptions::new()).unwrap();

    let mut disabled = buffer("ffi");
    let features = ["liga=0".parse::<Feature>().unwrap()];
    shape(
        &font,
        &mut disabled,
        ShapeOptions::new().features(&features),
    )
    .unwrap();
    assert_eq!(
        skipped.serialize(Some(&font), SerializeFlags::default()),
        disabled.serialize(Some(&font), SerializeFlags::default())
    );
    assert_eq!(skipped.len(), 3);

    let mut normal = buffer("ffi");
    shape(&font, &mut normal, ShapeOptions::new()).unwrap();
    assert!(normal.len() < skipped.len());
}

#[test]
fn rejecting_lookup_skips_its_operations_and_end_message() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut buffer = buffer("ffi");
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    buffer.set_message_function(move |_, _, message| {
        captured.lock().unwrap().push(message.to_string());
        !message.starts_with("start lookup")
    });
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.len(), 3);
    let messages = messages.lock().unwrap();
    assert!(messages
        .iter()
        .any(|message| message.starts_with("start lookup") && message.contains("feature 'liga'")));
    assert!(!messages
        .iter()
        .any(|message| message.starts_with("end lookup") || message.starts_with("ligating")));
    assert!(messages
        .iter()
        .any(|message| message.starts_with("end table GSUB")));
}

#[test]
fn return_value_from_end_messages_does_not_skip_shaping() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut normal = buffer("ffi AV");
    shape(&font, &mut normal, ShapeOptions::new()).unwrap();
    let mut traced = buffer("ffi AV");
    traced.set_message_function(|_, _, message| !message.starts_with("end"));
    shape(&font, &mut traced, ShapeOptions::new()).unwrap();
    assert_eq!(
        traced.serialize(Some(&font), SerializeFlags::default()),
        normal.serialize(Some(&font), SerializeFlags::default())
    );
}

#[test]
fn callback_survives_reuse_and_can_be_replaced_or_removed() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut buffer = buffer("ffi");
    let calls = Arc::new(Mutex::new(0));
    let captured = calls.clone();
    buffer.set_message_function(move |_, _, _| {
        *captured.lock().unwrap() += 1;
        true
    });
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    let first = *calls.lock().unwrap();
    assert!(first > 0);
    for reset in [false, true] {
        if reset {
            buffer.reset();
        } else {
            buffer.clear();
        }
        buffer.push_str("ffi");
        buffer.guess_segment_properties();
        shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    }
    assert_eq!(*calls.lock().unwrap(), first * 3);
    buffer.set_message_function(|_, _, _| true);
    buffer.clear();
    buffer.push_str("ffi");
    buffer.guess_segment_properties();
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    buffer.clear_message_function();
    buffer.clear();
    buffer.push_str("ffi");
    buffer.guess_segment_properties();
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(*calls.lock().unwrap(), first * 3);
}

#[test]
fn tracing_preserves_buffer_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Buffer>();
}

#[test]
fn tracing_preserves_results_at_the_shaping_limit() {
    let instance = Font::new(
        include_bytes!("fonts/text-rendering-tests/TestGSUBThree.ttf").to_vec(),
        0,
    )
    .unwrap()
    .instance_builder()
    .build();
    let font = ShaperFont::new(&instance);
    let mut normal = buffer("lol");
    shape(&font, &mut normal, ShapeOptions::new()).unwrap();
    let mut traced = buffer("lol");
    traced.set_message_function(|buffer, _, _| {
        assert_eq!(buffer.glyph_infos().len(), buffer.len());
        true
    });
    shape(&font, &mut traced, ShapeOptions::new()).unwrap();
    assert_eq!(
        traced.allocation_successful(),
        normal.allocation_successful()
    );
    assert_eq!(traced.len(), normal.len());
    let observed = traced.serialize(Some(&font), SerializeFlags::default());
    let expected = normal.serialize(Some(&font), SerializeFlags::default());
    assert_eq!(observed.len(), expected.len());
    // Report only the first differing byte for these very large buffers.
    assert_eq!(
        observed
            .bytes()
            .zip(expected.bytes())
            .position(|(a, b)| a != b),
        None
    );
}

#[test]
fn rejecting_composition_keeps_decomposed_glyphs() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut normal = buffer("o\u{0301}");
    // GSUB can compose the pair through ccmp independently of normalization.
    normal.set_message_function(|_, _, message| !message.starts_with("start table GSUB"));
    shape(&font, &mut normal, ShapeOptions::new()).unwrap();
    let mut skipped = buffer("o\u{0301}");
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    skipped.set_message_function(move |_, _, message| {
        captured.lock().unwrap().push(message.to_string());
        message != "start compose" && !message.starts_with("start table GSUB")
    });
    shape(&font, &mut skipped, ShapeOptions::new()).unwrap();
    assert_eq!(normal.len(), 1);
    assert_eq!(skipped.len(), 2);
    let messages = messages.lock().unwrap();
    assert!(messages.iter().any(|m| m == "start compose"));
    assert!(!messages.iter().any(|m| m == "end compose"));
}

#[test]
fn recursion_notifications_are_balanced_and_informational() {
    let instance = Font::new(
        include_bytes!("fonts/aots/gsub_context1_simple_f1.otf").to_vec(),
        0,
    )
    .unwrap()
    .instance_builder()
    .build();
    let font = ShaperFont::new(&instance);
    let features = ["test".parse::<Feature>().unwrap()];
    let mut buffer = buffer("\0\u{0014}\u{0015}\u{0016}\0");
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    buffer.set_message_function(move |_, _, message| {
        if message.contains("recursing") {
            captured.lock().unwrap().push(message.to_string());
        }
        !message.starts_with("start recursing")
    });
    shape(&font, &mut buffer, ShapeOptions::new().features(&features)).unwrap();
    let ids: Vec<_> = buffer
        .glyph_infos()
        .iter()
        .map(|info| info.glyph_id)
        .collect();
    assert_eq!(ids, [0, 60, 61, 62, 0]);
    assert_eq!(
        *messages.lock().unwrap(),
        [
            "start recursing to lookup 0 at 1",
            "end recursing to lookup 0",
            "start recursing to lookup 0 at 2",
            "end recursing to lookup 0",
            "start recursing to lookup 0 at 3",
            "end recursing to lookup 0",
        ]
    );
}

fn cli_trace(font: &str, text: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fonts")
        .join(font);
    hr_shape::shape(path.to_str().unwrap(), text, "--trace").unwrap()
}

#[test]
fn kern_subtable_and_machine_messages_nest_and_can_skip() {
    let output = cli_trace("text-rendering-tests/TestKERNOne.otf", "uT");
    let messages: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("trace: ")?.split_once("\tbuffer: "))
        .map(|(message, _)| message)
        .filter(|message| message.contains("kern") || message.contains("subtable"))
        .collect();
    assert_eq!(
        messages,
        [
            "start table kern",
            "start subtable 0",
            "start kern",
            "end kern",
            "end subtable 0",
            "end table kern"
        ]
    );
    // Missing layout tables are reported with an empty chosen-script tag.
    assert!(output.contains("start table GSUB script tag ''\tbuffer: "));
    let instance = Font::new(
        include_bytes!("fonts/text-rendering-tests/TestKERNOne.otf").to_vec(),
        0,
    )
    .unwrap()
    .instance_builder()
    .build();
    let font = ShaperFont::new(&instance);
    let mut skipped = buffer("uT");
    skipped.set_message_function(|_, _, message| message != "start kern");
    shape(&font, &mut skipped, ShapeOptions::new()).unwrap();
    let mut disabled = buffer("uT");
    let features = ["kern=0".parse::<Feature>().unwrap()];
    shape(
        &font,
        &mut disabled,
        ShapeOptions::new().features(&features),
    )
    .unwrap();
    assert_eq!(
        skipped.serialize(Some(&font), SerializeFlags::default()),
        disabled.serialize(Some(&font), SerializeFlags::default())
    );
}

#[test]
fn mort_trace_uses_the_mort_table_name() {
    let output = cli_trace("text-rendering-tests/TestAATMort.ttf", "ABCEFGX");
    assert!(output.contains("trace: start table mort\tbuffer: "));
    assert!(output.contains("trace: end table mort\tbuffer: "));
    assert!(!output.contains("table morx"));
}

#[test]
fn required_features_use_blank_trace_tags() {
    let output = cli_trace(
        "in-house/a59fd13f1525a91cbe529c882e93d9d1fbb80463.ttf",
        "AB",
    );
    assert!(output.contains("trace: start lookup 0 feature '    '\tbuffer: "));
    assert!(output.contains("trace: end lookup 0 feature '    '\tbuffer: "));
}

#[test]
fn unavailable_glyph_extents_are_omitted() {
    let instance = instance();
    let font = ShaperFont::new(&instance);
    let mut buffer = buffer("ffi");
    shape(&font, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(
        buffer.serialize(None, SerializeFlags::GLYPH_EXTENTS),
        buffer.serialize(None, SerializeFlags::default())
    );
}
