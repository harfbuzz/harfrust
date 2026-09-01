//! Shaping, mirroring HarfBuzz's `hb-shape.h`.

use core::ffi::{c_char, c_uint};
use core::ptr;
use std::panic::{catch_unwind, AssertUnwindSafe};

use harfrust::{Direction, Feature, ShapeError, ShapeOptions};

use crate::buffer::{hr_buffer_t, CStrArray};
use crate::common::{hr_bool_t, hr_feature_t};
use crate::font::hr_font_t;
use crate::font_funcs::FontFuncsAdapter;

/// Runs `f`, catching any panic rather than letting it unwind into C, which
/// would be undefined behaviour. Returns `None` if it panicked.
fn guard<T>(f: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok()
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
        buffer_ref.buffer.shape(instance, options)
    })
    .is_some_and(|result| result.is_ok())
    .into()
}

/// Shapes a buffer with a font, applying the given features.
///
/// The buffer's direction, script and language must be set beforehand; call
/// `hr_buffer_guess_segment_properties` to fill in whatever is missing, which
/// this does for the direction on your behalf. On return the buffer holds
/// glyphs.
///
/// Use `hr_shape_full` if you want to know whether the shaper ran out of room.
///
/// # Aborts
///
/// See [`hr_shape_full`].
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
/// extent of failing when it names shapers that are all unavailable.
///
/// Returns false when the shaper ran past its length, operation or nesting
/// limits, which pathological input can provoke and which a caller can
/// reasonably recover from.
///
/// # Aborts
///
/// Misusing the API aborts the process, as HarfBuzz's assertions do: passing a
/// buffer that already holds glyphs, or a font with nothing to shape with.
/// `hr_shape` returns nothing and so could not otherwise report these at
/// all.
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

    let outcome = guard(|| {
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
        buffer_ref.buffer.shape(instance, options)
    });

    match outcome {
        // A panic that unwound out of the shaper. Nothing useful to say.
        None => false.into(),
        Some(Ok(())) => true.into(),
        // Pathological input rather than a mistake by the caller, and
        // attacker-controllable, so it is reported rather than fatal. This is
        // the one failure HarfBuzz also lets `hb_shape_full` return.
        Some(Err(ShapeError::LimitsExceeded)) => false.into(),
        // Everything else is a misuse of the API, which HarfBuzz asserts on.
        // Aborting here rather than returning false means `hr_shape`, which
        // cannot report anything, does not swallow it.
        Some(Err(err)) => panic!("hr_shape: {err}"),
    }
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

/// The major version of this library.
///
/// These are literals so that they reach the generated header, and a test
/// keeps them in step with the crate version.
pub const HR_VERSION_MAJOR: c_uint = 0;
/// The minor version of this library.
pub const HR_VERSION_MINOR: c_uint = 13;
/// The micro version of this library.
pub const HR_VERSION_MICRO: c_uint = 3;
/// The version of this library, as a string.
pub const HR_VERSION_STRING: &str = "0.13.3";

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
    // Kept in step with the crate version, NUL-terminated for C.
    concat!(env!("CARGO_PKG_VERSION"), "\0")
        .as_ptr()
        .cast::<c_char>()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constants_match_the_crate() {
        assert_eq!(
            format!("{HR_VERSION_MAJOR}.{HR_VERSION_MINOR}.{HR_VERSION_MICRO}"),
            env!("CARGO_PKG_VERSION"),
        );
        assert_eq!(HR_VERSION_STRING, env!("CARGO_PKG_VERSION"));
    }

    /// The header is generated and committed, so it can fall behind the crate.
    #[test]
    fn generated_header_matches_the_crate_version() {
        let header = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/include/hr.h"))
            .expect("include/hr.h should be committed alongside the crate");

        for (name, value) in [
            ("HR_VERSION_MAJOR", HR_VERSION_MAJOR.to_string()),
            ("HR_VERSION_MINOR", HR_VERSION_MINOR.to_string()),
            ("HR_VERSION_MICRO", HR_VERSION_MICRO.to_string()),
            ("HR_VERSION_STRING", format!("{HR_VERSION_STRING:?}")),
        ] {
            let expected = format!("#define {name} {value}");
            assert!(
                header.contains(&expected),
                "include/hr.h is out of date: expected `{expected}`.                  Regenerate it with cbindgen, then run                  scripts/gen-hb-compat-header.py."
            );
        }
    }
}
