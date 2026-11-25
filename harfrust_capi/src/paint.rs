//! Paint callbacks with HarfBuzz-compatible ownership and dispatch.
// The C drawing/painting ABI uses f32 coordinates, including integer font scales.
#![allow(clippy::cast_precision_loss)]
use crate::color::hr_color_t;
use crate::common::Page;
use crate::draw::hr_draw_funcs_t;
use crate::font_funcs::{set_callback, Callback};
use crate::object::{self, Empty, Object, ObjectHeader};
use crate::{hr_blob_t, hr_glyph_extents_t, hr_tag_t};
use crate::{hr_bool_t, hr_codepoint_t, hr_destroy_func_t, hr_font_t, hr_user_data_key_t};
use core::ffi::{c_uint, c_void};
use std::sync::OnceLock;
/// Callback for push transform; data must be safe to use on any thread.
pub type hr_paint_push_transform_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        xx: f32,
        yx: f32,
        xy: f32,
        yy: f32,
        dx: f32,
        dy: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for pop transform; data must be safe to use on any thread.
pub type hr_paint_pop_transform_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        user_data: *mut c_void,
    ),
>;
/// Callback for color glyph; data must be safe to use on any thread.
///
/// It is offered each color glyph that another color glyph refers to, so that
/// a client that caches rendered glyphs can supply this one from its cache.
/// Returning false leaves the glyph to the library, which paints it as part
/// of the traversal in progress.
///
/// A callback that paints the glyph by calling `hr_font_paint_glyph` starts a
/// new traversal that knows nothing of the one in progress: cycle detection
/// and the limits on depth and size apply to each traversal on its own. Color
/// glyphs can refer to each other, so such a callback must guard against
/// cycles and limit how deeply it nests. Without that, a font can make it
/// recurse until the stack overflows. HarfBuzz leaves this to the callback in
/// the same way.
pub type hr_paint_color_glyph_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        glyph: hr_codepoint_t,
        font: *mut hr_font_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;
/// Callback for push clip glyph; data must be safe to use on any thread.
pub type hr_paint_push_clip_glyph_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        glyph: hr_codepoint_t,
        font: *mut hr_font_t,
        user_data: *mut c_void,
    ),
>;
/// Callback for push clip rectangle; data must be safe to use on any thread.
pub type hr_paint_push_clip_rectangle_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        xmin: f32,
        ymin: f32,
        xmax: f32,
        ymax: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for push clip path start; data must be safe to use on any thread.
pub type hr_paint_push_clip_path_start_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        draw_data: *mut *mut c_void,
        user_data: *mut c_void,
    ) -> *mut hr_draw_funcs_t,
>;
/// Callback for push clip path end; data must be safe to use on any thread.
pub type hr_paint_push_clip_path_end_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        user_data: *mut c_void,
    ),
>;
/// Callback for pop clip; data must be safe to use on any thread.
pub type hr_paint_pop_clip_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        user_data: *mut c_void,
    ),
>;
/// Callback for color; data must be safe to use on any thread.
pub type hr_paint_color_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        is_foreground: hr_bool_t,
        color: hr_color_t,
        user_data: *mut c_void,
    ),
>;
/// Callback for fill glyph; data must be safe to use on any thread.
pub type hr_paint_fill_glyph_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        glyph: hr_codepoint_t,
        font: *mut hr_font_t,
        is_foreground: hr_bool_t,
        color: hr_color_t,
        user_data: *mut c_void,
    ),
>;
/// Callback for image; data must be safe to use on any thread.
pub type hr_paint_image_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        image: *mut hr_blob_t,
        width: c_uint,
        height: c_uint,
        format: hr_tag_t,
        slant: f32,
        extents: *mut hr_glyph_extents_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;
/// Callback for linear gradient; data must be safe to use on any thread.
pub type hr_paint_linear_gradient_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        color_line: *mut hr_color_line_t,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for radial gradient; data must be safe to use on any thread.
pub type hr_paint_radial_gradient_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        color_line: *mut hr_color_line_t,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for sweep gradient; data must be safe to use on any thread.
pub type hr_paint_sweep_gradient_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        color_line: *mut hr_color_line_t,
        x0: f32,
        y0: f32,
        start_angle: f32,
        end_angle: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for push group; data must be safe to use on any thread.
pub type hr_paint_push_group_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        user_data: *mut c_void,
    ),
>;
/// Callback for push group for; data must be safe to use on any thread.
pub type hr_paint_push_group_for_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        mode: hr_paint_composite_mode_t,
        user_data: *mut c_void,
    ),
>;
/// Callback for pop group; data must be safe to use on any thread.
pub type hr_paint_pop_group_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        mode: hr_paint_composite_mode_t,
        user_data: *mut c_void,
    ),
>;
/// Callback for custom palette color; data must be safe to use on any thread.
pub type hr_paint_custom_palette_color_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        color_index: c_uint,
        color: *mut hr_color_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;
/// A reference-counted, optionally immutable collection of paint callbacks.
#[derive(Default)]
pub struct hr_paint_funcs_t {
    header: ObjectHeader,
    // Safety invariant: each callback's `func`, if any, may be called from
    // any thread that uses this object, with the callback's own `user_data`
    // and arguments that meet the contract of the `hr_paint_*` function named
    // after its field. For `push_clip_path_start`, the `draw_data` must also
    // be writable.
    push_transform: Option<Callback<hr_paint_push_transform_func_t>>,
    pop_transform: Option<Callback<hr_paint_pop_transform_func_t>>,
    color_glyph: Option<Callback<hr_paint_color_glyph_func_t>>,
    push_clip_glyph: Option<Callback<hr_paint_push_clip_glyph_func_t>>,
    push_clip_rectangle: Option<Callback<hr_paint_push_clip_rectangle_func_t>>,
    push_clip_path_start: Option<Callback<hr_paint_push_clip_path_start_func_t>>,
    push_clip_path_end: Option<Callback<hr_paint_push_clip_path_end_func_t>>,
    pop_clip: Option<Callback<hr_paint_pop_clip_func_t>>,
    color: Option<Callback<hr_paint_color_func_t>>,
    fill_glyph: Option<Callback<hr_paint_fill_glyph_func_t>>,
    image: Option<Callback<hr_paint_image_func_t>>,
    linear_gradient: Option<Callback<hr_paint_linear_gradient_func_t>>,
    radial_gradient: Option<Callback<hr_paint_radial_gradient_func_t>>,
    sweep_gradient: Option<Callback<hr_paint_sweep_gradient_func_t>>,
    push_group: Option<Callback<hr_paint_push_group_func_t>>,
    push_group_for: Option<Callback<hr_paint_push_group_for_func_t>>,
    pop_group: Option<Callback<hr_paint_pop_group_func_t>>,
    custom_palette_color: Option<Callback<hr_paint_custom_palette_color_func_t>>,
}
// SAFETY: callback data is opaque, and goes only to its callback and its
// destroy function. The README's "Threading" section requires both to be safe
// from any thread that uses the object.
unsafe impl Send for hr_paint_funcs_t {}
// SAFETY: as for `Send`. A shared reference only reads the callbacks, and
// what their data points to is the caller's to make safe from every thread
// that uses the object, under the same section.
unsafe impl Sync for hr_paint_funcs_t {}
impl Object for hr_paint_funcs_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }
    unsafe fn finalize(ptr: *mut Self) {
        // The callbacks go here rather than when the object is dropped. A
        // callback's destroy function is the caller's code, and may ask the
        // funcs object it is being released from a question. Here the object
        // is whole and nothing borrows it. Inside its drop, part of it has
        // gone already and the rest is borrowed by the drop.
        //
        // Each callback is taken out of its field before it is released, so
        // that no destroy function finds its own callback still in place.
        // They go in the order the fields are declared, which is the order
        // dropping the object released them in. Every callback field has to
        // be named here: one left out is released by the drop instead.
        //
        // SAFETY: each line names a field through `ptr`, swaps `None` into
        // it with `replace`, and drops what came out.
        // - Naming a field needs `ptr` to point into an allocation that
        //   holds a `Self`. `replace` needs the field to be aligned, to hold
        //   an initialized value, and to be valid for reads and writes, with
        //   nothing else using it meanwhile.
        // - This function's contract has `ptr` live, which makes each field
        //   in bounds, aligned and initialized, and has nothing else using
        //   the object: no reference into it is in use when a field is
        //   written.
        // - Dropping what came out runs a destroy function, between one
        //   `replace` and the next. It may read the object, which is sound:
        //   `replace` has returned, and the field it emptied holds `None`.
        //   It cannot leave the object unfit for the next line. An object
        //   releasing its last reference takes no changes, which is what
        //   `object::accepts_changes` reports and `set_callback` goes by,
        //   and a reference taken to it then does not free it when it is
        //   given back. Whatever the function borrowed ended when it
        //   returned.
        // Afterwards every callback field holds `None`, and stays that way
        // for the same reason, so dropping the object runs no caller code.
        unsafe {
            drop((&raw mut (*ptr).push_transform).replace(None));
            drop((&raw mut (*ptr).pop_transform).replace(None));
            drop((&raw mut (*ptr).color_glyph).replace(None));
            drop((&raw mut (*ptr).push_clip_glyph).replace(None));
            drop((&raw mut (*ptr).push_clip_rectangle).replace(None));
            drop((&raw mut (*ptr).push_clip_path_start).replace(None));
            drop((&raw mut (*ptr).push_clip_path_end).replace(None));
            drop((&raw mut (*ptr).pop_clip).replace(None));
            drop((&raw mut (*ptr).color).replace(None));
            drop((&raw mut (*ptr).fill_glyph).replace(None));
            drop((&raw mut (*ptr).image).replace(None));
            drop((&raw mut (*ptr).linear_gradient).replace(None));
            drop((&raw mut (*ptr).radial_gradient).replace(None));
            drop((&raw mut (*ptr).sweep_gradient).replace(None));
            drop((&raw mut (*ptr).push_group).replace(None));
            drop((&raw mut (*ptr).push_group_for).replace(None));
            drop((&raw mut (*ptr).pop_group).replace(None));
            drop((&raw mut (*ptr).custom_palette_color).replace(None));
        }
    }
    fn empty() -> *mut Self {
        static EMPTY: OnceLock<Empty<hr_paint_funcs_t>> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                Empty::new(hr_paint_funcs_t {
                    header: ObjectHeader::immortal(),
                    ..Default::default()
                })
            })
            .get()
    }
}
/// Creates an owned callback collection.
#[no_mangle]
pub extern "C" fn hr_paint_funcs_create() -> *mut hr_paint_funcs_t {
    object::create(hr_paint_funcs_t::default())
}
/// Returns the immortal empty collection.
#[no_mangle]
pub extern "C" fn hr_paint_funcs_get_empty() -> *mut hr_paint_funcs_t {
    hr_paint_funcs_t::empty()
}
/// Takes a reference. Accepts NULL.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_reference(
    funcs: *mut hr_paint_funcs_t,
) -> *mut hr_paint_funcs_t {
    // SAFETY: `funcs` is `NULL` or live by this function's contract.
    unsafe { object::reference(funcs) }
}
/// Releases an owned reference and its callback data. Accepts NULL.
///
/// # Safety
///
/// `funcs` must be `NULL` or live, and the caller must own the reference.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_destroy(funcs: *mut hr_paint_funcs_t) {
    // SAFETY: `funcs` is `NULL` or live, and the reference given up is the
    // caller's own, both by this function's contract.
    unsafe { object::destroy(funcs) }
}
/// Rejects subsequent callback changes.
///
/// # Safety
///
/// `funcs` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_make_immutable(funcs: *mut hr_paint_funcs_t) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract.
    unsafe { object::make_immutable(funcs) }
}
/// Reports whether callback changes are forbidden.
///
/// # Safety
///
/// `funcs` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_is_immutable(funcs: *const hr_paint_funcs_t) -> hr_bool_t {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract.
    i32::from(unsafe { object::is_immutable(funcs) })
}
/// Attaches address-keyed metadata, taking ownership only on success.
///
/// # Safety
///
/// `funcs` must be `NULL` or live, and `key` must outlive it. `destroy`, if
/// given, must be safe to call once with `data` from any thread that uses
/// `funcs`.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_user_data(
    funcs: *mut hr_paint_funcs_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    // SAFETY: by this function's contract `funcs` is `NULL` or live, `key`
    // outlives it, and `destroy` may be called with `data` when the entry
    // goes.
    i32::from(unsafe { object::set_user_data(funcs, key, data, destroy, replace != 0) })
}
/// Returns borrowed metadata.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_get_user_data(
    funcs: *const hr_paint_funcs_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `funcs` is `NULL` or live by this function's contract.
    unsafe { object::get_user_data(funcs.cast_mut(), key) }
}
/// Registers push transform. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// `funcs` must be live until this returns. Unless it is
/// immutable, no other thread may use it during the call, and nothing on
/// this thread may be in the middle of using it but a callback setter that
/// is running a `destroy`. For as long as `func` stays set, it must be safe
/// to call from any thread that uses `funcs`, with `user_data` and arguments
/// that meet the contract of [`hr_paint_push_transform`]. `destroy`, if
/// given, must be safe to call once with `user_data`, on whichever thread
/// releases the data and whenever that is: during this call, or when the
/// callback is replaced or `funcs` is freed.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_transform_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_transform_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: `set_callback` asks of `funcs`, `user_data` and `destroy` what
    // this function's contract does, and takes a `NULL` `funcs` besides.
    // Where it asks that no reference to the object be live, the contract
    // has nothing in the middle of using it but a setter running a
    // `destroy`, and `set_callback` documents that it holds no borrow then.
    // The closure returns the callback it displaces. Storing `func` with
    // `user_data` keeps the invariant on `push_transform`, by the same
    // contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_transform, new)
        });
    }
}
/// Dispatches push transform; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_transform(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    xx: f32,
    yx: f32,
    xy: f32,
    yy: f32,
    dx: f32,
    dy: f32,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_transform {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_transform`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe {
                func(
                    funcs,
                    paint_data,
                    xx,
                    yx,
                    xy,
                    yy,
                    dx,
                    dy,
                    callback.user_data,
                );
            }
        }
    }
}
/// Registers pop transform. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_pop_transform`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_pop_transform_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_pop_transform_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `pop_transform`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.pop_transform, new)
        });
    }
}
/// Dispatches pop transform; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_pop_transform(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.pop_transform {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `pop_transform`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe { func(funcs, paint_data, callback.user_data) }
        }
    }
}
/// Registers color glyph. Takes ownership of data, including when clearing or rejected.
///
/// A callback that paints by calling `hr_font_paint_glyph` is responsible for
/// guarding against cycles and for limiting how deeply it nests; see
/// `hr_paint_color_glyph_func_t`.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_color_glyph`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_color_glyph_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_color_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `color_glyph`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.color_glyph, new)
        });
    }
}
/// Dispatches color glyph; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with. `font`
/// must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_color_glyph(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    glyph: hr_codepoint_t,
    font: *mut hr_font_t,
) -> hr_bool_t {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.color_glyph {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `color_glyph`, `func` may be called
            // with its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            return unsafe { func(funcs, paint_data, glyph, font, callback.user_data) };
        }
    }
    0
}
/// Registers push clip glyph. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_clip_glyph`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_clip_glyph_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_clip_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_clip_glyph`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_clip_glyph, new)
        });
    }
}
/// Dispatches push clip glyph; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with. `font`
/// must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_clip_glyph(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    glyph: hr_codepoint_t,
    font: *mut hr_font_t,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_clip_glyph {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_clip_glyph`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe { func(funcs, paint_data, glyph, font, callback.user_data) }
        }
    }
}
/// Registers push clip rectangle. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_clip_rectangle`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_clip_rectangle_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_clip_rectangle_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_clip_rectangle`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_clip_rectangle, new)
        });
    }
}
/// Dispatches push clip rectangle; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_clip_rectangle(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    xmin: f32,
    ymin: f32,
    xmax: f32,
    ymax: f32,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_clip_rectangle {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_clip_rectangle`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe {
                func(
                    funcs,
                    paint_data,
                    xmin,
                    ymin,
                    xmax,
                    ymax,
                    callback.user_data,
                );
            }
        }
    }
}
/// Registers push clip path start. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_clip_path_start`] in place of [`hr_paint_push_transform`],
/// except that `func` need only be safe to call with a `draw_data` that is
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_clip_path_start_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_clip_path_start_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_clip_path_start`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_clip_path_start, new)
        });
    }
}
/// Dispatches push clip path start; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
/// `draw_data` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_clip_path_start(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    draw_data: *mut *mut c_void,
) -> *mut hr_draw_funcs_t {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    // A caller that does not want the draw data may pass `NULL`, but a
    // callback is owed a slot it can write, and writes it without looking.
    // Give it one of ours, as HarfBuzz does.
    let mut scratch = core::ptr::null_mut();
    let draw_data = if draw_data.is_null() {
        &raw mut scratch
    } else {
        draw_data
    };
    if let Some(callback) = &state.push_clip_path_start {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_clip_path_start`, `func` may
            // be called with its own `user_data` and arguments that meet this
            // function's contract, with a `draw_data` it can write. `funcs`
            // and `paint_data` are the caller's, and `draw_data` is the
            // caller's writable one or the local above, which outlives the
            // call.
            return unsafe { func(funcs, paint_data, draw_data, callback.user_data) };
        }
    }
    // SAFETY: `draw_data` is the caller's, writable by this function's
    // contract, or the local above. It only carries a value out, so it is
    // written through the pointer.
    unsafe { draw_data.write(core::ptr::null_mut()) };
    core::ptr::null_mut()
}
/// Registers push clip path end. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_clip_path_end`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_clip_path_end_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_clip_path_end_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_clip_path_end`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_clip_path_end, new)
        });
    }
}
/// Dispatches push clip path end; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_clip_path_end(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_clip_path_end {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_clip_path_end`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe { func(funcs, paint_data, callback.user_data) }
        }
    }
}
/// Registers pop clip. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_pop_clip`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_pop_clip_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_pop_clip_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `pop_clip`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.pop_clip, new)
        });
    }
}
/// Dispatches pop clip; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_pop_clip(funcs: *mut hr_paint_funcs_t, paint_data: *mut c_void) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.pop_clip {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `pop_clip`, `func` may be called
            // with its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            unsafe { func(funcs, paint_data, callback.user_data) }
        }
    }
}
/// Registers color. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_color`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_color_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_color_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `color`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.color, new)
        });
    }
}
/// Dispatches color; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_color(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    is_foreground: hr_bool_t,
    color: hr_color_t,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.color {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `color`, `func` may be called with
            // its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            unsafe { func(funcs, paint_data, is_foreground, color, callback.user_data) }
        }
    }
}
/// Registers fill glyph. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_fill_glyph`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_fill_glyph_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_fill_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `fill_glyph`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.fill_glyph, new)
        });
    }
}
/// Dispatches fill glyph; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with. `font`
/// must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_fill_glyph(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    glyph: hr_codepoint_t,
    font: *mut hr_font_t,
    is_foreground: hr_bool_t,
    color: hr_color_t,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.fill_glyph {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `fill_glyph`, `func` may be called
            // with its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            return unsafe {
                func(
                    funcs,
                    paint_data,
                    glyph,
                    font,
                    is_foreground,
                    color,
                    callback.user_data,
                );
            };
        }
    }
    // SAFETY: `funcs` is `NULL`, or live and unmodified until this returns
    // by this function's contract, and `paint_data` is its paint data,
    // which is what each of the three calls requires. The first requires a
    // `NULL` or live `font` besides, which the same contract gives.
    unsafe {
        hr_paint_push_clip_glyph(funcs, paint_data, glyph, font);
        hr_paint_color(funcs, paint_data, is_foreground, color);
        hr_paint_pop_clip(funcs, paint_data);
    }
}
/// Registers image. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_image`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_image_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_image_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `image`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.image, new)
        });
    }
}
/// Dispatches image; omitted callbacks use the HarfBuzz default.
///
/// `slant` is deprecated and ignored: the callback is handed zero, as in
/// HarfBuzz.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with. `image`
/// must be `NULL` or a live blob, and `extents` must be `NULL` or point to
/// glyph extents that can be read and written.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_image(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    image: *mut hr_blob_t,
    width: c_uint,
    height: c_uint,
    format: hr_tag_t,
    slant: f32,
    extents: *mut hr_glyph_extents_t,
) -> hr_bool_t {
    // Named as in the header, and unused.
    let _ = slant;
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.image {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `image`, `func` may be called with
            // its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            return unsafe {
                func(
                    funcs,
                    paint_data,
                    image,
                    width,
                    height,
                    format,
                    0.0,
                    extents,
                    callback.user_data,
                )
            };
        }
    }
    0
}
/// Registers linear gradient. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_linear_gradient`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_linear_gradient_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_linear_gradient_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `linear_gradient`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.linear_gradient, new)
        });
    }
}
/// Dispatches linear gradient; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
/// `color_line` must point to a color line that stays as
/// [`hr_color_line_get_color_stops`] requires until this returns.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_linear_gradient(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    color_line: *mut hr_color_line_t,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.linear_gradient {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `linear_gradient`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe {
                func(
                    funcs,
                    paint_data,
                    color_line,
                    x0,
                    y0,
                    x1,
                    y1,
                    x2,
                    y2,
                    callback.user_data,
                );
            }
        }
    }
}
/// Registers radial gradient. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_radial_gradient`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_radial_gradient_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_radial_gradient_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `radial_gradient`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.radial_gradient, new)
        });
    }
}
/// Dispatches radial gradient; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
/// `color_line` must point to a color line that stays as
/// [`hr_color_line_get_color_stops`] requires until this returns.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_radial_gradient(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    color_line: *mut hr_color_line_t,
    x0: f32,
    y0: f32,
    r0: f32,
    x1: f32,
    y1: f32,
    r1: f32,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.radial_gradient {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `radial_gradient`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe {
                func(
                    funcs,
                    paint_data,
                    color_line,
                    x0,
                    y0,
                    r0,
                    x1,
                    y1,
                    r1,
                    callback.user_data,
                );
            }
        }
    }
}
/// Registers sweep gradient. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_sweep_gradient`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_sweep_gradient_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_sweep_gradient_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `sweep_gradient`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.sweep_gradient, new)
        });
    }
}
/// Dispatches sweep gradient; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
/// `color_line` must point to a color line that stays as
/// [`hr_color_line_get_color_stops`] requires until this returns.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_sweep_gradient(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    color_line: *mut hr_color_line_t,
    x0: f32,
    y0: f32,
    start_angle: f32,
    end_angle: f32,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.sweep_gradient {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `sweep_gradient`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            unsafe {
                func(
                    funcs,
                    paint_data,
                    color_line,
                    x0,
                    y0,
                    start_angle,
                    end_angle,
                    callback.user_data,
                );
            }
        }
    }
}
/// Registers push group. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_group`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_group_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_group_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_group`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_group, new)
        });
    }
}
/// Dispatches push group; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_group(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_group {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_group`, `func` may be called
            // with its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            unsafe { func(funcs, paint_data, callback.user_data) }
        }
    }
}
/// Registers push group for. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_push_group_for`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_push_group_for_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_push_group_for_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `push_group_for`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.push_group_for, new)
        });
    }
}
/// Dispatches push group for; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_group_for(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    mode: hr_paint_composite_mode_t,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.push_group_for {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `push_group_for`, `func` may be
            // called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            return unsafe { func(funcs, paint_data, mode, callback.user_data) };
        }
    }
    // SAFETY: `funcs` is `NULL`, or live and unmodified until this returns
    // by this function's contract, and `paint_data` is its paint data, as
    // `hr_paint_push_group` requires.
    unsafe { hr_paint_push_group(funcs, paint_data) }
}
/// Registers pop group. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_pop_group`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_pop_group_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_pop_group_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `pop_group`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.pop_group, new)
        });
    }
}
/// Dispatches pop group; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_pop_group(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    mode: hr_paint_composite_mode_t,
) {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.pop_group {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `pop_group`, `func` may be called
            // with its own `user_data` and arguments that meet this function's
            // contract, and those are what it is given.
            unsafe { func(funcs, paint_data, mode, callback.user_data) }
        }
    }
}
/// Registers custom palette color. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_paint_funcs_set_push_transform_func`], with
/// [`hr_paint_custom_palette_color`] in place of [`hr_paint_push_transform`].
#[no_mangle]
pub unsafe extern "C" fn hr_paint_funcs_set_custom_palette_color_func(
    funcs: *mut hr_paint_funcs_t,
    func: hr_paint_custom_palette_color_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_paint_funcs_set_push_transform_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `funcs`,
    // `user_data` and `destroy`, and the closure returns the callback it
    // displaces. Storing `func` with `user_data` keeps the invariant on
    // `custom_palette_color`, by the same contract.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.custom_palette_color, new)
        });
    }
}
/// Dispatches custom palette color; omitted callbacks use the HarfBuzz default.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `paint_data` must be paint data its callbacks may be called with. `color`
/// must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_custom_palette_color(
    funcs: *mut hr_paint_funcs_t,
    paint_data: *mut c_void,
    color_index: c_uint,
    color: *mut hr_color_t,
) -> hr_bool_t {
    // SAFETY: `funcs` is `NULL`, or live by this function's contract, which
    // also keeps it unmodified while `state` borrows it.
    let state = unsafe { object::or_empty(funcs) };
    if let Some(callback) = &state.custom_palette_color {
        if let Some(func) = callback.func {
            // SAFETY: by the invariant on `custom_palette_color`, `func` may
            // be called with its own `user_data` and arguments that meet this
            // function's contract, and those are what it is given.
            return unsafe { func(funcs, paint_data, color_index, color, callback.user_data) };
        }
    }
    0
}

/// Gradient extend mode.
pub type hr_paint_extend_t = c_uint;
/// PNG image bytes, decoded by the client.
pub const HR_PAINT_IMAGE_FORMAT_PNG: hr_tag_t = 0x706e_6720;
/// SVG document bytes, rendered by the client.
pub const HR_PAINT_IMAGE_FORMAT_SVG: hr_tag_t = 0x7376_6720;
/// Premultiplied BGRA pixels in the sRGBA color space.
pub const HR_PAINT_IMAGE_FORMAT_BGRA: hr_tag_t = 0x4247_5241;
pub const HR_PAINT_EXTEND_PAD: hr_paint_extend_t = 0;
pub const HR_PAINT_EXTEND_REPEAT: hr_paint_extend_t = 1;
pub const HR_PAINT_EXTEND_REFLECT: hr_paint_extend_t = 2;
/// COLR compositing mode, with the OpenType/HarfBuzz numeric values.
pub type hr_paint_composite_mode_t = c_uint;
/// One gradient stop.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct hr_color_stop_t {
    pub offset: f32,
    pub is_foreground: hr_bool_t,
    pub color: hr_color_t,
}
pub type hr_color_line_get_color_stops_func_t = Option<
    unsafe extern "C" fn(
        line: *mut hr_color_line_t,
        data: *mut c_void,
        start: c_uint,
        count: *mut c_uint,
        stops: *mut hr_color_stop_t,
        user_data: *mut c_void,
    ) -> c_uint,
>;
pub type hr_color_line_get_extend_func_t = Option<
    unsafe extern "C" fn(
        line: *mut hr_color_line_t,
        data: *mut c_void,
        user_data: *mut c_void,
    ) -> hr_paint_extend_t,
>;
/// Borrowed gradient information, valid only for the enclosing paint callback.
#[repr(C)]
pub struct hr_color_line_t {
    pub data: *mut c_void,
    pub get_color_stops: hr_color_line_get_color_stops_func_t,
    pub get_color_stops_user_data: *mut c_void,
    pub get_extend: hr_color_line_get_extend_func_t,
    pub get_extend_user_data: *mut c_void,
    reserved: [*mut c_void; 8],
}
/// Returns the total number of stops and copies a paginated slice if requested.
///
/// When both output pointers are non-null, `count` gives capacity on entry
/// and the number written on return. A null array sets the count to zero
/// without reading it, whichever callback the line carries; the README has
/// the rule for a count and its array. A null color line, or one with no
/// callback, has no stops.
///
/// The line's `get_color_stops` is called with a count and an array, or with
/// `NULL` for both: a count passed here without an array is kept from it.
/// The count is one of the library's own, and the callback is not believed
/// to have written more stops than the capacity it gave.
///
/// # Safety
///
/// `line` must be `NULL` or point to a color line that nothing modifies
/// during the call. Each callback the line carries must be safe to call with
/// the line, its `data` and that callback's user data, and `get_color_stops`
/// with a count and an array that are as this function requires its own to
/// be. `count` must be `NULL` or writable. When `stops` is non-null, `count`
/// must be `NULL` or hold the number of stops `stops` is writable for.
#[no_mangle]
pub unsafe extern "C" fn hr_color_line_get_color_stops(
    line: *mut hr_color_line_t,
    start: c_uint,
    count: *mut c_uint,
    stops: *mut hr_color_stop_t,
) -> c_uint {
    // SAFETY: `Page::new` needs the count null or writable, and, with
    // neither pointer null, set to a number of stops the array is writable
    // for, until the page is dropped, which is before this returns. This
    // function's contract has `count` `NULL` or writable, and with a
    // non-null `stops` holding the number of stops that is writable for.
    let page = unsafe { Page::new(count, stops) };
    // A line that is not there, or that has no callback, has no stops. The
    // page is then dropped with nothing written, which is what the count
    // says.
    // SAFETY: `as_ref` needs `line` to be null, or aligned and pointing to a
    // live `hr_color_line_t` that nothing writes while `l` is read. By this
    // function's contract `line` is `NULL` or points to a color line that
    // nothing modifies during the call.
    let Some(l) = (unsafe { line.as_ref() }) else {
        return 0;
    };
    let Some(call) = l.get_color_stops else {
        return 0;
    };
    page.fill_from(|count, stops| {
        // SAFETY: `call` is the line's `get_color_stops`, which this
        // function's contract has safe to call with the line, its `data`,
        // its user data, and a count and an array that are as it requires
        // its own to be. By the invariant `fill_from` documents, the two
        // are both null, or the count is writable and holds a number of
        // stops the array is writable for.
        unsafe {
            call(
                line,
                l.data,
                start,
                count,
                stops,
                l.get_color_stops_user_data,
            )
        }
    })
}
/// Returns the extend mode, or PAD when absent.
///
/// # Safety
///
/// `line` must be as [`hr_color_line_get_color_stops`] requires.
#[no_mangle]
pub unsafe extern "C" fn hr_color_line_get_extend(line: *mut hr_color_line_t) -> hr_paint_extend_t {
    // SAFETY: `as_ref` needs `line` to be null, or aligned and pointing to a
    // live `hr_color_line_t` that nothing writes while `l` is read. By this
    // function's contract `line` is `NULL` or points to a color line that
    // nothing modifies during the call.
    let Some(l) = (unsafe { line.as_ref() }) else {
        return HR_PAINT_EXTEND_PAD;
    };
    // SAFETY: `call` is the line's `get_extend`, which this function's
    // contract has safe to call with the line, its `data` and its user data.
    l.get_extend.map_or(HR_PAINT_EXTEND_PAD, |call| unsafe {
        call(line, l.data, l.get_extend_user_data)
    })
}
/// Applies the font's scale relative to design units.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `data` must be paint data its callbacks may be called with. `font` must be
/// live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_font_transform(
    funcs: *mut hr_paint_funcs_t,
    data: *mut c_void,
    font: *mut hr_font_t,
) {
    // SAFETY: `scale` needs `font` to be `NULL` or live. It is `NULL`, or
    // live by this function's contract.
    let (x, y) = unsafe { crate::rendering::scale(font) };
    // SAFETY: `funcs` is `NULL`, or live and unmodified until this returns
    // by this function's contract, and `data` is its paint data, as
    // `hr_paint_push_transform` requires.
    unsafe { hr_paint_push_transform(funcs, data, x, 0.0, 0.0, y, 0.0, 0.0) }
}
/// Applies the inverse font scale; zero axes stay zero.
///
/// # Safety
///
/// `funcs` must be live and unmodified until this returns, and
/// `data` must be paint data its callbacks may be called with. `font` must be
/// live.
#[no_mangle]
pub unsafe extern "C" fn hr_paint_push_inverse_font_transform(
    funcs: *mut hr_paint_funcs_t,
    data: *mut c_void,
    font: *mut hr_font_t,
) {
    // SAFETY: `scale` needs `font` to be `NULL` or live. It is `NULL`, or
    // live by this function's contract.
    let (x, y) = unsafe { crate::rendering::scale(font) };
    // SAFETY: `funcs` is `NULL`, or live and unmodified until this returns
    // by this function's contract, and `data` is its paint data, as
    // `hr_paint_push_transform` requires.
    unsafe {
        hr_paint_push_transform(
            funcs,
            data,
            if x == 0.0 { 0.0 } else { 1.0 / x },
            0.0,
            0.0,
            if y == 0.0 { 0.0 } else { 1.0 / y },
            0.0,
            0.0,
        );
    }
}
pub const HR_PAINT_COMPOSITE_MODE_CLEAR: hr_paint_composite_mode_t = 0;
pub const HR_PAINT_COMPOSITE_MODE_SRC: hr_paint_composite_mode_t = 1;
pub const HR_PAINT_COMPOSITE_MODE_DEST: hr_paint_composite_mode_t = 2;
pub const HR_PAINT_COMPOSITE_MODE_SRC_OVER: hr_paint_composite_mode_t = 3;
pub const HR_PAINT_COMPOSITE_MODE_DEST_OVER: hr_paint_composite_mode_t = 4;
pub const HR_PAINT_COMPOSITE_MODE_SRC_IN: hr_paint_composite_mode_t = 5;
pub const HR_PAINT_COMPOSITE_MODE_DEST_IN: hr_paint_composite_mode_t = 6;
pub const HR_PAINT_COMPOSITE_MODE_SRC_OUT: hr_paint_composite_mode_t = 7;
pub const HR_PAINT_COMPOSITE_MODE_DEST_OUT: hr_paint_composite_mode_t = 8;
pub const HR_PAINT_COMPOSITE_MODE_SRC_ATOP: hr_paint_composite_mode_t = 9;
pub const HR_PAINT_COMPOSITE_MODE_DEST_ATOP: hr_paint_composite_mode_t = 10;
pub const HR_PAINT_COMPOSITE_MODE_XOR: hr_paint_composite_mode_t = 11;
pub const HR_PAINT_COMPOSITE_MODE_PLUS: hr_paint_composite_mode_t = 12;
pub const HR_PAINT_COMPOSITE_MODE_SCREEN: hr_paint_composite_mode_t = 13;
pub const HR_PAINT_COMPOSITE_MODE_OVERLAY: hr_paint_composite_mode_t = 14;
pub const HR_PAINT_COMPOSITE_MODE_DARKEN: hr_paint_composite_mode_t = 15;
pub const HR_PAINT_COMPOSITE_MODE_LIGHTEN: hr_paint_composite_mode_t = 16;
pub const HR_PAINT_COMPOSITE_MODE_COLOR_DODGE: hr_paint_composite_mode_t = 17;
pub const HR_PAINT_COMPOSITE_MODE_COLOR_BURN: hr_paint_composite_mode_t = 18;
pub const HR_PAINT_COMPOSITE_MODE_HARD_LIGHT: hr_paint_composite_mode_t = 19;
pub const HR_PAINT_COMPOSITE_MODE_SOFT_LIGHT: hr_paint_composite_mode_t = 20;
pub const HR_PAINT_COMPOSITE_MODE_DIFFERENCE: hr_paint_composite_mode_t = 21;
pub const HR_PAINT_COMPOSITE_MODE_EXCLUSION: hr_paint_composite_mode_t = 22;
pub const HR_PAINT_COMPOSITE_MODE_MULTIPLY: hr_paint_composite_mode_t = 23;
pub const HR_PAINT_COMPOSITE_MODE_HSL_HUE: hr_paint_composite_mode_t = 24;
pub const HR_PAINT_COMPOSITE_MODE_HSL_SATURATION: hr_paint_composite_mode_t = 25;
pub const HR_PAINT_COMPOSITE_MODE_HSL_COLOR: hr_paint_composite_mode_t = 26;
pub const HR_PAINT_COMPOSITE_MODE_HSL_LUMINOSITY: hr_paint_composite_mode_t = 27;

mod backend;

/// Legacy color-glyph callback, assumed to succeed.
pub type hr_font_paint_glyph_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        palette: c_uint,
        foreground: hr_color_t,
        user_data: *mut c_void,
    ),
>;
/// Custom color-glyph callback. Returns true if painting succeeded.
pub type hr_font_paint_glyph_or_fail_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        funcs: *mut hr_paint_funcs_t,
        paint_data: *mut c_void,
        palette: c_uint,
        foreground: hr_color_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;
/// Registers custom painting, owning its data even on rejection or clearing.
///
/// # Safety
///
/// `ffuncs` must be live until this returns. Unless it is
/// immutable, no other thread may use it during the call, and nothing on
/// this thread may be in the middle of using it, or a font that carries it,
/// but a callback setter that is running a `destroy`. For as long as `func`
/// stays set, it must be safe to call from any thread that uses `ffuncs`,
/// with `user_data`, a font that carries `ffuncs`, the font data that font
/// was given with them, and otherwise arguments that meet the contract of
/// [`hr_font_paint_glyph_or_fail`]. `destroy`, if given, must be safe to
/// call once with `user_data`, on whichever thread releases the data and
/// whenever that is: during this call, or when the callback is displaced or
/// `ffuncs` is freed.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_paint_glyph_func(
    ffuncs: *mut crate::hr_font_funcs_t,
    func: hr_font_paint_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: `set_callback` asks of `ffuncs`, `user_data` and `destroy`
    // what this function's contract does, and takes a `NULL` `ffuncs`
    // besides. Where it asks that no reference to the object be live, the
    // contract has nothing in the middle of using it but a setter running a
    // `destroy`, and `set_callback` documents that it holds no borrow then.
    // The closure returns both callbacks it takes out: the one it displaces,
    // and the one of the other kind, which this one supersedes.
    unsafe {
        set_callback(ffuncs, func, user_data, destroy, |state, new| {
            let old = core::mem::replace(&mut state.paint_glyph_legacy, new);
            (old, state.paint_glyph.take())
        });
    }
}
/// Registers custom painting returning success.
///
/// # Safety
///
/// As [`hr_font_funcs_set_paint_glyph_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_paint_glyph_or_fail_func(
    ffuncs: *mut crate::hr_font_funcs_t,
    func: hr_font_paint_glyph_or_fail_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_font_funcs_set_paint_glyph_func`, whose contract
    // this function shares: it covers what `set_callback` asks of `ffuncs`,
    // `user_data` and `destroy`. The closure returns both callbacks it takes
    // out: the one it displaces, and the one of the other kind, which this
    // one supersedes.
    unsafe {
        set_callback(ffuncs, func, user_data, destroy, |state, new| {
            let old = core::mem::replace(&mut state.paint_glyph, new);
            (old, state.paint_glyph_legacy.take())
        });
    }
}
/// Paints COLRv0/v1, SVG, CBDT or sbix through client callbacks.
/// Returns false on malformed data, unsupported images, or paint failure.
///
/// # Safety
///
/// `font` must be live and `funcs` must be `NULL` or live, both until this
/// returns. Until then, nothing may modify `funcs`, `font` or a font it
/// descends from, or the font callbacks any of those fonts carries; that
/// binds the callbacks this runs too. `data` must be paint data the
/// callbacks of `funcs` may be called with.
#[no_mangle]
pub unsafe extern "C" fn hr_font_paint_glyph_or_fail(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    funcs: *mut hr_paint_funcs_t,
    data: *mut c_void,
    palette: c_uint,
    foreground: hr_color_t,
) -> hr_bool_t {
    // SAFETY: `as_ref` needs `font` to be null, or aligned and pointing to a
    // live `hr_font_t` that nothing writes while `state` is used, which is
    // until this returns. `font` is `NULL`, or live by this function's
    // contract, and unmodified until then.
    let Some(state) = (unsafe { font.as_ref() }) else {
        return 0;
    };
    // SAFETY: `as_ref` needs `state.funcs` to be null, or aligned and
    // pointing to a live `hr_font_funcs_t` that nothing writes while `ffuncs`
    // is used. A font owns its reference to the callbacks it carries, so
    // they are live while the font is, and this function's contract keeps
    // them unmodified until it returns.
    if let Some(ffuncs) = unsafe { state.funcs.as_ref() } {
        if let Some(cb) = &ffuncs.paint_glyph_legacy {
            if let Some(call) = cb.func {
                // SAFETY: `call` and `cb.user_data` were given together to
                // `hr_font_funcs_set_paint_glyph_func`, the one place that
                // stores this callback. Its contract has `call` safe to call
                // with that data, a font that carries `ffuncs` and the font
                // data it was given with them, and otherwise arguments that
                // meet this function's contract. `font` carries `ffuncs`,
                // `callback_data` is that font data, and the rest are this
                // call's own arguments.
                unsafe {
                    call(
                        font,
                        state.callback_data(),
                        glyph,
                        funcs,
                        data,
                        palette,
                        foreground,
                        cb.user_data,
                    );
                }
                return 1;
            }
        }
        if let Some(cb) = &ffuncs.paint_glyph {
            if let Some(call) = cb.func {
                // SAFETY: as for the legacy callback above, with
                // `hr_font_funcs_set_paint_glyph_or_fail_func` as the one
                // place that stores this callback.
                return unsafe {
                    call(
                        font,
                        state.callback_data(),
                        glyph,
                        funcs,
                        data,
                        palette,
                        foreground,
                        cb.user_data,
                    )
                };
            }
        }
        if state.funcs != crate::font_funcs::builtin_funcs() && state.parent.is_null() {
            return 0;
        }
    }
    if state.funcs != crate::font_funcs::builtin_funcs() && !state.parent.is_null() {
        // SAFETY: making the reference needs `state.parent` to be non-null,
        // which was just checked, aligned, and pointing to a live `hr_font_t`
        // that nothing writes while `parent` is used. A font owns its
        // reference to its parent, so the parent is live while `font` is,
        // and this function's contract keeps the fonts `font` descends from
        // unmodified until it returns.
        let parent = unsafe { &*state.parent };
        // SAFETY: `funcs` is `NULL`, or live and unmodified until this
        // returns by this function's contract, and `data` is its paint data,
        // as `hr_paint_push_transform` requires.
        unsafe {
            hr_paint_push_transform(
                funcs,
                data,
                if parent.x_scale == 0 {
                    0.0
                } else {
                    state.x_scale as f32 / parent.x_scale as f32
                },
                0.0,
                0.0,
                if parent.y_scale == 0 {
                    0.0
                } else {
                    state.y_scale as f32 / parent.y_scale as f32
                },
                0.0,
                0.0,
            );
        };
        // SAFETY: this function's contract holds with the parent in place of
        // `font`. The parent is live, as above, and it, the fonts it
        // descends from and the font callbacks they carry are among what
        // the contract already keeps unmodified. The other arguments are
        // this call's own.
        let result = unsafe {
            hr_font_paint_glyph_or_fail(state.parent, glyph, funcs, data, palette, foreground)
        };
        // SAFETY: `funcs` is `NULL`, or live and unmodified until this
        // returns by this function's contract, and `data` is its paint data,
        // as `hr_paint_pop_transform` requires.
        unsafe { hr_paint_pop_transform(funcs, data) };
        return result;
    }
    // SAFETY: `paint` has this function's contract, and is given this call's
    // arguments.
    unsafe { backend::paint(font, glyph, funcs, data, palette, foreground) }
}
/// Compatibility entry point ignoring the success result.
///
/// # Safety
///
/// As [`hr_font_paint_glyph_or_fail`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_paint_glyph(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    funcs: *mut hr_paint_funcs_t,
    data: *mut c_void,
    palette: c_uint,
    foreground: hr_color_t,
) {
    // SAFETY: this function's contract is `hr_font_paint_glyph_or_fail`'s,
    // and it passes on the arguments it was given.
    unsafe { hr_font_paint_glyph_or_fail(font, glyph, funcs, data, palette, foreground) };
}
