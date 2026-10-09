//! Font subsetting through fontations' skera, using HarfRust C API handles.
//!
//! Enable the `subset` feature of `harfrust_capi`. The `hr_` prefix keeps
//! symbols separate from the C++ HarfBuzz subsetter. `hr-hb-subset.h` provides
//! HarfBuzz source aliases, not compatibility with HarfBuzz's opaque objects.

use crate::{hr_blob_t, hr_face_t, hr_set_t};

/// Font subsetting flags, matching HarfBuzz.
pub type hr_subset_flags_t = c_uint;

use core::{ffi::c_uint, ptr};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicUsize, Ordering},
};
use write_fonts::{
    read::{collections::IntSet, FontRef, TableProvider},
    types::{GlyphId, NameId, Tag},
    FontBuilder,
};

/// All supported subset flags are unset.
pub const HR_SUBSET_FLAGS_DEFAULT: hr_subset_flags_t = 0;
/// Remove TrueType hinting instructions.
pub const HR_SUBSET_FLAGS_NO_HINTING: hr_subset_flags_t = 0x1;
/// Keep original glyph IDs, leaving holes for removed glyphs.
pub const HR_SUBSET_FLAGS_RETAIN_GIDS: hr_subset_flags_t = 0x2;
/// Preserve the outline of glyph zero.
pub const HR_SUBSET_FLAGS_NOTDEF_OUTLINE: hr_subset_flags_t = 0x40;

const SUPPORTED_FLAGS: c_uint =
    HR_SUBSET_FLAGS_NO_HINTING | HR_SUBSET_FLAGS_RETAIN_GIDS | HR_SUBSET_FLAGS_NOTDEF_OUTLINE;

/// Subsetting configuration, owning its glyph and Unicode sets.
///
/// Mutation requires exclusive access. Returned sets are borrowed; take a
/// set reference if it must outlive this input.
pub struct hr_subset_input_t {
    references: AtomicUsize,
    glyphs: *mut hr_set_t,
    unicodes: *mut hr_set_t,
    flags: c_uint,
}

impl Drop for hr_subset_input_t {
    fn drop(&mut self) {
        unsafe {
            crate::hr_set_destroy(self.glyphs);
            crate::hr_set_destroy(self.unicodes);
        }
    }
}

/// Creates a subset input with empty glyph and Unicode selections.
///
/// As in the shaping C API, Rust allocation failure aborts the process.
#[no_mangle]
pub extern "C" fn hr_subset_input_create_or_fail() -> *mut hr_subset_input_t {
    Box::into_raw(Box::new(hr_subset_input_t {
        references: AtomicUsize::new(1),
        glyphs: crate::hr_set_create(),
        unicodes: crate::hr_set_create(),
        flags: HR_SUBSET_FLAGS_DEFAULT,
    }))
}

/// Takes another reference to a live input. `NULL` stays `NULL`.
///
/// # Safety
/// `input` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_reference(
    input: *mut hr_subset_input_t,
) -> *mut hr_subset_input_t {
    if let Some(input) = unsafe { input.as_ref() } {
        input.references.fetch_add(1, Ordering::Relaxed);
    }
    input
}

/// Releases an input reference and its owned sets. Accepts `NULL`.
///
/// # Safety
/// `input` must be `NULL` or an owned reference.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_destroy(input: *mut hr_subset_input_t) {
    let Some(state) = (unsafe { input.as_ref() }) else {
        return;
    };
    if state.references.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(unsafe { Box::from_raw(input) });
    }
}

/// Returns the borrowed set of glyph IDs to retain, or `NULL` for no input.
///
/// # Safety
/// `input` must be `NULL` or live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_glyph_set(input: *mut hr_subset_input_t) -> *mut hr_set_t {
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.glyphs)
}

/// Returns the borrowed set of Unicode codepoints to retain.
///
/// # Safety
/// `input` must be `NULL` or live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_unicode_set(
    input: *mut hr_subset_input_t,
) -> *mut hr_set_t {
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.unicodes)
}

/// Sets the subset flags. Unsupported flag bits cause subsetting to fail.
///
/// # Safety
/// `input` must be `NULL` or live and exclusively accessible.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_set_flags(input: *mut hr_subset_input_t, flags: c_uint) {
    if let Some(input) = unsafe { input.as_mut() } {
        input.flags = flags;
    }
}

/// Returns the configured flags, or zero for `NULL`.
///
/// # Safety
/// `input` must be `NULL` or live and not concurrently mutated.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_get_flags(
    input: *const hr_subset_input_t,
) -> hr_subset_flags_t {
    unsafe { input.as_ref() }.map_or(0, |input| input.flags)
}

// A reference returned by the shaping API, released even if skera panics.
struct Blob(*mut hr_blob_t);
impl Drop for Blob {
    fn drop(&mut self) {
        unsafe { crate::hr_blob_destroy(self.0) };
    }
}

unsafe fn font_bytes(face: *mut hr_face_t) -> Option<Vec<u8>> {
    // Enumeration also handles callback-only fonts and TTC faces, regardless
    // of the face's separately writable index metadata.
    let mut count = 0;
    let total = unsafe { crate::hr_face_get_table_tags(face, 0, &raw mut count, ptr::null_mut()) };
    if total == 0 || total > u16::MAX as u32 {
        return None;
    }
    let mut tags = vec![0; total as usize];
    count = total;
    let enumerated =
        unsafe { crate::hr_face_get_table_tags(face, 0, &raw mut count, tags.as_mut_ptr()) };
    if enumerated != total || count != total {
        return None;
    }
    let mut builder = FontBuilder::new();
    for tag in tags {
        let blob = Blob(unsafe { crate::hr_face_reference_table(face, tag) });
        let mut len = 0;
        let data = unsafe { crate::hr_blob_get_data(blob.0, &raw mut len) };
        if data.is_null() && len != 0 {
            return None;
        }
        let data = if len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) }
        };
        builder.add_raw(Tag::from_be_bytes(tag.to_be_bytes()), data.to_vec());
    }
    Some(builder.build())
}

unsafe fn ranges(set: *mut hr_set_t, max: u32) -> IntSet<u32> {
    let mut result = IntSet::empty();
    let (mut first, mut last) = (u32::MAX, u32::MAX);
    while unsafe { crate::hr_set_next_range(set, &raw mut first, &raw mut last) } != 0 {
        if first > max {
            break;
        }
        result.insert_range(first..=last.min(max));
    }
    result
}

unsafe fn subset(face: *mut hr_face_t, input: &hr_subset_input_t) -> Option<Vec<u8>> {
    if input.flags & !SUPPORTED_FLAGS != 0 {
        return None;
    }
    let count = unsafe { crate::hr_face_get_glyph_count(face) };
    if count == 0 {
        return None;
    }
    let bytes = unsafe { font_bytes(face) }?;
    let font = FontRef::new(&bytes).ok()?;
    // skera currently has no CFF/CFF2/VARC rewriting. Returning failure lets
    // callers keep the full font instead of embedding a font without outlines.
    if [b"CFF ", b"CFF2", b"VARC"]
        .iter()
        .any(|tag| font.data_for_tag(Tag::new(tag)).is_some())
    {
        return None;
    }
    let glyphs: IntSet<GlyphId> = unsafe { ranges(input.glyphs, count - 1) }
        .iter_ranges()
        .map(|range| GlyphId::from(*range.start())..=GlyphId::from(*range.end()))
        .fold(IntSet::empty(), |mut set, range| {
            set.insert_range(range);
            set
        });
    let unicodes = unsafe { ranges(input.unicodes, 0x10_FFFF) };
    let drop_tables = skera::DEFAULT_DROP_TABLES.iter().copied().collect();
    let mut scripts = IntSet::<Tag>::empty();
    scripts.invert();
    let features = skera::DEFAULT_LAYOUT_FEATURES.iter().copied().collect();
    let mut name_ids = IntSet::<NameId>::empty();
    name_ids.insert_range(NameId::from(0)..=NameId::from(6));
    let name_languages = [0x0409u16].into_iter().collect();
    let plan = skera::Plan::new(
        &glyphs,
        &unicodes,
        &font,
        skera::SubsetFlags::from(input.flags as u16),
        &drop_tables,
        &scripts,
        &features,
        &name_ids,
        &name_languages,
    );
    skera::subset_font(&font, &plan).ok()
}

/// Subsets a face with skera, returning a new face with serialized SFNT data.
///
/// Returns `NULL` for invalid input, unsupported flags or outline formats,
/// missing table enumeration, or skera failure. The original face and input
/// are unchanged. Rust panics in the subsetter are contained at this boundary.
///
/// # Safety
/// `face` and `input` must be `NULL` or live. Neither the input nor its sets
/// may be modified during the call. Face callbacks must satisfy HarfRust's
/// threading contract.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_or_fail(
    face: *mut hr_face_t,
    input: *const hr_subset_input_t,
) -> *mut hr_face_t {
    let Some(input) = (unsafe { input.as_ref() }) else {
        return ptr::null_mut();
    };
    let Ok(Some(bytes)) = catch_unwind(AssertUnwindSafe(|| unsafe { subset(face, input) })) else {
        return ptr::null_mut();
    };
    let Ok(len) = bytes.len().try_into() else {
        return ptr::null_mut();
    };
    let blob = Blob(unsafe {
        crate::hr_blob_create_or_fail(
            bytes.as_ptr().cast(),
            len,
            crate::HR_MEMORY_MODE_DUPLICATE,
            ptr::null_mut(),
            None,
        )
    });
    unsafe { crate::hr_face_create_or_fail(blob.0, 0) }
}
