//! Tests for the unified [`Buffer`] type and its conversions to and from the
//! typed [`UnicodeBuffer`] / [`GlyphBuffer`] pair.

use std::fs;
use std::path::PathBuf;

use harfrust::{
    font::{Font, FontInstance},
    shape, Buffer, BufferContentType, Direction, GlyphBuffer, ShapeError, ShapeOptions,
    UnicodeBuffer,
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
    buffer.shape(&instance, ShapeOptions::new()).unwrap();
    assert_eq!(buffer.content_type(), Some(BufferContentType::Glyphs));
    assert!(!buffer.is_empty());
    assert_eq!(buffer.glyph_positions().len(), buffer.len());
    // Running out of room is reported here rather than as a `ShapeError`.
    assert!(buffer.allocation_successful());
}

#[test]
fn shaping_a_shaped_buffer_is_refused() {
    let instance = test_instance();
    let mut buffer = unicode_buffer(TEXT);
    buffer.shape(&instance, ShapeOptions::new()).unwrap();
    let once = ids_and_clusters(buffer.glyph_infos());

    // Shaping glyphs as though they were text would be nonsense, so it is
    // reported rather than quietly ignored.
    assert_eq!(
        buffer.shape(&instance, ShapeOptions::new()),
        Err(ShapeError::AlreadyShaped)
    );
    assert_eq!(
        ids_and_clusters(buffer.glyph_infos()),
        once,
        "a refused call must leave the buffer alone"
    );

    // Relabelling the contents is how you ask for them to be shaped again.
    buffer.set_content_type(Some(BufferContentType::Unicode));
    assert!(buffer.shape(&instance, ShapeOptions::new()).is_ok());
}

#[test]
fn shaping_without_a_direction_is_refused() {
    let instance = test_instance();
    let mut buffer = Buffer::new();
    buffer.push_str(TEXT);
    // No direction, and none guessed.
    assert_eq!(buffer.direction(), Direction::Invalid);
    assert_eq!(
        buffer.shape(&instance, ShapeOptions::new()),
        Err(ShapeError::DirectionUnset)
    );
    assert_eq!(
        buffer.content_type(),
        Some(BufferContentType::Unicode),
        "a refused call must leave the buffer alone"
    );

    buffer.guess_segment_properties();
    assert!(buffer.shape(&instance, ShapeOptions::new()).is_ok());
}

#[test]
fn shaping_with_a_mismatched_plan_is_refused() {
    use harfrust::{script, ShapePlan};

    let instance = test_instance();
    let plan = ShapePlan::new(
        &instance,
        Direction::RightToLeft,
        Some(script::ARABIC),
        None,
        &[],
    );

    let mut buffer = unicode_buffer(TEXT);
    let before = ids_and_clusters(buffer.glyph_infos());
    let err = buffer
        .shape(&instance, ShapeOptions::new().plan(Some(&plan)))
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
    assert_eq!(buffer.content_type(), Some(BufferContentType::Unicode));

    // With the direction agreed, the script is checked next.
    let plan = ShapePlan::new(
        &instance,
        Direction::LeftToRight,
        Some(script::ARABIC),
        None,
        &[],
    );
    let err = buffer
        .shape(&instance, ShapeOptions::new().plan(Some(&plan)))
        .unwrap_err();
    assert_eq!(
        err,
        ShapeError::ScriptMismatch {
            plan: script::ARABIC,
            buffer: script::LATIN,
        }
    );
}

#[test]
fn a_matching_plan_shapes() {
    use harfrust::{script, ShapePlan};

    let instance = test_instance();
    let plan = ShapePlan::new(
        &instance,
        Direction::LeftToRight,
        Some(script::LATIN),
        None,
        &[],
    );

    let mut planned = unicode_buffer(TEXT);
    planned
        .shape(&instance, ShapeOptions::new().plan(Some(&plan)))
        .unwrap();

    let mut direct = unicode_buffer(TEXT);
    direct.shape(&instance, ShapeOptions::new()).unwrap();

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
fn buffer_matches_typed_buffer_shaping() {
    let instance = test_instance();

    let mut typed = UnicodeBuffer::new();
    typed.push_str(TEXT);
    typed.guess_segment_properties();
    let typed = shape(&instance, typed, ShapeOptions::new());

    let mut unified = unicode_buffer(TEXT);
    unified.shape(&instance, ShapeOptions::new()).unwrap();

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
    buffer.shape(&instance, ShapeOptions::new()).unwrap();
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

/// Property-based tests for the buffer's documented behaviour.
///
/// The settings and the items are drawn with `draw_silent` and recorded with
/// `tc.note`, so a failing case prints each of them as one line rather than
/// draw by draw. `.print_as_debug()` marks a `draw` of a type hegel cannot
/// otherwise print.
mod properties {
    use harfrust::{
        script, BufferClusterLevel, BufferContentType, Direction, GlyphId, Language, ShapeError,
        ShapeOptions, ShapePlan,
    };
    use hegel::generators::{self, Generator};

    use super::{ids_and_clusters, test_instance, Buffer, UnicodeBuffer};

    const DIRECTIONS: &[Direction] = &[
        Direction::Invalid,
        Direction::LeftToRight,
        Direction::RightToLeft,
        Direction::TopToBottom,
        Direction::BottomToTop,
    ];

    const CLUSTER_LEVELS: &[BufferClusterLevel] = &[
        BufferClusterLevel::MonotoneGraphemes,
        BufferClusterLevel::MonotoneCharacters,
        BufferClusterLevel::Characters,
        BufferClusterLevel::Graphemes,
    ];

    /// The buffer's settings, all of them generated.
    #[derive(Debug, Clone)]
    struct Settings {
        direction: Direction,
        script: harfrust::Script,
        language: Option<Language>,
        cluster_level: BufferClusterLevel,
        flags: harfrust::BufferFlags,
        invisible_glyph: Option<GlyphId>,
        not_found_variation_selector_glyph: Option<u32>,
    }

    fn draw_settings(tc: &hegel::TestCase) -> Settings {
        let settings = Settings {
            direction: tc.draw_silent(generators::sampled_from(DIRECTIONS.to_vec())),
            script: harfrust::Script::from_iso15924_tag(harfrust::Tag::new(&tc.draw_silent(
                generators::arrays(generators::integers::<u8>().min_value(b'A').max_value(b'z')),
            )))
            .unwrap_or(script::UNKNOWN),
            language: Language::new(
                tc.draw_silent(generators::text().min_size(1).max_size(12).codec("ascii")),
            ),
            cluster_level: tc.draw_silent(generators::sampled_from(CLUSTER_LEVELS.to_vec())),
            flags: harfrust::BufferFlags::from_bits_truncate(
                tc.draw_silent(generators::integers::<u32>()),
            ),
            invisible_glyph: tc.draw_silent(generators::optional(
                generators::integers::<u32>().map(GlyphId::new),
            )),
            not_found_variation_selector_glyph: tc
                .draw_silent(generators::optional(generators::integers::<u32>())),
        };
        tc.note(&format!("{settings:?}"));
        settings
    }

    fn apply(buffer: &mut Buffer, s: &Settings) {
        buffer.set_direction(s.direction);
        buffer.set_script(s.script);
        if let Some(language) = &s.language {
            buffer.set_language(language.clone());
        }
        buffer.set_cluster_level(s.cluster_level);
        buffer.set_flags(s.flags);
        buffer.set_invisible_glyph(s.invisible_glyph);
        buffer.set_not_found_variation_selector_glyph(s.not_found_variation_selector_glyph);
    }

    fn read(buffer: &Buffer) -> Settings {
        Settings {
            direction: buffer.direction(),
            script: buffer.script(),
            language: buffer.language(),
            cluster_level: buffer.cluster_level(),
            flags: buffer.flags(),
            invisible_glyph: buffer.invisible_glyph(),
            not_found_variation_selector_glyph: buffer.not_found_variation_selector_glyph(),
        }
    }

    fn same(a: &Settings, b: &Settings) -> bool {
        a.direction == b.direction
            && a.script == b.script
            && a.language == b.language
            && a.cluster_level == b.cluster_level
            && a.flags.bits() == b.flags.bits()
            && a.invisible_glyph == b.invisible_glyph
            && a.not_found_variation_selector_glyph == b.not_found_variation_selector_glyph
    }

    /// Draws (codepoint, cluster) pairs, with clusters in no particular order
    /// so that cluster grouping is exercised rather than only the monotone
    /// shape `push_str` produces.
    fn draw_items(tc: &hegel::TestCase) -> Vec<(u32, u32)> {
        let items = tc.draw_silent(
            generators::vecs(generators::tuples!(
                generators::integers::<u32>(),
                generators::integers::<u32>().max_value(6),
            ))
            .max_size(24),
        );
        tc.note(&format!("items = {items:?}"));
        items
    }

    fn filled(items: &[(u32, u32)]) -> Buffer {
        let mut buffer = Buffer::new();
        for &(codepoint, cluster) in items {
            buffer.push(codepoint, cluster);
        }
        buffer
    }

    /// Property: `push_str` gives each character its UTF-8 byte offset as its
    /// cluster value, and stores the codepoint itself as the item.
    ///
    /// Both halves are documented on `Buffer::push_str` and
    /// `GlyphInfo::glyph_id`; the oracle is `str::char_indices`.
    #[hegel::test]
    fn push_str_records_codepoints_at_their_byte_offsets(tc: hegel::TestCase) {
        let text: String = tc.draw(generators::text().max_size(64));
        let mut buffer = Buffer::new();
        buffer.push_str(&text);
        assert_eq!(
            ids_and_clusters(buffer.glyph_infos()),
            text.char_indices()
                .map(|(i, c)| (c as u32, i as u32))
                .collect::<Vec<_>>()
        );
    }

    /// Property: every buffer setting survives the trip through
    /// `UnicodeBuffer` and back.
    ///
    /// `round_trip_through_unicode_buffer` asserts this for the direction of
    /// one fixed buffer; the conversions are documented to change nothing but
    /// the content type.
    #[hegel::test]
    fn settings_survive_the_unicode_buffer_round_trip(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let settings = draw_settings(&tc);
        let mut buffer = filled(&items);
        apply(&mut buffer, &settings);

        let typed = UnicodeBuffer::try_from(buffer).expect("unicode content converts");
        let back = Buffer::from(typed);
        assert!(same(&read(&back), &settings), "{:?}", read(&back));
    }

    /// Property: the buffer contents survive the trip through `UnicodeBuffer`
    /// and back.
    ///
    /// Both conversions are documented to touch nothing but the content type,
    /// and `round_trip_through_glyph_buffer` asserts the same for a shaped
    /// buffer.
    #[hegel::test]
    fn contents_survive_the_unicode_buffer_round_trip(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let buffer = filled(&items);
        let before = ids_and_clusters(buffer.glyph_infos());

        let typed = UnicodeBuffer::try_from(buffer).expect("unicode content converts");
        let back = Buffer::from(typed);
        assert_eq!(ids_and_clusters(back.glyph_infos()), before);
    }

    /// Property: `reverse` is its own inverse.
    ///
    /// `reverse_and_reverse_clusters` asserts the forward half for one fixed
    /// buffer.
    #[hegel::test]
    fn reverse_is_an_involution(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let mut buffer = filled(&items);
        let before = ids_and_clusters(buffer.glyph_infos());
        buffer.reverse();
        buffer.reverse();
        assert_eq!(ids_and_clusters(buffer.glyph_infos()), before);
    }

    /// Property: `reverse_clusters` reverses the order of the runs of equal
    /// cluster while leaving the items within each run in their original
    /// order.
    ///
    /// That is the documented behaviour ("keeping the items within each
    /// cluster in their original order"); `reverse_and_reverse_clusters`
    /// asserts one instance of it.
    #[hegel::test]
    fn reverse_clusters_reverses_runs_and_not_their_contents(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let mut buffer = filled(&items);
        buffer.reverse_clusters();

        let mut expected: Vec<Vec<(u32, u32)>> = Vec::new();
        for item in &items {
            match expected.last_mut() {
                Some(run) if run[0].1 == item.1 => run.push(*item),
                _ => expected.push(vec![*item]),
            }
        }
        let expected: Vec<(u32, u32)> = expected.into_iter().rev().flatten().collect();
        assert_eq!(ids_and_clusters(buffer.glyph_infos()), expected);
    }

    /// Property: `reset_clusters` replaces every cluster value with the item's
    /// index, leaving the items themselves alone.
    ///
    /// "Resets the cluster value of each item to its index" is what the method
    /// documents.
    #[hegel::test]
    fn reset_clusters_numbers_items_from_zero(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let mut buffer = filled(&items);
        buffer.reset_clusters();
        assert_eq!(
            ids_and_clusters(buffer.glyph_infos()),
            items
                .iter()
                .enumerate()
                .map(|(i, (codepoint, _))| (*codepoint, i as u32))
                .collect::<Vec<_>>()
        );
    }

    /// Property: `set_length` keeps the first `len` items when shrinking and
    /// zero-fills when growing.
    ///
    /// "Growing the buffer fills the new items with zeros" is documented on
    /// `Buffer::set_length`.
    #[hegel::test]
    fn set_length_truncates_or_zero_fills(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let len = tc.draw(generators::integers::<usize>().max_value(64));
        let mut buffer = filled(&items);

        let mut expected = ids_and_clusters(buffer.glyph_infos());
        expected.truncate(len);
        expected.resize(len, (0, 0));

        assert!(buffer.set_length(len));
        assert_eq!(ids_and_clusters(buffer.glyph_infos()), expected);
    }

    /// Property: `clear` empties the buffer and drops its segment properties.
    ///
    /// `clear` matches HarfBuzz's `hb_buffer_clear_contents`;
    /// `clear_resets_content_type` covers one instance.
    #[hegel::test]
    fn clear_empties_the_buffer_and_drops_the_segment_properties(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let settings = draw_settings(&tc);
        let mut buffer = filled(&items);
        apply(&mut buffer, &settings);
        buffer.clear();

        assert!(buffer.is_empty());
        assert_eq!(buffer.content_type(), None);
        assert_eq!(buffer.direction(), Direction::Invalid);
        assert_eq!(buffer.script(), script::UNKNOWN);
        assert_eq!(buffer.language(), None);
        assert_eq!(
            buffer.cluster_level(),
            BufferClusterLevel::MonotoneGraphemes
        );
    }

    /// Property: `clear` keeps the flags and the invisible glyph.
    ///
    /// Like HarfBuzz's `hb_buffer_clear_contents`, `clear` leaves the buffer's
    /// configuration in place; `reset` is the call that clears that too.
    #[hegel::test]
    fn clear_keeps_the_flags_and_the_invisible_glyph(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let settings = draw_settings(&tc);
        let mut buffer = filled(&items);
        apply(&mut buffer, &settings);
        buffer.clear();

        assert_eq!(buffer.flags().bits(), settings.flags.bits());
        assert_eq!(buffer.invisible_glyph(), settings.invisible_glyph);
    }

    /// Property: `reset` restores a buffer to the state of a fresh one.
    ///
    /// `reset` is documented as resetting "to its default state";
    /// `reset_restores_defaults` checks four of those defaults against
    /// literals rather than against a fresh buffer.
    #[hegel::test]
    fn reset_leaves_a_buffer_indistinguishable_from_a_new_one(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let settings = draw_settings(&tc);
        let mut buffer = filled(&items);
        apply(&mut buffer, &settings);
        buffer.reset();

        let fresh = Buffer::new();
        assert!(same(&read(&buffer), &read(&fresh)), "{:?}", read(&buffer));
        assert_eq!(buffer.len(), fresh.len());
        assert_eq!(buffer.content_type(), fresh.content_type());
    }

    /// Property: `set_content_type` relabels the buffer without touching its
    /// contents.
    ///
    /// "This only relabels the buffer; it never clears it" is documented on
    /// `Buffer::set_content_type`.
    #[hegel::test]
    fn set_content_type_only_relabels(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let content_type = tc.draw(generators::optional(
            generators::booleans()
                .map(|glyphs| {
                    if glyphs {
                        BufferContentType::Glyphs
                    } else {
                        BufferContentType::Unicode
                    }
                })
                .print_as_debug(),
        ));
        let mut buffer = filled(&items);
        let before = ids_and_clusters(buffer.glyph_infos());
        buffer.set_content_type(content_type);
        assert_eq!(ids_and_clusters(buffer.glyph_infos()), before);
        assert_eq!(buffer.content_type(), content_type);
    }

    /// Property: `guess_segment_properties` only fills in what is unset.
    ///
    /// That is what the method documents ("Only properties that are still
    /// unset are filled in"), so a second call cannot change anything and a
    /// preset direction or script has to survive.
    #[hegel::test]
    fn guess_segment_properties_is_idempotent(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let mut buffer = filled(&items);
        if tc.draw(generators::booleans()) {
            buffer.set_direction(
                tc.draw(generators::sampled_from(DIRECTIONS.to_vec()).print_as_debug()),
            );
        }
        if tc.draw(generators::booleans()) {
            buffer.set_script(script::ARABIC);
        }

        buffer.guess_segment_properties();
        let (direction, script, language) =
            (buffer.direction(), buffer.script(), buffer.language());
        buffer.guess_segment_properties();
        assert_eq!(
            (buffer.direction(), buffer.script(), buffer.language()),
            (direction, script, language)
        );
    }

    /// Property: `guess_segment_properties` always resolves a direction.
    ///
    /// Shaping refuses a buffer with `Direction::Invalid`
    /// (`ShapeError::DirectionUnset`), and `guess_segment_properties` is what
    /// its documentation points callers at, so it has to produce one.
    #[hegel::test]
    fn guess_segment_properties_always_resolves_a_direction(tc: hegel::TestCase) {
        let items = draw_items(&tc);
        let mut buffer = filled(&items);
        buffer.guess_segment_properties();
        assert_ne!(buffer.direction(), Direction::Invalid);
    }

    /// Property: a refused `shape` call leaves the buffer exactly as it
    /// arrived.
    ///
    /// `Buffer::shape` documents every `ShapeError` as "a misuse of the API,
    /// caught before anything is touched, so a failure leaves the buffer
    /// exactly as it arrived". Each of the refusals is provoked here in turn.
    #[hegel::test]
    fn a_refused_shape_call_changes_nothing(tc: hegel::TestCase) {
        let instance = test_instance();
        let items = draw_items(&tc);
        let mut buffer = filled(&items);

        // 0: no direction, 1: already shaped, 2: plan built for another
        // direction, 3: plan built for another script.
        let refusal = tc.draw(generators::integers::<u8>().max_value(3));
        let plan = match refusal {
            0 => {
                buffer.set_direction(Direction::Invalid);
                None
            }
            1 => {
                buffer.set_direction(Direction::LeftToRight);
                buffer.set_content_type(Some(BufferContentType::Glyphs));
                None
            }
            2 => {
                buffer.set_direction(Direction::LeftToRight);
                Some(ShapePlan::new(
                    &instance,
                    Direction::RightToLeft,
                    Some(script::LATIN),
                    None,
                    &[],
                ))
            }
            _ => {
                buffer.set_direction(Direction::LeftToRight);
                buffer.set_script(script::LATIN);
                Some(ShapePlan::new(
                    &instance,
                    Direction::LeftToRight,
                    Some(script::ARABIC),
                    None,
                    &[],
                ))
            }
        };

        let before = ids_and_clusters(buffer.glyph_infos());
        let content_type = buffer.content_type();
        let err = buffer
            .shape(&instance, ShapeOptions::new().plan(plan.as_ref()))
            .expect_err("this call is a misuse and must be refused");
        assert!(
            matches!(
                err,
                ShapeError::DirectionUnset
                    | ShapeError::AlreadyShaped
                    | ShapeError::DirectionMismatch { .. }
                    | ShapeError::ScriptMismatch { .. }
            ),
            "{err:?}"
        );
        assert_eq!(ids_and_clusters(buffer.glyph_infos()), before);
        assert_eq!(buffer.content_type(), content_type);
    }
}
