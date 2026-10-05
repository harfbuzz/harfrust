//! Sets of codepoints and glyph IDs used by layout queries.

use core::ffi::c_void;
use read_fonts::collections::U32Set;
use std::sync::OnceLock;

use crate::common::{hr_bool_t, hr_codepoint_t, HR_CODEPOINT_INVALID};
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

pub struct hr_set_t {
    header: ObjectHeader,
    pub(crate) values: U32Set,
}

static EMPTY_SET: OnceLock<Empty<hr_set_t>> = OnceLock::new();

impl Object for hr_set_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_SET
            .get_or_init(|| {
                Empty::new(hr_set_t {
                    header: ObjectHeader::immortal(),
                    values: U32Set::default(),
                })
            })
            .get()
    }
}

impl hr_set_t {
    pub(crate) fn add(&mut self, value: u32) {
        if value != HR_CODEPOINT_INVALID {
            self.values.insert(value);
        }
    }

    pub(crate) fn include_all_glyphs(&mut self, count: u32) {
        if count > 0 {
            self.values.insert_range(0..=count - 1);
        }
    }
}

#[no_mangle]
pub extern "C" fn hr_set_create() -> *mut hr_set_t {
    object::create(hr_set_t {
        header: ObjectHeader::new(),
        values: U32Set::default(),
    })
}

#[no_mangle]
pub extern "C" fn hr_set_get_empty() -> *mut hr_set_t {
    hr_set_t::empty()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_reference(set: *mut hr_set_t) -> *mut hr_set_t {
    unsafe { object::reference(set) }
}

/// # Safety
/// `set` must be `NULL` or live, and the caller must own its reference.
#[no_mangle]
pub unsafe extern "C" fn hr_set_destroy(set: *mut hr_set_t) {
    unsafe { object::destroy(set) };
}

/// # Safety
/// `set` must be `NULL` or live, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_set_set_user_data(
    set: *mut hr_set_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(set, key, data, destroy, replace != 0) }.into()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_get_user_data(
    set: *mut hr_set_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(set, key) }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_clear(set: *mut hr_set_t) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        set.values.clear();
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_add(set: *mut hr_set_t, value: hr_codepoint_t) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        set.add(value);
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_has(set: *const hr_set_t, value: hr_codepoint_t) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    (value != HR_CODEPOINT_INVALID && set.values.contains(value)).into()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_is_empty(set: *const hr_set_t) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    set.values.is_empty().into()
}
