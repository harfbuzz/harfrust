//! Font subsetting through fontations' skera, using HarfRust C API handles.
//!
//! Enable the `subset` feature of `harfrust_capi`. The `hr_` prefix keeps
//! symbols separate from the C++ HarfBuzz subsetter. `hr-hb-subset.h` provides
//! HarfBuzz source aliases, not compatibility with HarfBuzz's opaque objects.

mod user_data;
use crate::{hr_blob_t, hr_face_t, hr_map_t, hr_set_t};

/// Font subsetting flags, matching HarfBuzz.
pub type hr_subset_flags_t = c_uint;

use core::{
    ffi::{c_int, c_uint, c_void},
    ptr,
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicI64, Ordering},
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
/// Tables to copy unchanged.
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

/// Reference count of an input or plan whose last reference has been
/// released, for as long as its destroy callbacks run.
///
/// A callback that takes a reference of its own and gives it back would
/// otherwise walk the count from zero to one and back, releasing the object
/// a second time. Parked this far below zero, no number of references a
/// callback could take and release brings the count back to one.
///
/// The count is 64 bits wide on every target, as in `object.rs`, so that
/// neither this nor a live count can be walked all the way round.
const DYING: i64 = i64::MIN / 2;

/// Subsetting configuration, owning its glyph and Unicode sets.
///
/// Mutation requires exclusive access. Returned sets are borrowed; take a
/// set reference if it must outlive this input.
pub struct hr_subset_input_t {
    references: AtomicI64,
    // Safety invariant: each is a live set, one of whose references this
    // input owns.
    sets: [*mut hr_set_t; 8],
    flags: c_uint,
    user_data: user_data::UserData,
}

impl Drop for hr_subset_input_t {
    fn drop(&mut self) {
        self.user_data.release();
        // SAFETY: `hr_set_destroy` needs each `set` to be `NULL` or live, and
        // the reference it gives up to be its caller's own. By the invariant
        // on `sets` each is live and one of its references is this input's,
        // which the input, dropped once, gives up here.
        unsafe {
            for set in self.sets {
                crate::hr_set_destroy(set);
            }
        }
    }
}

/// Creates a subset input with empty glyph and Unicode selections.
///
/// As in the shaping C API, Rust allocation failure aborts the process.
#[no_mangle]
pub extern "C" fn hr_subset_input_create_or_fail() -> *mut hr_subset_input_t {
    let sets = std::array::from_fn(|_| crate::hr_set_create());
    // SAFETY: every call in this block takes a set that must be live, and
    // modifies it, which takes exclusive access to it. Each entry of `sets`
    // is a new set that `hr_set_create` returned just above. This function
    // still holds the reference each came with, and nothing else has been
    // given their pointers.
    unsafe {
        for tag in DEFAULT_NO_SUBSET_TABLES {
            crate::hr_set_add(sets[2], u32::from_be_bytes(tag.to_be_bytes()));
        }
        for tag in skera::DEFAULT_DROP_TABLES {
            crate::hr_set_add(sets[3], u32::from_be_bytes(tag.to_be_bytes()));
        }
        crate::hr_set_add_range(sets[4], 0, 6);
        crate::hr_set_add(sets[5], 0x0409);
        for tag in skera::DEFAULT_LAYOUT_FEATURES {
            crate::hr_set_add(sets[6], u32::from_be_bytes(tag.to_be_bytes()));
        }
        crate::hr_set_invert(sets[7]);
    }
    Box::into_raw(Box::new(hr_subset_input_t {
        references: AtomicI64::new(1),
        // The invariant on `sets` holds: each is the new set made above, and
        // the reference it came with becomes the input's.
        sets,
        flags: HR_SUBSET_FLAGS_DEFAULT,
        user_data: user_data::UserData::default(),
    }))
}

/// Takes another reference to a live input. `NULL` stays `NULL`.
///
/// A reference taken from one of the input's own destroy callbacks, while its
/// last reference is being released, does not keep the input alive. It must
/// be given back before that callback returns.
///
/// # Safety
/// `input` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_reference(
    input: *mut hr_subset_input_t,
) -> *mut hr_subset_input_t {
    // SAFETY: `as_ref` needs `input` to be `NULL`, or a live input that
    // nothing writes while the reference is used. This function's contract
    // has it `NULL` or live. The setters, which write an input, need
    // exclusive access to it, and the call that frees it writes it only
    // when nothing else uses it.
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
    // SAFETY: `as_ref` needs `input` to be `NULL`, or a live input that
    // nothing writes while `state` is used. This function's contract has it
    // `NULL` or a reference the caller owns, which keeps the input
    // allocated until the count below gives that reference up. After that
    // `state` is used only by a call that took the count from one to zero,
    // which is the call that frees the input, once it is done with `state`.
    // The setters, which write an input, need exclusive access to it, and
    // what this call writes of it, it writes once it is done with `state`.
    let Some(state) = (unsafe { input.as_ref() }) else {
        return;
    };
    if state.references.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        state.references.store(DYING, Ordering::Relaxed);
        // Callbacks may query the object. Run them while access is shared,
        // before reconstructing the Box and taking exclusive ownership.
        state.user_data.release();
        // The sets go here too, rather than when the input is dropped. The
        // input's reference to one may be the last, and releasing that runs
        // the destroy functions of whatever the set was given. Those are the
        // caller's code, and may ask the input a question. Here nothing
        // borrows the input. Inside its drop, the drop does.
        //
        // Each set is replaced by the empty one before it is released, so
        // what its release runs finds an input that has let go of it, and
        // still has the sets that come after.
        for index in 0..8 {
            // SAFETY: this names one of the input's sets through `input` and
            // swaps the empty set into it with `replace`.
            // - Naming it needs `input` to point into an allocation that
            //   holds an input, and `index` to be one of its eight sets.
            //   `replace` needs the entry to be aligned, to hold an
            //   initialized value, and to be valid for reads and writes,
            //   with nothing else using it meanwhile.
            // - `input` is live, as for `state` above, and is the pointer
            //   that `hr_subset_input_create_or_fail` got from
            //   `Box::into_raw`, so the entry is in bounds, aligned and
            //   initialized, and may be written through it.
            // - Nothing else is using the input. The reference released
            //   above was the last, so only destructors this call runs can
            //   be, and those run so far have returned: the ones `release`
            //   ran, and the ones of the sets already released. None can
            //   have written the input: the setters need exclusive access
            //   to it, which nothing has while this call is under way.
            //   `state` is not used again.
            // Afterwards the entry holds the empty set, which is live for
            // good and asks for no reference, as the invariant on `sets`
            // needs, and `set` is the reference the input owned.
            let set = unsafe { (&raw mut (*input).sets[index]).replace(crate::hr_set_get_empty()) };
            // SAFETY: `hr_set_destroy` needs `NULL` or a live set, and the
            // reference it gives up to be its caller's own. `set` is what
            // the entry held: by the invariant on `sets`, a live set with a
            // reference that was the input's. That reference is this
            // call's now that the entry holds another set, and nothing has
            // released it since.
            unsafe { crate::hr_set_destroy(set) };
        }
        // SAFETY: `Box::from_raw` needs the pointer `Box::into_raw` gave for
        // this input, not yet freed, and nothing else to use the input
        // from here on.
        // - `input` is not `NULL`, and every input the API hands out is
        //   the one `hr_subset_input_create_or_fail` got from `Box::into_raw`.
        // - The caller's reference kept it allocated up to this call, and
        //   an input is freed only by the call that takes its count from one
        //   to zero, which happens once: the count is parked at `DYING`
        //   since, and 64 bits cannot be counted round.
        // - That reference was the last. The destructors run above, by
        //   `release` and by letting go of the sets, may still have used
        //   the input, and have all returned, having given back any
        //   reference they took, as the README requires. `state` is not
        //   used again.
        // - Dropping the input runs none of the caller's code, which could
        //   still reach it through `input`: its user data was released
        //   above and takes no more, and its sets are the empty set.
        drop(unsafe { Box::from_raw(input) });
    }
}

/// Returns the borrowed set of glyph IDs to retain.
///
/// # Safety
/// `input` must be live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_glyph_set(input: *mut hr_subset_input_t) -> *mut hr_set_t {
    // SAFETY: `input` is `NULL`, or live by this function's contract. Nothing
    // writes it meanwhile: the setters need exclusive access to an input,
    // and the call that frees it writes it only when nothing else uses it.
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.sets[0])
}

/// Returns the borrowed set of Unicode codepoints to retain.
///
/// # Safety
/// `input` must be live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_unicode_set(
    input: *mut hr_subset_input_t,
) -> *mut hr_set_t {
    // SAFETY: `input` is `NULL`, or live by this function's contract. Nothing
    // writes it meanwhile: the setters need exclusive access to an input,
    // and the call that frees it writes it only when nothing else uses it.
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.sets[1])
}

/// Returns a borrowed configurable input set, or `NULL` for an invalid selector.
///
/// # Safety
/// `input` must be live; the returned set is borrowed from it.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_set(
    input: *mut hr_subset_input_t,
    set_type: hr_subset_sets_t,
) -> *mut hr_set_t {
    // SAFETY: `input` is `NULL`, or live by this function's contract. Nothing
    // writes it meanwhile: the setters need exclusive access to an input,
    // and the call that frees it writes it only when nothing else uses it.
    unsafe { input.as_ref() }
        .and_then(|input| input.sets.get(set_type as usize).copied())
        .unwrap_or(ptr::null_mut())
}

/// Configures all glyphs, Unicodes, names, and layout items to be retained.
/// The input can be tailored afterwards. Table passthrough and glyph names
/// are enabled; no tables are explicitly dropped.
///
/// # Safety
/// `input` must be live and, along with its sets, exclusively
/// accessible: no other call may be using them, which rules out calling this
/// from a destructor of the input's own metadata.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_keep_everything(input: *mut hr_subset_input_t) {
    // SAFETY: `as_mut` needs `input` to be `NULL`, or live with no other
    // reference to it in use while this one is. It is `NULL`, or by this
    // function's contract live with no other call using it, and the
    // reference ends when this returns.
    let Some(input) = (unsafe { input.as_mut() }) else {
        return;
    };
    // SAFETY: every call in this block takes a set that must be live, and
    // modifies it, which takes exclusive access to it. By the invariant on
    // `sets` each is live, and this function's contract has no other call
    // using the input's sets.
    unsafe {
        for i in [0, 1, 4, 5, 6, 7] {
            crate::hr_set_clear(input.sets[i]);
            crate::hr_set_invert(input.sets[i]);
        }
        crate::hr_set_clear(input.sets[3]);
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
/// `input` must be live and exclusively accessible: no other call
/// may be using it, which rules out calling this from a destructor of the
/// input's own metadata.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_set_flags(input: *mut hr_subset_input_t, flags: c_uint) {
    // SAFETY: `as_mut` needs `input` to be `NULL`, or live with no other
    // reference to it in use while this one is. It is `NULL`, or by this
    // function's contract live with no other call using it, and the
    // reference ends when this returns.
    if let Some(input) = unsafe { input.as_mut() } {
        input.flags = flags;
    }
}

/// Returns the configured flags.
///
/// # Safety
/// `input` must be live and not concurrently mutated.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_get_flags(
    input: *const hr_subset_input_t,
) -> hr_subset_flags_t {
    // SAFETY: `input` is `NULL`, or live and not concurrently mutated by
    // this function's contract.
    unsafe { input.as_ref() }.map_or(0, |input| input.flags)
}

// A reference returned by the shaping API, released even if skera panics.
struct Blob(
    // Safety invariant: `NULL`, or a live blob, one of whose references this
    // owns.
    *mut hr_blob_t,
);
impl Drop for Blob {
    fn drop(&mut self) {
        // SAFETY: `hr_blob_destroy` needs `NULL` or a live blob, and the
        // reference it gives up to be its caller's own. The invariant on
        // `Blob` gives both, and a `Blob` is dropped once.
        unsafe { crate::hr_blob_destroy(self.0) };
    }
}

/// # Safety
/// `face` must be `NULL` or live.
unsafe fn font_bytes(face: *mut hr_face_t) -> Option<Vec<u8>> {
    // Enumeration also handles callback-only fonts and TTC faces, regardless
    // of the face's separately writable index metadata.
    let mut count = 0;
    // SAFETY: `face` is `NULL` or live by this function's contract. `count`
    // is a local that is set, and a `NULL` array asks only for the total.
    let total = unsafe { crate::hr_face_get_table_tags(face, 0, &raw mut count, ptr::null_mut()) };
    if total == 0 || total > u16::MAX as u32 {
        return None;
    }
    let mut tags = vec![0; total as usize];
    count = total;
    // SAFETY: `face` is `NULL` or live by this function's contract. `count`
    // is a local holding `total`, the capacity it advertises for `tags`,
    // which has `total` elements to write to.
    let enumerated =
        unsafe { crate::hr_face_get_table_tags(face, 0, &raw mut count, tags.as_mut_ptr()) };
    if enumerated != total || count != total {
        return None;
    }
    // Tables that do not overlap add up to at most the face's blob; charge
    // each one to that budget so a directory whose records alias the same
    // bytes cannot have them copied once per tag. A callback face supplies
    // its own tables and has no such bound.
    //
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let mut budget = unsafe { crate::object::or_empty(face.cast_const()) }
        .blob_len()
        .unwrap_or(usize::MAX);
    let mut builder = FontBuilder::new();
    for tag in tags {
        // SAFETY: `face` is `NULL` or live by this function's contract. The
        // blob that comes back is live and its reference the caller's, as
        // the invariant on `Blob` asks.
        let blob = Blob(unsafe { crate::hr_face_reference_table(face, tag) });
        let mut len = 0;
        // SAFETY: `blob.0` is `NULL` or live by the invariant on `Blob`, and
        // `len` is a local for the length to be written to.
        let data = unsafe { crate::hr_blob_get_data(blob.0, &raw mut len) };
        if data.is_null() && len != 0 {
            return None;
        }
        budget = budget.checked_sub(len as usize)?;
        let data = if len == 0 {
            &[]
        } else {
            // SAFETY: `from_raw_parts` needs `data` to be non-null and to
            // point to `len` initialized bytes that stay allocated and
            // unwritten while the slice is used; a `u8` has no alignment to
            // meet. `len` is not zero here, so `data` passed the check
            // above. `hr_blob_get_data` returned the two as the data of
            // `blob.0` and its length, and a blob's data stays where it is,
            // unwritten, for as long as the blob is live, which `blob`
            // keeps it to the end of this iteration. The slice is copied on
            // the next line and not kept.
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) }
        };
        builder.add_raw(Tag::from_be_bytes(tag.to_be_bytes()), data.to_vec());
    }
    Some(builder.build())
}

/// # Safety
/// `set` must be `NULL` or live.
unsafe fn ranges(set: *mut hr_set_t, max: u32) -> IntSet<u32> {
    let mut result = IntSet::empty();
    let (mut first, mut last) = (u32::MAX, u32::MAX);
    // SAFETY: `set` is `NULL` or live by this function's contract. `first`
    // and `last` are locals, both set, `last` being the iteration cursor.
    while unsafe { crate::hr_set_next_range(set, &raw mut first, &raw mut last) } != 0 {
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

struct Set(
    // Safety invariant: `NULL`, or a live set, one of whose references this
    // owns.
    *mut hr_set_t,
);
impl Drop for Set {
    fn drop(&mut self) {
        // SAFETY: `hr_set_destroy` needs `NULL` or a live set, and the
        // reference it gives up to be its caller's own. The invariant on
        // `Set` gives both, and a `Set` is dropped once.
        unsafe { crate::hr_set_destroy(self.0) };
    }
}

/// # Safety
/// `set` must be `NULL` or live.
unsafe fn tags(set: *mut hr_set_t) -> IntSet<Tag> {
    // Preserve the inverted representation: Skera recognizes it when collecting
    // layout scripts. Enumerating a dense all-tags set would visit billions of tags.
    //
    // SAFETY: `set` is `NULL` or live by this function's contract.
    let inverted = unsafe { crate::hr_set_is_inverted(set) } != 0;
    let copy = if inverted {
        // SAFETY: `set` is `NULL` or live by this function's contract. The
        // copy that comes back is a new set and its reference the caller's,
        // as the invariant on `Set` asks.
        let copy = Set(unsafe { crate::hr_set_copy(set) });
        // SAFETY: `copy.0` is `NULL` or live by the invariant on `Set`.
        // Modifying it takes exclusive access, and nothing else has been
        // given the pointer to a copy made just above.
        unsafe { crate::hr_set_invert(copy.0) };
        Some(copy)
    } else {
        None
    };
    let source = copy.as_ref().map_or(set, |copy| copy.0);
    let mut result = IntSet::empty();
    // SAFETY: `ranges` needs `source` to be `NULL` or live. It is `set`,
    // which is by this function's contract, or `copy.0`, which is by the
    // invariant on `Set` for as long as `copy` is held, to the end of this
    // function.
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

/// # Safety
/// `face` must be `NULL` or live.
unsafe fn make_plan(
    face: *mut hr_face_t,
    input: &hr_subset_input_t,
) -> Option<(Vec<u8>, skera::Plan)> {
    if input.flags & !SUPPORTED_FLAGS != 0 {
        return None;
    }
    // SAFETY: `face` is `NULL` or live by this function's contract.
    let count = unsafe { crate::hr_face_get_glyph_count(face) };
    if count == 0 {
        return None;
    }
    // SAFETY: `font_bytes` needs `face` to be `NULL` or live, which this
    // function requires.
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
    // SAFETY: `ranges` needs `NULL` or a live set. By the invariant on
    // `sets` each of the input's sets is live for as long as the input is,
    // and `input` is borrowed for all of this function.
    let glyphs: IntSet<GlyphId> = unsafe { ranges(input.sets[0], count - 1) }
        .iter_ranges()
        .map(|range| GlyphId::from(*range.start())..=GlyphId::from(*range.end()))
        .fold(IntSet::empty(), |mut set, range| {
            set.insert_range(range);
            set
        });
    // SAFETY: `input.sets[1]` is live by the invariant on `sets`, as above.
    let unicodes = unsafe { ranges(input.sets[1], 0x10_FFFF) };
    // SAFETY: `tags` needs `NULL` or a live set, which `input.sets[3]` is by
    // the invariant on `sets`, as above.
    let drop_tables = unsafe { tags(input.sets[3]) };
    // SAFETY: `input.sets[7]` is live by the invariant on `sets`, as above.
    let scripts = unsafe { tags(input.sets[7]) };
    // SAFETY: `input.sets[6]` is live by the invariant on `sets`, as above.
    let features = unsafe { tags(input.sets[6]) };
    let mut name_ids = IntSet::<NameId>::empty();
    // SAFETY: `input.sets[4]` is live by the invariant on `sets`, as above.
    for range in unsafe { ranges(input.sets[4], u16::MAX.into()) }.iter_ranges() {
        name_ids
            .insert_range(NameId::from(*range.start() as u16)..=NameId::from(*range.end() as u16));
    }
    let mut name_languages = IntSet::<u16>::empty();
    // SAFETY: `input.sets[5]` is live by the invariant on `sets`, as above.
    for range in unsafe { ranges(input.sets[5], u16::MAX.into()) }.iter_ranges() {
        name_languages.insert_range(*range.start() as u16..=*range.end() as u16);
    }
    let mut plan = skera::Plan::new(
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
    // SAFETY: `input.sets[2]` is live by the invariant on `sets`, as above.
    plan.set_no_subset_tables(&unsafe { tags(input.sets[2]) });
    Some((bytes, plan))
}

/// Subsets a face with skera, returning a new face with serialized SFNT data.
///
/// Returns `NULL` for invalid input, unsupported flags or outline formats,
/// missing table enumeration, or skera failure. The original face and input
/// are unchanged.
///
/// A panic in skera also returns `NULL`, where panics unwind, which is Cargo's
/// default. That does not cover reading the result back as a face, nor a
/// library built with `panic=abort`: a panic there aborts the process.
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
    // SAFETY: `input` is `NULL` or live, and not modified during the call,
    // both by this function's contract.
    let Some(input) = (unsafe { input.as_ref() }) else {
        return ptr::null_mut();
    };
    let Ok(Some(bytes)) = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `make_plan` needs `face` to be `NULL` or live, which this
        // function requires.
        let (bytes, plan) = unsafe { make_plan(face, input) }?;
        skera::subset_font(&FontRef::new(&bytes).ok()?, &plan).ok()
    })) else {
        return ptr::null_mut();
    };
    // SAFETY: `face_from_bytes` asks nothing of its caller.
    unsafe { face_from_bytes(&bytes) }
}

/// Returns `NULL`, or a new face over a copy of `bytes`, whose reference the
/// caller owns.
///
/// # Safety
/// Nothing is asked of the caller: `bytes` is a slice, which is all that
/// the calls made here need.
unsafe fn face_from_bytes(bytes: &[u8]) -> *mut hr_face_t {
    let Ok(len) = bytes.len().try_into() else {
        return ptr::null_mut();
    };
    // SAFETY: `hr_blob_create_or_fail` needs `len` readable bytes at the
    // pointer it is given. `bytes` is a slice and `len` its length. Told to
    // duplicate, it copies them before it returns, so they need not outlive
    // the call, and no destroy function is passed for it to call. The blob
    // that comes back, if any, is new and its reference the caller's, as
    // the invariant on `Blob` asks.
    let blob = Blob(unsafe {
        crate::hr_blob_create_or_fail(
            bytes.as_ptr().cast(),
            len,
            crate::HR_MEMORY_MODE_DUPLICATE,
            ptr::null_mut(),
            None,
        )
    });
    // SAFETY: `blob.0` is `NULL` or live by the invariant on `Blob`.
    unsafe { crate::hr_face_create_or_fail(blob.0, 0) }
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
    // SAFETY: `font_bytes` needs `face` to be `NULL` or live, which this
    // function requires.
    let Ok(Some(bytes)) = catch_unwind(AssertUnwindSafe(|| unsafe { font_bytes(face) })) else {
        return ptr::null_mut();
    };
    // SAFETY: `face_from_bytes` asks nothing of its caller.
    let result = unsafe { face_from_bytes(&bytes) };
    // SAFETY: `hr_face_make_immutable` takes `NULL` or a live face. `result`
    // is what `face_from_bytes` returns: `NULL`, or a new face whose
    // reference this function holds until it hands it to its caller.
    unsafe { crate::hr_face_make_immutable(result) };
    result
}

struct Map(
    // Safety invariant: a live map, one of whose references this owns.
    *mut hr_map_t,
);
impl Map {
    fn new() -> Self {
        // The invariant holds: this is a new map, and the reference
        // `hr_map_create` returns to it is this `Map`'s.
        Self(crate::hr_map_create())
    }
}
impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: `hr_map_destroy` needs `NULL` or a live map, and the
        // reference it gives up to be its caller's own. The invariant on
        // `Map` gives both, and a `Map` is dropped once.
        unsafe { crate::hr_map_destroy(self.0) };
    }
}

/// An immutable subset plan owning its font bytes and borrowed output maps.
/// Creating a plan snapshots its input. The input and original face can be
/// released afterwards; execution may be repeated or run concurrently.
pub struct hr_subset_plan_t {
    references: AtomicI64,
    bytes: Vec<u8>,
    plan: skera::Plan,
    maps: [Map; 3],
    user_data: user_data::UserData,
}

impl Drop for hr_subset_plan_t {
    fn drop(&mut self) {
        self.user_data.release();
    }
}

/// Creates a self-contained plan and its glyph mappings, or `NULL` on failure.
///
/// # Safety
/// `face` and `input` must be live. Input sets must not be mutated
/// during this call; face callbacks must satisfy the core API contract.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_create_or_fail(
    face: *mut hr_face_t,
    input: *const hr_subset_input_t,
) -> *mut hr_subset_plan_t {
    // SAFETY: `input` is `NULL`, or live by this function's contract. Nothing
    // writes it meanwhile: the setters need exclusive access to an input,
    // and the call that frees it writes it only when nothing else uses it.
    let Some(input) = (unsafe { input.as_ref() }) else {
        return ptr::null_mut();
    };
    // SAFETY: `make_plan` needs `face` to be `NULL` or live. It is `NULL`, or
    // live by this function's contract.
    let Ok(Some((bytes, plan))) =
        catch_unwind(AssertUnwindSafe(|| unsafe { make_plan(face, input) }))
    else {
        return ptr::null_mut();
    };
    let maps = std::array::from_fn(|_| Map::new());
    for (old, new) in plan.old_to_new_glyph_mapping() {
        // SAFETY: both calls take a map that must be live, and modify it,
        // which takes exclusive access to it. `maps[0]` and `maps[1]` are
        // live by the invariant on `Map`, and nothing else has been given
        // the pointers to maps made just above.
        unsafe {
            crate::hr_map_set(maps[0].0, old.to_u32(), new.to_u32());
            crate::hr_map_set(maps[1].0, new.to_u32(), old.to_u32());
        }
    }
    for (unicode, old) in plan.unicode_to_old_glyph_mapping() {
        // SAFETY: `maps[2]` is live by the invariant on `Map`. Modifying it
        // takes exclusive access, and nothing else has been given the
        // pointer to a map made just above.
        unsafe { crate::hr_map_set(maps[2].0, unicode, old.to_u32()) };
    }
    Box::into_raw(Box::new(hr_subset_plan_t {
        references: AtomicI64::new(1),
        bytes,
        plan,
        maps,
        user_data: user_data::UserData::default(),
    }))
}

/// Takes a reference to a subset plan. `NULL` stays `NULL`.
///
/// A reference taken from one of the plan's own destroy callbacks, while its
/// last reference is being released, does not keep the plan alive. It must be
/// given back before that callback returns.
///
/// # Safety
/// `plan` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_reference(
    plan: *mut hr_subset_plan_t,
) -> *mut hr_subset_plan_t {
    // SAFETY: `as_ref` needs `plan` to be `NULL`, or a live plan that
    // nothing writes while the reference is used. This function's contract
    // has it `NULL` or live, and a plan is written only by the call that
    // frees it, when nothing else uses it.
    if let Some(plan) = unsafe { plan.as_ref() } {
        plan.references.fetch_add(1, Ordering::Relaxed);
    }
    plan
}

/// Releases an owned plan reference. Accepts `NULL`.
///
/// # Safety
/// `plan` must be `NULL` or an owned reference.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_destroy(plan: *mut hr_subset_plan_t) {
    // SAFETY: `as_ref` needs `plan` to be `NULL`, or a live plan that
    // nothing writes while `state` is used. This function's contract has it
    // `NULL` or a reference the caller owns, which keeps the plan allocated
    // until the count below gives that reference up. After that `state` is
    // used only by a call that took the count from one to zero, which is
    // the call that frees the plan, once it is done with `state`. Nothing
    // else writes a plan, and what this call writes of it, it writes once
    // it is done with `state`.
    let Some(state) = (unsafe { plan.as_ref() }) else {
        return;
    };
    if state.references.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        state.references.store(DYING, Ordering::Relaxed);
        // Callbacks may query the object. Run them while access is shared,
        // before reconstructing the Box and taking exclusive ownership.
        state.user_data.release();
        // The maps go here too, rather than when the plan is dropped. The
        // plan's reference to one may be the last, and releasing that runs
        // the destroy functions of whatever the map was given. Those are the
        // caller's code, and may ask the plan a question, or execute it.
        // Here nothing borrows the plan and it still has its font. Inside
        // its drop, the drop borrows it, and the font goes before the maps.
        //
        // Each map is replaced by the empty one before it is released, so
        // what its release runs finds a plan that has let go of it, and
        // still has the maps that come after.
        for index in 0..3 {
            // SAFETY: this names one of the plan's maps through `plan` and
            // swaps the empty map into it with `replace`.
            // - Naming it needs `plan` to point into an allocation that
            //   holds a plan, and `index` to be one of its three maps.
            //   `replace` needs the entry to be aligned, to hold an
            //   initialized value, and to be valid for reads and writes,
            //   with nothing else using it meanwhile.
            // - `plan` is live, as for `state` above, and is the pointer
            //   that `hr_subset_plan_create_or_fail` got from
            //   `Box::into_raw`, so the entry is in bounds, aligned and
            //   initialized, and may be written through it.
            // - Nothing else is using the plan. The reference released
            //   above was the last, so only destructors this call runs can
            //   be, and those run so far have returned: the ones `release`
            //   ran, and the ones of the maps already released. None can
            //   have written the plan, which only this call does. `state`
            //   is not used again.
            // Afterwards the entry holds the empty map, which is live for
            // good and asks for no reference, as the invariant on `Map`
            // needs, and `map` owns the reference the plan had.
            let map =
                unsafe { (&raw mut (*plan).maps[index]).replace(Map(crate::hr_map_get_empty())) };
            // Releases that reference.
            drop(map);
        }
        // SAFETY: `Box::from_raw` needs the pointer `Box::into_raw` gave for
        // this plan, not yet freed, and nothing else to use the plan
        // from here on.
        // - `plan` is not `NULL`, and every plan the API hands out is
        //   the one `hr_subset_plan_create_or_fail` got from `Box::into_raw`.
        // - The caller's reference kept it allocated up to this call, and
        //   a plan is freed only by the call that takes its count from one
        //   to zero, which happens once: the count is parked at `DYING`
        //   since, and 64 bits cannot be counted round.
        // - That reference was the last. The destructors run above, by
        //   `release` and by letting go of the maps, may still have used
        //   the plan, and have all returned, having given back any
        //   reference they took, as the README requires. `state` is not
        //   used again.
        // - Dropping the plan runs none of the caller's code, which could
        //   still reach it through `plan`: its user data was released above
        //   and takes no more, and its maps are the empty map.
        drop(unsafe { Box::from_raw(plan) });
    }
}

/// Executes a subset plan, returning an independent face or `NULL` on failure.
///
/// # Safety
/// `plan` must be `NULL` or live; its borrowed maps must not be modified.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_execute_or_fail(
    plan: *mut hr_subset_plan_t,
) -> *mut hr_face_t {
    // SAFETY: `plan` is `NULL` or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    let Some(plan) = (unsafe { plan.as_ref() }) else {
        return ptr::null_mut();
    };
    let Ok(Some(bytes)) = catch_unwind(AssertUnwindSafe(|| {
        skera::subset_font(&FontRef::new(&plan.bytes).ok()?, &plan.plan).ok()
    })) else {
        return ptr::null_mut();
    };
    // SAFETY: `face_from_bytes` asks nothing of its caller.
    unsafe { face_from_bytes(&bytes) }
}

/// Returns the borrowed original-to-subset map, including closure glyphs.
/// Take a core map reference if it must outlive the plan.
///
/// # Safety
/// `plan` must be live. The returned map is read-only.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_old_to_new_glyph_mapping(
    plan: *const hr_subset_plan_t,
) -> *mut hr_map_t {
    // SAFETY: `plan` is `NULL`, or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    unsafe { plan.as_ref() }.map_or(ptr::null_mut(), |plan| plan.maps[0].0)
}

/// Returns the borrowed subset-to-original map, including closure glyphs.
///
/// # Safety
/// `plan` must be live. The returned map is read-only.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_new_to_old_glyph_mapping(
    plan: *const hr_subset_plan_t,
) -> *mut hr_map_t {
    // SAFETY: `plan` is `NULL`, or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    unsafe { plan.as_ref() }.map_or(ptr::null_mut(), |plan| plan.maps[1].0)
}

/// Returns the borrowed Unicode-to-original-glyph map.
///
/// # Safety
/// `plan` must be live. The returned map is read-only.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_unicode_to_old_glyph_mapping(
    plan: *const hr_subset_plan_t,
) -> *mut hr_map_t {
    // SAFETY: `plan` is `NULL`, or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    unsafe { plan.as_ref() }.map_or(ptr::null_mut(), |plan| plan.maps[2].0)
}

/// Attaches owned metadata to a subset input; rejected data stays caller-owned.
/// Replacing/removing metadata invokes its destructor outside the object lock.
///
/// # Safety
/// `input` must be `NULL` or live. The key must outlive the object, and the
/// supplied data/destructor must be safe to release from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_set_user_data(
    input: *mut hr_subset_input_t,
    key: *const crate::hr_user_data_key_t,
    data: *mut c_void,
    destroy: crate::hr_destroy_func_t,
    replace: c_int,
) -> c_int {
    // SAFETY: `input` is `NULL` or live by this function's contract. Nothing
    // writes it meanwhile, not even a destructor this call runs: the
    // setters need exclusive access to an input, and the call that frees
    // it writes it only when nothing else uses it.
    //
    // `set` builds an `Item` from `data` and `destroy`, which this
    // function's contract has safe to release from any thread, as the
    // invariant on `Item::destroy` asks.
    unsafe { input.as_ref() }
        .is_some_and(|input| input.user_data.set(key, data, destroy, replace != 0))
        .into()
}

/// Retrieves attached metadata, or `NULL` when absent.
///
/// # Safety
/// `input` must be `NULL` or live. Concurrent replacement may invalidate the returned pointer.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_input_get_user_data(
    input: *const hr_subset_input_t,
    key: *const crate::hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `input` is `NULL` or live by this function's contract. Nothing
    // writes it meanwhile: the setters need exclusive access to an input,
    // and the call that frees it writes it only when nothing else uses it.
    unsafe { input.as_ref() }.map_or(ptr::null_mut(), |input| input.user_data.get(key))
}

/// Attaches owned metadata to a subset plan; rejected data stays caller-owned.
/// Replacing/removing metadata invokes its destructor outside the object lock.
///
/// # Safety
/// `plan` must be `NULL` or live. The key must outlive the object, and the
/// supplied data/destructor must be safe to release from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_set_user_data(
    plan: *mut hr_subset_plan_t,
    key: *const crate::hr_user_data_key_t,
    data: *mut c_void,
    destroy: crate::hr_destroy_func_t,
    replace: c_int,
) -> c_int {
    // SAFETY: `plan` is `NULL` or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    //
    // `set` builds an `Item` from `data` and `destroy`, which this
    // function's contract has safe to release from any thread, as the
    // invariant on `Item::destroy` asks.
    unsafe { plan.as_ref() }
        .is_some_and(|plan| plan.user_data.set(key, data, destroy, replace != 0))
        .into()
}

/// Retrieves attached metadata, or `NULL` when absent.
///
/// # Safety
/// `plan` must be `NULL` or live. Concurrent replacement may invalidate the returned pointer.
#[no_mangle]
pub unsafe extern "C" fn hr_subset_plan_get_user_data(
    plan: *const hr_subset_plan_t,
    key: *const crate::hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `plan` is `NULL` or live by this function's contract, and a
    // plan is written only by the call that frees it, when nothing else
    // uses it.
    unsafe { plan.as_ref() }.map_or(ptr::null_mut(), |plan| plan.user_data.get(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    #[derive(Clone, Copy)]
    enum Owner {
        Input(*mut hr_subset_input_t),
        Plan(*mut hr_subset_plan_t),
    }

    struct Destruction {
        owner: Owner,
        key: crate::hr_user_data_key_t,
        calls: Cell<u32>,
        resource_released: Cell<bool>,
        observed_released: Cell<bool>,
        observed_empty: Cell<bool>,
        accepted: Cell<c_int>,
    }

    unsafe extern "C" fn release_resource(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state.resource_released.set(true);
    }

    unsafe extern "C" fn release_metadata(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state.calls.set(state.calls.get() + 1);
        state.observed_released.set(state.resource_released.get());
        // Replacing this key during Vec::drop used to drop the same item a
        // second time. Keep the reproducer bounded on that implementation.
        if state.calls.get() != 1 {
            return;
        }
        let (current, accepted) = unsafe {
            match state.owner {
                Owner::Input(input) => (
                    hr_subset_input_get_user_data(input, &raw const state.key),
                    hr_subset_input_set_user_data(input, &raw const state.key, data, None, 1),
                ),
                Owner::Plan(plan) => (
                    hr_subset_plan_get_user_data(plan, &raw const state.key),
                    hr_subset_plan_set_user_data(plan, &raw const state.key, data, None, 1),
                ),
            }
        };
        state.observed_empty.set(current.is_null());
        state.accepted.set(accepted);
    }

    fn check_destruction(owner: Owner) {
        let mut state = Destruction {
            owner,
            key: crate::hr_user_data_key_t { unused: 0 },
            calls: Cell::new(0),
            resource_released: Cell::new(false),
            observed_released: Cell::new(true),
            observed_empty: Cell::new(false),
            accepted: Cell::new(-1),
        };
        let data = (&raw mut state).cast();
        unsafe {
            match owner {
                Owner::Input(input) => {
                    let set = hr_subset_input_glyph_set(input);
                    assert_eq!(
                        crate::hr_set_set_user_data(
                            set,
                            &raw const state.key,
                            data,
                            Some(release_resource),
                            1
                        ),
                        1
                    );
                    assert_eq!(
                        hr_subset_input_set_user_data(
                            input,
                            &raw const state.key,
                            data,
                            Some(release_metadata),
                            1
                        ),
                        1
                    );
                    hr_subset_input_destroy(input);
                }
                Owner::Plan(plan) => {
                    let map = hr_subset_plan_old_to_new_glyph_mapping(plan);
                    assert_eq!(
                        crate::hr_map_set_user_data(
                            map,
                            &raw const state.key,
                            data,
                            Some(release_resource),
                            1
                        ),
                        1
                    );
                    assert_eq!(
                        hr_subset_plan_set_user_data(
                            plan,
                            &raw const state.key,
                            data,
                            Some(release_metadata),
                            1
                        ),
                        1
                    );
                    hr_subset_plan_destroy(plan);
                }
            }
        }
        assert_eq!(state.calls.get(), 1);
        assert!(!state.observed_released.get());
        assert!(state.resource_released.get());
        assert!(state.observed_empty.get());
        assert_eq!(state.accepted.get(), 0);
    }

    #[test]
    fn an_input_destroy_callback_may_take_and_release_a_reference() {
        // The destructor runs while the input's last reference is being
        // released. Taking a reference there and giving it back, as a scoped
        // handle would, must not release the input a second time.
        unsafe extern "C" fn borrow_briefly(data: *mut c_void) {
            let input = data.cast::<hr_subset_input_t>();
            unsafe {
                assert_eq!(hr_subset_input_reference(input), input);
                assert_eq!(hr_subset_input_get_flags(input), 0);
                hr_subset_input_destroy(input);
            }
        }
        static KEY: crate::hr_user_data_key_t = crate::hr_user_data_key_t { unused: 0 };
        let input = hr_subset_input_create_or_fail();
        unsafe {
            assert_ne!(
                hr_subset_input_set_user_data(
                    input,
                    &raw const KEY,
                    input.cast(),
                    Some(borrow_briefly),
                    1
                ),
                0
            );
            hr_subset_input_destroy(input);
        }
    }

    #[test]
    fn input_metadata_is_detached_before_callbacks_and_sets_outlive_it() {
        check_destruction(Owner::Input(hr_subset_input_create_or_fail()));
    }

    #[test]
    fn plan_metadata_is_detached_before_callbacks_and_maps_outlive_it() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../harfrust/tests/fonts/rb_custom/LaBelleAurore.ttf"
        ));
        unsafe {
            let blob = crate::hr_blob_create(
                bytes.as_ptr().cast(),
                bytes.len() as c_uint,
                crate::HR_MEMORY_MODE_DUPLICATE,
                ptr::null_mut(),
                None,
            );
            let face = crate::hr_face_create(blob, 0);
            let input = hr_subset_input_create_or_fail();
            let plan = hr_subset_plan_create_or_fail(face, input);
            assert!(!plan.is_null());
            hr_subset_input_destroy(input);
            crate::hr_face_destroy(face);
            crate::hr_blob_destroy(blob);
            check_destruction(Owner::Plan(plan));
        }
    }

    static QUERY_KEY: crate::hr_user_data_key_t = crate::hr_user_data_key_t { unused: 0 };

    /// What `query_input` is handed, and what the input told it.
    struct InputQuery {
        input: *mut hr_subset_input_t,
        calls: Cell<u32>,
        flags: Cell<hr_subset_flags_t>,
        user_data: Cell<*mut c_void>,
        glyph_set: Cell<*mut hr_set_t>,
        unicode_set: Cell<*mut hr_set_t>,
    }

    unsafe extern "C" fn query_input(data: *mut c_void) {
        let state = unsafe { &*data.cast::<InputQuery>() };
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state.flags.set(hr_subset_input_get_flags(state.input));
            state.user_data.set(hr_subset_input_get_user_data(
                state.input,
                &raw const QUERY_KEY,
            ));
            state.glyph_set.set(hr_subset_input_glyph_set(state.input));
            state
                .unicode_set
                .set(hr_subset_input_unicode_set(state.input));
        }
    }

    #[test]
    fn a_sets_destroy_may_query_the_input_releasing_it() {
        let state = InputQuery {
            input: hr_subset_input_create_or_fail(),
            calls: Cell::new(0),
            flags: Cell::new(0),
            user_data: Cell::new(ptr::dangling_mut()),
            glyph_set: Cell::new(ptr::null_mut()),
            unicode_set: Cell::new(ptr::null_mut()),
        };
        unsafe {
            let input = state.input;
            let unicodes = hr_subset_input_unicode_set(input);
            hr_subset_input_set_flags(input, HR_SUBSET_FLAGS_GLYPH_NAMES);
            assert_ne!(
                hr_subset_input_set_user_data(
                    input,
                    &raw const QUERY_KEY,
                    ptr::dangling_mut(),
                    None,
                    1
                ),
                0
            );
            // Released along with the glyph set, which only the input holds
            // a reference to.
            assert_ne!(
                crate::hr_set_set_user_data(
                    hr_subset_input_glyph_set(input),
                    &raw const QUERY_KEY,
                    ptr::from_ref(&state).cast_mut().cast(),
                    Some(query_input),
                    1
                ),
                0
            );
            assert_eq!(state.calls.get(), 0);
            hr_subset_input_destroy(input);
            // The input still had the sets that come after the glyph set.
            assert_eq!(state.unicode_set.get(), unicodes);
        }
        assert_eq!(state.calls.get(), 1);
        assert_eq!(state.flags.get(), HR_SUBSET_FLAGS_GLYPH_NAMES);
        assert!(state.user_data.get().is_null());
        // The set goes from an input that has let go of it.
        assert_eq!(state.glyph_set.get(), crate::hr_set_get_empty());
    }

    /// What `execute_plan` is handed, and what the plan gave it.
    struct PlanQuery {
        plan: Cell<*mut hr_subset_plan_t>,
        calls: Cell<u32>,
        user_data: Cell<*mut c_void>,
        executed: Cell<bool>,
        old_to_new: Cell<*mut hr_map_t>,
        new_to_old: Cell<*mut hr_map_t>,
    }

    unsafe extern "C" fn execute_plan(data: *mut c_void) {
        let state = unsafe { &*data.cast::<PlanQuery>() };
        let plan = state.plan.get();
        state.calls.set(state.calls.get() + 1);
        unsafe {
            state
                .user_data
                .set(hr_subset_plan_get_user_data(plan, &raw const QUERY_KEY));
            let face = hr_subset_plan_execute_or_fail(plan);
            state.executed.set(!face.is_null());
            crate::hr_face_destroy(face);
            state
                .old_to_new
                .set(hr_subset_plan_old_to_new_glyph_mapping(plan));
            state
                .new_to_old
                .set(hr_subset_plan_new_to_old_glyph_mapping(plan));
        }
    }

    #[test]
    fn a_maps_destroy_may_execute_the_plan_releasing_it() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../harfrust/tests/fonts/rb_custom/LaBelleAurore.ttf"
        ));
        let state = PlanQuery {
            plan: Cell::new(ptr::null_mut()),
            calls: Cell::new(0),
            user_data: Cell::new(ptr::dangling_mut()),
            executed: Cell::new(false),
            old_to_new: Cell::new(ptr::null_mut()),
            new_to_old: Cell::new(ptr::null_mut()),
        };
        unsafe {
            let blob = crate::hr_blob_create(
                bytes.as_ptr().cast(),
                bytes.len() as c_uint,
                crate::HR_MEMORY_MODE_DUPLICATE,
                ptr::null_mut(),
                None,
            );
            let face = crate::hr_face_create(blob, 0);
            let input = hr_subset_input_create_or_fail();
            let plan = hr_subset_plan_create_or_fail(face, input);
            assert!(!plan.is_null());
            hr_subset_input_destroy(input);
            crate::hr_face_destroy(face);
            crate::hr_blob_destroy(blob);
            state.plan.set(plan);
            let new_to_old = hr_subset_plan_new_to_old_glyph_mapping(plan);
            assert_ne!(
                hr_subset_plan_set_user_data(
                    plan,
                    &raw const QUERY_KEY,
                    ptr::dangling_mut(),
                    None,
                    1
                ),
                0
            );
            // Released along with the first map, which only the plan holds
            // a reference to.
            assert_ne!(
                crate::hr_map_set_user_data(
                    hr_subset_plan_old_to_new_glyph_mapping(plan),
                    &raw const QUERY_KEY,
                    ptr::from_ref(&state).cast_mut().cast(),
                    Some(execute_plan),
                    1
                ),
                0
            );
            assert_eq!(state.calls.get(), 0);
            hr_subset_plan_destroy(plan);
            // The plan still had the maps that come after the first.
            assert_eq!(state.new_to_old.get(), new_to_old);
        }
        assert_eq!(state.calls.get(), 1);
        assert!(state.user_data.get().is_null());
        // The map goes from a plan that has let go of it, and that still
        // has the font it subsets.
        assert_eq!(state.old_to_new.get(), crate::hr_map_get_empty());
        assert!(state.executed.get());
    }
}
