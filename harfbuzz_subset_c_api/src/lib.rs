//! Font subsetting through fontations' skera, using HarfRust C API handles.
//!
//! Link this library together with `harfrust_c`. The `hr_` prefix keeps
//! symbols separate from the C++ HarfBuzz subsetter. `hr-hb-subset.h` provides
//! HarfBuzz source aliases, not compatibility with HarfBuzz's opaque objects.

#![allow(non_camel_case_types)]

mod ffi;
pub use ffi::{hr_blob_t, hr_face_t, hr_set_t, hr_subset_flags_t};

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

/// Retain non-Unicode name records.
pub const HR_SUBSET_FLAGS_NAME_LEGACY: hr_subset_flags_t = 0x8;
/// Set the TrueType overlap flag on simple glyphs.
pub const HR_SUBSET_FLAGS_SET_OVERLAPS_FLAG: hr_subset_flags_t = 0x10;
/// Copy unrecognized tables unchanged.
pub const HR_SUBSET_FLAGS_PASSTHROUGH_UNRECOGNIZED: hr_subset_flags_t = 0x20;
/// Retain PostScript glyph names.
pub const HR_SUBSET_FLAGS_GLYPH_NAMES: hr_subset_flags_t = 0x80;
/// Preserve the OS/2 Unicode range bits.
pub const HR_SUBSET_FLAGS_NO_PRUNE_UNICODE_RANGES: hr_subset_flags_t = 0x100;
/// Disable layout substitution glyph closure.
pub const HR_SUBSET_FLAGS_NO_LAYOUT_CLOSURE: hr_subset_flags_t = 0x200;
/// Disable mirrored Unicode closure.
pub const HR_SUBSET_FLAGS_NO_BIDI_CLOSURE: hr_subset_flags_t = 0x800;

const SUPPORTED_FLAGS: c_uint = HR_SUBSET_FLAGS_NO_HINTING
    | HR_SUBSET_FLAGS_RETAIN_GIDS
    | HR_SUBSET_FLAGS_NAME_LEGACY
    | HR_SUBSET_FLAGS_SET_OVERLAPS_FLAG
    | HR_SUBSET_FLAGS_PASSTHROUGH_UNRECOGNIZED
    | HR_SUBSET_FLAGS_NOTDEF_OUTLINE
    | HR_SUBSET_FLAGS_GLYPH_NAMES
    | HR_SUBSET_FLAGS_NO_PRUNE_UNICODE_RANGES
    | HR_SUBSET_FLAGS_NO_LAYOUT_CLOSURE
    | HR_SUBSET_FLAGS_NO_BIDI_CLOSURE;

/// Identifies a configurable input set, with HarfBuzz's numeric values.
pub type hr_subset_sets_t = c_uint;
/// Glyph IDs to retain.
pub const HR_SUBSET_SETS_GLYPH_INDEX: hr_subset_sets_t = 0;
/// Unicode codepoints to retain.
pub const HR_SUBSET_SETS_UNICODE: hr_subset_sets_t = 1;
/// Tables to copy unchanged (currently the Skera default set only).
pub const HR_SUBSET_SETS_NO_SUBSET_TABLE_TAG: hr_subset_sets_t = 2;
/// Tables to omit from the output.
pub const HR_SUBSET_SETS_DROP_TABLE_TAG: hr_subset_sets_t = 3;
/// Name IDs to retain.
pub const HR_SUBSET_SETS_NAME_ID: hr_subset_sets_t = 4;
/// Name language IDs to retain.
pub const HR_SUBSET_SETS_NAME_LANG_ID: hr_subset_sets_t = 5;
/// Layout feature tags to retain.
pub const HR_SUBSET_SETS_LAYOUT_FEATURE_TAG: hr_subset_sets_t = 6;
/// Layout script tags to retain.
pub const HR_SUBSET_SETS_LAYOUT_SCRIPT_TAG: hr_subset_sets_t = 7;

const DEFAULT_NO_SUBSET_TABLES: [Tag; 5] = [
    Tag::new(b"gasp"),
    Tag::new(b"fpgm"),
    Tag::new(b"prep"),
    Tag::new(b"VDMX"),
    Tag::new(b"DSIG"),
];

/// Subsetting configuration, owning its glyph and Unicode sets.
///
/// Mutation requires exclusive access. Returned sets are borrowed; take a
/// set reference if it must outlive this input.
pub struct hr_subset_input_t {
    references: AtomicUsize,
    sets: [*mut hr_set_t; 8],
    flags: c_uint,
}

impl Drop for hr_subset_input_t {
    fn drop(&mut self) {
        unsafe {
            for set in self.sets {
                ffi::hr_set_destroy(set);
            }
        }
    }
}

/// Creates a subset input with empty glyph and Unicode selections.
///
/// As in the shaping C API, Rust allocation failure aborts the process.
#[no_mangle]
pub extern "C" fn hr_subset_input_create_or_fail() -> *mut hr_subset_input_t {
    let sets = std::array::from_fn(|_| unsafe { ffi::hr_set_create() });
    unsafe {
        for tag in DEFAULT_NO_SUBSET_TABLES {
            ffi::hr_set_add(sets[2], u32::from_be_bytes(tag.to_be_bytes()));
        }
        for tag in skera::DEFAULT_DROP_TABLES {
            ffi::hr_set_add(sets[3], u32::from_be_bytes(tag.to_be_bytes()));
        }
        ffi::hr_set_add_range(sets[4], 0, 6);
        ffi::hr_set_add(sets[5], 0x0409);
        for tag in skera::DEFAULT_LAYOUT_FEATURES {
            ffi::hr_set_add(sets[6], u32::from_be_bytes(tag.to_be_bytes()));
        }
        ffi::hr_set_invert(sets[7]);
    }
    Box::into_raw(Box::new(hr_subset_input_t {
        references: AtomicUsize::new(1),
        sets,
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
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.sets[0])
}

/// Returns the borrowed set of Unicode codepoints to retain.
///
/// # Safety
/// `input` must be `NULL` or live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_unicode_set(
    input: *mut hr_subset_input_t,
) -> *mut hr_set_t {
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.sets[1])
}

/// Returns a borrowed configurable input set, or `NULL` for an invalid selector.
///
/// Changes to the no-subset table set currently cause subsetting to fail:
/// published Skera does not expose customization of that set yet.
///
/// # Safety
/// `input` must be `NULL` or live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_set(
    input: *mut hr_subset_input_t,
    set_type: hr_subset_sets_t,
) -> *mut hr_set_t {
    unsafe { input.as_ref() }
        .and_then(|input| input.sets.get(set_type as usize).copied())
        .unwrap_or(ptr::null_mut())
}

/// Configures all glyphs, Unicodes, names, and layout items to be retained.
/// The input can be tailored afterwards. Table passthrough and glyph names
/// are enabled; no tables are explicitly dropped.
///
/// # Safety
/// `input` must be `NULL` or live and exclusively accessible.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_keep_everything(input: *mut hr_subset_input_t) {
    let Some(input) = (unsafe { input.as_mut() }) else {
        return;
    };
    unsafe {
        for i in [0, 1, 4, 5, 6, 7] {
            ffi::hr_set_clear(input.sets[i]);
            ffi::hr_set_invert(input.sets[i]);
        }
        ffi::hr_set_clear(input.sets[3]);
    }
    input.flags = HR_SUBSET_FLAGS_NOTDEF_OUTLINE
        | HR_SUBSET_FLAGS_GLYPH_NAMES
        | HR_SUBSET_FLAGS_NAME_LEGACY
        | HR_SUBSET_FLAGS_NO_PRUNE_UNICODE_RANGES
        | HR_SUBSET_FLAGS_PASSTHROUGH_UNRECOGNIZED;
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
        unsafe { ffi::hr_blob_destroy(self.0) };
    }
}

unsafe fn font_bytes(face: *mut hr_face_t) -> Option<Vec<u8>> {
    // Enumeration also handles callback-only fonts and TTC faces, regardless
    // of the face's separately writable index metadata.
    let mut count = 0;
    let total = unsafe { ffi::hr_face_get_table_tags(face, 0, &raw mut count, ptr::null_mut()) };
    if total == 0 || total > u16::MAX as u32 {
        return None;
    }
    let mut tags = vec![0; total as usize];
    count = total;
    let enumerated =
        unsafe { ffi::hr_face_get_table_tags(face, 0, &raw mut count, tags.as_mut_ptr()) };
    if enumerated != total || count != total {
        return None;
    }
    let mut builder = FontBuilder::new();
    for tag in tags {
        let blob = Blob(unsafe { ffi::hr_face_reference_table(face, tag) });
        let mut len = 0;
        let data = unsafe { ffi::hr_blob_get_data(blob.0, &raw mut len) };
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
    while unsafe { ffi::hr_set_next_range(set, &raw mut first, &raw mut last) } != 0 {
        if first > max {
            break;
        }
        result.insert_range(first..=last.min(max));
        if last >= max {
            break;
        }
    }
    result
}

struct Set(*mut hr_set_t);
impl Drop for Set {
    fn drop(&mut self) {
        unsafe { ffi::hr_set_destroy(self.0) };
    }
}

unsafe fn tags(set: *mut hr_set_t) -> IntSet<Tag> {
    // Preserve the inverted representation: Skera recognizes it when collecting
    // layout scripts. Enumerating a dense all-tags set would visit billions of tags.
    let inverted = unsafe { ffi::hr_set_is_inverted(set) } != 0;
    let copy = if inverted {
        let copy = Set(unsafe { ffi::hr_set_copy(set) });
        unsafe { ffi::hr_set_invert(copy.0) };
        Some(copy)
    } else {
        None
    };
    let source = copy.as_ref().map_or(set, |copy| copy.0);
    let mut result = IntSet::empty();
    for range in unsafe { ranges(source, u32::MAX) }.iter_ranges() {
        result.insert_range(
            Tag::from_be_bytes(range.start().to_be_bytes())
                ..=Tag::from_be_bytes(range.end().to_be_bytes()),
        );
    }
    if inverted {
        result.invert();
        result.remove(Tag::from_be_bytes(u32::MAX.to_be_bytes()));
    }
    result
}

unsafe fn subset(face: *mut hr_face_t, input: &hr_subset_input_t) -> Option<Vec<u8>> {
    if input.flags & !SUPPORTED_FLAGS != 0 {
        return None;
    }
    let count = unsafe { ffi::hr_face_get_glyph_count(face) };
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
    let glyphs: IntSet<GlyphId> = unsafe { ranges(input.sets[0], count - 1) }
        .iter_ranges()
        .map(|range| GlyphId::from(*range.start())..=GlyphId::from(*range.end()))
        .fold(IntSet::empty(), |mut set, range| {
            set.insert_range(range);
            set
        });
    let unicodes = unsafe { ranges(input.sets[1], 0x10_FFFF) };
    if unsafe { tags(input.sets[2]) } != DEFAULT_NO_SUBSET_TABLES.into_iter().collect() {
        return None;
    }
    let drop_tables = unsafe { tags(input.sets[3]) };
    let scripts = unsafe { tags(input.sets[7]) };
    let features = unsafe { tags(input.sets[6]) };
    let mut name_ids = IntSet::<NameId>::empty();
    for range in unsafe { ranges(input.sets[4], u16::MAX.into()) }.iter_ranges() {
        name_ids
            .insert_range(NameId::from(*range.start() as u16)..=NameId::from(*range.end() as u16));
    }
    let mut name_languages = IntSet::<u16>::empty();
    for range in unsafe { ranges(input.sets[5], u16::MAX.into()) }.iter_ranges() {
        name_languages.insert_range(*range.start() as u16..=*range.end() as u16);
    }
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
    unsafe { face_from_bytes(&bytes) }
}

unsafe fn face_from_bytes(bytes: &[u8]) -> *mut hr_face_t {
    let Ok(len) = bytes.len().try_into() else {
        return ptr::null_mut();
    };
    let blob = Blob(unsafe {
        ffi::hr_blob_create_or_fail(bytes.as_ptr().cast(), len, 0, ptr::null_mut(), None)
    });
    unsafe { ffi::hr_face_create_or_fail(blob.0, 0) }
}

/// Copies a font's tables into an immutable, self-contained face for reuse.
///
/// This preserves the preprocessing API's ownership contract; it currently
/// does not add a Skera acceleration cache. It accepts any readable SFNT,
/// including outline formats that the subset operation cannot yet rewrite.
///
/// # Safety
/// `face` must be `NULL` or live, with callbacks satisfying the core API contract.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_preprocess(face: *mut hr_face_t) -> *mut hr_face_t {
    let Ok(Some(bytes)) = catch_unwind(AssertUnwindSafe(|| unsafe { font_bytes(face) })) else {
        return ptr::null_mut();
    };
    let result = unsafe { face_from_bytes(&bytes) };
    unsafe { ffi::hr_face_make_immutable(result) };
    result
}
