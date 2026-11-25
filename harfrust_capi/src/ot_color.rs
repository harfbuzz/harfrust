//! OpenType CPAL palette queries.

use crate::color::hr_color_t;
use crate::common::{hr_bool_t, Page};
use crate::face::hr_face_t;
use crate::object;
use core::ffi::c_uint;
use read_fonts::{tables::cpal::Cpal, TableProvider};

/// Flags describing the backgrounds on which a palette is usable.
pub type hr_ot_color_palette_flags_t = c_uint;
/// No background preference.
pub const HR_OT_COLOR_PALETTE_FLAG_DEFAULT: hr_ot_color_palette_flags_t = 0;
/// The palette is usable on a light background.
pub const HR_OT_COLOR_PALETTE_FLAG_USABLE_WITH_LIGHT_BACKGROUND: hr_ot_color_palette_flags_t = 1;
/// The palette is usable on a dark background.
pub const HR_OT_COLOR_PALETTE_FLAG_USABLE_WITH_DARK_BACKGROUND: hr_ot_color_palette_flags_t = 2;

fn cpal(face: &hr_face_t) -> Option<Cpal<'_>> {
    face.font()?.tables().cpal().ok()
}

/// Returns whether the face has a CPAL table with any palettes.
/// # Safety
/// `face` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_color_has_palettes(face: *mut hr_face_t) -> hr_bool_t {
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    cpal(face)
        .is_some_and(|table| table.num_palettes() != 0)
        .into()
}

/// Returns the number of palettes in CPAL, or zero if absent.
/// # Safety
/// `face` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_color_palette_get_count(face: *mut hr_face_t) -> c_uint {
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    cpal(face).map_or(0, |table| table.num_palettes() as c_uint)
}

/// Returns a palette's background flags, or the default flags if unavailable.
/// # Safety
/// `face` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_color_palette_get_flags(
    face: *mut hr_face_t,
    palette_index: c_uint,
) -> hr_ot_color_palette_flags_t {
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    cpal(face)
        .and_then(|table| table.palette_types_array()?.ok())
        .and_then(|flags| flags.get(palette_index as usize))
        .map_or(HR_OT_COLOR_PALETTE_FLAG_DEFAULT, |flags| flags.get().bits())
}

/// Returns the total number of colors in a palette and copies a page of them.
///
/// Colors are unpremultiplied sRGB values in `hr_color_t` packing. When both
/// output pointers are non-null, `color_count` gives capacity on entry and
/// the number written on return. A null array sets the count to zero without
/// reading it; the README has the rule for a count and its array. An invalid
/// palette index returns zero and clears the count.
///
/// # Safety
/// `face` must be null or live; `color_count` must be null or writable;
/// `colors` must hold the input capacity when non-null.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_color_palette_get_colors(
    face: *mut hr_face_t,
    palette_index: c_uint,
    start_offset: c_uint,
    color_count: *mut c_uint,
    colors: *mut hr_color_t,
) -> c_uint {
    // SAFETY: `Page::new` needs the count null or writable, and, with
    // neither pointer null, set to a number of colors the array is writable
    // for, until the page is dropped, which is before this returns. This
    // function's contract has `color_count` null or writable and a non-null
    // `colors` hold the capacity it gives on entry, which the README has
    // the caller set before the call.
    let page = unsafe { Page::new(color_count, colors) };
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    let palette = cpal(face).and_then(|table| {
        let start = table
            .color_record_indices()
            .get(palette_index as usize)?
            .get() as usize;
        let records = table.color_records_array()?.ok()?;
        let entries = table.num_palette_entries() as usize;
        let start = start.min(records.len());
        let end = start.saturating_add(entries).min(records.len());
        Some((entries as c_uint, &records[start..end]))
    });
    let Some((total, records)) = palette else {
        // There is no such palette. The page is dropped with nothing
        // written, which is what the count then says.
        return 0;
    };
    page.fill(records.iter().skip(start_offset as usize).map(|record| {
        u32::from_be_bytes([record.blue(), record.green(), record.red(), record.alpha()])
    }));
    total
}
