//! Fonts as a set of tables, mirroring HarfBuzz's `hb-face.h`.

use core::ffi::{c_uint, c_void};
use core::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, OnceLock};

use harfrust::font::{Font, FontBlob, FontTableFunction};
use harfrust::Tag;
use read_fonts::TableProvider;

use crate::blob::hr_blob_t;
use crate::common::{hr_bool_t, hr_tag_t, tag_from_rust, tag_to_rust};
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};
use crate::plan::PlanCache;

/// Callback returning the data for one table of a face.
///
/// The callback owns the reference it returns; this library takes it over and
/// releases it. Return `NULL` for a table the face does not have.
pub type hr_reference_table_func_t = Option<
    unsafe extern "C" fn(
        face: *mut hr_face_t,
        tag: hr_tag_t,
        user_data: *mut c_void,
    ) -> *mut hr_blob_t,
>;

/// A table callback together with the data it was registered with.
struct TableFunc {
    func: hr_reference_table_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
    /// Filled in once the face exists, so the callback can be handed the face
    /// it is being asked about. Tables are only ever fetched lazily, well
    /// after construction, so this is never read while still null in practice.
    face: AtomicPtr<hr_face_t>,
}

// SAFETY: `hr_face_create_for_tables` documents that the callback and its user
// data must be safe to use from any thread. `read-fonts` requires this because
// it may load a table from whichever thread first touches it.
unsafe impl Send for TableFunc {}
unsafe impl Sync for TableFunc {}

impl TableFunc {
    fn call(&self, tag: Tag) -> Option<FontBlob> {
        let func = self.func?;
        let face = self.face.load(Ordering::Acquire);
        // SAFETY: the caller registered this callback and promised it is safe
        // to invoke with the user data it supplied.
        let blob = unsafe { func(face, tag_from_rust(tag), self.user_data) };
        let owned = unsafe { blob.as_ref() }?;
        // Clone the underlying storage, which is reference counted and outlives
        // the wrapper, then release the reference the callback handed us.
        let data = owned.blob.clone();
        unsafe { object::destroy(blob) };
        if data.as_ref().is_empty() {
            return None;
        }
        Some(data)
    }
}

impl Drop for TableFunc {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: `destroy` was supplied alongside `user_data`.
            unsafe { destroy(self.user_data) };
        }
    }
}

/// How a face gets at its table data.
enum FaceSource {
    /// No data at all; only the immortal empty face uses this.
    Empty,
    /// A blob, over which this face owns a reference.
    Blob(*mut hr_blob_t),
    /// A caller-supplied table callback.
    Function(Arc<TableFunc>),
}

impl Drop for FaceSource {
    fn drop(&mut self) {
        if let FaceSource::Blob(blob) = *self {
            // SAFETY: the face owns this reference.
            unsafe { object::destroy(blob) };
        }
    }
}

/// A font face: the tables of one font, before any size or variation settings.
pub struct hr_face_t {
    header: ObjectHeader,
    /// `None` only for the immortal empty face.
    pub(crate) font: Option<Font>,
    index: c_uint,
    source: FaceSource,
    /// Shape plans built over this face, reused across `hr_shape` calls the way
    /// HarfBuzz's cached shape plans are.
    pub(crate) plans: PlanCache,
}

impl hr_face_t {
    pub(crate) fn font(&self) -> Option<&Font> {
        self.font.as_ref()
    }

    fn upem(&self) -> c_uint {
        self.font
            .as_ref()
            .and_then(|font| font.tables().head().ok())
            .map_or(1000, |head| c_uint::from(head.units_per_em()))
    }

    fn glyph_count(&self) -> c_uint {
        self.font
            .as_ref()
            .and_then(|font| font.tables().maxp().ok())
            .map_or(0, |maxp| c_uint::from(maxp.num_glyphs()))
    }
}

static EMPTY_FACE: OnceLock<Empty<hr_face_t>> = OnceLock::new();

impl Object for hr_face_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_FACE
            .get_or_init(|| {
                Empty::new(hr_face_t {
                    header: ObjectHeader::immortal(),
                    font: None,
                    index: 0,
                    source: FaceSource::Empty,
                    plans: PlanCache::default(),
                })
            })
            .get()
    }
}

/// Creates a face over the font at `index` within `blob`.
///
/// The face takes its own reference to the blob. Never returns `NULL`; a blob
/// that does not parse yields the empty face.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_face_create(blob: *mut hr_blob_t, index: c_uint) -> *mut hr_face_t {
    let face = unsafe { hr_face_create_or_fail(blob, index) };
    if face.is_null() {
        return hr_face_t::empty();
    }
    face
}

/// Creates a face over a blob, returning `NULL` if it does not parse.
///
/// # Safety
///
/// `blob` must be `NULL` or a live blob.
#[no_mangle]
pub unsafe extern "C" fn hr_face_create_or_fail(
    blob: *mut hr_blob_t,
    index: c_uint,
) -> *mut hr_face_t {
    let Some(blob_ref) = (unsafe { blob.as_ref() }) else {
        return core::ptr::null_mut();
    };
    let Ok(font) = Font::new(blob_ref.blob.clone(), index) else {
        return core::ptr::null_mut();
    };
    // Keep the blob alive for as long as the face.
    let owned = unsafe { object::reference(blob) };
    object::create(hr_face_t {
        header: ObjectHeader::new(),
        font: Some(font),
        index,
        source: FaceSource::Blob(owned),
        plans: PlanCache::default(),
    })
}

/// Creates a face whose tables are supplied by a callback.
///
/// The callback is invoked lazily, once per table, and may be called from any
/// thread; it and its user data must be safe to use concurrently. `destroy` is
/// called with `user_data` when the face is freed.
///
/// Never returns `NULL`.
///
/// # Safety
///
/// `reference_table_func` must be safe to call with `user_data` from any
/// thread, and must return either `NULL` or a blob reference it gives up
/// ownership of.
#[no_mangle]
pub unsafe extern "C" fn hr_face_create_for_tables(
    reference_table_func: hr_reference_table_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) -> *mut hr_face_t {
    let state = Arc::new(TableFunc {
        func: reference_table_func,
        user_data,
        destroy,
        face: AtomicPtr::new(core::ptr::null_mut()),
    });
    let for_closure = Arc::clone(&state);
    let table_fn: Arc<dyn Fn(Tag) -> Option<FontBlob> + Send + Sync> =
        Arc::new(move |tag| for_closure.call(tag));
    // A table-function font is built lazily and never fails here.
    let Ok(font) = Font::new(FontTableFunction::new(table_fn), 0) else {
        return hr_face_t::empty();
    };
    let face = object::create(hr_face_t {
        header: ObjectHeader::new(),
        font: Some(font),
        index: 0,
        source: FaceSource::Function(Arc::clone(&state)),
        plans: PlanCache::default(),
    });
    // Now that the face exists, let the callback see it. This is a plain
    // pointer, not a reference, so it does not keep the face alive.
    state.face.store(face, Ordering::Release);
    face
}

/// Returns the immortal empty face.
#[no_mangle]
pub extern "C" fn hr_face_get_empty() -> *mut hr_face_t {
    hr_face_t::empty()
}

/// Increments a face's reference count.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_reference(face: *mut hr_face_t) -> *mut hr_face_t {
    unsafe { object::reference(face) }
}

/// Decrements a face's reference count, freeing it at zero.
///
/// # Safety
///
/// `face` must be `NULL` or a live face, and the caller must own the
/// reference being released.
#[no_mangle]
pub unsafe extern "C" fn hr_face_destroy(face: *mut hr_face_t) {
    unsafe { object::destroy(face) };
}

/// Attaches user data to a face.
///
/// # Safety
///
/// `face` must be `NULL` or a live face, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_face_set_user_data(
    face: *mut hr_face_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(face, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data previously attached to a face.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_get_user_data(
    face: *mut hr_face_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(face, key) }
}

/// Marks a face immutable.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_make_immutable(face: *mut hr_face_t) {
    unsafe { object::make_immutable(face) };
}

/// Returns whether a face has been marked immutable.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_is_immutable(face: *mut hr_face_t) -> hr_bool_t {
    unsafe { object::is_immutable(face.cast_const()) }.into()
}

/// Returns the index this face was created with, within its collection.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_get_index(face: *mut hr_face_t) -> c_uint {
    unsafe { object::or_empty(face.cast_const()) }.index
}

/// Returns a face's design units per em, or 1000 if it has no `head` table.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_get_upem(face: *mut hr_face_t) -> c_uint {
    unsafe { object::or_empty(face.cast_const()) }.upem()
}

/// Returns the number of glyphs in a face.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_get_glyph_count(face: *mut hr_face_t) -> c_uint {
    unsafe { object::or_empty(face.cast_const()) }.glyph_count()
}

/// Returns the blob a face was created over, or the empty blob for a face
/// built from table callbacks.
///
/// The caller owns the returned reference.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_reference_blob(face: *mut hr_face_t) -> *mut hr_blob_t {
    let face = unsafe { object::or_empty(face.cast_const()) };
    match face.source {
        FaceSource::Blob(blob) => unsafe { object::reference(blob) },
        FaceSource::Empty | FaceSource::Function(_) => hr_blob_t::empty(),
    }
}

/// Returns the data of one table, or the empty blob if the face has no such
/// table.
///
/// The caller owns the returned reference. For a blob-backed face the result
/// shares the face's storage rather than copying. For a face built from table
/// callbacks, this invokes the callback.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_face_reference_table(
    face: *mut hr_face_t,
    tag: hr_tag_t,
) -> *mut hr_blob_t {
    let face_ref = unsafe { object::or_empty(face.cast_const()) };
    match &face_ref.source {
        FaceSource::Empty => hr_blob_t::empty(),
        FaceSource::Function(state) => match state.call(tag_to_rust(tag)) {
            Some(data) => hr_blob_t::new(data),
            None => hr_blob_t::empty(),
        },
        FaceSource::Blob(blob) => {
            let Some(blob_ref) = (unsafe { blob.as_ref() }) else {
                return hr_blob_t::empty();
            };
            let bytes = blob_ref.bytes();
            let Ok(font_ref) = read_fonts::FontRef::from_index(bytes, face_ref.index) else {
                return hr_blob_t::empty();
            };
            let Some(table) = font_ref.data_for_tag(tag_to_rust(tag)) else {
                return hr_blob_t::empty();
            };
            let table = table.as_bytes();
            // `table` is a subslice of `bytes`, so this offset is well defined;
            // fall back to the empty blob rather than trusting it blindly.
            let Some(start) = (table.as_ptr() as usize).checked_sub(bytes.as_ptr() as usize) else {
                return hr_blob_t::empty();
            };
            let Some(end) = start.checked_add(table.len()) else {
                return hr_blob_t::empty();
            };
            hr_blob_t::sub(&blob_ref.blob, start, end)
        }
    }
}
