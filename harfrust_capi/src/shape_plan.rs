//! Reusable shape plans and the segment properties that identify them.
//! Mirrors HarfBuzz's `hb-shape-plan.h`.
//!
//! A plan captures everything that can be worked out from a face, a set of
//! segment properties and a list of features, before any text is seen. Building
//! one is the expensive half of shaping, so a plan built once can be executed
//! over many buffers that share those properties.
//!
//! [`hr_shape`](crate::hr_shape) already reuses plans through a per-face cache,
//! so reach for this only when you want to hold a plan yourself.

use core::ffi::{c_char, c_uint, c_void};
use std::sync::{Arc, OnceLock};

use harfrust::font::{Font, NormalizedCoord};
use harfrust::{Direction, Language, Script, ShapePlan};

use crate::buffer::hr_buffer_t;
use crate::common::{
    direction_from_rust, direction_to_rust, hr_bool_t, hr_direction_t, hr_feature_t, hr_language_t,
    hr_script_t, language_from_rust, language_to_rust, script_from_rust, script_to_rust, write_out,
    HR_DIRECTION_INVALID,
};
use crate::face::hr_face_t;
use crate::font::hr_font_t;
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};
use crate::shape::{collect_features, shaper_list_allows_ot};

/// The direction, script and language a run of text is set in.
///
/// Zeroing this struct leaves every field unset, which is what
/// `HR_SEGMENT_PROPERTIES_DEFAULT` expands to.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct hr_segment_properties_t {
    /// The direction the text is set in.
    pub direction: hr_direction_t,
    /// The script the text is written in.
    pub script: hr_script_t,
    /// The language the text is in, or `NULL` if unset.
    pub language: hr_language_t,
    /// Reserved; always set to `NULL`.
    pub reserved1: *mut c_void,
    /// Reserved; always set to `NULL`.
    pub reserved2: *mut c_void,
}

impl hr_segment_properties_t {
    /// # Safety
    ///
    /// The `language` field must be `NULL` or an interned language returned by
    /// this API.
    pub(crate) unsafe fn to_rust(self) -> (Direction, Option<Script>, Option<Language>) {
        (
            direction_to_rust(self.direction),
            script_to_rust(self.script),
            // SAFETY: `language_to_rust` takes `NULL` or an interned language
            // returned by this API, which is what the contract above requires
            // the field to be.
            unsafe { language_to_rust(self.language) },
        )
    }
}

/// Returns whether two sets of segment properties are the same.
///
/// # Safety
///
/// Both pointers must be `NULL` or readable.
#[no_mangle]
pub unsafe extern "C" fn hr_segment_properties_equal(
    a: *const hr_segment_properties_t,
    b: *const hr_segment_properties_t,
) -> hr_bool_t {
    // SAFETY: each `as_ref` needs its pointer to be null, or aligned and
    // pointing to initialized properties that nothing writes while the
    // reference is used, which is for the comparison below. This function's
    // contract has `a` and `b` each `NULL` or readable. They may be the same
    // properties, which two shared references may be too.
    let (Some(a), Some(b)) = (unsafe { a.as_ref() }, unsafe { b.as_ref() }) else {
        return core::ptr::eq(a, b).into();
    };
    // Including the reserved fields, which a caller may be using to tell two
    // otherwise identical property sets apart.
    (a.direction == b.direction
        && a.script == b.script
        && a.language == b.language
        && a.reserved1 == b.reserved1
        && a.reserved2 == b.reserved2)
        .into()
}

/// Returns a hash of a set of segment properties.
///
/// # Safety
///
/// `p` must be `NULL` or readable.
#[no_mangle]
pub unsafe extern "C" fn hr_segment_properties_hash(p: *const hr_segment_properties_t) -> c_uint {
    // SAFETY: `as_ref` needs `p` to be null, or aligned and pointing to
    // initialized properties that nothing writes while the reference is used,
    // which is for the reads below. This function's contract has it `NULL` or
    // readable.
    let Some(p) = (unsafe { p.as_ref() }) else {
        return 0;
    };
    let language = p.language as usize as c_uint;
    p.direction
        .wrapping_mul(31)
        .wrapping_add(p.script)
        .wrapping_mul(31)
        .wrapping_add(language)
}

/// Fills in whichever of `p`'s fields are unset from `src`.
///
/// # Safety
///
/// Both pointers must be `NULL` or readable, and `p` must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_segment_properties_overlay(
    p: *mut hr_segment_properties_t,
    src: *const hr_segment_properties_t,
) {
    if p.is_null() || src.is_null() {
        return;
    }
    // Read before the destination is borrowed: nothing stops a caller from
    // passing the same properties as both, which HarfBuzz allows and which
    // holding a shared and an exclusive reference to at once would not be.
    // SAFETY: `read` needs `src` valid for reads, aligned and pointing to
    // initialized properties. It is not null, as checked above, so this
    // function's contract has it readable. The type is `Copy`, so the copy
    // owns nothing the original also owns.
    let src = unsafe { core::ptr::read(src) };
    // SAFETY: making the reference needs `p` non-null, aligned and pointing to
    // initialized properties that nothing else accesses while it is used. `p`
    // is not null, as checked above, so this function's contract has it
    // readable and writable. The caller's `src` may be the same pointer, and
    // it was read through for the last time above.
    let p = unsafe { &mut *p };
    // Each property is filled in only while the two still agree on everything
    // before it: text the source describes differently is not text these
    // properties describe, so the rest of it says nothing about this.
    if p.direction == HR_DIRECTION_INVALID {
        p.direction = src.direction;
    }
    if p.direction != src.direction {
        return;
    }
    if p.script == crate::common::HR_SCRIPT_INVALID {
        p.script = src.script;
    }
    if p.script != src.script {
        return;
    }
    if p.language.is_null() {
        p.language = src.language;
    }
}

/// A reusable plan for shaping text with given properties.
pub struct hr_shape_plan_t {
    header: ObjectHeader,
    /// Owned reference to the face the plan was built over.
    // Safety invariant: a live face returned by this API, on which this plan
    // owns one reference.
    face: *mut hr_face_t,
    /// `None` only for the immortal empty plan.
    pub(crate) plan: Option<Arc<ShapePlan>>,
}

impl Drop for hr_shape_plan_t {
    fn drop(&mut self) {
        // SAFETY: `destroy` needs a face that is `NULL` or live, and a
        // reference to it that is the caller's to give up. The invariant on
        // `face` has it live with one reference that is this plan's, and a
        // plan being dropped gives it up once, here.
        unsafe { object::destroy(self.face) };
    }
}

static EMPTY_SHAPE_PLAN: OnceLock<Empty<hr_shape_plan_t>> = OnceLock::new();

impl Object for hr_shape_plan_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    unsafe fn finalize(ptr: *mut Self) {
        // The face goes here rather than when the plan is dropped. The
        // plan's reference to it may be the last, and releasing that runs
        // the destroy functions of everything the face was given. Those are
        // the caller's code, and may ask this plan a question. Here nothing
        // borrows the plan. Inside its drop, the drop does.
        //
        // The plan is left holding the empty face, as the empty plan does,
        // which `Drop` then releases to no effect.
        //
        // SAFETY: this names the `face` field through `ptr` and swaps the
        // empty face into it with `replace`.
        // - Naming the field needs `ptr` to point into an allocation that
        //   holds a `Self`. `replace` needs the field to be aligned, to hold
        //   an initialized value, and to be valid for reads and writes, with
        //   nothing else using it meanwhile.
        // - This function's contract has `ptr` live, which makes the field
        //   in bounds, aligned and initialized, and has nothing else using
        //   the plan. None of the caller's code runs before `replace` has
        //   returned, so no reference into the plan is in use while the
        //   field is written.
        // Afterwards `face` is the plan's reference to its face, and the
        // field no longer lays claim to it.
        let face = unsafe { (&raw mut (*ptr).face).replace(hr_face_t::empty()) };
        // SAFETY: `destroy` needs a null or live face, and the reference
        // being released to be the caller's own. `face` came out of the
        // field that holds the plan's owned reference to its face, which
        // kept the face live until now, and the field has given that
        // reference up to this function. Nothing has released it since.
        unsafe { object::destroy(face) };
    }

    fn empty() -> *mut Self {
        EMPTY_SHAPE_PLAN
            .get_or_init(|| {
                Empty::new(hr_shape_plan_t {
                    header: ObjectHeader::immortal(),
                    // Upholds the invariant on `face`: the empty face is
                    // immortal, so it is always live, and taking or giving
                    // up a reference to it does nothing.
                    face: hr_face_t::empty(),
                    plan: None,
                })
            })
            .get()
    }
}

/// Reads `num_coords` normalized coordinates, or none if `coords` is `NULL`.
///
/// # Safety
///
/// `coords` must be `NULL` or point to `num_coords` readable entries.
unsafe fn collect_coords(coords: *const i32, num_coords: c_uint) -> Vec<NormalizedCoord> {
    if coords.is_null() || num_coords == 0 {
        return Vec::new();
    }
    // SAFETY: `from_raw_parts` needs `coords` non-null and aligned, with
    // `num_coords` initialized entries readable there while the slice is
    // used, which is for the copy made here. `coords` is not null, as
    // checked above, so this function's contract has it point to
    // `num_coords` readable entries.
    unsafe { core::slice::from_raw_parts(coords, num_coords as usize) }
        .iter()
        .map(|coord| NormalizedCoord::from_bits(*coord as i16))
        .collect()
}

/// Builds the font instance a plan is compiled against.
fn instance_for(face: &hr_face_t, coords: &[NormalizedCoord], cached: bool) -> Option<Font> {
    let font = face.font()?;
    // Empty coordinates select the default instance. Nondefault cached plans
    // also use it when the layout cannot vary with the location.
    if cached && (coords.is_empty() || !face.plans.has_layout_variations(font)) {
        return Some(font.default_instance());
    }
    Some(
        font.instance_builder()
            .normalized_coords(coords.iter().copied())
            .build(),
    )
}

/// # Safety
///
/// As [`hr_shape_plan_create`] for the pointers this takes. The coordinates
/// arrive as a slice, already read.
unsafe fn create_plan(
    face: *mut hr_face_t,
    props: *const hr_segment_properties_t,
    user_features: *const hr_feature_t,
    num_user_features: c_uint,
    coords: &[NormalizedCoord],
    shaper_list: *const *const c_char,
    cached: bool,
) -> *mut hr_shape_plan_t {
    // SAFETY: `shaper_list_allows_ot` needs `shaper_list` to be `NULL` or a
    // `NULL`-terminated array of NUL-terminated strings. This function's
    // contract is `hr_shape_plan_create`'s for it, which requires that.
    if !unsafe { shaper_list_allows_ot(shaper_list) } {
        return hr_shape_plan_t::empty();
    }
    // SAFETY: each `as_ref` needs its pointer to be null, or aligned and
    // pointing to a live value that nothing writes while the reference is
    // used, which is no longer than this call. This function's contract is
    // `hr_shape_plan_create`'s for both. It has `face` `NULL` or a live face,
    // and the README ("Threading") keeps a face that is being written from
    // being shared, and a callback from modifying the face it is handed. It
    // has `props` `NULL` or readable.
    let (Some(face_ref), Some(props)) = (unsafe { face.as_ref() }, unsafe { props.as_ref() })
    else {
        return hr_shape_plan_t::empty();
    };
    // SAFETY: `to_rust` needs the language in `props` to be `NULL` or an
    // interned language returned by this API. This function's contract is
    // `hr_shape_plan_create2`'s, and so `hr_shape_plan_create`'s, which
    // requires exactly that.
    let (direction, script, language) = unsafe { props.to_rust() };
    // Compiling a plan requires a direction; HarfBuzz's own planner assumes one
    // too, so refuse rather than guess on the caller's behalf.
    if direction == Direction::Invalid {
        return hr_shape_plan_t::empty();
    }

    let Some(instance) = instance_for(face_ref, coords, cached) else {
        return hr_shape_plan_t::empty();
    };
    // SAFETY: `collect_features` needs `user_features` to be `NULL` or point
    // to `num_user_features` readable entries. This function's contract is
    // `hr_shape_plan_create`'s for them, which requires that.
    let features = unsafe { collect_features(user_features, num_user_features) };

    let plan = if cached {
        face_ref
            .plans
            .get(&instance, direction, script, language.as_ref(), &features)
            .clone()
    } else {
        Arc::new(ShapePlan::new(
            &instance,
            direction,
            script,
            language.as_ref(),
            &features,
        ))
    };

    object::create(hr_shape_plan_t {
        header: ObjectHeader::new(),
        // SAFETY: `reference` needs `face` to be `NULL` or live. It is not
        // null, `face_ref` having been made from it, and it is live by this
        // function's contract. The reference taken is the new plan's own, as
        // the invariant on its `face` field requires.
        face: unsafe { object::reference(face) },
        plan: Some(plan),
    })
}

/// Builds a plan for shaping text with the given properties over a face.
///
/// Never returns `NULL`; if a plan cannot be built, including when `props`
/// leaves the direction unset, this returns the empty plan.
///
/// # Safety
///
/// `face` must be `NULL` or a live face, `props` must be `NULL` or readable,
/// `user_features` must point to `num_user_features` readable entries, and
/// `shaper_list` must be `NULL` or a `NULL`-terminated array of NUL-terminated
/// strings. The language in `props` must be `NULL` or an interned language
/// returned by this API.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_create(
    face: *mut hr_face_t,
    props: *const hr_segment_properties_t,
    user_features: *const hr_feature_t,
    num_user_features: c_uint,
    shaper_list: *const *const c_char,
) -> *mut hr_shape_plan_t {
    // SAFETY: `create_plan` asks of `face`, `props`, `user_features`,
    // `num_user_features` and `shaper_list` what this function's contract asks
    // of them.
    unsafe {
        create_plan(
            face,
            props,
            user_features,
            num_user_features,
            &[],
            shaper_list,
            false,
        )
    }
}

/// Builds a plan for a particular variation of a variable font.
///
/// Coordinates are 2.14 fixed point values in axis order, as
/// `hr_font_get_var_coords_normalized` reports them. They choose the variation
/// the plan is built for; executing it against a font set to another shapes
/// with the font's, as it does in HarfBuzz.
///
/// # Safety
///
/// As [`hr_shape_plan_create`], and `coords` must be `NULL` or point to
/// `num_coords` readable entries.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn hr_shape_plan_create2(
    face: *mut hr_face_t,
    props: *const hr_segment_properties_t,
    user_features: *const hr_feature_t,
    num_user_features: c_uint,
    coords: *const i32,
    num_coords: c_uint,
    shaper_list: *const *const c_char,
) -> *mut hr_shape_plan_t {
    // SAFETY: `collect_coords` needs `coords` to be `NULL` or point to
    // `num_coords` readable entries, which this function's contract requires.
    // `create_plan` asks of the other arguments what `hr_shape_plan_create`
    // asks of them, which this function's contract requires too.
    unsafe {
        create_plan(
            face,
            props,
            user_features,
            num_user_features,
            &collect_coords(coords, num_coords),
            shaper_list,
            false,
        )
    }
}

/// Builds a plan, reusing one from the face's cache when an equal plan is
/// already there.
///
/// This is the cache [`hr_shape`](crate::hr_shape) uses, so a plan taken from
/// here costs nothing beyond the lookup once it has been built.
///
/// # Safety
///
/// See [`hr_shape_plan_create`].
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_create_cached(
    face: *mut hr_face_t,
    props: *const hr_segment_properties_t,
    user_features: *const hr_feature_t,
    num_user_features: c_uint,
    shaper_list: *const *const c_char,
) -> *mut hr_shape_plan_t {
    // SAFETY: `create_plan` asks of `face`, `props`, `user_features`,
    // `num_user_features` and `shaper_list` what `hr_shape_plan_create` asks
    // of them, and this function's contract is that one's.
    unsafe {
        create_plan(
            face,
            props,
            user_features,
            num_user_features,
            &[],
            shaper_list,
            true,
        )
    }
}

/// Builds a plan for a particular variation, reusing one from the face's cache
/// when an equal plan is already there.
///
/// # Safety
///
/// See [`hr_shape_plan_create2`].
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn hr_shape_plan_create_cached2(
    face: *mut hr_face_t,
    props: *const hr_segment_properties_t,
    user_features: *const hr_feature_t,
    num_user_features: c_uint,
    coords: *const i32,
    num_coords: c_uint,
    shaper_list: *const *const c_char,
) -> *mut hr_shape_plan_t {
    // SAFETY: this function's contract is `hr_shape_plan_create2`'s. That has
    // `coords` `NULL` or pointing to `num_coords` readable entries, which is
    // what `collect_coords` needs, and asks of the other arguments what
    // `hr_shape_plan_create` asks of them, which is what `create_plan` needs.
    unsafe {
        create_plan(
            face,
            props,
            user_features,
            num_user_features,
            &collect_coords(coords, num_coords),
            shaper_list,
            true,
        )
    }
}

/// Returns the immortal empty plan, which shapes nothing.
#[no_mangle]
pub extern "C" fn hr_shape_plan_get_empty() -> *mut hr_shape_plan_t {
    hr_shape_plan_t::empty()
}

/// Increments a plan's reference count.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_reference(
    shape_plan: *mut hr_shape_plan_t,
) -> *mut hr_shape_plan_t {
    // SAFETY: `shape_plan` is `NULL` or a live plan by this function's
    // contract, which is what `reference` needs.
    unsafe { object::reference(shape_plan) }
}

/// Decrements a plan's reference count, freeing it at zero.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan, and the caller must own the
/// reference being released.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_destroy(shape_plan: *mut hr_shape_plan_t) {
    // SAFETY: `destroy` needs `shape_plan` to be `NULL` or live, and the
    // reference given up to be the caller's own. This function's contract
    // requires both.
    unsafe { object::destroy(shape_plan) };
}

/// Attaches user data to a plan.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_set_user_data(
    shape_plan: *mut hr_shape_plan_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    // SAFETY: `set_user_data` needs `shape_plan` to be `NULL` or live and
    // `key` to outlive it, which this function's contract requires. `destroy`
    // is kept to be called with `data` later, and the README ("Threading") has
    // the caller make that safe from any thread that uses the plan.
    unsafe { object::set_user_data(shape_plan, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data previously attached to a plan.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_get_user_data(
    shape_plan: *mut hr_shape_plan_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `shape_plan` is `NULL` or a live plan by this function's
    // contract, which is what `get_user_data` needs. `key` is compared by
    // address and never read.
    unsafe { object::get_user_data(shape_plan, key) }
}

/// Returns the name of the shaper a plan will use, or `NULL` for the empty
/// plan.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_get_shaper(
    shape_plan: *mut hr_shape_plan_t,
) -> *const c_char {
    // SAFETY: `shape_plan` is `NULL` or a live plan by this function's
    // contract.
    let plan = unsafe { object::or_empty(shape_plan.cast_const()) };
    if plan.plan.is_none() {
        return core::ptr::null();
    }
    c"ot".as_ptr()
}

/// Writes the properties a plan was built for into `props`.
///
/// # Safety
///
/// `shape_plan` must be `NULL` or a live plan, and `props` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_get_segment_properties(
    shape_plan: *mut hr_shape_plan_t,
    props: *mut hr_segment_properties_t,
) {
    if props.is_null() {
        return;
    }
    // SAFETY: `shape_plan` is `NULL` or a live plan by this function's
    // contract.
    let plan = unsafe { object::or_empty(shape_plan.cast_const()) };
    let Some(inner) = plan.plan.as_ref() else {
        let unset = hr_segment_properties_t {
            direction: HR_DIRECTION_INVALID,
            script: crate::common::HR_SCRIPT_INVALID,
            language: core::ptr::null(),
            reserved1: core::ptr::null_mut(),
            reserved2: core::ptr::null_mut(),
        };
        // SAFETY: `props` is `NULL` or writable by this function's contract.
        unsafe { write_out(props, unset) };
        return;
    };
    let value = hr_segment_properties_t {
        direction: direction_from_rust(inner.direction()),
        script: inner
            .script()
            .map_or(crate::common::HR_SCRIPT_INVALID, script_from_rust),
        language: language_from_rust(inner.language()),
        reserved1: core::ptr::null_mut(),
        reserved2: core::ptr::null_mut(),
    };
    // SAFETY: `props` is `NULL` or writable by this function's contract.
    unsafe { write_out(props, value) };
}

/// Shapes a buffer using a plan that was built beforehand.
///
/// Returns false without shaping if the plan is the empty one, if `font` or
/// `buffer` is `NULL`, or if the buffer's last reference is being released.
/// An empty buffer returns true, having nothing to do.
/// If the shaper panics, this returns false and clears the buffer, as
/// `hr_shape_full` does, and like it only where panics unwind.
///
/// # Aborts
///
/// Using a plan that does not apply aborts the process, as HarfBuzz's
/// assertions do: the plan must have been built over the same face, and for
/// the direction, script and language the buffer carries. These are
/// programming errors rather than conditions to recover from, and shaping
/// through a mismatched plan yields a wrong answer rather than a slow one.
/// Use `hr_shape` if you would rather have a plan chosen for you.
///
/// Variation settings are not among them: HarfBuzz does not compare those,
/// and shapes with the font's whatever the plan was built for.
///
/// # Safety
///
/// `shape_plan`, `font` and `buffer` must be `NULL` or live, and `features`
/// must point to `num_features` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_shape_plan_execute(
    shape_plan: *mut hr_shape_plan_t,
    font: *mut hr_font_t,
    buffer: *mut hr_buffer_t,
    features: *const hr_feature_t,
    num_features: c_uint,
) -> hr_bool_t {
    // Nothing to shape is nothing to fail at, whatever plan was handed over
    // and whatever it was built for: HarfBuzz answers an empty buffer before
    // it looks at anything else, the empty plan included.
    //
    // SAFETY: `as_ref` needs `buffer` to be null, or aligned and pointing to a
    // live `hr_buffer_t` that nothing writes while the reference is used. This
    // function's contract has it `NULL` or live, and a buffer that is being
    // changed belongs to the one thread changing it (README, "Threading"). The
    // reference is used for this one read and not kept.
    if unsafe { buffer.as_ref() }.is_some_and(|buffer| buffer.buffer.is_empty()) {
        return true.into();
    }
    // SAFETY: each `as_ref` needs its pointer to be null, or aligned and
    // pointing to a live object that nothing writes while the reference is
    // used. This function's contract has `shape_plan` and `font` each `NULL`
    // or live. A plan's fields are set when it is made and not written again,
    // and `plan_ref` is not used past the share of the plan taken below,
    // before any callback could release it. A font being shaped with must not
    // be modified or freed, by its callbacks or from another thread (README,
    // "Threading").
    let (Some(plan_ref), Some(font_ref)) =
        (unsafe { shape_plan.as_ref() }, unsafe { font.as_ref() })
    else {
        return false.into();
    };
    let Some(plan) = plan_ref.plan.as_ref() else {
        return false.into();
    };
    // The plan is compiled against one face's tables ...
    assert!(
        font_ref.face() == plan_ref.face,
        "shape plan was built for a different face than this font"
    );
    // The variation the plan was built for is not checked against the
    // font's, as HarfBuzz does not check it either: the font's settings are
    // the ones shaping uses, and a plan built for another variation of the
    // same face still applies to it.
    if font_ref.instance().is_none() {
        return false.into();
    }
    // Take a share of the plan, so that a callback destroying it during
    // shaping cannot pull it out from under the call.
    let plan = Arc::clone(plan);
    // SAFETY: `as_mutable` needs `buffer` to be null or live, which the
    // caller gives, and the reference it returns not to be aliased. It
    // refuses a buffer whose last reference is being released while its
    // user data destructors run, and the immortal empty buffer, which the
    // test at the top has already answered for. Any other buffer is this
    // caller's alone for the call, and the shared reference taken for that
    // test is gone.
    let Some(buffer_ref) = (unsafe { object::as_mutable(buffer) }) else {
        return false.into();
    };
    // An empty buffer needs no work, and matches nothing either way.
    if buffer_ref.buffer.is_empty() {
        return true.into();
    }
    assert!(
        plan_matches_buffer(&plan, &buffer_ref.buffer),
        "shape plan properties do not match the buffer: \
         plan is {:?}/{:?}/{:?}, buffer is {:?}/{:?}/{:?}",
        plan.direction(),
        plan.script(),
        plan.language(),
        buffer_ref.buffer.direction(),
        buffer_ref.buffer.script(),
        buffer_ref.buffer.language(),
    );

    // SAFETY: `collect_features` needs `features` to be `NULL` or point to
    // `num_features` readable entries, which this function's contract
    // requires.
    let features = unsafe { collect_features(features, num_features) };
    // SAFETY: `shape_with_plan` needs `font` to be `NULL` or a live font that
    // stays live and unmodified until it returns. This function's contract
    // has it `NULL` or live, and nothing above releases or writes to it. A
    // font being shaped with must not be modified or freed, by its callbacks
    // or from another thread, so it stays as it is until shaping is done.
    unsafe { crate::shape::shape_with_plan(font, buffer_ref, &features, &plan) }
}

/// Returns whether a buffer carries the properties a plan was built for.
///
/// HarfRust compares a plan's script against the buffer's with an unset script
/// standing in for `Zzzz`, so this normalizes the same way.
fn plan_matches_buffer(plan: &ShapePlan, buffer: &harfrust::Buffer) -> bool {
    let unset = Script::UNKNOWN;
    plan.direction() == buffer.direction()
        && plan.script().unwrap_or(unset) == buffer.script().unwrap_or(unset)
        && plan.language() == buffer.language()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_instance_skips_coords_only_without_layout_variations() {
        let cases: &[(&[u8], i16, bool)] = &[
            (
                include_bytes!("../../harfrust/tests/fonts/in-house/NotoSans-VF.abc.ttf"),
                8192,
                false,
            ),
            (
                include_bytes!(
                    "../../harfrust/tests/fonts/text-rendering-tests/AdobeVFPrototype-Subset.otf"
                ),
                -8192,
                true,
            ),
            (
                include_bytes!("../../harfrust/tests/fonts/default-gpos-variations.ttf"),
                8192,
                true,
            ),
        ];
        for &(bytes, coordinate, has_variations) in cases {
            unsafe {
                let blob = crate::hr_blob_create(
                    bytes.as_ptr().cast(),
                    bytes.len() as u32,
                    crate::HR_MEMORY_MODE_READONLY,
                    core::ptr::null_mut(),
                    None,
                );
                let face = crate::hr_face_create(blob, 0);
                let coords = [NormalizedCoord::from_bits(coordinate)];
                let varied = instance_for(&*face, &coords, false).unwrap();
                // A present table may select no record at this location.
                let selected = varied.feature_variations();
                assert!(selected.gsub.is_none() && selected.gpos.is_none());
                assert_eq!((*face).plans.has_layout_variations(&varied), has_variations);
                assert_eq!(varied.normalized_coords()[0], coords[0]);
                let cached = instance_for(&*face, &coords, true).unwrap();
                assert_eq!(
                    cached.normalized_coords(),
                    if has_variations {
                        varied.normalized_coords()
                    } else {
                        &[]
                    }
                );
                let default = instance_for(&*face, &[], true).unwrap();
                assert_eq!(default.normalized_coords(), []);
                let selection = default.feature_variations();
                assert_eq!(
                    selection.gsub.is_some() || selection.gpos.is_some(),
                    has_variations
                );
                assert_eq!(
                    selection,
                    instance_for(&*face, &[], false)
                        .unwrap()
                        .feature_variations()
                );
                crate::hr_face_destroy(face);
                crate::hr_blob_destroy(blob);
            }
        }
    }
}
