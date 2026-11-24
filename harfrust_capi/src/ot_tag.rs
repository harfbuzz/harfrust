//! Conversion between Unicode script/language values and OpenType tags.

use core::ffi::c_uint;

use harfrust::{Language, Script, Tag};

use crate::common::{
    hr_language_t, hr_script_t, hr_tag_t, language_from_owned, language_ref, script_to_rust,
    tag_from_rust, tag_to_rust,
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
/// # Safety
/// `language` must be NULL or an interned language returned by this API.
/// Each non-NULL output array must have the capacity in its associated count.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_tags_from_script_and_language(
    script: hr_script_t,
    language: hr_language_t,
    script_count: *mut c_uint,
    script_tags: *mut hr_tag_t,
    language_count: *mut c_uint,
    language_tags: *mut hr_tag_t,
) {
    let language = unsafe { language_ref(language) };
    let (scripts, languages) = Script::tags_for_language(script_to_rust(script), language);
    unsafe {
        write_tags(scripts.as_slice(), script_count, script_tags);
        write_tags(languages.as_slice(), language_count, language_tags);
    }
}

/// Matches HarfBuzz's in/out count behavior: a missing or zero-capacity
/// output leaves the count untouched.
unsafe fn write_tags(tags: &[Tag], count: *mut c_uint, output: *mut hr_tag_t) {
    // The array is looked at first. A count that comes without one is left
    // alone and need not have been set, so no reference is made to it.
    if output.is_null() {
        return;
    }
    let Some(count) = (unsafe { count.as_mut() }) else {
        return;
    };
    if *count == 0 {
        return;
    }
    let written = (*count as usize).min(tags.len());
    for (index, tag) in tags[..written].iter().copied().enumerate() {
        unsafe { *output.add(index) = tag_from_rust(tag) };
    }
    *count = written as c_uint;
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
