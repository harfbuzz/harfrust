//! The shared object's ABI boundary to HarfRust. Do not embed a second copy
//! of harfrust_capi: handles and immortal objects belong to that library.

use core::ffi::{c_char, c_int, c_uint, c_void};

#[repr(C)]
pub struct hr_face_t {
    _private: [u8; 0],
}
#[repr(C)]
pub struct hr_blob_t {
    _private: [u8; 0],
}
#[repr(C)]
pub struct hr_set_t {
    _private: [u8; 0],
}

pub type hr_subset_flags_t = c_uint;

pub type Destroy = Option<unsafe extern "C" fn(*mut c_void)>;

extern "C" {
    pub fn hr_set_create() -> *mut hr_set_t;
    pub fn hr_set_destroy(set: *mut hr_set_t);
    pub fn hr_set_next_range(set: *const hr_set_t, first: *mut c_uint, last: *mut c_uint) -> c_int;
    pub fn hr_face_get_table_tags(
        face: *const hr_face_t,
        start: c_uint,
        count: *mut c_uint,
        tags: *mut c_uint,
    ) -> c_uint;
    pub fn hr_face_reference_table(face: *mut hr_face_t, tag: c_uint) -> *mut hr_blob_t;
    pub fn hr_face_get_glyph_count(face: *mut hr_face_t) -> c_uint;
    pub fn hr_face_create_or_fail(blob: *mut hr_blob_t, index: c_uint) -> *mut hr_face_t;
    pub fn hr_blob_get_data(blob: *mut hr_blob_t, length: *mut c_uint) -> *const c_char;
    pub fn hr_blob_create_or_fail(
        data: *const c_char,
        length: c_uint,
        mode: c_int,
        user_data: *mut c_void,
        destroy: Destroy,
    ) -> *mut hr_blob_t;
    pub fn hr_blob_destroy(blob: *mut hr_blob_t);
}
