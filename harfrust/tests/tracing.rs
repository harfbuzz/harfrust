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
        assert!(!snapshot.is_empty());
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
