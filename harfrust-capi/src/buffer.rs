//! Buffers of text and of the glyphs shaping produces. Mirrors HarfBuzz's
//! `hb-buffer.h`.

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::OnceLock;

use harfrust::{
    Buffer, BufferClusterLevel, BufferContentType, BufferFlags, GlyphInfo, GlyphPosition,
    SerializeFlags,
};

use crate::common::{direction_from_rust, direction_to_rust, hr_direction_t, write_c_string};
use crate::common::{
    hr_bool_t, hr_codepoint_t, hr_language_t, hr_mask_t, hr_position_t, hr_script_t,
    language_from_rust, language_to_rust, script_from_rust, script_to_rust,
};
use crate::font::hr_font_t;
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

// HarfBuzz buffers retain at most five context codepoints. Four bytes per
// codepoint are enough to cover those without validating an arbitrarily long
// prefix or suffix passed to hr_buffer_add_utf8.
const MAX_CONTEXT_UTF8_BYTES: usize = 5 * 4;

/// What a buffer currently holds.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum hr_buffer_content_type_t {
    /// Nothing, or contents that have been cleared.
    HR_BUFFER_CONTENT_TYPE_INVALID = 0,
    /// Input characters, ready to be shaped.
    HR_BUFFER_CONTENT_TYPE_UNICODE = 1,
    /// The glyphs shaping produced.
    HR_BUFFER_CONTENT_TYPE_GLYPHS = 2,
}

/// Flags controlling how a buffer is shaped.
///
/// These are combined with a bitwise or, so this is an integer typedef rather
/// than an enumeration.
pub type hr_buffer_flags_t = u32;

/// No flags set.
pub const HR_BUFFER_FLAG_DEFAULT: hr_buffer_flags_t = 0x0000_0000;
/// The buffer holds the start of a paragraph.
pub const HR_BUFFER_FLAG_BOT: hr_buffer_flags_t = 0x0000_0001;
/// The buffer holds the end of a paragraph.
pub const HR_BUFFER_FLAG_EOT: hr_buffer_flags_t = 0x0000_0002;
/// Show default-ignorable characters using the font's own glyphs.
pub const HR_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES: hr_buffer_flags_t = 0x0000_0004;
/// Remove default-ignorable characters entirely.
pub const HR_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES: hr_buffer_flags_t = 0x0000_0008;
/// Do not insert a dotted circle around invalid sequences.
pub const HR_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE: hr_buffer_flags_t = 0x0000_0010;
/// Verify the shaping result. Accepted but not yet acted on.
pub const HR_BUFFER_FLAG_VERIFY: hr_buffer_flags_t = 0x0000_0020;
/// Produce the unsafe-to-concat glyph flag, which costs extra work.
pub const HR_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT: hr_buffer_flags_t = 0x0000_0040;
/// Produce the safe-to-insert-tatweel glyph flag.
pub const HR_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL: hr_buffer_flags_t = 0x0000_0080;
/// Every flag defined above.
pub const HR_BUFFER_FLAG_DEFINED: hr_buffer_flags_t = 0x0000_00FF;

/// How clusters are merged during shaping.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum hr_buffer_cluster_level_t {
    /// Merge clusters by grapheme, keeping cluster values monotonic.
    HR_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES = 0,
    /// Merge clusters by character, keeping cluster values monotonic.
    HR_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS = 1,
    /// Do not merge clusters, and do not keep cluster values monotonic.
    HR_BUFFER_CLUSTER_LEVEL_CHARACTERS = 2,
    /// Merge clusters by grapheme without keeping cluster values monotonic.
    HR_BUFFER_CLUSTER_LEVEL_GRAPHEMES = 3,
}

/// Flags attached to an individual glyph by shaping.
///
/// These are combined with a bitwise or, so this is an integer typedef rather
/// than an enumeration.
pub type hr_glyph_flags_t = u32;

/// Breaking the text at this cluster requires reshaping both sides.
pub const HR_GLYPH_FLAG_UNSAFE_TO_BREAK: hr_glyph_flags_t = 0x0000_0001;
/// Changing the text on one side of this cluster may change the other.
pub const HR_GLYPH_FLAG_UNSAFE_TO_CONCAT: hr_glyph_flags_t = 0x0000_0002;
/// A tatweel may be inserted before this cluster to elongate the run.
pub const HR_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL: hr_glyph_flags_t = 0x0000_0004;
/// Every flag defined above.
pub const HR_GLYPH_FLAG_DEFINED: hr_glyph_flags_t = 0x0000_0007;

/// The format `hr_buffer_serialize_glyphs` writes.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum hr_buffer_serialize_format_t {
    /// A human-readable one-line form.
    HR_BUFFER_SERIALIZE_FORMAT_TEXT = 0x5445_5854,
    /// JSON. Not supported by this library.
    HR_BUFFER_SERIALIZE_FORMAT_JSON = 0x4A53_4F4E,
    /// An unrecognised format.
    HR_BUFFER_SERIALIZE_FORMAT_INVALID = 0x0000_0000,
}

/// Flags controlling what `hr_buffer_serialize_glyphs` includes.
///
/// These are combined with a bitwise or, so this is an integer typedef rather
/// than an enumeration.
pub type hr_buffer_serialize_flags_t = u32;

/// Include everything.
pub const HR_BUFFER_SERIALIZE_FLAG_DEFAULT: hr_buffer_serialize_flags_t = 0x0000_0000;
/// Leave out cluster values.
pub const HR_BUFFER_SERIALIZE_FLAG_NO_CLUSTERS: hr_buffer_serialize_flags_t = 0x0000_0001;
/// Leave out positions.
pub const HR_BUFFER_SERIALIZE_FLAG_NO_POSITIONS: hr_buffer_serialize_flags_t = 0x0000_0002;
/// Write glyph indices rather than names.
pub const HR_BUFFER_SERIALIZE_FLAG_NO_GLYPH_NAMES: hr_buffer_serialize_flags_t = 0x0000_0004;
/// Include each glyph's ink extents.
pub const HR_BUFFER_SERIALIZE_FLAG_GLYPH_EXTENTS: hr_buffer_serialize_flags_t = 0x0000_0008;
/// Include each glyph's flags.
pub const HR_BUFFER_SERIALIZE_FLAG_GLYPH_FLAGS: hr_buffer_serialize_flags_t = 0x0000_0010;
/// Leave out advances, making offsets absolute.
pub const HR_BUFFER_SERIALIZE_FLAG_NO_ADVANCES: hr_buffer_serialize_flags_t = 0x0000_0020;
/// Every flag defined above.
pub const HR_BUFFER_SERIALIZE_FLAG_DEFINED: hr_buffer_serialize_flags_t = 0x0000_003F;

/// One item in a buffer: an input character before shaping, a glyph after.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct hr_glyph_info_t {
    /// The input codepoint before shaping, the glyph index after.
    pub codepoint: hr_codepoint_t,
    /// Reserved for internal use.
    pub mask: hr_mask_t,
    /// Index into the input text of the cluster this item belongs to.
    pub cluster: u32,
    /// Reserved for internal use.
    pub var1: u32,
    /// Reserved for internal use.
    pub var2: u32,
}

/// The position of one glyph, relative to the current point.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct hr_glyph_position_t {
    /// How far the pen moves after this glyph, when setting horizontally.
    pub x_advance: hr_position_t,
    /// How far the pen moves after this glyph, when setting vertically.
    pub y_advance: hr_position_t,
    /// How far this glyph shifts on the x-axis before being drawn.
    pub x_offset: hr_position_t,
    /// How far this glyph shifts on the y-axis before being drawn.
    pub y_offset: hr_position_t,
    /// Reserved for internal use.
    pub var: u32,
}

// These are handed out as pointers straight into HarfRust's own storage, so
// their layouts have to agree exactly.
const _: () = assert!(size_of::<hr_glyph_info_t>() == size_of::<GlyphInfo>());
const _: () = assert!(align_of::<hr_glyph_info_t>() == align_of::<GlyphInfo>());
const _: () = assert!(size_of::<hr_glyph_position_t>() == size_of::<GlyphPosition>());
const _: () = assert!(align_of::<hr_glyph_position_t>() == align_of::<GlyphPosition>());

/// A `NULL`-terminated array of static C strings, safe to share across
/// threads because the strings it points at are immutable and never freed.
pub(crate) struct CStrArray<const N: usize>(pub(crate) [*const c_char; N]);

// SAFETY: the contents are pointers to string literals with static storage.
unsafe impl<const N: usize> Sync for CStrArray<N> {}

/// A buffer of text to shape, and of the glyphs shaping produces.
pub struct hr_buffer_t {
    header: ObjectHeader,
    pub(crate) buffer: Buffer,
}

static EMPTY_BUFFER: OnceLock<Empty<hr_buffer_t>> = OnceLock::new();

impl Object for hr_buffer_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_BUFFER
            .get_or_init(|| {
                Empty::new(hr_buffer_t {
                    header: ObjectHeader::immortal(),
                    buffer: Buffer::new(),
                })
            })
            .get()
    }
}

fn content_type_to_rust(value: hr_buffer_content_type_t) -> Option<BufferContentType> {
    match value {
        hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_INVALID => None,
        hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_UNICODE => {
            Some(BufferContentType::Unicode)
        }
        hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_GLYPHS => Some(BufferContentType::Glyphs),
    }
}

fn content_type_from_rust(value: Option<BufferContentType>) -> hr_buffer_content_type_t {
    match value {
        None => hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_INVALID,
        Some(BufferContentType::Unicode) => {
            hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_UNICODE
        }
        Some(BufferContentType::Glyphs) => hr_buffer_content_type_t::HR_BUFFER_CONTENT_TYPE_GLYPHS,
    }
}

fn cluster_level_to_rust(value: hr_buffer_cluster_level_t) -> BufferClusterLevel {
    match value {
        hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES => {
            BufferClusterLevel::MonotoneGraphemes
        }
        hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS => {
            BufferClusterLevel::MonotoneCharacters
        }
        hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_CHARACTERS => {
            BufferClusterLevel::Characters
        }
        hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_GRAPHEMES => {
            BufferClusterLevel::Graphemes
        }
    }
}

fn cluster_level_from_rust(value: BufferClusterLevel) -> hr_buffer_cluster_level_t {
    match value {
        BufferClusterLevel::MonotoneGraphemes => {
            hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES
        }
        BufferClusterLevel::MonotoneCharacters => {
            hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS
        }
        BufferClusterLevel::Characters => {
            hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_CHARACTERS
        }
        BufferClusterLevel::Graphemes => {
            hr_buffer_cluster_level_t::HR_BUFFER_CLUSTER_LEVEL_GRAPHEMES
        }
    }
}

/// Creates an empty buffer.
#[no_mangle]
pub extern "C" fn hr_buffer_create() -> *mut hr_buffer_t {
    object::create(hr_buffer_t {
        header: ObjectHeader::new(),
        buffer: Buffer::new(),
    })
}

/// Creates an empty buffer carrying the same properties as `src`.
///
/// # Safety
///
/// `src` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_create_similar(src: *mut hr_buffer_t) -> *mut hr_buffer_t {
    let created = hr_buffer_create();
    let (Some(src), Some(dst)) = (unsafe { src.as_ref() }, unsafe { created.as_mut() }) else {
        return created;
    };
    dst.buffer.set_direction(src.buffer.direction());
    dst.buffer.set_script(src.buffer.script());
    if let Some(language) = src.buffer.language() {
        dst.buffer.set_language(language);
    }
    dst.buffer.set_flags(src.buffer.flags());
    dst.buffer.set_cluster_level(src.buffer.cluster_level());
    dst.buffer.set_invisible_glyph(src.buffer.invisible_glyph());
    dst.buffer
        .set_not_found_variation_selector_glyph(src.buffer.not_found_variation_selector_glyph());
    created
}

/// Returns the immortal empty buffer.
#[no_mangle]
pub extern "C" fn hr_buffer_get_empty() -> *mut hr_buffer_t {
    hr_buffer_t::empty()
}

/// Increments a buffer's reference count.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reference(buffer: *mut hr_buffer_t) -> *mut hr_buffer_t {
    unsafe { object::reference(buffer) }
}

/// Decrements a buffer's reference count, freeing it at zero.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and the caller must own the
/// reference being released.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_destroy(buffer: *mut hr_buffer_t) {
    unsafe { object::destroy(buffer) };
}

/// Attaches user data to a buffer.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_user_data(
    buffer: *mut hr_buffer_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(buffer, key, data, destroy, replace != 0) }.into()
}

/// Retrieves user data previously attached to a buffer.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_user_data(
    buffer: *mut hr_buffer_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(buffer, key) }
}

/// Clears a buffer's contents and resets its properties to their defaults.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reset(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.reset();
    }
}

/// Clears a buffer's contents, keeping its properties and its allocation.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_clear_contents(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.clear();
    }
}

/// Grows a buffer so it can hold at least `size` items without reallocating.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_pre_allocate(
    buffer: *mut hr_buffer_t,
    size: c_uint,
) -> hr_bool_t {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return false.into();
    };
    buffer.buffer.reserve(size as usize).into()
}

/// Returns whether every allocation on a buffer has succeeded.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_allocation_successful(buffer: *mut hr_buffer_t) -> hr_bool_t {
    unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .allocation_successful()
        .into()
}

/// Appends one codepoint to a buffer with the given cluster value.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_add(
    buffer: *mut hr_buffer_t,
    codepoint: hr_codepoint_t,
    cluster: c_uint,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.push(codepoint, cluster);
    }
}

/// Resolves HarfBuzz's `item_offset` / `item_length` convention against a
/// sequence of `total` code units.
///
/// A negative length means "to the end". Returns the half-open item range.
fn item_range(total: usize, item_offset: c_uint, item_length: c_int) -> (usize, usize) {
    let start = (item_offset as usize).min(total);
    let end = if item_length < 0 {
        total
    } else {
        start.saturating_add(item_length as usize).min(total)
    };
    (start, end)
}

/// Appends decoded UTF-8 in one allocation, adjusting the relative cluster
/// values produced by `Buffer::push_str` to HarfBuzz's absolute offsets.
fn append_utf8(buffer: &mut Buffer, text: &str, cluster_offset: c_uint) {
    let old_len = buffer.len();
    buffer.push_str(text);
    if cluster_offset != 0 {
        for info in &mut buffer.glyph_infos_mut()[old_len..] {
            info.cluster = info.cluster.wrapping_add(cluster_offset);
        }
    }
}

/// Appends UTF-8 text to a buffer.
///
/// Only `text[item_offset .. item_offset + item_length]` is added; the text
/// around it becomes shaping context. A negative `item_length` means "to the
/// end", and a negative `text_length` means `text` is NUL-terminated. Cluster
/// values are byte offsets into `text`.
///
/// # Safety
///
/// `text` must point to `text_length` readable bytes, or be NUL-terminated
/// when `text_length` is negative.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_add_utf8(
    buffer: *mut hr_buffer_t,
    text: *const c_char,
    text_length: c_int,
    item_offset: c_uint,
    item_length: c_int,
) {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    if text.is_null() {
        return;
    }
    let bytes = if text_length < 0 {
        unsafe { core::ffi::CStr::from_ptr(text) }.to_bytes()
    } else {
        unsafe { core::slice::from_raw_parts(text.cast::<u8>(), text_length as usize) }
    };
    let (start, end) = item_range(bytes.len(), item_offset, item_length);
    // Borrowed for well-formed text, which is the whole point: this is the
    // hottest call in the API and it should not allocate.
    let decode = String::from_utf8_lossy;

    if start > 0 {
        let context = &bytes[..start];
        let context = &context[context.len().saturating_sub(MAX_CONTEXT_UTF8_BYTES)..];
        buffer.buffer.set_pre_context(&decode(context));
    }
    if end < bytes.len() {
        let context = &bytes[end..];
        let context = &context[..context.len().min(MAX_CONTEXT_UTF8_BYTES)];
        buffer.buffer.set_post_context(&decode(context));
    }
    // Cluster values are offsets into the whole text, not into the item.
    //
    // Well-formed text is the overwhelmingly common case, and validating it
    // outright is faster than the lossy decoder, which walks the bytes once to
    // find the ill-formed sequences and then again to yield the characters.
    let item = &bytes[start..end];
    match core::str::from_utf8(item) {
        Ok(text) => append_utf8(&mut buffer.buffer, text, start as c_uint),
        Err(_) => append_utf8(&mut buffer.buffer, &decode(item), start as c_uint),
    }
}

/// Appends UTF-32 codepoints to a buffer.
///
/// Follows the same `item_offset` / `item_length` convention as
/// [`hr_buffer_add_utf8`], with cluster values counted in codepoints.
///
/// # Safety
///
/// `text` must point to `text_length` readable codepoints, or be terminated
/// by a zero when `text_length` is negative.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_add_utf32(
    buffer: *mut hr_buffer_t,
    text: *const u32,
    text_length: c_int,
    item_offset: c_uint,
    item_length: c_int,
) {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    if text.is_null() {
        return;
    }
    let len = if text_length < 0 {
        let mut len = 0usize;
        // SAFETY: the caller guarantees a zero terminator.
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        len
    } else {
        text_length as usize
    };
    let items = unsafe { core::slice::from_raw_parts(text, len) };
    let (start, end) = item_range(len, item_offset, item_length);

    if start > 0 {
        buffer
            .buffer
            .set_pre_context_codepoints(&items[..start].iter().copied().rev().collect::<Vec<_>>());
    }
    if end < len {
        buffer.buffer.set_post_context_codepoints(&items[end..]);
    }
    for (index, &codepoint) in items[start..end].iter().enumerate() {
        buffer.buffer.push(codepoint, (start + index) as c_uint);
    }
}

/// Appends codepoints to a buffer.
///
/// Identical to [`hr_buffer_add_utf32`]; both are provided to match HarfBuzz.
///
/// # Safety
///
/// See [`hr_buffer_add_utf32`].
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_add_codepoints(
    buffer: *mut hr_buffer_t,
    text: *const hr_codepoint_t,
    text_length: c_int,
    item_offset: c_uint,
    item_length: c_int,
) {
    unsafe { hr_buffer_add_utf32(buffer, text, text_length, item_offset, item_length) };
}

/// Appends a range of one buffer's items to another.
///
/// # Safety
///
/// Both buffers must be `NULL` or live, and must not be the same buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_append(
    buffer: *mut hr_buffer_t,
    source: *const hr_buffer_t,
    start: c_uint,
    end: c_uint,
) {
    if core::ptr::eq(buffer.cast_const(), source) {
        return;
    }
    let Some(source) = (unsafe { source.as_ref() }) else {
        return;
    };
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    let infos = source.buffer.glyph_infos();
    let start = (start as usize).min(infos.len());
    let end = (end as usize).clamp(start, infos.len());
    if start == end {
        return;
    }
    buffer.buffer.push_glyph_infos(&infos[start..end]);
}

/// Returns what a buffer currently holds.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_content_type(
    buffer: *mut hr_buffer_t,
) -> hr_buffer_content_type_t {
    content_type_from_rust(
        unsafe { object::or_empty(buffer.cast_const()) }
            .buffer
            .content_type(),
    )
}

/// Relabels what a buffer holds, without clearing it.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_content_type(
    buffer: *mut hr_buffer_t,
    content_type: hr_buffer_content_type_t,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer
            .buffer
            .set_content_type(content_type_to_rust(content_type));
    }
}

/// Sets a buffer's text direction.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_direction(
    buffer: *mut hr_buffer_t,
    direction: hr_direction_t,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.set_direction(direction_to_rust(direction));
    }
}

/// Returns a buffer's text direction.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_direction(buffer: *mut hr_buffer_t) -> hr_direction_t {
    direction_from_rust(
        unsafe { object::or_empty(buffer.cast_const()) }
            .buffer
            .direction(),
    )
}

/// Sets a buffer's script.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_script(buffer: *mut hr_buffer_t, script: hr_script_t) {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    if let Some(script) = script_to_rust(script) {
        buffer.buffer.set_script(script);
    }
}

/// Returns a buffer's script.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_script(buffer: *mut hr_buffer_t) -> hr_script_t {
    script_from_rust(
        unsafe { object::or_empty(buffer.cast_const()) }
            .buffer
            .script(),
    )
}

/// Sets a buffer's language.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `language` must be `NULL` or
/// a value returned by `hr_language_from_string`.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_language(buffer: *mut hr_buffer_t, language: hr_language_t) {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    if let Some(language) = unsafe { language_to_rust(language) } {
        buffer.buffer.set_language(language);
    }
}

/// Returns a buffer's language, or `NULL` if it has none.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_language(buffer: *mut hr_buffer_t) -> hr_language_t {
    language_from_rust(
        unsafe { object::or_empty(buffer.cast_const()) }
            .buffer
            .language(),
    )
}

/// Sets a buffer's flags.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_flags(buffer: *mut hr_buffer_t, flags: hr_buffer_flags_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer
            .buffer
            .set_flags(BufferFlags::from_bits_truncate(flags));
    }
}

/// Returns a buffer's flags.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_flags(buffer: *mut hr_buffer_t) -> hr_buffer_flags_t {
    unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .flags()
        .bits()
}

/// Sets a buffer's cluster level.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_cluster_level(
    buffer: *mut hr_buffer_t,
    cluster_level: hr_buffer_cluster_level_t,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer
            .buffer
            .set_cluster_level(cluster_level_to_rust(cluster_level));
    }
}

/// Returns a buffer's cluster level.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_cluster_level(
    buffer: *mut hr_buffer_t,
) -> hr_buffer_cluster_level_t {
    cluster_level_from_rust(
        unsafe { object::or_empty(buffer.cast_const()) }
            .buffer
            .cluster_level(),
    )
}

/// Returns the number of items in a buffer.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_length(buffer: *mut hr_buffer_t) -> c_uint {
    unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .len() as c_uint
}

/// Sets the number of items in a buffer, zero-filling any growth.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_length(
    buffer: *mut hr_buffer_t,
    length: c_uint,
) -> hr_bool_t {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return false.into();
    };
    buffer.buffer.set_length(length as usize).into()
}

/// Returns a buffer's items, writing their count to `length`.
///
/// Before shaping, each item's `codepoint` holds an input character. The
/// pointer stays valid until the buffer's contents change.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `length` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_glyph_infos(
    buffer: *mut hr_buffer_t,
    length: *mut c_uint,
) -> *mut hr_glyph_info_t {
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        if let Some(out) = unsafe { length.as_mut() } {
            *out = 0;
        }
        return core::ptr::null_mut();
    };
    let infos = buffer.buffer.glyph_infos_mut();
    if let Some(out) = unsafe { length.as_mut() } {
        *out = infos.len() as c_uint;
    }
    // The layouts are asserted to match above.
    infos.as_mut_ptr().cast::<hr_glyph_info_t>()
}

/// Returns a buffer's glyph positions, writing their count to `length`.
///
/// Positions are allocated and zeroed on demand if the buffer has not been
/// shaped. The pointer stays valid until the buffer's contents change.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `length` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_glyph_positions(
    buffer: *mut hr_buffer_t,
    length: *mut c_uint,
) -> *mut hr_glyph_position_t {
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        if let Some(out) = unsafe { length.as_mut() } {
            *out = 0;
        }
        return core::ptr::null_mut();
    };
    let positions = buffer.buffer.glyph_positions_mut();
    if let Some(out) = unsafe { length.as_mut() } {
        *out = positions.len() as c_uint;
    }
    positions.as_mut_ptr().cast::<hr_glyph_position_t>()
}

/// Returns whether a buffer has glyph positions, which is true after shaping.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_has_positions(buffer: *mut hr_buffer_t) -> hr_bool_t {
    (!unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .glyph_positions()
        .is_empty())
    .into()
}

/// Returns the flags shaping attached to one item.
///
/// # Safety
///
/// `info` must be `NULL` or point to a readable item.
#[no_mangle]
pub unsafe extern "C" fn hr_glyph_info_get_glyph_flags(
    info: *const hr_glyph_info_t,
) -> hr_glyph_flags_t {
    unsafe { info.as_ref() }.map_or(0, |info| info.mask & HR_GLYPH_FLAG_DEFINED)
}

/// Sets the glyph used in place of invisible characters.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_invisible_glyph(
    buffer: *mut hr_buffer_t,
    invisible: hr_codepoint_t,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        let glyph = (invisible != 0).then(|| harfrust::GlyphId::from(invisible));
        buffer.buffer.set_invisible_glyph(glyph);
    }
}

/// Returns the glyph used in place of invisible characters, or zero.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_invisible_glyph(buffer: *mut hr_buffer_t) -> hr_codepoint_t {
    unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .invisible_glyph()
        .map_or(0, harfrust::GlyphId::to_u32)
}

/// Sets the glyph used in place of variation selectors the font lacks.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_not_found_variation_selector_glyph(
    buffer: *mut hr_buffer_t,
    glyph: hr_codepoint_t,
) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer
            .buffer
            .set_not_found_variation_selector_glyph(Some(glyph));
    }
}

/// Returns the glyph used in place of variation selectors the font lacks.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_not_found_variation_selector_glyph(
    buffer: *mut hr_buffer_t,
) -> hr_codepoint_t {
    unsafe { object::or_empty(buffer.cast_const()) }
        .buffer
        .not_found_variation_selector_glyph()
        .unwrap_or(0)
}

/// Writes a buffer's direction, script and language into `props`.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `props` must be `NULL` or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_get_segment_properties(
    buffer: *mut hr_buffer_t,
    props: *mut crate::shape_plan::hr_segment_properties_t,
) {
    let Some(out) = (unsafe { props.as_mut() }) else {
        return;
    };
    let buffer = unsafe { object::or_empty(buffer.cast_const()) };
    *out = crate::shape_plan::hr_segment_properties_t {
        direction: direction_from_rust(buffer.buffer.direction()),
        script: script_from_rust(buffer.buffer.script()),
        language: language_from_rust(buffer.buffer.language()),
        reserved1: core::ptr::null_mut(),
        reserved2: core::ptr::null_mut(),
    };
}

/// Sets a buffer's direction, script and language at once.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer, and `props` must be `NULL` or
/// readable.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_set_segment_properties(
    buffer: *mut hr_buffer_t,
    props: *const crate::shape_plan::hr_segment_properties_t,
) {
    let (Some(buffer), Some(props)) = (unsafe { object::as_mutable(buffer) }, unsafe {
        props.as_ref()
    }) else {
        return;
    };
    let (direction, script, language) = props.to_rust();
    buffer.buffer.set_direction(direction);
    if let Some(script) = script {
        buffer.buffer.set_script(script);
    }
    if let Some(language) = language {
        buffer.buffer.set_language(language);
    }
}

/// Fills in whichever of direction, script and language are still unset, by
/// inspecting the buffer's contents.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_guess_segment_properties(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.guess_segment_properties();
    }
}

/// Reverses a buffer's contents.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reverse(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.reverse();
    }
}

/// Reverses part of a buffer's contents.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reverse_range(
    buffer: *mut hr_buffer_t,
    start: c_uint,
    end: c_uint,
) {
    let Some(buffer) = (unsafe { object::as_mutable(buffer) }) else {
        return;
    };
    let len = buffer.buffer.len();
    let start = (start as usize).min(len);
    let end = (end as usize).clamp(start, len);
    buffer.buffer.reverse_range(start, end);
}

/// Reverses a buffer's contents, keeping each cluster's items in order.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reverse_clusters(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.reverse_clusters();
    }
}

/// Resets every item's cluster value to its index.
///
/// # Safety
///
/// `buffer` must be `NULL` or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_reset_clusters(buffer: *mut hr_buffer_t) {
    if let Some(buffer) = unsafe { object::as_mutable(buffer) } {
        buffer.buffer.reset_clusters();
    }
}

/// Returns the serialization format matching a name, such as `"text"`.
///
/// # Safety
///
/// See `hr_tag_from_string`.
#[no_mangle]
pub unsafe extern "C" fn hr_buffer_serialize_format_from_string(
    str_: *const c_char,
    len: c_int,
) -> hr_buffer_serialize_format_t {
    let Some(s) = (unsafe { crate::common::str_from_raw(str_, len) }) else {
        return hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_INVALID;
    };
    match s.to_ascii_lowercase().as_str() {
        "text" => hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_TEXT,
        "json" => hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_JSON,
        _ => hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_INVALID,
    }
}

/// Returns the name of a serialization format, or `NULL` if it is invalid.
#[no_mangle]
pub extern "C" fn hr_buffer_serialize_format_to_string(
    format: hr_buffer_serialize_format_t,
) -> *const c_char {
    let name: &[u8] = match format {
        hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_TEXT => b"text\0",
        hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_JSON => b"json\0",
        hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_INVALID => {
            return core::ptr::null()
        }
    };
    name.as_ptr().cast::<c_char>()
}

/// Returns the serialization formats this library supports, as a
/// `NULL`-terminated array of names.
#[no_mangle]
pub extern "C" fn hr_buffer_serialize_list_formats() -> *const *const c_char {
    static FORMATS: CStrArray<2> = CStrArray([c"text".as_ptr(), core::ptr::null()]);
    FORMATS.0.as_ptr()
}

/// Writes a human-readable form of `buffer[start..end]` into `buf`.
///
/// Returns the number of items serialized, writing the number of bytes used to
/// `buf_consumed`. Only
/// [`hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_TEXT`] is
/// supported; any other format serializes nothing.
///
/// # Safety
///
/// `buffer` and `font` must be `NULL` or live, `buf` must point to `buf_size`
/// writable bytes, and `buf_consumed` must be `NULL` or writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn hr_buffer_serialize_glyphs(
    buffer: *mut hr_buffer_t,
    start: c_uint,
    end: c_uint,
    buf: *mut c_char,
    buf_size: c_uint,
    buf_consumed: *mut c_uint,
    font: *mut hr_font_t,
    format: hr_buffer_serialize_format_t,
    flags: hr_buffer_serialize_flags_t,
) -> c_uint {
    let write_consumed = |len: c_uint| {
        if let Some(out) = unsafe { buf_consumed.as_mut() } {
            *out = len;
        }
    };
    write_consumed(0);

    if format != hr_buffer_serialize_format_t::HR_BUFFER_SERIALIZE_FORMAT_TEXT {
        return 0;
    }
    let buffer = unsafe { object::or_empty(buffer.cast_const()) };
    let font = unsafe { object::or_empty(font.cast_const()) };
    let Some(instance) = font.instance.as_deref() else {
        return 0;
    };

    let infos = buffer.buffer.glyph_infos();
    let start = (start as usize).min(infos.len());
    let end = (end as usize).clamp(start, infos.len());
    if start == end {
        return 0;
    }

    let flags = SerializeFlags::from_bits_truncate((flags & 0xFF) as u8);
    let text = if start == 0 && end == infos.len() {
        buffer.buffer.serialize(instance, flags)
    } else {
        // Serialize a copy holding just the requested range.
        let mut slice = Buffer::new();
        slice.push_glyph_infos(&infos[start..end]);
        let positions = buffer.buffer.glyph_positions();
        if !positions.is_empty() {
            slice
                .glyph_positions_mut()
                .copy_from_slice(&positions[start..end]);
        }
        slice.serialize(instance, flags)
    };

    unsafe { write_c_string(&text, buf, buf_size) };
    write_consumed(text.len().min(buf_size.saturating_sub(1) as usize) as c_uint);
    (end - start) as c_uint
}
