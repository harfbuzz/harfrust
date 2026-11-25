//! Tests for the optional WebAssembly shaper.
#![cfg(feature = "wasm")]

use std::fmt::Write as _;

use harfrust::{
    shape, Buffer, Direction, Feature, Font, ShapeError, ShapeOptions, Shaper, ShaperFont, Tag,
};
use read_fonts::FontRef;

const ROBOTO: &[u8] = include_bytes!("../benches/fonts/Roboto-Regular.ttf");

/// Rebuilds an SFNT with an extra table, which is how the tests install a
/// guest shaper into a normal font.
fn add_table(base: &[u8], tag: [u8; 4], table: &[u8]) -> Vec<u8> {
    let font = FontRef::new(base).unwrap();
    let mut tables: Vec<([u8; 4], Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .map(|record| {
            let tag = record.tag().to_be_bytes();
            let data = font.table_data(record.tag()).unwrap();
            (tag, data.as_bytes().to_vec())
        })
        .collect();
    tables.push((tag, table.to_vec()));
    tables.sort_by_key(|table| table.0);

    let num_tables = u16::try_from(tables.len()).unwrap();
    let mut entry_selector = 0u16;
    while (1u16 << (entry_selector + 1)) <= num_tables {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables * 16 - search_range;

    let mut output = Vec::new();
    output.extend_from_slice(&base[..4]);
    output.extend_from_slice(&num_tables.to_be_bytes());
    output.extend_from_slice(&search_range.to_be_bytes());
    output.extend_from_slice(&entry_selector.to_be_bytes());
    output.extend_from_slice(&range_shift.to_be_bytes());

    let mut offset = 12 + 16 * tables.len() as u32;
    let records_start = output.len();
    output.resize(records_start + 16 * tables.len(), 0);
    for (index, (tag, data)) in tables.iter().enumerate() {
        let record = records_start + index * 16;
        output[record..record + 4].copy_from_slice(tag);
        output[record + 4..record + 8].copy_from_slice(&checksum(data).to_be_bytes());
        output[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
        output[record + 12..record + 16]
            .copy_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        offset += u32::try_from(data.len()).unwrap().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        output.extend_from_slice(data);
        while output.len() % 4 != 0 {
            output.push(0);
        }
    }
    output
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut bytes = [0u8; 4];
        bytes[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(bytes));
    }
    sum
}

/// Shapes with the given shapers. A font's module is asked for explicitly,
/// since the default list is OpenType only.
fn shape_text_with(
    font_data: Vec<u8>,
    text: &str,
    features: &[Feature],
    shapers: &[Shaper],
) -> Result<Buffer, ShapeError> {
    let font = Font::new(font_data, 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    shape(
        &shaper,
        &mut buffer,
        ShapeOptions::new().features(features).shapers(shapers),
    )?;
    Ok(buffer)
}

fn shape_text(font_data: Vec<u8>, text: &str, features: &[Feature]) -> Buffer {
    shape_text_with(font_data, text, features, &[Shaper::Wasm, Shaper::Ot]).unwrap()
}

/// Maps every input character to its nominal glyph and default advance.
const MAP_GUEST: &str = r#"
(module
  (import "env" "buffer_copy_contents" (func $copy (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (import "env" "font_get_glyph" (func $glyph (param i32 i32 i32) (result i32)))
  (import "env" "font_get_glyph_h_advance" (func $advance (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $i i32)
    (local $n i32)
    (local $info i32)
    (local $pos i32)
    (local $glyph i32)
    ;; Ask the host for arrays sized to the actual buffer.
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 0))
    (i32.store (i32.const 8) (i32.const 0))
    (if (i32.eqz (call $copy (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (local.set $n (i32.load (i32.const 0)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $info (i32.add (i32.load (i32.const 4)) (i32.mul (local.get $i) (i32.const 20))))
        (local.set $glyph (call $glyph (local.get $font) (i32.load (local.get $info)) (i32.const 0)))
        (i32.store (local.get $info) (local.get $glyph))
        (local.set $pos (i32.add (i32.load (i32.const 8)) (i32.mul (local.get $i) (i32.const 20))))
        (i32.store (local.get $pos) (call $advance (local.get $font) (local.get $glyph)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (if (i32.eqz (call $set (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (i32.const 1)))
"#;

#[test]
fn wasm_shaper_maps_codepoints() {
    let wasm = wat::parse_str(MAP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let text = "abc";
    let buffer = shape_text(data, text, &[]);

    let plain = Font::new(ROBOTO.to_vec(), 0).unwrap();
    let shaper = ShaperFont::new(&plain);
    assert_eq!(buffer.len(), text.len());
    for (index, character) in text.chars().enumerate() {
        let glyph = shaper.nominal_glyph(character as u32).unwrap();
        assert_eq!(buffer.glyph_infos()[index].glyph_id, glyph.to_u32());
        assert_eq!(
            buffer.glyph_positions()[index].x_advance,
            shaper.glyph_h_advance(glyph)
        );
    }
}

/// Runs the normal OpenType shaper from inside the guest, then nudges every
/// glyph's x offset.
const OT_THEN_OFFSET_GUEST: &str = r#"
(module
  (import "env" "shape_with" (func $shape_with (param i32 i32 i32 i32 i32) (result i32)))
  (import "env" "buffer_copy_contents" (func $copy (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 1024) "ot\00")
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $i i32)
    (local $n i32)
    (local $pos i32)
    (if (i32.eqz (call $shape_with (local.get $font) (local.get $buf)
                                   (i32.const 0) (i32.const 0) (i32.const 1024)))
      (then (return (i32.const 0))))
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 0))
    (i32.store (i32.const 8) (i32.const 0))
    (if (i32.eqz (call $copy (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (local.set $n (i32.load (i32.const 0)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $pos (i32.add (i32.load (i32.const 8)) (i32.mul (local.get $i) (i32.const 20))))
        (i32.store (i32.add (local.get $pos) (i32.const 8))
                   (i32.add (i32.load (i32.add (local.get $pos) (i32.const 8)))
                            (i32.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (if (i32.eqz (call $set (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (i32.const 1)))
"#;

#[test]
fn wasm_shaper_can_delegate_to_ot() {
    let wasm = wat::parse_str(OT_THEN_OFFSET_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = shape_text(data, "AV", &[]);
    let expected = shape_text(ROBOTO.to_vec(), "AV", &[]);

    assert_eq!(buffer.len(), expected.len());
    for index in 0..buffer.len() {
        let actual_info = &buffer.glyph_infos()[index];
        let expected_info = &expected.glyph_infos()[index];
        assert_eq!(actual_info.glyph_id, expected_info.glyph_id);
        assert_eq!(actual_info.cluster, expected_info.cluster);

        let actual_pos = &buffer.glyph_positions()[index];
        let expected_pos = &expected.glyph_positions()[index];
        assert_eq!(actual_pos.x_advance, expected_pos.x_advance);
        assert_eq!(actual_pos.y_advance, expected_pos.y_advance);
        assert_eq!(actual_pos.y_offset, expected_pos.y_offset);
        assert_eq!(actual_pos.x_offset, expected_pos.x_offset + 7);
    }
}

/// Exercises the face, table, name, extents, variation and outline queries.
const QUERY_GUEST: &str = r#"
(module
  (import "env" "buffer_copy_contents" (func $copy (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (import "env" "font_get_face" (func $face (param i32) (result i32)))
  (import "env" "face_get_upem" (func $upem (param i32) (result i32)))
  (import "env" "face_copy_table" (func $table (param i32 i32 i32) (result i32)))
  (import "env" "font_get_glyph" (func $glyph (param i32 i32 i32) (result i32)))
  (import "env" "font_glyph_to_string" (func $name (param i32 i32 i32 i32)))
  (import "env" "font_get_glyph_extents" (func $extents (param i32 i32 i32) (result i32)))
  (import "env" "font_copy_coords" (func $coords (param i32 i32) (result i32)))
  (import "env" "coords_free" (func $coords_free (param i32)))
  (import "env" "font_copy_glyph_outline" (func $outline (param i32 i32 i32) (result i32)))
  (import "env" "font_get_glyph_h_advance" (func $advance (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $face i32)
    (local $g i32)
    (local $i i32)
    (local $n i32)
    (local $info i32)
    (local $pos i32)
    (local.set $face (call $face (local.get $font)))
    (if (i32.eqz (local.get $face)) (then (return (i32.const 0))))
    (if (i32.eqz (call $upem (local.get $face))) (then (return (i32.const 0))))
    ;; blob_t at 2000; the host allocates the "head" table.
    (i32.store (i32.const 2000) (i32.const 0))
    (i32.store (i32.const 2004) (i32.const 0))
    (if (i32.eqz (call $table (local.get $face) (i32.const 1751474532) (i32.const 2000)))
      (then (return (i32.const 0))))
    (if (i32.lt_u (i32.load (i32.const 2000)) (i32.const 50))
      (then (return (i32.const 0))))
    (local.set $g (call $glyph (local.get $font) (i32.const 97) (i32.const 0)))
    (if (i32.eqz (local.get $g)) (then (return (i32.const 0))))
    (call $name (local.get $font) (local.get $g) (i32.const 2100) (i32.const 32))
    (if (i32.eqz (i32.load8_u (i32.const 2100))) (then (return (i32.const 0))))
    (if (i32.eqz (call $extents (local.get $font) (local.get $g) (i32.const 2060)))
      (then (return (i32.const 0))))
    ;; coords_t at 2020; a static font has no coordinates.
    (i32.store (i32.const 2020) (i32.const 0))
    (i32.store (i32.const 2024) (i32.const 0))
    (if (i32.eqz (call $coords (local.get $font) (i32.const 2020)))
      (then (return (i32.const 0))))
    ;; coords_free empties the structure.
    (i32.store (i32.const 2020) (i32.const 3))
    (i32.store (i32.const 2024) (i32.const 2200))
    (call $coords_free (i32.const 2020))
    (if (i32.load (i32.const 2020)) (then (return (i32.const 0))))
    (if (i32.load (i32.const 2024)) (then (return (i32.const 0))))
    ;; glyph_outline_t at 2040.
    (i32.store (i32.const 2040) (i32.const 0))
    (i32.store (i32.const 2044) (i32.const 0))
    (i32.store (i32.const 2048) (i32.const 0))
    (i32.store (i32.const 2052) (i32.const 0))
    (if (i32.eqz (call $outline (local.get $font) (local.get $g) (i32.const 2040)))
      (then (return (i32.const 0))))
    (if (i32.eqz (i32.load (i32.const 2040))) (then (return (i32.const 0))))
    (if (i32.eqz (i32.load (i32.const 2048))) (then (return (i32.const 0))))
    ;; Map the buffer so the result is a valid glyph run.
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 0))
    (i32.store (i32.const 8) (i32.const 0))
    (if (i32.eqz (call $copy (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (local.set $n (i32.load (i32.const 0)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $info (i32.add (i32.load (i32.const 4)) (i32.mul (local.get $i) (i32.const 20))))
        (local.set $g (call $glyph (local.get $font) (i32.load (local.get $info)) (i32.const 0)))
        (i32.store (local.get $info) (local.get $g))
        (local.set $pos (i32.add (i32.load (i32.const 8)) (i32.mul (local.get $i) (i32.const 20))))
        (i32.store (local.get $pos) (call $advance (local.get $font) (local.get $g)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (if (i32.eqz (call $set (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (i32.const 1)))
"#;

#[test]
fn wasm_shaper_host_queries() {
    let wasm = wat::parse_str(QUERY_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = shape_text(data, "abc", &[]);
    assert_eq!(buffer.len(), 3);
    assert!(buffer.glyph_infos().iter().all(|info| info.glyph_id != 0));
}

/// A real font small enough to sit in a guest's data segment.
const SMALL_FONT: &[u8] =
    include_bytes!("fonts/in-house/1c2c3fc37b2d4c3cb2ef726c6cdaaabd4b7f3eb9.ttf");

/// Creates a font and a face of its own and destroys them again, then maps
/// the buffer with the font it was given.
fn destroy_guest() -> String {
    let data = SMALL_FONT.iter().fold(String::new(), |mut data, byte| {
        let _ = write!(data, "\\{byte:02x}");
        data
    });
    let length = SMALL_FONT.len();
    format!(
        r#"
(module
  (import "env" "buffer_copy_contents" (func $copy (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (import "env" "face_create" (func $face_create (param i32 i32) (result i32)))
  (import "env" "face_destroy" (func $face_destroy (param i32)))
  (import "env" "face_get_upem" (func $upem (param i32) (result i32)))
  (import "env" "font_create" (func $font_create (param i32) (result i32)))
  (import "env" "font_destroy" (func $font_destroy (param i32)))
  (import "env" "font_get_face" (func $get_face (param i32) (result i32)))
  (import "env" "font_get_glyph" (func $glyph (param i32 i32 i32) (result i32)))
  (import "env" "font_get_glyph_h_advance" (func $advance (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 4096) "{data}")
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $created i32)
    (local $face i32)
    (local $again i32)
    (local $i i32)
    (local $n i32)
    (local $info i32)
    (local $pos i32)
    (local $g i32)
    ;; A font on the face we were given, destroyed before we return.
    (local.set $created (call $font_create (call $get_face (local.get $font))))
    (if (i32.eqz (local.get $created)) (then (return (i32.const 0))))
    (if (i32.eqz (call $glyph (local.get $created) (i32.const 97) (i32.const 0)))
      (then (return (i32.const 0))))
    (call $font_destroy (local.get $created))
    ;; Its token names nothing now.
    (if (call $glyph (local.get $created) (i32.const 97) (i32.const 0))
      (then (return (i32.const 0))))
    ;; A face of our own, from the font at 4096, and a font on it.
    (i32.store (i32.const 3000) (i32.const {length}))
    (i32.store (i32.const 3004) (i32.const 4096))
    (local.set $face (call $face_create (i32.const 3000) (i32.const 0)))
    (if (i32.eqz (local.get $face)) (then (return (i32.const 0))))
    (if (i32.eqz (call $upem (local.get $face))) (then (return (i32.const 0))))
    (local.set $created (call $font_create (local.get $face)))
    (if (i32.eqz (local.get $created)) (then (return (i32.const 0))))
    ;; The face can go first. Its token then names nothing...
    (call $face_destroy (local.get $face))
    (if (call $upem (local.get $face)) (then (return (i32.const 0))))
    ;; ...and the font gives a new one for the face it still has.
    (local.set $again (call $get_face (local.get $created)))
    (if (i32.eqz (local.get $again)) (then (return (i32.const 0))))
    (if (i32.eq (local.get $again) (local.get $face)) (then (return (i32.const 0))))
    (if (i32.ne (local.get $again) (call $get_face (local.get $created)))
      (then (return (i32.const 0))))
    (if (i32.eqz (call $upem (local.get $again))) (then (return (i32.const 0))))
    (call $font_destroy (local.get $created))
    (call $face_destroy (local.get $again))
    (if (call $upem (local.get $again)) (then (return (i32.const 0))))
    ;; What we were given is not ours to destroy.
    (call $font_destroy (local.get $font))
    (call $face_destroy (call $get_face (local.get $font)))
    (if (i32.eqz (call $upem (call $get_face (local.get $font))))
      (then (return (i32.const 0))))
    ;; Map the buffer with the font we were given.
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 0))
    (i32.store (i32.const 8) (i32.const 0))
    (if (i32.eqz (call $copy (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (local.set $n (i32.load (i32.const 0)))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $info (i32.add (i32.load (i32.const 4)) (i32.mul (local.get $i) (i32.const 20))))
        (local.set $g (call $glyph (local.get $font) (i32.load (local.get $info)) (i32.const 0)))
        (i32.store (local.get $info) (local.get $g))
        (local.set $pos (i32.add (i32.load (i32.const 8)) (i32.mul (local.get $i) (i32.const 20))))
        (i32.store (local.get $pos) (call $advance (local.get $font) (local.get $g)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (if (i32.eqz (call $set (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (i32.const 1)))
"#
    )
}

#[test]
fn wasm_shaper_destroys_faces_and_fonts() {
    let wasm = wat::parse_str(destroy_guest()).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let text = "abc";
    // The guest returns failure, which `shape_text` unwraps, unless every
    // step went as its comments say.
    let buffer = shape_text(data, text, &[]);

    let plain = Font::new(ROBOTO.to_vec(), 0).unwrap();
    let shaper = ShaperFont::new(&plain);
    assert_eq!(buffer.len(), text.len());
    for (index, character) in text.chars().enumerate() {
        let glyph = shaper.nominal_glyph(character as u32).unwrap();
        assert_eq!(buffer.glyph_infos()[index].glyph_id, glyph.to_u32());
    }
}
fn try_shape_text(font_data: Vec<u8>, text: &str) -> Result<Buffer, ShapeError> {
    shape_text_with(font_data, text, &[], &[Shaper::Wasm, Shaper::Ot])
}

/// Never returns.
const LOOP_GUEST: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
    (loop $forever (br $forever))
    (i32.const 1)))
"#;

/// Never gets as far as `shape`.
const START_LOOP_GUEST: &str = r#"
(module
  (memory (export "memory") 1)
  (func $spin (loop $forever (br $forever)))
  (start $spin)
  (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
    (i32.const 1)))
"#;

#[test]
fn wasm_shaper_stops_a_module_that_does_not() {
    let wasm = wat::parse_str(LOOP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(matches!(
        try_shape_text(data, "abc"),
        Err(ShapeError::WasmShapingFailed)
    ));

    // A module that cannot be instantiated leaves the font to OpenType.
    let wasm = wat::parse_str(START_LOOP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = try_shape_text(data, "abc").unwrap();
    let plain = shape_text(ROBOTO.to_vec(), "abc", &[]);
    assert_eq!(buffer.len(), plain.len());
    for (ours, theirs) in buffer.glyph_infos().iter().zip(plain.glyph_infos()) {
        assert_eq!(ours.glyph_id, theirs.glyph_id);
    }
}

/// Takes memory a mebibyte at a time until it is refused, and reports
/// success if that happened within the first 64.
const GROW_GUEST: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
    (local $count i32)
    (block $done
      (loop $loop
        (br_if $done (i32.eq (memory.grow (i32.const 16)) (i32.const -1)))
        (local.set $count (i32.add (local.get $count) (i32.const 1)))
        (br_if $loop (i32.lt_u (local.get $count) (i32.const 4096)))))
    (i32.and (i32.gt_u (local.get $count) (i32.const 0))
             (i32.lt_u (local.get $count) (i32.const 64)))))
"#;

#[test]
fn wasm_shaper_limits_memory() {
    let wasm = wat::parse_str(GROW_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(try_shape_text(data, "abc").is_ok());
}

/// Few instructions of its own, and a lot of work for the host: copies the
/// `glyf` table a hundred thousand times.
const HOST_WORK_GUEST: &str = r#"
(module
  (import "env" "face_copy_table" (func $copy (param i32 i32 i32) (result i32)))
  (memory (export "memory") 16)
  (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
    (local $i i32)
    (loop $loop
      ;; A blob of a megabyte at address 1024.
      (i32.store (i32.const 0) (i32.const 1000000))
      (i32.store (i32.const 4) (i32.const 1024))
      (drop (call $copy (i32.const 1) (i32.const 0x676c7966) (i32.const 0)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $loop (i32.lt_u (local.get $i) (i32.const 100000))))
    (i32.const 1)))
"#;

#[test]
fn wasm_shaper_charges_for_host_work() {
    let wasm = wat::parse_str(HOST_WORK_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(matches!(
        try_shape_text(data, "abc"),
        Err(ShapeError::WasmShapingFailed)
    ));
}

/// Creates fonts until it is refused, and reports success if that happened
/// within the first 300.
const CREATE_GUEST: &str = r#"
(module
  (import "env" "font_create" (func $font_create (param i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
    (local $i i32)
    (local $count i32)
    (loop $loop
      (if (call $font_create (i32.const 1))
        (then (local.set $count (i32.add (local.get $count) (i32.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $loop (i32.lt_u (local.get $i) (i32.const 1000))))
    (i32.and (i32.gt_u (local.get $count) (i32.const 0))
             (i32.lt_u (local.get $count) (i32.const 300)))))
"#;

#[test]
fn wasm_shaper_limits_created_objects() {
    let wasm = wat::parse_str(CREATE_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(try_shape_text(data, "abc").is_ok());
}

fn glyph_ids(buffer: &Buffer) -> Vec<u32> {
    buffer
        .glyph_infos()
        .iter()
        .map(|info| info.glyph_id)
        .collect()
}

#[test]
fn wasm_shaper_is_opt_in() {
    // The default shaper list does not name the module, so a font whose
    // module never returns still shapes, with OpenType.
    let wasm = wat::parse_str(LOOP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let font = Font::new(data, 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    shape(&shaper, &mut buffer, ShapeOptions::new()).unwrap();

    assert_eq!(
        glyph_ids(&buffer),
        glyph_ids(&shape_text(ROBOTO.to_vec(), "abc", &[]))
    );
}

#[test]
fn wasm_shaper_falls_through_to_the_next_shaper() {
    // Roboto has no `Wasm` table, so the WebAssembly shaper passes it over
    // and the OpenType shaper after it in the list shapes the font.
    let with_wasm =
        shape_text_with(ROBOTO.to_vec(), "abc", &[], &[Shaper::Wasm, Shaper::Ot]).unwrap();
    let ot_only = shape_text_with(ROBOTO.to_vec(), "abc", &[], &[Shaper::Ot]).unwrap();
    assert_eq!(glyph_ids(&with_wasm), glyph_ids(&ot_only));
}

#[test]
fn wasm_shaper_alone_needs_a_module() {
    assert!(matches!(
        shape_text_with(ROBOTO.to_vec(), "abc", &[], &[Shaper::Wasm]),
        Err(ShapeError::ShaperUnavailable)
    ));
}

#[test]
fn empty_shaper_list_is_an_error() {
    assert!(matches!(
        shape_text_with(ROBOTO.to_vec(), "abc", &[], &[]),
        Err(ShapeError::ShaperUnavailable)
    ));
}

#[test]
fn a_plan_shapes_with_the_shaper_it_was_built_for() {
    let wasm = wat::parse_str(LOOP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let font = Font::new(data, 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let plan = harfrust::ShapePlan::new(
        &font,
        Direction::LeftToRight,
        Some(harfrust::Script::LATIN),
        None,
        &[],
    );
    assert_eq!(plan.shaper(), Shaper::Ot);

    // The plan's shaper wins over the requested list, which never runs: the
    // module would not return if it did.
    let mut buffer = Buffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    shape(
        &shaper,
        &mut buffer,
        ShapeOptions::new()
            .plan(Some(&plan))
            .shapers(&[Shaper::Wasm]),
    )
    .unwrap();
    assert_eq!(
        glyph_ids(&buffer),
        glyph_ids(&shape_text(ROBOTO.to_vec(), "abc", &[]))
    );
}

/// Gives the buffer a length of 70000 items, which a single character does
/// not allow, and reports whatever `buffer_set_contents` said. A buffer with
/// the default limit would accept it.
const OVERSIZED_GUEST: &str = r#"
(module
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 24)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (i32.store (i32.const 0) (i32.const 70000))
    (i32.store (i32.const 4) (i32.const 1024))
    (i32.store (i32.const 8) (i32.const 1024))
    (call $set (local.get $buf) (i32.const 0))))
"#;

#[test]
fn wasm_shaper_bounds_the_buffer_length() {
    let wasm = wat::parse_str(OVERSIZED_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(matches!(
        try_shape_text(data, "a"),
        Err(ShapeError::WasmShapingFailed)
    ));
}

/// Delegates to OpenType, then tries to grow the buffer past what the call
/// allows. Reports success only if the host refused the new length.
const SHAPE_WITH_THEN_GROW_GUEST: &str = r#"
(module
  (import "env" "shape_with" (func $shape_with (param i32 i32 i32 i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 24)
  (data (i32.const 16) "ot")
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (if (i32.eqz (call $shape_with (local.get $font) (local.get $buf)
                                   (i32.const 0) (i32.const 0) (i32.const 16)))
      (then (return (i32.const 0))))
    (i32.store (i32.const 0) (i32.const 70000))
    (i32.store (i32.const 4) (i32.const 1024))
    (i32.store (i32.const 8) (i32.const 1024))
    (if (result i32) (i32.eqz (call $set (local.get $buf) (i32.const 0)))
      (then (i32.const 1))
      (else (i32.const 0)))))
"#;

#[test]
fn wasm_shaper_keeps_the_length_bound_after_shape_with() {
    let wasm = wat::parse_str(SHAPE_WITH_THEN_GROW_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    assert!(try_shape_text(data, "abc").is_ok());
}

// ---------------------------------------------------------------------------
// Fonts from HarfBuzz's test suite (harfbuzz/test/api/fonts)
//
// Their modules are built from HarfBuzz's C sources against
// `hb-wasm-api.h`; the licences are in fonts/wasm.
// ---------------------------------------------------------------------------

/// The module imports a function nothing provides, as modules made with
/// wasm-bindgen do, and never calls it.
const WASM_IMPORT: &[u8] = include_bytes!("fonts/wasm/wasm-import.ttf");
/// Answers with a few values, one per glyph, chosen by the first character.
const WASM_API: &[u8] = include_bytes!("fonts/wasm/wasm-api.ttf");
/// The same module in a variable font, for the font's coordinates.
const WASM_COORDS: &[u8] = include_bytes!("fonts/wasm/wasm-coords.ttf");

#[test]
fn wasm_shaper_runs_a_module_with_unused_imports() {
    // The module leaves the buffer as it found it, so the codepoints come
    // back as glyph ids. That it ran at all is the point: it imports a
    // function the host does not define, and OpenType would have mapped the
    // text to real glyphs.
    let buffer = shape_text(WASM_IMPORT.to_vec(), "abc", &[]);
    assert_eq!(glyph_ids(&buffer), vec![97, 98, 99]);
}

#[test]
fn wasm_shaper_treats_a_zero_result_as_failure() {
    // Case 'l' of the module returns false from `shape`, without trapping.
    assert!(matches!(
        shape_text_with(WASM_API.to_vec(), "l", &[], &[Shaper::Wasm]),
        Err(ShapeError::WasmShapingFailed)
    ));
}

#[test]
fn wasm_shaper_reports_how_much_was_copied() {
    // Case 'a' hands the host arrays larger than what goes into them and
    // answers with the lengths the host reported: the buffer's, the maxp
    // table's, the coordinates (this font has none), the outline's points and
    // contours, and how many of 20000 outline copies failed.
    let maxp = FontRef::new(WASM_API)
        .unwrap()
        .table_data(Tag::new(b"maxp"))
        .unwrap()
        .as_bytes()
        .len();
    let buffer = shape_text(WASM_API.to_vec(), "a", &[]);
    let values = glyph_ids(&buffer);
    assert_eq!(values[0], 1);
    assert_eq!(values[1], u32::try_from(maxp).unwrap());
    assert_eq!(values[2], 0);
    // Neither count is the 256 and 16 the module offered.
    assert!(values[3] > 0 && values[3] < 256);
    assert_eq!(values[4], 2);
    assert_eq!(values[5], 0);
}

#[test]
fn wasm_shaper_copies_and_frees_font_coords() {
    // Case 'm' copies the font's coordinates into an array it asks the host
    // for, frees it, and does that 200000 times, counting what fails. It
    // answers with the failures and the number of coordinates.
    let font = Font::new(WASM_COORDS.to_vec(), 0).unwrap();
    let font = font
        .instance_builder()
        .variations([(Tag::new(b"wght"), 0.5), (Tag::new(b"wdth"), 0.75)])
        .build();
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    buffer.push_str("m");
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    shape(
        &shaper,
        &mut buffer,
        ShapeOptions::new().shapers(&[Shaper::Wasm]),
    )
    .unwrap();
    assert_eq!(glyph_ids(&buffer), vec![0, 2]);
}

#[test]
fn wasm_shaper_is_not_called_for_an_empty_buffer() {
    // The module would never return if it ran.
    let wasm = wat::parse_str(LOOP_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = shape_text(data, "", &[]);
    assert!(glyph_ids(&buffer).is_empty());
}

// ---------------------------------------------------------------------------
// Host API calls the hand-written modules above do not reach
// ---------------------------------------------------------------------------

/// Answers with the feature count and the first feature's fields, one value
/// per glyph.
const FEATURES_GUEST: &str = r#"
(module
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (i32.store (i32.const 0) (i32.const 5))
    (i32.store (i32.const 4) (i32.const 1024))
    (i32.store (i32.const 8) (i32.const 2048))
    (i32.store (i32.const 1024) (local.get $count))
    (i32.store (i32.const 1044) (i32.load (local.get $features)))
    (i32.store (i32.const 1064) (i32.load (i32.add (local.get $features) (i32.const 4))))
    (i32.store (i32.const 1084) (i32.load (i32.add (local.get $features) (i32.const 8))))
    (i32.store (i32.const 1104) (i32.load (i32.add (local.get $features) (i32.const 12))))
    (call $set (local.get $buf) (i32.const 0))))
"#;

#[test]
fn wasm_shaper_passes_features_to_the_module() {
    let wasm = wat::parse_str(FEATURES_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let features = [Feature::new(Tag::new(b"dlig"), 1, 2..4)];
    let buffer = shape_text(data, "a", &features);
    assert_eq!(
        glyph_ids(&buffer),
        vec![1, u32::from_be_bytes(*b"dlig"), 1, 2, 3]
    );
}

/// Answers with the direction and script the host gave it.
const RTL_GUEST: &str = r#"
(module
  (import "env" "buffer_get_direction" (func $direction (param i32) (result i32)))
  (import "env" "buffer_get_script" (func $script (param i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (i32.store (i32.const 0) (i32.const 2))
    (i32.store (i32.const 4) (i32.const 1024))
    (i32.store (i32.const 8) (i32.const 2048))
    (i32.store (i32.const 1024) (call $direction (local.get $buf)))
    (i32.store (i32.const 1044) (call $script (local.get $buf)))
    (call $set (local.get $buf) (i32.const 0))))
"#;

#[test]
fn wasm_shaper_shapes_right_to_left() {
    let wasm = wat::parse_str(RTL_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let font = Font::new(data, 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    buffer.push_str("\u{0627}\u{0628}\u{062C}");
    buffer.set_direction(Direction::RightToLeft);
    buffer.guess_segment_properties();
    shape(
        &shaper,
        &mut buffer,
        ShapeOptions::new().shapers(&[Shaper::Wasm]),
    )
    .unwrap();
    // HarfBuzz's direction numbers: 4 for left-to-right, 5 for right-to-left.
    assert_eq!(glyph_ids(&buffer), vec![5, u32::from_be_bytes(*b"Arab")]);
}

/// Copies the buffer with no room of its own, grows the arrays the host
/// allocated, and answers with the length and the glyphs that came along.
const REALLOC_GUEST: &str = r#"
(module
  (import "env" "buffer_copy_contents" (func $copy (param i32 i32) (result i32)))
  (import "env" "buffer_contents_realloc" (func $realloc (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $info i32)
    (local $length i32)
    (local $first i32)
    (local $second i32)
    (local $third i32)
    (i32.store (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (i32.const 0))
    (i32.store (i32.const 8) (i32.const 0))
    (if (i32.eqz (call $copy (local.get $buf) (i32.const 0)))
      (then (return (i32.const 0))))
    (if (i32.eqz (call $realloc (i32.const 0) (i32.const 8)))
      (then (return (i32.const 0))))
    (local.set $length (i32.load (i32.const 0)))
    (local.set $info (i32.load (i32.const 4)))
    (local.set $first (i32.load (local.get $info)))
    (local.set $second (i32.load (i32.add (local.get $info) (i32.const 20))))
    (local.set $third (i32.load (i32.add (local.get $info) (i32.const 40))))
    (i32.store (i32.const 1024) (i32.const 4))
    (i32.store (i32.const 1028) (i32.const 2048))
    (i32.store (i32.const 1032) (i32.const 3072))
    (i32.store (i32.const 2048) (local.get $length))
    (i32.store (i32.const 2068) (local.get $first))
    (i32.store (i32.const 2088) (local.get $second))
    (i32.store (i32.const 2108) (local.get $third))
    (call $set (local.get $buf) (i32.const 1024))))
"#;

#[test]
fn wasm_shaper_reallocates_buffer_contents() {
    let wasm = wat::parse_str(REALLOC_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = shape_text(data, "abc", &[]);
    // The length is the size asked for, and the items were copied along.
    assert_eq!(glyph_ids(&buffer), vec![8, 97, 98, 99]);
}

/// Sets coordinates on the root font, which it may not, and on a font of its
/// own, which it may.
const SET_COORDS_GUEST: &str = r#"
(module
  (import "env" "font_create" (func $font_create (param i32) (result i32)))
  (import "env" "font_set_coords" (func $set_coords (param i32 i32) (result i32)))
  (import "env" "buffer_set_contents" (func $set (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "shape") (param $plan i32) (param $font i32) (param $buf i32)
                              (param $features i32) (param $count i32) (result i32)
    (local $created i32)
    (i32.store (i32.const 16) (i32.const 1))
    (i32.store (i32.const 20) (i32.const 32))
    (i32.store (i32.const 32) (i32.const 8192))
    (i32.store (i32.const 1024) (i32.const 2))
    (i32.store (i32.const 1028) (i32.const 2048))
    (i32.store (i32.const 1032) (i32.const 3072))
    (i32.store (i32.const 2048) (call $set_coords (local.get $font) (i32.const 16)))
    (local.set $created (call $font_create (i32.const 1)))
    (i32.store (i32.const 2068) (call $set_coords (local.get $created) (i32.const 16)))
    (call $set (local.get $buf) (i32.const 1024))))
"#;

#[test]
fn wasm_shaper_sets_coords_on_created_fonts_only() {
    let wasm = wat::parse_str(SET_COORDS_GUEST).unwrap();
    let data = add_table(ROBOTO, *b"Wasm", &wasm);
    let buffer = shape_text(data, "a", &[]);
    // The root font is borrowed, so setting its coordinates fails; the font
    // created from its face is the module's own.
    assert_eq!(glyph_ids(&buffer), vec![0, 1]);
}

#[test]
fn wasm_shaper_falls_back_from_a_module_it_cannot_use() {
    // A module with no `shape` export, and one with no memory: neither can
    // exchange buffer contents, so the font is left to OpenType.
    let plain = glyph_ids(&shape_text(ROBOTO.to_vec(), "abc", &[]));
    for guest in [
        r#"(module (memory (export "memory") 1))"#,
        r#"(module
             (func (export "shape") (param i32 i32 i32 i32 i32) (result i32)
               (i32.const 1)))"#,
    ] {
        let wasm = wat::parse_str(guest).unwrap();
        let data = add_table(ROBOTO, *b"Wasm", &wasm);
        assert_eq!(glyph_ids(&shape_text(data, "abc", &[])), plain);
    }
}
