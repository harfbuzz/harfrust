//! Shaping, mirroring HarfBuzz's `hb-shape.h`.

use core::ffi::{c_char, c_uint};
use core::ptr;
use std::panic::{catch_unwind, AssertUnwindSafe};

use harfrust::{Direction, Feature, ShapeOptions};

use crate::buffer::{hr_buffer_t, CStrArray};
use crate::common::{hr_bool_t, hr_feature_t};
use crate::font::hr_font_t;
use crate::font_funcs::FontFuncsAdapter;

/// Runs `f`, turning any panic into `false` rather than letting it unwind into
/// C, which would be undefined behaviour.
fn guard(f: impl FnOnce()) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_ok()
}

/// Reads `num_features` features, or none if `features` is `NULL`.
///
/// # Safety
///
/// `features` must be `NULL` or point to `num_features` readable entries.
pub(crate) unsafe fn collect_features(
    features: *const hr_feature_t,
    num_features: c_uint,
) -> Vec<Feature> {
    if features.is_null() || num_features == 0 {
        return Vec::new();
    }
    // SAFETY: the caller guarantees the array is readable.
    unsafe { core::slice::from_raw_parts(features, num_features as usize) }
        .iter()
        .map(|feature| Feature::from(*feature))
        .collect()
}

/// Shapes a buffer with a plan the caller has already chosen.
///
/// `font` is the pointer the font callbacks are handed; `font_ref` is that same
/// font, borrowed.
pub(crate) fn shape_with_plan(
    font: *mut hr_font_t,
    font_ref: &hr_font_t,
    buffer_ref: &mut hr_buffer_t,
    features: &[Feature],
    plan: &harfrust::ShapePlan,
) -> hr_bool_t {
    let Some(instance) = font_ref.instance.as_ref() else {
        return false.into();
    };
    let mut adapter = FontFuncsAdapter {
        funcs: font_ref.funcs,
        font,
        font_data: font_ref.font_data,
    };
    guard(|| {
        let mut options = ShapeOptions::new()
            .scale_separate(Some((font_ref.x_scale, font_ref.y_scale)))
            .features(features)
            .plan(Some(plan));
        if font_ref.ptem > 0.0 {
            options = options.point_size(Some(font_ref.ptem));
        }
        if !font_ref.funcs.is_null() {
            options = options.font_funcs(Some(&mut adapter));
        }
        buffer_ref.buffer.shape(instance, options);
    })
    .into()
}

/// Shapes a buffer with a font, applying the given features.
///
/// The buffer's direction, script and language must be set beforehand; call
/// `hr_buffer_guess_segment_properties` to fill in whatever is missing. On
/// return the buffer holds glyphs, and shaping a buffer that already holds
/// glyphs does nothing.
///
/// # Safety
///
/// `font` and `buffer` must be `NULL` or live, and `features` must point to
/// `num_features` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_shape(
    font: *mut hr_font_t,
    buffer: *mut hr_buffer_t,
    features: *const hr_feature_t,
    num_features: c_uint,
) {
    unsafe { hr_shape_full(font, buffer, features, num_features, ptr::null()) };
}

/// Shapes a buffer, selecting from a list of shaper names.
///
/// This library has a single shaper, so `shaper_list` is honoured only to the
/// extent of failing when it names shapers that are all unavailable. Returns
/// false if shaping could not be carried out.
///
/// # Safety
///
/// `font` and `buffer` must be `NULL` or live, `features` must point to
/// `num_features` readable entries, and `shaper_list` must be `NULL` or a
/// `NULL`-terminated array of NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_full(
    font: *mut hr_font_t,
    buffer: *mut hr_buffer_t,
    features: *const hr_feature_t,
    num_features: c_uint,
    shaper_list: *const *const c_char,
) -> hr_bool_t {
    if !unsafe { shaper_list_allows_ot(shaper_list) } {
        return false.into();
    }
    let (Some(font_ref), Some(buffer_ref)) = (unsafe { font.as_ref() }, unsafe { buffer.as_mut() })
    else {
        return false.into();
    };
    let Some(instance) = font_ref.instance.as_ref() else {
        return false.into();
    };

    let features = unsafe { collect_features(features, num_features) };

    let mut adapter = FontFuncsAdapter {
        funcs: font_ref.funcs,
        font,
        font_data: font_ref.font_data,
    };

    let ok = guard(|| {
        // Building a plan requires a direction; HarfBuzz tolerates an unset one,
        // so fill in whatever the caller left out rather than failing.
        if buffer_ref.buffer.direction() == Direction::Invalid {
            buffer_ref.buffer.guess_segment_properties();
        }

        // Reuse a plan from the face's cache, as HarfBuzz's `hb_shape` does.
        let plan = unsafe { font_ref.face().as_ref() }.map(|face| {
            face.plans.get(
                instance,
                buffer_ref.buffer.direction(),
                Some(buffer_ref.buffer.script()),
                buffer_ref.buffer.language(),
                &features,
            )
        });

        let mut options = ShapeOptions::new()
            .scale_separate(Some((font_ref.x_scale, font_ref.y_scale)))
            .features(&features)
            .plan(plan.as_deref());
        if font_ref.ptem > 0.0 {
            options = options.point_size(Some(font_ref.ptem));
        }
        if !font_ref.funcs.is_null() {
            options = options.font_funcs(Some(&mut adapter));
        }
        buffer_ref.buffer.shape(instance, options);
    });
    ok.into()
}

/// Returns whether `shaper_list` permits this library's shaper.
///
/// # Safety
///
/// `shaper_list` must be `NULL` or a `NULL`-terminated array of
/// NUL-terminated strings.
pub(crate) unsafe fn shaper_list_allows_ot(shaper_list: *const *const c_char) -> bool {
    let Some(list) = (unsafe { shaper_list.as_ref() }) else {
        return true;
    };
    let mut entry: *const *const c_char = list;
    let mut saw_any = false;
    loop {
        let name = unsafe { *entry };
        if name.is_null() {
            break;
        }
        saw_any = true;
        if let Ok(name) = unsafe { core::ffi::CStr::from_ptr(name) }.to_str() {
            if name == "ot" {
                return true;
            }
        }
        entry = unsafe { entry.add(1) };
    }
    // An empty list places no restriction; a list without "ot" rules us out.
    !saw_any
}

/// Returns the shapers this library provides, as a `NULL`-terminated array of
/// names.
#[no_mangle]
pub extern "C" fn hr_shape_list_shapers() -> *const *const c_char {
    static SHAPERS: CStrArray<2> = CStrArray([c"ot".as_ptr(), ptr::null()]);
    SHAPERS.0.as_ptr()
}

/// Returns the version of the underlying HarfRust library.
///
/// # Safety
///
/// Each of `major`, `minor` and `micro` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_version(major: *mut c_uint, minor: *mut c_uint, micro: *mut c_uint) {
    let parse = |s: &str| s.parse::<c_uint>().unwrap_or(0);
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let values = [
        parts.next().map_or(0, parse),
        parts.next().map_or(0, parse),
        parts.next().map_or(0, parse),
    ];
    for (out, value) in [major, minor, micro].into_iter().zip(values) {
        if let Some(out) = unsafe { out.as_mut() } {
            *out = value;
        }
    }
}

/// Returns the version of the underlying HarfRust library as a string.
#[no_mangle]
pub extern "C" fn hr_version_string() -> *const c_char {
    c"0.13.3".as_ptr()
}

/// Returns whether the library is at least the given version.
#[no_mangle]
pub extern "C" fn hr_version_atleast(major: c_uint, minor: c_uint, micro: c_uint) -> hr_bool_t {
    let mut have = [0; 3];
    // SAFETY: all three pointers are valid.
    unsafe {
        hr_version(
            ptr::from_mut(&mut have[0]),
            ptr::from_mut(&mut have[1]),
            ptr::from_mut(&mut have[2]),
        );
    };
    (have >= [major, minor, micro]).into()
}
