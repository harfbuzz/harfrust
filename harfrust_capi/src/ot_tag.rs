//! Conversion between Unicode script/language values and OpenType tags.

use core::ffi::c_uint;

use harfrust::{Language, Script};

use crate::common::{
    hr_language_t, hr_script_t, hr_tag_t, language_from_owned, language_ref, script_to_rust,
    tag_from_rust, tag_to_rust, Page,
};
use crate::common::{
    HR_SCRIPT_BENGALI, HR_SCRIPT_DEVANAGARI, HR_SCRIPT_GUJARATI, HR_SCRIPT_GURMUKHI,
    HR_SCRIPT_INVALID, HR_SCRIPT_KANNADA, HR_SCRIPT_MALAYALAM, HR_SCRIPT_MATH, HR_SCRIPT_MYANMAR,
    HR_SCRIPT_ORIYA, HR_SCRIPT_TAMIL, HR_SCRIPT_TELUGU, HR_SCRIPT_UNKNOWN,
};

/// Converts an OpenType language-system tag to an interned language.
#[no_mangle]
pub extern "C" fn hr_ot_tag_to_language(tag: hr_tag_t) -> hr_language_t {
    language_from_owned(Language::from_tag(tag_to_rust(tag)))
}

/// Converts a script and language to their preferred OpenType tags.
///
/// Each array comes with a count of its own, which gives its capacity on
/// entry and the number of tags written on return. A count whose array is
/// NULL is set to zero without being read; the README has the rule for a
/// count and its array.
///
/// # Safety
/// `language` must be NULL or an interned language returned by this API.
/// Each count must be NULL or writable, and each non-NULL output array must
/// have the capacity in its associated count.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_tags_from_script_and_language(
    script: hr_script_t,
    language: hr_language_t,
    script_count: *mut c_uint,
    script_tags: *mut hr_tag_t,
    language_count: *mut c_uint,
    language_tags: *mut hr_tag_t,
) {
    // SAFETY: `language` is `NULL` or interned by this function's contract.
    let language = unsafe { language_ref(language) };
    let (scripts, languages) = Script::tags_for_language(script_to_rust(script), language);
    // Both pages are made before either is filled, so that each count is
    // read as the caller set it, even one that lies where the other pair is
    // written.
    // SAFETY: `Page::new` needs the count null or writable, and, with
    // neither pointer null, set to a number of tags the array is writable
    // for, until the page is dropped, which is before this returns. This
    // function's contract has `script_count` `NULL` or writable, and a
    // non-null `script_tags` have the capacity that is in it.
    let script_page = unsafe { Page::new(script_count, script_tags) };
    // SAFETY: as above, with `language_count` and `language_tags`. Nothing
    // has been written since this function was called, so the count still
    // holds what the caller put in it.
    let language_page = unsafe { Page::new(language_count, language_tags) };
    script_page.fill(scripts.as_slice().iter().copied().map(tag_from_rust));
    language_page.fill(languages.as_slice().iter().copied().map(tag_from_rust));
}

/// The script an OpenType script tag stands for, as `hb_ot_tag_to_script`
/// works it out. The result is an ISO 15924 code, which is `HR_SCRIPT_UNKNOWN`
/// for a version 2 or 3 tag that is not one of the ten that exist.
pub(crate) fn script_from_ot_tag(tag: hr_tag_t) -> hr_script_t {
    let mut bytes = tag.to_be_bytes();
    if matches!(bytes[3], b'2' | b'3') {
        // The version 3 tags name the scripts the version 2 tags do.
        bytes[3] = b'2';
        return match &bytes {
            b"bng2" => HR_SCRIPT_BENGALI,
            b"dev2" => HR_SCRIPT_DEVANAGARI,
            b"gjr2" => HR_SCRIPT_GUJARATI,
            b"gur2" => HR_SCRIPT_GURMUKHI,
            b"knd2" => HR_SCRIPT_KANNADA,
            b"mlm2" => HR_SCRIPT_MALAYALAM,
            b"ory2" => HR_SCRIPT_ORIYA,
            b"tml2" => HR_SCRIPT_TAMIL,
            b"tel2" => HR_SCRIPT_TELUGU,
            b"mym2" => HR_SCRIPT_MYANMAR,
            _ => HR_SCRIPT_UNKNOWN,
        };
    }
    match &bytes {
        b"DFLT" => return HR_SCRIPT_INVALID,
        b"math" => return HR_SCRIPT_MATH,
        _ => {}
    }
    // The rest is arithmetic. Spaces at the end of a tag stand for the letter
    // before them (`nko ` is `Nkoo`), and the first letter is a capital.
    if bytes[2] == b' ' {
        bytes[2] = bytes[1];
    }
    if bytes[3] == b' ' {
        bytes[3] = bytes[2];
    }
    bytes[0] &= !0x20;
    u32::from_be_bytes(bytes)
}
