//! Tests for the unified [`Buffer`] type and its conversions to and from the
//! typed [`UnicodeBuffer`] / [`GlyphBuffer`] pair.

use std::fs;
use std::path::PathBuf;

use harfrust::{
    font::{Font, FontInstance},
    shape, Buffer, BufferContentType, Direction, GlyphBuffer, ShapeOptions, UnicodeBuffer,
};

fn test_instance() -> FontInstance {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("rb_custom")
        .join("OpenSans.subset1.ttf");
    let data = fs::read(path).expect("failed to read test font");
    let font = Font::new(data, 0).expect("failed to parse test font");
    FontInstance::builder(&font).build()
}

fn ids_and_clusters(infos: &[harfrust::GlyphInfo]) -> Vec<(u32, u32)> {
    infos.iter().map(|i| (i.glyph_id, i.cluster)).collect()
}

fn advances(positions: &[harfrust::GlyphPosition]) -> Vec<(i32, i32, i32, i32)> {
    positions
        .iter()
        .map(|p| (p.x_advance, p.y_advance, p.x_offset, p.y_offset))
        .collect()
}

const TEXT: &str = "Hello, world!";

fn unicode_buffer(text: &str) -> Buffer {
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer
}

#[test]
fn new_buffer_has_no_content_type() {
    let buffer = Buffer::new();
    assert_eq!(buffer.content_type(), None);
    assert!(buffer.is_empty());
    assert_eq!(buffer.len(), 0);
}

#[test]
fn adding_text_sets_unicode_content_type() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    assert_eq!(buffer.content_type(), Some(BufferContentType::Unicode));
    assert_eq!(buffer.len(), TEXT.chars().count());

    let mut buffer = Buffer::new();
    buffer.push('x' as u32, 0);
    assert_eq!(buffer.content_type(), Some(BufferContentType::Unicode));

    let mut buffer = Buffer::new();
    buffer.push_codepoints(&['a' as u32, 'b' as u32]);
    assert_eq!(buffer.content_type(), Some(BufferContentType::Unicode));
    assert_eq!(buffer.len(), 2);
}

#[test]
fn glyph_infos_readable_before_shaping() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    // Before shaping, `glyph_id` holds the input codepoint.
    let codepoints: Vec<u32> = buffer.glyph_infos().iter().map(|i| i.glyph_id).collect();
    assert_eq!(
        codepoints,
        TEXT.chars().map(|c| c as u32).collect::<Vec<_>>()
    );
    // ... and no positions have been allocated yet.
    assert!(buffer.glyph_positions().is_empty());
}

#[test]
fn glyph_positions_mut_allocates_on_demand() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    assert!(buffer.glyph_positions().is_empty());
    let positions = buffer.glyph_positions_mut();
    assert_eq!(positions.len(), TEXT.chars().count());
    assert!(positions
        .iter()
        .all(|p| p.x_advance == 0 && p.y_advance == 0));
}

#[test]
fn shaping_sets_glyphs_content_type() {
    let instance = test_instance();
    let mut buffer = unicode_buffer(TEXT);
    buffer.shape(&instance, ShapeOptions::new());
    assert_eq!(buffer.content_type(), Some(BufferContentType::Glyphs));
    assert!(!buffer.is_empty());
    assert_eq!(buffer.glyph_positions().len(), buffer.len());
}

#[test]
fn shaping_a_shaped_buffer_is_a_noop() {
    let instance = test_instance();
    let mut buffer = unicode_buffer(TEXT);
    buffer.shape(&instance, ShapeOptions::new());
    let once = ids_and_clusters(buffer.glyph_infos());

    buffer.shape(&instance, ShapeOptions::new());
    assert_eq!(ids_and_clusters(buffer.glyph_infos()), once);
}

#[test]
fn buffer_matches_typed_buffer_shaping() {
    let instance = test_instance();

    let mut typed = UnicodeBuffer::new();
    typed.push_str(TEXT);
    typed.guess_segment_properties();
    let typed = shape(&instance, typed, ShapeOptions::new());

    let mut unified = unicode_buffer(TEXT);
    unified.shape(&instance, ShapeOptions::new());

    assert_eq!(
        ids_and_clusters(typed.glyph_infos()),
        ids_and_clusters(unified.glyph_infos())
    );
    assert_eq!(
        advances(typed.glyph_positions()),
        advances(unified.glyph_positions())
    );
}

#[test]
fn round_trip_through_unicode_buffer() {
    let mut typed = UnicodeBuffer::new();
    typed.push_str(TEXT);
    typed.set_direction(Direction::LeftToRight);

    let unified = Buffer::from(typed);
    assert_eq!(unified.content_type(), Some(BufferContentType::Unicode));
    assert_eq!(unified.direction(), Direction::LeftToRight);
    assert_eq!(unified.len(), TEXT.chars().count());

    let back = UnicodeBuffer::try_from(unified).expect("unicode content should convert back");
    assert_eq!(back.len(), TEXT.chars().count());
    assert_eq!(back.direction(), Direction::LeftToRight);
}

#[test]
fn round_trip_through_glyph_buffer() {
    let instance = test_instance();
    let mut typed = UnicodeBuffer::new();
    typed.push_str(TEXT);
    typed.guess_segment_properties();
    let shaped = shape(&instance, typed, ShapeOptions::new());
    let expected = ids_and_clusters(shaped.glyph_infos());

    let unified = Buffer::from(shaped);
    assert_eq!(unified.content_type(), Some(BufferContentType::Glyphs));
    assert_eq!(ids_and_clusters(unified.glyph_infos()), expected);

    let back = GlyphBuffer::try_from(unified).expect("glyph content should convert back");
    assert_eq!(ids_and_clusters(back.glyph_infos()), expected);
}

#[test]
fn conversions_reject_mismatched_content() {
    let instance = test_instance();

    // A shaped buffer is not a UnicodeBuffer.
    let mut buffer = unicode_buffer(TEXT);
    buffer.shape(&instance, ShapeOptions::new());
    let err = UnicodeBuffer::try_from(buffer).unwrap_err();
    assert_eq!(err.found, Some(BufferContentType::Glyphs));
    assert_eq!(err.expected, BufferContentType::Unicode);

    // ... and an unshaped one is not a GlyphBuffer.
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    let err = GlyphBuffer::try_from(buffer).unwrap_err();
    assert_eq!(err.found, Some(BufferContentType::Unicode));
    assert_eq!(err.expected, BufferContentType::Glyphs);
}

#[test]
fn empty_buffer_converts_either_way() {
    // With no content type set, an empty buffer is still valid input.
    assert!(UnicodeBuffer::try_from(Buffer::new()).is_ok());
    assert!(GlyphBuffer::try_from(Buffer::new()).is_err());
}

#[test]
fn clear_resets_content_type() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    assert_eq!(buffer.content_type(), Some(BufferContentType::Unicode));
    buffer.clear();
    assert_eq!(buffer.content_type(), None);
    assert!(buffer.is_empty());
}

#[test]
fn reset_restores_defaults() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    buffer.set_direction(Direction::RightToLeft);
    buffer.set_cluster_level(harfrust::BufferClusterLevel::Characters);
    buffer.set_flags(harfrust::BufferFlags::BEGINNING_OF_TEXT);

    buffer.reset();
    assert_eq!(buffer.content_type(), None);
    assert_eq!(buffer.direction(), Direction::Invalid);
    assert_eq!(
        buffer.cluster_level(),
        harfrust::BufferClusterLevel::MonotoneGraphemes
    );
    assert_eq!(buffer.flags().bits(), harfrust::BufferFlags::empty().bits());
}

#[test]
fn set_length_grows_and_truncates() {
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    let original = buffer.len();

    assert!(buffer.set_length(original + 3));
    assert_eq!(buffer.len(), original + 3);
    assert!(buffer.glyph_infos()[original..]
        .iter()
        .all(|i| i.glyph_id == 0));

    assert!(buffer.set_length(2));
    assert_eq!(buffer.len(), 2);
}

#[test]
fn reverse_and_reverse_clusters() {
    let mut buffer = Buffer::new();
    // Two clusters of two items each.
    buffer.push('a' as u32, 0);
    buffer.push('b' as u32, 0);
    buffer.push('c' as u32, 1);
    buffer.push('d' as u32, 1);

    let mut reversed = Buffer::new();
    reversed.push_glyph_infos(buffer.glyph_infos());
    reversed.reverse();
    let codepoints: Vec<u32> = reversed.glyph_infos().iter().map(|i| i.glyph_id).collect();
    assert_eq!(
        codepoints,
        vec!['d' as u32, 'c' as u32, 'b' as u32, 'a' as u32]
    );

    // Reversing by cluster keeps each cluster's items in their original order.
    buffer.reverse_clusters();
    let codepoints: Vec<u32> = buffer.glyph_infos().iter().map(|i| i.glyph_id).collect();
    assert_eq!(
        codepoints,
        vec!['c' as u32, 'd' as u32, 'a' as u32, 'b' as u32]
    );
}

#[test]
fn properties_round_trip() {
    let mut buffer = Buffer::new();
    buffer.set_direction(Direction::RightToLeft);
    buffer.set_script(harfrust::script::ARABIC);
    buffer.set_language(harfrust::Language::new("ar").unwrap());
    buffer.set_invisible_glyph(Some(harfrust::GlyphId::new(3)));
    buffer.set_not_found_variation_selector_glyph(Some(7));

    assert_eq!(buffer.direction(), Direction::RightToLeft);
    assert_eq!(buffer.script(), harfrust::script::ARABIC);
    assert_eq!(buffer.language().unwrap().as_str(), "ar");
    assert_eq!(buffer.invisible_glyph(), Some(harfrust::GlyphId::new(3)));
    assert_eq!(buffer.not_found_variation_selector_glyph(), Some(7));
}
