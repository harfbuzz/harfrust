//! Tests for the optional WebAssembly shaper.
#![cfg(feature = "wasm")]

use harfrust::{shape, Buffer, Direction, Feature, Font, ShapeOptions, ShaperFont};
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

fn shape_text(font_data: Vec<u8>, text: &str, features: &[Feature]) -> Buffer {
    let font = Font::new(font_data, 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.set_direction(Direction::LeftToRight);
    buffer.guess_segment_properties();
    shape(&shaper, &mut buffer, ShapeOptions::new().features(features)).unwrap();
    buffer
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
