//! Binary data with a lifetime, mirroring HarfBuzz's `hb-blob.h`.

use core::ffi::{c_char, c_uint, c_void};
use std::sync::{Arc, OnceLock};

use harfrust::font::FontBlob;

use crate::common::hr_bool_t;
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

/// How a blob relates to the memory it was created over.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum hr_memory_mode_t {
    /// Copy the data. The caller keeps ownership of the original buffer.
    HR_MEMORY_MODE_DUPLICATE = 0,
    /// Use the data in place. It must outlive the blob and never change.
    HR_MEMORY_MODE_READONLY = 1,
    /// Use the data in place. Treated as read-only by this library.
    HR_MEMORY_MODE_WRITABLE = 2,
    /// Use the data in place. Treated as read-only by this library.
    HR_MEMORY_MODE_READONLY_MAY_MAKE_WRITABLE = 3,
}

/// Bytes owned by the caller, released through a destroy callback.
struct ForeignBytes {
    data: *const u8,
    len: usize,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
}

// SAFETY: the caller promises, as documented on `hr_blob_create`, that the
// data stays valid and unchanging for the blob's lifetime, and that the
// destroy callback may run on any thread.
unsafe impl Send for ForeignBytes {}
unsafe impl Sync for ForeignBytes {}

impl AsRef<[u8]> for ForeignBytes {
    fn as_ref(&self) -> &[u8] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        // SAFETY: upheld by the caller of `hr_blob_create`.
        unsafe { core::slice::from_raw_parts(self.data, self.len) }
    }
}

impl Drop for ForeignBytes {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: `destroy` was supplied alongside `user_data`.
            unsafe { destroy(self.user_data) };
        }
    }
}

/// A slice of another blob, keeping the parent's data alive.
struct SubBytes {
    parent: FontBlob,
    start: usize,
    end: usize,
}

impl AsRef<[u8]> for SubBytes {
    fn as_ref(&self) -> &[u8] {
        self.parent
            .as_ref()
            .get(self.start..self.end)
            .unwrap_or(&[])
    }
}

/// Binary data with a lifetime.
pub struct hr_blob_t {
    header: ObjectHeader,
    pub(crate) blob: FontBlob,
}

impl hr_blob_t {
    pub(crate) fn new(blob: FontBlob) -> *mut Self {
        object::create(hr_blob_t {
            header: ObjectHeader::new(),
            blob,
        })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        self.blob.as_ref()
    }

    /// Creates a blob over a range of `parent`, sharing its storage.
    pub(crate) fn sub(parent: &FontBlob, start: usize, end: usize) -> *mut Self {
        if start >= end || end > parent.as_ref().len() {
            return Self::empty();
        }
        let sub = SubBytes {
            parent: parent.clone(),
            start,
            end,
        };
        Self::new(FontBlob::Shared(
            Arc::new(sub) as Arc<dyn AsRef<[u8]> + Send + Sync>
        ))
    }
}

static EMPTY_BLOB: OnceLock<Empty<hr_blob_t>> = OnceLock::new();

impl Object for hr_blob_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_BLOB
            .get_or_init(|| {
                Empty::new(hr_blob_t {
                    header: ObjectHeader::immortal(),
                    blob: FontBlob::Static(&[]),
                })
            })
            .get()
    }
}

/// Creates a blob over `length` bytes at `data`.
///
/// With [`hr_memory_mode_t::HR_MEMORY_MODE_DUPLICATE`] the data is copied and
/// `destroy` is called before returning. Otherwise the blob borrows the data,
/// which must stay valid and unchanged until `destroy` is invoked.
///
/// Never returns `NULL`; on failure it returns the empty blob.
///
/// # Safety
///
/// `data` must point to `length` readable bytes. `destroy`, if given, must be
/// safe to call from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_create(
    data: *const c_char,
    length: c_uint,
    mode: hr_memory_mode_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) -> *mut hr_blob_t {
    let blob = unsafe { blob_from_raw(data, length, mode, user_data, destroy) };
    match blob {
        Some(blob) => hr_blob_t::new(blob),
        None => hr_blob_t::empty(),
    }
}

/// Creates a blob, returning `NULL` rather than the empty blob on failure.
///
/// # Safety
///
/// See [`hr_blob_create`].
#[no_mangle]
pub unsafe extern "C" fn hr_blob_create_or_fail(
    data: *const c_char,
    length: c_uint,
    mode: hr_memory_mode_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) -> *mut hr_blob_t {
    let blob = unsafe { blob_from_raw(data, length, mode, user_data, destroy) };
    match blob {
        Some(blob) => hr_blob_t::new(blob),
        None => core::ptr::null_mut(),
    }
}

/// # Safety
///
/// See [`hr_blob_create`].
unsafe fn blob_from_raw(
    data: *const c_char,
    length: c_uint,
    mode: hr_memory_mode_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) -> Option<FontBlob> {
    if data.is_null() || length == 0 {
        // Release the caller's data; an empty blob owns nothing.
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return None;
    }
    if mode == hr_memory_mode_t::HR_MEMORY_MODE_DUPLICATE {
        // SAFETY: the caller guarantees `length` readable bytes at `data`.
        let copy =
            unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length as usize) }.to_vec();
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return Some(FontBlob::from(copy));
    }
    let bytes = ForeignBytes {
        data: data.cast::<u8>(),
        len: length as usize,
        user_data,
        destroy,
    };
    Some(FontBlob::Shared(
        Arc::new(bytes) as Arc<dyn AsRef<[u8]> + Send + Sync>
    ))
}

/// Creates a blob over the contents of a file.
///
/// Never returns `NULL`; on failure it returns the empty blob.
///
/// # Safety
///
/// `file_name` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_create_from_file(file_name: *const c_char) -> *mut hr_blob_t {
    let blob = unsafe { hr_blob_create_from_file_or_fail(file_name) };
    if blob.is_null() {
        return hr_blob_t::empty();
    }
    blob
}

/// Creates a blob over the contents of a file, returning `NULL` on failure.
///
/// # Safety
///
/// `file_name` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_create_from_file_or_fail(
    file_name: *const c_char,
) -> *mut hr_blob_t {
    if file_name.is_null() {
        return core::ptr::null_mut();
    }
    let path = unsafe { core::ffi::CStr::from_ptr(file_name) };
    let Ok(path) = path.to_str() else {
        return core::ptr::null_mut();
    };
    match std::fs::read(path) {
        Ok(data) => hr_blob_t::new(FontBlob::from(data)),
        Err(_) => core::ptr::null_mut(),
    }
}

/// Creates a blob over a range of another blob, sharing its storage.
///
/// The range is clamped to the parent's bounds. Never returns `NULL`.
///
/// # Safety
///
/// `parent` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_create_sub_blob(
    parent: *mut hr_blob_t,
    offset: c_uint,
    length: c_uint,
) -> *mut hr_blob_t {
    let Some(parent) = (unsafe { parent.as_ref() }) else {
        return hr_blob_t::empty();
    };
    let total = parent.bytes().len();
    let start = (offset as usize).min(total);
    let end = start.saturating_add(length as usize).min(total);
    if start >= end {
        return hr_blob_t::empty();
    }
    let sub = SubBytes {
        parent: parent.blob.clone(),
        start,
        end,
    };
    hr_blob_t::new(FontBlob::Shared(
        Arc::new(sub) as Arc<dyn AsRef<[u8]> + Send + Sync>
    ))
}

/// Returns a new blob holding a private copy of `blob`'s data.
///
/// Returns `NULL` if the blob is empty or the copy could not be made.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_copy_writable_or_fail(blob: *mut hr_blob_t) -> *mut hr_blob_t {
    let Some(blob) = (unsafe { blob.as_ref() }) else {
        return core::ptr::null_mut();
    };
    let bytes = blob.bytes();
    if bytes.is_empty() {
        return core::ptr::null_mut();
    }
    hr_blob_t::new(FontBlob::from(bytes.to_vec()))
}

/// Returns the immortal empty blob.
#[no_mangle]
pub extern "C" fn hr_blob_get_empty() -> *mut hr_blob_t {
    hr_blob_t::empty()
}

/// Increments a blob's reference count.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_reference(blob: *mut hr_blob_t) -> *mut hr_blob_t {
    unsafe { object::reference(blob) }
}

/// Decrements a blob's reference count, freeing it at zero.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob, and the caller must own the
/// reference being released.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_destroy(blob: *mut hr_blob_t) {
    unsafe { object::destroy(blob) };
}

/// Attaches user data to a blob.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_set_user_data(
    blob: *mut hr_blob_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(blob, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data previously attached to a blob.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_get_user_data(
    blob: *mut hr_blob_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(blob, key) }
}

/// Marks a blob immutable.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_make_immutable(blob: *mut hr_blob_t) {
    unsafe { object::make_immutable(blob) };
}

/// Returns whether a blob has been marked immutable.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_is_immutable(blob: *mut hr_blob_t) -> hr_bool_t {
    unsafe { object::is_immutable(blob.cast_const()) }.into()
}

/// Returns the length of a blob's data, in bytes.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_get_length(blob: *mut hr_blob_t) -> c_uint {
    unsafe { object::or_empty(blob.cast_const()) }.bytes().len() as c_uint
}

/// Returns a blob's data, writing its length to `length` when non-`NULL`.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob, and `length` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_get_data(
    blob: *mut hr_blob_t,
    length: *mut c_uint,
) -> *const c_char {
    let bytes = unsafe { object::or_empty(blob.cast_const()) }.bytes();
    if let Some(length) = unsafe { length.as_mut() } {
        *length = bytes.len() as c_uint;
    }
    bytes.as_ptr().cast::<c_char>()
}

/// Always returns `NULL`, writing zero to `length`.
///
/// Blobs in this library are read-only, since the shaping API never needs to
/// modify font data in place. Use [`hr_blob_copy_writable_or_fail`] to obtain
/// a private, mutable copy instead.
///
/// # Safety
///
/// `length` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_blob_get_data_writable(
    _blob: *mut hr_blob_t,
    length: *mut c_uint,
) -> *mut c_char {
    if let Some(length) = unsafe { length.as_mut() } {
        *length = 0;
    }
    core::ptr::null_mut()
}
