//! Tests for the unified [`Buffer`] type.

use std::fs;
use std::path::PathBuf;

use harfrust::{
    font::Font, shape, Buffer, ContentType, Direction, ShapeError, ShapeOptions, ShaperFont,
};

fn test_instance() -> Font {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("rb_custom")
        .join("OpenSans.subset1.ttf");
    let data = fs::read(path).expect("failed to read test font");
    let font = Font::new(data, 0).expect("failed to parse test font");
    font.instance_builder().build()
}

fn shape_with_instance(
    instance: &Font,
    buffer: &mut Buffer,
    options: ShapeOptions<'_>,
) -> Result<(), ShapeError> {
    let shaper = ShaperFont::new(instance);
    let font = shaper;
    shape(&font, buffer, options)
}

fn ids_and_clusters(infos: &[harfrust::GlyphInfo]) -> Vec<(u32, u32)> {
    infos.iter().map(|i| (i.glyph_id, i.cluster)).collect()
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
    assert_eq!(buffer.content_type(), Some(ContentType::Unicode));
    assert_eq!(buffer.len(), TEXT.chars().count());

    let mut buffer = Buffer::new();
    buffer.push('x' as u32, 0);
    assert_eq!(buffer.content_type(), Some(ContentType::Unicode));

    let mut buffer = Buffer::new();
    buffer.push_codepoints(&['a' as u32, 'b' as u32]);
    assert_eq!(buffer.content_type(), Some(ContentType::Unicode));
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
    shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.content_type(), Some(ContentType::Glyphs));
    assert!(!buffer.is_empty());
    assert_eq!(buffer.glyph_positions().len(), buffer.len());
    // Running out of room is reported here rather than as a `ShapeError`.
    assert!(buffer.allocation_successful());
}

#[test]
fn corrupted_shaping_data_uses_empty_layout_without_failing() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fonts/rb_custom/OpenSans.subset1.ttf");
    let data = fs::read(path).unwrap();
    let font = Font::new(data, 0).unwrap();
    let instance = font.instance_builder().build();
    harfrust::font::_font_interop::_get_or_init_shaping_data(&font, || Box::new(42_u32));

    let shaping_font = ShaperFont::new(&instance);
    let mut buffer = unicode_buffer(TEXT);
    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.content_type(), Some(ContentType::Glyphs));
    assert_eq!(buffer.len(), TEXT.chars().count());
}

#[test]
fn shaping_a_shaped_buffer_is_refused() {
    let instance = test_instance();
    let mut buffer = unicode_buffer(TEXT);
    shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).unwrap();
    let once = ids_and_clusters(buffer.glyph_infos());

    // Shaping glyphs as though they were text would be nonsense, so it is
    // reported rather than quietly ignored.
    assert_eq!(
        shape_with_instance(&instance, &mut buffer, ShapeOptions::new()),
        Err(ShapeError::AlreadyShaped)
    );
    assert_eq!(
        ids_and_clusters(buffer.glyph_infos()),
        once,
        "a refused call must leave the buffer alone"
    );

    // Relabelling the contents is how you ask for them to be shaped again.
    buffer.set_content_type(Some(ContentType::Unicode));
    assert!(shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).is_ok());
}

#[test]
fn shaping_without_a_direction_is_refused() {
    let instance = test_instance();
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    // No direction, and none guessed.
    assert_eq!(buffer.direction(), Direction::Invalid);
    assert_eq!(
        shape_with_instance(&instance, &mut buffer, ShapeOptions::new()),
        Err(ShapeError::DirectionUnset)
    );
    assert_eq!(
        buffer.content_type(),
        Some(ContentType::Unicode),
        "a refused call must leave the buffer alone"
    );

    buffer.guess_segment_properties();
    assert!(shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).is_ok());
}

#[test]
fn shaping_with_a_mismatched_plan_is_refused() {
    use harfrust::{Script, ShapePlan};

    let instance = test_instance();
    let plan = ShapePlan::new(
        &instance,
        Direction::RightToLeft,
        Some(Script::ARABIC),
        None,
        &[],
    );

    let mut buffer = unicode_buffer(TEXT);
    let before = ids_and_clusters(buffer.glyph_infos());
    let err = shape_with_instance(
        &instance,
        &mut buffer,
        ShapeOptions::new().plan(Some(&plan)),
    )
    .unwrap_err();
    // The direction is checked first.
    assert_eq!(
        err,
        ShapeError::DirectionMismatch {
            plan: Direction::RightToLeft,
            buffer: Direction::LeftToRight,
        }
    );
    assert_eq!(
        ids_and_clusters(buffer.glyph_infos()),
        before,
        "a refused call must leave the buffer alone"
    );
    assert_eq!(buffer.content_type(), Some(ContentType::Unicode));

    // With the direction agreed, the script is checked next.
    let plan = ShapePlan::new(
        &instance,
        Direction::LeftToRight,
        Some(Script::ARABIC),
        None,
        &[],
    );
    let err = shape_with_instance(
        &instance,
        &mut buffer,
        ShapeOptions::new().plan(Some(&plan)),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ShapeError::ScriptMismatch {
            plan: Script::ARABIC,
            buffer: Script::LATIN,
        }
    );
}

#[test]
fn a_matching_plan_shapes() {
    use harfrust::{Script, ShapePlan};

    let instance = test_instance();
    let plan = ShapePlan::new(
        &instance,
        Direction::LeftToRight,
        Some(Script::LATIN),
        None,
        &[],
    );

    let mut planned = unicode_buffer(TEXT);
    shape_with_instance(
        &instance,
        &mut planned,
        ShapeOptions::new().plan(Some(&plan)),
    )
    .unwrap();

    let mut direct = unicode_buffer(TEXT);
    shape_with_instance(&instance, &mut direct, ShapeOptions::new()).unwrap();

    assert_eq!(
        ids_and_clusters(planned.glyph_infos()),
        ids_and_clusters(direct.glyph_infos())
    );
}

#[test]
fn shape_errors_describe_themselves() {
    assert_eq!(
        ShapeError::AlreadyShaped.to_string(),
        "buffer already holds shaped glyphs"
    );
    assert_eq!(
        ShapeError::DirectionMismatch {
            plan: Direction::RightToLeft,
            buffer: Direction::LeftToRight,
        }
        .to_string(),
        "buffer direction does not match plan direction: LeftToRight != RightToLeft"
    );
}
