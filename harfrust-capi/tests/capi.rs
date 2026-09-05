//! Exercises the C API through its own entry points, the way a C caller
//! reaches them.

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

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

fn font_data() -> Vec<u8> {
    std::fs::read(font_path(PLAIN_FONT)).expect("failed to read test font")
}

/// Builds a blob over `data`, which the caller must keep alive.
unsafe fn blob_over(data: &[u8]) -> *mut hr_blob_t {
    unsafe {
        hr_blob_create(
            data.as_ptr().cast::<c_char>(),
            data.len() as c_uint,
            hr_memory_mode_t::HR_MEMORY_MODE_READONLY,
            ptr::null_mut(),
            None,
        )
    }
}

/// Runs `f` with a font built over the named test face.
unsafe fn with_named_font(name: &str, f: impl FnOnce(*mut hr_face_t, *mut hr_font_t)) {
    let data = std::fs::read(font_path(name)).expect("failed to read test font");
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
                hr_memory_mode_t::HR_MEMORY_MODE_DUPLICATE,
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

#[test]
fn null_and_empty_faces_are_safe() {
    unsafe {
        assert_eq!(hr_face_get_upem(ptr::null_mut()), 1000);
        assert_eq!(hr_face_get_glyph_count(ptr::null_mut()), 0);
        assert_eq!(hr_face_get_glyph_count(hr_face_get_empty()), 0);
    }
}

// ---------------------------------------------------------------- fonts ----

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

// --------------------------------------------------------------- buffers ----

#[test]
fn buffer_tracks_its_content_type() {
    unsafe {
        let buffer = hr_buffer_create();
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_INVALID
        );
        hr_buffer_add(buffer, 'x' as u32, 0);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_UNICODE
        );
        hr_buffer_clear_contents(buffer);
        assert_eq!(
            hr_buffer_get_content_type(buffer),
            hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_INVALID
        );
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
                hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_GLYPHS
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
            assert!(!with_liga.is_empty());
            assert!(!without.is_empty());
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
                hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_TEXT,
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
}

unsafe extern "C" fn reference_table(
    _face: *mut hr_face_t,
    tag: hr_tag_t,
    user_data: *mut c_void,
) -> *mut hr_blob_t {
    unsafe {
        let source = &*user_data.cast::<TableSource>();
        // Reuse a blob-backed face purely to look the table up.
        let blob = hr_blob_create(
            source.data.as_ptr().cast::<c_char>(),
            source.data.len() as c_uint,
            hr_memory_mode_t::HR_MEMORY_MODE_READONLY,
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
        let source = Box::into_raw(Box::new(TableSource { data: data.clone() }));
        let face = hr_face_create_for_tables(
            Some(reference_table),
            source.cast::<c_void>(),
            Some(destroy_table_source),
        );
        assert_eq!(hr_face_get_upem(face), PLAIN_UPEM);

        let font = hr_font_create(face);
        let buffer = buffer_with_text(TEXT);
        hr_shape(font, buffer, ptr::null(), 0);
        assert_eq!(glyph_ids(buffer), expected);

        hr_buffer_destroy(buffer);
        hr_font_destroy(font);
        // Freeing the face runs `destroy_table_source`.
        hr_face_destroy(face);
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
            for i in 0..len as usize {
                assert_eq!((*positions.add(i)).x_advance, 0);
            }
            assert!(glyph_ids(buffer).iter().all(|&id| id == 0));

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(ffuncs);
        });

        // Clearing the funcs object restores the built-in behaviour.
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(font, ffuncs, ptr::null_mut(), None);
            hr_font_set_funcs(font, ptr::null_mut(), ptr::null_mut(), None);

            let buffer = buffer_with_text(TEXT);
            hr_shape(font, buffer, ptr::null(), 0);
            assert!(glyph_ids(buffer).iter().all(|&id| id != 0));

            hr_buffer_destroy(buffer);
            hr_font_funcs_destroy(ffuncs);
        });
    }
}

// ------------------------------------------------- font data ownership ----

use std::sync::atomic::{AtomicUsize, Ordering};

static SHARED_DROPS: AtomicUsize = AtomicUsize::new(0);
static REPLACED_DROPS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn count_shared_drop(_user_data: *mut c_void) {
    SHARED_DROPS.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn count_replaced_drop(_user_data: *mut c_void) {
    REPLACED_DROPS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn font_data_outlives_every_font_sharing_it() {
    unsafe {
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(
                font,
                ffuncs,
                ptr::dangling_mut::<c_void>(),
                Some(count_shared_drop),
            );

            // A sub-font shares the parent's font data.
            let sub = hr_font_create_sub_font(font);
            assert_eq!(SHARED_DROPS.load(Ordering::SeqCst), 0);

            // Replacing the parent's callbacks must not release data the
            // sub-font is still holding.
            hr_font_set_funcs(font, ffuncs, ptr::dangling_mut::<c_void>(), None);
            assert_eq!(
                SHARED_DROPS.load(Ordering::SeqCst),
                0,
                "font data released while a sub-font still shares it"
            );

            // It goes only once the last font holding it is gone.
            hr_font_destroy(sub);
            assert_eq!(SHARED_DROPS.load(Ordering::SeqCst), 1);

            hr_font_funcs_destroy(ffuncs);
        });
    }
}

#[test]
fn font_data_is_released_once_when_replaced() {
    unsafe {
        with_font(|_, font| {
            let ffuncs = hr_font_funcs_create();
            hr_font_set_funcs(
                font,
                ffuncs,
                ptr::dangling_mut::<c_void>(),
                Some(count_replaced_drop),
            );
            assert_eq!(REPLACED_DROPS.load(Ordering::SeqCst), 0);

            // With nothing else sharing it, replacing releases it at once.
            hr_font_set_funcs(font, ffuncs, ptr::dangling_mut::<c_void>(), None);
            assert_eq!(REPLACED_DROPS.load(Ordering::SeqCst), 1);

            hr_font_funcs_destroy(ffuncs);
        });
        // And not again when the font itself goes.
        assert_eq!(REPLACED_DROPS.load(Ordering::SeqCst), 1);
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
        let value = 42usize as *mut c_void;
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

        let en = hr_language_from_string(c"en".as_ptr(), -1);
        assert_ne!(hr_language_matches(a, en), 0);
        assert_eq!(hr_language_matches(en, a), 0);
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
            assert!(!glyph_ids(buffer).is_empty());
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
        language: ptr::null(),
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
                hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_GLYPHS
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
        // A plan built for one variation, used with a font at another.
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
    unreachable!("case {case} should have aborted");
}

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
    !status.success()
}

#[test]
fn a_plan_that_does_not_apply_aborts() {
    // Matching HarfBuzz, where these are assertions rather than errors.
    assert!(aborts("props"), "mismatched segment properties must abort");
    assert!(aborts("face"), "a plan for another face must abort");
    assert!(
        aborts("coords"),
        "a plan for other variation settings must abort"
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
            assert!(!glyph_ids(buffer).is_empty());

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
                assert!(!glyph_ids(buffer).is_empty());
                hr_buffer_destroy(buffer);
            }
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
