//! OpenType localized-name retrieval.
use crate::blob::hr_blob_destroy;
use crate::common::{hr_language_t, language_ref, Page};
use crate::face::{hr_face_reference_table, hr_face_t};
use crate::object;
use core::ffi::c_uint;
use read_fonts::{
    tables::name::{Name, NameRecord},
    FontData, Offset, TableProvider,
};

#[path = "ot_name_languages.rs"]
mod languages;

/// An OpenType name-table identifier.
pub type hr_ot_name_id_t = c_uint;
/// Predefined OpenType name-table identifiers.
pub type hr_ot_name_id_predefined_t = c_uint;

pub const HR_OT_NAME_ID_COPYRIGHT: hr_ot_name_id_predefined_t = 0;
pub const HR_OT_NAME_ID_FONT_FAMILY: hr_ot_name_id_predefined_t = 1;
pub const HR_OT_NAME_ID_FONT_SUBFAMILY: hr_ot_name_id_predefined_t = 2;
pub const HR_OT_NAME_ID_UNIQUE_ID: hr_ot_name_id_predefined_t = 3;
pub const HR_OT_NAME_ID_FULL_NAME: hr_ot_name_id_predefined_t = 4;
pub const HR_OT_NAME_ID_VERSION_STRING: hr_ot_name_id_predefined_t = 5;
pub const HR_OT_NAME_ID_POSTSCRIPT_NAME: hr_ot_name_id_predefined_t = 6;
pub const HR_OT_NAME_ID_TRADEMARK: hr_ot_name_id_predefined_t = 7;
pub const HR_OT_NAME_ID_MANUFACTURER: hr_ot_name_id_predefined_t = 8;
pub const HR_OT_NAME_ID_DESIGNER: hr_ot_name_id_predefined_t = 9;
pub const HR_OT_NAME_ID_DESCRIPTION: hr_ot_name_id_predefined_t = 10;
pub const HR_OT_NAME_ID_VENDOR_URL: hr_ot_name_id_predefined_t = 11;
pub const HR_OT_NAME_ID_DESIGNER_URL: hr_ot_name_id_predefined_t = 12;
pub const HR_OT_NAME_ID_LICENSE: hr_ot_name_id_predefined_t = 13;
pub const HR_OT_NAME_ID_LICENSE_URL: hr_ot_name_id_predefined_t = 14;
pub const HR_OT_NAME_ID_TYPOGRAPHIC_FAMILY: hr_ot_name_id_predefined_t = 16;
pub const HR_OT_NAME_ID_TYPOGRAPHIC_SUBFAMILY: hr_ot_name_id_predefined_t = 17;
pub const HR_OT_NAME_ID_MAC_FULL_NAME: hr_ot_name_id_predefined_t = 18;
pub const HR_OT_NAME_ID_SAMPLE_TEXT: hr_ot_name_id_predefined_t = 19;
pub const HR_OT_NAME_ID_CID_FINDFONT_NAME: hr_ot_name_id_predefined_t = 20;
pub const HR_OT_NAME_ID_WWS_FAMILY: hr_ot_name_id_predefined_t = 21;
pub const HR_OT_NAME_ID_WWS_SUBFAMILY: hr_ot_name_id_predefined_t = 22;
pub const HR_OT_NAME_ID_LIGHT_BACKGROUND: hr_ot_name_id_predefined_t = 23;
pub const HR_OT_NAME_ID_DARK_BACKGROUND: hr_ot_name_id_predefined_t = 24;
pub const HR_OT_NAME_ID_VARIATIONS_PS_PREFIX: hr_ot_name_id_predefined_t = 25;
pub const HR_OT_NAME_ID_INVALID: hr_ot_name_id_predefined_t = 0xFFFF;

fn platform_language<'a>(ltag: &'a [u8], record: &NameRecord) -> Option<&'a str> {
    let code = record.language_id();
    let map = match record.platform_id() {
        3 => languages::MS_LANGUAGES,
        1 => languages::MAC_LANGUAGES,
        0 => {
            // Unicode-platform name records use Apple's ltag table.
            let data = FontData::new(ltag);
            if data.read_at::<u32>(0).ok()? != 1 || code as u32 >= data.read_at::<u32>(8).ok()? {
                return None;
            }
            let start = data.read_at::<u16>(12 + code as usize * 4).ok()? as usize;
            // HarfBuzz reads at most 63 bytes of a tag. This runs once per
            // candidate record, so the font must not get to choose how much
            // of the table is validated each time.
            let len = (data.read_at::<u16>(14 + code as usize * 4).ok()? as usize).min(63);
            return core::str::from_utf8(data.as_bytes().get(start..start + len)?).ok();
        }
        _ => return None,
    };
    map.binary_search_by_key(&code, |entry| entry.0)
        .ok()
        .map(|index| map[index].1)
}

fn score(record: &NameRecord) -> Option<u8> {
    Some(match (record.platform_id(), record.encoding_id()) {
        (3, 10) => 0,
        (0, 6) => 1,
        (0, 4) => 2,
        (3, 1) => 3,
        (0, 3) => 4,
        (0, 2) => 5,
        (0, 1) => 6,
        (0, 0) => 7,
        (3, 0) => 8,
        (1, 0) => 10,
        _ => return None,
    })
}

fn select_name<'a>(
    table: &Name<'a>,
    ltag: &[u8],
    name_id: hr_ot_name_id_t,
    requested: &str,
) -> Option<(u8, &'a [u8])> {
    table
        .name_record()
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            if record.name_id().to_u16() as c_uint != name_id {
                return None;
            }
            let score = score(record)?;
            let language = platform_language(ltag, record)?;
            let exact = requested.eq_ignore_ascii_case(language);
            let fallback = requested
                .get(..language.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(language))
                && requested.as_bytes().get(language.len()) == Some(&b'-');
            if !exact && !fallback {
                return None;
            }
            // Prefer an exact language, then the best encoding, then record order.
            Some(((u8::from(!exact), score, index), record))
        })
        .min_by_key(|(key, _)| *key)
        .and_then(|((_, score, _), record)| {
            let start = record.string_offset().non_null().unwrap_or(0);
            let end = start + record.length() as usize;
            Some((score, table.string_data().as_bytes().get(start..end)?))
        })
}

/// Returns a localized name's full UTF-16 length, excluding the terminator.
///
/// A null language requests English. Exact language matches take precedence
/// over a matching parent language. With nonzero input capacity, copies a
/// complete-codepoint prefix, reserves one unit for a terminating zero, and
/// sets `text_size` to the units written, excluding that zero. With zero
/// capacity or a null count, only returns the required length. A null text
/// array copies no units and sets `text_size` to zero without reading it;
/// the README has the rule for a count and its array.
///
/// # Safety
/// `face` must be live; `language` must be null or interned;
/// `text_size` must be null or writable; `text` must hold the input capacity
/// when non-null.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_name_get_utf16(
    face: *mut hr_face_t,
    name_id: hr_ot_name_id_t,
    language: hr_language_t,
    text_size: *mut c_uint,
    text: *mut u16,
) -> c_uint {
    let raw_face = face;
    // SAFETY: `face` is `NULL`, or live by this function's contract.
    let face = unsafe { object::or_empty(face.cast_const()) };
    // SAFETY: `language` is `NULL` or interned by this function's contract.
    let language = unsafe { language_ref(language) };
    let requested = language.map_or("en", |language| language.as_str());
    let selected = face
        .font()
        .and_then(|font| font.tables().name().ok())
        .and_then(|table| {
            // ltag is outside HarfRust's shaping table cache; fetch it through
            // the face's generic table source so callback faces work too.
            // SAFETY: `raw_face` is `face` as the caller passed it: `NULL`, or
            // live by this function's contract. A face made from table
            // callbacks runs its callback here, while `face` and `table`
            // are borrowed; the README forbids a callback to modify or free
            // the object it is handed.
            let blob = unsafe { hr_face_reference_table(raw_face, u32::from_be_bytes(*b"ltag")) };
            // SAFETY: `blob` is the blob `hr_face_reference_table` just
            // returned, with a reference that is this code's until it is
            // released below, so it is live while `data` is used.
            let data = unsafe { object::or_empty(blob.cast_const()) };
            let selected = select_name(&table, data.bytes(), name_id, requested);
            // SAFETY: `blob` is live, and the reference released is the one
            // `hr_face_reference_table` handed this code to own. `data` is
            // not used again, and `selected` borrows from `table`, not from
            // the blob.
            unsafe { hr_blob_destroy(blob) };
            selected
        });
    // SAFETY: `Page::terminated` needs the count null or writable, and, with
    // neither pointer null, set to a number of units the array is writable
    // for, until the page is dropped, which is before this returns. This
    // function's contract has `text_size` null or writable and a non-null
    // `text` hold the capacity it gives on entry, which the README has the
    // caller set before the call.
    let mut page = unsafe { Page::terminated(text_size, text, 0) };
    let (mut total, mut full) = (0, false);
    let mut output = |ch: char| {
        let mut units = [0; 2];
        let units = ch.encode_utf16(&mut units);
        total += units.len() as c_uint;
        // A character is written whole or not at all, and none is once one
        // has not fit.
        if !full && units.len() <= page.room() {
            for unit in units {
                page.push(*unit);
            }
        } else {
            full = true;
        }
    };
    if let Some((encoding, bytes)) = selected {
        if encoding == 10 {
            // HarfBuzz treats Mac Roman name records as ASCII.
            for byte in bytes {
                output(if byte.is_ascii() {
                    *byte as char
                } else {
                    char::REPLACEMENT_CHARACTER
                });
            }
        } else {
            for ch in char::decode_utf16(
                bytes
                    .chunks_exact(2)
                    .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]])),
            ) {
                output(ch.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }
    // Dropping the page writes the terminating zero, where there is room for
    // one, and the number of units before it.
    total
}
