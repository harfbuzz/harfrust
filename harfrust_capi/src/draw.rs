//! Draw callbacks with HarfBuzz-compatible ownership and dispatch.
// The C drawing/painting ABI uses f32 coordinates, including integer font scales.
#![allow(clippy::cast_precision_loss)]
use crate::font_funcs::{set_callback, Callback};
use crate::object::{self, Empty, Object, ObjectHeader};
use crate::{hr_bool_t, hr_codepoint_t, hr_destroy_func_t, hr_font_t, hr_user_data_key_t};
use core::ffi::c_void;
use std::sync::OnceLock;
/// Callback for move to; data must be safe to use on any thread.
pub type hr_draw_move_to_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        st: *mut hr_draw_state_t,
        to_x: f32,
        to_y: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for line to; data must be safe to use on any thread.
pub type hr_draw_line_to_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        st: *mut hr_draw_state_t,
        to_x: f32,
        to_y: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for quadratic to; data must be safe to use on any thread.
pub type hr_draw_quadratic_to_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        st: *mut hr_draw_state_t,
        control_x: f32,
        control_y: f32,
        to_x: f32,
        to_y: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for cubic to; data must be safe to use on any thread.
pub type hr_draw_cubic_to_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        st: *mut hr_draw_state_t,
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        to_x: f32,
        to_y: f32,
        user_data: *mut c_void,
    ),
>;
/// Callback for close path; data must be safe to use on any thread.
pub type hr_draw_close_path_func_t = Option<
    unsafe extern "C" fn(
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        st: *mut hr_draw_state_t,
        user_data: *mut c_void,
    ),
>;
/// A reference-counted, optionally immutable collection of draw callbacks.
#[derive(Default)]
pub struct hr_draw_funcs_t {
    header: ObjectHeader,
    // Safety invariant: the function of each callback below, if it has one,
    // is safe to call on any thread with this object's pointer, draw data
    // meant for this object's callbacks, a state it may read and write, and
    // the callback's own `user_data`.
    move_to: Option<Callback<hr_draw_move_to_func_t>>,
    line_to: Option<Callback<hr_draw_line_to_func_t>>,
    quadratic_to: Option<Callback<hr_draw_quadratic_to_func_t>>,
    cubic_to: Option<Callback<hr_draw_cubic_to_func_t>>,
    close_path: Option<Callback<hr_draw_close_path_func_t>>,
}
// SAFETY: callback data is opaque, and goes only to its callback and its
// destroy function. The README's "Threading" section requires both to be safe
// from any thread that uses the object.
unsafe impl Send for hr_draw_funcs_t {}
// SAFETY: as for `Send`. Shared, the callbacks are read and called from
// several threads at once, which is the use that requirement is there for.
unsafe impl Sync for hr_draw_funcs_t {}
impl Object for hr_draw_funcs_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }
    fn empty() -> *mut Self {
        static EMPTY: OnceLock<Empty<hr_draw_funcs_t>> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                Empty::new(hr_draw_funcs_t {
                    header: ObjectHeader::immortal(),
                    ..Default::default()
                })
            })
            .get()
    }
}
/// Creates an owned callback collection.
#[no_mangle]
pub extern "C" fn hr_draw_funcs_create() -> *mut hr_draw_funcs_t {
    object::create(hr_draw_funcs_t::default())
}
/// Returns the immortal empty collection.
#[no_mangle]
pub extern "C" fn hr_draw_funcs_get_empty() -> *mut hr_draw_funcs_t {
    hr_draw_funcs_t::empty()
}
/// Takes a reference. Accepts NULL.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_reference(
    funcs: *mut hr_draw_funcs_t,
) -> *mut hr_draw_funcs_t {
    // SAFETY: `funcs` is `NULL` or live by this function's contract. The
    // reference taken is the caller's.
    unsafe { object::reference(funcs) }
}
/// Releases an owned reference and its callback data. Accepts NULL.
///
/// # Safety
///
/// `funcs` must be `NULL` or live, and the caller must own the reference
/// being released.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_destroy(funcs: *mut hr_draw_funcs_t) {
    // SAFETY: by this function's contract `funcs` is `NULL` or live, and the
    // caller owns the reference it gives up here.
    unsafe { object::destroy(funcs) }
}
/// Rejects subsequent callback changes.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_make_immutable(funcs: *mut hr_draw_funcs_t) {
    // SAFETY: `funcs` is `NULL` or live by this function's contract.
    unsafe { object::make_immutable(funcs) }
}
/// Reports whether callback changes are forbidden.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_is_immutable(funcs: *const hr_draw_funcs_t) -> hr_bool_t {
    // SAFETY: `funcs` is `NULL` or live by this function's contract.
    i32::from(unsafe { object::is_immutable(funcs) })
}
/// Attaches address-keyed metadata, taking ownership only on success.
///
/// # Safety
///
/// `funcs` must be `NULL` or live, and `key` must outlive it. `destroy`, if
/// given, must be safe to call once with `data`, on whichever thread
/// releases it.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_user_data(
    funcs: *mut hr_draw_funcs_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    // SAFETY: by this function's contract `funcs` is `NULL` or live, `key`
    // outlives it, and `destroy` may be called once with `data` wherever
    // the object comes to release it.
    i32::from(unsafe { object::set_user_data(funcs, key, data, destroy, replace != 0) })
}
/// Returns borrowed metadata.
///
/// # Safety
///
/// `funcs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_get_user_data(
    funcs: *const hr_draw_funcs_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `funcs` is `NULL` or live by this function's contract.
    unsafe { object::get_user_data(funcs.cast_mut(), key) }
}
/// Registers move to. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// `funcs` must be `NULL` or live. Mutation requires exclusive access:
/// unless `funcs` is immutable, nothing else may be using it while this
/// runs.
///
/// `func`, if given, must be safe to call on any thread with `funcs`, draw
/// data meant for the callbacks of `funcs`, a state it may read and write,
/// and `user_data`. `destroy`, if given, must be safe to call once with
/// `user_data`, on whichever thread releases the data and at whichever
/// point that is: during this call when `func` is `NULL` or `funcs` refuses
/// the change, and otherwise when the callback is replaced or `funcs` is
/// freed.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_move_to_func(
    funcs: *mut hr_draw_funcs_t,
    func: hr_draw_move_to_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: `set_callback` asks of `funcs` and of `destroy` what this
    // function's contract does, and of `store` that it hand back the
    // callback it takes out, which is all this one does. The callback it
    // puts in upholds the invariant on the slot: the same contract has
    // `func` safe to call as the invariant says.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.move_to, new)
        });
    }
}
/// Registers line to. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_draw_funcs_set_move_to_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_line_to_func(
    funcs: *mut hr_draw_funcs_t,
    func: hr_draw_line_to_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_draw_funcs_set_move_to_func`, whose contract is
    // this function's own.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.line_to, new)
        });
    }
}
/// Registers quadratic to. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_draw_funcs_set_move_to_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_quadratic_to_func(
    funcs: *mut hr_draw_funcs_t,
    func: hr_draw_quadratic_to_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_draw_funcs_set_move_to_func`, whose contract is
    // this function's own.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.quadratic_to, new)
        });
    }
}
/// Registers cubic to. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_draw_funcs_set_move_to_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_cubic_to_func(
    funcs: *mut hr_draw_funcs_t,
    func: hr_draw_cubic_to_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_draw_funcs_set_move_to_func`, whose contract is
    // this function's own.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.cubic_to, new)
        });
    }
}
/// Registers close path. Takes ownership of data, including when clearing or rejected.
///
/// # Safety
///
/// As [`hr_draw_funcs_set_move_to_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_funcs_set_close_path_func(
    funcs: *mut hr_draw_funcs_t,
    func: hr_draw_close_path_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_draw_funcs_set_move_to_func`, whose contract is
    // this function's own.
    unsafe {
        set_callback(funcs, func, user_data, destroy, |state, new| {
            core::mem::replace(&mut state.close_path, new)
        });
    }
}

/// Drawing state. Its layout matches HarfBuzz, including private reserved words.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct hr_draw_state_t {
    pub path_open: hr_bool_t,
    pub path_start_x: f32,
    pub path_start_y: f32,
    pub current_x: f32,
    pub current_y: f32,
    reserved: [u32; 7],
}
/// Calls the move-to callback, if there is one.
///
/// # Safety
///
/// As [`hr_draw_move_to`], and `st` must not be `NULL`.
unsafe fn emit_move(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    x: f32,
    y: f32,
) {
    // SAFETY: `funcs` is `NULL` or live by this function's contract, and `f`
    // is not used past this call.
    let f = unsafe { object::or_empty(funcs) };
    if let Some(cb) = &f.move_to {
        if let Some(call) = cb.func {
            // SAFETY: by the invariant on the callbacks of
            // `hr_draw_funcs_t`, `call` may be called on this thread with
            // the pointer of the object that holds it, draw data meant for
            // that object's callbacks, a state it may read and write, and
            // its own `user_data`. `funcs` is that pointer, and not `NULL`,
            // since the empty object holds no callbacks. `data` and `st`
            // are that draw data and that state by this function's
            // contract.
            unsafe { call(funcs, data, st, x, y, cb.user_data) }
        }
    }
}
/// Calls the line-to callback, if there is one.
///
/// # Safety
///
/// As [`hr_draw_move_to`], and `st` must not be `NULL`.
unsafe fn emit_line(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    x: f32,
    y: f32,
) {
    // SAFETY: `funcs` is `NULL` or live by this function's contract, and `f`
    // is not used past this call.
    let f = unsafe { object::or_empty(funcs) };
    if let Some(cb) = &f.line_to {
        if let Some(call) = cb.func {
            // SAFETY: as in `emit_move`: the invariant on the callbacks has
            // `call` safe to call with `funcs`, which holds it, its own
            // `user_data`, and the draw data and state that this function's
            // contract has `data` and `st` be.
            unsafe { call(funcs, data, st, x, y, cb.user_data) }
        }
    }
}
/// Opens a contour at the current point, unless one is open.
///
/// # Safety
///
/// As [`hr_draw_move_to`].
unsafe fn start(funcs: *mut hr_draw_funcs_t, data: *mut c_void, st: *mut hr_draw_state_t) {
    if st.is_null() {
        return;
    }
    // SAFETY: each `(*st)` here reads or writes one field of the state
    // through the pointer, and no reference to the state is made. That
    // needs `st` to be non-null and aligned, and to point to a state that
    // has been set and may be read and written. It is not null, as just
    // checked, and this function's contract has the rest. `emit_move` is
    // handed this function's own arguments under what is its own contract,
    // with `st` not null. Its callback is handed `st` too and may read and
    // write the state through it, which is why nothing here holds a
    // reference to the state across the call.
    unsafe {
        if (*st).path_open == 0 {
            emit_move(funcs, data, st, (*st).current_x, (*st).current_y);
            (*st).path_open = 1;
            (*st).path_start_x = (*st).current_x;
            (*st).path_start_y = (*st).current_y;
        }
    }
}
/// Begins a contour, closing any open contour. Move callbacks are emitted lazily.
///
/// # Safety
///
/// `funcs` must be `NULL` or live, and must not be modified while this
/// runs. `data` must be draw data meant for its callbacks. `st` must be
/// `NULL`, or point to a state that has been set and that this may read and
/// write: nothing else may access it while this runs, apart from the
/// callbacks this hands it to.
#[no_mangle]
pub unsafe extern "C" fn hr_draw_move_to(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    to_x: f32,
    to_y: f32,
) {
    if st.is_null() {
        return;
    }
    // SAFETY: as in `start`, each `(*st)` reads or writes one field of the
    // state through the pointer, which is not null, as just checked, and
    // which this function's contract has pointing to a state it may read
    // and write. `hr_draw_close_path` is handed this function's own
    // arguments under what is its own contract.
    unsafe {
        if (*st).path_open != 0 {
            hr_draw_close_path(funcs, data, st);
        }
        (*st).current_x = to_x;
        (*st).current_y = to_y;
    }
}
/// Adds a line, opening a contour when needed.
///
/// # Safety
///
/// As [`hr_draw_move_to`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_line_to(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    to_x: f32,
    to_y: f32,
) {
    if st.is_null() {
        return;
    }
    // SAFETY: `start` and `emit_line` are handed this function's own
    // arguments under what is its own contract, with `st` not null, as
    // `emit_line` asks and as just checked. The two writes after them are
    // as in `start`: fields of the state, through a pointer that this
    // function's contract has pointing to a state it may write.
    unsafe {
        start(funcs, data, st);
        emit_line(funcs, data, st, to_x, to_y);
        (*st).current_x = to_x;
        (*st).current_y = to_y;
    }
}
/// Adds a cubic Bezier segment.
///
/// # Safety
///
/// As [`hr_draw_move_to`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_cubic_to(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    c1x: f32,
    c1y: f32,
    c2x: f32,
    c2y: f32,
    to_x: f32,
    to_y: f32,
) {
    if st.is_null() {
        return;
    }
    // SAFETY: `start` is handed this function's own arguments under what is
    // its own contract.
    unsafe { start(funcs, data, st) };
    // SAFETY: `funcs` is `NULL` or live by this function's contract, and `f`
    // is not used past this call.
    let f = unsafe { object::or_empty(funcs) };
    if let Some(cb) = &f.cubic_to {
        if let Some(call) = cb.func {
            // SAFETY: as in `emit_move`: the invariant on the callbacks has
            // `call` safe to call with `funcs`, which holds it, its own
            // `user_data`, and the draw data and state that this function's
            // contract has `data` and `st` be, `st` not being null.
            unsafe {
                call(
                    funcs,
                    data,
                    st,
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    to_x,
                    to_y,
                    cb.user_data,
                );
            }
        }
    }
    // SAFETY: as in `start`, these write two fields of the state through
    // `st`, which is not null, as checked above, and which this function's
    // contract has pointing to a state it may write.
    unsafe {
        (*st).current_x = to_x;
        (*st).current_y = to_y;
    }
}
/// Adds a quadratic segment, converting to cubic when no quadratic callback exists.
///
/// # Safety
///
/// As [`hr_draw_move_to`].
#[no_mangle]
pub unsafe extern "C" fn hr_draw_quadratic_to(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
    cx: f32,
    cy: f32,
    to_x: f32,
    to_y: f32,
) {
    if st.is_null() {
        return;
    }
    // SAFETY: `start` is handed this function's own arguments under what is
    // its own contract.
    unsafe { start(funcs, data, st) };
    // SAFETY: `funcs` is `NULL` or live by this function's contract, and `f`
    // is not used past this call.
    let f = unsafe { object::or_empty(funcs) };
    if let Some(cb) = &f.quadratic_to {
        if let Some(call) = cb.func {
            // SAFETY: as in `emit_move`: the invariant on the callbacks has
            // `call` safe to call with `funcs`, which holds it, its own
            // `user_data`, and the draw data and state that this function's
            // contract has `data` and `st` be, `st` not being null.
            unsafe { call(funcs, data, st, cx, cy, to_x, to_y, cb.user_data) }
        }
    } else {
        // SAFETY: `hr_draw_cubic_to` is handed this function's own `funcs`,
        // `data` and `st` under what is its own contract. The two `(*st)`
        // are as in `start`: fields of the state, read through a pointer
        // that is not null, as checked above, and that this function's
        // contract has pointing to a state it may read.
        unsafe {
            hr_draw_cubic_to(
                funcs,
                data,
                st,
                ((*st).current_x + 2.0 * cx) / 3.0,
                ((*st).current_y + 2.0 * cy) / 3.0,
                (to_x + 2.0 * cx) / 3.0,
                (to_y + 2.0 * cy) / 3.0,
                to_x,
                to_y,
            );
        }
    }
    // SAFETY: as in `start`, these write two fields of the state through
    // `st`, which is not null, as checked above, and which this function's
    // contract has pointing to a state it may write.
    unsafe {
        (*st).current_x = to_x;
        (*st).current_y = to_y;
    }
}
/// Closes a contour, emitting its final line before close when needed.
///
/// # Safety
///
/// As [`hr_draw_move_to`].
#[no_mangle]
#[allow(clippy::float_cmp)] // Exact equality determines whether a closing segment exists.
pub unsafe extern "C" fn hr_draw_close_path(
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    st: *mut hr_draw_state_t,
) {
    if st.is_null() {
        return;
    }
    // SAFETY: as in `start`, this reads a field of the state through `st`,
    // which is not null, as just checked, and which this function's
    // contract has pointing to a state it may read.
    if unsafe { (*st).path_open } != 0 {
        // SAFETY: as above, for four more fields.
        if unsafe { (*st).current_x != (*st).path_start_x || (*st).current_y != (*st).path_start_y }
        {
            // SAFETY: `hr_draw_line_to` is handed this function's own
            // arguments under what is its own contract, and the two fields
            // are read as above.
            unsafe { hr_draw_line_to(funcs, data, st, (*st).path_start_x, (*st).path_start_y) }
        }
        // SAFETY: `funcs` is `NULL` or live by this function's contract,
        // and `f` is not used past this call.
        let f = unsafe { object::or_empty(funcs) };
        if let Some(cb) = &f.close_path {
            if let Some(call) = cb.func {
                // SAFETY: as in `emit_move`: the invariant on the callbacks
                // has `call` safe to call with `funcs`, which holds it, its
                // own `user_data`, and the draw data and state that this
                // function's contract has `data` and `st` be, `st` not being
                // null.
                unsafe { call(funcs, data, st, cb.user_data) }
            }
        }
    }
    // SAFETY: `write` needs `st` to be valid for writes and aligned. It is
    // not null, as checked above, and this function's contract has it
    // pointing to a state this may write.
    unsafe { st.write(hr_draw_state_t::default()) };
}

/// Legacy callback, assumed to succeed.
pub type hr_font_draw_glyph_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        user_data: *mut c_void,
    ),
>;
/// Custom glyph-outline callback. Returns true if the glyph was drawn.
pub type hr_font_draw_glyph_or_fail_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        funcs: *mut hr_draw_funcs_t,
        draw_data: *mut c_void,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;
/// Sets a custom outline callback. Takes ownership of its data even when rejected.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live. Unless it is immutable, nothing else
/// may be using it while this runs.
///
/// `func`, if given, must be safe to call on any thread with a live font
/// that carries `ffuncs`, that font's data, draw funcs that are `NULL` or
/// live, draw data meant for their callbacks, and `user_data`. It may use
/// the draw funcs and the draw data only to draw with, on the thread it was
/// called on, one call at a time, and before it returns, and must not
/// modify the draw funcs or the font. `destroy`, if given, must be safe to
/// call once with `user_data`, on whichever thread releases the data and at
/// whichever point that is: during this call when `func` is `NULL` or
/// `ffuncs` refuses the change, and otherwise when the callback is replaced
/// or cleared, or `ffuncs` is freed.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_draw_glyph_func(
    ffuncs: *mut crate::hr_font_funcs_t,
    func: hr_font_draw_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: `set_callback` asks of `ffuncs` and of `destroy` what this
    // function's contract does, and of `store` that it hand back every
    // callback it takes out. This one takes out two and hands back both:
    // the one it replaces, and the one of the other kind, which it clears.
    unsafe {
        set_callback(ffuncs, func, user_data, destroy, |state, new| {
            let old = core::mem::replace(&mut state.draw_glyph_legacy, new);
            (old, state.draw_glyph.take())
        });
    }
}
/// Sets a custom outline callback returning success.
///
/// # Safety
///
/// As [`hr_font_funcs_set_draw_glyph_func`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_draw_glyph_or_fail_func(
    ffuncs: *mut crate::hr_font_funcs_t,
    func: hr_font_draw_glyph_or_fail_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: as in `hr_font_funcs_set_draw_glyph_func`, whose contract is
    // this function's own. Both callbacks this takes out are handed back.
    unsafe {
        set_callback(ffuncs, func, user_data, destroy, |state, new| {
            let old = core::mem::replace(&mut state.draw_glyph, new);
            (old, state.draw_glyph_legacy.take())
        });
    }
}
/// Draws an unhinted glyph using Skrifa, or the installed font callback.
/// Supported outlines are glyf, CFF, CFF2, and VARC. Returns false on invalid input.
///
/// # Safety
///
/// `font` and `funcs` must be `NULL` or live, and callbacks must not modify
/// either, nor a font that `font` is a sub-font of. `draw_data` must be draw
/// data meant for the callbacks of `funcs`.
#[no_mangle]
pub unsafe extern "C" fn hr_font_draw_glyph_or_fail(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    funcs: *mut hr_draw_funcs_t,
    draw_data: *mut c_void,
) -> hr_bool_t {
    // SAFETY: `as_ref` needs `font` to be null, or a live font that nothing
    // writes while `state` is used, which is until this returns. This
    // function's contract has `font` `NULL` or live and left alone by
    // callbacks, and whoever else writes a live object answers for no
    // reference to it being in use (see "Exclusive access" in `object`).
    let Some(state) = (unsafe { font.as_ref() }) else {
        return 0;
    };
    // SAFETY: `as_ref` needs the font's callbacks to be null, or a live
    // funcs object that nothing writes while `ffuncs` is used. A font owns
    // a reference to its callbacks, so they are live for as long as the
    // font is left alone, which this function's contract has it. As above,
    // whoever writes a live object answers for no reference to it being in
    // use.
    if let Some(ffuncs) = unsafe { state.funcs.as_ref() } {
        if let Some(cb) = &ffuncs.draw_glyph_legacy {
            if let Some(call) = cb.func {
                // SAFETY: `call` and `cb.user_data` were set together by
                // `hr_font_funcs_set_draw_glyph_func`, which alone fills
                // this slot. Its contract has `call` safe to call on any
                // thread with a live font that carries these callbacks,
                // that font's data, draw funcs that are `NULL` or live,
                // draw data meant for their callbacks, and that user data.
                // `font` is live, as `state` shows, and carries `ffuncs`
                // along with the data `callback_data` returns. `funcs` and
                // `draw_data` are as this function's contract has them.
                unsafe {
                    call(
                        font,
                        state.callback_data(),
                        glyph,
                        funcs,
                        draw_data,
                        cb.user_data,
                    );
                }
                return 1;
            }
        }
        if let Some(cb) = &ffuncs.draw_glyph {
            if let Some(call) = cb.func {
                // SAFETY: as above, with
                // `hr_font_funcs_set_draw_glyph_or_fail_func` the one that
                // fills this slot, under the same contract.
                return unsafe {
                    call(
                        font,
                        state.callback_data(),
                        glyph,
                        funcs,
                        draw_data,
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
        // SAFETY: `parent_draw` asks of `font`, `funcs` and `draw_data`
        // what this function's contract does, and for the parent of `font`,
        // which is what `state.parent` is. A font owns a reference to its
        // parent, so that is live for as long as `font` is left alone.
        return unsafe { parent_draw(font, state.parent, glyph, funcs, draw_data) };
    }
    // SAFETY: `hr_font_get_face` takes `NULL` or a live font, which this
    // function's contract has `font` be, and returns the face that font
    // owns a reference to. That face is live for as long as `font` is left
    // alone, so `as_ref` has what it needs of it until this returns, which
    // is as long as `render_face` is used: as above, whoever writes a live
    // object answers for no reference to it being in use.
    let Some(render_face) = (unsafe { crate::hr_font_get_face(font).as_ref() }) else {
        return 0;
    };
    // SAFETY: `render_face` is the face of a live font, borrowed just above
    // from the live pointer that font holds, and stays borrowed for as long
    // as the outlines are used.
    let Some(outlines) = (unsafe { crate::rendering::outlines(render_face) }) else {
        return 0;
    };
    let Some(outline) = outlines.get(read_fonts::types::GlyphId::new(glyph)) else {
        return 0;
    };
    // SAFETY: `font` is live by this function's contract. Its coordinates
    // stay put while they are copied: as above, whoever would change them
    // answers for the font not being in use.
    let coords = unsafe { crate::rendering::coords(font) };
    // SAFETY: `font` is live by this function's contract, as above.
    let (x, y) = unsafe { crate::rendering::scale(font) };
    let settings = skrifa::outline::DrawSettings::unhinted(
        skrifa::instance::Size::new(state.upem() as f32),
        coords.as_slice(),
    )
    .with_path_style(skrifa::outline::pen::PathStyle::HarfBuzz);
    // SAFETY: until it is dropped, which is when this returns, the pen needs
    // `funcs` to be `NULL` or live and unmodified, and `draw_data` to be what
    // its callbacks are called with. This function's contract has `funcs`
    // `NULL` or live and left alone by callbacks for the call, changing it
    // from elsewhere needs exclusive access, and `draw_data` is what the
    // caller gave to go with it.
    let mut pen = unsafe { crate::rendering::Pen::new(funcs, draw_data, x, y) };
    i32::from(outline.draw(settings, &mut pen).is_ok())
}
/// Compatibility entry point ignoring the success result.
///
/// # Safety
///
/// As [`hr_font_draw_glyph_or_fail`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_draw_glyph(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    funcs: *mut hr_draw_funcs_t,
    draw_data: *mut c_void,
) {
    // SAFETY: `hr_font_draw_glyph_or_fail` is handed this function's own
    // arguments under what is its own contract.
    unsafe { hr_font_draw_glyph_or_fail(font, glyph, funcs, draw_data) };
}

/// # Safety
///
/// As [`hr_font_draw_glyph_or_fail`], and `parent` must be the parent of
/// `font`: `NULL`, or live.
unsafe fn parent_draw(
    font: *mut hr_font_t,
    parent: *mut hr_font_t,
    glyph: hr_codepoint_t,
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
) -> hr_bool_t {
    use crate::rendering::Pen;
    use skrifa::outline::OutlinePen;
    /// # Safety
    ///
    /// `data` must point to a [`Pen`] that nothing else uses while this
    /// runs.
    unsafe extern "C" fn forward_move(
        _: *mut hr_draw_funcs_t,
        data: *mut c_void,
        _: *mut hr_draw_state_t,
        x: f32,
        y: f32,
        _: *mut c_void,
    ) {
        // SAFETY: making the reference needs `data` to be non-null and
        // aligned, and to point to a `Pen` that nothing else uses while the
        // reference is used, which is for this call. This function's
        // contract has just that.
        unsafe { &mut *data.cast::<Pen>() }.move_to(x, y);
    }
    /// # Safety
    ///
    /// As `forward_move`.
    unsafe extern "C" fn forward_line(
        _: *mut hr_draw_funcs_t,
        data: *mut c_void,
        _: *mut hr_draw_state_t,
        x: f32,
        y: f32,
        _: *mut c_void,
    ) {
        // SAFETY: as in `forward_move`, whose contract is this function's
        // own.
        unsafe { &mut *data.cast::<Pen>() }.line_to(x, y);
    }
    /// # Safety
    ///
    /// As `forward_move`.
    unsafe extern "C" fn forward_quad(
        _: *mut hr_draw_funcs_t,
        data: *mut c_void,
        _: *mut hr_draw_state_t,
        a: f32,
        b: f32,
        x: f32,
        y: f32,
        _: *mut c_void,
    ) {
        // SAFETY: as in `forward_move`, whose contract is this function's
        // own.
        unsafe { &mut *data.cast::<Pen>() }.quad_to(a, b, x, y);
    }
    /// # Safety
    ///
    /// As `forward_move`.
    unsafe extern "C" fn forward_curve(
        _: *mut hr_draw_funcs_t,
        data: *mut c_void,
        _: *mut hr_draw_state_t,
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        x: f32,
        y: f32,
        _: *mut c_void,
    ) {
        // SAFETY: as in `forward_move`, whose contract is this function's
        // own.
        unsafe { &mut *data.cast::<Pen>() }.curve_to(a, b, c, d, x, y);
    }
    /// # Safety
    ///
    /// As `forward_move`.
    unsafe extern "C" fn forward_close(
        _: *mut hr_draw_funcs_t,
        data: *mut c_void,
        _: *mut hr_draw_state_t,
        _: *mut c_void,
    ) {
        // SAFETY: as in `forward_move`, whose contract is this function's
        // own.
        unsafe { &mut *data.cast::<Pen>() }.close();
    }
    let forwarding = hr_draw_funcs_create();
    // SAFETY: each of the five calls installs one forwarder on `forwarding`
    // under the contract of `hr_draw_funcs_set_move_to_func`.
    // - `forwarding` is live and takes changes, having just been created,
    //   and nothing else has its pointer yet.
    // - A forwarder uses nothing but its draw data, and is safe to call on
    //   any thread with draw data that points to a `Pen` nothing else is
    //   using. That is what draw data meant for `forwarding` is.
    // - There is no user data, and no destructor to call.
    unsafe {
        hr_draw_funcs_set_move_to_func(forwarding, Some(forward_move), core::ptr::null_mut(), None);
        hr_draw_funcs_set_line_to_func(forwarding, Some(forward_line), core::ptr::null_mut(), None);
        hr_draw_funcs_set_quadratic_to_func(
            forwarding,
            Some(forward_quad),
            core::ptr::null_mut(),
            None,
        );
        hr_draw_funcs_set_cubic_to_func(
            forwarding,
            Some(forward_curve),
            core::ptr::null_mut(),
            None,
        );
        hr_draw_funcs_set_close_path_func(
            forwarding,
            Some(forward_close),
            core::ptr::null_mut(),
            None,
        );
    }
    // SAFETY: `font` is `NULL` or live by this function's contract.
    let (sx, sy) = unsafe { crate::rendering::scale(font) };
    // SAFETY: `parent` is `NULL` or live by this function's contract.
    let (px, py) = unsafe { crate::rendering::scale(parent) };
    // SAFETY: until it is dropped, which is when this returns, the pen needs
    // `funcs` to be `NULL` or live and unmodified, and `data` to be what its
    // callbacks are called with. This function's contract has both so for
    // the call.
    let mut pen = unsafe {
        Pen::new(
            funcs,
            data,
            if px == 0.0 { 0.0 } else { sx / px },
            if py == 0.0 { 0.0 } else { sy / py },
        )
    };
    // Stream each segment through the pen as the parent emits it: an outline
    // can be far longer than the font that produced it, so it is never held.
    //
    // SAFETY: `hr_font_draw_glyph_or_fail` needs a font and draw funcs that
    // are `NULL` or live and that callbacks leave alone, along with the
    // fonts above that font, and draw data meant for the funcs' callbacks.
    // - `parent` is `NULL` or live by this function's contract, which also
    //   has callbacks leave alone every font that `font` is a sub-font of:
    //   `parent`, and whatever is above it.
    // - `forwarding` is live, since this function holds the one reference
    //   to it until it is destroyed below. The only callback of the
    //   caller's that is handed it is the draw-glyph callback of a font
    //   above, which the contract of its setter has not modify the draw
    //   funcs it is given.
    // - The draw data points to `pen`, which lives until this returns and
    //   which this function does not touch while the call runs. Whatever is
    //   handed `forwarding` and the pen draws with them on this thread, one
    //   call at a time, and is done before it returns: a `Pen` does, and the
    //   setter of a draw-glyph callback requires it of one. So nothing else
    //   is using the pen when a forwarder runs, which is all they need.
    let result =
        unsafe { hr_font_draw_glyph_or_fail(parent, glyph, forwarding, (&raw mut pen).cast()) };
    // SAFETY: `forwarding` is live, and this function owns the reference
    // that `hr_draw_funcs_create` returned for it, which it gives up here.
    unsafe { hr_draw_funcs_destroy(forwarding) };
    result
}
