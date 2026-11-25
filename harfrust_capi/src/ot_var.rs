//! OpenType variation-axis metadata.
use crate::common::{hr_tag_t, tag_from_rust, Page};
use crate::face::hr_face_t;
use crate::object;
use crate::ot_name::hr_ot_name_id_t;
use core::ffi::c_uint;
use read_fonts::{tables::fvar::AxisInstanceArrays, TableProvider};

/// Flags on an OpenType variation axis.
pub type hr_ot_var_axis_flags_t = c_uint;
/// The axis should not be exposed directly in user interfaces.
pub const HR_OT_VAR_AXIS_FLAG_HIDDEN: hr_ot_var_axis_flags_t = 1;

/// Metadata for an axis in the font's `fvar` table.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct hr_ot_var_axis_info_t {
    pub axis_index: c_uint,
    pub tag: hr_tag_t,
    pub name_id: hr_ot_name_id_t,
    pub flags: hr_ot_var_axis_flags_t,
    pub min_value: f32,
    pub default_value: f32,
    pub max_value: f32,
    /// Reserved; always zero.
    pub reserved: c_uint,
}

fn axes(face: &hr_face_t) -> Option<AxisInstanceArrays<'_>> {
    face.font()?
        .tables()
        .fvar()
        .ok()?
        .axis_instance_arrays()
        .ok()
}

/// Returns the number of variation axes, or zero without readable `fvar` data.
/// # Safety
/// `face` must be null or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_var_get_axis_count(face: *mut hr_face_t) -> c_uint {
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    axes(face).map_or(0, |arrays| arrays.axes().len() as c_uint)
}

/// Returns the total axis count and copies axis metadata from `start_offset`.
///
/// When both output pointers are non-null, `axes_count` gives the capacity on
/// entry and the number written on return. A null array sets the count to
/// zero without reading it; the README has the rule for a count and its
/// array. Axis indices remain relative to the full `fvar` axis array.
/// # Safety
/// `face` must be null or live; `axes_count` must be null or writable;
/// `axes_array` must hold the input capacity when non-null.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_var_get_axis_infos(
    face: *mut hr_face_t,
    start_offset: c_uint,
    axes_count: *mut c_uint,
    axes_array: *mut hr_ot_var_axis_info_t,
) -> c_uint {
    // SAFETY: `Page::new` needs the count null or writable, and, with
    // neither pointer null, set to a number of axes the array is writable
    // for, until the page is dropped, which is before this returns. This
    // function's contract has `axes_count` null or writable and a non-null
    // `axes_array` hold the capacity it gives on entry, which the README
    // has the caller set before the call.
    let page = unsafe { Page::new(axes_count, axes_array) };
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    let arrays = axes(face);
    let records = arrays.as_ref().map_or(&[][..], |arrays| arrays.axes());
    page.fill(
        records
            .iter()
            .enumerate()
            .skip(start_offset as usize)
            .map(|(index, axis)| {
                let default_value = axis.default_value().to_f32();
                hr_ot_var_axis_info_t {
                    axis_index: index as c_uint,
                    tag: tag_from_rust(axis.axis_tag()),
                    name_id: axis.axis_name_id().to_u16() as c_uint,
                    flags: axis.flags() as c_uint,
                    min_value: axis.min_value().to_f32().min(default_value),
                    default_value,
                    max_value: axis.max_value().to_f32().max(default_value),
                    reserved: 0,
                }
            }),
    );
    records.len() as c_uint
}
