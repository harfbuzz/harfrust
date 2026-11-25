//! Shared Skrifa input: an immutable SFNT assembled once per face.
//! Read known rendering tables directly, so callback faces need no enumeration.
// The C drawing/painting ABI uses f32 coordinates, including integer font scales.
#![allow(clippy::cast_precision_loss)]
use crate::*;
use core::ffi::c_void;
use harfrust::font::Blob;
use read_fonts::{types::F2Dot14, FontRef};

/// Skrifa's view of a face: the SFNT assembled from its tables, and the
/// outline collection that borrows it.
///
/// Building the collection parses the whole CFF Top DICT, whose length the
/// font chooses, so it is built once per face here rather than once per
/// glyph. Like `PreparedFont`, this keeps the view and the data it borrows in
/// one private owner; the explicit `Drop` clears the borrow before the blob is
/// released even if the fields are ever reordered.
pub(crate) struct RenderData {
    // Safety invariant: when `Some`, this borrows only from the bytes `blob`
    // owns, which are neither written nor released until this is `None`
    // again.
    outlines: Option<skrifa::OutlineGlyphCollection<'static>>,
    blob: Blob,
}

impl RenderData {
    fn new(bytes: Vec<u8>) -> Option<Self> {
        let blob = Blob::from(bytes);
        let font = FontRef::new(&blob).ok()?;
        let outlines = skrifa::OutlineGlyphCollection::new(&font);
        // SAFETY: the collection borrows the data owned by `blob`, which is
        // reference counted, stable across moving the `Blob` and never written
        // again. `Drop` clears the borrow before the blob is released.
        let outlines = Some(unsafe {
            core::mem::transmute::<
                skrifa::OutlineGlyphCollection<'_>,
                skrifa::OutlineGlyphCollection<'static>,
            >(outlines)
        });
        Some(Self { outlines, blob })
    }
}

impl Drop for RenderData {
    fn drop(&mut self) {
        self.outlines = None;
    }
}

/// The face's Skrifa data, assembled on first use and kept with the face.
///
/// # Safety
///
/// `face` must be a live face: its tables are fetched as a caller of this
/// API would fetch them, through a pointer made from the borrow.
unsafe fn render_data(face: &hr_face_t) -> Option<&RenderData> {
    face.render_data
        .get_or_init(|| {
            // SAFETY: `collect_tables` needs `NULL` or a live face, and uses
            // the pointer for shared access only. `face` borrows a live
            // face, by this function's contract, so the pointer made from it
            // points to one, and a pointer made from a shared borrow is good
            // for that much.
            unsafe { collect_tables(core::ptr::from_ref(face).cast_mut()) }
                .and_then(RenderData::new)
        })
        .as_ref()
}
/// The face's assembled SFNT, built on first use and never changed after.
///
/// # Safety
///
/// As [`render_data`].
pub(crate) unsafe fn render_blob(face: &hr_face_t) -> Option<&Blob> {
    // SAFETY: `render_data` has this function's contract, for the same face.
    unsafe { render_data(face) }.map(|data| &data.blob)
}
/// A view of the face's assembled SFNT.
///
/// # Safety
///
/// As [`render_data`].
pub(crate) unsafe fn font_ref(face: &hr_face_t) -> Option<FontRef<'_>> {
    // SAFETY: `render_blob` has this function's contract, for the same face.
    FontRef::new(unsafe { render_blob(face) }?).ok()
}
/// The face's outlines, built on first use and kept with the face.
///
/// # Safety
///
/// As [`render_data`].
pub(crate) unsafe fn outlines(face: &hr_face_t) -> Option<&skrifa::OutlineGlyphCollection<'_>> {
    // SAFETY: `render_data` has this function's contract, for the same face.
    unsafe { render_data(face) }?.outlines.as_ref()
}
/// Copies the tables Skrifa reads out of a face and assembles them into an
/// SFNT, or `None` if the face has none of them or they add up to more than
/// its blob could hold.
///
/// # Safety
///
/// `face` must be `NULL` or a live face. It is used for shared access only,
/// so it may be a pointer made from a shared borrow of the face.
unsafe fn collect_tables(face: *mut hr_face_t) -> Option<Vec<u8>> {
    const TAGS: [&[u8; 4]; 24] = [
        b"head", b"maxp", b"hhea", b"hmtx", b"vhea", b"vmtx", b"VORG", b"glyf", b"loca", b"gvar",
        b"HVAR", b"VVAR", b"CFF ", b"CFF2", b"VARC", b"fvar", b"avar", b"COLR", b"CPAL", b"CBLC",
        b"CBDT", b"sbix", b"SVG ", b"cmap",
    ];
    // A directory whose records overlap can list the same bytes under every
    // tag. Charge each table to the face's blob so that copying them cannot
    // cost more than one copy of the font; a callback face supplies its own
    // tables and has no such bound.
    //
    // SAFETY: `face` is `NULL` or a live face by this function's contract.
    let mut budget = unsafe { object::or_empty(face.cast_const()) }
        .blob_len()
        .unwrap_or(usize::MAX);
    let mut tables = Vec::new();
    for bytes in TAGS {
        let tag = u32::from_be_bytes(*bytes);
        // SAFETY: `face` is `NULL` or a live face by this function's
        // contract. Fetching a table takes no more than shared access to the
        // face, as it has to for the README to let several threads use one
        // face at once, so this keeps to what the contract says of `face`.
        // The reference that comes back is this function's to release.
        let blob = unsafe { hr_face_reference_table(face, tag) };
        let mut len = 0;
        // SAFETY: `blob` is live: `hr_face_reference_table` returned it
        // with a reference that is not released until below. `len` is a
        // local for the length to be written to.
        let data = unsafe { hr_blob_get_data(blob, &raw mut len) };
        let Some(left) = budget.checked_sub(len as usize) else {
            // SAFETY: `blob` is live, and the reference released is the one
            // `hr_face_reference_table` gave this function, which nothing
            // has released yet.
            unsafe { hr_blob_destroy(blob) };
            return None;
        };
        budget = left;
        if len != 0 && !data.is_null() {
            tables.push((
                tag,
                // SAFETY: `from_raw_parts` needs `data` non-null, which was
                // just checked, with `len` initialized bytes behind it that
                // stay allocated and unwritten while the slice is used.
                // `hr_blob_get_data` returned the blob's data and wrote its
                // length to `len`. Blobs are read-only, and `hr_blob_create`
                // has data a blob borrows stay unchanged for as long as the
                // blob does. This one is kept live by the reference released
                // below, after `to_vec` has copied the bytes out. A byte
                // pointer is always aligned.
                unsafe { core::slice::from_raw_parts(data.cast::<u8>(), len as usize) }.to_vec(),
            ));
        }
        // SAFETY: as in the early return above. This is the other way out
        // of the loop body, so the reference is released once.
        unsafe { hr_blob_destroy(blob) };
    }
    if tables.is_empty() {
        return None;
    }
    tables.sort_by_key(|(tag, _)| *tag);
    let count = tables.len() as u16;
    let mut output = vec![0; 12 + 16 * tables.len()];
    let version = if tables.iter().any(|(tag, _)| {
        *tag == u32::from_be_bytes(*b"CFF ") || *tag == u32::from_be_bytes(*b"CFF2")
    }) {
        u32::from_be_bytes(*b"OTTO")
    } else {
        0x0001_0000
    };
    output[..4].copy_from_slice(&version.to_be_bytes());
    output[4..6].copy_from_slice(&count.to_be_bytes());
    let selector = count.ilog2() as u16;
    let search = (1u16 << selector) * 16;
    output[6..8].copy_from_slice(&search.to_be_bytes());
    output[8..10].copy_from_slice(&selector.to_be_bytes());
    output[10..12].copy_from_slice(&(count * 16 - search).to_be_bytes());
    for (i, (tag, data)) in tables.iter().enumerate() {
        let at = 12 + 16 * i;
        let offset = u32::try_from(output.len()).ok()?;
        let length = u32::try_from(data.len()).ok()?;
        let checksum = data.chunks(4).fold(0u32, |sum, chunk| {
            let mut b = [0; 4];
            b[..chunk.len()].copy_from_slice(chunk);
            sum.wrapping_add(u32::from_be_bytes(b))
        });
        for (pos, value) in [
            (at, *tag),
            (at + 4, checksum),
            (at + 8, offset),
            (at + 12, length),
        ] {
            output[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
        }
        output.extend_from_slice(data);
        output.resize((output.len() + 3) & !3, 0);
    }
    Some(output)
}
/// The font's normalized variation coordinates, as Skrifa takes them.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
pub(crate) unsafe fn coords(font: *mut hr_font_t) -> Vec<F2Dot14> {
    let mut len = 0;
    // SAFETY: `font` is `NULL` or a live font by this function's contract,
    // and `len` is a local for the count to be written to.
    let data = unsafe { hr_font_get_var_coords_normalized(font, &raw mut len) };
    if len == 0 || data.is_null() {
        return Vec::new();
    }
    // SAFETY: `from_raw_parts` needs `data` non-null and aligned, with `len`
    // initialized coordinates behind it that stay allocated and unwritten
    // while the slice is used. `hr_font_get_var_coords_normalized` returned
    // the font's coordinates, not `NULL` as just checked, and wrote their
    // count to `len`; they stay as they are until the font's variation
    // settings change or it is freed. The font is live, the README has
    // nothing change a font that is in use, and the slice is copied out by
    // the end of this statement.
    unsafe { core::slice::from_raw_parts(data, len as usize) }
        .iter()
        .map(|v| F2Dot14::from_bits(*v as i16))
        .collect()
}
/// The font's scale as a multiple of its face's units per em, across and
/// up.
///
/// # Safety
///
/// `font` must be `NULL` or a live font.
pub(crate) unsafe fn scale(font: *mut hr_font_t) -> (f32, f32) {
    let mut x = 0;
    let mut y = 0;
    // SAFETY: `font` is `NULL` or a live font by this function's contract,
    // and `x` and `y` are locals for the scale to be written to.
    unsafe { hr_font_get_scale(font, &raw mut x, &raw mut y) };
    // SAFETY: `hr_font_get_face` takes `font` on the same terms. What it
    // returns is the font's face, which is live for as long as the font is,
    // or the empty face for no font, and `hr_face_get_upem` takes either.
    let upem = unsafe { hr_face_get_upem(hr_font_get_face(font)) }.max(1) as f32;
    (x as f32 / upem, y as f32 / upem)
}

// A parent callback draws in the parent's scale. Forward each segment at the
// child's scale, retaining correct path state at the public callback boundary.
pub(crate) struct Pen {
    // Safety invariant: for as long as the pen exists, `funcs` is `NULL` or
    // live and unmodified, and `data` is draw data its callbacks may be
    // called with.
    funcs: *mut hr_draw_funcs_t,
    data: *mut c_void,
    state: hr_draw_state_t,
    x: f32,
    y: f32,
    start: Option<(f32, f32)>,
    current: (f32, f32),
}
impl Pen {
    /// Makes a pen that draws to `funcs`, scaling horizontal coordinates by
    /// `x` and vertical ones by `y`.
    ///
    /// # Safety
    ///
    /// Until the pen is dropped, `funcs` must be `NULL` or live and must not
    /// be modified, and `data` must be draw data its callbacks may be called
    /// with.
    pub(crate) unsafe fn new(
        funcs: *mut hr_draw_funcs_t,
        data: *mut c_void,
        x: f32,
        y: f32,
    ) -> Self {
        Self {
            funcs,
            data,
            state: hr_draw_state_t::default(),
            x,
            y,
            start: None,
            current: (0.0, 0.0),
        }
    }
}
impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.current = (x, y);
        // SAFETY: `hr_draw_move_to` takes `NULL` or live callbacks that stay
        // unmodified while it runs, draw data to call them with, and a state
        // it may read and write. The invariant on `funcs` and `data` gives
        // the first two, and the state is the pen's own, borrowed exclusively
        // here.
        unsafe {
            hr_draw_move_to(
                self.funcs,
                self.data,
                &raw mut self.state,
                x * self.x,
                y * self.y,
            );
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.start.get_or_insert(self.current);
        self.current = (x, y);
        // SAFETY: as in `move_to`.
        unsafe {
            hr_draw_line_to(
                self.funcs,
                self.data,
                &raw mut self.state,
                x * self.x,
                y * self.y,
            );
        }
    }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.start.get_or_insert(self.current);
        self.current = (x, y);
        // SAFETY: as in `move_to`.
        unsafe {
            hr_draw_quadratic_to(
                self.funcs,
                self.data,
                &raw mut self.state,
                a * self.x,
                b * self.y,
                x * self.x,
                y * self.y,
            );
        }
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.start.get_or_insert(self.current);
        self.current = (x, y);
        // SAFETY: as in `move_to`.
        unsafe {
            hr_draw_cubic_to(
                self.funcs,
                self.data,
                &raw mut self.state,
                a * self.x,
                b * self.y,
                c * self.x,
                d * self.y,
                x * self.x,
                y * self.y,
            );
        }
    }
    #[allow(clippy::float_cmp)] // Preserve contour topology even when an axis scale is zero.
    fn close(&mut self) {
        if let Some((x, y)) = self.start.take() {
            if self.current != (x, y) {
                // SAFETY: as in `move_to`.
                unsafe {
                    hr_draw_line_to(
                        self.funcs,
                        self.data,
                        &raw mut self.state,
                        x * self.x,
                        y * self.y,
                    );
                }
            }
        }
        // SAFETY: as in `move_to`.
        unsafe { hr_draw_close_path(self.funcs, self.data, &raw mut self.state) }
    }
}
impl Drop for Pen {
    fn drop(&mut self) {
        skrifa::outline::OutlinePen::close(self);
    }
}
