//! Caller-supplied font callbacks, mirroring the parts of HarfBuzz's
//! `hb-font.h` that HarfRust's [`FontFuncs`] trait can back.
//!
//! A callback that is not set falls through to HarfRust's own implementation,
//! which reads the font's tables. This matches HarfBuzz's parent-funcs
//! chaining for the common case of overriding one or two callbacks.

use core::ffi::c_void;
use core::ptr;
use std::sync::OnceLock;

use harfrust::font::{BuiltinFontFuncs, FontFuncs};
use harfrust::{GlyphExtents, GlyphId};

use crate::common::{hr_bool_t, hr_codepoint_t, hr_glyph_extents_t, hr_position_t};
use crate::font::hr_font_t;
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

/// Maps a Unicode scalar value to a glyph. Returns false if there is none.
pub type hr_font_get_nominal_glyph_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        unicode: hr_codepoint_t,
        glyph: *mut hr_codepoint_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;

/// Maps a Unicode scalar value and variation selector to a glyph.
pub type hr_font_get_variation_glyph_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        unicode: hr_codepoint_t,
        variation_selector: hr_codepoint_t,
        glyph: *mut hr_codepoint_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;

/// Returns a glyph's advance along the current direction.
pub type hr_font_get_glyph_advance_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        user_data: *mut c_void,
    ) -> hr_position_t,
>;

/// Returns a glyph's origin along the current direction.
pub type hr_font_get_glyph_origin_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        x: *mut hr_position_t,
        y: *mut hr_position_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;

/// Returns a glyph's ink extents.
pub type hr_font_get_glyph_extents_func_t = Option<
    unsafe extern "C" fn(
        font: *mut hr_font_t,
        font_data: *mut c_void,
        glyph: hr_codepoint_t,
        extents: *mut hr_glyph_extents_t,
        user_data: *mut c_void,
    ) -> hr_bool_t,
>;

/// One callback and the data it was registered with.
struct Callback<F> {
    func: F,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
}

impl<F> Drop for Callback<F> {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: `destroy` was supplied alongside `user_data`.
            unsafe { destroy(self.user_data) };
        }
    }
}

/// Pairs a callback with the data it was given, or releases that data when
/// there is no callback to own it.
///
/// Every setter takes ownership of `user_data`, including the ones clearing a
/// callback: HarfBuzz runs the destructor either way, and a caller that had
/// to know which calls take ownership could not free anything safely.
fn callback_taking<F>(
    func: Option<F>,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) -> Option<Callback<Option<F>>> {
    if func.is_none() {
        if let Some(destroy) = destroy {
            // SAFETY: `destroy` was supplied alongside `user_data`.
            unsafe { destroy(user_data) };
        }
        return None;
    }
    Some(Callback {
        func,
        user_data,
        destroy,
    })
}

/// A set of font callbacks.
#[derive(Default)]
pub struct hr_font_funcs_t {
    header: ObjectHeader,
    nominal_glyph: Option<Callback<hr_font_get_nominal_glyph_func_t>>,
    variation_glyph: Option<Callback<hr_font_get_variation_glyph_func_t>>,
    h_advance: Option<Callback<hr_font_get_glyph_advance_func_t>>,
    v_advance: Option<Callback<hr_font_get_glyph_advance_func_t>>,
    v_origin: Option<Callback<hr_font_get_glyph_origin_func_t>>,
    extents: Option<Callback<hr_font_get_glyph_extents_func_t>>,
}

// SAFETY: `hr_font_set_funcs` documents that callbacks and their user data
// must be safe to use from any thread.
unsafe impl Send for hr_font_funcs_t {}
unsafe impl Sync for hr_font_funcs_t {}

static EMPTY_FONT_FUNCS: OnceLock<Empty<hr_font_funcs_t>> = OnceLock::new();

impl Object for hr_font_funcs_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_FONT_FUNCS
            .get_or_init(|| {
                Empty::new(hr_font_funcs_t {
                    header: ObjectHeader::immortal(),
                    ..Default::default()
                })
            })
            .get()
    }
}

/// Creates an empty set of font callbacks.
#[no_mangle]
pub extern "C" fn hr_font_funcs_create() -> *mut hr_font_funcs_t {
    object::create(hr_font_funcs_t {
        header: ObjectHeader::new(),
        ..Default::default()
    })
}

/// Returns the immortal empty set of font callbacks.
#[no_mangle]
pub extern "C" fn hr_font_funcs_get_empty() -> *mut hr_font_funcs_t {
    hr_font_funcs_t::empty()
}

/// Increments the reference count of a set of font callbacks.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_reference(
    ffuncs: *mut hr_font_funcs_t,
) -> *mut hr_font_funcs_t {
    unsafe { object::reference(ffuncs) }
}

/// Decrements the reference count, freeing at zero and running every
/// registered destroy callback.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the caller must own the reference.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_destroy(ffuncs: *mut hr_font_funcs_t) {
    unsafe { object::destroy(ffuncs) };
}

/// Attaches user data to a set of font callbacks.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_user_data(
    ffuncs: *mut hr_font_funcs_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(ffuncs, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data attached to a set of font callbacks.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_get_user_data(
    ffuncs: *mut hr_font_funcs_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(ffuncs, key) }
}

/// Marks a set of font callbacks immutable.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_make_immutable(ffuncs: *mut hr_font_funcs_t) {
    unsafe { object::make_immutable(ffuncs) };
}

/// Returns whether a set of font callbacks has been marked immutable.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_is_immutable(ffuncs: *mut hr_font_funcs_t) -> hr_bool_t {
    unsafe { object::is_immutable(ffuncs.cast_const()) }.into()
}

/// Sets the callback mapping a Unicode scalar value to a glyph.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_nominal_glyph_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_nominal_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.nominal_glyph = callback_taking(func, user_data, destroy);
}

/// Sets the callback mapping a Unicode scalar value and variation selector
/// to a glyph.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_variation_glyph_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_variation_glyph_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.variation_glyph = callback_taking(func, user_data, destroy);
}

/// Sets the callback returning a glyph's horizontal advance.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_glyph_h_advance_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_glyph_advance_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.h_advance = callback_taking(func, user_data, destroy);
}

/// Sets the callback returning a glyph's vertical advance.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_glyph_v_advance_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_glyph_advance_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.v_advance = callback_taking(func, user_data, destroy);
}

/// Sets the callback returning a glyph's vertical origin.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_glyph_v_origin_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_glyph_origin_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.v_origin = callback_taking(func, user_data, destroy);
}

/// Sets the callback returning a glyph's ink extents.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which the parent font answers, or
/// nothing does: no glyph, no extents, and an advance of the font's own
/// scale, which is what HarfBuzz answers with. Setting a
/// callback on an immutable object is ignored, and releases `user_data`
/// immediately.
///
/// # Safety
///
/// `ffuncs` must be `NULL` or live, and the callback must be safe to call with
/// `user_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_funcs_set_glyph_extents_func(
    ffuncs: *mut hr_font_funcs_t,
    func: hr_font_get_glyph_extents_func_t,
    user_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(ffuncs) = (unsafe { object::as_mutable(ffuncs) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(user_data) };
        }
        return;
    };
    ffuncs.extents = callback_taking(func, user_data, destroy);
}

/// Bridges a [`hr_font_funcs_t`] into HarfRust's [`FontFuncs`] trait for the
/// duration of one shaping call.
///
/// A funcs object is authoritative: once one is installed on a font, every
/// callback comes from it, and a callback that was never set yields the same
/// "not available" answer HarfBuzz's nil implementation gives — no glyph, a
/// zero advance, a zero origin, no extents. HarfRust's own table-driven
/// implementation is used only when a font has no funcs object at all.
pub(crate) struct FontFuncsAdapter<'a> {
    /// The C API does not support callbacks mutating or freeing their font, so
    /// the font itself keeps these values alive for the duration of shaping.
    state: &'a hr_font_t,
    font: *mut hr_font_t,
}

/// What came back: either a value, or the fact that nobody in the chain
/// carries callbacks and the font's own tables should answer.
pub(crate) enum Answer<T> {
    Builtin,
    Value(T),
}

/// Where a callback was found, and what it expects to be handed.
enum Resolved<'c, F> {
    /// No font in the chain carries callbacks, so the font's own tables
    /// answer.
    Builtin,
    /// Callbacks were installed somewhere, and none of them answer for this.
    Missing,
    Found {
        font: *mut hr_font_t,
        data: *mut c_void,
        callback: &'c Callback<F>,
        /// The scale the answer will be in, against this font's own.
        scale: (i32, i32),
    },
}

/// Finds the nearest font in the chain whose callbacks answer for one field.
///
/// A funcs object that does not carry the callback delegates to the parent,
/// as HarfBuzz's do, and so does a font carrying no callbacks at all. A chain
/// with none anywhere falls to the built-in implementation, while one that
/// has callbacks but not this one reports nothing available.
macro_rules! resolve {
    ($adapter:expr, $field:ident) => {{
        // Reborrowed, so that a caller holding `&mut self` keeps it.
        let adapter: &FontFuncsAdapter<'_> = &*$adapter;
        let mut font = adapter.font;
        let mut state: &hr_font_t = adapter.state;
        let mut installed = false;
        loop {
            // SAFETY: each font owns its reference to its callbacks and to
            // its parent, and outlives this adapter.
            if let Some(funcs) = unsafe { state.funcs.as_ref() } {
                installed = true;
                if let Some(callback) = funcs.$field.as_ref() {
                    break Resolved::Found {
                        font,
                        data: state.callback_data(),
                        callback,
                        scale: adapter.scale_from(state),
                    };
                }
            }
            match unsafe { state.parent.as_ref() } {
                Some(parent) => {
                    font = state.parent;
                    state = parent;
                }
                None => {
                    break if installed {
                        Resolved::Missing
                    } else {
                        Resolved::Builtin
                    }
                }
            }
        }
    }};
}

/// Rescales a distance answered in `from` units into `into` units, which is
/// what a parent's answer needs when the sub-font asking is a different size.
fn rescale(value: i32, from: i32, into: i32) -> i32 {
    if from == into || from == 0 {
        return value;
    }
    ((i64::from(value) * i64::from(into)) / i64::from(from)) as i32
}

impl<'a> FontFuncsAdapter<'a> {
    pub(crate) fn new(font: *mut hr_font_t, state: &'a hr_font_t) -> Self {
        Self { state, font }
    }

    fn funcs(&self) -> Option<&hr_font_funcs_t> {
        // SAFETY: `state` owns the reference for as long as this adapter lives.
        unsafe { self.state.funcs.as_ref() }
    }

    fn font_data(&self) -> *mut c_void {
        self.state.callback_data()
    }

    /// How much larger this font is than `ancestor`, which is what a distance
    /// coming back from the ancestor's callbacks has to be multiplied by.
    fn scale_from(&self, ancestor: &hr_font_t) -> (i32, i32) {
        (ancestor.x_scale, ancestor.y_scale)
    }

    /// What the installed nominal-glyph callback answers, or `None` when
    /// there is no such callback to ask.
    ///
    /// Shaping and the public getters both go through here, so that a font
    /// answers the same whichever of them is asking.
    pub(crate) fn call_nominal_glyph(&self, c: u32) -> Option<Answer<hr_codepoint_t>> {
        let (font, data, cb) = match resolve!(self, nominal_glyph) {
            Resolved::Builtin => return Some(Answer::Builtin),
            Resolved::Missing => return None,
            Resolved::Found {
                font,
                data,
                callback,
                ..
            } => (font, data, callback),
        };
        let func = cb.func?;
        let mut glyph: hr_codepoint_t = 0;
        // SAFETY: the callback was registered by the caller for this purpose.
        let found = unsafe { func(font, data, c, ptr::from_mut(&mut glyph), cb.user_data) };
        (found != 0).then_some(Answer::Value(glyph))
    }

    /// As [`FontFuncsAdapter::call_nominal_glyph`], for a variation
    /// selector.
    pub(crate) fn call_variation_glyph(&self, c: u32, vs: u32) -> Option<Answer<hr_codepoint_t>> {
        let (font, data, cb) = match resolve!(self, variation_glyph) {
            Resolved::Builtin => return Some(Answer::Builtin),
            Resolved::Missing => return None,
            Resolved::Found {
                font,
                data,
                callback,
                ..
            } => (font, data, callback),
        };
        let func = cb.func?;
        let mut glyph: hr_codepoint_t = 0;
        // SAFETY: as above.
        let found = unsafe { func(font, data, c, vs, ptr::from_mut(&mut glyph), cb.user_data) };
        (found != 0).then_some(Answer::Value(glyph))
    }

    /// What a glyph's extents are, through whichever callback answers.
    pub(crate) fn call_extents(&self, glyph: u32) -> Option<Answer<hr_glyph_extents_t>> {
        let (font, data, cb, scale) = match resolve!(self, extents) {
            Resolved::Builtin => return Some(Answer::Builtin),
            Resolved::Missing => return None,
            Resolved::Found {
                font,
                data,
                callback,
                scale,
            } => (font, data, callback, scale),
        };
        let func = cb.func?;
        let mut extents = hr_glyph_extents_t::default();
        // SAFETY: as above.
        let found = unsafe { func(font, data, glyph, ptr::from_mut(&mut extents), cb.user_data) };
        if found == 0 {
            return None;
        }
        extents.x_bearing = rescale(extents.x_bearing, scale.0, self.state.x_scale);
        extents.width = rescale(extents.width, scale.0, self.state.x_scale);
        extents.y_bearing = rescale(extents.y_bearing, scale.1, self.state.y_scale);
        extents.height = rescale(extents.height, scale.1, self.state.y_scale);
        Some(Answer::Value(extents))
    }
}

impl FontFuncs for FontFuncsAdapter<'_> {
    fn nominal_glyph(&mut self, builtin: &BuiltinFontFuncs, c: u32) -> Option<GlyphId> {
        match self.call_nominal_glyph(c)? {
            Answer::Builtin => builtin.nominal_glyph(c),
            Answer::Value(glyph) => Some(GlyphId::from(glyph)),
        }
    }

    fn variant_glyph(&mut self, builtin: &BuiltinFontFuncs, c: u32, vs: u32) -> Option<GlyphId> {
        match self.call_variation_glyph(c, vs)? {
            Answer::Builtin => builtin.variant_glyph(c, vs),
            Answer::Value(glyph) => Some(GlyphId::from(glyph)),
        }
    }

    fn advance_width(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        let (font, data, cb, scale) = match resolve!(self, h_advance) {
            Resolved::Builtin => return builtin.advance_width(glyph),
            // Nothing answers, so every glyph is as wide as the font is
            // tall. HarfBuzz's own answer, and not zero.
            Resolved::Missing => return self.state.x_scale,
            Resolved::Found {
                font,
                data,
                callback,
                scale,
            } => (font, data, callback, scale),
        };
        let Some(func) = cb.func else {
            return 0;
        };
        // SAFETY: as above.
        let advance = unsafe { func(font, data, glyph.to_u32(), cb.user_data) };
        rescale(advance, scale.0, self.state.x_scale)
    }

    fn advance_height(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        let (font, data, cb, scale) = match resolve!(self, v_advance) {
            Resolved::Builtin => return builtin.advance_height(glyph),
            // As above, downwards.
            Resolved::Missing => return -self.state.y_scale,
            Resolved::Found {
                font,
                data,
                callback,
                scale,
            } => (font, data, callback, scale),
        };
        let Some(func) = cb.func else {
            return 0;
        };
        // SAFETY: as above.
        let advance = unsafe { func(font, data, glyph.to_u32(), cb.user_data) };
        rescale(advance, scale.1, self.state.y_scale)
    }

    fn vertical_origin(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> (i32, i32) {
        let (font, data, cb, scale) = match resolve!(self, v_origin) {
            Resolved::Builtin => return builtin.vertical_origin(glyph),
            Resolved::Missing => return (0, 0),
            Resolved::Found {
                font,
                data,
                callback,
                scale,
            } => (font, data, callback, scale),
        };
        let Some(func) = cb.func else {
            return (0, 0);
        };
        let (mut x, mut y) = (0, 0);
        // SAFETY: as above.
        let found = unsafe {
            func(
                font,
                data,
                glyph.to_u32(),
                ptr::from_mut(&mut x),
                ptr::from_mut(&mut y),
                cb.user_data,
            )
        };
        if found == 0 {
            return (0, 0);
        }
        (
            rescale(x, scale.0, self.state.x_scale),
            rescale(y, scale.1, self.state.y_scale),
        )
    }

    fn extents(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> Option<GlyphExtents> {
        match self.call_extents(glyph.to_u32())? {
            Answer::Builtin => builtin.extents(glyph),
            Answer::Value(extents) => Some(GlyphExtents {
                x_bearing: extents.x_bearing,
                y_bearing: extents.y_bearing,
                width: extents.width,
                height: extents.height,
            }),
        }
    }
}
