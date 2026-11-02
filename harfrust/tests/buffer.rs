//! Tests for the unified [`Buffer`] type.

use std::fs;
use std::path::PathBuf;

use harfrust::{
    font::Font, shape, Buffer, BufferFlags, ContentType, Direction, ShapeError, ShapeOptions,
    ShaperFont,
};

fn test_instance() -> Font {
    font_instance("rb_custom/OpenSans.subset1.ttf")
}

fn font_instance(name: &str) -> Font {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fonts")
        .join(name);
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

/// Builds a font from `tables`, which must be sorted by tag.
fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut data = 0x0001_0000u32.to_be_bytes().to_vec();
    data.extend((tables.len() as u16).to_be_bytes());
    data.extend([0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, table) in tables {
        data.extend(*tag);
        data.extend(0u32.to_be_bytes());
        data.extend((offset as u32).to_be_bytes());
        data.extend((table.len() as u32).to_be_bytes());
        offset += table.len().next_multiple_of(4);
    }
    for (_, table) in tables {
        data.extend(table);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    data
}

/// A two-glyph font mapping `a` to glyph 1, whose version 1.0 `kern` table
/// holds `subtables` Format 0 subtables that each kern the pair (1, 1) by -2.
fn many_kern_subtables_font(subtables: usize) -> Vec<u8> {
    fn words(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
    // Version, revision, checksum, magic, flags, 1000 units per em.
    let mut head = words(&[1, 0, 0, 0, 0, 0, 0x5F0F, 0x3CF5, 0, 1000]);
    head.extend([0; 34]);
    // Version, metrics, and two entries in `hmtx`.
    let mut hhea = words(&[1, 0]);
    hhea.extend([0; 30]);
    hhea.extend(words(&[2]));
    // One format 12 subtable mapping U+0061 to glyph 1.
    let cmap = words(&[
        0, 1, 3, 10, 0, 12, 12, 0, 0, 28, 0, 0, 0, 1, 0, 0x61, 0, 0x61, 0, 1,
    ]);
    let mut kern = words(&[1, 0, (subtables >> 16) as u16, subtables as u16]);
    for _ in 0..subtables {
        // Length, coverage (horizontal, format 0), tuple index, one pair
        // with unused binary-search hints, glyphs 1 and 1, value -2.
        kern.extend(words(&[0, 22, 0, 0, 1, 0, 0, 0, 1, 1, 0xFFFE]));
    }
    sfnt(&[
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", vec![1, 244, 0, 0, 1, 244, 0, 0]),
        (b"kern", kern),
        (b"maxp", vec![0, 0, 0x50, 0, 0, 2]),
    ])
}

#[test]
fn kern_subtable_passes_are_charged_to_the_operation_limit() {
    // Each subtable that applies moves the second glyph of every pair one
    // unit left, so a glyph's x offset counts the passes made over the
    // buffer. Sixteen glyphs get the minimum budget of 65536 operations,
    // which pays for 4096 passes over them; the remaining subtables must
    // be skipped and reported through `allocation_successful`.
    let font = Font::new(many_kern_subtables_font(5000), 0).unwrap();
    let instance = font.instance_builder().build();
    let mut buffer = unicode_buffer(&"a".repeat(16));
    shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.len(), 16);
    assert_eq!(buffer.glyph_positions()[8].x_offset, -4096);
    assert!(!buffer.allocation_successful());

    // A font within the budget applies every subtable.
    let font = Font::new(many_kern_subtables_font(100), 0).unwrap();
    let instance = font.instance_builder().build();
    let mut buffer = unicode_buffer(&"a".repeat(16));
    shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.glyph_positions()[8].x_offset, -100);
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

#[test]
fn not_found_variation_selector_glyph_is_not_limited_to_u16() {
    // The font has no glyph for U+0066 U+FE00.
    let instance = font_instance("in-house/bbc24004e776f348a0f72287d24b0124867ee750.ttf");

    for glyph in [u32::from(u16::MAX) + 1, 1_000_000] {
        let mut buffer = unicode_buffer("f\u{FE00}");
        buffer.set_not_found_variation_selector_glyph(Some(glyph));
        shape_with_instance(&instance, &mut buffer, ShapeOptions::new()).unwrap();

        assert_eq!(ids_and_clusters(buffer.glyph_infos()), [(2, 0), (glyph, 0)]);
        assert_eq!(buffer.glyph_positions()[1].x_advance, 0);
    }
}

#[test]
fn emptied_buffer_shapes_like_a_fresh_one() {
    // The font has a dotted circle and GPOS lookups that run over the text.
    let instance = font_instance("in-house/1a5face3fcbd929d228235c2f72bbd6f8eb37424.ttf");
    // Starts with a mark, so shaping inserts a dotted circle in front of it.
    let text = "\u{0947}\u{0905}";

    let mut fresh = unicode_buffer(text);
    fresh.set_flags(BufferFlags::BEGINNING_OF_TEXT);
    shape_with_instance(&instance, &mut fresh, ShapeOptions::new()).unwrap();

    // Emptying the buffer with `set_length(0)` keeps the cursor where the
    // previous shaping left it; that must not leak into the next one.
    let mut reused = unicode_buffer("\u{0915}\u{094D}\u{0937}\u{093F} \u{0924}\u{094D}\u{0930}");
    reused.set_flags(BufferFlags::BEGINNING_OF_TEXT);
    shape_with_instance(&instance, &mut reused, ShapeOptions::new()).unwrap();
    reused.set_length(0);
    reused.push_str(text);
    reused.guess_segment_properties();
    shape_with_instance(&instance, &mut reused, ShapeOptions::new()).unwrap();

    assert!(reused.allocation_successful());
    assert_eq!(
        ids_and_clusters(reused.glyph_infos()),
        ids_and_clusters(fresh.glyph_infos())
    );
}
