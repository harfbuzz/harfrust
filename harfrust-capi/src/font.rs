//! Fonts: a face together with a size and variation settings. Mirrors the
//! shaping-relevant half of HarfBuzz's `hb-font.h`.

use core::ffi::{c_int, c_uint, c_void};
use std::sync::{Arc, OnceLock};

use harfrust::font::{FontInstance, FontVariation, NormalizedCoord};
use harfrust::Shaper;
use read_fonts::TableProvider;

use crate::common::{hr_bool_t, hr_codepoint_t, hr_variation_t};
use crate::face::hr_face_t;
use crate::font_funcs::hr_font_funcs_t;
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

/// The `font_data` a caller attached along with a set of callbacks.
///
/// Shared because a sub-font inherits its parent's. Whoever lets go of the
/// last reference runs the destroy callback, so replacing a font's callbacks
/// cannot pull the data out from under a sub-font.
pub(crate) struct FontData {
    pub(crate) data: *mut c_void,
    destroy: hr_destroy_func_t,
}

// SAFETY: `hr_font_set_funcs` documents that the callbacks and their data must
// be safe to use, and to release, from any thread.
unsafe impl Send for FontData {}
unsafe impl Sync for FontData {}

impl Drop for FontData {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: `destroy` was supplied alongside `data`.
            unsafe { destroy(self.data) };
        }
    }
}

/// A prepared shaper and the stable allocation it borrows.
///
/// Keeping these in one private owner makes replacing the instance drop the
/// shaper first. The explicit `Drop` preserves that invariant if the fields
/// are ever reordered.
struct PreparedFont {
    shaper: Option<Shaper<'static>>,
    builtin_shaper: Option<OnceLock<Shaper<'static>>>,
    instance: Box<FontInstance>,
}

impl PreparedFont {
    fn new(instance: FontInstance) -> Self {
        let instance = Box::new(instance);
        let shaper = Shaper::from_font_instance(&instance).map(|shaper| {
            // SAFETY: the shaper borrows the allocation owned by `instance`,
            // which is stable across moving the Box. `Drop` clears the shaper
            // before that allocation is released.
            unsafe { core::mem::transmute::<Shaper<'_>, Shaper<'static>>(shaper) }
        });
        Self {
            shaper,
            builtin_shaper: Some(OnceLock::new()),
            instance,
        }
    }

    fn shaper(&self, preload_builtin_data: bool) -> Option<&Shaper<'static>> {
        let shaper = self.shaper.as_ref()?;
        if !preload_builtin_data {
            return Some(shaper);
        }
        let cache = self.builtin_shaper.as_ref()?;
        Some(cache.get_or_init(|| {
            let mut shaper = shaper.clone();
            shaper.preload_builtin_font_data();
            shaper
        }))
    }
}

impl Drop for PreparedFont {
    fn drop(&mut self) {
        self.shaper = None;
        self.builtin_shaper = None;
    }
}

/// A font: a face with a scale, an optional point size, and variation
/// settings applied.
pub struct hr_font_t {
    header: ObjectHeader,
    /// Owned reference to the face this font draws from.
    face: *mut hr_face_t,
    /// `None` only for the immortal empty font.
    prepared: Option<PreparedFont>,
    pub(crate) x_scale: c_int,
    pub(crate) y_scale: c_int,
    pub(crate) ptem: f32,
    /// Normalized coordinates in HarfBuzz's representation: 2.14 values widened
    /// to `int`, so `hr_font_get_var_coords_normalized` can hand back a
    /// pointer directly.
    coords: Vec<c_int>,
    /// Owned reference to the callbacks, or null for the built-in ones.
    pub(crate) funcs: *mut hr_font_funcs_t,
    pub(crate) font_data: Option<Arc<FontData>>,
    /// Owned reference to the parent, for fonts made by
    /// [`hr_font_create_sub_font`].
    parent: *mut hr_font_t,
}

impl hr_font_t {
    pub(crate) fn face(&self) -> *mut hr_face_t {
        self.face
    }

    pub(crate) fn instance(&self) -> Option<&FontInstance> {
        self.prepared.as_ref().map(|prepared| &*prepared.instance)
    }

    pub(crate) fn shaper(&self) -> Option<&Shaper<'static>> {
        self.prepared
            .as_ref()
            .and_then(|prepared| prepared.shaper(self.funcs.is_null()))
    }

    /// Rebuilds the font instance after a change to variation settings, and
    /// refreshes the normalized coordinate mirror.
    fn set_instance(&mut self, instance: FontInstance) {
        let coords = instance
            .normalized_coords()
            .iter()
            .map(|coord| c_int::from(coord.to_bits()))
            .collect();
        let prepared = PreparedFont::new(instance);
        self.coords = coords;
        self.prepared = Some(prepared);
    }
}

impl Drop for hr_font_t {
    fn drop(&mut self) {
        self.prepared = None;
        // SAFETY: this font owns one reference to each of these.
        unsafe {
            object::destroy(self.funcs);
            object::destroy(self.parent);
            object::destroy(self.face);
        }
    }
}

static EMPTY_FONT: OnceLock<Empty<hr_font_t>> = OnceLock::new();

impl Object for hr_font_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_FONT
            .get_or_init(|| {
                Empty::new(hr_font_t {
                    header: ObjectHeader::immortal(),
                    face: hr_face_t::empty(),
                    prepared: None,
                    x_scale: 0,
                    y_scale: 0,
                    ptem: 0.0,
                    coords: Vec::new(),
                    funcs: core::ptr::null_mut(),
                    font_data: None,
                    parent: core::ptr::null_mut(),
                })
            })
            .get()
    }
}

/// Creates a font over a face.
///
/// The font takes its own reference to the face. Its scale starts at the
/// face's units per em, so positions come out in font units until
/// [`hr_font_set_scale`] says otherwise.
///
/// Never returns `NULL`; a face with no font data yields the empty font.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_font_create(face: *mut hr_face_t) -> *mut hr_font_t {
    let Some(face_ref) = (unsafe { face.as_ref() }) else {
        return hr_font_t::empty();
    };
    let Some(font) = face_ref.font() else {
        return hr_font_t::empty();
    };
    let upem = font
        .tables()
        .head()
        .map_or(1000, |head| c_int::from(head.units_per_em()));
    let owned_face = unsafe { object::reference(face) };
    let mut this = hr_font_t {
        header: ObjectHeader::new(),
        face: owned_face,
        prepared: None,
        x_scale: upem,
        y_scale: upem,
        ptem: 0.0,
        coords: Vec::new(),
        funcs: core::ptr::null_mut(),
        font_data: None,
        parent: core::ptr::null_mut(),
    };
    this.set_instance(FontInstance::builder(font).build());
    object::create(this)
}

/// Creates a font that starts out as a copy of `parent`.
///
/// The sub-font holds a reference to its parent and inherits its scale, point
/// size, variation settings and callbacks; changing the sub-font afterwards
/// does not affect the parent.
///
/// # Safety
///
/// `parent` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_create_sub_font(parent: *mut hr_font_t) -> *mut hr_font_t {
    let Some(parent_ref) = (unsafe { parent.as_ref() }) else {
        return hr_font_t::empty();
    };
    let Some(instance) = parent_ref.instance() else {
        return hr_font_t::empty();
    };
    let mut this = hr_font_t {
        header: ObjectHeader::new(),
        face: unsafe { object::reference(parent_ref.face) },
        prepared: None,
        x_scale: parent_ref.x_scale,
        y_scale: parent_ref.y_scale,
        ptem: parent_ref.ptem,
        coords: Vec::new(),
        funcs: unsafe { object::reference(parent_ref.funcs) },
        // Shared, so that replacing the parent's callbacks does not release
        // data this font is still using.
        font_data: parent_ref.font_data.clone(),
        parent: unsafe { object::reference(parent) },
    };
    this.set_instance(
        FontInstance::builder(instance.font())
            .normalized_coords(instance.normalized_coords().iter().copied())
            .build(),
    );
    object::create(this)
}

/// Returns the immortal empty font.
#[no_mangle]
pub extern "C" fn hr_font_get_empty() -> *mut hr_font_t {
    hr_font_t::empty()
}

/// Increments a font's reference count.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_reference(font: *mut hr_font_t) -> *mut hr_font_t {
    unsafe { object::reference(font) }
}

/// Decrements a font's reference count, freeing it at zero.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and the caller must own the
/// reference being released.
#[no_mangle]
pub unsafe extern "C" fn hr_font_destroy(font: *mut hr_font_t) {
    unsafe { object::destroy(font) };
}

/// Attaches user data to a font.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_user_data(
    font: *mut hr_font_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(font, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data previously attached to a font.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_user_data(
    font: *mut hr_font_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(font, key) }
}

/// Marks a font immutable.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_make_immutable(font: *mut hr_font_t) {
    unsafe { object::make_immutable(font) };
}

/// Returns whether a font has been marked immutable.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_is_immutable(font: *mut hr_font_t) -> hr_bool_t {
    unsafe { object::is_immutable(font.cast_const()) }.into()
}

/// Returns the face a font was created over, without taking a reference.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_face(font: *mut hr_font_t) -> *mut hr_face_t {
    unsafe { object::or_empty(font.cast_const()) }.face()
}

/// Returns a font's parent, or `NULL` if it is not a sub-font.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_parent(font: *mut hr_font_t) -> *mut hr_font_t {
    unsafe { object::or_empty(font.cast_const()) }.parent
}

/// Sets a font's scale.
///
/// Positions are reported as `font_units * scale / upem`, so a scale equal to
/// the face's units per em leaves values in font units. For 26.6 fixed point
/// at a given pixel size, pass `size * 64`.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_scale(font: *mut hr_font_t, x_scale: c_int, y_scale: c_int) {
    if let Some(font) = unsafe { object::as_mutable(font) } {
        font.x_scale = x_scale;
        font.y_scale = y_scale;
    }
}

/// Returns a font's scale.
///
/// # Safety
///
/// `font` must be `NULL` or a live font; `x_scale` and `y_scale` must be
/// `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_scale(
    font: *mut hr_font_t,
    x_scale: *mut c_int,
    y_scale: *mut c_int,
) {
    let font = unsafe { object::or_empty(font.cast_const()) };
    if let Some(out) = unsafe { x_scale.as_mut() } {
        *out = font.x_scale;
    }
    if let Some(out) = unsafe { y_scale.as_mut() } {
        *out = font.y_scale;
    }
}

/// Sets a font's point size, used when applying the `trak` table.
///
/// Pass zero to disable tracking.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_ptem(font: *mut hr_font_t, ptem: f32) {
    if let Some(font) = unsafe { object::as_mutable(font) } {
        font.ptem = ptem;
    }
}

/// Returns a font's point size, or zero if none is set.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_ptem(font: *mut hr_font_t) -> f32 {
    unsafe { object::or_empty(font.cast_const()) }.ptem
}

/// Sets a font's variation settings, in user space.
///
/// Axes that are not named are reset to their default values, so this
/// replaces any previous variation settings rather than adding to them.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `variations` must point to
/// `variations_length` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_variations(
    font: *mut hr_font_t,
    variations: *const hr_variation_t,
    variations_length: c_uint,
) {
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(instance) = font.instance() else {
        return;
    };
    let settings: Vec<FontVariation> = if variations.is_null() || variations_length == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees the array is readable.
        unsafe { core::slice::from_raw_parts(variations, variations_length as usize) }
            .iter()
            .map(|variation| {
                FontVariation::new(crate::common::tag_to_rust(variation.tag), variation.value)
            })
            .collect()
    };
    let rebuilt = FontInstance::builder(instance.font())
        .variations(settings)
        .build();
    font.set_instance(rebuilt);
}

/// Sets a font's variation settings from normalized coordinates.
///
/// Coordinates are 2.14 fixed point values in axis order. Missing axes take
/// their default value; extra coordinates are ignored.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `coords` must point to
/// `coords_length` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_var_coords_normalized(
    font: *mut hr_font_t,
    coords: *const c_int,
    coords_length: c_uint,
) {
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(instance) = font.instance() else {
        return;
    };
    let settings: Vec<NormalizedCoord> = if coords.is_null() || coords_length == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees the array is readable.
        unsafe { core::slice::from_raw_parts(coords, coords_length as usize) }
            .iter()
            .map(|coord| NormalizedCoord::from_bits(*coord as i16))
            .collect()
    };
    let rebuilt = FontInstance::builder(instance.font())
        .normalized_coords(settings)
        .build();
    font.set_instance(rebuilt);
}

/// Returns a font's normalized coordinates, writing their count to `length`.
///
/// The returned pointer stays valid until the font's variation settings change
/// or the font is freed.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `length` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_var_coords_normalized(
    font: *mut hr_font_t,
    length: *mut c_uint,
) -> *const c_int {
    let font = unsafe { object::or_empty(font.cast_const()) };
    if let Some(out) = unsafe { length.as_mut() } {
        *out = font.coords.len() as c_uint;
    }
    font.coords.as_ptr()
}

/// Sets a font's variation settings from a named instance in the `fvar` table.
///
/// An index that does not exist resets the settings to their defaults.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_var_named_instance(font: *mut hr_font_t, instance: c_uint) {
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(current) = font.instance() else {
        return;
    };
    let rebuilt = FontInstance::builder(current.font())
        .named_instance(instance as usize)
        .build();
    font.set_instance(rebuilt);
}

/// Sets the callbacks a font uses during shaping.
///
/// The font takes a reference to `ffuncs` and takes ownership of `font_data`,
/// releasing it through `destroy` when the callbacks are replaced or the font
/// is freed. Pass `NULL` for `ffuncs` to go back to the built-in callbacks.
///
/// As in HarfBuzz, an installed funcs object is authoritative: it is not
/// blended with the built-in callbacks, and any callback it leaves unset
/// reports nothing available, giving glyph 0, a zero advance and no extents.
/// Populate every callback you need before installing the object.
///
/// Callbacks are handed the font they were installed on so they can read its
/// scale and variation settings. They must not modify it, nor free it, while
/// shaping is under way; [`hr_font_make_immutable`] is a convenient way to
/// guarantee that.
///
/// # Safety
///
/// `font` must be `NULL` or a live font, `ffuncs` must be `NULL` or live, and
/// the callbacks must be safe to call with `font_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_funcs(
    font: *mut hr_font_t,
    ffuncs: *mut hr_font_funcs_t,
    font_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        if let Some(destroy) = destroy {
            unsafe { destroy(font_data) };
        }
        return;
    };
    let previous = font.funcs;
    font.funcs = unsafe { object::reference(ffuncs) };
    unsafe { object::destroy(previous) };
    // Dropping the old data runs its destroy callback, unless a sub-font still
    // holds a reference to it.
    font.font_data = Some(Arc::new(FontData {
        data: font_data,
        destroy,
    }));
}

/// Maps a Unicode scalar value to a glyph, returning false if the font has
/// none.
///
/// This consults the font's `cmap` directly and does not run the callbacks set
/// by [`hr_font_set_funcs`].
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `glyph` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_nominal_glyph(
    font: *mut hr_font_t,
    unicode: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    let font = unsafe { object::or_empty(font.cast_const()) };
    let found = font
        .instance()
        .and_then(|instance| instance.font().tables().cmap().ok())
        .and_then(|cmap| cmap.map_codepoint(unicode));
    let Some(found) = found else {
        return false.into();
    };
    if let Some(out) = unsafe { glyph.as_mut() } {
        *out = found.to_u32();
    }
    true.into()
}

/// Maps a Unicode scalar value and variation selector to a glyph, returning
/// false if the font has none.
///
/// This consults the font's `cmap` directly and does not run the callbacks set
/// by [`hr_font_set_funcs`].
///
/// # Safety
///
/// `font` must be `NULL` or a live font, and `glyph` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_variation_glyph(
    font: *mut hr_font_t,
    unicode: hr_codepoint_t,
    variation_selector: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    use read_fonts::tables::cmap::MapVariant;

    let font_ref = unsafe { object::or_empty(font.cast_const()) };
    let Some(cmap) = font_ref
        .instance()
        .and_then(|instance| instance.font().tables().cmap().ok())
    else {
        return false.into();
    };
    let Some((_, uvs)) = cmap.uvs_subtable() else {
        return false.into();
    };
    let found = uvs
        .map_variant(unicode, variation_selector)
        .and_then(|variant| match variant {
            MapVariant::UseDefault => cmap.map_codepoint(unicode),
            MapVariant::Variant(gid) => Some(gid),
        });
    let Some(found) = found else {
        return false.into();
    };
    if let Some(out) = unsafe { glyph.as_mut() } {
        *out = found.to_u32();
    }
    true.into()
}
