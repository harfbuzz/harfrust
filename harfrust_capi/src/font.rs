//! Fonts: a face together with a size and variation settings. Mirrors the
//! shaping-relevant half of HarfBuzz's `hb-font.h`.

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::{Arc, OnceLock};

use harfrust::{
    font::{name::GlyphName, Font, NormalizedCoord, Variation},
    ShaperFont,
};
use read_fonts::TableProvider;

use crate::common::{hr_bool_t, hr_codepoint_t, hr_direction_t, hr_position_t, hr_variation_t};
use crate::common::{hr_glyph_extents_t, ot_round, write_out};
use crate::face::hr_face_t;
use crate::font_funcs::{hr_font_funcs_t, Answer, FontFuncsAdapter};
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

/// The `font_data` a caller attached along with a set of callbacks.
///
/// The font it was given to is its only owner: the `Arc` it is kept in is
/// never cloned, so dropping it there runs `destroy`. A sub-font carries none
/// of its own, and its parent's callbacks are handed the parent's.
pub(crate) struct FontData {
    pub(crate) data: *mut c_void,
    // Safety invariant: `destroy`, if any, is safe to call once with `data`,
    // on any thread.
    destroy: hr_destroy_func_t,
}

// SAFETY: `data` is opaque to us; it is only handed to the caller's callbacks
// and, when this is dropped, to `destroy`. Callers are required to make both
// safe from any thread that uses the font, which the README documents under
// "Threading".
unsafe impl Send for FontData {}
// SAFETY: as for `Send`. All a shared `FontData` gives out is the pointer
// `data`, to be handed to the caller's callbacks, and the same rule has those
// safe to call with it from every thread that uses the font.
unsafe impl Sync for FontData {}

impl Drop for FontData {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: by the invariant on `destroy`, one call with `data` is
            // sound on whichever thread drops this, and it is dropped once.
            unsafe { destroy(self.data) };
        }
    }
}

/// A prepared font view and the stable allocation it borrows.
///
/// Keeping these in one private owner makes replacing the instance drop the
/// font view first. The explicit `Drop` preserves that invariant if the fields
/// are ever reordered.
struct PreparedFont {
    // Safety invariant: when `Some`, this borrows only from the `Font` that
    // `instance` owns, and that `Font` is neither mutated nor released until
    // this is `None` again.
    shaper: Option<ShaperFont<'static, 'static>>,
    /// Never cloned: the `Arc` is here for what it does not claim. A `Box`
    /// asserts unique access to its allocation each time it is moved or
    /// reached through `&mut`, which would invalidate the borrows `shaper`
    /// holds whenever this struct is moved into place or mutated. An `Arc`
    /// only ever shares its contents.
    instance: Arc<Font>,
}

impl PreparedFont {
    fn new(instance: Font) -> Self {
        let instance = Arc::new(instance);
        // SAFETY: dereferencing a raw pointer to make a reference needs it
        // non-null, aligned, and pointing to an initialized `Font` that stays
        // live and unmutated for as long as the reference is used.
        // - `Arc::as_ptr` points to the `Font` inside the `Arc`'s allocation,
        //   and is valid while a strong reference exists.
        // - `instance` is the only strong reference. It is stored next to
        //   `shaper` below and nothing clones or replaces it, so it is
        //   released only when this `PreparedFont` is dropped, after `Drop`
        //   has cleared `shaper`.
        // - Nothing mutates the `Font` in the meantime: an `Arc` gives out
        //   shared access only, and `Arc::get_mut` is never called.
        // The `'static` lifetime overstates this: the reference is good for
        // the life of this `PreparedFont`, which is as long as `shaper` is
        // kept. Nothing safe hands the view out. `shaper()` is unsafe, so
        // that whoever takes the view answers for keeping it, and any clone
        // of it, within a borrow of this `PreparedFont`.
        let font: &'static Font = unsafe { &*Arc::as_ptr(&instance) };
        let shaper = Some(ShaperFont::new(font));
        Self { shaper, instance }
    }

    /// The prepared view.
    ///
    /// # Safety
    ///
    /// The view's `'static` lifetimes are not real: it borrows the `Font` this
    /// `PreparedFont` owns. The view, any clone of it, and anything borrowed
    /// through either must not be used after the borrow of `self` this was
    /// called with ends.
    unsafe fn shaper(&self) -> Option<&ShaperFont<'static, 'static>> {
        self.shaper.as_ref()
    }
}

impl Drop for PreparedFont {
    fn drop(&mut self) {
        // Upholds the invariant on `shaper`: the borrow goes before the
        // fields are dropped, and with them the `Font` it points into.
        self.shaper = None;
    }
}

/// A font: a face with a scale, optional pixel and point sizes, and variation
/// settings applied.
pub struct hr_font_t {
    header: ObjectHeader,
    /// Owned reference to the face this font draws from.
    // Safety invariant: a live face, which this font holds a reference to.
    face: *mut hr_face_t,
    /// `None` only for the immortal empty font, and for a font being freed
    /// once it has let go of its face.
    prepared: Option<PreparedFont>,
    pub(crate) x_scale: c_int,
    pub(crate) y_scale: c_int,
    pub(crate) x_ppem: c_uint,
    pub(crate) y_ppem: c_uint,
    pub(crate) ptem: f32,
    /// Normalized coordinates in HarfBuzz's representation: 2.14 values widened
    /// to `int`, so `hr_font_get_var_coords_normalized` can hand back a
    /// pointer directly.
    coords: Vec<c_int>,
    /// Owned reference to the callbacks, or null for none of its own: its
    /// parent's then apply, or the built-in ones if it has no parent.
    // Safety invariant: `NULL`, or a live funcs object, which this font holds
    // a reference to.
    pub(crate) funcs: *mut hr_font_funcs_t,
    // Safety invariant: while `funcs` carries callbacks, this holds the font
    // data they are to be handed, which they are safe to call with from any
    // thread.
    pub(crate) font_data: Option<Arc<FontData>>,
    /// Owned reference to the parent, for fonts made by
    /// [`hr_font_create_sub_font`]. Callbacks this font does not carry are
    /// answered by the parent, so this is walked on every dispatch.
    // Safety invariant: `NULL`, or a live font, which this font holds a
    // reference to.
    pub(crate) parent: *mut hr_font_t,
}

impl hr_font_t {
    /// Whether this font, or any it descends from, carries callbacks.
    ///
    /// A sub-font answers with its parent's callbacks, so the adapter has to
    /// be installed for it even when it carries none of its own.
    pub(crate) fn has_callbacks(&self) -> bool {
        let mut font = self;
        loop {
            // The built-in callbacks are what shaping does anyway, so a chain
            // carrying only those needs nothing installed over it.
            if !font.funcs.is_null() && font.funcs != crate::font_funcs::builtin_funcs() {
                return true;
            }
            // SAFETY: each font owns its reference to its parent.
            match unsafe { font.parent.as_ref() } {
                Some(parent) => font = parent,
                None => return false,
            }
        }
    }

    /// The glyph the font's own tables give for a character.
    ///
    /// This goes through the same charmap shaping uses, which knows about the
    /// legacy cmap subtables -- Macintosh Roman, and the Windows symbol
    /// encoding's private-use pages -- so that a glyph shaping can find is one
    /// this reports.
    pub(crate) fn builtin_nominal_glyph(&self, unicode: u32) -> Option<hr_codepoint_t> {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        unsafe { self.shaper() }?
            .default_nominal_glyph(unicode)
            .map(|glyph| glyph.to_u32())
    }

    /// As [`hr_font_t::builtin_nominal_glyph`], for a variation selector.
    pub(crate) fn builtin_variation_glyph(
        &self,
        unicode: u32,
        variation_selector: u32,
    ) -> Option<hr_codepoint_t> {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        unsafe { self.shaper() }?
            .default_variation_glyph(unicode, variation_selector)
            .map(|glyph| glyph.to_u32())
    }

    /// What the font's own tables say a glyph's extents are.
    pub(crate) fn builtin_extents(&self, glyph: hr_codepoint_t) -> Option<hr_glyph_extents_t> {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        let extents =
            unsafe { self.shaper() }?.default_glyph_extents(harfrust::GlyphId::from(glyph))?;
        Some(hr_glyph_extents_t {
            x_bearing: extents.x_bearing,
            y_bearing: extents.y_bearing,
            width: extents.width,
            height: extents.height,
        })
    }

    /// This face's design units, which everything the tables say is in.
    pub(crate) fn upem(&self) -> c_int {
        // SAFETY: a font owns its reference to its face.
        unsafe { self.face.as_ref() }.map_or(1000, |face| face.upem() as c_int)
    }

    /// How far the font's own tables advance a glyph, in reported units.
    pub(crate) fn builtin_h_advance(&self, glyph: hr_codepoint_t) -> i32 {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        let Some(font) = (unsafe { self.shaper() }) else {
            return 0;
        };
        font.default_glyph_h_advance(harfrust::GlyphId::from(glyph))
    }

    /// As [`hr_font_t::builtin_h_advance`], downwards.
    pub(crate) fn builtin_v_advance(&self, glyph: hr_codepoint_t) -> i32 {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        let Some(font) = (unsafe { self.shaper() }) else {
            return 0;
        };
        font.default_glyph_v_advance(harfrust::GlyphId::from(glyph))
    }

    /// Where the font's own tables hang a glyph from, in reported units.
    pub(crate) fn builtin_v_origin(&self, glyph: hr_codepoint_t) -> Option<(i32, i32)> {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        Some(unsafe { self.shaper() }?.default_glyph_v_origin(harfrust::GlyphId::from(glyph)))
    }

    /// How far a glyph advances, through whichever callback answers.
    ///
    /// # Safety
    ///
    /// As [`FontFuncsAdapter::new`], with `self` for its `state`.
    pub(crate) unsafe fn glyph_h_advance(
        &self,
        font: *mut hr_font_t,
        glyph: hr_codepoint_t,
    ) -> i32 {
        // SAFETY: the adapter's contract is this function's own, for the
        // same pair.
        match unsafe { FontFuncsAdapter::new(font, self) }.call_h_advance(glyph) {
            Answer::Builtin => self.builtin_h_advance(glyph),
            Answer::Value(advance) => advance,
        }
    }

    /// As [`hr_font_t::glyph_h_advance`], downwards.
    ///
    /// # Safety
    ///
    /// As [`FontFuncsAdapter::new`], with `self` for its `state`.
    pub(crate) unsafe fn glyph_v_advance(
        &self,
        font: *mut hr_font_t,
        glyph: hr_codepoint_t,
    ) -> i32 {
        // SAFETY: the adapter's contract is this function's own, for the
        // same pair.
        match unsafe { FontFuncsAdapter::new(font, self) }.call_v_advance(glyph) {
            Answer::Builtin => self.builtin_v_advance(glyph),
            Answer::Value(advance) => advance,
        }
    }

    /// Where a glyph hangs from, through whichever callback answers.
    ///
    /// # Safety
    ///
    /// As [`FontFuncsAdapter::new`], with `self` for its `state`.
    pub(crate) unsafe fn glyph_v_origin(
        &self,
        font: *mut hr_font_t,
        glyph: hr_codepoint_t,
    ) -> Option<(i32, i32)> {
        // SAFETY: the adapter's contract is this function's own, for the
        // same pair.
        match unsafe { FontFuncsAdapter::new(font, self) }.call_v_origin(glyph) {
            Answer::Builtin => self.builtin_v_origin(glyph),
            Answer::Value(origin) => origin,
        }
    }

    /// The name the face gives a glyph, if it names it at all.
    pub(crate) fn glyph_name(&self, glyph: hr_codepoint_t) -> Option<GlyphName> {
        // SAFETY: the view is used for this one lookup, while `self` is
        // borrowed, and what comes back borrows nothing from it.
        let shaper = unsafe { self.shaper() }?;
        shaper
            .glyph_name(glyph.into())
            .filter(|name| !name.is_synthesized())
    }

    /// A glyph's extents, from whichever callback answers, or from the font's
    /// own tables when none is installed. `None` when nothing can say.
    ///
    /// # Safety
    ///
    /// As [`FontFuncsAdapter::new`], with `self` for its `state`.
    pub(crate) unsafe fn glyph_extents(
        &self,
        font: *mut hr_font_t,
        glyph: hr_codepoint_t,
    ) -> Option<hr_glyph_extents_t> {
        // SAFETY: the adapter's contract is this function's own, for the
        // same pair.
        match unsafe { FontFuncsAdapter::new(font, self) }.call_extents(glyph)? {
            Answer::Builtin => self.builtin_extents(glyph),
            Answer::Value(extents) => Some(extents),
        }
    }

    /// The data this font's own callbacks were given.
    pub(crate) fn callback_data(&self) -> *mut c_void {
        self.font_data
            .as_ref()
            .map_or(core::ptr::null_mut(), |data| data.data)
    }

    pub(crate) fn face(&self) -> *mut hr_face_t {
        self.face
    }

    pub(crate) fn instance(&self) -> Option<&Font> {
        self.prepared.as_ref().map(|prepared| &*prepared.instance)
    }

    /// The view of this font that shaping and the built-in callbacks use.
    ///
    /// # Safety
    ///
    /// The view's `'static` lifetimes are not real: it borrows from this
    /// font. The view, any clone of it, and anything borrowed through either
    /// must not be used after the borrow of `self` this was called with ends.
    pub(crate) unsafe fn shaper(&self) -> Option<&ShaperFont<'static, 'static>> {
        let prepared = self.prepared.as_ref()?;
        // SAFETY: `prepared` is borrowed from `self` for as long as `self` is
        // here, and this function's caller keeps what it is given within
        // that.
        unsafe { prepared.shaper() }
    }

    /// Rebuilds the font instance after a change to variation settings, and
    /// refreshes the normalized coordinate mirror.
    fn set_instance(&mut self, instance: Font) {
        let coords = instance
            .normalized_coords()
            .iter()
            .map(|coord| c_int::from(coord.to_bits()))
            .collect();
        let mut prepared = PreparedFont::new(instance);
        if let Some(shaper) = &mut prepared.shaper {
            shaper.set_scale_separate(self.x_scale, self.y_scale);
        }
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

    unsafe fn finalize(ptr: *mut Self) {
        // What the font holds for the caller goes here rather than when the
        // font is dropped. Letting go of it runs the caller's code: the
        // destroy function of the font data, and those of whatever the
        // parent, the face and the callbacks were given, where this font's
        // reference to one of them is the last. That code may ask the font
        // a question. Here nothing borrows the font. Inside its drop, the
        // drop does.
        //
        // They go in HarfBuzz's order: the font data, the parent, the face,
        // the callbacks.

        // The font data's destroy function runs with the font still as it
        // was, as in HarfBuzz: a getter it calls is answered by the font's
        // callbacks, and those are handed the data being released. So only
        // the destroy function is taken out for now. That also keeps the
        // data from being released a second time when it is dropped, and
        // upholds the invariant on `FontData::destroy`, which has no
        // function left to call.
        //
        // SAFETY: making the reference needs `ptr` to be aligned and to
        // point to a `Self`, and nothing else to use the field for as long
        // as the reference is used.
        // - This function's contract has `ptr` live, with nothing else using
        //   the font.
        // - The reference is used within this statement only. That runs none
        //   of the caller's code, and is over before the destroy function is
        //   called.
        let release = unsafe { &mut (*ptr).font_data }
            .as_mut()
            // Never `None`: the font is its data's only owner.
            .and_then(Arc::get_mut)
            .and_then(|font_data| Some((font_data.destroy.take()?, font_data.data)));
        if let Some((destroy, data)) = release {
            // SAFETY: by the invariant on `FontData::destroy`, one call with
            // `data` is sound on whichever thread makes it. This is that
            // call: the function was taken out of the font data just above,
            // which leaves nothing else to make it.
            unsafe { destroy(data) };
        }

        // With the data gone, the callbacks it was given with are not to be
        // called for this font again. The font lets go of both together,
        // with none of the caller's code run in between, which is what the
        // invariant on `font_data` asks. Its reference to the callbacks is
        // kept until the end, to be released last.
        //
        // SAFETY: each of the five statements like this one names a field
        // through `ptr` and swaps an empty value into it with `replace`.
        // - Naming a field needs `ptr` to point into an allocation that
        //   holds a `Self`. `replace` needs the field to be aligned, to hold
        //   an initialized value, and to be valid for reads and writes, with
        //   nothing else using it meanwhile.
        // - This function's contract has `ptr` live, which makes each field
        //   in bounds, aligned and initialized, and has nothing else using
        //   the font: no reference into it is in use when a field is
        //   written. The one made above ended with its statement.
        // - Destroy functions run between these statements. They may read
        //   the font, which is sound: no `replace` is under way then, and
        //   each field holds a value its invariant allows, so one that every
        //   reader of a font allows for. They cannot leave the font unfit
        //   for the next statement. A font releasing its last reference
        //   takes no changes, which is what `object::as_mutable` goes by,
        //   and a reference taken to it then does not free it when it is
        //   given back. Whatever they borrowed ended when they returned.
        // Afterwards the font holds no callbacks, no font data, no parent,
        // no view of its tables and the empty face, and stays that way for
        // the same reason, so dropping it runs no caller code.
        let funcs = unsafe { (&raw mut (*ptr).funcs).replace(core::ptr::null_mut()) };
        // SAFETY: as for `funcs`, just above.
        let font_data = unsafe { (&raw mut (*ptr).font_data).replace(None) };
        // Its destroy function has run, so this runs nothing.
        drop(font_data);

        // The parent. Whatever this releases finds a font that has none.
        //
        // SAFETY: as for `funcs`. The destroy function run since has
        // returned.
        let parent = unsafe { (&raw mut (*ptr).parent).replace(core::ptr::null_mut()) };
        // SAFETY: `destroy` needs a font that is `NULL` or live, and a
        // reference to it that is the caller's to give up. `parent` is what
        // the field held: by its invariant `NULL`, or a live font that this
        // font held a reference to. That reference is this function's now
        // that the field holds `NULL`, and nothing has released it since.
        unsafe { object::destroy(parent) };

        // The face, and before it the view of its tables that the font
        // prepared. The view shares those tables with the face. Kept past
        // the font's reference to the face, it could be the last thing
        // holding them, and letting go of them is the caller's code again:
        // the destroy functions of the tables, and of the callback or the
        // blob they came from. While the face is live it holds them too, so
        // dropping the view here runs nothing.
        {
            // SAFETY: as for `funcs`. Every destroy function run since has
            // returned.
            let _prepared = unsafe { (&raw mut (*ptr).prepared).replace(None) };
            // Dropped where it stands, as this block ends, and not handed
            // to `drop`. A view passed to a function vouches, for as long
            // as that function runs, for the `Font` it borrows staying put,
            // and that function would be the one to release it.
        }
        // Whatever the face releases finds a font over the empty face.
        //
        // SAFETY: as for `funcs`. Dropping the view ran none of the caller's
        // code.
        let face = unsafe { (&raw mut (*ptr).face).replace(hr_face_t::empty()) };
        // SAFETY: `destroy` needs a face that is `NULL` or live, and a
        // reference to it that is the caller's to give up. `face` is what
        // the field held: by its invariant a live face that this font held
        // a reference to. That reference is this function's now that the
        // field holds the empty face, which is live for good and asks for
        // none. Nothing has released it since.
        unsafe { object::destroy(face) };

        // The callbacks, last. The font let go of them above, and whatever
        // this releases finds a font with none.
        //
        // SAFETY: `destroy` needs a funcs object that is `NULL` or live, and
        // a reference to it that is the caller's to give up. `funcs` is what
        // the field held until it was emptied above: by its invariant
        // `NULL`, or a live funcs object that this font held a reference to.
        // That reference has been this function's since, and nothing has
        // released it: the object is still live.
        unsafe { object::destroy(funcs) };
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
                    x_ppem: 0,
                    y_ppem: 0,
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
/// The font takes its own reference to the face, and makes the face
/// immutable, as HarfBuzz does: a font reads its face from whichever thread
/// it is used on, so the face has to hold still once a font exists. Its
/// scale starts at the face's units per em, so positions come out in font
/// units until [`hr_font_set_scale`] says otherwise.
///
/// Never returns `NULL`; a face with no font data yields the empty font.
///
/// # Safety
///
/// `face` must be `NULL` or a live face.
#[no_mangle]
pub unsafe extern "C" fn hr_font_create(face: *mut hr_face_t) -> *mut hr_font_t {
    // SAFETY: `face` is `NULL` or a live face by this function's contract,
    // and the borrow does not outlast this call.
    let Some(face_ref) = (unsafe { face.as_ref() }) else {
        return hr_font_t::empty();
    };
    let Some(font) = face_ref.font() else {
        return hr_font_t::empty();
    };
    // Frozen before anything is read from the face on the font's behalf, as
    // in HarfBuzz, so that a table callback run from here on already sees
    // the face it serves frozen. Only the face's own setters look at this,
    // and from here on they have a reader to race.
    //
    // SAFETY: `make_immutable` needs a null or live face. `face` is non-null,
    // as `face_ref` shows, and live by this function's contract.
    unsafe { object::make_immutable(face) };
    let upem = font
        .tables()
        .head()
        .map_or(1000, |head| c_int::from(head.units_per_em()));
    // SAFETY: `face` is live, as above. The reference taken becomes the
    // font's own, which is what the invariant on its `face` asks for.
    let owned_face = unsafe { object::reference(face) };
    let mut this = hr_font_t {
        header: ObjectHeader::new(),
        face: owned_face,
        prepared: None,
        x_scale: upem,
        y_scale: upem,
        x_ppem: 0,
        y_ppem: 0,
        ptem: 0.0,
        coords: Vec::new(),
        funcs: core::ptr::null_mut(),
        font_data: None,
        parent: core::ptr::null_mut(),
    };
    this.set_instance(font.default_instance());
    object::create(this)
}

/// Creates a font that starts out as a copy of `parent`.
///
/// The sub-font holds a reference to its parent and inherits its scale, pixel
/// and point sizes, variation settings and callbacks; changing it afterwards
/// does not affect the parent.
///
/// # Safety
///
/// `parent` must be `NULL` or a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_create_sub_font(parent: *mut hr_font_t) -> *mut hr_font_t {
    // SAFETY: `parent` is `NULL` or a live font by this function's contract,
    // and the borrow does not outlast this call.
    let Some(parent_ref) = (unsafe { parent.as_ref() }) else {
        return hr_font_t::empty();
    };
    let Some(instance) = parent_ref.instance() else {
        return hr_font_t::empty();
    };
    let mut this = hr_font_t {
        header: ObjectHeader::new(),
        // SAFETY: the parent's face is live, by the invariant on `face`. The
        // reference taken becomes the sub-font's own.
        face: unsafe { object::reference(parent_ref.face) },
        prepared: None,
        x_scale: parent_ref.x_scale,
        y_scale: parent_ref.y_scale,
        x_ppem: parent_ref.x_ppem,
        y_ppem: parent_ref.y_ppem,
        ptem: parent_ref.ptem,
        coords: Vec::new(),
        // Nothing of its own: a sub-font answers with whatever its parent
        // answers with at the time it is asked, so that callbacks installed
        // on the parent afterwards are its callbacks too. Copying them here
        // would freeze the parent as it was.
        funcs: core::ptr::null_mut(),
        font_data: None,
        // SAFETY: `parent` is live by this function's contract, and not
        // `NULL`, as `parent_ref` shows. The reference taken becomes the
        // sub-font's own.
        parent: unsafe { object::reference(parent) },
    };
    this.set_instance(
        instance
            .instance_builder()
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
    // SAFETY: `font` is `NULL` or a live font by this function's contract.
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
    // SAFETY: `font` is `NULL` or a live font, and the reference released is
    // the caller's to release, both by this function's contract.
    unsafe { object::destroy(font) };
}

/// Attaches user data to a font.
///
/// # Safety
///
/// `font` must be a live font, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_user_data(
    font: *mut hr_font_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract, which
    // also has `key` outlive it.
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
    // SAFETY: `font` is `NULL` or a live font by this function's contract.
    unsafe { object::get_user_data(font, key) }
}

/// Marks a font immutable, and every font it descends from.
///
/// A sub-font answers with its parent's callbacks and scale at the time it is
/// asked, so it only holds still if its parent does. HarfBuzz freezes the
/// chain the same way.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_make_immutable(font: *mut hr_font_t) {
    let mut font = font;
    // SAFETY: `font` is `NULL`, or live by this function's contract to
    // begin with, and from then on the `parent` of a live font, which the
    // invariant on that field has `NULL` or live.
    while let Some(state) = unsafe { font.as_ref() } {
        // SAFETY: `font` is not `NULL`, as `state` shows, and live as above.
        unsafe { object::make_immutable(font) };
        font = state.parent;
    }
}

/// Returns whether a font has been marked immutable.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_is_immutable(font: *mut hr_font_t) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    unsafe { object::is_immutable(font.cast_const()) }.into()
}

/// Returns the face a font was created over, without taking a reference.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_face(font: *mut hr_font_t) -> *mut hr_face_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(font.cast_const()) }.face()
}

/// Returns a font's parent, or `NULL` if it is not a sub-font.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_parent(font: *mut hr_font_t) -> *mut hr_font_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let parent = unsafe { object::or_empty(font.cast_const()) }.parent;
    // A font made from a face has no parent of its own, and answers with the
    // font that is empty rather than with nothing: a caller walking up the
    // chain has something to stop at.
    if parent.is_null() {
        return hr_font_t::empty();
    }
    parent
}

/// Sets a font's scale.
///
/// Positions are reported as `font_units * scale / upem`, so a scale equal to
/// the face's units per em leaves values in font units. For 26.6 fixed point
/// at a given pixel size, pass `size * 64`.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_scale(font: *mut hr_font_t, x_scale: c_int, y_scale: c_int) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    if let Some(font) = unsafe { object::as_mutable(font) } {
        font.x_scale = x_scale;
        font.y_scale = y_scale;
        if let Some(shaper) = font
            .prepared
            .as_mut()
            .and_then(|prepared| prepared.shaper.as_mut())
        {
            shaper.set_scale_separate(x_scale, y_scale);
        }
    }
}

/// Returns a font's scale.
///
/// # Safety
///
/// `font` must be a live font; `x_scale` and `y_scale` must be
/// `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_scale(
    font: *mut hr_font_t,
    x_scale: *mut c_int,
    y_scale: *mut c_int,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let font = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `x_scale` is `NULL` or writable by this function's contract.
    unsafe { write_out(x_scale, font.x_scale) };
    // SAFETY: `y_scale` is `NULL` or writable by this function's contract.
    unsafe { write_out(y_scale, font.y_scale) };
}

/// Sets a font's horizontal and vertical pixels per em. Zero leaves an axis
/// unset. `BASE` device adjustments use the ppem for their coordinate axis.
///
/// # Safety
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_ppem(font: *mut hr_font_t, x_ppem: c_uint, y_ppem: c_uint) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    if let Some(font) = unsafe { object::as_mutable(font) } {
        font.x_ppem = x_ppem;
        font.y_ppem = y_ppem;
    }
}

/// Returns a font's horizontal and vertical pixels per em.
///
/// # Safety
/// `font` must be a live font; output pointers must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_ppem(
    font: *mut hr_font_t,
    x_ppem: *mut c_uint,
    y_ppem: *mut c_uint,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let font = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `x_ppem` is `NULL` or writable by this function's contract.
    unsafe { write_out(x_ppem, font.x_ppem) };
    // SAFETY: `y_ppem` is `NULL` or writable by this function's contract.
    unsafe { write_out(y_ppem, font.y_ppem) };
}

/// Sets a font's point size, used when applying the `trak` table.
///
/// Pass zero to disable tracking.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_ptem(font: *mut hr_font_t, ptem: f32) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    if let Some(font) = unsafe { object::as_mutable(font) } {
        font.ptem = ptem;
    }
}

/// Returns a font's point size, or zero if none is set.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_ptem(font: *mut hr_font_t) -> f32 {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(font.cast_const()) }.ptem
}

/// Sets a font's variation settings, in user space.
///
/// Axes that are not named are reset to their default values, so this
/// replaces any previous variation settings rather than adding to them.
///
/// # Safety
///
/// `font` must be a live font, and `variations` must point to
/// `variations_length` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_variations(
    font: *mut hr_font_t,
    variations: *const hr_variation_t,
    variations_length: c_uint,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(instance) = font.instance() else {
        return;
    };
    let settings: Vec<Variation> = if variations.is_null() || variations_length == 0 {
        Vec::new()
    } else {
        // SAFETY: `from_raw_parts` needs `variations` non-null and aligned,
        // with `variations_length` initialized entries behind it that nothing
        // writes while the slice is used. It is not `NULL` here, and this
        // function's contract has it pointing to that many readable entries.
        // The slice is read into `settings` and gone by the end of this
        // statement.
        unsafe { core::slice::from_raw_parts(variations, variations_length as usize) }
            .iter()
            .map(|variation| {
                Variation::new(crate::common::tag_to_rust(variation.tag), variation.value)
            })
            .collect()
    };
    let rebuilt = instance.instance_builder().variations(settings).build();
    font.set_instance(rebuilt);
}

/// Sets a font's variation settings from normalized coordinates.
///
/// Coordinates are 2.14 fixed point values in axis order. Missing axes take
/// their default value; extra coordinates are ignored.
///
/// # Safety
///
/// `font` must be a live font, and `coords` must point to
/// `coords_length` readable entries.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_var_coords_normalized(
    font: *mut hr_font_t,
    coords: *const c_int,
    coords_length: c_uint,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(instance) = font.instance() else {
        return;
    };
    let settings: Vec<NormalizedCoord> = if coords.is_null() || coords_length == 0 {
        Vec::new()
    } else {
        // SAFETY: `from_raw_parts` needs `coords` non-null and aligned, with
        // `coords_length` initialized entries behind it that nothing writes
        // while the slice is used. It is not `NULL` here, and this function's
        // contract has it pointing to that many readable entries. The slice
        // is read into `settings` and gone by the end of this statement.
        unsafe { core::slice::from_raw_parts(coords, coords_length as usize) }
            .iter()
            .map(|coord| NormalizedCoord::from_bits(*coord as i16))
            .collect()
    };
    let rebuilt = instance
        .instance_builder()
        .normalized_coords(settings)
        .build();
    font.set_instance(rebuilt);
}

/// Returns a font's normalized coordinates, writing their count to `length`.
///
/// The returned pointer stays valid until the font's variation settings change
/// or the font is freed. A font with no coordinates returns `NULL`, as in
/// HarfBuzz.
///
/// # Safety
///
/// `font` must be a live font, and `length` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_var_coords_normalized(
    font: *mut hr_font_t,
    length: *mut c_uint,
) -> *const c_int {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let font = unsafe { object::or_empty(font.cast_const()) };
    if !length.is_null() {
        // SAFETY: `length` is non-null and, by this function's contract,
        // writable. It only returns a value, so it is written through the
        // pointer and not a reference: the caller need not have set it.
        unsafe { length.write(font.coords.len() as c_uint) };
    }
    // An empty vector still has an address, one that points at nothing. A
    // caller that tests the pointer and not the count would read from it.
    if font.coords.is_empty() {
        return core::ptr::null();
    }
    font.coords.as_ptr()
}

/// Sets a font's variation settings from a named instance in the `fvar` table.
///
/// An index that does not exist resets the settings to their defaults.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_var_named_instance(font: *mut hr_font_t, instance: c_uint) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased.
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let Some(current) = font.instance() else {
        return;
    };
    let rebuilt = current.instance_builder().named_instance(instance).build();
    font.set_instance(rebuilt);
}

/// Sets the callbacks a font uses during shaping.
///
/// The font takes a reference to `ffuncs` and takes ownership of `font_data`,
/// releasing it through `destroy` when the callbacks are replaced or the font
/// is freed. Passing `NULL` for `ffuncs` asks for no callbacks at all, which
/// answers nothing; [`hr_ot_font_set_funcs`] is the way back to the built-in
/// ones.
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
/// `font` must be a live font, `ffuncs` must be `NULL` or live, and
/// the callbacks must be safe to call with `font_data` from any thread.
#[no_mangle]
pub unsafe extern "C" fn hr_font_set_funcs(
    font: *mut hr_font_t,
    ffuncs: *mut hr_font_funcs_t,
    font_data: *mut c_void,
    destroy: hr_destroy_func_t,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased. Its last use puts the new data in place, before any of the
    // destroy callbacks below run, so those are free to query the font.
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        if let Some(destroy) = destroy {
            // SAFETY: `destroy` is the caller's function for releasing
            // `font_data`, which the README has safe to call with it from
            // any thread that uses the font. The font takes no changes, so
            // no `FontData` is made to release the data later, and this is
            // the one call.
            unsafe { destroy(font_data) };
        }
        return;
    };
    let previous = font.funcs;
    // No callbacks is not the same as never having set any: HarfBuzz reads
    // NULL as its empty funcs, which answer nothing, where a font that was
    // never given callbacks reads its own tables.
    let ffuncs = if ffuncs.is_null() {
        hr_font_funcs_t::empty()
    } else {
        ffuncs
    };
    // SAFETY: `ffuncs` is the caller's, live by this function's contract, or
    // the empty funcs object, which always is. The reference taken becomes
    // the font's own, which is what the invariant on `funcs` asks for.
    font.funcs = unsafe { object::reference(ffuncs) };
    // The new data upholds the invariant on `FontData::destroy`: `destroy`
    // is the caller's function for releasing `font_data`, which the README
    // has safe to call with it from any thread that uses the font, and only
    // dropping the `FontData` calls it. It upholds the one on `font_data`
    // too: the data goes in with the callbacks installed just above, which
    // this function's contract has safe to call with it from any thread.
    //
    // Dropping the old data runs its destroy callback: this font held the only
    // reference to it. That happens before the callbacks it was given with
    // are released, as in HarfBuzz.
    let previous_data = font.font_data.replace(Arc::new(FontData {
        data: font_data,
        destroy,
    }));
    drop(previous_data);
    // SAFETY: `previous` is what `funcs` held until it was replaced above:
    // by the invariant on that field, `NULL` or a live funcs object. The
    // reference to it was the font's, and is this function's to release now
    // that the font holds another.
    unsafe { object::destroy(previous) };
}

/// Installs the built-in callbacks, which read the font's own tables.
///
/// A font starts out answering this way, and this puts it back after
/// [`hr_font_set_funcs`] has installed others. HarfBuzz spells it
/// `hb_ot_font_set_funcs`, and it is the only way back there too: asking
/// `hr_font_set_funcs` for no callbacks means no callbacks, not these.
///
/// Any data the replaced callbacks were given is released, as it is when they
/// are replaced by other callbacks.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_font_set_funcs(font: *mut hr_font_t) {
    // SAFETY: `font` is `NULL`, or live by this function's contract. A
    // font that takes changes is in no other use while it is changed, which
    // the README asks of callers under "Threading", so the reference is not
    // aliased. Its last use takes the old data out, before any of the
    // destroy callbacks below run, so those are free to query the font.
    let Some(font) = (unsafe { object::as_mutable(font) }) else {
        return;
    };
    let previous = font.funcs;
    // Said outright, rather than by carrying nothing: a sub-font carrying
    // nothing asks its parent, and this is a font that does not.
    //
    // SAFETY: the built-in funcs object is immortal, so it is always live,
    // and referencing it does nothing.
    font.funcs = unsafe { object::reference(crate::font_funcs::builtin_funcs()) };
    // The built-in funcs object carries no callbacks, so the invariant on
    // `font_data` asks for nothing to be kept.
    let previous_data = font.font_data.take();
    drop(previous_data);
    // SAFETY: as in `hr_font_set_funcs`: `previous` is what `funcs` held
    // until it was replaced above, `NULL` or a live funcs object, with a
    // reference that is now this function's to release.
    unsafe { object::destroy(previous) };
}

/// Maps a Unicode scalar value to a glyph, returning false if the font has
/// none.
///
/// Callbacks set by [`hr_font_set_funcs`] answer this, as they answer for the
/// font while shaping. Only a font that was never given any reads the font's
/// own `cmap`.
///
/// # Safety
///
/// `font` must be a live font, and `glyph` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_nominal_glyph(
    font: *mut hr_font_t,
    unicode: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // The glyph is reported as not found before anything is asked, so a
    // caller that ignores the return value reads 0 rather than whatever it
    // happened to pass in. HarfBuzz writes it the same way.
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, 0) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    let found = match unsafe { FontFuncsAdapter::new(font, state) }.call_nominal_glyph(unicode) {
        // Callbacks are installed somewhere in the chain, and none of them
        // answer for this.
        None => None,
        Some(Answer::Value(glyph)) => Some(glyph),
        // Nobody carries callbacks, so the font's own tables answer.
        Some(Answer::Builtin) => state.builtin_nominal_glyph(unicode),
    };
    let Some(found) = found else {
        return false.into();
    };
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, found) };
    true.into()
}

/// Maps a strided batch of Unicode codepoints to nominal glyphs.
///
/// Stops at the first missing glyph and returns the number mapped. Zero
/// strides may be used to read or write the same location repeatedly.
///
/// # Safety
/// `font` must be live. The arrays must be valid for `count`
/// entries at their respective byte strides, `first_unicode` to read and
/// `first_glyph` to write.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_nominal_glyphs(
    font: *mut hr_font_t,
    count: c_uint,
    first_unicode: *const hr_codepoint_t,
    unicode_stride: c_uint,
    first_glyph: *mut hr_codepoint_t,
    glyph_stride: c_uint,
) -> c_uint {
    if count == 0 || first_unicode.is_null() || first_glyph.is_null() {
        return 0;
    }
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    let adapter = unsafe { FontFuncsAdapter::new(font, state) };
    // SAFETY: `call_nominal_glyphs` needs `count` codepoints to read
    // `unicode_stride` bytes apart, and as many glyphs to write
    // `glyph_stride` bytes apart. Neither array is `NULL`, as checked above,
    // and this function's contract has each valid for `count` entries at its
    // stride, `first_unicode` to read and `first_glyph` to write.
    unsafe {
        adapter.call_nominal_glyphs(
            count,
            first_unicode,
            unicode_stride,
            first_glyph,
            glyph_stride,
        )
    }
}

/// Maps a Unicode scalar value and variation selector to a glyph, returning
/// false if the font has none.
///
/// Callbacks set by [`hr_font_set_funcs`] answer this, as they answer for the
/// font while shaping. Only a font that was never given any reads the font's
/// own `cmap`.
///
/// # Safety
///
/// `font` must be a live font, and `glyph` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_variation_glyph(
    font: *mut hr_font_t,
    unicode: hr_codepoint_t,
    variation_selector: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // As in `hr_font_get_nominal_glyph`, not found until it is.
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, 0) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    let found = match unsafe { FontFuncsAdapter::new(font, state) }
        .call_variation_glyph(unicode, variation_selector)
    {
        None => None,
        Some(Answer::Value(glyph)) => Some(glyph),
        Some(Answer::Builtin) => state.builtin_variation_glyph(unicode, variation_selector),
    };
    let Some(found) = found else {
        return false.into();
    };
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, found) };
    true.into()
}

/// Maps a Unicode scalar value to a glyph, with or without a variation
/// selector, returning false if the font has none.
///
/// A zero selector calls [`hr_font_get_nominal_glyph`]. A nonzero selector
/// calls [`hr_font_get_variation_glyph`], which returns false and writes
/// glyph zero for an unsupported sequence.
///
/// # Safety
///
/// `font` must be a live font, and `glyph` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph(
    font: *mut hr_font_t,
    unicode: hr_codepoint_t,
    variation_selector: hr_codepoint_t,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    // SAFETY: Both getters accept the same font and output pointer as this function.
    unsafe {
        if variation_selector == 0 {
            hr_font_get_nominal_glyph(font, unicode, glyph)
        } else {
            hr_font_get_variation_glyph(font, unicode, variation_selector, glyph)
        }
    }
}

/// How far a glyph advances when text runs horizontally.
///
/// Callbacks set by [`hr_font_set_funcs`] answer this, as they answer for the
/// font while shaping. Only a font that was never given any reads the font's
/// own `hmtx`. The answer is in the units [`hr_font_set_scale`] asks for.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_h_advance(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
) -> hr_position_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // `glyph_h_advance` takes either pair.
    unsafe { state.glyph_h_advance(font, glyph) }
}

/// As [`hr_font_get_glyph_h_advance`], for text running vertically.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_v_advance(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
) -> hr_position_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // `glyph_v_advance` takes either pair.
    unsafe { state.glyph_v_advance(font, glyph) }
}

/// Fills in horizontal advances for a run of glyphs.
///
/// The glyphs and the advances are each read and written every `stride`
/// bytes, so a caller can walk its own structures rather than pack arrays.
///
/// # Safety
///
/// `font` must be a live font. `first_glyph` must be `NULL` or
/// point at `count` glyphs `glyph_stride` bytes apart, and `first_advance`
/// must be `NULL` or point at `count` writable advances `advance_stride`
/// bytes apart.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_h_advances(
    font: *mut hr_font_t,
    count: c_uint,
    first_glyph: *const hr_codepoint_t,
    glyph_stride: c_uint,
    first_advance: *mut hr_position_t,
    advance_stride: c_uint,
) {
    // SAFETY: `advances` has this function's contract, and is handed the
    // same font and runs.
    unsafe {
        advances(
            font,
            count,
            first_glyph,
            glyph_stride,
            first_advance,
            advance_stride,
            true,
        );
    }
}

/// As [`hr_font_get_glyph_h_advances`], for text running vertically.
///
/// # Safety
///
/// As [`hr_font_get_glyph_h_advances`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_v_advances(
    font: *mut hr_font_t,
    count: c_uint,
    first_glyph: *const hr_codepoint_t,
    glyph_stride: c_uint,
    first_advance: *mut hr_position_t,
    advance_stride: c_uint,
) {
    // SAFETY: `advances` has this function's contract, and is handed the
    // same font and runs.
    unsafe {
        advances(
            font,
            count,
            first_glyph,
            glyph_stride,
            first_advance,
            advance_stride,
            false,
        );
    }
}

/// The walk both advance runs share.
///
/// # Safety
///
/// As [`hr_font_get_glyph_h_advances`].
unsafe fn advances(
    font: *mut hr_font_t,
    count: c_uint,
    first_glyph: *const hr_codepoint_t,
    glyph_stride: c_uint,
    first_advance: *mut hr_position_t,
    advance_stride: c_uint,
    horizontal: bool,
) {
    if first_glyph.is_null() || first_advance.is_null() {
        return;
    }
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    let adapter = unsafe { FontFuncsAdapter::new(font, state) };
    if horizontal
        // SAFETY: `call_h_advances` needs `count` glyphs to read
        // `glyph_stride` bytes apart, and as many advances to write and
        // read back `advance_stride` bytes apart. Neither run is `NULL`, as
        // checked above, so this function's contract has the first as that,
        // and the second as writable storage, which can be read back once
        // it is written.
        && unsafe {
            adapter.call_h_advances(
                count,
                first_glyph,
                glyph_stride,
                first_advance,
                advance_stride,
            )
        }
    {
        return;
    }
    // Each entry is reached from the first rather than from the one before
    // it. A run ends where its last entry does, so stepping a stride on from
    // that entry would leave the run whenever the stride is wider than the
    // entry, which is not allowed even of a pointer that is never read.
    for i in 0..count as usize {
        // A stride can land anywhere, so nothing here assumes alignment.
        // SAFETY: `byte_add` needs the offset to stay inside the allocation
        // `first_glyph` points into, and `read_unaligned` needs a readable,
        // initialized glyph there. The caller guarantees `count` glyphs
        // `glyph_stride` bytes apart, and `i < count`, so entry `i` starts
        // `i * glyph_stride` bytes into that run.
        let id = unsafe {
            first_glyph
                .byte_add(i * glyph_stride as usize)
                .read_unaligned()
        };
        let value = if horizontal {
            // SAFETY: `glyph_h_advance` takes the pair the adapter above was
            // given.
            unsafe { state.glyph_h_advance(font, id) }
        } else {
            // SAFETY: `glyph_v_advance` takes the pair the adapter above was
            // given.
            unsafe { state.glyph_v_advance(font, id) }
        };
        // SAFETY: as above for the advances, which the caller guarantees are
        // `advance_stride` bytes apart and writable. An advance is a plain
        // integer, so overwriting one drops nothing.
        unsafe {
            first_advance
                .byte_add(i * advance_stride as usize)
                .write_unaligned(value);
        };
    }
}

/// Where a glyph hangs from when text runs horizontally.
///
/// Always the glyph's own origin, so this reports `0, 0` and true, which is
/// what HarfBuzz answers for a font reading its own tables.
///
/// # Safety
///
/// `font` must be a live font, and `x` and `y` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_h_origin(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    let origin = unsafe { FontFuncsAdapter::new(font, state) }.call_h_origin(glyph);
    // SAFETY: `x` is `NULL` or writable by this function's contract.
    unsafe { write_out(x, origin.map_or(0, |origin| origin.0)) };
    // SAFETY: `y` is `NULL` or writable by this function's contract.
    unsafe { write_out(y, origin.map_or(0, |origin| origin.1)) };
    origin.is_some().into()
}

/// Returns the horizontal kerning supplied by font callbacks, or zero.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_h_kerning(
    font: *mut hr_font_t,
    first_glyph: hr_codepoint_t,
    second_glyph: hr_codepoint_t,
) -> hr_position_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    unsafe { FontFuncsAdapter::new(font, state) }.call_kerning(first_glyph, second_glyph, false)
}

/// Returns the vertical kerning supplied by font callbacks, or zero.
///
/// # Safety
///
/// `font` must be a live font.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_v_kerning(
    font: *mut hr_font_t,
    first_glyph: hr_codepoint_t,
    second_glyph: hr_codepoint_t,
) -> hr_position_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // The adapter takes either pair.
    unsafe { FontFuncsAdapter::new(font, state) }.call_kerning(first_glyph, second_glyph, true)
}

/// Where a glyph hangs from when text runs vertically, returning false when
/// nothing can say.
///
/// Callbacks set by [`hr_font_set_funcs`] answer this, as they answer for the
/// font while shaping. Only a font that was never given any reads the font's
/// own `VORG` and `vmtx`.
///
/// # Safety
///
/// `font` must be a live font, and `x` and `y` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_v_origin(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // Nowhere until somewhere, so a caller ignoring the return value reads
    // zeroes rather than whatever it passed in.
    // SAFETY: `x` is `NULL` or writable by this function's contract.
    unsafe { write_out(x, 0) };
    // SAFETY: `y` is `NULL` or writable by this function's contract.
    unsafe { write_out(y, 0) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // `glyph_v_origin` takes either pair.
    let Some((origin_x, origin_y)) = (unsafe { state.glyph_v_origin(font, glyph) }) else {
        return false.into();
    };
    // SAFETY: `x` is `NULL` or writable by this function's contract.
    unsafe { write_out(x, origin_x) };
    // SAFETY: `y` is `NULL` or writable by this function's contract.
    unsafe { write_out(y, origin_y) };
    true.into()
}

/// A glyph's ink extents, returning false when the font cannot say.
///
/// Callbacks set by [`hr_font_set_funcs`] answer this, as they answer for the
/// font while shaping. Only a font that was never given any reads the font's
/// own outlines. The answer is in the units [`hr_font_set_scale`] asks for.
///
/// # Safety
///
/// `font` must be a live font, and `extents` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_extents(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    extents: *mut hr_glyph_extents_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // Nothing until something, as above.
    // SAFETY: `extents` is `NULL` or writable by this function's contract.
    unsafe { write_out(extents, hr_glyph_extents_t::default()) };
    // SAFETY: `state` is `font` through `or_empty`: the font it points to,
    // live by this function's contract, or the empty font if it is `NULL`.
    // `glyph_extents` takes either pair.
    let Some(found) = (unsafe { state.glyph_extents(font, glyph) }) else {
        return false.into();
    };
    // SAFETY: `extents` is `NULL` or writable by this function's contract.
    unsafe { write_out(extents, found) };
    true.into()
}

/// How far a glyph advances in the given direction: horizontally into `x`,
/// vertically into `y`, and zero into the other.
///
/// # Safety
///
/// `font` must be a live font, and `x` and `y` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_advance_for_direction(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    let (dx, dy) = if crate::common::hr_direction_is_horizontal(direction) != 0 {
        // SAFETY: `state` is `font` through `or_empty`: the font it points
        // to, live by this function's contract, or the empty font if it is
        // `NULL`. `glyph_h_advance` takes either pair.
        (unsafe { state.glyph_h_advance(font, glyph) }, 0)
    } else {
        // SAFETY: as in the other branch; `glyph_v_advance` takes the same
        // pairs.
        (0, unsafe { state.glyph_v_advance(font, glyph) })
    };
    // SAFETY: `x` is `NULL` or writable by this function's contract.
    unsafe { write_out(x, dx) };
    // SAFETY: `y` is `NULL` or writable by this function's contract.
    unsafe { write_out(y, dy) };
}

/// As [`hr_font_get_glyph_advance_for_direction`], for a run of glyphs.
///
/// # Safety
///
/// As [`hr_font_get_glyph_h_advances`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_advances_for_direction(
    font: *mut hr_font_t,
    direction: hr_direction_t,
    count: c_uint,
    first_glyph: *const hr_codepoint_t,
    glyph_stride: c_uint,
    first_advance: *mut hr_position_t,
    advance_stride: c_uint,
) {
    let horizontal = crate::common::hr_direction_is_horizontal(direction) != 0;
    // SAFETY: `advances` has this function's contract, and is handed the
    // same font and runs.
    unsafe {
        advances(
            font,
            count,
            first_glyph,
            glyph_stride,
            first_advance,
            advance_stride,
            horizontal,
        );
    }
}

/// The origin every direction-relative call is measured from.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
unsafe fn origin_for_direction(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
) -> (hr_position_t, hr_position_t) {
    // SAFETY: `font` is `NULL` or a live font by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    if crate::common::hr_direction_is_horizontal(direction) != 0 {
        // SAFETY: `state` is `font` through `or_empty`: the font it points
        // to, live by this function's contract, or the empty font if it is
        // `NULL`. The adapter takes either pair.
        return unsafe { FontFuncsAdapter::new(font, state) }
            .call_h_origin(glyph)
            .unwrap_or((0, 0));
    }
    // SAFETY: `glyph_v_origin` takes the same pairs, and this is the pair
    // described above.
    if let Some(origin) = unsafe { state.glyph_v_origin(font, glyph) } {
        return origin;
    }
    // Nothing says where it hangs from vertically, so HarfBuzz guesses from
    // the horizontal origin: the middle of the advance, at the ascender.
    // SAFETY: as for `glyph_v_origin` above.
    let x = unsafe { state.glyph_h_advance(font, glyph) } / 2;
    let y = state
        .instance()
        .and_then(|instance| instance.metrics().hhea_line)
        .map_or(0, |line| {
            ot_round(line.ascender.to_f64() * f64::from(state.y_scale) / f64::from(state.upem()))
                as i32
        });
    (x, y)
}

/// Where a glyph hangs from in the given direction.
///
/// A font that cannot say where a glyph hangs from vertically has one guessed
/// for it from the horizontal origin, as HarfBuzz does.
///
/// # Safety
///
/// `font` must be a live font, and `x` and `y` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_origin_for_direction(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) {
    // SAFETY: `origin_for_direction` asks only that `font` be `NULL` or a
    // live font. It is `NULL`, or live by this function's contract.
    let (origin_x, origin_y) = unsafe { origin_for_direction(font, glyph, direction) };
    // SAFETY: `x` is `NULL` or writable by this function's contract.
    unsafe { write_out(x, origin_x) };
    // SAFETY: `y` is `NULL` or writable by this function's contract.
    unsafe { write_out(y, origin_y) };
}

/// Moves a point from the glyph's own origin to the direction's origin.
///
/// # Safety
///
/// `font` must be a live font, and `x` and `y` must be `NULL` or
/// readable and writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_add_glyph_origin_for_direction(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) {
    // SAFETY: `origin_for_direction` asks only that `font` be `NULL` or a
    // live font. It is `NULL`, or live by this function's contract.
    let (origin_x, origin_y) = unsafe { origin_for_direction(font, glyph, direction) };
    // SAFETY: `x` is `NULL` or readable and writable by this function's
    // contract. It carries a value in, so the caller has set it, and none of
    // the caller's code runs while the reference is held. The reference is
    // gone before `y` is looked at, so the two may be the same place.
    if let Some(out) = unsafe { x.as_mut() } {
        *out = out.saturating_add(origin_x);
    }
    // SAFETY: as for `x`, whose reference is gone by now.
    if let Some(out) = unsafe { y.as_mut() } {
        *out = out.saturating_add(origin_y);
    }
}

/// Moves a point from the direction's origin back to the glyph's own.
///
/// # Safety
///
/// As [`hr_font_add_glyph_origin_for_direction`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_subtract_glyph_origin_for_direction(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
    x: *mut hr_position_t,
    y: *mut hr_position_t,
) {
    // SAFETY: `origin_for_direction` asks only that `font` be `NULL` or a
    // live font. It is `NULL`, or live by this function's contract.
    let (origin_x, origin_y) = unsafe { origin_for_direction(font, glyph, direction) };
    // SAFETY: `x` is `NULL` or readable and writable by this function's
    // contract. It carries a value in, so the caller has set it, and none of
    // the caller's code runs while the reference is held. The reference is
    // gone before `y` is looked at, so the two may be the same place.
    if let Some(out) = unsafe { x.as_mut() } {
        *out = out.saturating_sub(origin_x);
    }
    // SAFETY: as for `x`, whose reference is gone by now.
    if let Some(out) = unsafe { y.as_mut() } {
        *out = out.saturating_sub(origin_y);
    }
}

/// A glyph's ink extents, measured from the direction's origin rather than
/// from the glyph's own.
///
/// # Safety
///
/// `font` must be a live font, and `extents` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_extents_for_origin(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    direction: hr_direction_t,
    extents: *mut hr_glyph_extents_t,
) -> hr_bool_t {
    // The extents are asked for into a local, so that what is adjusted below
    // is never read back out of the caller's storage. That storage is still
    // written whenever `hr_font_get_glyph_extents` would write it: with
    // nothing before the font is asked, and with the font's answer before
    // the origin is.
    let mut found = hr_glyph_extents_t::default();
    // SAFETY: `extents` is `NULL` or writable by this function's contract.
    unsafe { write_out(extents, found) };
    // SAFETY: `font` carries through from the caller, and `found` is a local
    // of this function.
    let known = unsafe { hr_font_get_glyph_extents(font, glyph, &raw mut found) };
    if known == 0 {
        return known;
    }
    // SAFETY: `extents` is `NULL` or writable by this function's contract.
    unsafe { write_out(extents, found) };
    // SAFETY: `origin_for_direction` asks only that `font` be `NULL` or a
    // live font. It is `NULL`, or live by this function's contract.
    let (origin_x, origin_y) = unsafe { origin_for_direction(font, glyph, direction) };
    found.x_bearing = found.x_bearing.saturating_sub(origin_x);
    found.y_bearing = found.y_bearing.saturating_sub(origin_y);
    // SAFETY: `extents` is `NULL` or writable by this function's contract.
    unsafe { write_out(extents, found) };
    true.into()
}

/// Writes as much of `bytes` as fits, always NUL-terminated.
///
/// # Safety
///
/// `out` must point at `size` writable bytes that `bytes` does not overlap,
/// and `size` must not be zero.
unsafe fn write_cstr(bytes: &[u8], out: *mut c_char, size: c_uint) {
    let take = bytes.len().min(size as usize - 1);
    // SAFETY: `copy_nonoverlapping` needs `take` bytes to read at the source
    // and to write at `out`, and the two not to overlap. `take` is no more
    // than `bytes.len()` and no more than `size - 1`, and this function's
    // contract has `out` good for `size` bytes that `bytes` does not
    // overlap. A byte pointer is always aligned. `out.add(take)` is then one
    // of those `size` bytes, the last at most, so the terminator is written
    // inside them.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, take);
        *out.add(take) = 0;
    }
}

/// The name the face gives a glyph, returning false when it names none.
///
/// The name is written NUL-terminated, truncated to fit.
///
/// # Safety
///
/// `font` must be a live font, and `name` must be `NULL` or point
/// at `size` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_name(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    name: *mut c_char,
    size: c_uint,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    if size != 0 {
        // SAFETY: `name` is `NULL` or writable for `size` bytes, which is at
        // least this one, by this function's contract.
        unsafe { write_out(name, 0) };
    }
    let Some(found) = state.glyph_name(glyph) else {
        return false.into();
    };
    if name.is_null() || size == 0 {
        // Nowhere to put it, but the face does name the glyph.
        return true.into();
    }
    // SAFETY: `name` is not `NULL` and `size` is not zero, as just checked,
    // so this function's contract has `name` pointing at `size` writable
    // bytes. The name is read out of `found`, a local of this function,
    // which the caller's buffer does not overlap.
    unsafe { write_cstr(found.as_str().as_bytes(), name, size) };
    true.into()
}

/// The glyph a face gives a name to, returning false when it names none such.
///
/// A length of -1 means the name is NUL-terminated.
///
/// # Safety
///
/// `font` must be a live font, `name` must be `NULL` or readable
/// for its length, and `glyph` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_font_get_glyph_from_name(
    font: *mut hr_font_t,
    name: *const c_char,
    len: c_int,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    // SAFETY: `font` is `NULL`, or live by this function's contract.
    let state = unsafe { object::or_empty(font.cast_const()) };
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, 0) };
    // SAFETY: `name` is `NULL` or readable for its length by this function's
    // contract: for `len` bytes, or up to its NUL when `len` is negative.
    // The string is not used past this call.
    let Some(wanted) = (unsafe { crate::common::str_from_raw(name, len) }) else {
        return false.into();
    };
    // The font indexes its stored names once and searches that afterwards.
    // Asking every glyph the face claims for its name instead would reread
    // the name source for each of a `MAXP` count nothing bounds.
    let found = state
        .instance()
        .and_then(|instance| instance.glyph_by_name(wanted))
        .map(|found| found.to_u32());
    let Some(found) = found else {
        return false.into();
    };
    // SAFETY: `glyph` is `NULL` or writable by this function's contract.
    unsafe { write_out(glyph, found) };
    true.into()
}

/// The name a glyph goes by, falling back to `gidNNN` when the face names it
/// nothing.
///
/// # Safety
///
/// As [`hr_font_get_glyph_name`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_glyph_to_string(
    font: *mut hr_font_t,
    glyph: hr_codepoint_t,
    s: *mut c_char,
    size: c_uint,
) {
    // SAFETY: `hr_font_get_glyph_name` has this function's contract, and is
    // handed the same font and buffer.
    if unsafe { hr_font_get_glyph_name(font, glyph, s, size) } != 0 {
        return;
    }
    if s.is_null() || size == 0 {
        return;
    }
    let text = format!("gid{glyph}");
    // SAFETY: `s` is not `NULL` and `size` is not zero, as just checked, so
    // this function's contract has `s` pointing at `size` writable bytes.
    // `text` was allocated just above, so the caller's buffer does not
    // overlap it.
    unsafe { write_cstr(text.as_bytes(), s, size) };
}

/// The glyph a string names: by the face's own names, by glyph number, by
/// `gidNNN`, or by `uniXXXX`.
///
/// # Safety
///
/// As [`hr_font_get_glyph_from_name`].
#[no_mangle]
pub unsafe extern "C" fn hr_font_glyph_from_string(
    font: *mut hr_font_t,
    s: *const c_char,
    len: c_int,
    glyph: *mut hr_codepoint_t,
) -> hr_bool_t {
    // SAFETY: `hr_font_get_glyph_from_name` has this function's contract,
    // and is handed the same font, string and output.
    if unsafe { hr_font_get_glyph_from_name(font, s, len, glyph) } != 0 {
        return true.into();
    }
    // SAFETY: `s` is `NULL` or readable for its length by this function's
    // contract: for `len` bytes, or up to its NUL when `len` is negative.
    // The string is not used past this call.
    let Some(text) = (unsafe { crate::common::str_from_raw(s, len) }) else {
        return false.into();
    };
    let parsed = text
        .parse::<hr_codepoint_t>()
        .ok()
        .or_else(|| text.strip_prefix("gid")?.parse::<hr_codepoint_t>().ok());
    if let Some(found) = parsed {
        // SAFETY: `glyph` is `NULL` or writable by this function's contract.
        unsafe { write_out(glyph, found) };
        return true.into();
    }
    // `uniXXXX` names a character, and the font says which glyph that is.
    let Some(unicode) = text
        .strip_prefix("uni")
        .and_then(|hex| hr_codepoint_t::from_str_radix(hex, 16).ok())
    else {
        return false.into();
    };
    // SAFETY: `font` is `NULL`, or live by this function's contract, which
    // has `glyph` `NULL` or writable. `hr_font_get_nominal_glyph` asks the
    // same of both, and takes a `NULL` font all the same, as this one does.
    unsafe { hr_font_get_nominal_glyph(font, unicode, glyph) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::ptr;

    struct Destruction {
        font: *mut hr_font_t,
        face: *mut hr_face_t,
        calls: Cell<u32>,
        queried_face: Cell<bool>,
        replacement_calls: Cell<u32>,
        replacement_queried_face: Cell<bool>,
        replacement_released_immediately: Cell<bool>,
        reset_rejected: Cell<bool>,
    }

    unsafe extern "C" fn release_replacement(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state
            .replacement_calls
            .set(state.replacement_calls.get() + 1);
        state
            .replacement_queried_face
            .set(unsafe { hr_font_get_face(state.font) == state.face });
    }

    unsafe extern "C" fn release_font_data(data: *mut c_void) {
        let state = unsafe { &*data.cast::<Destruction>() };
        state.calls.set(state.calls.get() + 1);
        state
            .queried_face
            .set(unsafe { hr_font_get_face(state.font) == state.face });
        // Finalization rejects replacement ownership and, as for immutable
        // fonts, immediately releases the supplied data outside any &mut.
        unsafe { hr_font_set_funcs(state.font, ptr::null_mut(), data, Some(release_replacement)) };
        state
            .replacement_released_immediately
            .set(state.replacement_calls.get() == 1);
        let funcs = unsafe { (*state.font).funcs };
        unsafe { hr_ot_font_set_funcs(state.font) };
        state
            .reset_rejected
            .set(unsafe { (*state.font).funcs } == funcs);
    }

    fn check_finalization(font: *mut hr_font_t, through_sub_font: bool) {
        let state = Destruction {
            font,
            face: unsafe { hr_font_get_face(font) },
            calls: Cell::new(0),
            queried_face: Cell::new(false),
            replacement_calls: Cell::new(0),
            replacement_queried_face: Cell::new(false),
            replacement_released_immediately: Cell::new(false),
            reset_rejected: Cell::new(false),
        };
        assert!(!state.face.is_null());
        unsafe {
            hr_font_set_funcs(
                font,
                ptr::null_mut(),
                ptr::from_ref(&state).cast_mut().cast(),
                Some(release_font_data),
            );
            let sub = through_sub_font.then(|| hr_font_create_sub_font(font));
            hr_font_destroy(font);
            if let Some(sub) = sub {
                assert_eq!(state.calls.get(), 0);
                hr_font_destroy(sub);
            }
        }
        assert_eq!(state.calls.get(), 1);
        assert!(state.queried_face.get());
        assert_eq!(state.replacement_calls.get(), 1);
        assert!(state.replacement_queried_face.get());
        assert!(state.replacement_released_immediately.get());
        assert!(state.reset_rejected.get());
    }

    #[test]
    fn isolated_font_data_destroy_can_query_the_font() {
        // A font with no prepared view, so that this exercises nothing but
        // callback aliasing during font destruction.
        let font = object::create(hr_font_t {
            header: ObjectHeader::new(),
            face: hr_face_t::empty(),
            prepared: None,
            x_scale: 1000,
            y_scale: 1000,
            x_ppem: 0,
            y_ppem: 0,
            ptem: 0.0,
            coords: Vec::new(),
            funcs: ptr::null_mut(),
            font_data: None,
            parent: ptr::null_mut(),
        });
        check_finalization(font, false);
    }

    fn public_font() -> *mut hr_font_t {
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
            let font = hr_font_create(face);
            crate::hr_face_destroy(face);
            crate::hr_blob_destroy(blob);
            assert_ne!(font, hr_font_get_empty());
            font
        }
    }

    #[test]
    fn public_font_data_destroy_can_query_the_font() {
        check_finalization(public_font(), false);
    }

    #[test]
    fn public_font_data_destroy_can_query_parent_released_by_sub_font() {
        check_finalization(public_font(), true);
    }
}
