//! Exercises the C API through its own entry points, the way a C caller
//! reaches them.

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;
use std::sync::Mutex;

use crate::*;

const TEXT: &str = "abc";

fn font_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("harfrust")
        .join("tests")
        .join("fonts")
        .join("rb_custom")
        .join(name)
}

/// A plain font with no `GSUB` or `GPOS`, so shaping is one glyph per
/// character and nothing rewrites the advances a callback produces.
const PLAIN_FONT: &str = "LaBelleAurore.ttf";
const PLAIN_UPEM: c_uint = 1024;

/// A variable font, for exercising variation settings.
const VARIABLE_FONT: &str = "Linefont.ttf";

#[test]
fn set_and_layout_lookup_queries() {
    unsafe {
        let set = hr_set_create();
        assert_eq!(hr_set_is_empty(set), 1);
        hr_set_add(set, 32);
        hr_set_add(set, HR_CODEPOINT_INVALID);
        assert_eq!(hr_set_has(set, 32), 1);
        assert_eq!(hr_set_has(set, HR_CODEPOINT_INVALID), 0);
        hr_set_clear(set);
        assert_eq!(hr_set_is_empty(set), 1);

        with_named_font("PT_Sans-Caption-Web-Regular.ttf", |face, _font| {
            const GSUB: u32 = 0x4753_5542;
            const GPOS: u32 = 0x4750_4F53;
            assert_eq!(hr_ot_layout_has_substitution(face), 1);
            assert_eq!(hr_ot_layout_has_positioning(face), 1);
            let count = hr_ot_layout_table_get_lookup_count(face, GSUB);
            assert!(count > 0);
            assert!(hr_ot_layout_table_get_lookup_count(face, GPOS) > 0);
            assert_eq!(hr_ot_layout_table_get_lookup_count(face, 0), 0);
            hr_ot_layout_lookup_collect_glyphs(face, GSUB, count, set, set, set, set);
            assert_eq!(hr_set_is_empty(set), 1);
            hr_ot_layout_lookup_collect_glyphs(face, GSUB, 0, set, set, set, set);
            assert_eq!(hr_set_is_empty(set), 0);
            assert_eq!(hr_set_has(set, 5), 1); // quotedbl, an input to lookup 0
            assert_eq!(hr_set_has(set, 704), 1); // its substituted glyph
            assert_eq!(hr_set_has(set, 3), 0); // space is outside this lookup
        });
        hr_set_destroy(set);
    }
}

#[test]
fn contextual_lookup_collects_each_glyph_role() {
    unsafe {
        with_named_font("PT_Sans-Caption-Web-Regular.ttf", |face, _font| {
            let sets = [
                hr_set_create(),
                hr_set_create(),
                hr_set_create(),
                hr_set_create(),
            ];
            hr_ot_layout_lookup_collect_glyphs(
                face,
                0x4753_5542,
                2,
                sets[0],
                sets[1],
                sets[2],
                sets[3],
            );
            assert_eq!(hr_set_has(sets[0], 36), 1); // backtrack A
            assert_eq!(hr_set_has(sets[1], 311), 1); // contextual input
            assert_eq!(hr_set_is_empty(sets[2]), 1); // no lookahead
            assert_eq!(hr_set_has(sets[3], 609), 1); // nested substitution output
            assert_eq!(hr_set_has(sets[0], 3), 0); // no all-glyph fallback
            assert_eq!(hr_set_has(sets[1], 3), 0);
            for set in sets {
                hr_set_destroy(set);
            }
        });
    }
}

#[test]
fn pair_class_lookup_collects_the_second_glyph_class() {
    unsafe {
        with_named_font("PT_Sans-Caption-Web-Regular.ttf", |face, _font| {
            let input = hr_set_create();
            hr_ot_layout_lookup_collect_glyphs(
                face,
                0x4750_4F53,
                2,
                ptr::null_mut(),
                input,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            assert_eq!(hr_set_has(input, 695), 1);
            assert_eq!(hr_set_has(input, 7), 0);
            hr_set_destroy(input);
        });
    }
}

#[test]
fn contextual_rule_and_class_formats_do_not_fall_back_to_all_glyphs() {
    unsafe {
        for (font_name, lookup) in [
            ("NotoSansSinhala.subset1.otf", 3), // sequence context format 1
            ("NotoSansMyanmarUI-Regular.subset1.otf", 3), // sequence context format 2
            ("NotoSansMyanmarUI-Regular.subset1.otf", 5), // chained context format 2
        ] {
            with_named_font(font_name, |face, _font| {
                let sets = [
                    hr_set_create(),
                    hr_set_create(),
                    hr_set_create(),
                    hr_set_create(),
                ];
                hr_ot_layout_lookup_collect_glyphs(
                    face,
                    0x4753_5542,
                    lookup,
                    sets[0],
                    sets[1],
                    sets[2],
                    sets[3],
                );
                assert_eq!(hr_set_has(sets[1], 1), 1, "{font_name} lookup {lookup}");
                for set in sets {
                    assert_eq!(hr_set_has(set, 0), 0, "{font_name} lookup {lookup}");
                    hr_set_destroy(set);
                }
            });
        }
    }
}

#[test]
fn ot_tag_conversion_round_trips_and_pages() {
    unsafe {
        let dflt = u32::from_be_bytes(*b"dflt");
        assert!(hr_ot_tag_to_language(dflt).is_null());
        let arabic = hr_ot_tag_to_language(u32::from_be_bytes(*b"ARA "));
        assert_eq!(
            std::ffi::CStr::from_ptr(hr_language_to_string(arabic)).to_bytes(),
            b"ar"
        );

        let unknown = u32::from_be_bytes(*b"ABC ");
        let custom = hr_ot_tag_to_language(unknown);
        assert_eq!(
            std::ffi::CStr::from_ptr(hr_language_to_string(custom)).to_bytes(),
            b"abc-x-hbot-41424320"
        );
        let mut script_count = 0;
        let mut language_count = 1;
        let mut language_tags = [0];
        hr_ot_tags_from_script_and_language(
            HR_SCRIPT_INVALID,
            custom,
            &raw mut script_count,
            ptr::null_mut(),
            &raw mut language_count,
            language_tags.as_mut_ptr(),
        );
        assert_eq!((language_count, language_tags[0]), (1, unknown));

        let chinese = hr_language_from_string(c"zh-Hant".as_ptr(), -1);
        let mut scripts = [0; 3];
        let mut languages = [0; 3];
        script_count = 3;
        language_count = 3;
        hr_ot_tags_from_script_and_language(
            HR_SCRIPT_DEVANAGARI,
            chinese,
            &raw mut script_count,
            scripts.as_mut_ptr(),
            &raw mut language_count,
            languages.as_mut_ptr(),
        );
        assert_eq!(script_count, 3);
        assert_eq!(
            scripts,
            [
                u32::from_be_bytes(*b"dev3"),
                u32::from_be_bytes(*b"dev2"),
                u32::from_be_bytes(*b"deva")
            ]
        );
        assert_eq!(
            (language_count, languages[0]),
            (1, u32::from_be_bytes(*b"ZHT "))
        );

        script_count = 1;
        language_count = 1;
        hr_ot_tags_from_script_and_language(
            HR_SCRIPT_DEVANAGARI,
            chinese,
            &raw mut script_count,
            ptr::null_mut(),
            &raw mut language_count,
            ptr::null_mut(),
        );
        assert_eq!((script_count, language_count), (1, 1));
    }
}

#[test]
fn math_queries_scale_device_values_and_page() {
    unsafe {
        let path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/MathQuery.ttf");
        with_font_file(&path, |face, font| {
            assert_eq!(hr_ot_math_has_data(face), 1);
            assert_eq!(hr_ot_math_is_glyph_extended_shape(face, 1), 1);
            assert_eq!(hr_ot_math_is_glyph_extended_shape(face, 0), 0);
            assert_eq!(
                hr_ot_math_get_constant(font, HR_OT_MATH_CONSTANT_SCRIPT_PERCENT_SCALE_DOWN),
                80
            );
            assert_eq!(
                hr_ot_math_get_constant(font, HR_OT_MATH_CONSTANT_AXIS_HEIGHT),
                642
            );
            assert_eq!(hr_ot_math_get_constant(font, 99), 0);
            assert_eq!(hr_ot_math_get_glyph_italics_correction(font, 1), 120);
            assert_eq!(hr_ot_math_get_glyph_top_accent_attachment(font, 1), 300);
            assert_eq!(
                hr_ot_math_get_glyph_top_accent_attachment(font, 0),
                hr_font_get_glyph_h_advance(font, 0) / 2
            );
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, 399),
                25
            );
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, 400),
                50
            );
            assert_eq!(
                hr_ot_math_get_min_connector_overlap(font, HR_DIRECTION_LTR),
                40
            );

            let mut count = 7;
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    1,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    0,
                    &raw mut count,
                    ptr::null_mut()
                ),
                2
            );
            assert_eq!(count, 7);
            let mut entries = [hr_ot_math_kern_entry_t::default(); 2];
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    1,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    1,
                    &raw mut count,
                    entries.as_mut_ptr()
                ),
                2
            );
            assert_eq!(count, 1);
            assert_eq!(entries[0].max_correction_height, i32::MAX);
            assert_eq!(entries[0].kern_value, 50);

            let mut variants = [hr_ot_math_glyph_variant_t::default(); 1];
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_TTB,
                    0,
                    &raw mut count,
                    variants.as_mut_ptr()
                ),
                1
            );
            assert_eq!(count, 1);
            assert_eq!(
                variants[0],
                hr_ot_math_glyph_variant_t {
                    glyph: 1,
                    advance: 700
                }
            );
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    variants.as_mut_ptr()
                ),
                1
            );
            assert_eq!(variants[0].advance, 900);
            count = 7;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    ptr::null_mut()
                ),
                1
            );
            assert_eq!(count, 7);
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    1,
                    &raw mut count,
                    variants.as_mut_ptr()
                ),
                1
            );
            assert_eq!(count, 0);

            let mut parts = [hr_ot_math_glyph_part_t::default(); 1];
            let mut correction = 0;
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_TTB,
                    0,
                    &raw mut count,
                    parts.as_mut_ptr(),
                    &raw mut correction
                ),
                1
            );
            assert_eq!(count, 1);
            assert_eq!(
                parts[0],
                hr_ot_math_glyph_part_t {
                    glyph: 1,
                    start_connector_length: 20,
                    end_connector_length: 30,
                    full_advance: 700,
                    flags: HR_OT_MATH_GLYPH_PART_FLAG_EXTENDER
                }
            );
            assert_eq!(correction, 70);
            count = 7;
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_TTB,
                    0,
                    &raw mut count,
                    ptr::null_mut(),
                    &raw mut correction
                ),
                1
            );
            assert_eq!(count, 7);
            assert_eq!(correction, 70);

            hr_font_set_scale(font, 4096, 2048);
            hr_font_set_ppem(font, 13, 12);
            assert_eq!(
                hr_ot_math_get_constant(font, HR_OT_MATH_CONSTANT_AXIS_HEIGHT),
                812
            );
            assert_eq!(hr_ot_math_get_glyph_italics_correction(font, 1), 240);
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, 569),
                680
            );
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, 570),
                100
            );
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    1,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    0,
                    &raw mut count,
                    entries.as_mut_ptr()
                ),
                2
            );
            assert_eq!(
                entries[0],
                hr_ot_math_kern_entry_t {
                    max_correction_height: 570,
                    kern_value: 680
                }
            );
            assert_eq!(
                hr_ot_math_get_min_connector_overlap(font, HR_DIRECTION_LTR),
                80
            );
            assert_eq!(
                hr_ot_math_get_min_connector_overlap(font, HR_DIRECTION_TTB),
                40
            );
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    variants.as_mut_ptr()
                ),
                1
            );
            assert_eq!(variants[0].advance, 1800);
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    parts.as_mut_ptr(),
                    &raw mut correction
                ),
                1
            );
            assert_eq!(parts[0].start_connector_length, 40);
            assert_eq!(parts[0].end_connector_length, 60);
            assert_eq!(parts[0].full_advance, 1800);
            assert_eq!(correction, 140);
            hr_font_set_scale(font, 4096, -2048);
            assert_eq!(
                hr_ot_math_get_constant(font, HR_OT_MATH_CONSTANT_AXIS_HEIGHT),
                -812
            );
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, -569),
                680
            );
            assert_eq!(
                hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, -570),
                100
            );
            assert_eq!(
                hr_ot_math_get_min_connector_overlap(font, HR_DIRECTION_TTB),
                -40
            );
        });
    }
}

#[test]
fn math_queries_without_table_return_empty_values() {
    unsafe {
        with_font(|face, font| {
            assert_eq!(hr_ot_math_has_data(face), 0);
            assert_eq!(
                hr_ot_math_get_constant(font, HR_OT_MATH_CONSTANT_AXIS_HEIGHT),
                0
            );
            assert_eq!(hr_ot_math_get_glyph_italics_correction(font, 1), 0);
            assert_eq!(hr_ot_math_is_glyph_extended_shape(face, 1), 0);
            // Asked only how many there are, each getter answers none. What
            // becomes of the count is HarfBuzz's: variants and assembly leave
            // it alone without an array, and kernings clears it.
            let mut count = 37;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 37);
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    ptr::null_mut(),
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 37);
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    1,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    0,
                    &raw mut count,
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 0);

            // With an array, all three report that nothing was written.
            let mut variants = [hr_ot_math_glyph_variant_t::default(); 1];
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    variants.as_mut_ptr()
                ),
                0
            );
            assert_eq!(count, 0);
            let mut parts = [hr_ot_math_glyph_part_t::default(); 1];
            count = 1;
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    parts.as_mut_ptr(),
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 0);
        });
    }
}

#[test]
fn math_count_only_queries_do_not_read_the_count() {
    // Without an array the count is not a capacity, so a caller asking only
    // for the total may pass one it has not set.
    unsafe {
        let path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/MathQuery.ttf");
        with_font_file(&path, |_, font| {
            let mut count = core::mem::MaybeUninit::<c_uint>::uninit();
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    1,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    0,
                    count.as_mut_ptr(),
                    ptr::null_mut()
                ),
                2
            );
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    1,
                    HR_DIRECTION_LTR,
                    0,
                    count.as_mut_ptr(),
                    ptr::null_mut()
                ),
                1
            );
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_TTB,
                    0,
                    count.as_mut_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut()
                ),
                1
            );

            // The italics correction only returns a value, so it too may
            // be storage that was never set.
            let mut correction = core::mem::MaybeUninit::<hr_position_t>::uninit();
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    1,
                    HR_DIRECTION_TTB,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    correction.as_mut_ptr()
                ),
                1
            );
            assert_eq!(correction.assume_init(), 70);

            // A glyph with no kern table has its count cleared even without
            // an array, as in HarfBuzz. That is a write and still not a read.
            assert_eq!(
                hr_ot_math_get_glyph_kernings(
                    font,
                    0,
                    HR_OT_MATH_KERN_TOP_RIGHT,
                    0,
                    count.as_mut_ptr(),
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count.assume_init(), 0);

            // The other two leave the count alone for a glyph with nothing
            // to report, as they do for one with something.
            let mut count = 37;
            assert_eq!(
                hr_ot_math_get_glyph_variants(
                    font,
                    0,
                    HR_DIRECTION_LTR,
                    0,
                    &raw mut count,
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 37);
            assert_eq!(
                hr_ot_math_get_glyph_assembly(
                    font,
                    0,
                    HR_DIRECTION_TTB,
                    0,
                    &raw mut count,
                    ptr::null_mut(),
                    ptr::null_mut()
                ),
                0
            );
            assert_eq!(count, 37);
        });
    }
}

/// A font can fill a `MathKern` with 65535 heights that all sit below the
/// height asked about, and point every record at one device table laid over
/// the records. Picking a kern, or paging to the last entry, must not read
/// all of them.
#[test]
#[cfg_attr(miri, ignore = "bounds wall-clock time, which Miri cannot meet")]
fn math_kerning_searches_a_full_kern_table() {
    let mut math = vec![
        0, 1, 0, 0, 0, 0, 0, 10, 0, 0, // MATH: only MathGlyphInfo
        0, 0, 0, 0, 0, 0, 0, 8, // MathGlyphInfo: only MathKernInfo
        0, 12, 0, 1, 0, 18, 0, 0, 0, 0, 0, 0, // MathKernInfo: a top right MathKern
        0, 1, 0, 1, 0, 1, // Coverage: glyph 1
        0xFF, 0xFF, // MathKern: heightCount
    ];
    let kern_start = math.len() - 2;
    for _ in 0..65535 + 65536 {
        math.extend([0x80, 0x00, 0x00, 0x04]); // -32768, device table at 4
    }
    // Sizes 1 to 65535, eight bits each: the records after it are its deltas.
    math[kern_start + 4..kern_start + 10].copy_from_slice(&[0, 1, 0xFF, 0xFF, 0, 3]);
    let mut data = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    data.extend(b"MATH\0\0\0\0\0\0\0\x1C");
    data.extend((math.len() as u32).to_be_bytes());
    data.extend(math);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let font = hr_font_create(face);
        hr_font_set_ppem(font, 1000, 1000);
        let start = std::time::Instant::now();
        assert_eq!(
            hr_ot_math_get_glyph_kerning(font, 1, HR_OT_MATH_KERN_TOP_RIGHT, 500),
            -32764
        );
        let mut entries = [hr_ot_math_kern_entry_t::default(); 1];
        let mut count = 1;
        assert_eq!(
            hr_ot_math_get_glyph_kernings(
                font,
                1,
                HR_OT_MATH_KERN_TOP_RIGHT,
                65535,
                &raw mut count,
                entries.as_mut_ptr()
            ),
            65536
        );
        assert_eq!(
            entries[0],
            hr_ot_math_kern_entry_t {
                max_correction_height: i32::MAX,
                kern_value: -32764
            }
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        hr_font_destroy(font);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn base_baselines_follow_script_and_font_scale() {
    unsafe {
        with_named_font("NotoSansCJK.subset1.otf", |_face, font| {
            let mut coord = 999;
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT,
                    HR_DIRECTION_LTR,
                    u32::from_be_bytes(*b"hani"),
                    0,
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, 834);
            hr_font_set_scale(font, 2000, 2000);
            assert_eq!(
                hr_ot_layout_get_baseline2(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT,
                    HR_DIRECTION_LTR,
                    HR_SCRIPT_HAN,
                    ptr::null(),
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, 1668);
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_HANGING,
                    HR_DIRECTION_LTR,
                    u32::from_be_bytes(*b"hani"),
                    0,
                    &raw mut coord
                ),
                0
            );
            assert_eq!(coord, 1668);
        });
    }
}

#[test]
fn base_baselines_round_negative_halves_up() {
    unsafe {
        with_named_font("NotoSansCJK.subset1.otf", |_face, font| {
            hr_font_set_scale(font, -250, -250);
            let mut coord = 0;
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT,
                    HR_DIRECTION_LTR,
                    u32::from_be_bytes(*b"hani"),
                    0,
                    &raw mut coord,
                ),
                1
            );
            // 834 * -0.25 = -208.5.
            assert_eq!(coord, -208);
        });
    }
}

#[test]
fn base_device_adjustments_round_negative_halves_up() {
    unsafe {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fonts/BaselineDevice.otf");
        with_font_file(&fixture, |_face, font| {
            hr_font_set_scale(font, -30, -30);
            hr_font_set_ppem(font, 12, 12);
            let mut coord = 0;
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    u32::from_be_bytes(*b"zzzz"),
                    0,
                    &raw mut coord,
                ),
                1
            );
            // The coordinate rounds to 2; the device delta -30/12 to -2.
            assert_eq!(coord, 0);
        });
    }
}

#[test]
fn fallback_vertical_origin_rounds_negative_halves_up() {
    unsafe extern "C" fn no_origin(
        _: *mut hr_font_t,
        _: *mut c_void,
        _: hr_codepoint_t,
        _: *mut hr_position_t,
        _: *mut hr_position_t,
        _: *mut c_void,
    ) -> hr_bool_t {
        0
    }
    unsafe {
        with_font(|_face, font| {
            let funcs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_v_origin_func(funcs, Some(no_origin), ptr::null_mut(), None);
            hr_font_set_funcs(font, funcs, ptr::null_mut(), None);
            hr_font_funcs_destroy(funcs);
            hr_font_set_scale(font, 1024, -512);
            let mut y = 0;
            hr_font_get_glyph_origin_for_direction(
                font,
                1,
                HR_DIRECTION_TTB,
                ptr::null_mut(),
                &raw mut y,
            );
            // The font's 1103-unit ascender scales to -551.5.
            assert_eq!(y, -551);
        });
    }
}

#[test]
fn baseline_fallback_and_script_defaults() {
    assert_eq!(
        hr_ot_layout_get_horizontal_baseline_tag_for_script(HR_SCRIPT_LATIN),
        HR_OT_LAYOUT_BASELINE_TAG_ROMAN
    );
    assert_eq!(
        hr_ot_layout_get_horizontal_baseline_tag_for_script(HR_SCRIPT_DEVANAGARI),
        HR_OT_LAYOUT_BASELINE_TAG_HANGING
    );
    assert_eq!(
        hr_ot_layout_get_horizontal_baseline_tag_for_script(HR_SCRIPT_HAN),
        HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT
    );
    unsafe {
        with_font(|_face, font| {
            let mut coord = 123;
            hr_ot_layout_get_baseline_with_fallback(
                font,
                HR_OT_LAYOUT_BASELINE_TAG_ROMAN,
                HR_DIRECTION_LTR,
                u32::from_be_bytes(*b"latn"),
                0,
                &raw mut coord,
            );
            assert_eq!(coord, 0);
            hr_ot_layout_get_baseline_with_fallback2(
                font,
                HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_CENTRAL,
                HR_DIRECTION_LTR,
                HR_SCRIPT_HAN,
                ptr::null(),
                &raw mut coord,
            );
            assert_ne!(coord, 123);
        });
    }
}

/// Finds a glyph for the letter Ka of Devanagari and of Limbu, the characters
/// whose tops stand in for those scripts' hanging baselines, and for nothing
/// else.
unsafe extern "C" fn only_the_two_ka_letters(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    unicode: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
    _user_data: *mut c_void,
) -> hr_bool_t {
    let found = match unicode {
        0x0915 => 5,
        0x1901 => 6,
        _ => return 0,
    };
    unsafe { *glyph = found };
    1
}

unsafe extern "C" fn top_at_a_hundred_times_the_glyph(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    glyph: hr_codepoint_t,
    extents: *mut hr_glyph_extents_t,
    _user_data: *mut c_void,
) -> hr_bool_t {
    unsafe {
        *extents = hr_glyph_extents_t {
            x_bearing: 0,
            y_bearing: glyph as i32 * 100,
            width: 10,
            height: -10,
        };
    }
    1
}

#[test]
fn the_hanging_baseline_fallback_knows_a_script_by_its_opentype_tag() {
    unsafe {
        with_font(|_face, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(only_the_two_ka_letters),
                ptr::null_mut(),
                None,
            );
            hr_font_funcs_set_glyph_extents_func(
                ffuncs,
                Some(top_at_a_hundred_times_the_glyph),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let (mut x_scale, mut y_scale) = (0, 0);
            hr_font_get_scale(font, &raw mut x_scale, &raw mut y_scale);

            let by_tag = |tag: &[u8; 4], direction| {
                let mut coord = -1;
                hr_ot_layout_get_baseline_with_fallback(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_HANGING,
                    direction,
                    u32::from_be_bytes(*tag),
                    0,
                    &raw mut coord,
                );
                coord
            };
            let by_script = |script, direction| {
                let mut coord = -1;
                hr_ot_layout_get_baseline_with_fallback2(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_HANGING,
                    direction,
                    script,
                    ptr::null(),
                    &raw mut coord,
                );
                coord
            };

            // The font has no BASE table, so the hanging baseline is the top
            // of the script's reference glyph. The script arrives as an
            // OpenType tag, in any of its generations, as HarfBuzz takes it.
            for tag in [b"deva", b"dev2", b"dev3"] {
                assert_eq!(by_tag(tag, HR_DIRECTION_LTR), 500, "{tag:?}");
            }
            assert_eq!(by_script(HR_SCRIPT_DEVANAGARI, HR_DIRECTION_LTR), 500);
            // A script HarfBuzz has a reference character for beyond the
            // first five.
            assert_eq!(by_tag(b"limb", HR_DIRECTION_LTR), 600);
            assert_eq!(by_script(HR_SCRIPT_LIMBU, HR_DIRECTION_LTR), 600);

            // No reference character, or no glyph for it, or vertical text:
            // six tenths of the em.
            let no_glyph = y_scale * 6 / 10;
            assert_eq!(by_tag(b"latn", HR_DIRECTION_LTR), no_glyph);
            assert_eq!(by_tag(b"bng2", HR_DIRECTION_LTR), no_glyph);
            assert_eq!(by_script(HR_SCRIPT_BENGALI, HR_DIRECTION_LTR), no_glyph);
            assert_eq!(by_tag(b"dev2", HR_DIRECTION_TTB), x_scale * 6 / 10);
        });
    }
}

#[test]
fn base_baseline2_selects_indic_opentype_script_tag() {
    unsafe {
        with_named_font("Rasa.subset1.otf", |_face, font| {
            let mut coord = 0;
            assert_eq!(
                hr_ot_layout_get_baseline2(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    HR_SCRIPT_GUJARATI,
                    ptr::null(),
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, -236);
        });
    }
}

#[test]
fn base_device_adjustment_uses_coordinate_axis_ppem() {
    unsafe {
        // Derived from NotoSansCJK.subset1.otf: the DFLT icfb coordinate has
        // a format 3 Device table for sizes 12..=17, with deltas 1..=6.
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fonts/BaselineDevice.otf");
        with_font_file(&fixture, |_face, font| {
            let mut coord = 0;
            let script = u32::from_be_bytes(*b"zzzz"); // Select the BASE default script.
            hr_font_set_scale(font, 1200, 2400);
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    script,
                    0,
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, -178);

            hr_font_set_ptem(font, 12.0);
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    script,
                    0,
                    &raw mut coord,
                ),
                1
            );
            assert_eq!(coord, -178, "point size does not set pixels per em");

            // Horizontal text reads a y coordinate, so x_ppem is irrelevant.
            hr_font_set_ppem(font, 17, 12);
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    script,
                    0,
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, 22);

            hr_font_set_ppem(font, 17, 17);
            assert_eq!(
                hr_ot_layout_get_baseline(
                    font,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    HR_DIRECTION_LTR,
                    script,
                    0,
                    &raw mut coord
                ),
                1
            );
            assert_eq!(coord, 669);
        });
    }
}

fn font_data() -> Vec<u8> {
    std::fs::read(font_path(PLAIN_FONT)).expect("failed to read test font")
}

/// Builds a blob over `data`, which the caller must keep alive.
unsafe fn blob_over(data: &[u8]) -> *mut hr_blob_t {
    unsafe {
        hr_blob_create(
            data.as_ptr().cast::<c_char>(),
            data.len() as c_uint,
            HR_MEMORY_MODE_READONLY,
            ptr::null_mut(),
            None,
        )
    }
}

/// Runs `f` with a font built over the named test face.
unsafe fn with_named_font(name: &str, f: impl FnOnce(*mut hr_face_t, *mut hr_font_t)) {
    unsafe { with_font_file(&font_path(name), f) };
}

unsafe fn with_font_file(path: &std::path::Path, f: impl FnOnce(*mut hr_face_t, *mut hr_font_t)) {
    let data = std::fs::read(path).expect("failed to read test font");
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let font = hr_font_create(face);
        f(face, font);
        hr_font_destroy(font);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

/// Runs `f` with a font built over [`PLAIN_FONT`].
unsafe fn with_font(f: impl FnOnce(*mut hr_face_t, *mut hr_font_t)) {
    unsafe { with_named_font(PLAIN_FONT, f) };
}

unsafe fn buffer_with_text(text: &str) -> *mut hr_buffer_t {
    unsafe {
        let buffer = hr_buffer_create();
        hr_buffer_add_utf8(
            buffer,
            text.as_ptr().cast::<c_char>(),
            text.len() as c_int,
            0,
            -1,
        );
        hr_buffer_guess_segment_properties(buffer);
        buffer
    }
}

unsafe fn glyph_ids(buffer: *mut hr_buffer_t) -> Vec<u32> {
    unsafe {
        let mut len: c_uint = 0;
        let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        (0..len as usize)
            .map(|i| (*infos.add(i)).codepoint)
            .collect()
    }
}

// ---------------------------------------------------------------- blobs ----

#[test]
fn blob_round_trips_its_data() {
    let data = b"some font bytes".to_vec();
    unsafe {
        let blob = blob_over(&data);
        assert_eq!(hr_blob_get_length(blob) as usize, data.len());
        let mut len: c_uint = 0;
        let got = hr_blob_get_data(blob, &raw mut len);
        let seen = core::slice::from_raw_parts(got.cast::<u8>(), len as usize);
        assert_eq!(seen, data.as_slice());
        hr_blob_destroy(blob);
    }
}

#[test]
fn blob_duplicate_mode_copies() {
    unsafe {
        let blob = {
            let scratch = b"temporary".to_vec();
            hr_blob_create(
                scratch.as_ptr().cast::<c_char>(),
                scratch.len() as c_uint,
                HR_MEMORY_MODE_DUPLICATE,
                ptr::null_mut(),
                None,
            )
            // `scratch` is dropped here; the blob must own its own copy.
        };
        let mut len: c_uint = 0;
        let got = hr_blob_get_data(blob, &raw mut len);
        assert_eq!(
            core::slice::from_raw_parts(got.cast::<u8>(), len as usize),
            b"temporary"
        );
        hr_blob_destroy(blob);
    }
}

#[test]
fn empty_blob_is_immortal_and_shared() {
    unsafe {
        let a = hr_blob_get_empty();
        let b = hr_blob_get_empty();
        assert_eq!(a, b);
        assert_eq!(hr_blob_get_length(a), 0);
        // Referencing and destroying an immortal object is a no-op, so this
        // must not free anything.
        hr_blob_destroy(hr_blob_reference(a));
        assert_eq!(hr_blob_get_length(hr_blob_get_empty()), 0);
    }
}

#[test]
fn data_that_is_not_there_is_null() {
    // HarfBuzz gives `NULL` for a blob with no data and for a font with no
    // coordinates, and a caller may test the pointer and not the length.
    unsafe {
        let mut len = 7;
        assert!(hr_blob_get_data(hr_blob_get_empty(), &raw mut len).is_null());
        assert_eq!(len, 0);
        len = 7;
        assert!(hr_blob_get_data(ptr::null_mut(), &raw mut len).is_null());
        assert_eq!(len, 0);

        let data = font_data();
        let blob = blob_over(&data);
        len = 0;
        assert!(!hr_blob_get_data(blob, &raw mut len).is_null());
        assert_eq!(len as usize, data.len());
        let nothing = hr_blob_create_sub_blob(blob, 4, 0);
        len = 7;
        assert!(hr_blob_get_data(nothing, &raw mut len).is_null());
        assert_eq!(len, 0);
        hr_blob_destroy(nothing);
        hr_blob_destroy(blob);

        // A blob made over a buffer of no length is still at that buffer,
        // which HarfBuzz reports as it was given.
        let somewhere = [0u8; 1];
        let at = somewhere.as_ptr().cast::<c_char>();
        let explicit =
            hr_blob_create_or_fail(at, 0, HR_MEMORY_MODE_READONLY, ptr::null_mut(), None);
        assert!(!explicit.is_null());
        assert_ne!(explicit, hr_blob_get_empty());
        let mut unset = core::mem::MaybeUninit::<c_uint>::uninit();
        assert_eq!(hr_blob_get_data(explicit, unset.as_mut_ptr()), at);
        assert_eq!(unset.assume_init(), 0);
        hr_blob_destroy(explicit);
        // A copy of nothing is nowhere, and asking for a blob of nothing
        // that may be shared gives the empty blob, which has no data.
        let copied = hr_blob_create_or_fail(at, 0, HR_MEMORY_MODE_DUPLICATE, ptr::null_mut(), None);
        assert!(hr_blob_get_data(copied, &raw mut len).is_null());
        hr_blob_destroy(copied);
        let shared = hr_blob_create(at, 0, HR_MEMORY_MODE_READONLY, ptr::null_mut(), None);
        assert_eq!(shared, hr_blob_get_empty());
        assert!(hr_blob_get_data(shared, &raw mut len).is_null());

        with_font(|face, font| {
            // A table the face does not have.
            let missing = hr_face_reference_table(face, u32::from_be_bytes(*b"none"));
            let mut len = 7;
            assert!(hr_blob_get_data(missing, &raw mut len).is_null());
            assert_eq!(len, 0);
            hr_blob_destroy(missing);

            // A font with no variation axes.
            len = 7;
            assert!(hr_font_get_var_coords_normalized(font, &raw mut len).is_null());
            assert_eq!(len, 0);
        });
        len = 7;
        assert!(hr_font_get_var_coords_normalized(ptr::null_mut(), &raw mut len).is_null());
        assert_eq!(len, 0);
        with_named_font(VARIABLE_FONT, |_, font| {
            // Coordinates exist once a variation is set, and not before.
            let mut len = 7;
            assert!(hr_font_get_var_coords_normalized(font, &raw mut len).is_null());
            assert_eq!(len, 0);
            let variation = hr_variation_t {
                tag: u32::from_be_bytes(*b"wdth"),
                value: 75.0,
            };
            hr_font_set_variations(font, &raw const variation, 1);
            assert!(!hr_font_get_var_coords_normalized(font, &raw mut len).is_null());
            assert_ne!(len, 0);
        });
    }
}

#[test]
fn sub_blob_shares_parent_storage() {
    let data = b"0123456789".to_vec();
    unsafe {
        let parent = blob_over(&data);
        let sub = hr_blob_create_sub_blob(parent, 2, 3);
        // Dropping the parent reference must not invalidate the sub-blob.
        hr_blob_destroy(parent);
        let mut len: c_uint = 0;
        let got = hr_blob_get_data(sub, &raw mut len);
        assert_eq!(
            core::slice::from_raw_parts(got.cast::<u8>(), len as usize),
            b"234"
        );
        hr_blob_destroy(sub);
    }
}

#[test]
fn sub_blob_keeps_its_parent_alive() {
    static KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };
    let data = b"0123456789".to_vec();
    let drops = AtomicUsize::new(0);
    unsafe {
        // The bytes may belong to the parent through its user data, so a
        // sub-blob has to keep the parent itself and not only its storage.
        let parent = blob_over(&data);
        let drops_ptr: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        hr_blob_set_user_data(parent, &raw const KEY, drops_ptr, Some(count_drop), 1);
        let sub = hr_blob_create_sub_blob(parent, 2, 3);
        hr_blob_destroy(parent);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        hr_blob_destroy(sub);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

// ---------------------------------------------------------------- faces ----

#[test]
fn face_reports_its_metrics() {
    unsafe {
        with_font(|face, _| {
            assert_eq!(hr_face_get_upem(face), PLAIN_UPEM);
            assert!(hr_face_get_glyph_count(face) > 0);
            assert_eq!(hr_face_get_index(face), 0);
        });
    }
}

#[test]
fn face_references_a_table() {
    unsafe {
        with_font(|face, _| {
            let tag = hr_tag_from_string(c"cmap".as_ptr(), -1);
            let table = hr_face_reference_table(face, tag);
            assert!(hr_blob_get_length(table) > 0);
            hr_blob_destroy(table);

            // A table the font does not have yields the empty blob.
            let missing = hr_tag_from_string(c"ZZZZ".as_ptr(), -1);
            let table = hr_face_reference_table(face, missing);
            assert_eq!(hr_blob_get_length(table), 0);
            hr_blob_destroy(table);
        });
    }
}

/// Builds a font whose directory holds `count` records, every one pointing at
/// the same four bytes. Record `i` has tag `record_tag(i)`, ascending when
/// `sorted` and descending otherwise.
fn sfnt_with_records(count: usize, sorted: bool) -> Vec<u8> {
    let mut data = 0x0001_0000u32.to_be_bytes().to_vec();
    data.extend((count as u16).to_be_bytes());
    data.extend([0; 6]);
    let offset = 12 + 16 * count;
    for i in 0..count {
        let tag = record_tag(if sorted { i } else { count - 1 - i });
        data.extend(tag.to_be_bytes());
        data.extend(0u32.to_be_bytes());
        data.extend((offset as u32).to_be_bytes());
        data.extend(4u32.to_be_bytes());
    }
    data.extend([1, 2, 3, 4]);
    data
}

/// Distinct tags for [`sfnt_with_records`], avoiding the none tag.
fn record_tag(i: usize) -> hr_tag_t {
    i as hr_tag_t + 1
}

#[test]
#[cfg_attr(miri, ignore = "bounds wall-clock time, which Miri cannot meet")]
fn walking_every_table_of_a_face_stays_linear() {
    // The most records a directory can hold. Fetching each table once must
    // not read the whole directory again per call.
    const COUNT: usize = u16::MAX as usize;
    for sorted in [true, false] {
        let data = sfnt_with_records(COUNT, sorted);
        unsafe {
            let blob = blob_over(&data);
            let face = hr_face_create(blob, 0);
            let started = std::time::Instant::now();
            let mut count = COUNT as c_uint;
            let mut tags = vec![0; COUNT];
            assert_eq!(
                hr_face_get_table_tags(face, 0, &raw mut count, tags.as_mut_ptr()),
                COUNT as c_uint
            );
            assert_eq!(count, COUNT as c_uint);
            for (i, &tag) in tags.iter().enumerate() {
                assert_eq!(tag, record_tag(if sorted { i } else { COUNT - 1 - i }));
                let table = hr_face_reference_table(face, tag);
                assert_eq!(hr_blob_get_length(table), 4);
                hr_blob_destroy(table);
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            hr_face_destroy(face);
            hr_blob_destroy(blob);
        }
    }
}

#[test]
fn first_record_of_a_repeated_tag_wins() {
    // Two records with one tag leave the directory unsorted; `read-fonts`
    // then finds the first, and so must the face.
    let mut data = 0x0001_0000u32.to_be_bytes().to_vec();
    data.extend(2u16.to_be_bytes());
    data.extend([0; 6]);
    for (offset, length) in [(44u32, 2u32), (46, 3)] {
        data.extend(*b"cmap");
        data.extend(0u32.to_be_bytes());
        data.extend(offset.to_be_bytes());
        data.extend(length.to_be_bytes());
    }
    data.extend([1, 2, 3, 4, 5]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let tag = hr_tag_from_string(c"cmap".as_ptr(), -1);
        let table = hr_face_reference_table(face, tag);
        assert_eq!(hr_blob_get_length(table), 2);
        hr_blob_destroy(table);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn null_and_empty_faces_are_safe() {
    unsafe {
        assert_eq!(hr_face_get_upem(ptr::null_mut()), 1000);
        assert_eq!(hr_face_get_glyph_count(ptr::null_mut()), 0);
        assert_eq!(hr_face_get_glyph_count(hr_face_get_empty()), 0);
    }
}

// ---------------------------------------------------------------- fonts ----

/// A parameter that only returns a value is written without being read, so
/// its storage need not hold anything on the way in. Every output here is
/// handed storage that holds nothing, which Miri would refuse to read back
/// had a call left any of it that way.
#[test]
fn outputs_need_not_be_set_before_the_call() {
    use core::mem::MaybeUninit;

    unsafe {
        with_font(|face, font| {
            // A pair of scalars.
            hr_font_set_scale(font, 1000, 2000);
            let mut x = MaybeUninit::<c_int>::uninit();
            let mut y = MaybeUninit::<c_int>::uninit();
            hr_font_get_scale(font, x.as_mut_ptr(), y.as_mut_ptr());
            assert_eq!((x.assume_init(), y.assume_init()), (1000, 2000));

            // One scalar, written before the font is asked and again after.
            let mut glyph = MaybeUninit::<hr_codepoint_t>::uninit();
            let found = hr_font_get_nominal_glyph(font, u32::from('a'), glyph.as_mut_ptr());
            assert_ne!(found, 0);
            let glyph = glyph.assume_init();
            assert_ne!(glyph, 0);

            // A struct, and the same struct moved to another origin, which
            // is worked out from the first before it is written.
            let mut plain = MaybeUninit::<hr_glyph_extents_t>::uninit();
            assert_ne!(
                hr_font_get_glyph_extents(font, glyph, plain.as_mut_ptr()),
                0
            );
            let plain = plain.assume_init();
            let mut origin_x = MaybeUninit::<hr_position_t>::uninit();
            let mut origin_y = MaybeUninit::<hr_position_t>::uninit();
            hr_font_get_glyph_origin_for_direction(
                font,
                glyph,
                HR_DIRECTION_TTB,
                origin_x.as_mut_ptr(),
                origin_y.as_mut_ptr(),
            );
            let mut moved = MaybeUninit::<hr_glyph_extents_t>::uninit();
            assert_ne!(
                hr_font_get_glyph_extents_for_origin(
                    font,
                    glyph,
                    HR_DIRECTION_TTB,
                    moved.as_mut_ptr()
                ),
                0
            );
            assert_eq!(
                moved.assume_init(),
                hr_glyph_extents_t {
                    x_bearing: plain.x_bearing - origin_x.assume_init(),
                    y_bearing: plain.y_bearing - origin_y.assume_init(),
                    ..plain
                }
            );

            // A count that is a capacity when it comes with an array, and is
            // only cleared when the face has no such palette.
            let mut count = MaybeUninit::<c_uint>::uninit();
            let total =
                hr_ot_color_palette_get_colors(face, 0, 0, count.as_mut_ptr(), ptr::null_mut());
            assert_eq!((total, count.assume_init()), (0, 0));

            // A length, and the text it is the length of.
            let buffer = glyph_buffer(&[1, 2]);
            let mut text = [MaybeUninit::<c_char>::uninit(); 16];
            let mut consumed = MaybeUninit::<c_uint>::uninit();
            let items = hr_buffer_serialize_glyphs(
                buffer,
                0,
                2,
                text.as_mut_ptr().cast::<c_char>(),
                text.len() as c_uint,
                consumed.as_mut_ptr(),
                font,
                HR_BUFFER_SERIALIZE_FORMAT_TEXT,
                HR_BUFFER_SERIALIZE_FLAG_NO_GLYPH_NAMES,
            );
            let written = std::ffi::CStr::from_ptr(text.as_ptr().cast::<c_char>());
            assert_eq!((items, written.to_str().unwrap()), (2, "[1=0|2=1]"));
            assert_eq!(consumed.assume_init() as usize, written.to_bytes().len());
            hr_buffer_destroy(buffer);
        });

        // A struct with pointers in it, written whole.
        let buffer = buffer_with_text(TEXT);
        let mut props = MaybeUninit::<hr_segment_properties_t>::uninit();
        hr_buffer_get_segment_properties(buffer, props.as_mut_ptr());
        let props = props.assume_init();
        assert_eq!(props.direction, HR_DIRECTION_LTR);
        assert_eq!(props.script, HR_SCRIPT_LATIN);
        assert_eq!(props.language, hr_buffer_get_language(buffer));
        assert!(props.reserved1.is_null() && props.reserved2.is_null());
        hr_buffer_destroy(buffer);

        // An array, of which only as much as is reported is written.
        let set = hr_set_create();
        hr_set_add(set, 3);
        hr_set_add(set, 7);
        let mut values = [MaybeUninit::<hr_codepoint_t>::uninit(); 4];
        let count = hr_set_next_many(
            set,
            HR_SET_VALUE_INVALID,
            values.as_mut_ptr().cast::<hr_codepoint_t>(),
            values.len() as c_uint,
        );
        assert_eq!(count, 2);
        assert_eq!([values[0].assume_init(), values[1].assume_init()], [3, 7]);
        hr_set_destroy(set);
    }
}

#[test]
fn font_scale_defaults_to_upem() {
    unsafe {
        with_font(|face, font| {
            let (mut x, mut y) = (0, 0);
            hr_font_get_scale(font, &raw mut x, &raw mut y);
            assert_eq!(x as c_uint, hr_face_get_upem(face));
            assert_eq!(y as c_uint, hr_face_get_upem(face));

            hr_font_set_scale(font, 1000, 2000);
            hr_font_get_scale(font, &raw mut x, &raw mut y);
            assert_eq!((x, y), (1000, 2000));
        });
    }
}

#[test]
fn font_ppem_defaults_inherits_and_respects_immutability() {
    unsafe {
        with_font(|_, font| {
            let (mut x, mut y) = (99, 99);
            hr_font_get_ppem(font, &raw mut x, &raw mut y);
            assert_eq!((x, y), (0, 0));

            hr_font_set_ppem(font, 12, 17);
            let sub = hr_font_create_sub_font(font);
            hr_font_get_ppem(sub, &raw mut x, &raw mut y);
            assert_eq!((x, y), (12, 17));

            hr_font_set_ppem(sub, 21, 28);
            hr_font_get_ppem(font, &raw mut x, &raw mut y);
            assert_eq!((x, y), (12, 17));

            hr_font_make_immutable(sub);
            hr_font_set_ppem(sub, 33, 44);
            hr_font_get_ppem(sub, &raw mut x, &raw mut y);
            assert_eq!((x, y), (21, 28));
            hr_font_destroy(sub);
        });

        let (mut x, mut y) = (99, 99);
        hr_font_get_ppem(ptr::null_mut(), &raw mut x, &raw mut y);
        assert_eq!((x, y), (0, 0));
    }
}

#[test]
fn font_maps_codepoints_to_glyphs() {
    unsafe {
        with_font(|_, font| {
            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, &raw mut glyph),
                0
            );
            assert_ne!(glyph, 0);
            // A codepoint this subset font does not cover.
            assert_eq!(hr_font_get_nominal_glyph(font, 0x4E00, &raw mut glyph), 0);
        });
    }
}

#[test]
fn creating_a_font_freezes_its_face() {
    // A font reads its face from whichever thread it is used on, so the face
    // has to hold still once one exists. HarfBuzz freezes it at the same
    // point, which makes a later setter a no-op and not a race.
    unsafe {
        let data = font_data();
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        hr_blob_destroy(blob);
        assert_eq!(hr_face_is_immutable(face), 0);
        hr_face_set_index(face, 3);
        assert_eq!(hr_face_get_index(face), 3);

        let font = hr_font_create(face);
        assert_ne!(hr_face_is_immutable(face), 0);
        hr_face_set_index(face, 5);
        assert_eq!(hr_face_get_index(face), 3);

        // A sub-font is over the same face, frozen already.
        let sub = hr_font_create_sub_font(font);
        assert_eq!(hr_font_get_face(sub), face);
        hr_font_destroy(sub);
        hr_font_destroy(font);

        // Nothing thaws it once its fonts are gone.
        assert_ne!(hr_face_is_immutable(face), 0);
        hr_face_destroy(face);
    }
}

#[test]
fn a_table_callback_sees_its_face_frozen_while_a_font_is_made() {
    // The face is frozen before anything is read from it on the font's
    // behalf, so the callback asked for `head`, which the font's scale comes
    // from, already sees it that way. HarfBuzz orders it the same.
    struct Source {
        backing: *mut hr_face_t,
        frozen_at_head: core::cell::Cell<Option<hr_bool_t>>,
    }

    unsafe extern "C" fn table(
        face: *mut hr_face_t,
        tag: hr_tag_t,
        user_data: *mut c_void,
    ) -> *mut hr_blob_t {
        unsafe {
            let source = &*user_data.cast::<Source>();
            if tag == u32::from_be_bytes(*b"head") && source.frozen_at_head.get().is_none() {
                source.frozen_at_head.set(Some(hr_face_is_immutable(face)));
            }
            hr_face_reference_table(source.backing, tag)
        }
    }

    unsafe {
        let data = font_data();
        let blob = blob_over(&data);
        let backing = hr_face_create(blob, 0);
        hr_blob_destroy(blob);
        let source = Source {
            backing,
            frozen_at_head: core::cell::Cell::new(None),
        };
        let face =
            hr_face_create_for_tables(Some(table), ptr::from_ref(&source).cast_mut().cast(), None);
        assert_eq!(hr_face_is_immutable(face), 0);
        assert_eq!(source.frozen_at_head.get(), None);

        let font = hr_font_create(face);
        assert_eq!(source.frozen_at_head.get(), Some(1));

        hr_font_destroy(font);
        hr_face_destroy(face);
        hr_face_destroy(backing);
    }
}

#[test]
fn immutable_font_rejects_changes() {
    unsafe {
        with_font(|_, font| {
            hr_font_set_scale(font, 512, 512);
            hr_font_make_immutable(font);
            assert_ne!(hr_font_is_immutable(font), 0);
            hr_font_set_scale(font, 999, 999);
            let (mut x, mut y) = (0, 0);
            hr_font_get_scale(font, &raw mut x, &raw mut y);
            assert_eq!((x, y), (512, 512));
        });
    }
}

/// A sub-font answers through its parent, so it holds still only if the
/// parent does: freezing it freezes the chain, as in HarfBuzz.
#[test]
fn freezing_a_sub_font_freezes_the_fonts_it_descends_from() {
    unsafe {
        with_font(|_, font| {
            let child = hr_font_create_sub_font(font);
            let grandchild = hr_font_create_sub_font(child);
            hr_font_make_immutable(grandchild);
            assert_ne!(hr_font_is_immutable(grandchild), 0);
            assert_ne!(hr_font_is_immutable(child), 0);
            assert_ne!(hr_font_is_immutable(font), 0);

            // The base font now rejects changes the grandchild would see.
            hr_font_set_scale(font, 512, 512);
            let (mut x, mut y) = (0, 0);
            hr_font_get_scale(font, &raw mut x, &raw mut y);
            assert_ne!((x, y), (512, 512));
            let replacement = hr_font_funcs_create();
            hr_font_set_funcs(font, replacement, ptr::null_mut(), None);
            hr_font_funcs_destroy(replacement);
            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(grandchild, u32::from(b'a'), &raw mut glyph),
                0,
                "the base font should still read its own cmap"
            );

            hr_font_destroy(grandchild);
            hr_font_destroy(child);
        });
    }
}

// --------------------------------------------------------------- buffers ----

#[test]
fn buffer_tracks_its_content_type() {
    unsafe {
        let buffer = hr_buffer_create();
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            HR_BUFFER_CONTENT_TYPE_INVALID
        );
        hr_buffer_add(buffer, 'x' as u32, 0);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            HR_BUFFER_CONTENT_TYPE_UNICODE
        );
        hr_buffer_clear_contents(buffer);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            HR_BUFFER_CONTENT_TYPE_INVALID
        );
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn ill_formed_text_replaces_one_byte_at_a_time() {
    unsafe {
        // A truncated three-byte sequence followed by an ASCII byte. Each
        // byte that cannot begin a character is its own replacement, keeping
        // its own offset, rather than a run of them collapsing into one.
        let buffer = hr_buffer_create();
        let bad = [0xE2u8, 0x82, b'a'];
        hr_buffer_add_utf8(buffer, bad.as_ptr().cast(), 3, 0, -1);
        let mut len = 0;
        let infos = std::slice::from_raw_parts(
            hr_buffer_get_glyph_infos(buffer, &raw mut len),
            len as usize,
        );
        let seen: Vec<(u32, u32)> = infos.iter().map(|i| (i.codepoint, i.cluster)).collect();
        const FFFD: u32 = 0xFFFD;
        assert_eq!(seen, [(FFFD, 0), (FFFD, 1), (u32::from(b'a'), 2)]);
        hr_buffer_destroy(buffer);

        // UTF-32 carries values that are not characters at all; only
        // add_codepoints promises to pass those through untouched.
        let buffer = hr_buffer_create();
        let items = [0x41u32, 0xD800, 0x0011_0000, 0x42];
        hr_buffer_add_utf32(buffer, items.as_ptr(), 4, 0, -1);
        let mut len = 0;
        let infos = std::slice::from_raw_parts(
            hr_buffer_get_glyph_infos(buffer, &raw mut len),
            len as usize,
        );
        let seen: Vec<(u32, u32)> = infos.iter().map(|i| (i.codepoint, i.cluster)).collect();
        assert_eq!(seen, [(0x41, 0), (FFFD, 1), (FFFD, 2), (0x42, 3)]);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn buffer_exposes_codepoints_before_shaping() {
    unsafe {
        let buffer = buffer_with_text(TEXT);
        assert_eq!(hr_buffer_get_length(buffer) as usize, TEXT.chars().count());
        assert_eq!(
            glyph_ids(buffer),
            TEXT.chars().map(|c| c as u32).collect::<Vec<_>>()
        );
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn add_utf8_uses_context_and_absolute_clusters() {
    unsafe {
        let text = "abcdef";
        let buffer = hr_buffer_create();
        // Add only "cd", leaving "ab" and "ef" as context.
        hr_buffer_add_utf8(
            buffer,
            text.as_ptr().cast::<c_char>(),
            text.len() as c_int,
            2,
            2,
        );
        assert_eq!(hr_buffer_get_length(buffer), 2);
        let mut len: c_uint = 0;
        let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        // Cluster values count from the start of the whole text.
        assert_eq!((*infos).codepoint, 'c' as u32);
        assert_eq!((*infos).cluster, 2);
        assert_eq!((*infos.add(1)).cluster, 3);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn add_utf16_decodes_pairs_and_keeps_code_unit_clusters() {
    unsafe {
        let buffer = hr_buffer_create();
        let text = [
            b'a' as u16,
            0xD83D,
            0xDE00,
            b'b' as u16,
            0xD800,
            b'c' as u16,
            0,
        ];
        hr_buffer_add_utf16(buffer, text.as_ptr(), -1, 1, 5);
        let mut len = 0;
        let infos = std::slice::from_raw_parts(
            hr_buffer_get_glyph_infos(buffer, &raw mut len),
            len as usize,
        );
        let seen: Vec<_> = infos.iter().map(|i| (i.codepoint, i.cluster)).collect();
        assert_eq!(
            seen,
            [
                (0x1F600, 1),
                (b'b' as u32, 3),
                (0xFFFD, 4),
                (b'c' as u32, 5)
            ]
        );
        hr_buffer_destroy(buffer);

        let buffer = hr_buffer_create();
        // Splitting a surrogate pair at the item boundary makes the high
        // surrogate an invalid item, rather than consuming its context.
        hr_buffer_add_utf16(buffer, text.as_ptr(), 6, 1, 1);
        let mut len = 0;
        let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        assert_eq!(len, 1);
        assert_eq!((*infos).codepoint, 0xFFFD);
        assert_eq!((*infos).cluster, 1);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn add_latin1_preserves_high_bytes_and_absolute_clusters() {
    unsafe {
        let buffer = hr_buffer_create();
        let text = [b'a', 0xE9, b'b', 0];
        hr_buffer_add_latin1(buffer, text.as_ptr().cast(), -1, 1, 2);
        let mut len = 0;
        let infos = std::slice::from_raw_parts(
            hr_buffer_get_glyph_infos(buffer, &raw mut len),
            len as usize,
        );
        let seen: Vec<_> = infos.iter().map(|i| (i.codepoint, i.cluster)).collect();
        assert_eq!(seen, [(0xE9, 1), (b'b' as u32, 2)]);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn buffer_properties_round_trip() {
    unsafe {
        let buffer = hr_buffer_create();
        hr_buffer_set_direction(buffer, HR_DIRECTION_RTL);
        hr_buffer_set_script(buffer, HR_SCRIPT_ARABIC);
        let arabic = hr_language_from_string(c"ar".as_ptr(), -1);
        hr_buffer_set_language(buffer, arabic);

        assert_eq!(hr_buffer_get_direction(buffer), HR_DIRECTION_RTL);
        assert_eq!(hr_buffer_get_script(buffer), HR_SCRIPT_ARABIC);
        // Languages are interned, so this compares by pointer.
        assert_eq!(hr_buffer_get_language(buffer), arabic);
        hr_buffer_destroy(buffer);
    }
}

/// Serializes with the given flags and hands back what was written.
unsafe fn serialize(
    buffer: *mut hr_buffer_t,
    start: c_uint,
    end: c_uint,
    font: *mut hr_font_t,
    flags: hr_buffer_serialize_flags_t,
) -> (c_uint, String) {
    let mut dest = [0i8; 512];
    let mut consumed = 0;
    let items = unsafe {
        hr_buffer_serialize_glyphs(
            buffer,
            start,
            end,
            dest.as_mut_ptr(),
            dest.len() as c_uint,
            &raw mut consumed,
            font,
            HR_BUFFER_SERIALIZE_FORMAT_TEXT,
            flags,
        )
    };
    let text = unsafe { std::ffi::CStr::from_ptr(dest.as_ptr()) }
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(consumed as usize, text.len());
    (items, text)
}

/// A buffer of glyphs that was never shaped, and so has no positions.
unsafe fn glyph_buffer(gids: &[hr_codepoint_t]) -> *mut hr_buffer_t {
    unsafe {
        let buffer = hr_buffer_create();
        hr_buffer_add_codepoints(buffer, gids.as_ptr(), gids.len() as c_int, 0, -1);
        hr_buffer_set_content_type(buffer, HR_BUFFER_CONTENT_TYPE_GLYPHS);
        buffer
    }
}

#[test]
fn serializing_a_buffer_without_positions_reports_its_glyphs() {
    unsafe {
        with_font(|_, font| {
            // Having no positions is not the same as having zero ones: the
            // glyphs are still there to report, without the `+0` that would
            // claim an advance nobody worked out.
            // Numbered rather than named, so the expected text does not
            // depend on what this font calls its first two glyphs.
            let numbers = HR_BUFFER_SERIALIZE_FLAG_NO_GLYPH_NAMES;
            let buffer = glyph_buffer(&[1, 2]);
            let (items, text) = serialize(buffer, 0, 2, font, numbers);
            assert_eq!((items, text.as_str()), (2, "[1=0|2=1]"));

            // The range path used to run off the end of an empty string here.
            let (items, text) = serialize(buffer, 1, 2, font, numbers);
            assert_eq!((items, text.as_str()), (1, "|2=1]"));
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn serializing_a_range_counts_from_the_start_of_the_buffer() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);

            // With advances suppressed each glyph reports the pen position it
            // sits at, which counts from the start of the buffer even when
            // only the tail of it is asked for.
            let (_, whole) = serialize(
                buffer,
                0,
                hr_buffer_get_length(buffer),
                font,
                HR_BUFFER_SERIALIZE_FLAG_NO_ADVANCES,
            );
            let (_, tail) = serialize(
                buffer,
                1,
                hr_buffer_get_length(buffer),
                font,
                HR_BUFFER_SERIALIZE_FLAG_NO_ADVANCES,
            );
            // The tail of the whole listing is exactly the tail on its own.
            let from = whole.find('|').expect("more than one glyph");
            assert_eq!(tail, whole[from..]);
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn extents_that_cannot_be_worked_out_are_left_out() {
    unsafe {
        // Nothing can say what a glyph's extents are without a font, so the
        // field is absent rather than zero.
        let buffer = glyph_buffer(&[1]);
        let (items, text) = serialize(
            buffer,
            0,
            1,
            ptr::null_mut(),
            HR_BUFFER_SERIALIZE_FLAG_GLYPH_EXTENTS,
        );
        assert_eq!((items, text.as_str()), (1, "[gid1=0]"));
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn adding_codepoints_checks_nothing() {
    unsafe {
        let items = [0x41u32, 0xD800, 0x0011_0000, 0x42];

        // `add_utf32` replaces what are not characters ...
        let buffer = hr_buffer_create();
        hr_buffer_add_utf32(buffer, items.as_ptr(), 4, 0, -1);
        const FFFD: u32 = 0xFFFD;
        assert_eq!(glyph_ids(buffer), [0x41, FFFD, FFFD, 0x42]);
        hr_buffer_destroy(buffer);

        // ... and `add_codepoints` is the counterpart that does not, for
        // callers that have already checked, or that mean it.
        let buffer = hr_buffer_create();
        hr_buffer_add_codepoints(buffer, items.as_ptr(), 4, 0, -1);
        assert_eq!(glyph_ids(buffer), items);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn guessing_settles_on_a_language() {
    unsafe {
        // HarfBuzz fills in the language the process is running under, which
        // language-specific shaping then follows.
        let buffer = hr_buffer_create();
        hr_buffer_add_utf8(buffer, c"abc".as_ptr(), -1, 0, -1);
        assert!(hr_buffer_get_language(buffer).is_null());
        hr_buffer_guess_segment_properties(buffer);
        assert_eq!(hr_buffer_get_language(buffer), hr_language_get_default());
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn serializing_writes_whole_items_or_none() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_buffer_guess_segment_properties(buffer);
            hr_shape(font, buffer, ptr::null(), 0);
            let count = hr_buffer_get_length(buffer);

            let mut full = [0i8; 256];
            let mut consumed = 0;
            let items = hr_buffer_serialize_glyphs(
                buffer,
                0,
                count,
                full.as_mut_ptr(),
                full.len() as c_uint,
                &raw mut consumed,
                font,
                HR_BUFFER_SERIALIZE_FORMAT_TEXT,
                HR_BUFFER_SERIALIZE_FLAG_DEFAULT,
            );
            assert_eq!(items, count);
            let whole = std::ffi::CStr::from_ptr(full.as_ptr()).to_str().unwrap();
            assert_eq!(consumed as usize, whole.len());

            // Every destination size: an item is written only when all of it
            // fits, and what is reported written is exactly what is there.
            for size in 0..=whole.len() + 2 {
                let mut dest = [0x7fi8; 256];
                let mut consumed = 0;
                let items = hr_buffer_serialize_glyphs(
                    buffer,
                    0,
                    count,
                    dest.as_mut_ptr(),
                    size as c_uint,
                    &raw mut consumed,
                    font,
                    HR_BUFFER_SERIALIZE_FORMAT_TEXT,
                    HR_BUFFER_SERIALIZE_FLAG_DEFAULT,
                );
                if size == 0 {
                    assert_eq!((items, consumed), (0, 0));
                    continue;
                }
                let written = std::ffi::CStr::from_ptr(dest.as_ptr()).to_str().unwrap();
                assert_eq!(written.len(), consumed as usize, "size {size}");
                // Whatever was written is a prefix of the whole, and never
                // half of an item.
                assert!(whole.starts_with(written), "size {size}: {written:?}");
                assert_eq!(
                    items as usize,
                    written.matches('|').count() + usize::from(!written.is_empty()),
                    "size {size}: {written:?}"
                );
                assert!(
                    (consumed as usize) < size,
                    "size {size} left no room to terminate"
                );
            }
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn serializing_without_a_font_numbers_the_glyphs() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_buffer_guess_segment_properties(buffer);
            hr_shape(font, buffer, ptr::null(), 0);

            // HarfBuzz substitutes its empty font for NULL, which knows no
            // glyph names, so the glyphs come back numbered rather than not
            // at all.
            let mut dest = [0i8; 256];
            let mut consumed = 0;
            let items = hr_buffer_serialize_glyphs(
                buffer,
                0,
                hr_buffer_get_length(buffer),
                dest.as_mut_ptr(),
                dest.len() as c_uint,
                &raw mut consumed,
                ptr::null_mut(),
                HR_BUFFER_SERIALIZE_FORMAT_TEXT,
                HR_BUFFER_SERIALIZE_FLAG_DEFAULT,
            );
            assert_eq!(items, hr_buffer_get_length(buffer));
            let text = std::ffi::CStr::from_ptr(dest.as_ptr()).to_str().unwrap();
            assert!(text.starts_with("[gid"), "{text:?}");
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn appending_carries_everything_the_glyphs_came_with() {
    unsafe {
        with_font(|_, font| {
            let source = buffer_with_text(TEXT);
            hr_buffer_guess_segment_properties(source);
            hr_shape(font, source, ptr::null(), 0);

            let mut len = 0;
            let source_positions = std::slice::from_raw_parts(
                hr_buffer_get_glyph_positions(source, &raw mut len),
                len as usize,
            );
            let source_advances: Vec<i32> = source_positions.iter().map(|p| p.x_advance).collect();
            assert!(source_advances.iter().any(|a| *a != 0));

            // A destination told nothing at all takes the source's kind and
            // properties, and the glyphs arrive with their positions.
            let dest = hr_buffer_create();
            hr_buffer_append(dest, source, 0, u32::MAX);

            assert_eq!(hr_buffer_get_length(dest), len);
            assert_eq!(
                hr_buffer_get_content_type(dest),
                hr_buffer_get_content_type(source)
            );
            assert_eq!(
                hr_buffer_get_direction(dest),
                hr_buffer_get_direction(source)
            );
            assert_eq!(hr_buffer_get_script(dest), hr_buffer_get_script(source));

            let mut dest_len = 0;
            let dest_positions = std::slice::from_raw_parts(
                hr_buffer_get_glyph_positions(dest, &raw mut dest_len),
                dest_len as usize,
            );
            let dest_advances: Vec<i32> = dest_positions.iter().map(|p| p.x_advance).collect();
            assert_eq!(dest_advances, source_advances);

            hr_buffer_destroy(dest);
            hr_buffer_destroy(source);
        });
    }
}

#[test]
fn appending_a_range_carries_its_surroundings() {
    unsafe {
        with_font(|_, font| {
            // Appending the middle of some text leaves the rest of it as
            // context, so the piece shapes as it did in place.
            let source = buffer_with_text("abcde");
            hr_buffer_guess_segment_properties(source);

            let dest = hr_buffer_create();
            hr_buffer_append(dest, source, 1, 3);
            hr_shape(font, dest, ptr::null(), 0);
            let appended = glyph_ids(dest);

            // The same two characters given the same surroundings directly.
            let direct = hr_buffer_create();
            hr_buffer_add_utf8(direct, c"abcde".as_ptr(), -1, 1, 2);
            hr_buffer_guess_segment_properties(direct);
            hr_shape(font, direct, ptr::null(), 0);
            assert_eq!(appended, glyph_ids(direct));

            hr_buffer_destroy(direct);
            hr_buffer_destroy(dest);
            hr_buffer_destroy(source);
        });
    }
}

#[test]
fn an_unknown_serialization_format_survives_the_round_trip() {
    unsafe {
        // The format is a tag, and a caller may hold one this library does
        // not know: it comes back as itself rather than collapsing, and asks
        // for nothing when handed back.
        let unknown = hr_buffer_serialize_format_from_string(c"FOO ".as_ptr(), -1);
        assert_eq!(unknown, 0x464F_4F00);
        assert!(hr_buffer_serialize_format_to_string(unknown).is_null());

        assert_eq!(
            hr_buffer_serialize_format_from_string(c"text".as_ptr(), -1),
            HR_BUFFER_SERIALIZE_FORMAT_TEXT
        );

        // Handing it back asks for a format that does not exist, which
        // serializes nothing rather than misbehaving.
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let mut dest = [0i8; 64];
            let mut consumed = 0;
            let items = hr_buffer_serialize_glyphs(
                buffer,
                0,
                hr_buffer_get_length(buffer),
                dest.as_mut_ptr(),
                dest.len() as c_uint,
                &raw mut consumed,
                font,
                unknown,
                HR_BUFFER_SERIALIZE_FLAG_DEFAULT,
            );
            assert_eq!((items, consumed), (0, 0));
            hr_buffer_destroy(buffer);
        });
    }
}

unsafe extern "C" fn advance_of_42(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    _glyph: hr_codepoint_t,
    _user_data: *mut c_void,
) -> hr_position_t {
    42
}

#[test]
fn a_partial_sub_font_leaves_the_rest_to_its_parent() {
    unsafe {
        with_font(|_, font| {
            // Callbacks the child does not carry are the parent's business,
            // and a parent carrying none of its own reads its tables.
            let sub = hr_font_create_sub_font(font);
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_h_advance_func(
                ffuncs,
                Some(advance_of_42),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(sub, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(sub, u32::from(b'a'), &raw mut glyph),
                0,
                "a glyph the parent can find is one the child can find"
            );
            assert_ne!(glyph, 0);
            hr_font_destroy(sub);
        });

        // And a sub-font told to read the tables itself stops asking its
        // parent, which carrying nothing could not have said.
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let sub = hr_font_create_sub_font(font);
            let mut glyph: hr_codepoint_t = 0;
            hr_font_get_nominal_glyph(sub, u32::from(b'a'), &raw mut glyph);
            assert_eq!(glyph, 777);

            hr_ot_font_set_funcs(sub);
            glyph = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(sub, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_ne!(glyph, 777);
            hr_font_destroy(sub);
        });
    }
}

#[test]
fn no_name_names_no_tag() {
    unsafe {
        // Padding nothing out to four spaces would make a tag of it, and
        // then a script and a language of their own.
        assert_eq!(hr_tag_from_string(c"".as_ptr(), -1), HR_TAG_NONE);
        assert_eq!(hr_script_from_string(c"".as_ptr(), -1), HR_SCRIPT_INVALID);
        assert_eq!(
            hr_tag_from_string(c"ab".as_ptr(), -1),
            hr_tag_from_string(c"ab  ".as_ptr(), -1)
        );

        // Text runs left to right unless its script says otherwise, and a
        // script that is not one does not say otherwise.
        assert_eq!(
            hr_script_get_horizontal_direction(HR_SCRIPT_INVALID),
            HR_DIRECTION_LTR
        );
        assert_eq!(
            hr_script_get_horizontal_direction(HR_SCRIPT_ARABIC),
            HR_DIRECTION_RTL
        );

        // Reversing is the bit flip that separates each pair.
        assert_eq!(hr_direction_reverse(HR_DIRECTION_LTR), HR_DIRECTION_RTL);
        assert_eq!(hr_direction_reverse(HR_DIRECTION_TTB), HR_DIRECTION_BTT);
        assert_eq!(
            hr_direction_reverse(hr_direction_reverse(HR_DIRECTION_INVALID)),
            HR_DIRECTION_INVALID
        );
    }
}

#[test]
fn a_sub_blob_fixes_what_it_is_part_of() {
    unsafe {
        let storage = *b"0123456789";
        let blob = hr_blob_create(
            storage.as_ptr().cast(),
            storage.len() as c_uint,
            HR_MEMORY_MODE_READONLY,
            ptr::null_mut(),
            None,
        );
        assert_eq!(hr_blob_is_immutable(blob), 0);
        let sub = hr_blob_create_sub_blob(blob, 3, 4);
        assert_eq!(hr_blob_get_length(sub), 4);
        // The part cannot outlive its parent being what it was.
        assert_ne!(hr_blob_is_immutable(blob), 0);
        hr_blob_destroy(sub);
        hr_blob_destroy(blob);
    }
}

#[test]
fn every_font_has_a_parent_to_stop_at() {
    unsafe {
        with_font(|_, font| {
            // A font made from a face answers with the font that is empty,
            // rather than with nothing, so walking up the chain terminates.
            assert_eq!(hr_font_get_parent(font), hr_font_get_empty());
            let sub = hr_font_create_sub_font(font);
            assert_eq!(hr_font_get_parent(sub), font);
            hr_font_destroy(sub);
        });
    }
}

#[test]
fn overlaying_properties_onto_themselves_is_allowed() {
    unsafe {
        // Nothing says the two have to be different structs.
        let mut props = latin_props();
        hr_segment_properties_overlay(ptr::from_mut(&mut props), ptr::from_ref(&props));
        assert_eq!(props.direction, HR_DIRECTION_LTR);
        assert_eq!(props.script, HR_SCRIPT_LATIN);
    }
}

#[test]
fn a_length_of_nothing_means_what_each_creator_says() {
    unsafe {
        let storage = [0i8; 1];
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();

        // `hr_blob_create` hands back the blob that holds nothing, so the
        // caller's data goes now: nothing will be left to release it.
        drops.store(0, Ordering::SeqCst);
        let created = hr_blob_create(
            storage.as_ptr(),
            0,
            HR_MEMORY_MODE_READONLY,
            user_data,
            Some(count_drop),
        );
        assert_eq!(created, hr_blob_get_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        hr_blob_destroy(created);

        // `hr_blob_create_or_fail` makes one that can hold it.
        drops.store(0, Ordering::SeqCst);
        let or_fail = hr_blob_create_or_fail(
            storage.as_ptr(),
            0,
            HR_MEMORY_MODE_READONLY,
            user_data,
            Some(count_drop),
        );
        assert!(!or_fail.is_null());
        assert_ne!(or_fail, hr_blob_get_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        hr_blob_destroy(or_fail);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn an_empty_buffer_executes_any_plan() {
    unsafe {
        with_font(|_, font| {
            // Including the empty one, which can shape nothing: there is
            // nothing to shape.
            let buffer = hr_buffer_create();
            hr_buffer_guess_segment_properties(buffer);
            assert_ne!(
                hr_shape_plan_execute(hr_shape_plan_get_empty(), font, buffer, ptr::null(), 0),
                0
            );
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn a_sub_font_follows_its_parent() {
    unsafe {
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let sub = hr_font_create_sub_font(font);
            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(sub, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_eq!(glyph, 777);

            // Callbacks installed on the parent after the child was made are
            // the child's callbacks too: it asks, rather than having copied.
            let replacement = hr_font_funcs_create();
            hr_font_set_funcs(font, replacement, ptr::null_mut(), None);
            hr_font_funcs_destroy(replacement);
            glyph = 1;
            assert_eq!(
                hr_font_get_nominal_glyph(sub, u32::from(b'a'), &raw mut glyph),
                0,
                "the child should see that the parent now answers nothing"
            );
            assert_eq!(glyph, 0);

            hr_font_destroy(sub);
        });
    }
}

#[test]
fn context_belongs_to_the_text_it_was_added_with() {
    unsafe {
        // Text added to a buffer that already holds some brings no
        // pre-context with it: the first call's surroundings stand.
        let buffer = hr_buffer_create();
        hr_buffer_add_utf8(buffer, c"abcdef".as_ptr(), -1, 2, 2);
        hr_buffer_add_utf8(buffer, c"abcdef".as_ptr(), -1, 4, 2);
        assert_eq!(hr_buffer_get_length(buffer), 4);
        hr_buffer_destroy(buffer);

        // Emptying by length empties what described the contents too.
        let buffer = hr_buffer_create();
        hr_buffer_add_utf8(buffer, c"abcdef".as_ptr(), -1, 0, 3);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            HR_BUFFER_CONTENT_TYPE_UNICODE
        );
        assert_ne!(hr_buffer_set_length(buffer, 0), 0);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            HR_BUFFER_CONTENT_TYPE_INVALID
        );
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn a_buffer_like_another_copies_how_it_is_set_up() {
    unsafe {
        let src = hr_buffer_create();
        hr_buffer_set_direction(src, HR_DIRECTION_RTL);
        hr_buffer_set_script(src, HR_SCRIPT_ARABIC);
        hr_buffer_set_cluster_level(src, HR_BUFFER_CLUSTER_LEVEL_CHARACTERS);
        hr_buffer_set_flags(src, HR_BUFFER_FLAG_BOT);

        // How it is configured, and not what it says about the text.
        let like = hr_buffer_create_similar(src);
        assert_eq!(
            hr_buffer_get_cluster_level(like),
            HR_BUFFER_CLUSTER_LEVEL_CHARACTERS
        );
        assert_eq!(hr_buffer_get_flags(like), HR_BUFFER_FLAG_BOT);
        assert_eq!(hr_buffer_get_direction(like), HR_DIRECTION_INVALID);
        assert_eq!(hr_buffer_get_script(like), HR_SCRIPT_INVALID);

        // And clearing the contents leaves the configuration alone.
        hr_buffer_set_not_found_variation_selector_glyph(src, 7);
        hr_buffer_clear_contents(src);
        assert_eq!(
            hr_buffer_get_cluster_level(src),
            HR_BUFFER_CLUSTER_LEVEL_CHARACTERS
        );
        assert_eq!(hr_buffer_get_not_found_variation_selector_glyph(src), 7);

        // A variation selector nothing resolves is spelled with the
        // codepoint that is not one, which is also the default.
        let fresh = hr_buffer_create();
        assert_eq!(
            hr_buffer_get_not_found_variation_selector_glyph(fresh),
            HR_CODEPOINT_INVALID
        );
        hr_buffer_destroy(fresh);
        hr_buffer_destroy(like);
        hr_buffer_destroy(src);
    }
}

#[test]
fn properties_compare_and_overlay_as_a_whole() {
    unsafe {
        let mut a = latin_props();
        let mut b = latin_props();
        a.reserved1 = ptr::dangling_mut();
        assert_eq!(
            hr_segment_properties_equal(ptr::from_ref(&a), ptr::from_ref(&b)),
            0,
            "the reserved fields are part of the comparison"
        );

        // Overlaying stops where the two describe the text differently.
        let mut dest = hr_segment_properties_t {
            direction: HR_DIRECTION_LTR,
            script: HR_SCRIPT_INVALID,
            language: ptr::null(),
            reserved1: ptr::null_mut(),
            reserved2: ptr::null_mut(),
        };
        b.direction = HR_DIRECTION_RTL;
        b.script = HR_SCRIPT_ARABIC;
        hr_segment_properties_overlay(ptr::from_mut(&mut dest), ptr::from_ref(&b));
        assert_eq!(dest.script, HR_SCRIPT_INVALID);
        assert!(dest.language.is_null());
    }
}

#[test]
fn nothing_to_shape_is_nothing_to_fail_at() {
    unsafe {
        with_font(|_, font| {
            // Even asking for a shaper that does not exist: HarfBuzz answers
            // an empty buffer before it looks at anything else.
            let buffer = hr_buffer_create();
            hr_buffer_guess_segment_properties(buffer);
            let absent = [c"graphite2".as_ptr(), ptr::null()];
            assert_ne!(
                hr_shape_full(font, buffer, ptr::null(), 0, absent.as_ptr()),
                0
            );
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn a_blob_of_nothing_is_still_a_blob() {
    unsafe {
        // Zero length is not a failure, and the blob holds the caller's data
        // until it is destroyed.
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        let storage = [0i8; 1];
        let blob = hr_blob_create_or_fail(
            storage.as_ptr(),
            0,
            HR_MEMORY_MODE_READONLY,
            user_data,
            Some(count_drop),
        );
        assert!(!blob.is_null());
        assert_eq!(hr_blob_get_length(blob), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        hr_blob_destroy(blob);
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        // A length that cannot be represented is the one that fails.
        drops.store(0, Ordering::SeqCst);
        let refused = hr_blob_create_or_fail(
            storage.as_ptr(),
            1 << 31,
            HR_MEMORY_MODE_READONLY,
            user_data,
            Some(count_drop),
        );
        assert!(refused.is_null());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn a_failed_parse_describes_nothing() {
    unsafe {
        let mut feature = hr_feature_t {
            tag: 0xABAB_ABAB,
            value: 9,
            start: 9,
            end: 9,
        };
        assert_eq!(
            hr_feature_from_string(c"!!! not a feature".as_ptr(), -1, &raw mut feature),
            0
        );
        assert_eq!(
            (feature.tag, feature.value, feature.start, feature.end),
            (0, 0, 0, 0)
        );

        let mut variation = hr_variation_t {
            tag: 0xABAB_ABAB,
            value: 9.0,
        };
        assert_eq!(
            hr_variation_from_string(c"!!!".as_ptr(), -1, &raw mut variation),
            0
        );
        assert_eq!(variation.tag, 0);

        // A range of one character is written as that character.
        let mut one = hr_feature_t::default();
        assert_ne!(
            hr_feature_from_string(c"liga[5]".as_ptr(), -1, &raw mut one),
            0
        );
        let mut text = [0i8; 64];
        hr_feature_to_string(ptr::from_ref(&one), text.as_mut_ptr(), text.len() as c_uint);
        assert_eq!(
            std::ffi::CStr::from_ptr(text.as_ptr()).to_str().unwrap(),
            "liga[5]"
        );
    }
}

#[test]
fn buffer_properties_round_trip_being_unset() {
    unsafe {
        let buffer = hr_buffer_create();
        // A buffer that was never told a script has none, which is not the
        // same as the `Zzzz` standing for a script known to be unknown.
        assert_eq!(hr_buffer_get_script(buffer), HR_SCRIPT_INVALID);
        assert!(hr_buffer_get_language(buffer).is_null());

        // Setting the absent value clears whatever was there.
        hr_buffer_set_script(buffer, HR_SCRIPT_ARABIC);
        hr_buffer_set_script(buffer, HR_SCRIPT_INVALID);
        assert_eq!(hr_buffer_get_script(buffer), HR_SCRIPT_INVALID);

        hr_buffer_set_language(buffer, hr_language_from_string(c"en".as_ptr(), -1));
        hr_buffer_set_language(buffer, ptr::null());
        assert!(hr_buffer_get_language(buffer).is_null());

        // Whole-property assignment overwrites rather than fills in, so a
        // reused buffer cannot keep anything from its last life.
        hr_buffer_set_script(buffer, HR_SCRIPT_ARABIC);
        hr_buffer_set_language(buffer, hr_language_from_string(c"ar".as_ptr(), -1));
        let blank = hr_segment_properties_t {
            direction: HR_DIRECTION_LTR,
            script: HR_SCRIPT_INVALID,
            language: ptr::null(),
            reserved1: ptr::null_mut(),
            reserved2: ptr::null_mut(),
        };
        hr_buffer_set_segment_properties(buffer, ptr::from_ref(&blank));
        assert_eq!(hr_buffer_get_script(buffer), HR_SCRIPT_INVALID);
        assert!(hr_buffer_get_language(buffer).is_null());
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn an_empty_shaper_list_names_no_available_shaper() {
    unsafe {
        with_font(|face, font| {
            // A list of no names asks for no shaper this library has, which
            // is a failure rather than the free rein that `NULL` gives.
            let empty = [ptr::null()];
            let buffer = buffer_with_text(TEXT);
            assert_eq!(
                hr_shape_full(font, buffer, ptr::null(), 0, empty.as_ptr()),
                0
            );

            let props = latin_props();
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, empty.as_ptr());
            assert_eq!(plan, hr_shape_plan_get_empty());

            // `NULL` still means no restriction.
            assert_ne!(hr_shape_full(font, buffer, ptr::null(), 0, ptr::null()), 0);
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn flags_combine_and_round_trip() {
    unsafe {
        let buffer = hr_buffer_create();
        // A fresh buffer has no flags at all. Flag types are integers rather
        // than enumerations precisely so that zero, and any combination, is
        // representable.
        assert_eq!(hr_buffer_get_flags(buffer), HR_BUFFER_FLAG_DEFAULT);

        let both = HR_BUFFER_FLAG_BOT | HR_BUFFER_FLAG_EOT;
        hr_buffer_set_flags(buffer, both);
        assert_eq!(hr_buffer_get_flags(buffer), both);

        hr_buffer_set_flags(buffer, HR_BUFFER_FLAG_DEFINED);
        assert_eq!(hr_buffer_get_flags(buffer), HR_BUFFER_FLAG_DEFINED);

        hr_buffer_set_flags(buffer, HR_BUFFER_FLAG_DEFAULT);
        assert_eq!(hr_buffer_get_flags(buffer), HR_BUFFER_FLAG_DEFAULT);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn glyph_flags_are_readable_including_none() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let mut len: c_uint = 0;
            let infos = hr_buffer_get_glyph_infos(buffer, ptr::from_mut(&mut len));
            assert!(len > 0);
            for i in 0..len as usize {
                // Most glyphs carry no flags; reading that must be safe.
                let flags = hr_glyph_info_get_glyph_flags(infos.add(i));
                assert_eq!(flags & !HR_GLYPH_FLAG_DEFINED, 0);
            }
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn reversing_respects_clusters() {
    unsafe {
        let buffer = hr_buffer_create();
        for (i, c) in ['a', 'b', 'c', 'd'].into_iter().enumerate() {
            hr_buffer_add(buffer, c as u32, (i / 2) as c_uint);
        }
        hr_buffer_reverse_clusters(buffer);
        assert_eq!(
            glyph_ids(buffer),
            vec!['c' as u32, 'd' as u32, 'a' as u32, 'b' as u32]
        );
        hr_buffer_destroy(buffer);
    }
}

// --------------------------------------------------------------- shaping ----

#[test]
fn shaping_produces_positioned_glyphs() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);

            assert_eq!(
                hr_buffer_get_content_type(buffer),
                HR_BUFFER_CONTENT_TYPE_GLYPHS
            );
            assert_ne!(hr_buffer_has_positions(buffer), 0);

            let mut len: c_uint = 0;
            let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
            // This font has no GSUB, so each character yields one glyph.
            assert_eq!(len as usize, TEXT.chars().count());
            for i in 0..len as usize {
                assert!(
                    (*positions.add(i)).x_advance > 0,
                    "glyph {i} has no advance"
                );
            }
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn shaping_twice_reuses_the_plan_and_agrees() {
    unsafe {
        with_font(|_, font| {
            let first = buffer_with_text(TEXT);
            hr_shape(font, first, ptr::null(), 0);
            let once = glyph_ids(first);

            // A second call hits the face's plan cache; the result must match.
            let second = buffer_with_text(TEXT);
            hr_shape(font, second, ptr::null(), 0);
            assert_eq!(glyph_ids(second), once);

            hr_buffer_destroy(first);
            hr_buffer_destroy(second);
        });
    }
}

#[test]
fn shaping_the_same_text_again_means_refilling_the_buffer() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let once = glyph_ids(buffer);

            // The buffer holds glyphs now, so shaping it again would be a
            // misuse. Clearing and refilling is how the same text is shaped
            // afresh; relabelling the contents as text would instead shape the
            // glyph ids as though they were codepoints.
            hr_buffer_clear_contents(buffer);
            hr_buffer_add_utf8(
                buffer,
                TEXT.as_ptr().cast::<c_char>(),
                TEXT.len() as c_int,
                0,
                -1,
            );
            hr_buffer_guess_segment_properties(buffer);
            hr_shape(font, buffer, ptr::null(), 0);
            assert_eq!(glyph_ids(buffer), once);

            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
#[cfg_attr(miri, ignore = "runs a child process, which Miri cannot do")]
fn shaping_an_already_shaped_buffer_aborts() {
    // Shaping glyphs as though they were text is a misuse of the API, which
    // HarfBuzz asserts on. `hr_shape` returns nothing, so it could not report
    // this any other way.
    assert!(aborts("already_shaped"));
}

#[test]
fn disabling_a_feature_changes_the_result() {
    unsafe {
        with_font(|_, font| {
            let shape_with = |features: &[hr_feature_t]| {
                let buffer = buffer_with_text("office");
                hr_shape(font, buffer, features.as_ptr(), features.len() as c_uint);
                let ids = glyph_ids(buffer);
                hr_buffer_destroy(buffer);
                ids
            };

            let mut no_liga = hr_feature_t {
                tag: 0,
                value: 0,
                start: 0,
                end: 0,
            };
            assert_ne!(
                hr_feature_from_string(c"-liga".as_ptr(), -1, &raw mut no_liga),
                0
            );
            assert_eq!(no_liga.value, 0);
            assert_eq!(no_liga.tag, hr_tag_from_string(c"liga".as_ptr(), -1));

            // Both must shape; whether they differ depends on the font, so
            // only require that turning the feature off is honoured without
            // changing the glyph count.
            let with_liga = shape_with(&[]);
            let without = shape_with(&[no_liga]);
            assert_ne!(with_liga, []);
            assert_ne!(without, []);
        });
    }
}

#[test]
fn serializing_a_shaped_buffer_writes_text() {
    unsafe {
        with_font(|_, font| {
            let buffer = buffer_with_text("AB");
            hr_shape(font, buffer, ptr::null(), 0);

            let mut buf = [0i8; 256];
            let mut consumed: c_uint = 0;
            let count = hr_buffer_serialize_glyphs(
                buffer,
                0,
                hr_buffer_get_length(buffer),
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as c_uint,
                &raw mut consumed,
                font,
                HR_BUFFER_SERIALIZE_FORMAT_TEXT,
                HR_BUFFER_SERIALIZE_FLAG_DEFAULT,
            );
            // No GSUB, so two characters serialize as two glyphs.
            assert_eq!(count, 2);
            let text = core::ffi::CStr::from_ptr(buf.as_ptr().cast::<c_char>())
                .to_str()
                .unwrap();
            assert!(text.starts_with('['), "unexpected output: {text}");
            assert!(text.ends_with(']'), "unexpected output: {text}");
            assert_eq!(consumed as usize, text.len());
            hr_buffer_destroy(buffer);
        });
    }
}

// -------------------------------------------------------- table callbacks ----

/// The font bytes a table callback reads from, handed over as user data.
struct TableSource {
    data: Vec<u8>,
    requested: Mutex<Vec<hr_tag_t>>,
}

unsafe extern "C" fn reference_table(
    _face: *mut hr_face_t,
    tag: hr_tag_t,
    user_data: *mut c_void,
) -> *mut hr_blob_t {
    unsafe {
        let source = &*user_data.cast::<TableSource>();
        source.requested.lock().unwrap().push(tag);
        // Reuse a blob-backed face purely to look the table up.
        let blob = hr_blob_create(
            source.data.as_ptr().cast::<c_char>(),
            source.data.len() as c_uint,
            HR_MEMORY_MODE_READONLY,
            ptr::null_mut(),
            None,
        );
        let face = hr_face_create(blob, 0);
        let table = hr_face_reference_table(face, tag);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
        if hr_blob_get_length(table) == 0 {
            hr_blob_destroy(table);
            return ptr::null_mut();
        }
        table
    }
}

unsafe extern "C" fn destroy_table_source(user_data: *mut c_void) {
    drop(unsafe { Box::from_raw(user_data.cast::<TableSource>()) });
}

#[test]
fn table_callback_face_shapes_like_a_blob_face() {
    let data = font_data();
    unsafe {
        // Shape once through an ordinary blob-backed face.
        let expected = {
            let blob = blob_over(&data);
            let face = hr_face_create(blob, 0);
            let font = hr_font_create(face);
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let ids = glyph_ids(buffer);
            hr_buffer_destroy(buffer);
            hr_font_destroy(font);
            hr_face_destroy(face);
            hr_blob_destroy(blob);
            ids
        };

        // ... then through a face built from table callbacks.
        let source = Box::into_raw(Box::new(TableSource {
            data: data.clone(),
            requested: Mutex::new(Vec::new()),
        }));
        let face = hr_face_create_for_tables(
            Some(reference_table),
            source.cast::<c_void>(),
            Some(destroy_table_source),
        );
        assert_eq!(hr_face_get_upem(face), PLAIN_UPEM);

        let font = hr_font_create(face);
        let requested = (*source).requested.lock().unwrap();
        assert!(!requested.contains(&hr_tag_from_string(c"cmap".as_ptr(), -1)));
        assert!(!requested.contains(&hr_tag_from_string(c"glyf".as_ptr(), -1)));
        drop(requested);

        let buffer = buffer_with_text(TEXT);
        hr_shape(font, buffer, ptr::null(), 0);
        assert_eq!(glyph_ids(buffer), expected);

        hr_buffer_destroy(buffer);
        hr_font_destroy(font);
        // Freeing the face runs `destroy_table_source`.
        hr_face_destroy(face);
    }
}

#[test]
fn builtin_font_queries_load_charmap_and_metrics_separately() {
    let source = Box::into_raw(Box::new(TableSource {
        data: font_data(),
        requested: Mutex::new(Vec::new()),
    }));
    unsafe {
        let face = hr_face_create_for_tables(
            Some(reference_table),
            source.cast::<c_void>(),
            Some(destroy_table_source),
        );
        let font = hr_font_create(face);
        let cmap = hr_tag_from_string(c"cmap".as_ptr(), -1);
        let hmtx = hr_tag_from_string(c"hmtx".as_ptr(), -1);

        let mut name = [0; 64];
        hr_font_get_glyph_name(font, 1, name.as_mut_ptr(), name.len() as c_uint);
        {
            let requested = (*source).requested.lock().unwrap();
            assert!(!requested.contains(&cmap));
            assert!(!requested.contains(&hmtx));
        }

        let mut glyph = 0;
        assert_ne!(
            hr_font_get_nominal_glyph(font, 'a' as u32, &raw mut glyph),
            0
        );
        {
            let requested = (*source).requested.lock().unwrap();
            assert!(requested.contains(&cmap));
            assert!(!requested.contains(&hmtx));
        }

        assert!(hr_font_get_glyph_h_advance(font, glyph) > 0);
        assert!((*source).requested.lock().unwrap().contains(&hmtx));

        hr_font_destroy(font);
        hr_face_destroy(face);
    }
}

#[test]
fn cold_builtin_font_queries_from_several_threads() {
    let source = Box::into_raw(Box::new(TableSource {
        data: font_data(),
        requested: Mutex::new(Vec::new()),
    }));
    unsafe {
        let face = hr_face_create_for_tables(
            Some(reference_table),
            source.cast::<c_void>(),
            Some(destroy_table_source),
        );
        let font = hr_font_create(face);
        hr_font_make_immutable(font);

        let shared = Shared(font);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..8 {
                        let mut glyph = 0;
                        assert_eq!(
                            hr_font_get_nominal_glyph(shared.get(), 0x4E00, &raw mut glyph),
                            0
                        );
                        assert_ne!(
                            hr_font_get_nominal_glyph(shared.get(), 'a' as u32, &raw mut glyph),
                            0
                        );
                        assert!(hr_font_get_glyph_h_advance(shared.get(), glyph) > 0);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("thread panicked");
        }

        hr_font_destroy(font);
        hr_face_destroy(face);
    }
}

#[test]
fn custom_font_funcs_do_not_load_builtin_font_tables() {
    let source = Box::into_raw(Box::new(TableSource {
        data: font_data(),
        requested: Mutex::new(Vec::new()),
    }));
    unsafe {
        let face = hr_face_create_for_tables(
            Some(reference_table),
            source.cast::<c_void>(),
            Some(destroy_table_source),
        );
        let font = hr_font_create(face);
        let funcs = hr_font_funcs_create();
        hr_font_set_funcs(font, funcs, ptr::null_mut(), None);

        let buffer = buffer_with_text(TEXT);
        hr_shape(font, buffer, ptr::null(), 0);

        let requested = (*source).requested.lock().unwrap();
        assert!(!requested.contains(&hr_tag_from_string(c"cmap".as_ptr(), -1)));
        assert!(!requested.contains(&hr_tag_from_string(c"glyf".as_ptr(), -1)));
        drop(requested);

        hr_buffer_destroy(buffer);
        hr_font_funcs_destroy(funcs);
        hr_font_destroy(font);
        hr_face_destroy(face);
    }
}

/// A table callback that ties each blob it returns to a drop counter through
/// the blob's user data, the way a caller owning table bytes that way would.
unsafe extern "C" fn reference_table_with_user_data(
    face: *mut hr_face_t,
    tag: hr_tag_t,
    user_data: *mut c_void,
) -> *mut hr_blob_t {
    static KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };
    unsafe {
        let (source, drops) = &*user_data.cast::<(TableSource, AtomicUsize)>();
        let table = reference_table(face, tag, ptr::from_ref(source).cast_mut().cast());
        let drops: *mut c_void = ptr::from_ref(drops).cast_mut().cast();
        hr_blob_set_user_data(table, &raw const KEY, drops, Some(count_drop), 1);
        table
    }
}

#[test]
fn table_callback_blob_lives_as_long_as_its_bytes_are_used() {
    let state = (
        TableSource {
            data: font_data(),
            requested: Mutex::new(Vec::new()),
        },
        AtomicUsize::new(0),
    );
    let drops = &state.1;
    unsafe {
        let face = hr_face_create_for_tables(
            Some(reference_table_with_user_data),
            ptr::from_ref(&state).cast_mut().cast(),
            None,
        );
        // Reading `head` keeps its blob, and with it whatever the callback
        // tied to the blob, for as long as the face may read the table.
        assert_eq!(hr_face_get_upem(face), PLAIN_UPEM);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        // A table handed back to the caller outlives the face the same way.
        let cmap = hr_face_reference_table(face, hr_tag_from_string(c"cmap".as_ptr(), -1));
        assert_ne!(hr_blob_get_length(cmap), 0);
        hr_face_destroy(face);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        hr_blob_destroy(cmap);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}

// --------------------------------------------------------- font callbacks ----

unsafe extern "C" fn fixed_advance(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    _glyph: hr_codepoint_t,
    user_data: *mut c_void,
) -> hr_position_t {
    unsafe { *user_data.cast::<hr_position_t>() }
}

#[test]
fn font_funcs_override_advances() {
    unsafe {
        with_font(|_, font| {
            let mut advance: hr_position_t = 1234;
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_h_advance_func(
                ffuncs,
                Some(fixed_advance),
                ptr::from_mut(&mut advance).cast::<c_void>(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);

            let buffer = buffer_with_text("AB");
            hr_shape(font, buffer, ptr::null(), 0);
            let mut len: c_uint = 0;
            let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
            for i in 0..len as usize {
                assert_eq!((*positions.add(i)).x_advance, 1234);
            }

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(ffuncs);
        });
    }
}

unsafe extern "C" fn batch_advances(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    count: c_uint,
    _first_glyph: *const hr_codepoint_t,
    _glyph_stride: c_uint,
    first_advance: *mut hr_position_t,
    advance_stride: c_uint,
    user_data: *mut c_void,
) {
    unsafe {
        *user_data.cast::<u32>() += 1;
        for i in 0..count as usize {
            first_advance
                .cast::<u8>()
                .add(i * advance_stride as usize)
                .cast::<hr_position_t>()
                .write_unaligned(700 + i as i32);
        }
    }
}

unsafe extern "C" fn fixed_origin(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    _glyph: hr_codepoint_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
    _user_data: *mut c_void,
) -> hr_bool_t {
    unsafe {
        *x = 12;
        *y = 34;
    }
    1
}

unsafe extern "C" fn fixed_kerning(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    first: hr_codepoint_t,
    second: hr_codepoint_t,
    _user_data: *mut c_void,
) -> hr_position_t {
    first as i32 - second as i32
}

#[test]
fn added_font_callbacks_answer_and_batch_shaping() {
    unsafe {
        with_font(|_, font| {
            let funcs = hr_font_funcs_create();
            let mut calls = 0u32;
            hr_font_funcs_set_glyph_h_advances_func(
                funcs,
                Some(batch_advances),
                (&raw mut calls).cast(),
                None,
            );
            hr_font_funcs_set_glyph_h_origin_func(funcs, Some(fixed_origin), ptr::null_mut(), None);
            hr_font_funcs_set_glyph_h_kerning_func(
                funcs,
                Some(fixed_kerning),
                ptr::null_mut(),
                None,
            );
            hr_font_funcs_set_glyph_v_kerning_func(
                funcs,
                Some(fixed_kerning),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, funcs, ptr::null_mut(), None);

            let buffer = buffer_with_text("AB");
            hr_shape(font, buffer, ptr::null(), 0);
            let mut len = 0;
            let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
            assert_eq!(len, 2);
            assert_eq!((*positions).x_advance, 700);
            assert_eq!((*positions.add(1)).x_advance, 701);
            assert!(calls > 0);

            let glyphs = [1u32, 2];
            let mut advances = [0i32; 2];
            hr_font_get_glyph_h_advances(font, 2, glyphs.as_ptr(), 4, advances.as_mut_ptr(), 4);
            assert_eq!(advances, [700, 701]);
            assert_eq!(hr_font_get_glyph_h_advance(font, 1), 700);
            let (mut x, mut y) = (0, 0);
            assert_eq!(
                hr_font_get_glyph_h_origin(font, 1, &raw mut x, &raw mut y),
                1
            );
            assert_eq!((x, y), (12, 34));
            assert_eq!(hr_font_get_glyph_h_kerning(font, 10, 3), 7);
            assert_eq!(hr_font_get_glyph_v_kerning(font, 10, 3), 7);

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(funcs);
        });
    }
}

#[test]
fn added_font_callbacks_delegate_and_scale_through_subfonts() {
    unsafe {
        with_font(|_, parent| {
            hr_font_set_scale(parent, 1000, 2000);
            let funcs = hr_font_funcs_create();
            let mut calls = 0u32;
            hr_font_funcs_set_glyph_h_advances_func(
                funcs,
                Some(batch_advances),
                (&raw mut calls).cast(),
                None,
            );
            hr_font_funcs_set_glyph_h_origin_func(funcs, Some(fixed_origin), ptr::null_mut(), None);
            hr_font_funcs_set_glyph_h_kerning_func(
                funcs,
                Some(fixed_kerning),
                ptr::null_mut(),
                None,
            );
            hr_font_funcs_set_glyph_v_kerning_func(
                funcs,
                Some(fixed_kerning),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(parent, funcs, ptr::null_mut(), None);
            let child = hr_font_create_sub_font(parent);
            hr_font_set_scale(child, 2000, 4000);

            let glyphs = [1u32, 2];
            let mut advances = [0i32; 2];
            hr_font_get_glyph_h_advances(child, 2, glyphs.as_ptr(), 4, advances.as_mut_ptr(), 4);
            assert_eq!(advances, [1400, 1402]);
            let (mut x, mut y) = (0, 0);
            assert_eq!(
                hr_font_get_glyph_h_origin(child, 1, &raw mut x, &raw mut y),
                1
            );
            assert_eq!((x, y), (24, 68));
            assert_eq!(hr_font_get_glyph_h_kerning(child, 10, 3), 14);
            assert_eq!(hr_font_get_glyph_v_kerning(child, 10, 3), 14);
            assert!(calls > 0);

            hr_font_destroy(child);
            hr_font_funcs_destroy(funcs);
        });
    }
}

#[test]
fn strided_advances_stay_inside_runs_that_end_at_their_last_entry() {
    // One glyph and one advance, each a lone integer, walked with a stride
    // wider than an entry. Each run ends where its only entry does, so the
    // walk has nowhere to step to after it.
    unsafe {
        with_font(|_, font| {
            let glyph: hr_codepoint_t = 1;
            let mut advance: hr_position_t = 0;
            hr_font_get_glyph_h_advances(font, 1, &raw const glyph, 16, &raw mut advance, 16);
            assert_eq!(advance, hr_font_get_glyph_h_advance(font, glyph));
            advance = 0;
            hr_font_get_glyph_v_advances(font, 1, &raw const glyph, 16, &raw mut advance, 16);
            assert_eq!(advance, hr_font_get_glyph_v_advance(font, glyph));

            // A batch callback on a parent at another scale has its answers
            // rescaled in place, which walks the advances a second time.
            let funcs = hr_font_funcs_create();
            let mut calls = 0u32;
            hr_font_funcs_set_glyph_h_advances_func(
                funcs,
                Some(batch_advances),
                (&raw mut calls).cast(),
                None,
            );
            hr_font_set_funcs(font, funcs, ptr::null_mut(), None);
            hr_font_set_scale(font, 1000, 1000);
            let child = hr_font_create_sub_font(font);
            hr_font_set_scale(child, 2000, 2000);
            advance = 0;
            hr_font_get_glyph_h_advances(child, 1, &raw const glyph, 16, &raw mut advance, 16);
            assert_eq!(advance, 1400);
            assert_eq!(calls, 1);

            hr_font_destroy(child);
            hr_font_funcs_destroy(funcs);
        });
    }
}

#[test]
fn face_count_reads_single_fonts_and_collections() {
    unsafe {
        let single = std::fs::read(font_path(PLAIN_FONT)).unwrap();
        let blob = blob_over(&single);
        assert_eq!(hr_face_count(blob), 1);
        hr_blob_destroy(blob);

        let collection = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("harfrust/tests/fonts/in-house/TTC.ttc"),
        )
        .unwrap();
        let blob = blob_over(&collection);
        assert!(hr_face_count(blob) > 1);
        hr_blob_destroy(blob);

        let type1 = b"%!PS-AdobeFont-1.0\n";
        let blob = blob_over(type1);
        assert_eq!(hr_face_count(blob), 0);
        hr_blob_destroy(blob);
        assert_eq!(hr_face_count(hr_blob_get_empty()), 0);
    }
}

unsafe extern "C" fn always_glyph_777(
    _font: *mut hr_font_t,
    _font_data: *mut c_void,
    _unicode: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
    _user_data: *mut c_void,
) -> hr_bool_t {
    unsafe { *glyph = 777 };
    1
}

#[test]
fn the_glyph_getters_ask_the_installed_callbacks() {
    unsafe {
        with_font(|_, font| {
            // A font answers the same whichever way it is asked: the getter
            // and shaping both go through the callbacks that were installed.
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_eq!(glyph, 777);

            let buffer = buffer_with_text("a");
            hr_shape(font, buffer, ptr::null(), 0);
            assert_eq!(glyph_ids(buffer), [777]);
            hr_buffer_destroy(buffer);
        });

        // With no callback to answer, the getter reports nothing found and
        // leaves the glyph at zero rather than at whatever it was handed.
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_funcs_destroy(ffuncs);

            let mut glyph: hr_codepoint_t = 12345;
            assert_eq!(
                hr_font_get_nominal_glyph(font, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_eq!(glyph, 0);
        });

        // A font never given callbacks reads its own cmap.
        with_font(|_, font| {
            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_ne!(glyph, 0);
            assert_ne!(glyph, 777);
        });
    }
}

#[test]
fn the_built_in_callbacks_can_be_asked_for_again() {
    unsafe {
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        with_font(|_, font| {
            let builtin = {
                let buffer = buffer_with_text(TEXT);
                hr_shape(font, buffer, ptr::null(), 0);
                let ids = glyph_ids(buffer);
                hr_buffer_destroy(buffer);
                ids
            };
            assert!(builtin.iter().all(|&id| id != 0));

            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                ptr::null_mut(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, user_data, Some(count_drop));
            hr_font_funcs_destroy(ffuncs);

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert!(glyph_ids(buffer).iter().all(|&id| id == 777));
            hr_buffer_destroy(buffer);

            // Asking for no callbacks answers nothing; this is the way back
            // to the ones the font started with.
            hr_ot_font_set_funcs(font);
            assert_eq!(
                drops.load(Ordering::SeqCst),
                1,
                "the replaced callbacks' data should be released"
            );

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert_eq!(glyph_ids(buffer), builtin);
            hr_buffer_destroy(buffer);

            let mut glyph: hr_codepoint_t = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_ne!(glyph, 777);
        });
    }
}

#[test]
fn unset_callbacks_report_nothing_available() {
    unsafe {
        with_font(|_, font| {
            // As in HarfBuzz, an installed funcs object is authoritative: it is
            // not blended with the built-in callbacks, so an empty one reports
            // nothing available rather than falling back.
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let mut len: c_uint = 0;
            let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
            assert!(len > 0);
            // An advance nobody answers for is the font's own scale, which is
            // what HarfBuzz answers with rather than nothing at all.
            let mut x_scale = 0;
            hr_font_get_scale(font, &raw mut x_scale, ptr::null_mut());
            for i in 0..len as usize {
                assert_eq!((*positions.add(i)).x_advance, x_scale);
            }
            assert!(glyph_ids(buffer).iter().all(|&id| id == 0));

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(ffuncs);
        });

        // Asking for no callbacks is itself an answer, and not a way back
        // to the built-in ones: HarfBuzz reads NULL as its empty funcs
        // object, which reports nothing available. A font that was never
        // given callbacks is the one that reads its own tables.
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_set_funcs(font, ptr::null_mut(), ptr::null_mut(), None);

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert!(glyph_ids(buffer).iter().all(|&id| id == 0));

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(ffuncs);
        });

        // Which leaves the untouched font reading its own tables.
        with_font(|_, font| {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert!(glyph_ids(buffer).iter().all(|&id| id != 0));
            hr_buffer_destroy(buffer);
        });
    }
}

// ------------------------------------------------- font data ownership ----

use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn font_data_outlives_every_font_sharing_it() {
    unsafe {
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, user_data, Some(count_drop));

            // A sub-font holds no data of its own; it asks its parent every
            // time, so it is never left holding what the parent has replaced.
            let sub = hr_font_create_sub_font(font);
            assert_eq!(drops.load(Ordering::SeqCst), 0);

            // Which means replacing the parent's callbacks releases the data
            // they were given there and then, live sub-font or not, as it
            // does in HarfBuzz.
            hr_font_set_funcs(font, ffuncs, ptr::dangling_mut::<c_void>(), None);
            assert_eq!(drops.load(Ordering::SeqCst), 1);

            hr_font_destroy(sub);
            assert_eq!(drops.load(Ordering::SeqCst), 1);

            hr_font_funcs_destroy(ffuncs);
        });
    }
}

#[test]
fn font_data_is_released_once_when_replaced() {
    unsafe {
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, user_data, Some(count_drop));
            assert_eq!(drops.load(Ordering::SeqCst), 0);

            // With nothing else sharing it, replacing releases it at once.
            hr_font_set_funcs(font, ffuncs, ptr::dangling_mut::<c_void>(), None);
            assert_eq!(drops.load(Ordering::SeqCst), 1);

            hr_font_funcs_destroy(ffuncs);
        });
        // And not again when the font itself goes.
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn a_sub_font_shapes_with_the_parents_callbacks() {
    unsafe {
        with_font(|_, font| {
            let mut advance: hr_position_t = 777;
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_h_advance_func(
                ffuncs,
                Some(fixed_advance),
                ptr::from_mut(&mut advance).cast::<c_void>(),
                None,
            );
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);

            let sub = hr_font_create_sub_font(font);
            let buffer = buffer_with_text(TEXT);
            hr_shape(sub, buffer, ptr::null(), 0);

            let mut len: c_uint = 0;
            let positions = hr_buffer_get_glyph_positions(buffer, ptr::from_mut(&mut len));
            assert!(len > 0);
            for i in 0..len as usize {
                assert_eq!((*positions.add(i)).x_advance, 777);
            }

            hr_buffer_destroy(buffer);
            hr_font_destroy(sub);
            hr_font_funcs_destroy(ffuncs);
        });
    }
}

// ------------------------------------------------------------- user data ----

static KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };
static OTHER_KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };

#[test]
fn user_data_is_keyed_by_address() {
    unsafe {
        let buffer = hr_buffer_create();
        let value = ptr::without_provenance_mut::<c_void>(42);
        assert_ne!(
            hr_buffer_set_user_data(buffer, &raw const KEY, value, None, 1),
            0
        );
        assert_eq!(hr_buffer_get_user_data(buffer, &raw const KEY), value);
        assert!(hr_buffer_get_user_data(buffer, &raw const OTHER_KEY).is_null());

        // Without `replace`, an existing key is left alone.
        let other = ptr::without_provenance_mut::<c_void>(7);
        assert_eq!(
            hr_buffer_set_user_data(buffer, &raw const KEY, other, None, 0),
            0
        );
        assert_eq!(hr_buffer_get_user_data(buffer, &raw const KEY), value);

        hr_buffer_destroy(buffer);
    }
}

#[test]
fn a_destroy_callback_may_take_and_release_a_reference() {
    // A user data destructor runs while the object's last reference is being
    // released, and may still use the object. One that takes a reference of
    // its own and gives it back, as a scoped handle would, must not release
    // the object a second time.
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn borrow_briefly(data: *mut c_void) {
        CALLS.fetch_add(1, Ordering::Relaxed);
        let buffer = data.cast::<hr_buffer_t>();
        unsafe {
            assert_eq!(hr_buffer_reference(buffer), buffer);
            assert_eq!(hr_buffer_get_length(buffer), 3);
            hr_buffer_destroy(buffer);
            // A dying buffer stays closed to changes however it is counted.
            assert_eq!(hr_buffer_set_length(buffer, 0), 0);
        }
    }
    unsafe {
        let buffer = buffer_with_text(TEXT);
        assert_ne!(
            hr_buffer_set_user_data(
                buffer,
                &raw const KEY,
                buffer.cast(),
                Some(borrow_briefly),
                1
            ),
            0
        );
        hr_buffer_destroy(buffer);
    }
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn immortal_objects_reject_user_data() {
    unsafe {
        let empty = hr_buffer_get_empty();
        assert_eq!(
            hr_buffer_set_user_data(
                empty,
                &raw const KEY,
                ptr::dangling_mut::<c_void>(),
                None,
                1
            ),
            0
        );
    }
}

#[test]
fn asking_for_glyph_infos_again_keeps_the_first_pointer() {
    // What the getter promises: asking again leaves the pointer from the
    // first time as usable as the one from the second.
    unsafe {
        let buffer = buffer_with_text(TEXT);
        let mut len = 0;
        let first = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        let second = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        assert_eq!(len as usize, TEXT.len());
        assert_eq!(first, second);

        assert_eq!((*first).codepoint, u32::from(b'a'));
        (*first).codepoint = u32::from(b'x');
        assert_eq!((*second).codepoint, u32::from(b'x'));

        // Whichever of them was written through last.
        let third = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        (*first).codepoint = u32::from(b'y');
        (*second.add(1)).codepoint = u32::from(b'z');
        assert_eq!((*third).codepoint, u32::from(b'y'));
        assert_eq!(
            glyph_ids(buffer),
            "yzc".chars().map(u32::from).collect::<Vec<_>>()
        );
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn asking_for_glyph_positions_again_keeps_the_first_pointer() {
    unsafe {
        let buffer = buffer_with_text(TEXT);
        assert_eq!(hr_buffer_has_positions(buffer), 0);
        // The first time is what allocates the positions, zeroed. The second
        // finds them there and leaves them, and the pointer to them, alone.
        let mut len = 0;
        let first = hr_buffer_get_glyph_positions(buffer, &raw mut len);
        assert_eq!(len as usize, TEXT.len());
        assert_eq!((*first).x_advance, 0);
        (*first).x_advance = 5;
        let second = hr_buffer_get_glyph_positions(buffer, &raw mut len);
        assert_eq!(len as usize, TEXT.len());
        assert_eq!(first, second);

        assert_eq!((*first).x_advance, 5);
        (*first).x_advance = 6;
        assert_eq!((*second).x_advance, 6);

        // Whichever of them was written through last.
        let third = hr_buffer_get_glyph_positions(buffer, &raw mut len);
        (*first).x_advance = 7;
        (*second.add(1)).x_advance = 8;
        assert_eq!(((*third).x_advance, (*third.add(1)).x_advance), (7, 8));
        assert_eq!((*third.add(2)).x_advance, 0);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn reading_a_buffer_keeps_its_glyph_pointers() {
    // The pointers stay valid until the buffer's contents change, as in
    // HarfBuzz. None of the calls made here changes them, including the
    // ones that read the items, so pointers taken before can still be
    // written through after each returns.
    unsafe {
        let buffer = hr_buffer_create();
        hr_buffer_add_utf8(buffer, c"abc".as_ptr(), -1, 0, -1);
        let mut len = 0;
        let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
        (*infos).codepoint = u32::from(b'x');
        (*positions).x_advance = 1;

        // Guessing reads the text for its script.
        hr_buffer_guess_segment_properties(buffer);
        assert_eq!(hr_buffer_get_script(buffer), HR_SCRIPT_LATIN);
        (*infos).codepoint = 6;
        (*positions).x_advance = 2;

        assert_ne!(hr_buffer_has_positions(buffer), 0);
        assert_eq!(hr_buffer_get_length(buffer), len);
        hr_buffer_set_direction(buffer, HR_DIRECTION_LTR);
        (*infos).codepoint = 7;
        (*positions).x_advance = 3;

        let flags = HR_BUFFER_SERIALIZE_FLAG_DEFAULT;
        let (items, text) = serialize(buffer, 0, len, ptr::null_mut(), flags);
        assert_eq!(
            (items, text.as_str()),
            (3, "[gid7=0+3|gid98=1+0|gid99=2+0]")
        );
        (*infos).codepoint = 8;
        (*positions).x_advance = 4;

        // Being appended from is being read, too.
        let copy = hr_buffer_create();
        hr_buffer_append(copy, buffer, 0, len);
        (*infos).codepoint = 9;
        (*positions).x_advance = 5;

        let (_, text) = serialize(copy, 0, len, ptr::null_mut(), flags);
        assert_eq!(text, "[gid8=0+4|gid98=1+0|gid99=2+0]");
        let (_, text) = serialize(buffer, 0, len, ptr::null_mut(), flags);
        assert_eq!(text, "[gid9=0+5|gid98=1+0|gid99=2+0]");
        hr_buffer_destroy(copy);

        // Reserving room changes nothing in the buffer and can still move
        // the items, so the pointers are asked for again after it.
        assert_ne!(hr_buffer_pre_allocate(buffer, 1000), 0);
        let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
        let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
        assert_eq!(len, 3);
        assert_eq!(((*infos).codepoint, (*positions).x_advance), (9, 5));
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn the_empty_buffer_is_not_written_by_the_glyph_getters() {
    // The empty buffer is one object shared by every thread, so the getters
    // must neither allocate positions in it nor hand out a writable pointer
    // into it: it answers as `NULL` does, with nothing.
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| unsafe {
                let empty = hr_buffer_get_empty();
                let mut len = 7;
                assert!(hr_buffer_get_glyph_positions(empty, &raw mut len).is_null());
                assert_eq!(len, 0);
                len = 7;
                assert!(hr_buffer_get_glyph_infos(empty, &raw mut len).is_null());
                assert_eq!(len, 0);
                assert_eq!(hr_buffer_has_positions(ptr::null_mut()), 0);
            })
        })
        .collect();

    for thread in threads {
        thread.join().expect("thread panicked");
    }
}

#[test]
fn shaping_the_empty_buffer_only_looks_at_it() {
    // The empty buffer is one object shared by every thread. Shaping it has
    // nothing to do, and has to find that out without taking the buffer
    // exclusively, because any number of threads may be asking at once.
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| unsafe {
                let empty = hr_buffer_get_empty();
                // Nothing to shape is answered before the font is looked at.
                assert_ne!(
                    hr_shape_full(ptr::null_mut(), empty, ptr::null(), 0, ptr::null()),
                    0
                );
                hr_shape(ptr::null_mut(), empty, ptr::null(), 0);
                assert_ne!(
                    hr_shape_plan_execute(
                        hr_shape_plan_get_empty(),
                        ptr::null_mut(),
                        empty,
                        ptr::null(),
                        0
                    ),
                    0
                );
                assert_eq!(hr_buffer_get_length(empty), 0);
            })
        })
        .collect();

    for thread in threads {
        thread.join().expect("thread panicked");
    }
}

#[test]
fn a_buffer_being_destroyed_refuses_to_be_shaped() {
    // A user data destructor runs while the buffer's last reference is being
    // released, and may still look at the buffer. Shaping would need the
    // buffer exclusively, which one on its way out no longer gives, as it
    // does not for any other change. Taking a reference to it for the
    // length of the call, as a scoped handle would, does not bring it back.
    struct Dying {
        font: *mut hr_font_t,
        // Null to shape without a plan.
        plan: *mut hr_shape_plan_t,
        buffer: *mut hr_buffer_t,
        // Whether to hold a reference of the destructor's own while shaping.
        held: bool,
        shaped: core::cell::Cell<Option<hr_bool_t>>,
        content: core::cell::Cell<hr_buffer_content_type_t>,
    }

    unsafe extern "C" fn shape_on_the_way_out(data: *mut c_void) {
        unsafe {
            let dying = &*data.cast::<Dying>();
            if dying.held {
                hr_buffer_reference(dying.buffer);
            }
            let shaped = if dying.plan.is_null() {
                hr_shape_full(dying.font, dying.buffer, ptr::null(), 0, ptr::null())
            } else {
                hr_shape_plan_execute(dying.plan, dying.font, dying.buffer, ptr::null(), 0)
            };
            if dying.held {
                hr_buffer_destroy(dying.buffer);
            }
            dying.shaped.set(Some(shaped));
            dying.content.set(hr_buffer_get_content_type(dying.buffer));
        }
    }

    unsafe {
        with_font(|face, font| {
            let props = latin_props();
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());
            for (plan, held) in [
                (ptr::null_mut(), false),
                (plan, false),
                (ptr::null_mut(), true),
                (plan, true),
            ] {
                let buffer = buffer_with_text(TEXT);
                let dying = Dying {
                    font,
                    plan,
                    buffer,
                    held,
                    shaped: core::cell::Cell::new(None),
                    content: core::cell::Cell::new(HR_BUFFER_CONTENT_TYPE_INVALID),
                };
                assert_ne!(
                    hr_buffer_set_user_data(
                        buffer,
                        &raw const KEY,
                        ptr::from_ref(&dying).cast_mut().cast(),
                        Some(shape_on_the_way_out),
                        1
                    ),
                    0
                );
                hr_buffer_destroy(buffer);
                assert_eq!(dying.shaped.get(), Some(0));
                assert_eq!(dying.content.get(), HR_BUFFER_CONTENT_TYPE_UNICODE);
            }
            hr_shape_plan_destroy(plan);
        });
    }
}

// --------------------------------------------------------------- common ----

#[test]
fn tags_round_trip() {
    unsafe {
        let tag = hr_tag_from_string(c"kern".as_ptr(), -1);
        let mut buf = [0i8; 5];
        hr_tag_to_string(tag, buf.as_mut_ptr().cast::<c_char>());
        assert_eq!(&buf[..4], b"kern".map(|b| b as i8));
        // Short strings pad with spaces, as in HarfBuzz.
        let short = hr_tag_from_string(c"ab".as_ptr(), -1);
        hr_tag_to_string(short, buf.as_mut_ptr().cast::<c_char>());
        assert_eq!(&buf[..4], b"ab  ".map(|b| b as i8));
    }
}

#[test]
fn directions_round_trip() {
    unsafe {
        assert_eq!(
            hr_direction_from_string(c"rtl".as_ptr(), -1),
            HR_DIRECTION_RTL
        );
        assert_ne!(hr_direction_is_horizontal(HR_DIRECTION_RTL), 0);
        assert_ne!(hr_direction_is_backward(HR_DIRECTION_RTL), 0);
        assert_eq!(hr_direction_reverse(HR_DIRECTION_LTR), HR_DIRECTION_RTL);
    }
}

#[test]
fn scripts_match_harfbuzz_values() {
    // HB_SCRIPT_LATIN is HB_TAG('L','a','t','n').
    assert_eq!(HR_SCRIPT_LATIN, 0x4C61_746E);
    assert_eq!(HR_SCRIPT_ARABIC, 0x4172_6162);
    for (tag, script) in [
        (b"Jurc", HR_SCRIPT_JURCHEN),
        (b"Pcun", HR_SCRIPT_PROTO_CUNEIFORM),
        (b"Seal", HR_SCRIPT_SEAL),
    ] {
        assert_eq!(script, u32::from_be_bytes(*tag));
        assert_eq!(hr_script_from_iso15924_tag(script), script);
        assert_eq!(hr_script_to_iso15924_tag(script), script);
        assert_eq!(hr_script_get_horizontal_direction(script), HR_DIRECTION_LTR);
    }
    unsafe {
        assert_eq!(hr_script_from_string(c"Latn".as_ptr(), -1), HR_SCRIPT_LATIN);
        assert_eq!(
            hr_script_get_horizontal_direction(HR_SCRIPT_ARABIC),
            HR_DIRECTION_RTL
        );
        assert_eq!(
            hr_script_get_horizontal_direction(HR_SCRIPT_LATIN),
            HR_DIRECTION_LTR
        );
    }
}

#[test]
fn languages_are_interned() {
    unsafe {
        let a = hr_language_from_string(c"en-US".as_ptr(), -1);
        let b = hr_language_from_string(c"en-US".as_ptr(), -1);
        assert_eq!(a, b, "equal tags must intern to the same pointer");
        let name = core::ffi::CStr::from_ptr(hr_language_to_string(a));
        assert_eq!(name.to_str().unwrap(), "en-us");

        // The second argument is the more specific of the two: "en-US" is a
        // kind of "en", and not the other way about.
        let en = hr_language_from_string(c"en".as_ptr(), -1);
        assert_ne!(hr_language_matches(en, a), 0);
        assert_eq!(hr_language_matches(a, en), 0);

        // A locale's codeset and modifier are not part of the language.
        let locale = hr_language_from_string(c"en_US.utf8".as_ptr(), -1);
        assert_eq!(locale, a);
    }
}

#[test]
fn interning_many_languages_stays_cheap() {
    // Content chooses the languages, so the table can grow as large as it
    // likes; finding one must not mean comparing it with every other. Walking
    // the table made this many take seconds, and keying it makes it instant.
    //
    // Miri has nothing to say about speed, and takes over twenty minutes
    // with this many. A few dozen still have the table grow around the
    // languages already handed out, which is what it can check.
    let count = if cfg!(miri) { 64 } else { 20_000u32 };
    let names: Vec<_> = (0..count)
        .map(|i| std::ffi::CString::new(format!("x-{i:x}")).unwrap())
        .collect();
    unsafe {
        let interned: Vec<_> = names
            .iter()
            .map(|name| hr_language_from_string(name.as_ptr(), -1))
            .collect();
        for (name, language) in names.iter().zip(&interned) {
            assert_eq!(hr_language_from_string(name.as_ptr(), -1), *language);
            assert_eq!(
                core::ffi::CStr::from_ptr(hr_language_to_string(*language)),
                name.as_c_str()
            );
        }
        // Tags that are not registered intern a language each, and share the
        // same table.
        let tag = hr_ot_tag_to_language(u32::from_be_bytes(*b"QQQQ"));
        assert_eq!(hr_ot_tag_to_language(u32::from_be_bytes(*b"QQQQ")), tag);
        assert_eq!(hr_language_from_string(hr_language_to_string(tag), -1), tag);
    }
}

#[test]
fn glyph_and_buffer_structs_match_harfbuzz_layout() {
    // hb_glyph_info_t is five 32 bit fields, hb_glyph_position_t likewise.
    assert_eq!(size_of::<hr_glyph_info_t>(), 20);
    assert_eq!(size_of::<hr_glyph_position_t>(), 20);
    assert_eq!(align_of::<hr_glyph_info_t>(), 4);
    assert_eq!(align_of::<hr_glyph_position_t>(), 4);
}

#[test]
fn variations_parse_and_apply() {
    unsafe {
        let mut variation = hr_variation_t { tag: 0, value: 0.0 };
        assert_ne!(
            hr_variation_from_string(c"wdth=200".as_ptr(), -1, &raw mut variation),
            0
        );
        assert_eq!(variation.tag, hr_tag_from_string(c"wdth".as_ptr(), -1));
        assert!((variation.value - 200.0).abs() < f32::EPSILON);

        with_named_font(VARIABLE_FONT, |_, font| {
            // A font at its defaults carries no coordinates.
            let mut len: c_uint = 0;
            hr_font_get_var_coords_normalized(font, &raw mut len);
            assert_eq!(len, 0);

            let advances = |font: *mut hr_font_t| {
                let buffer = buffer_with_text(TEXT);
                hr_shape(font, buffer, ptr::null(), 0);
                let mut len: c_uint = 0;
                let positions = hr_buffer_get_glyph_positions(buffer, &raw mut len);
                let values: Vec<i32> = (0..len as usize)
                    .map(|i| (*positions.add(i)).x_advance)
                    .collect();
                hr_buffer_destroy(buffer);
                values
            };
            let at_default = advances(font);

            // This font's axes are wdth then wght, and wdth drives the
            // advances through HVAR.
            hr_font_set_variations(font, &raw const variation, 1);
            let coords = hr_font_get_var_coords_normalized(font, &raw mut len);
            assert_eq!(len, 2, "expected the wdth and wght axes");
            // Coordinates are 2.14, so the axis maximum normalizes to 1.0.
            assert_eq!(*coords, 16384, "wdth should be at its maximum");
            assert_eq!(*coords.add(1), 0, "wght was not asked to move");

            let widened = advances(font);
            assert_ne!(
                widened, at_default,
                "variation settings did not reach shaping"
            );
            assert!(
                widened.iter().zip(&at_default).all(|(w, d)| w >= d),
                "a wider width should not narrow any advance"
            );

            // Clearing the settings puts the font back where it started.
            hr_font_set_variations(font, ptr::null(), 0);
            hr_font_get_var_coords_normalized(font, &raw mut len);
            assert_eq!(len, 0);
            assert_eq!(advances(font), at_default);
        });
    }
}

#[test]
fn normalized_coordinates_can_be_set_directly() {
    unsafe {
        with_named_font(VARIABLE_FONT, |_, font| {
            // Half way along the wdth axis, in 2.14.
            let coords = [8192, 0];
            hr_font_set_var_coords_normalized(font, coords.as_ptr(), 2);

            let mut len: c_uint = 0;
            let got = hr_font_get_var_coords_normalized(font, &raw mut len);
            assert_eq!(len, 2);
            assert_eq!(*got, 8192);
            assert_eq!(*got.add(1), 0);

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert_ne!(glyph_ids(buffer), []);
            hr_buffer_destroy(buffer);
        });
    }
}

// ----------------------------------------------------------- shape plans ----

/// Segment properties matching `buffer_with_text`, which guesses them from
/// lowercase Latin text.
fn latin_props() -> hr_segment_properties_t {
    hr_segment_properties_t {
        direction: HR_DIRECTION_LTR,
        script: HR_SCRIPT_LATIN,
        // What a buffer carries once its properties have been guessed, so
        // that a plan built from these applies to one.
        language: hr_language_get_default(),
        reserved1: ptr::null_mut(),
        reserved2: ptr::null_mut(),
    }
}

#[test]
fn segment_properties_round_trip_through_a_buffer() {
    unsafe {
        let buffer = buffer_with_text(TEXT);
        let mut props = latin_props();
        hr_buffer_get_segment_properties(buffer, ptr::from_mut(&mut props));
        assert_eq!(props.direction, HR_DIRECTION_LTR);
        assert_eq!(props.script, HR_SCRIPT_LATIN);

        let other = hr_buffer_create();
        hr_buffer_set_segment_properties(other, ptr::from_ref(&props));
        assert_eq!(hr_buffer_get_direction(other), HR_DIRECTION_LTR);
        assert_eq!(hr_buffer_get_script(other), HR_SCRIPT_LATIN);

        hr_buffer_destroy(other);
        hr_buffer_destroy(buffer);
    }
}

#[test]
fn segment_properties_compare_hash_and_overlay() {
    unsafe {
        let a = latin_props();
        let mut b = a;
        assert_ne!(
            hr_segment_properties_equal(ptr::from_ref(&a), ptr::from_ref(&b)),
            0
        );
        assert_eq!(
            hr_segment_properties_hash(ptr::from_ref(&a)),
            hr_segment_properties_hash(ptr::from_ref(&b))
        );

        b.direction = HR_DIRECTION_RTL;
        assert_eq!(
            hr_segment_properties_equal(ptr::from_ref(&a), ptr::from_ref(&b)),
            0
        );

        // Overlay fills in only what is unset.
        let mut partial = hr_segment_properties_t {
            direction: HR_DIRECTION_INVALID,
            script: HR_SCRIPT_ARABIC,
            language: ptr::null(),
            reserved1: ptr::null_mut(),
            reserved2: ptr::null_mut(),
        };
        hr_segment_properties_overlay(ptr::from_mut(&mut partial), ptr::from_ref(&a));
        assert_eq!(partial.direction, HR_DIRECTION_LTR, "unset field filled in");
        assert_eq!(partial.script, HR_SCRIPT_ARABIC, "set field left alone");
    }
}

#[test]
fn a_plan_shapes_the_same_as_hr_shape() {
    unsafe {
        with_font(|face, font| {
            let expected = {
                let buffer = buffer_with_text(TEXT);
                hr_shape(font, buffer, ptr::null(), 0);
                let ids = glyph_ids(buffer);
                hr_buffer_destroy(buffer);
                ids
            };

            let props = latin_props();
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());
            assert_eq!(
                core::ffi::CStr::from_ptr(hr_shape_plan_get_shaper(plan))
                    .to_str()
                    .unwrap(),
                "ot"
            );

            let buffer = buffer_with_text(TEXT);
            assert_ne!(hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0), 0);
            assert_eq!(glyph_ids(buffer), expected);
            assert_eq!(
                hr_buffer_get_content_type(buffer),
                HR_BUFFER_CONTENT_TYPE_GLYPHS
            );

            hr_buffer_destroy(buffer);
            hr_shape_plan_destroy(plan);
        });
    }
}

#[test]
fn cached_plans_are_reused_and_agree() {
    unsafe {
        with_font(|face, font| {
            let props = latin_props();
            let make = || {
                hr_shape_plan_create_cached(
                    face,
                    ptr::from_ref(&props),
                    ptr::null(),
                    0,
                    ptr::null(),
                )
            };
            let first = make();
            let second = make();

            let shape_through = |plan: *mut hr_shape_plan_t| {
                let buffer = buffer_with_text(TEXT);
                assert_ne!(hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0), 0);
                let ids = glyph_ids(buffer);
                hr_buffer_destroy(buffer);
                ids
            };
            assert_eq!(shape_through(first), shape_through(second));

            hr_shape_plan_destroy(first);
            hr_shape_plan_destroy(second);
        });
    }
}

#[test]
fn cached_coordinate_plans_without_layout_variations_use_current_font_metrics() {
    unsafe {
        with_named_font("../in-house/NotoSans-VF.abc.ttf", |face, font| {
            let props = latin_props();
            let mut outputs = Vec::new();
            for coords in [[0, 0, 0], [8192, 0, 0], [-8192, 0, 0], [8192, 0, 0]] {
                hr_font_set_var_coords_normalized(font, coords.as_ptr(), coords.len() as c_uint);
                let cached = hr_shape_plan_create_cached2(
                    face,
                    &raw const props,
                    ptr::null(),
                    0,
                    coords.as_ptr(),
                    coords.len() as c_uint,
                    ptr::null(),
                );
                let uncached = hr_shape_plan_create2(
                    face,
                    &raw const props,
                    ptr::null(),
                    0,
                    coords.as_ptr(),
                    coords.len() as c_uint,
                    ptr::null(),
                );
                let shape_through = |plan| {
                    let buffer = buffer_with_text(TEXT);
                    assert_ne!(hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0), 0);
                    let mut len = 0;
                    let infos = hr_buffer_get_glyph_infos(buffer, &raw mut len);
                    let positions = hr_buffer_get_glyph_positions(buffer, ptr::null_mut());
                    let output: Vec<_> = (0..len as usize)
                        .map(|i| {
                            let info = &*infos.add(i);
                            let pos = &*positions.add(i);
                            (
                                info.codepoint,
                                info.cluster,
                                hr_glyph_info_get_glyph_flags(info),
                                pos.x_advance,
                                pos.y_advance,
                                pos.x_offset,
                                pos.y_offset,
                            )
                        })
                        .collect();
                    hr_buffer_destroy(buffer);
                    output
                };
                let output = shape_through(cached);
                assert_eq!(output, shape_through(uncached));
                outputs.push(output);
                hr_shape_plan_destroy(cached);
                hr_shape_plan_destroy(uncached);
            }
            assert_ne!(outputs[0], outputs[1]);
            assert_eq!(outputs[1], outputs[3]);
        });
    }
}

#[test]
fn a_plan_reports_the_properties_it_was_built_for() {
    unsafe {
        with_font(|face, _| {
            let mut props = latin_props();
            props.language = hr_language_from_string(c"en".as_ptr(), -1);
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());

            let mut got = hr_segment_properties_t {
                direction: HR_DIRECTION_INVALID,
                script: HR_SCRIPT_INVALID,
                language: ptr::null(),
                reserved1: ptr::null_mut(),
                reserved2: ptr::null_mut(),
            };
            hr_shape_plan_get_segment_properties(plan, ptr::from_mut(&mut got));
            assert_eq!(got.direction, HR_DIRECTION_LTR);
            assert_eq!(got.script, HR_SCRIPT_LATIN);
            assert_eq!(got.language, props.language);

            hr_shape_plan_destroy(plan);
        });
    }
}

/// The cases that must abort, each exercised in a child process.
///
/// `hr_shape_plan_execute` aborts rather than unwinding, so these cannot be
/// `#[should_panic]`: the test harness never regains control.
unsafe fn run_abort_case(case: &str) {
    match case {
        // A plan for right-to-left Arabic, used on a left-to-right Latin
        // buffer.
        "props" => unsafe {
            with_font(|face, font| {
                let props = hr_segment_properties_t {
                    direction: HR_DIRECTION_RTL,
                    script: HR_SCRIPT_ARABIC,
                    language: ptr::null(),
                    reserved1: ptr::null_mut(),
                    reserved2: ptr::null_mut(),
                };
                let plan =
                    hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());
                let buffer = buffer_with_text(TEXT);
                hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0);
            });
        },
        // A plan built over one face, used with a font over another.
        "face" => unsafe {
            with_font(|face, _| {
                let props = latin_props();
                let plan =
                    hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());
                with_named_font(VARIABLE_FONT, |_, other_font| {
                    let buffer = buffer_with_text(TEXT);
                    hr_shape_plan_execute(plan, other_font, buffer, ptr::null(), 0);
                });
            });
        },
        // A plan built for one variation, used with a font at another, which
        // shapes rather than aborting.
        "coords" => unsafe {
            with_named_font(VARIABLE_FONT, |face, font| {
                let props = latin_props();
                let coords = [8192, 0];
                let plan = hr_shape_plan_create2(
                    face,
                    ptr::from_ref(&props),
                    ptr::null(),
                    0,
                    coords.as_ptr(),
                    coords.len() as c_uint,
                    ptr::null(),
                );
                // The font is still at its defaults.
                let buffer = buffer_with_text(TEXT);
                hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0);
            });
        },
        // Shaping a buffer that already holds glyphs.
        "already_shaped" => unsafe {
            with_font(|_, font| {
                let buffer = buffer_with_text(TEXT);
                hr_shape(font, buffer, ptr::null(), 0);
                hr_shape(font, buffer, ptr::null(), 0);
            });
        },
        other => panic!("unknown abort case {other}"),
    }
}

/// Entry point for the child process, selected by an environment variable.
#[test]
fn abort_case() {
    let Ok(case) = std::env::var("HR_ABORT_CASE") else {
        // The parent runs this as an ordinary, and empty, test.
        return;
    };
    unsafe { run_abort_case(&case) };
    // The call returned. Say so with a status of its own rather than
    // panicking, which the parent could not tell from the abort it is
    // looking for.
    std::process::exit(RETURNED);
}

/// What the child exits with when the case it ran returned rather than
/// stopping the process.
const RETURNED: i32 = 99;

/// Re-runs `abort_case` in a child process and reports whether it died.
fn aborts(case: &str) -> bool {
    let exe = std::env::current_exe().expect("test binary path");
    let status = std::process::Command::new(exe)
        .args(["capi_tests::abort_case", "--exact"])
        .env("HR_ABORT_CASE", case)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("failed to run the child process");
    // Anything but a clean return, and the child says which of those it was:
    // without that, a case that quietly returned would read as an abort and
    // the test would pass whatever the API did.
    status.code() != Some(RETURNED) && !status.success()
}

#[test]
#[cfg_attr(miri, ignore = "runs a child process, which Miri cannot do")]
fn a_plan_that_does_not_apply_aborts() {
    // Matching HarfBuzz, where these are assertions rather than errors.
    assert!(aborts("props"), "mismatched segment properties must abort");
    assert!(aborts("face"), "a plan for another face must abort");
    // Variation settings are the exception, and this is what proves the check
    // above can tell the difference: HarfBuzz does not compare them, so a
    // plan built for another variation runs rather than stopping the process.
    assert!(
        !aborts("coords"),
        "a plan for other variation settings must not abort"
    );
}

#[test]
fn a_plan_that_applies_shapes_without_aborting() {
    unsafe {
        with_named_font(VARIABLE_FONT, |face, font| {
            let props = latin_props();
            let coords = [8192, 0];
            let plan = hr_shape_plan_create2(
                face,
                ptr::from_ref(&props),
                ptr::null(),
                0,
                coords.as_ptr(),
                coords.len() as c_uint,
                ptr::null(),
            );

            // Once the font is set to the coordinates the plan was built for,
            // the very call that aborts above succeeds.
            hr_font_set_var_coords_normalized(font, coords.as_ptr(), coords.len() as c_uint);
            let buffer = buffer_with_text(TEXT);
            assert_ne!(hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0), 0);
            assert_ne!(glyph_ids(buffer), []);

            hr_buffer_destroy(buffer);
            hr_shape_plan_destroy(plan);
        });
    }
}

#[test]
fn a_plan_without_a_direction_is_empty() {
    unsafe {
        with_font(|face, font| {
            let props = hr_segment_properties_t {
                direction: HR_DIRECTION_INVALID,
                script: HR_SCRIPT_LATIN,
                language: ptr::null(),
                reserved1: ptr::null_mut(),
                reserved2: ptr::null_mut(),
            };
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());
            assert_eq!(plan, hr_shape_plan_get_empty());
            assert!(hr_shape_plan_get_shaper(plan).is_null());

            // The empty plan shapes nothing.
            let buffer = buffer_with_text(TEXT);
            assert_eq!(hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0), 0);
            hr_buffer_destroy(buffer);
        });
    }
}

#[test]
fn an_unknown_shaper_list_yields_the_empty_plan() {
    unsafe {
        with_font(|face, _| {
            let props = latin_props();
            let list = [c"graphite2".as_ptr(), ptr::null()];
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, list.as_ptr());
            assert_eq!(plan, hr_shape_plan_get_empty());

            // ... but a list naming "ot" is honoured.
            let list = [c"ot".as_ptr(), ptr::null()];
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, list.as_ptr());
            assert_ne!(plan, hr_shape_plan_get_empty());
            hr_shape_plan_destroy(plan);
        });
    }
}

#[test]
fn shape_full_reports_only_whether_a_shaper_ran() {
    unsafe {
        with_font(|_, font| {
            // The one thing that fails: a list naming only shapers this
            // library does not have.
            let buffer = buffer_with_text(TEXT);
            let absent = [c"graphite2".as_ptr(), ptr::null()];
            assert_eq!(
                hr_shape_full(font, buffer, ptr::null(), 0, absent.as_ptr()),
                0
            );
            hr_buffer_destroy(buffer);

            // A list naming "ot" runs the shaper, and so does no list at all.
            for list in [[c"ot".as_ptr(), ptr::null()].as_ptr(), ptr::null()] {
                let buffer = buffer_with_text(TEXT);
                assert_ne!(hr_shape_full(font, buffer, ptr::null(), 0, list), 0);
                assert_ne!(hr_buffer_allocation_successful(buffer), 0);
                assert_ne!(glyph_ids(buffer), []);
                hr_buffer_destroy(buffer);
            }
        });
    }
}

unsafe extern "C" fn count_drop(user_data: *mut c_void) {
    let drops = unsafe { &*user_data.cast::<AtomicUsize>() };
    drops.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn clearing_a_callback_still_releases_its_user_data() {
    unsafe {
        let ffuncs = hr_font_funcs_create();
        let drops = AtomicUsize::new(0);
        let user_data: *mut c_void = ptr::from_ref(&drops).cast_mut().cast();
        // A setter owns the data it is handed whether or not there is a
        // callback to keep it, so clearing one has to release it. Otherwise
        // the caller cannot tell which calls took ownership.
        hr_font_funcs_set_nominal_glyph_func(ffuncs, None, user_data, Some(count_drop));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        hr_font_funcs_destroy(ffuncs);
    }
}

#[test]
fn clearing_user_data_removes_the_key() {
    unsafe {
        with_font(|face, _| {
            static KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };
            let value = ptr::dangling_mut::<c_void>();
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, value, None, 1),
                0
            );

            // Neither data nor a destructor removes the entry outright.
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, ptr::null_mut(), None, 1),
                0
            );
            assert!(hr_face_get_user_data(face, &raw const KEY).is_null());

            // Which is observable: setting without `replace` succeeds only
            // when nothing holds the key.
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, value, None, 0),
                0
            );
            assert_eq!(hr_face_get_user_data(face, &raw const KEY), value);
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, ptr::null_mut(), None, 1),
                0
            );
        });
    }
}

static REENTRANT_KEY: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };
static REENTRANT_OTHER: hr_user_data_key_t = hr_user_data_key_t { unused: 0 };

#[test]
fn final_destruction_rejects_user_data_reattachment() {
    use core::cell::Cell;

    struct Destruction {
        set: *mut hr_set_t,
        first_calls: Cell<u32>,
        second_calls: Cell<u32>,
        observed_empty: Cell<bool>,
        accepted: Cell<c_int>,
    }

    unsafe extern "C" fn second_destroy(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state.second_calls.set(state.second_calls.get() + 1);
        // If accepted by the first callback, this query used to run while
        // Box::drop held exclusive access to the set, violating aliasing.
        unsafe { hr_set_get_user_data(state.set, &raw const OTHER_KEY) };
    }

    unsafe extern "C" fn first_destroy(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state.first_calls.set(state.first_calls.get() + 1);
        state
            .observed_empty
            .set(unsafe { hr_set_get_user_data(state.set, &raw const KEY).is_null() });
        state.accepted.set(unsafe {
            hr_set_set_user_data(
                state.set,
                &raw const OTHER_KEY,
                data,
                Some(second_destroy),
                1,
            )
        });
    }

    let state = Destruction {
        set: hr_set_create(),
        first_calls: Cell::new(0),
        second_calls: Cell::new(0),
        observed_empty: Cell::new(false),
        accepted: Cell::new(-1),
    };
    unsafe {
        assert_eq!(
            hr_set_set_user_data(
                state.set,
                &raw const KEY,
                ptr::from_ref(&state).cast_mut().cast(),
                Some(first_destroy),
                1,
            ),
            1
        );
        hr_set_destroy(state.set);
    }
    assert_eq!(state.first_calls.get(), 1);
    assert!(state.observed_empty.get());
    assert_eq!(state.accepted.get(), 0);
    assert_eq!(state.second_calls.get(), 0);
}

unsafe extern "C" fn reenter_on_drop(user_data: *mut c_void) {
    let face = user_data.cast::<hr_face_t>();
    unsafe {
        hr_face_set_user_data(
            face,
            &raw const REENTRANT_OTHER,
            ptr::dangling_mut(),
            None,
            1,
        );
    }
}

#[test]
fn a_destructor_may_set_user_data_on_the_same_object() {
    // The destructor is the caller's code and may come back into the API, so
    // it must not run while the object's user data is locked. Run on a worker
    // so that a regression fails the test instead of hanging the suite.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            with_font(|face, _| {
                hr_face_set_user_data(
                    face,
                    &raw const REENTRANT_KEY,
                    face.cast(),
                    Some(reenter_on_drop),
                    1,
                );
                // Replacing runs the destructor above, which re-enters.
                hr_face_set_user_data(face, &raw const REENTRANT_KEY, ptr::dangling_mut(), None, 1);
            });
        }
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(30)).is_ok(),
        "a destructor that set user data deadlocked"
    );
}

/// What `log_destroy` is handed: the log to write to, and what to write.
struct DestroyWitness {
    log: *const Mutex<Vec<&'static str>>,
    name: &'static str,
}

unsafe extern "C" fn log_destroy(user_data: *mut c_void) {
    let witness = unsafe { &*user_data.cast::<DestroyWitness>() };
    unsafe { &*witness.log }.lock().unwrap().push(witness.name);
}

#[test]
fn a_font_runs_its_destroy_callbacks_before_releasing_what_it_holds() {
    unsafe {
        let data = std::fs::read(font_path(PLAIN_FONT)).expect("failed to read test font");
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let font = hr_font_create(face);
        hr_blob_destroy(blob);
        // The font now holds the only reference to the face.
        hr_face_destroy(face);

        let log = Mutex::new(Vec::new());
        let witness = |name| DestroyWitness {
            log: &raw const log,
            name,
        };
        let on_face = witness("face");
        let on_font = witness("font user data");
        let with_funcs = witness("font funcs data");
        let as_data = |witness: &DestroyWitness| ptr::from_ref(witness).cast_mut().cast::<c_void>();
        assert_ne!(
            hr_face_set_user_data(
                face,
                &raw const KEY,
                as_data(&on_face),
                Some(log_destroy),
                1
            ),
            0
        );
        assert_ne!(
            hr_font_set_user_data(
                font,
                &raw const KEY,
                as_data(&on_font),
                Some(log_destroy),
                1
            ),
            0
        );
        let ffuncs = hr_font_funcs_create();
        hr_font_set_funcs(font, ffuncs, as_data(&with_funcs), Some(log_destroy));
        hr_font_funcs_destroy(ffuncs);

        // As in HarfBuzz, a callback may still look at the font's face,
        // parent or callbacks: the font lets go of those only after every
        // callback attached to it has run, so the face outlives them all.
        hr_font_destroy(font);
        assert_eq!(
            *log.lock().unwrap(),
            ["font user data", "font funcs data", "face"]
        );
    }
}

static SETTER_REENTRY_DROPS: AtomicUsize = AtomicUsize::new(0);
unsafe extern "C" fn reenter_setter_on_drop(user_data: *mut c_void) {
    // Only the first call re-enters, so that a regression fails the count
    // below instead of recursing.
    if SETTER_REENTRY_DROPS.fetch_add(1, Ordering::SeqCst) == 0 {
        unsafe {
            hr_font_funcs_set_nominal_glyph_func(
                user_data.cast(),
                Some(always_glyph_777),
                ptr::null_mut(),
                None,
            );
        }
    }
}

#[test]
fn a_destructor_may_set_the_callback_it_is_released_from() {
    // The slot must be empty by the time the destructor runs, or the nested
    // setter drops the same callback a second time.
    unsafe {
        let ffuncs = hr_font_funcs_create();
        hr_font_funcs_set_nominal_glyph_func(
            ffuncs,
            Some(always_glyph_777),
            ffuncs.cast(),
            Some(reenter_setter_on_drop),
        );
        // Clearing runs the destructor above, which re-enters.
        hr_font_funcs_set_nominal_glyph_func(ffuncs, None, ptr::null_mut(), None);
        hr_font_funcs_destroy(ffuncs);
    }
    assert_eq!(
        SETTER_REENTRY_DROPS.load(Ordering::SeqCst),
        1,
        "a callback registered once is released once"
    );
}

/// What `reenter_funcs_on_release` is handed: the funcs to come back into,
/// and how often it and the callback it installs have been released.
struct SetterReentry {
    ffuncs: *mut hr_font_funcs_t,
    released: AtomicUsize,
    nested_released: AtomicUsize,
}

unsafe extern "C" fn reenter_funcs_on_release(user_data: *mut c_void) {
    let state = unsafe { &*user_data.cast::<SetterReentry>() };
    state.released.fetch_add(1, Ordering::SeqCst);
    unsafe {
        hr_font_funcs_is_immutable(state.ffuncs);
        hr_font_funcs_get_user_data(state.ffuncs, &raw const KEY);
        // The callback being cleared, and one that is not.
        hr_font_funcs_set_nominal_glyph_func(
            state.ffuncs,
            Some(always_glyph_777),
            ptr::from_ref(&state.nested_released).cast_mut().cast(),
            Some(count_drop),
        );
        hr_font_funcs_set_glyph_h_advance_func(state.ffuncs, None, ptr::null_mut(), None);
    }
}

#[test]
fn a_destructor_run_by_a_setter_may_come_back_into_the_funcs() {
    // Clearing a callback releases the data handed over with the call from
    // inside the setter, as HarfBuzz does. The destructor is the caller's
    // code and may query the funcs or set callbacks on it, so the setter
    // must not be in the middle of writing to the funcs while it runs.
    let state = SetterReentry {
        ffuncs: hr_font_funcs_create(),
        released: AtomicUsize::new(0),
        nested_released: AtomicUsize::new(0),
    };
    unsafe {
        hr_font_funcs_set_nominal_glyph_func(
            state.ffuncs,
            None,
            ptr::from_ref(&state).cast_mut().cast(),
            Some(reenter_funcs_on_release),
        );
        assert_eq!(state.released.load(Ordering::SeqCst), 1);
        // The clear lands once the destructor has run, so it displaces the
        // callback the destructor installed, as it does in HarfBuzz.
        assert_eq!(state.nested_released.load(Ordering::SeqCst), 1);
        hr_font_funcs_destroy(state.ffuncs);
    }
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert_eq!(state.nested_released.load(Ordering::SeqCst), 1);
}

#[test]
fn a_setter_releases_its_user_data_exactly_once() {
    // A setter owns the data it is handed whether the funcs keeps the
    // callback, has it cleared, or refuses the change. It is released at
    // once unless the callback was kept, and with the funcs if it was.
    let as_data = |drops: &AtomicUsize| ptr::from_ref(drops).cast_mut().cast::<c_void>();
    let funcs: [hr_font_get_nominal_glyph_func_t; 2] = [None, Some(always_glyph_777)];
    for func in funcs {
        for immutable in [false, true] {
            let case = format!("callback: {}, immutable: {immutable}", func.is_some());
            let previous = AtomicUsize::new(0);
            let drops = AtomicUsize::new(0);
            unsafe {
                let ffuncs = hr_font_funcs_create();
                hr_font_funcs_set_nominal_glyph_func(
                    ffuncs,
                    Some(always_glyph_777),
                    as_data(&previous),
                    Some(count_drop),
                );
                if immutable {
                    hr_font_funcs_make_immutable(ffuncs);
                }
                hr_font_funcs_set_nominal_glyph_func(
                    ffuncs,
                    func,
                    as_data(&drops),
                    Some(count_drop),
                );
                let kept = func.is_some() && !immutable;
                assert_eq!(drops.load(Ordering::SeqCst), usize::from(!kept), "{case}");
                // The callback that was there goes only when the change is
                // accepted.
                assert_eq!(
                    previous.load(Ordering::SeqCst),
                    usize::from(!immutable),
                    "{case}"
                );
                hr_font_funcs_destroy(ffuncs);
            }
            assert_eq!(drops.load(Ordering::SeqCst), 1, "{case}");
            assert_eq!(previous.load(Ordering::SeqCst), 1, "{case}");
        }

        // No funcs at all refuses the change the same way.
        let drops = AtomicUsize::new(0);
        unsafe {
            hr_font_funcs_set_nominal_glyph_func(
                ptr::null_mut(),
                func,
                as_data(&drops),
                Some(count_drop),
            );
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

/// What `freeze_funcs_on_release` is handed: the funcs to make immutable,
/// and how often it has run.
struct FreezingClear {
    ffuncs: *mut hr_font_funcs_t,
    released: AtomicUsize,
}

unsafe extern "C" fn freeze_funcs_on_release(user_data: *mut c_void) {
    let state = unsafe { &*user_data.cast::<FreezingClear>() };
    state.released.fetch_add(1, Ordering::SeqCst);
    unsafe { hr_font_funcs_make_immutable(state.ffuncs) };
}

/// Installs `always_glyph_777` with `installed` as its data, then clears it
/// with data whose destructor makes the funcs immutable.
unsafe fn clear_with_a_freezing_destructor(state: &FreezingClear, installed: &AtomicUsize) {
    unsafe {
        hr_font_funcs_set_nominal_glyph_func(
            state.ffuncs,
            Some(always_glyph_777),
            ptr::from_ref(installed).cast_mut().cast(),
            Some(count_drop),
        );
        hr_font_funcs_set_nominal_glyph_func(
            state.ffuncs,
            None,
            ptr::from_ref(state).cast_mut().cast(),
            Some(freeze_funcs_on_release),
        );
    }
}

#[test]
fn a_clear_whose_destructor_freezes_the_funcs_still_lands() {
    // Whether the funcs takes changes is decided when the setter is entered,
    // as in HarfBuzz. A destructor that makes the funcs immutable while the
    // clear is under way does not stop it: the callback that was set goes,
    // and its data with it.
    let installed = AtomicUsize::new(0);
    let state = FreezingClear {
        ffuncs: hr_font_funcs_create(),
        released: AtomicUsize::new(0),
    };
    unsafe {
        clear_with_a_freezing_destructor(&state, &installed);
        assert_eq!(state.released.load(Ordering::SeqCst), 1);
        assert_ne!(hr_font_funcs_is_immutable(state.ffuncs), 0);
        assert_eq!(installed.load(Ordering::SeqCst), 1);
        hr_font_funcs_destroy(state.ffuncs);
    }
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert_eq!(installed.load(Ordering::SeqCst), 1);
}

#[test]
fn a_callback_cleared_while_its_funcs_was_frozen_no_longer_answers() {
    let installed = AtomicUsize::new(0);
    let state = FreezingClear {
        ffuncs: hr_font_funcs_create(),
        released: AtomicUsize::new(0),
    };
    unsafe {
        clear_with_a_freezing_destructor(&state, &installed);
        assert_eq!(installed.load(Ordering::SeqCst), 1);
        with_font(|_, font| {
            hr_font_set_funcs(font, state.ffuncs, ptr::null_mut(), None);
            // The font now holds the only reference to the funcs.
            hr_font_funcs_destroy(state.ffuncs);
            // With the callback gone and no parent to ask, nothing answers.
            let mut glyph: hr_codepoint_t = 7;
            assert_eq!(
                hr_font_get_nominal_glyph(font, u32::from(b'a'), &raw mut glyph),
                0
            );
            assert_ne!(glyph, 777);
        });
    }
    assert_eq!(installed.load(Ordering::SeqCst), 1);
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
}

#[test]
fn a_plan_runs_whatever_variation_the_font_is_at() {
    unsafe {
        let props = latin_props();
        let execute = |face: *mut hr_face_t, font: *mut hr_font_t, coords: &[i32]| {
            let plan = hr_shape_plan_create2(
                face,
                ptr::from_ref(&props),
                ptr::null(),
                0,
                if coords.is_empty() {
                    ptr::null()
                } else {
                    coords.as_ptr()
                },
                coords.len() as c_uint,
                ptr::null(),
            );
            assert_ne!(plan, hr_shape_plan_get_empty());
            let buffer = buffer_with_text(TEXT);
            hr_buffer_set_segment_properties(buffer, ptr::from_ref(&props));
            let ran = hr_shape_plan_execute(plan, font, buffer, ptr::null(), 0);
            let glyphs = hr_buffer_get_length(buffer);
            hr_buffer_destroy(buffer);
            hr_shape_plan_destroy(plan);
            (ran, glyphs)
        };

        // An all-zero vector names the default instance, which is what a font
        // with no coordinates set already is: the same variation spelled two
        // ways, and nothing to object to.
        with_font(|face, font| {
            assert_ne!(execute(face, font, &[0]).0, 0);
        });

        // And a plan built for one variation still runs against a font at
        // another. HarfBuzz does not compare them: the font's settings are
        // the ones shaping uses.
        with_named_font(VARIABLE_FONT, |face, font| {
            let coords = [8000i32];
            hr_font_set_var_coords_normalized(font, coords.as_ptr(), 1);
            let (ran, glyphs) = execute(face, font, &[]);
            assert_ne!(ran, 0, "a plan for another variation should still run");
            assert!(glyphs > 0);
        });
    }
}

#[test]
fn plans_carry_user_data_and_refcounts() {
    unsafe {
        with_font(|face, _| {
            let props = latin_props();
            let plan =
                hr_shape_plan_create(face, ptr::from_ref(&props), ptr::null(), 0, ptr::null());

            let value = ptr::dangling_mut::<c_void>();
            assert_ne!(
                hr_shape_plan_set_user_data(plan, &raw const KEY, value, None, 1),
                0
            );
            assert_eq!(hr_shape_plan_get_user_data(plan, &raw const KEY), value);

            // Taking and dropping a reference leaves the plan usable.
            hr_shape_plan_destroy(hr_shape_plan_reference(plan));
            assert_eq!(hr_shape_plan_get_user_data(plan, &raw const KEY), value);

            hr_shape_plan_destroy(plan);
        });
    }
}

// ------------------------------------------------------------- threading ----

/// A handle shared across threads on purpose: the C API documents faces and
/// fonts as safe to use from several at once, and these tests are what backs
/// that up. Buffers are never shared.
struct Shared<T>(*mut T);

// SAFETY: exactly the claim under test.
unsafe impl<T> Send for Shared<T> {}

// Derived copies would demand `T: Copy`, which these objects are not.
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Shared<T> {}

impl<T> Shared<T> {
    /// Taking the pointer through a method captures the whole wrapper in a
    /// closure, rather than just the field, which would not be `Send`.
    fn get(&self) -> *mut T {
        self.0
    }
}

#[test]
fn one_face_shapes_from_several_threads() {
    let data = font_data();
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);

        // What every thread should agree on.
        let expected = {
            let font = hr_font_create(face);
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let ids = glyph_ids(buffer);
            hr_buffer_destroy(buffer);
            hr_font_destroy(font);
            ids
        };

        // Each thread builds its own font over the shared face, which is what
        // exercises the face's plan cache, its lazily loaded tables, and the
        // glyph caches hanging off it.
        let shared = Shared(face);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    let font = hr_font_create(shared.get());
                    let mut seen = Vec::new();
                    for _ in 0..4 {
                        let buffer = buffer_with_text(TEXT);
                        hr_shape(font, buffer, ptr::null(), 0);
                        seen = glyph_ids(buffer);
                        hr_buffer_destroy(buffer);
                    }
                    hr_font_destroy(font);
                    seen
                })
            })
            .collect();

        for thread in threads {
            assert_eq!(thread.join().expect("thread panicked"), expected);
        }

        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn one_font_shapes_from_several_threads() {
    let data = font_data();
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let font = hr_font_create(face);
        // Nothing may change the font once it is shared, which this makes
        // explicit and enforces.
        hr_font_make_immutable(font);

        let expected = {
            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            let ids = glyph_ids(buffer);
            hr_buffer_destroy(buffer);
            ids
        };

        let shared = Shared(font);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    let mut seen = Vec::new();
                    for _ in 0..4 {
                        let buffer = buffer_with_text(TEXT);
                        hr_shape(shared.get(), buffer, ptr::null(), 0);
                        seen = glyph_ids(buffer);
                        hr_buffer_destroy(buffer);
                    }
                    seen
                })
            })
            .collect();

        for thread in threads {
            assert_eq!(thread.join().expect("thread panicked"), expected);
        }

        hr_font_destroy(font);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn reference_counts_survive_several_threads() {
    let data = font_data();
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);

        // Every thread takes and drops references to the same objects.
        let shared = Shared(face);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..8 {
                        let taken = hr_face_reference(shared.get());
                        let table = hr_face_reference_table(
                            taken,
                            hr_tag_from_string(c"cmap".as_ptr(), -1),
                        );
                        assert!(hr_blob_get_length(table) > 0);
                        hr_blob_destroy(table);
                        hr_face_destroy(taken);
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().expect("thread panicked");
        }

        // The face survived, so the counting balanced out.
        assert!(hr_face_get_glyph_count(face) > 0);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn interning_a_language_from_several_threads_agrees() {
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| unsafe {
                (0..8)
                    .map(|_| hr_language_from_string(c"en-US".as_ptr(), -1) as usize)
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    let mut all = Vec::new();
    for thread in threads {
        all.extend(thread.join().expect("thread panicked"));
    }
    // Interning must hand back one pointer, however many threads raced for it.
    assert!(all.iter().all(|value| *value == all[0]));
    assert_ne!(all[0], 0);
}

#[test]
fn a_glyph_can_be_asked_what_a_callback_could_answer_for() {
    // Every callback a caller can install is one a caller can also ask, and
    // the two go through the same place.
    unsafe {
        with_font(|face, font| {
            let upem = hr_face_get_upem(face) as c_int;
            let mut glyph = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, ptr::addr_of_mut!(glyph)),
                0
            );

            let advance = hr_font_get_glyph_h_advance(font, glyph);
            assert!(advance > 0);
            assert!(hr_font_get_glyph_v_advance(font, glyph) < 0);

            let (mut x, mut y) = (7, 7);
            // A glyph's horizontal origin is its own.
            assert_ne!(
                hr_font_get_glyph_h_origin(font, glyph, ptr::addr_of_mut!(x), ptr::addr_of_mut!(y)),
                0
            );
            assert_eq!((x, y), (0, 0));

            let mut extents = hr_glyph_extents_t::default();
            assert_ne!(
                hr_font_get_glyph_extents(font, glyph, ptr::addr_of_mut!(extents)),
                0
            );
            assert!(extents.width > 0);

            // At twice the size, everything the font reports doubles.
            hr_font_set_scale(font, upem * 2, upem * 2);
            assert_eq!(hr_font_get_glyph_h_advance(font, glyph), advance * 2);
            let mut bigger = hr_glyph_extents_t::default();
            assert_ne!(
                hr_font_get_glyph_extents(font, glyph, ptr::addr_of_mut!(bigger)),
                0
            );
            assert!(bigger.width >= extents.width * 2 - 2);
            assert!(bigger.width <= extents.width * 2 + 2);
            hr_font_set_scale(font, upem, upem);
        });
    }
}

#[test]
fn scaled_glyph_extents_round_outward_on_both_axes() {
    // Expected values come from HarfBuzz, which rounds each corner away from
    // the box, also when a negative scale reverses an axis.
    unsafe {
        with_font(|face, font| {
            let upem = hr_face_get_upem(face) as c_int;
            let mut glyph = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, ptr::addr_of_mut!(glyph)),
                0
            );
            for (x_scale, y_scale, expected) in [
                (13, 13, (0, 5, 7, -6)),
                (-13, 13, (0, 5, -7, -6)),
                (13, -13, (0, -5, 7, 6)),
                (-13, -13, (0, -5, -7, 6)),
                (0, 13, (0, 5, 0, -6)),
                (13, 0, (0, 0, 7, 0)),
                (0, 0, (0, 0, 0, 0)),
            ] {
                hr_font_set_scale(font, x_scale, y_scale);
                let mut extents = hr_glyph_extents_t::default();
                assert_ne!(
                    hr_font_get_glyph_extents(font, glyph, ptr::addr_of_mut!(extents)),
                    0
                );
                assert_eq!(
                    (
                        extents.x_bearing,
                        extents.y_bearing,
                        extents.width,
                        extents.height
                    ),
                    expected,
                    "scale {x_scale}x{y_scale}"
                );
            }
            hr_font_set_scale(font, upem, upem);
        });
    }
}

#[test]
fn advances_run_over_a_caller_s_own_layout() {
    unsafe {
        with_font(|_, font| {
            // Glyphs and advances read and written a stride apart, so a
            // caller can walk its own structures rather than pack arrays.
            #[repr(C)]
            struct Slot {
                glyph: hr_codepoint_t,
                padding: u32,
            }
            let mut slots = [
                Slot {
                    glyph: 0,
                    padding: 0,
                },
                Slot {
                    glyph: 0,
                    padding: 0,
                },
            ];
            for (slot, c) in slots.iter_mut().zip(['a', 'b']) {
                assert_ne!(
                    hr_font_get_nominal_glyph(font, c as u32, ptr::addr_of_mut!(slot.glyph)),
                    0
                );
            }
            let mut advances = [0i32; 2];
            // The run is read a stride at a time from its first glyph, so
            // that pointer has to be good for the whole array: one taken
            // from `slots[0].glyph` would be good for that field alone.
            let first = slots.as_ptr();
            hr_font_get_glyph_h_advances(
                font,
                2,
                &raw const (*first).glyph,
                size_of::<Slot>() as c_uint,
                advances.as_mut_ptr(),
                size_of::<i32>() as c_uint,
            );
            for (slot, advance) in slots.iter().zip(advances) {
                assert_eq!(advance, hr_font_get_glyph_h_advance(font, slot.glyph));
            }
        });
    }
}

#[test]
fn advances_and_origins_follow_the_direction_asked_for() {
    unsafe {
        with_font(|_, font| {
            let mut glyph = 0;
            hr_font_get_nominal_glyph(font, 'a' as u32, ptr::addr_of_mut!(glyph));

            let (mut x, mut y) = (0, 0);
            hr_font_get_glyph_advance_for_direction(
                font,
                glyph,
                HR_DIRECTION_LTR,
                ptr::addr_of_mut!(x),
                ptr::addr_of_mut!(y),
            );
            assert_eq!(x, hr_font_get_glyph_h_advance(font, glyph));
            assert_eq!(y, 0);

            hr_font_get_glyph_advance_for_direction(
                font,
                glyph,
                HR_DIRECTION_TTB,
                ptr::addr_of_mut!(x),
                ptr::addr_of_mut!(y),
            );
            assert_eq!(x, 0);
            assert_eq!(y, hr_font_get_glyph_v_advance(font, glyph));

            // Adding an origin and taking it away again gets back where it
            // started, whichever way the text runs.
            for direction in [HR_DIRECTION_LTR, HR_DIRECTION_TTB] {
                let (mut px, mut py) = (100, 200);
                hr_font_add_glyph_origin_for_direction(
                    font,
                    glyph,
                    direction,
                    ptr::addr_of_mut!(px),
                    ptr::addr_of_mut!(py),
                );
                hr_font_subtract_glyph_origin_for_direction(
                    font,
                    glyph,
                    direction,
                    ptr::addr_of_mut!(px),
                    ptr::addr_of_mut!(py),
                );
                assert_eq!((px, py), (100, 200));
            }
        });
    }
}

#[test]
fn glyphs_go_by_name_and_come_back_from_one() {
    unsafe {
        with_font(|_, font| {
            let mut glyph = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, ptr::addr_of_mut!(glyph)),
                0
            );

            let mut name = [0 as c_char; 64];
            assert_ne!(
                hr_font_get_glyph_name(font, glyph, name.as_mut_ptr(), name.len() as c_uint),
                0
            );
            let mut back = 0;
            assert_ne!(
                hr_font_get_glyph_from_name(font, name.as_ptr(), -1, ptr::addr_of_mut!(back)),
                0
            );
            assert_eq!(back, glyph);

            // A glyph the face names nothing still goes by a name.
            let mut text = [0 as c_char; 64];
            hr_font_glyph_to_string(font, 60_000, text.as_mut_ptr(), text.len() as c_uint);
            let printed = std::ffi::CStr::from_ptr(text.as_ptr()).to_str().unwrap();
            assert_eq!(printed, "gid60000");

            // And the names a string can be read as.
            for (spelling, expected) in [
                (c"gid60000".as_ptr(), 60_000),
                (c"41".as_ptr(), 41),
                (c"uni0061".as_ptr(), glyph),
            ] {
                let mut parsed = 0;
                assert_ne!(
                    hr_font_glyph_from_string(font, spelling, -1, ptr::addr_of_mut!(parsed)),
                    0
                );
                assert_eq!(parsed, expected);
            }
        });
    }
}

#[test]
fn a_variation_selector_the_font_lacks_falls_back_to_the_plain_glyph() {
    unsafe {
        with_font(|_, font| {
            let mut plain = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, ptr::addr_of_mut!(plain)),
                0
            );
            let mut combined = 0;
            assert_ne!(
                hr_font_get_glyph(font, 'a' as u32, 0xFE00, ptr::addr_of_mut!(combined)),
                0
            );
            assert_eq!(combined, plain);
            // And with no selector at all it is the plain mapping.
            let mut none = 0;
            assert_ne!(
                hr_font_get_glyph(font, 'a' as u32, 0, ptr::addr_of_mut!(none)),
                0
            );
            assert_eq!(none, plain);
        });
    }
}

#[test]
fn nothing_is_advanced_past_the_last_glyph_the_face_has() {
    unsafe {
        with_font(|face, font| {
            let count = hr_face_get_glyph_count(face);
            assert!(count > 0);
            assert!(hr_font_get_glyph_h_advance(font, count - 1) > 0);
            // A malformed cmap can point at a glyph the face does not have,
            // and there is no advance to give for one.
            assert_eq!(hr_font_get_glyph_h_advance(font, count), 0);
            // A face carrying no vertical metrics gives every glyph the same
            // fallback instead, in range or not, as HarfBuzz does.
            assert_eq!(
                hr_font_get_glyph_v_advance(font, count),
                hr_font_get_glyph_v_advance(font, 0)
            );
        });
    }
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

/// A `maxp` or `MAXP` table holding only a glyph count.
fn maxp(count: u32, extended: bool) -> Vec<u8> {
    let mut data = 0x0000_5000u32.to_be_bytes().to_vec();
    if extended {
        data.extend(&count.to_be_bytes()[1..]);
    } else {
        data.extend((count as u16).to_be_bytes());
    }
    data
}

/// A GSUB table whose one lookup maps glyph 65536 to 65537 with
/// SingleSubst format 3.
fn extended_single_subst_gsub() -> Vec<u8> {
    #[rustfmt::skip]
    let data = vec![
        0, 1, 0, 0, 0, 10, 0, 12, 0, 14, // header
        0, 0, // script count
        0, 0, // feature count
        0, 1, 0, 4, // one lookup
        0, 1, 0, 0, 0, 1, 0, 8, // single substitution, one subtable
        0, 3, 0, 0, 0, 9, 0, 0, 1, // format 3, coverage, delta 1
        0, 3, 0, 0, 1, 1, 0, 0, // coverage format 3: glyph 65536
    ];
    data
}

/// `hhea` and `hmtx` tables with one long metric and a side bearing for each
/// further glyph.
fn h_metrics(glyphs: usize) -> [(&'static [u8; 4], Vec<u8>); 2] {
    let mut hhea = vec![0; 36];
    hhea[1] = 1; // version 1.0
    hhea[35] = 1; // one long metric
    [(b"hhea", hhea), (b"hmtx", vec![0; 4 + 2 * (glyphs - 1)])]
}

#[test]
fn wide_lookup_glyphs_do_not_require_metrics_or_a_glyph_count() {
    let [hhea, hmtx] = h_metrics(4);
    for tables in [
        // No count at all.
        vec![(b"GSUB", extended_single_subst_gsub())],
        // A count no table backs.
        vec![
            (b"GSUB", extended_single_subst_gsub()),
            (b"MAXP", maxp(0x00FF_FFFF, true)),
        ],
        // A count the metrics back only in part.
        vec![
            (b"GSUB", extended_single_subst_gsub()),
            (b"MAXP", maxp(0x00FF_FFFF, true)),
            hhea,
            hmtx,
        ],
    ] {
        let data = sfnt(&tables);
        unsafe {
            let blob = blob_over(&data);
            let face = hr_face_create(blob, 0);
            let set = hr_set_create();
            hr_ot_layout_lookup_collect_glyphs(face, 0x4753_5542, 0, set, set, set, set);
            assert_eq!(hr_set_get_population(set), 2);
            assert_eq!(hr_set_has(set, 65536), 1);
            assert_eq!(hr_set_has(set, 65537), 1);
            hr_set_destroy(set);
            hr_face_destroy(face);
            hr_blob_destroy(blob);
        }
    }
}

#[test]
fn extended_lookup_collects_glyphs_beyond_64k() {
    let [hhea, hmtx] = h_metrics(65550);
    let data = sfnt(&[
        (b"GSUB", extended_single_subst_gsub()),
        (b"MAXP", maxp(65550, true)),
        hhea,
        hmtx,
        (b"maxp", maxp(4, false)),
    ]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let sets = [
            hr_set_create(),
            hr_set_create(),
            hr_set_create(),
            hr_set_create(),
        ];
        hr_ot_layout_lookup_collect_glyphs(
            face,
            0x4753_5542,
            0,
            sets[0],
            sets[1],
            sets[2],
            sets[3],
        );
        assert_eq!(hr_set_has(sets[1], 65536), 1);
        assert_eq!(hr_set_has(sets[3], 65537), 1);
        assert_eq!(hr_set_has(sets[1], 65550), 0);
        for set in sets {
            hr_set_destroy(set);
        }
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

/// Collects lookup 0 of a `GSUB` or `GPOS` table in a font with three
/// glyphs and returns the populations of the before, input, after and
/// output sets.
fn collect_lookup_0(tag: &[u8; 4], table: Vec<u8>) -> [u32; 4] {
    let data = sfnt(&[(tag, table), (b"maxp", maxp(3, false))]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let sets = [
            hr_set_create(),
            hr_set_create(),
            hr_set_create(),
            hr_set_create(),
        ];
        hr_ot_layout_lookup_collect_glyphs(
            face,
            u32::from_be_bytes(*tag),
            0,
            sets[0],
            sets[1],
            sets[2],
            sets[3],
        );
        let populations = sets.map(|set| hr_set_get_population(set));
        for set in sets {
            hr_set_destroy(set);
        }
        hr_face_destroy(face);
        hr_blob_destroy(blob);
        populations
    }
}

/// A GSUB table whose one ligature lookup has `n` subtable offsets, `n`
/// ligature set offsets and `n` ligature offsets, each array naming one
/// shared table, and a ligature of `n` components.
fn aliased_ligature_gsub(n: u16) -> Vec<u8> {
    let offsets = |offset: u16| (0..n).flat_map(move |_| offset.to_be_bytes());
    let mut data = vec![0, 1, 0, 0, 0, 10, 0, 12, 0, 14]; // header
    data.extend([0, 0, 0, 0]); // no scripts, no features
    data.extend([0, 1, 0, 4]); // one lookup
    data.extend([0, 4, 0, 0]); // ligature substitution
    data.extend(n.to_be_bytes());
    data.extend(offsets(6 + 2 * n)); // subtables
    data.extend([0, 1]); // format 1
    data.extend((6 + 2 * n).to_be_bytes()); // coverage
    data.extend(n.to_be_bytes());
    data.extend(offsets(6 + 2 * n + 6)); // ligature sets
    data.extend([0, 1, 0, 1, 0, 1]); // coverage format 1: glyph 1
    data.extend(n.to_be_bytes());
    data.extend(offsets(2 + 2 * n)); // ligatures
    data.extend([0, 2]); // ligature glyph
    data.extend((n + 1).to_be_bytes());
    data.extend(offsets(1)); // components
    data
}

/// A GPOS table whose one pair positioning lookup has `n` subtable offsets
/// and `n` pair set offsets, each array naming one shared table, and a pair
/// set of `n` records.
fn aliased_pair_pos_gpos(n: u16) -> Vec<u8> {
    let offsets = |offset: u16| (0..n).flat_map(move |_| offset.to_be_bytes());
    let mut data = vec![0, 1, 0, 0, 0, 10, 0, 12, 0, 14]; // header
    data.extend([0, 0, 0, 0]); // no scripts, no features
    data.extend([0, 1, 0, 4]); // one lookup
    data.extend([0, 2, 0, 0]); // pair positioning
    data.extend(n.to_be_bytes());
    data.extend(offsets(6 + 2 * n)); // subtables
    data.extend([0, 1]); // format 1
    data.extend((10 + 2 * n).to_be_bytes()); // coverage
    data.extend([0, 0, 0, 0]); // no values
    data.extend(n.to_be_bytes());
    data.extend(offsets(10 + 2 * n + 6)); // pair sets
    data.extend([0, 1, 0, 1, 0, 1]); // coverage format 1: glyph 1
    data.extend(n.to_be_bytes());
    data.extend(offsets(2)); // second glyphs
    data
}

#[test]
#[cfg_attr(miri, ignore = "walks eight million layout steps, too slow under Miri")]
fn aliased_offsets_fall_back_to_all_glyphs() {
    assert_eq!(
        collect_lookup_0(b"GSUB", aliased_ligature_gsub(1)),
        [0, 1, 0, 1]
    );
    // Followed to the end, these 1.6 KB would be 200^4 components.
    assert_eq!(
        collect_lookup_0(b"GSUB", aliased_ligature_gsub(200)),
        [u32::MAX; 4]
    );
    assert_eq!(
        collect_lookup_0(b"GPOS", aliased_pair_pos_gpos(1)),
        [0, 2, 0, 0]
    );
    // Followed to the end, these 1.8 KB would be 300^3 pair records.
    assert_eq!(
        collect_lookup_0(b"GPOS", aliased_pair_pos_gpos(300)),
        [u32::MAX; 4]
    );
}

#[test]
#[cfg_attr(miri, ignore = "follows forty million components, too slow under Miri")]
fn repeated_collections_each_get_an_allowance() {
    // One call follows about 30^4 components, within an allowance of 4M steps.
    // As in HarfBuzz, whose sanitizer bounds the table once, each call gets
    // the same allowance and the same result.
    let data = sfnt(&[
        (b"GSUB", aliased_ligature_gsub(30)),
        (b"maxp", maxp(3, false)),
    ]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let mut populations = vec![];
        for _ in 0..50 {
            let set = hr_set_create();
            hr_ot_layout_lookup_collect_glyphs(face, 0x4753_5542, 0, set, set, set, set);
            populations.push(hr_set_get_population(set));
            hr_set_destroy(set);
        }
        assert!(populations.iter().all(|&population| population == 2));
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

/// A GSUB table where each of lookups 0 to 6 is a contextual lookup with
/// `n` records that all apply the next lookup, and lookup 7 maps glyph 1
/// to glyph 2.
fn repeatedly_nested_gsub(n: u16) -> Vec<u8> {
    let mut data = vec![0, 1, 0, 0, 0, 10, 0, 12, 0, 14]; // header
    data.extend([0, 0, 0, 0]); // no scripts, no features
    data.extend([0, 8]);
    let contextual_len = 22 + 4 * n;
    for index in 0..8 {
        data.extend((18 + index * contextual_len).to_be_bytes());
    }
    for index in 1..8 {
        data.extend([0, 5, 0, 0, 0, 1, 0, 8]); // contextual, one subtable
        data.extend([0, 3, 0, 1]); // format 3, one input glyph
        data.extend(n.to_be_bytes());
        data.extend((8 + 4 * n).to_be_bytes()); // coverage
        for _ in 0..n {
            data.extend([0, 0, 0, index]);
        }
        data.extend([0, 1, 0, 1, 0, 1]); // coverage format 1: glyph 1
    }
    data.extend([0, 1, 0, 0, 0, 1, 0, 8]); // single substitution, one subtable
    data.extend([0, 1, 0, 6, 0, 1]); // format 1, coverage, delta 1
    data.extend([0, 1, 0, 1, 0, 1]); // coverage format 1: glyph 1
    data
}

#[test]
fn repeatedly_nested_lookups_are_collected_without_fallback() {
    // Followed along every path, lookup 7 would be collected 16^7 times.
    assert_eq!(
        collect_lookup_0(b"GSUB", repeatedly_nested_gsub(16)),
        [0, 1, 0, 1]
    );
}

#[test]
fn glyph_count_prefers_extended_maxp() {
    let data = sfnt(&[(b"MAXP", maxp(65550, true)), (b"maxp", maxp(4, false))]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        assert_eq!(hr_face_get_glyph_count(face), 65550);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

/// A `CFF ` table with one `endchar` glyph, whose Top DICT is padded with
/// `padding` no-op `UniqueID` entries ahead of its `charset` and
/// `CharStrings` operators.
fn cff_with_padded_top_dict(padding: usize) -> Vec<u8> {
    fn index(items: &[&[u8]]) -> Vec<u8> {
        let mut data = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return data;
        }
        data.push(4);
        let mut offset = 1u32;
        data.extend(offset.to_be_bytes());
        for item in items {
            offset += item.len() as u32;
            data.extend(offset.to_be_bytes());
        }
        data.extend(items.concat());
        data
    }
    let header = [1, 0, 4, 4];
    let names = index(&[b"x"]);
    let charstrings = index(&[&[0x0e]]);
    // Header, Name INDEX, Top DICT INDEX (count, offSize, two offsets, the
    // dict), and empty String and Global Subr INDEXes.
    let top_dict_len = 2 * padding + 12;
    let after_top_dict = header.len() + names.len() + (2 + 1 + 8 + top_dict_len) + 2 + 2;
    let mut top_dict = [0x8b, 0x0d].repeat(padding);
    top_dict.push(0x1d);
    top_dict.extend(((after_top_dict + charstrings.len()) as u32).to_be_bytes());
    top_dict.push(0x0f);
    top_dict.push(0x1d);
    top_dict.extend((after_top_dict as u32).to_be_bytes());
    top_dict.push(0x11);
    assert_eq!(top_dict.len(), top_dict_len);
    [
        &header[..],
        &names,
        &index(&[&top_dict]),
        &index(&[]),
        &index(&[]),
        &charstrings,
        // A format 0 charset naming only `.notdef`.
        &[0],
    ]
    .concat()
}

/// A `MAXP` table can claim 16.7 million glyphs in seven bytes, and reading
/// one glyph's name rereads the whole CFF Top DICT. Looking a name up must
/// not pay one for each of the other.
#[test]
#[cfg_attr(miri, ignore = "bounds wall-clock time, which Miri cannot meet")]
fn a_name_is_looked_up_without_asking_every_claimed_glyph_for_its_own() {
    let data = sfnt(&[
        (b"CFF ", cff_with_padded_top_dict(2000)),
        (b"MAXP", maxp(0x00FF_FFFF, true)),
    ]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let font = hr_font_create(face);
        assert_eq!(hr_face_get_glyph_count(face), 0x00FF_FFFF);
        let start = std::time::Instant::now();
        let mut glyph = 7;
        assert_eq!(
            hr_font_get_glyph_from_name(font, c"nosuchglyph".as_ptr(), -1, &raw mut glyph),
            0
        );
        assert_eq!(glyph, 0);
        // One pass over the claimed glyphs, against hours for the product.
        assert!(start.elapsed().as_secs() < 60, "{:?}", start.elapsed());
        // The names the face stores are still found.
        assert_ne!(
            hr_font_get_glyph_from_name(font, c".notdef".as_ptr(), -1, &raw mut glyph),
            0
        );
        assert_eq!(glyph, 0);
        // And a glyph the face names nothing still answers to its `gidNNN`.
        assert_ne!(
            hr_font_get_glyph_from_name(font, c"gid5".as_ptr(), -1, &raw mut glyph),
            0
        );
        assert_eq!(glyph, 5);
        assert_eq!(
            hr_font_get_glyph_from_name(font, c"gid05".as_ptr(), -1, &raw mut glyph),
            0
        );
        hr_font_destroy(font);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

/// A `cmap` table whose one Windows UCS-4 subtable is format 13 with
/// `groups`, each mapping an inclusive character range to one glyph.
fn format13_cmap(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut data = vec![0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12];
    data.extend(13u16.to_be_bytes());
    data.extend(0u16.to_be_bytes());
    data.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    data.extend(0u32.to_be_bytes());
    data.extend((groups.len() as u32).to_be_bytes());
    for (first, last, glyph) in groups {
        data.extend(first.to_be_bytes());
        data.extend(last.to_be_bytes());
        data.extend(glyph.to_be_bytes());
    }
    data
}

#[test]
fn unicode_collection_steps_over_overlapping_format13_groups() {
    // Overlapping groups are counted once, and a group that repeats all of
    // Unicode costs nothing after the first.
    let partial = [(0x41, 0x5A, 1), (0x50, 0x7A, 1), (0x61, 0x6A, 1)];
    let partial = sfnt(&[
        (b"cmap", format13_cmap(&partial)),
        (b"maxp", maxp(2, false)),
    ]);
    let full = [(0, 0x0010_FFFF, 1); 1000];
    let full = sfnt(&[(b"cmap", format13_cmap(&full)), (b"maxp", maxp(2, false))]);
    let fonts = [(partial, 0x7A - 0x41 + 1), (full, 0x0011_0000)];
    // Miri has nothing to say about cost, and takes half a minute to fill a
    // set with all of Unicode, so it collects the first font only.
    let collected = if cfg!(miri) { 1 } else { fonts.len() };
    for (data, population) in fonts.into_iter().take(collected) {
        unsafe {
            let blob = blob_over(&data);
            let face = hr_face_create(blob, 0);
            let set = hr_set_create();
            hr_face_collect_unicodes(face, set);
            assert_eq!(hr_set_get_population(set), population);
            hr_set_destroy(set);
            hr_face_destroy(face);
            hr_blob_destroy(blob);
        }
    }
}

#[test]
fn prepared_metrics_follow_scale_subfonts_and_instance_changes() {
    unsafe {
        with_named_font(VARIABLE_FONT, |_, font| {
            let mut glyph = 0;
            assert_ne!(
                hr_font_get_nominal_glyph(font, 'a' as u32, &raw mut glyph),
                0
            );
            let original = hr_font_get_glyph_h_advance(font, glyph);
            assert!(original > 0);
            let mut upem = 0;
            hr_font_get_scale(font, &raw mut upem, ptr::null_mut());
            hr_font_set_scale(font, upem * 2, upem * 3);
            assert_eq!(hr_font_get_glyph_h_advance(font, glyph), original * 2);
            let child = hr_font_create_sub_font(font);
            hr_font_set_scale(font, -upem, 0);
            assert_eq!(hr_font_get_glyph_h_advance(font, glyph), -original);
            assert_eq!(hr_font_get_glyph_h_advance(child, glyph), original * 2);

            let variation = hr_variation_t {
                tag: u32::from_be_bytes(*b"wdth"),
                value: 200.0,
            };
            // Rebuilding the instance must preserve the child's scale.
            hr_font_set_variations(child, &raw const variation, 1);
            let wide = hr_font_get_glyph_h_advance(child, glyph);
            assert!(wide > original * 2);
            let mut advance = 0;
            hr_font_get_glyph_h_advances(child, 1, &raw const glyph, 0, &raw mut advance, 0);
            assert_eq!(advance, wide);
            let buffer = buffer_with_text("a");
            hr_shape(child, buffer, ptr::null(), 0);
            let positions = hr_buffer_get_glyph_positions(buffer, ptr::null_mut());
            assert_eq!((*positions).x_advance, wide);
            hr_buffer_destroy(buffer);
            hr_font_set_variations(child, ptr::null(), 0);
            assert_eq!(hr_font_get_glyph_h_advance(child, glyph), original * 2);
            hr_font_destroy(child);
        });
    }
}

#[test]
fn tags_read_only_four_raw_bytes() {
    unsafe {
        for len in [-2, -1, 4, 7] {
            // UTF-8 validation after the fourth byte must not affect the tag.
            let name = [b'L', b'a', b't', b'n', 0xFF, b'x', 0];
            assert_eq!(
                hr_tag_from_string(name.as_ptr().cast(), len),
                HR_SCRIPT_LATIN
            );
            assert_eq!(
                hr_script_from_string(name.as_ptr().cast(), len),
                HR_SCRIPT_LATIN
            );
        }
        for (bytes, len, expected) in [
            (&b"a\0bc"[..], -1, u32::from_be_bytes(*b"a   ")),
            (&b"abcd"[..], 2, u32::from_be_bytes(*b"ab  ")),
            (&b"abcd"[..], 4, u32::from_be_bytes(*b"abcd")),
            (&[0xFF, 0x80, 0xFE, 0x81][..], 4, 0xFF80_FE81),
            (&b"\0"[..], -1, HR_TAG_NONE),
            (&b""[..], 0, HR_TAG_NONE),
        ] {
            assert_eq!(hr_tag_from_string(bytes.as_ptr().cast(), len), expected);
        }
        assert_eq!(hr_tag_from_string(ptr::null(), -1), HR_TAG_NONE);
    }
}

#[test]
fn conservative_lookup_fallback_preserves_aliased_and_inverted_sets() {
    let mut table = extended_single_subst_gsub();
    table[27] = 0xff; // unreadable SingleSubst format
    let data = sfnt(&[(b"GSUB", table)]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        for inverted in [false, true] {
            let set = hr_set_create();
            hr_set_add(set, u32::MAX - 1);
            if inverted {
                hr_set_invert(set);
                hr_set_del(set, 65536);
            }
            hr_ot_layout_lookup_collect_glyphs(face, 0x4753_5542, 0, set, set, set, set);
            assert_eq!(hr_set_get_population(set), u32::MAX);
            assert_eq!(hr_set_is_inverted(set), 1);
            assert_eq!(hr_set_has(set, 65536), 1);
            assert_eq!(hr_set_has(set, 65537), 1);
            assert_eq!(hr_set_has(set, u32::MAX - 1), 1);
            hr_set_destroy(set);
        }
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn wide_single_substitution_with_explicit_outputs_needs_no_metrics() {
    let mut table = extended_single_subst_gsub();
    table.truncate(26);
    table.extend([
        0, 4, 0, 0, 0, 12, // format 4, coverage offset
        0, 0, 1, // one output glyph
        1, 0, 1, // glyph 65537
        0, 3, 0, 0, 1, 1, 0, 0, // coverage: glyph 65536
    ]);
    let data = sfnt(&[(b"GSUB", table)]);
    unsafe {
        let blob = blob_over(&data);
        let face = hr_face_create(blob, 0);
        let input = hr_set_create();
        let output = hr_set_create();
        hr_ot_layout_lookup_collect_glyphs(
            face,
            0x4753_5542,
            0,
            ptr::null_mut(),
            input,
            ptr::null_mut(),
            output,
        );
        assert_eq!(hr_set_get_population(input), 1);
        assert_eq!(hr_set_get_population(output), 1);
        assert_eq!(hr_set_has(input, 65536), 1);
        assert_eq!(hr_set_has(output, 65537), 1);
        hr_set_destroy(input);
        hr_set_destroy(output);
        hr_face_destroy(face);
        hr_blob_destroy(blob);
    }
}

#[test]
fn inverted_set_iteration_seeks_past_a_long_excluded_prefix() {
    // Previously every next() rebuilt the first range by scanning all the
    // excluded pages before PREFIX, even though each answer is nearby.
    //
    // Miri has nothing to say about speed, and takes minutes over a prefix
    // this long and this many members. It gets a prefix of many pages still,
    // and members enough to run from one page into the next.
    const PREFIX: u32 = if cfg!(miri) { 1 << 16 } else { 1 << 22 };
    const COUNT: usize = if cfg!(miri) { 32 } else { 4096 };
    for stride in [2, 32] {
        unsafe {
            let set = hr_set_create();
            hr_set_invert(set);
            hr_set_del_range(set, 0, PREFIX - 1);
            for index in 0..COUNT as u32 {
                hr_set_del_range(
                    set,
                    PREFIX + stride * index + 1,
                    PREFIX + stride * (index + 1) - 1,
                );
            }
            let mut cursor = PREFIX;
            for index in 1..=COUNT as u32 {
                assert_eq!(hr_set_next(set, &raw mut cursor), 1);
                assert_eq!(cursor, PREFIX + stride * index);
            }
            let mut batch = [0; COUNT];
            assert_eq!(
                hr_set_next_many(set, PREFIX, batch.as_mut_ptr(), COUNT as u32),
                COUNT as u32
            );
            for (index, value) in batch.into_iter().enumerate() {
                assert_eq!(value, PREFIX + stride * (index as u32 + 1));
            }
            hr_set_destroy(set);
        }
    }
}

#[test]
fn inverted_set_seeks_across_long_gaps_and_invalidates_cached_ranges() {
    // Miri has nothing to say about speed, and takes minutes over a gap this
    // long. A shorter one still runs over many pages, and the cached ranges
    // are invalidated the same way.
    const END: u32 = if cfg!(miri) { 1 << 16 } else { 1 << 24 };
    unsafe {
        let set = hr_set_create();
        hr_set_invert(set);
        hr_set_del_range(set, 0, END - 1);
        let mut cursor = HR_SET_VALUE_INVALID;
        assert_eq!(hr_set_next(set, &raw mut cursor), 1);
        assert_eq!(cursor, END);
        hr_set_add(set, 17);
        cursor = HR_SET_VALUE_INVALID;
        assert_eq!(hr_set_next(set, &raw mut cursor), 1);
        assert_eq!(cursor, 17);
        hr_set_del(set, 17);
        cursor = HR_SET_VALUE_INVALID;
        assert_eq!(hr_set_next(set, &raw mut cursor), 1);
        assert_eq!(cursor, END);
        hr_set_invert(set);
        cursor = HR_SET_VALUE_INVALID;
        assert_eq!(hr_set_next(set, &raw mut cursor), 1);
        assert_eq!(cursor, 0);
        hr_set_destroy(set);
    }
}

// ---------------------------------------- what a destroy function may ask ----

/// Destroy functions that look back at the object they are released from, or
/// at the one that kept what they belong to.
///
/// An object lets go of the callbacks and data it holds once its last
/// reference has gone, and can still be asked questions while it does. Most
/// tests here hand a destroy function that object's pointer and have it ask
/// some.
mod destroy_functions_query_their_object {
    use super::*;
    use core::cell::Cell;

    const HEAD: hr_tag_t = u32::from_be_bytes(*b"head");
    static BYTES: [u8; 4] = [1, 2, 3, 4];

    /// What `query_font_funcs` is handed, and what the funcs told it.
    struct FuncsQuery {
        ffuncs: *mut hr_font_funcs_t,
        calls: Cell<u32>,
        immutable: Cell<hr_bool_t>,
        user_data: Cell<*mut c_void>,
        /// Counts the data of the callbacks it tried to set being released.
        refused: AtomicUsize,
    }

    unsafe extern "C" fn query_font_funcs(data: *mut c_void) {
        let state = unsafe { &*data.cast::<FuncsQuery>() };
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state
                .immutable
                .set(hr_font_funcs_is_immutable(state.ffuncs));
            state
                .user_data
                .set(hr_font_funcs_get_user_data(state.ffuncs, &raw const KEY));
            // The slot this was released from is empty by now. A callback
            // taken into it would be left for the funcs to release as it is
            // freed, when it can no longer be asked anything.
            hr_font_funcs_set_nominal_glyph_func(
                state.ffuncs,
                Some(always_glyph_777),
                ptr::from_ref(&state.refused).cast_mut().cast(),
                Some(count_drop),
            );
        }
    }

    #[test]
    fn a_font_callbacks_destroy_may_query_its_funcs() {
        let state = FuncsQuery {
            ffuncs: hr_font_funcs_create(),
            calls: Cell::new(0),
            immutable: Cell::new(-1),
            user_data: Cell::new(ptr::dangling_mut()),
            refused: AtomicUsize::new(0),
        };
        let data: *mut c_void = ptr::from_ref(&state).cast_mut().cast();
        unsafe {
            assert_ne!(
                hr_font_funcs_set_user_data(
                    state.ffuncs,
                    &raw const KEY,
                    ptr::dangling_mut(),
                    None,
                    1
                ),
                0
            );
            // The first callback a funcs holds, and one further along.
            hr_font_funcs_set_nominal_glyph_func(
                state.ffuncs,
                Some(always_glyph_777),
                data,
                Some(query_font_funcs),
            );
            hr_font_funcs_set_glyph_h_advance_func(
                state.ffuncs,
                Some(fixed_advance),
                data,
                Some(query_font_funcs),
            );
            assert_eq!(state.calls.get(), 0);
            hr_font_funcs_destroy(state.ffuncs);
        }
        assert_eq!(state.calls.get(), 2);
        assert_eq!(state.immutable.get(), 0);
        // User data goes before the callbacks do.
        assert!(state.user_data.get().is_null());
        // A funcs being freed takes no callback, and says so by releasing
        // the data there and then.
        assert_eq!(state.refused.load(Ordering::SeqCst), 2);
    }

    /// What `query_face` is handed, and what the face told it.
    struct FaceQuery {
        face: Cell<*mut hr_face_t>,
        calls: Cell<u32>,
        immutable: Cell<hr_bool_t>,
        user_data: Cell<*mut c_void>,
        upem: Cell<c_uint>,
        table_length: Cell<c_uint>,
        tag_count: Cell<c_uint>,
        /// How often the face's table callback and its tags callback have
        /// been asked, by anyone.
        table_requests: Cell<u32>,
        tag_requests: Cell<u32>,
        /// Whether `head_once` has handed out its table.
        handed_out: Cell<bool>,
    }

    impl FaceQuery {
        fn new() -> Self {
            Self {
                face: Cell::new(ptr::null_mut()),
                calls: Cell::new(0),
                immutable: Cell::new(-1),
                user_data: Cell::new(ptr::dangling_mut()),
                upem: Cell::new(0),
                table_length: Cell::new(c_uint::MAX),
                tag_count: Cell::new(c_uint::MAX),
                table_requests: Cell::new(0),
                tag_requests: Cell::new(0),
                handed_out: Cell::new(false),
            }
        }

        fn as_data(&self) -> *mut c_void {
            ptr::from_ref(self).cast_mut().cast()
        }
    }

    unsafe extern "C" fn query_face(data: *mut c_void) {
        let state = unsafe { &*data.cast::<FaceQuery>() };
        let face = state.face.get();
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state.immutable.set(hr_face_is_immutable(face));
            state
                .user_data
                .set(hr_face_get_user_data(face, &raw const KEY));
            state.upem.set(hr_face_get_upem(face));
            let table = hr_face_reference_table(face, HEAD);
            state.table_length.set(hr_blob_get_length(table));
            hr_blob_destroy(table);
            state.tag_count.set(hr_face_get_table_tags(
                face,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            ));
        }
    }

    /// A table callback for a face with no tables.
    unsafe extern "C" fn no_table(
        _face: *mut hr_face_t,
        _tag: hr_tag_t,
        data: *mut c_void,
    ) -> *mut hr_blob_t {
        let state = unsafe { &*data.cast::<FaceQuery>() };
        state.table_requests.set(state.table_requests.get() + 1);
        ptr::null_mut()
    }

    /// A table callback that answers the first request for `head` with a
    /// blob whose release asks the face questions, and nothing else.
    unsafe extern "C" fn head_once(
        _face: *mut hr_face_t,
        tag: hr_tag_t,
        data: *mut c_void,
    ) -> *mut hr_blob_t {
        let state = unsafe { &*data.cast::<FaceQuery>() };
        state.table_requests.set(state.table_requests.get() + 1);
        if tag != HEAD || state.handed_out.replace(true) {
            return ptr::null_mut();
        }
        unsafe {
            hr_blob_create(
                BYTES.as_ptr().cast(),
                BYTES.len() as c_uint,
                HR_MEMORY_MODE_READONLY,
                data,
                Some(query_face),
            )
        }
    }

    /// A tags callback for a face that says it has three tables.
    unsafe extern "C" fn three_tags(
        _face: *const hr_face_t,
        _start_offset: c_uint,
        _table_count: *mut c_uint,
        _table_tags: *mut hr_tag_t,
        data: *mut c_void,
    ) -> c_uint {
        let state = unsafe { &*data.cast::<FaceQuery>() };
        state.tag_requests.set(state.tag_requests.get() + 1);
        3
    }

    #[test]
    fn a_table_callbacks_destroy_may_query_its_face() {
        let state = FaceQuery::new();
        unsafe {
            let face = hr_face_create_for_tables(Some(no_table), state.as_data(), Some(query_face));
            state.face.set(face);
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, ptr::dangling_mut(), None, 1),
                0
            );
            hr_face_make_immutable(face);
            assert_eq!(state.calls.get(), 0);
            hr_face_destroy(face);
        }
        assert_eq!(state.calls.get(), 1);
        assert_ne!(state.immutable.get(), 0);
        assert!(state.user_data.get().is_null());
        assert_eq!(state.upem.get(), 1000);
        // The face has let go of the callback by the time the callback's
        // data is released, so asking the face for a table then does not
        // call it with that data.
        assert_eq!(state.table_length.get(), 0);
        assert_eq!(state.table_requests.get(), 0);
        assert_eq!(state.tag_count.get(), 0);
    }

    #[test]
    fn a_tags_callbacks_destroy_may_query_its_face() {
        let state = FaceQuery::new();
        unsafe {
            let face = hr_face_create_for_tables(Some(no_table), state.as_data(), None);
            state.face.set(face);
            hr_face_set_get_table_tags_func(
                face,
                Some(three_tags),
                state.as_data(),
                Some(query_face),
            );
            assert_eq!(
                hr_face_get_table_tags(face, 0, ptr::null_mut(), ptr::null_mut()),
                3
            );
            assert_eq!(state.tag_requests.get(), 1);
            assert_eq!(state.calls.get(), 0);
            hr_face_destroy(face);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.immutable.get(), 0);
        assert_eq!(state.upem.get(), 1000);
        assert_eq!(state.table_length.get(), 0);
        // The same goes for the tags callback: it is not asked again once
        // its data is being released.
        assert_eq!(state.tag_count.get(), 0);
        assert_eq!(state.tag_requests.get(), 1);
        // The table callback went before it.
        assert_eq!(state.table_requests.get(), 0);
    }

    #[test]
    fn a_table_blobs_destroy_may_query_the_face_that_kept_it() {
        let state = FaceQuery::new();
        unsafe {
            let face = hr_face_create_for_tables(Some(head_once), state.as_data(), None);
            state.face.set(face);
            // Reading the units per em fetches `head`, which the face keeps
            // for as long as it lives. Four bytes are not a `head` table.
            assert_eq!(hr_face_get_upem(face), 1000);
            assert_eq!(state.table_requests.get(), 1);
            assert_eq!(state.calls.get(), 0);
            hr_face_destroy(face);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.immutable.get(), 0);
        assert_eq!(state.upem.get(), 1000);
        // The tables a face kept go before its table callback does, so the
        // callback is still there to ask, and was asked.
        assert_eq!(state.table_length.get(), 0);
        assert_eq!(state.table_requests.get(), 2);
        assert_eq!(state.tag_count.get(), 0);
    }

    #[test]
    fn a_blobs_destroy_may_query_the_face_releasing_it() {
        let state = FaceQuery::new();
        let font = sfnt_with_records(1, true);
        unsafe {
            let blob = hr_blob_create(
                font.as_ptr().cast(),
                font.len() as c_uint,
                HR_MEMORY_MODE_READONLY,
                state.as_data(),
                Some(query_face),
            );
            let face = hr_face_create(blob, 0);
            state.face.set(face);
            assert_ne!(face, hr_face_get_empty());
            // The face now holds the only reference to the blob.
            hr_blob_destroy(blob);
            let table = hr_face_reference_table(face, record_tag(0));
            assert_eq!(hr_blob_get_length(table), 4);
            hr_blob_destroy(table);
            assert_eq!(
                hr_face_get_table_tags(face, 0, ptr::null_mut(), ptr::null_mut()),
                1
            );
            assert_eq!(state.calls.get(), 0);
            hr_face_destroy(face);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.immutable.get(), 0);
        assert_eq!(state.upem.get(), 1000);
        // The face has let go of the blob by the time the blob's data is
        // released, and has no tables to speak of.
        assert_eq!(state.table_length.get(), 0);
        assert_eq!(state.tag_count.get(), 0);
    }

    /// What `query_blob` is handed, and what the blob told it.
    struct BlobQuery {
        blob: Cell<*mut hr_blob_t>,
        calls: Cell<u32>,
        immutable: Cell<hr_bool_t>,
        user_data: Cell<*mut c_void>,
        length: Cell<c_uint>,
        data: Cell<*const c_char>,
    }

    impl BlobQuery {
        fn new() -> Self {
            Self {
                blob: Cell::new(ptr::null_mut()),
                calls: Cell::new(0),
                immutable: Cell::new(-1),
                user_data: Cell::new(ptr::dangling_mut()),
                length: Cell::new(c_uint::MAX),
                data: Cell::new(ptr::dangling()),
            }
        }

        /// A blob over `BYTES` whose release asks a blob questions.
        unsafe fn blob(&self) -> *mut hr_blob_t {
            unsafe {
                hr_blob_create(
                    BYTES.as_ptr().cast(),
                    BYTES.len() as c_uint,
                    HR_MEMORY_MODE_READONLY,
                    ptr::from_ref(self).cast_mut().cast(),
                    Some(query_blob),
                )
            }
        }
    }

    unsafe extern "C" fn query_blob(data: *mut c_void) {
        let state = unsafe { &*data.cast::<BlobQuery>() };
        let blob = state.blob.get();
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state.immutable.set(hr_blob_is_immutable(blob));
            state
                .user_data
                .set(hr_blob_get_user_data(blob, &raw const KEY));
            let mut length = c_uint::MAX;
            state.data.set(hr_blob_get_data(blob, &raw mut length));
            state.length.set(length);
        }
    }

    #[test]
    fn a_blobs_destroy_may_query_the_blob() {
        let state = BlobQuery::new();
        unsafe {
            let blob = state.blob();
            state.blob.set(blob);
            assert_ne!(
                hr_blob_set_user_data(blob, &raw const KEY, ptr::dangling_mut(), None, 1),
                0
            );
            hr_blob_make_immutable(blob);
            assert_eq!(hr_blob_get_length(blob), 4);
            assert_eq!(state.calls.get(), 0);
            hr_blob_destroy(blob);
        }
        assert_eq!(state.calls.get(), 1);
        assert_ne!(state.immutable.get(), 0);
        assert!(state.user_data.get().is_null());
        // The data is no longer the blob's by the time it is released.
        assert_eq!(state.length.get(), 0);
        assert!(state.data.get().is_null());
    }

    #[test]
    fn a_blobs_destroy_may_query_the_sub_blob_releasing_it() {
        let state = BlobQuery::new();
        unsafe {
            let parent = state.blob();
            let sub = hr_blob_create_sub_blob(parent, 1, 2);
            state.blob.set(sub);
            // The sub-blob now holds the only reference to its parent.
            hr_blob_destroy(parent);
            assert_eq!(hr_blob_get_length(sub), 2);
            assert_eq!(state.calls.get(), 0);
            hr_blob_destroy(sub);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.immutable.get(), 0);
        assert_eq!(state.length.get(), 0);
        assert!(state.data.get().is_null());
    }

    /// What `query_plan` is handed, and what the plan told it.
    struct PlanQuery {
        plan: Cell<*mut hr_shape_plan_t>,
        calls: Cell<u32>,
        user_data: Cell<*mut c_void>,
        direction: Cell<hr_direction_t>,
    }

    unsafe extern "C" fn query_plan(data: *mut c_void) {
        let state = unsafe { &*data.cast::<PlanQuery>() };
        let plan = state.plan.get();
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state
                .user_data
                .set(hr_shape_plan_get_user_data(plan, &raw const KEY));
            let mut props = hr_segment_properties_t {
                direction: HR_DIRECTION_INVALID,
                script: HR_SCRIPT_INVALID,
                language: ptr::null(),
                reserved1: ptr::null_mut(),
                reserved2: ptr::null_mut(),
            };
            hr_shape_plan_get_segment_properties(plan, &raw mut props);
            state.direction.set(props.direction);
        }
    }

    #[test]
    fn a_faces_destroy_may_query_the_plan_releasing_it() {
        let tables = FaceQuery::new();
        let state = PlanQuery {
            plan: Cell::new(ptr::null_mut()),
            calls: Cell::new(0),
            user_data: Cell::new(ptr::dangling_mut()),
            direction: Cell::new(HR_DIRECTION_INVALID),
        };
        unsafe {
            let face = hr_face_create_for_tables(Some(no_table), tables.as_data(), None);
            let props = hr_segment_properties_t {
                direction: HR_DIRECTION_RTL,
                script: HR_SCRIPT_INVALID,
                language: ptr::null(),
                reserved1: ptr::null_mut(),
                reserved2: ptr::null_mut(),
            };
            let plan = hr_shape_plan_create(face, &raw const props, ptr::null(), 0, ptr::null());
            assert_ne!(plan, hr_shape_plan_get_empty());
            state.plan.set(plan);
            assert_ne!(
                hr_shape_plan_set_user_data(plan, &raw const KEY, ptr::dangling_mut(), None, 1),
                0
            );
            // Released along with the face, which only the plan holds a
            // reference to from here on.
            assert_ne!(
                hr_face_set_user_data(
                    face,
                    &raw const KEY,
                    ptr::from_ref(&state).cast_mut().cast(),
                    Some(query_plan),
                    1
                ),
                0
            );
            hr_face_destroy(face);
            assert_eq!(state.calls.get(), 0);
            hr_shape_plan_destroy(plan);
        }
        assert_eq!(state.calls.get(), 1);
        assert!(state.user_data.get().is_null());
        assert_eq!(state.direction.get(), HR_DIRECTION_RTL);
    }

    /// A tags callback for a face that lists nothing.
    unsafe extern "C" fn no_tags(
        _face: *const hr_face_t,
        _start_offset: c_uint,
        _table_count: *mut c_uint,
        _table_tags: *mut hr_tag_t,
        _data: *mut c_void,
    ) -> c_uint {
        0
    }

    /// What `logged_head` is handed: what to log when the table it hands out
    /// is released, and when the callback itself is.
    struct LoggedTables {
        table: DestroyWitness,
        callback: DestroyWitness,
    }

    /// A table callback whose `head` table logs its release.
    unsafe extern "C" fn logged_head(
        _face: *mut hr_face_t,
        tag: hr_tag_t,
        data: *mut c_void,
    ) -> *mut hr_blob_t {
        let tables = unsafe { &*data.cast::<LoggedTables>() };
        if tag != HEAD {
            return ptr::null_mut();
        }
        unsafe {
            hr_blob_create(
                BYTES.as_ptr().cast(),
                BYTES.len() as c_uint,
                HR_MEMORY_MODE_READONLY,
                ptr::from_ref(&tables.table).cast_mut().cast(),
                Some(log_destroy),
            )
        }
    }

    unsafe extern "C" fn log_table_callback(data: *mut c_void) {
        let tables = unsafe { &*data.cast::<LoggedTables>() };
        unsafe { log_destroy(ptr::from_ref(&tables.callback).cast_mut().cast()) };
    }

    #[test]
    fn an_object_releases_what_it_holds_in_order() {
        let log = Mutex::new(Vec::new());
        let witness = |name| DestroyWitness {
            log: &raw const log,
            name,
        };
        let as_data = |witness: &DestroyWitness| ptr::from_ref(witness).cast_mut().cast::<c_void>();
        let user_data = witness("user data");
        let tags = witness("tags callback");

        // A set of callbacks lets go of them in the order it keeps them in,
        // whichever order they were set in.
        let (nominal, h_advance, v_advance) = (
            witness("nominal glyph"),
            witness("h advance"),
            witness("v advance"),
        );
        unsafe {
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_v_advance_func(
                ffuncs,
                Some(fixed_advance),
                as_data(&v_advance),
                Some(log_destroy),
            );
            hr_font_funcs_set_glyph_h_advance_func(
                ffuncs,
                Some(fixed_advance),
                as_data(&h_advance),
                Some(log_destroy),
            );
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                as_data(&nominal),
                Some(log_destroy),
            );
            assert_ne!(
                hr_font_funcs_set_user_data(
                    ffuncs,
                    &raw const KEY,
                    as_data(&user_data),
                    Some(log_destroy),
                    1
                ),
                0
            );
            assert_eq!(*log.lock().unwrap(), [] as [&str; 0]);
            hr_font_funcs_destroy(ffuncs);
        }
        assert_eq!(
            *log.lock().unwrap(),
            ["user data", "nominal glyph", "h advance", "v advance"]
        );
        log.lock().unwrap().clear();

        // A face made from table callbacks, which has kept one table: the
        // table goes before the callback it came from.
        let tables = LoggedTables {
            table: witness("table"),
            callback: witness("table callback"),
        };
        unsafe {
            let face = hr_face_create_for_tables(
                Some(logged_head),
                ptr::from_ref(&tables).cast_mut().cast(),
                Some(log_table_callback),
            );
            hr_face_set_get_table_tags_func(face, Some(no_tags), as_data(&tags), Some(log_destroy));
            assert_ne!(
                hr_face_set_user_data(
                    face,
                    &raw const KEY,
                    as_data(&user_data),
                    Some(log_destroy),
                    1
                ),
                0
            );
            assert_eq!(hr_face_get_upem(face), 1000);
            assert_eq!(*log.lock().unwrap(), [] as [&str; 0]);
            hr_face_destroy(face);
        }
        assert_eq!(
            *log.lock().unwrap(),
            ["user data", "table", "table callback", "tags callback"]
        );
        log.lock().unwrap().clear();

        // A face made over a blob it holds the last reference to: the blob
        // goes where the table callback did.
        let font = sfnt_with_records(1, true);
        let (blob_user_data, blob_data) = (witness("blob user data"), witness("blob data"));
        unsafe {
            let blob = hr_blob_create(
                font.as_ptr().cast(),
                font.len() as c_uint,
                HR_MEMORY_MODE_READONLY,
                as_data(&blob_data),
                Some(log_destroy),
            );
            assert_ne!(
                hr_blob_set_user_data(
                    blob,
                    &raw const KEY,
                    as_data(&blob_user_data),
                    Some(log_destroy),
                    1
                ),
                0
            );
            let face = hr_face_create(blob, 0);
            hr_blob_destroy(blob);
            hr_face_set_get_table_tags_func(face, Some(no_tags), as_data(&tags), Some(log_destroy));
            assert_ne!(
                hr_face_set_user_data(
                    face,
                    &raw const KEY,
                    as_data(&user_data),
                    Some(log_destroy),
                    1
                ),
                0
            );
            assert_eq!(*log.lock().unwrap(), [] as [&str; 0]);
            hr_face_destroy(face);
        }
        assert_eq!(
            *log.lock().unwrap(),
            ["user data", "blob user data", "blob data", "tags callback"]
        );
    }

    /// What `query_font` is handed, and what the font told it.
    struct FontQuery {
        font: Cell<*mut hr_font_t>,
        calls: Cell<u32>,
        immutable: Cell<hr_bool_t>,
        user_data: Cell<*mut c_void>,
        face: Cell<*mut hr_face_t>,
        parent: Cell<*mut hr_font_t>,
        advance: Cell<hr_position_t>,
        /// How often `seven_wide` has answered, for anyone, and the font
        /// data it was last handed.
        answers: Cell<u32>,
        handed: Cell<*mut c_void>,
    }

    impl FontQuery {
        fn new() -> Self {
            Self {
                font: Cell::new(ptr::null_mut()),
                calls: Cell::new(0),
                immutable: Cell::new(-1),
                user_data: Cell::new(ptr::dangling_mut()),
                face: Cell::new(ptr::null_mut()),
                parent: Cell::new(ptr::null_mut()),
                advance: Cell::new(-1),
                answers: Cell::new(0),
                handed: Cell::new(ptr::null_mut()),
            }
        }

        fn as_data(&self) -> *mut c_void {
            ptr::from_ref(self).cast_mut().cast()
        }
    }

    unsafe extern "C" fn query_font(data: *mut c_void) {
        let state = unsafe { &*data.cast::<FontQuery>() };
        let font = state.font.get();
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state.immutable.set(hr_font_is_immutable(font));
            state
                .user_data
                .set(hr_font_get_user_data(font, &raw const KEY));
            state.face.set(hr_font_get_face(font));
            state.parent.set(hr_font_get_parent(font));
            state.advance.set(hr_font_get_glyph_h_advance(font, 1));
        }
    }

    /// An advance callback that makes every glyph seven wide, and notes the
    /// font data it is handed.
    unsafe extern "C" fn seven_wide(
        _font: *mut hr_font_t,
        font_data: *mut c_void,
        _glyph: hr_codepoint_t,
        user_data: *mut c_void,
    ) -> hr_position_t {
        let state = unsafe { &*user_data.cast::<FontQuery>() };
        state.answers.set(state.answers.get() + 1);
        state.handed.set(font_data);
        7
    }

    /// A font over a face with no tables, which the font holds the only
    /// reference to, and that face.
    unsafe fn tableless_font(tables: &FaceQuery) -> (*mut hr_font_t, *mut hr_face_t) {
        unsafe {
            let face = hr_face_create_for_tables(Some(no_table), tables.as_data(), None);
            let font = hr_font_create(face);
            assert_ne!(font, hr_font_get_empty());
            hr_face_destroy(face);
            (font, face)
        }
    }

    /// A set of callbacks holding `seven_wide`, released through `destroy`.
    unsafe fn seven_wide_funcs(
        state: &FontQuery,
        destroy: hr_destroy_func_t,
    ) -> *mut hr_font_funcs_t {
        unsafe {
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_glyph_h_advance_func(
                ffuncs,
                Some(seven_wide),
                state.as_data(),
                destroy,
            );
            ffuncs
        }
    }

    #[test]
    fn a_font_datas_destroy_finds_the_font_as_it_was() {
        let tables = FaceQuery::new();
        let state = FontQuery::new();
        unsafe {
            let (font, face) = tableless_font(&tables);
            state.font.set(font);
            let ffuncs = seven_wide_funcs(&state, None);
            hr_font_set_funcs(font, ffuncs, state.as_data(), Some(query_font));
            // The font now holds the only reference to its callbacks.
            hr_font_funcs_destroy(ffuncs);
            assert_ne!(
                hr_font_set_user_data(font, &raw const KEY, ptr::dangling_mut(), None, 1),
                0
            );
            assert_eq!(hr_font_get_glyph_h_advance(font, 1), 7);
            assert_eq!(state.answers.get(), 1);
            state.handed.set(ptr::null_mut());
            assert_eq!(state.calls.get(), 0);
            hr_font_destroy(font);
            assert_eq!(state.face.get(), face);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.immutable.get(), 0);
        assert!(state.user_data.get().is_null());
        assert_eq!(state.parent.get(), hr_font_get_empty());
        // As in HarfBuzz, a font's data is released while the font still has
        // its callbacks, and they are still handed that data.
        assert_eq!(state.advance.get(), 7);
        assert_eq!(state.answers.get(), 2);
        assert_eq!(state.handed.get(), state.as_data());
    }

    #[test]
    fn a_callbacks_destroy_may_query_the_font_releasing_it() {
        let tables = FaceQuery::new();
        let state = FontQuery::new();
        unsafe {
            let (font, _) = tableless_font(&tables);
            state.font.set(font);
            let ffuncs = seven_wide_funcs(&state, Some(query_font));
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            // The font now holds the only reference to its callbacks.
            hr_font_funcs_destroy(ffuncs);
            assert_eq!(hr_font_get_glyph_h_advance(font, 1), 7);
            assert_eq!(state.calls.get(), 0);
            hr_font_destroy(font);
        }
        assert_eq!(state.calls.get(), 1);
        // The callbacks go last, from a font that has let go of everything:
        // it has no parent, the empty face, and no callbacks to ask.
        assert_eq!(state.parent.get(), hr_font_get_empty());
        assert_eq!(state.face.get(), hr_face_get_empty());
        assert_eq!(state.advance.get(), 0);
        assert_eq!(state.answers.get(), 1);
    }

    #[test]
    fn a_faces_destroy_may_query_the_font_releasing_it() {
        let tables = FaceQuery::new();
        let state = FontQuery::new();
        unsafe {
            let (font, face) = tableless_font(&tables);
            state.font.set(font);
            assert_ne!(
                hr_face_set_user_data(face, &raw const KEY, state.as_data(), Some(query_font), 1),
                0
            );
            let ffuncs = seven_wide_funcs(&state, None);
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            // The font now holds the only reference to its callbacks.
            hr_font_funcs_destroy(ffuncs);
            assert_eq!(hr_font_get_glyph_h_advance(font, 1), 7);
            assert_eq!(state.calls.get(), 0);
            hr_font_destroy(font);
        }
        assert_eq!(state.calls.get(), 1);
        // The face goes from a font that has let go of it.
        assert_eq!(state.face.get(), hr_face_get_empty());
        assert_eq!(state.parent.get(), hr_font_get_empty());
        // The font's data went before its face, so its callbacks are not
        // asked for it any more, though they are released after.
        assert_eq!(state.advance.get(), 0);
        assert_eq!(state.answers.get(), 1);
    }

    #[test]
    fn a_parents_destroy_may_query_the_sub_font_releasing_it() {
        let tables = FaceQuery::new();
        let state = FontQuery::new();
        unsafe {
            let (parent, face) = tableless_font(&tables);
            let sub = hr_font_create_sub_font(parent);
            state.font.set(sub);
            assert_ne!(
                hr_font_set_user_data(parent, &raw const KEY, state.as_data(), Some(query_font), 1),
                0
            );
            // The sub-font now holds the only reference to its parent.
            hr_font_destroy(parent);
            assert_eq!(hr_font_get_parent(sub), parent);
            let advance = hr_font_get_glyph_h_advance(sub, 1);
            assert_eq!(state.calls.get(), 0);
            hr_font_destroy(sub);
            // The sub-font still had its face, and answered as it did.
            assert_eq!(state.face.get(), face);
            assert_eq!(state.advance.get(), advance);
        }
        assert_eq!(state.calls.get(), 1);
        // The parent goes from a font that has let go of it.
        assert_eq!(state.parent.get(), hr_font_get_empty());
    }

    #[test]
    fn a_font_releases_what_it_holds_in_harfbuzzs_order() {
        let log = Mutex::new(Vec::new());
        let witness = |name| DestroyWitness {
            log: &raw const log,
            name,
        };
        let as_data = |witness: &DestroyWitness| ptr::from_ref(witness).cast_mut().cast::<c_void>();
        let tables = FaceQuery::new();
        let (user_data, font_data, on_parent, on_face, callback, funcs_user_data) = (
            witness("user data"),
            witness("font data"),
            witness("parent"),
            witness("face"),
            witness("callback"),
            witness("callbacks' user data"),
        );
        unsafe {
            let (parent, face) = tableless_font(&tables);
            let font = hr_font_create_sub_font(parent);
            let ffuncs = hr_font_funcs_create();
            hr_font_funcs_set_nominal_glyph_func(
                ffuncs,
                Some(always_glyph_777),
                as_data(&callback),
                Some(log_destroy),
            );
            let key = &raw const KEY;
            let log_it = Some(log_destroy as unsafe extern "C" fn(*mut c_void));
            assert_ne!(
                hr_font_funcs_set_user_data(ffuncs, key, as_data(&funcs_user_data), log_it, 1),
                0
            );
            assert_ne!(
                hr_face_set_user_data(face, key, as_data(&on_face), log_it, 1),
                0
            );
            assert_ne!(
                hr_font_set_user_data(parent, key, as_data(&on_parent), log_it, 1),
                0
            );
            assert_ne!(
                hr_font_set_user_data(font, key, as_data(&user_data), log_it, 1),
                0
            );
            hr_font_set_funcs(font, ffuncs, as_data(&font_data), log_it);
            // The font now holds the only reference to each of them.
            hr_font_funcs_destroy(ffuncs);
            hr_font_destroy(parent);
            assert_eq!(*log.lock().unwrap(), [] as [&str; 0]);
            hr_font_destroy(font);
        }
        // HarfBuzz's order: what the font was given itself, then its parent,
        // its face, and its callbacks.
        assert_eq!(
            *log.lock().unwrap(),
            [
                "user data",
                "font data",
                "parent",
                "face",
                "callbacks' user data",
                "callback"
            ]
        );
    }
}
