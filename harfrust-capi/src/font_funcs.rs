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
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Object, ObjectHeader};

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

static EMPTY_FONT_FUNCS: OnceLock<usize> = OnceLock::new();

impl Object for hr_font_funcs_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        let addr = *EMPTY_FONT_FUNCS.get_or_init(|| {
            Box::into_raw(Box::new(hr_font_funcs_t {
                header: ObjectHeader::immortal(),
                ..Default::default()
            })) as usize
        });
        addr as *mut Self
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
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.nominal_glyph = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Sets the callback mapping a Unicode scalar value and variation selector
/// to a glyph.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.variation_glyph = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Sets the callback returning a glyph's horizontal advance.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.h_advance = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Sets the callback returning a glyph's vertical advance.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.v_advance = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Sets the callback returning a glyph's vertical origin.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.v_origin = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Sets the callback returning a glyph's ink extents.
///
/// Takes ownership of `user_data`, releasing it through `destroy` when the
/// callback is replaced or the funcs object is freed. Passing a `NULL` callback
/// clears any previously set one, after which it reports nothing available
/// rather than falling back to HarfRust's own implementation. Setting a
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
    ffuncs.extents = func.map(|func| Callback {
        func: Some(func),
        user_data,
        destroy,
    });
}

/// Bridges a [`hr_font_funcs_t`] into HarfRust's [`FontFuncs`] trait for the
/// duration of one shaping call.
///
/// A funcs object is authoritative: once one is installed on a font, every
/// callback comes from it, and a callback that was never set yields the same
/// "not available" answer HarfBuzz's nil implementation gives — no glyph, a
/// zero advance, a zero origin, no extents. HarfRust's own table-driven
/// implementation is used only when a font has no funcs object at all.
pub(crate) struct FontFuncsAdapter {
    pub(crate) funcs: *mut hr_font_funcs_t,
    pub(crate) font: *mut hr_font_t,
    pub(crate) font_data: *mut c_void,
}

impl FontFuncsAdapter {
    fn funcs(&self) -> Option<&hr_font_funcs_t> {
        // SAFETY: the font owns a reference to this object for the whole call.
        unsafe { self.funcs.as_ref() }
    }
}

impl FontFuncs for FontFuncsAdapter {
    fn nominal_glyph(&mut self, _builtin: &BuiltinFontFuncs, c: u32) -> Option<GlyphId> {
        let cb = self.funcs()?.nominal_glyph.as_ref()?;
        let func = cb.func?;
        let mut glyph: hr_codepoint_t = 0;
        // SAFETY: the callback was registered by the caller for this purpose.
        let found = unsafe {
            func(
                self.font,
                self.font_data,
                c,
                ptr::from_mut(&mut glyph),
                cb.user_data,
            )
        };
        (found != 0).then(|| GlyphId::from(glyph))
    }

    fn variant_glyph(&mut self, _builtin: &BuiltinFontFuncs, c: u32, vs: u32) -> Option<GlyphId> {
        let cb = self.funcs()?.variation_glyph.as_ref()?;
        let func = cb.func?;
        let mut glyph: hr_codepoint_t = 0;
        // SAFETY: as above.
        let found = unsafe {
            func(
                self.font,
                self.font_data,
                c,
                vs,
                ptr::from_mut(&mut glyph),
                cb.user_data,
            )
        };
        (found != 0).then(|| GlyphId::from(glyph))
    }

    fn advance_width(&mut self, _builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        let Some(cb) = self.funcs().and_then(|f| f.h_advance.as_ref()) else {
            return 0;
        };
        let Some(func) = cb.func else {
            return 0;
        };
        // SAFETY: as above.
        unsafe { func(self.font, self.font_data, glyph.to_u32(), cb.user_data) }
    }

    fn advance_height(&mut self, _builtin: &BuiltinFontFuncs, glyph: GlyphId) -> i32 {
        let Some(cb) = self.funcs().and_then(|f| f.v_advance.as_ref()) else {
            return 0;
        };
        let Some(func) = cb.func else {
            return 0;
        };
        // SAFETY: as above.
        unsafe { func(self.font, self.font_data, glyph.to_u32(), cb.user_data) }
    }

    fn vertical_origin(&mut self, _builtin: &BuiltinFontFuncs, glyph: GlyphId) -> (i32, i32) {
        let Some(cb) = self.funcs().and_then(|f| f.v_origin.as_ref()) else {
            return (0, 0);
        };
        let Some(func) = cb.func else {
            return (0, 0);
        };
        let (mut x, mut y) = (0, 0);
        // SAFETY: as above.
        let found = unsafe {
            func(
                self.font,
                self.font_data,
                glyph.to_u32(),
                ptr::from_mut(&mut x),
                ptr::from_mut(&mut y),
                cb.user_data,
            )
        };
        if found == 0 {
            return (0, 0);
        }
        (x, y)
    }

    fn extents(&mut self, _builtin: &BuiltinFontFuncs, glyph: GlyphId) -> Option<GlyphExtents> {
        let cb = self.funcs()?.extents.as_ref()?;
        let func = cb.func?;
        let mut extents = hr_glyph_extents_t::default();
        // SAFETY: as above.
        let found = unsafe {
            func(
                self.font,
                self.font_data,
                glyph.to_u32(),
                ptr::from_mut(&mut extents),
                cb.user_data,
            )
        };
        (found != 0).then_some(GlyphExtents {
            x_bearing: extents.x_bearing,
            y_bearing: extents.y_bearing,
            width: extents.width,
            height: extents.height,
        })
    }
}
