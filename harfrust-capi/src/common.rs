//! Scalar types, tags, directions, scripts, languages, features and
//! variations. Mirrors HarfBuzz's `hb-common.h`.
//!
//! Every enumerator and flag value here is numerically identical to its
//! HarfBuzz counterpart, so code can be ported by renaming `hb_` to `hr_`.

use core::ffi::{c_char, c_int, c_uint};
use core::str::FromStr;
use std::sync::Mutex;

use harfrust::{Direction, Feature, Language, Script, Tag, Variation};

/// A boolean, as C sees it: zero is false, non-zero is true.
pub type hr_bool_t = c_int;

/// A Unicode scalar value.
pub type hr_codepoint_t = u32;

/// A position, in whatever units the configured font scale implies.
pub type hr_position_t = i32;

/// A mask of feature bits applied to an item in a buffer.
pub type hr_mask_t = u32;

/// A four byte OpenType tag, packed big-endian into a 32 bit integer.
pub type hr_tag_t = u32;

/// The tag matching no script, language or feature.
pub const HR_TAG_NONE: hr_tag_t = 0;
/// The largest possible tag value.
pub const HR_TAG_MAX: hr_tag_t = 0xffff_ffff;
/// The largest possible tag value that is still signed-safe.
pub const HR_TAG_MAX_SIGNED: hr_tag_t = 0x7fff_ffff;

/// Value applied to a feature that covers the whole buffer, as its start.
pub const HR_FEATURE_GLOBAL_START: c_uint = 0;
/// Value applied to a feature that covers the whole buffer, as its end.
pub const HR_FEATURE_GLOBAL_END: c_uint = c_uint::MAX;

/// Builds a tag from a string, padding with spaces and truncating past four
/// bytes, as HarfBuzz's `hb_tag_from_string` does.
pub(crate) fn tag_from_str(s: &str) -> Tag {
    let mut bytes = [b' '; 4];
    for (slot, byte) in bytes.iter_mut().zip(s.bytes()) {
        *slot = byte;
    }
    Tag::new(&bytes)
}

pub(crate) fn tag_to_rust(tag: hr_tag_t) -> Tag {
    Tag::from_be_bytes(tag.to_be_bytes())
}

pub(crate) fn tag_from_rust(tag: Tag) -> hr_tag_t {
    u32::from_be_bytes(tag.to_be_bytes())
}

/// Reads a C string of `len` bytes, or up to its NUL when `len` is negative.
///
/// # Safety
///
/// `ptr` must be `NULL`, or point to `len` readable bytes, or to a
/// NUL-terminated string when `len` is negative.
pub(crate) unsafe fn str_from_raw<'a>(ptr: *const c_char, len: c_int) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    let bytes = if len < 0 {
        unsafe { core::ffi::CStr::from_ptr(ptr) }.to_bytes()
    } else {
        unsafe { core::slice::from_raw_parts(ptr.cast::<u8>(), len as usize) }
    };
    // Stop at an embedded NUL, as HarfBuzz does.
    let bytes = match bytes.iter().position(|&b| b == 0) {
        Some(nul) => &bytes[..nul],
        None => bytes,
    };
    core::str::from_utf8(bytes).ok()
}

/// Converts a string into a tag, padding with spaces and truncating past four
/// bytes.
///
/// Pass a negative `len` for a NUL-terminated string.
///
/// # Safety
///
/// See [`str_from_raw`].
#[no_mangle]
pub unsafe extern "C" fn hr_tag_from_string(str_: *const c_char, len: c_int) -> hr_tag_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return HR_TAG_NONE;
    };
    tag_from_rust(tag_from_str(s))
}

/// Writes a tag's four bytes into `buf`. No terminating NUL is written.
///
/// # Safety
///
/// `buf` must point to at least four writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hr_tag_to_string(tag: hr_tag_t, buf: *mut c_char) {
    if buf.is_null() {
        return;
    }
    let bytes = tag.to_be_bytes();
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), buf, 4) };
}

/// The direction in which text is set.
///
/// This is an integer typedef rather than an enumeration because it appears in
/// [`hr_segment_properties_t`], which callers fill in themselves; a value
/// outside the set below is then merely unrecognised rather than undefined.
pub type hr_direction_t = u32;

/// Initial, unset direction.
pub const HR_DIRECTION_INVALID: hr_direction_t = 0;
/// Text is set horizontally from left to right.
pub const HR_DIRECTION_LTR: hr_direction_t = 4;
/// Text is set horizontally from right to left.
pub const HR_DIRECTION_RTL: hr_direction_t = 5;
/// Text is set vertically from top to bottom.
pub const HR_DIRECTION_TTB: hr_direction_t = 6;
/// Text is set vertically from bottom to top.
pub const HR_DIRECTION_BTT: hr_direction_t = 7;

pub(crate) fn direction_to_rust(direction: hr_direction_t) -> Direction {
    match direction {
        HR_DIRECTION_LTR => Direction::LeftToRight,
        HR_DIRECTION_RTL => Direction::RightToLeft,
        HR_DIRECTION_TTB => Direction::TopToBottom,
        HR_DIRECTION_BTT => Direction::BottomToTop,
        _ => Direction::Invalid,
    }
}

pub(crate) fn direction_from_rust(direction: Direction) -> hr_direction_t {
    match direction {
        Direction::Invalid => HR_DIRECTION_INVALID,
        Direction::LeftToRight => HR_DIRECTION_LTR,
        Direction::RightToLeft => HR_DIRECTION_RTL,
        Direction::TopToBottom => HR_DIRECTION_TTB,
        Direction::BottomToTop => HR_DIRECTION_BTT,
    }
}

/// Parses a direction from its name, matching on the first letter only.
///
/// # Safety
///
/// See [`str_from_raw`].
#[no_mangle]
pub unsafe extern "C" fn hr_direction_from_string(
    str_: *const c_char,
    len: c_int,
) -> hr_direction_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return HR_DIRECTION_INVALID;
    };
    Direction::from_str(s).map_or(HR_DIRECTION_INVALID, direction_from_rust)
}

/// Returns the name of a direction, as a NUL-terminated static string.
#[no_mangle]
pub extern "C" fn hr_direction_to_string(direction: hr_direction_t) -> *const c_char {
    let name: &[u8] = match direction {
        HR_DIRECTION_LTR => b"ltr\0",
        HR_DIRECTION_RTL => b"rtl\0",
        HR_DIRECTION_TTB => b"ttb\0",
        HR_DIRECTION_BTT => b"btt\0",
        _ => b"invalid\0",
    };
    name.as_ptr().cast::<c_char>()
}

/// Returns whether a direction is horizontal.
#[no_mangle]
pub extern "C" fn hr_direction_is_horizontal(direction: hr_direction_t) -> hr_bool_t {
    matches!(direction, HR_DIRECTION_LTR | HR_DIRECTION_RTL).into()
}

/// Returns whether a direction is vertical.
#[no_mangle]
pub extern "C" fn hr_direction_is_vertical(direction: hr_direction_t) -> hr_bool_t {
    matches!(direction, HR_DIRECTION_TTB | HR_DIRECTION_BTT).into()
}

/// Returns whether a direction runs forwards (left-to-right or top-to-bottom).
#[no_mangle]
pub extern "C" fn hr_direction_is_forward(direction: hr_direction_t) -> hr_bool_t {
    matches!(direction, HR_DIRECTION_LTR | HR_DIRECTION_TTB).into()
}

/// Returns whether a direction runs backwards.
#[no_mangle]
pub extern "C" fn hr_direction_is_backward(direction: hr_direction_t) -> hr_bool_t {
    matches!(direction, HR_DIRECTION_RTL | HR_DIRECTION_BTT).into()
}

/// Returns whether a direction is set at all.
#[no_mangle]
pub extern "C" fn hr_direction_is_valid(direction: hr_direction_t) -> hr_bool_t {
    matches!(
        direction,
        HR_DIRECTION_LTR | HR_DIRECTION_RTL | HR_DIRECTION_TTB | HR_DIRECTION_BTT
    )
    .into()
}

/// Returns the direction running opposite to the given one.
#[no_mangle]
pub extern "C" fn hr_direction_reverse(direction: hr_direction_t) -> hr_direction_t {
    match direction {
        HR_DIRECTION_LTR => HR_DIRECTION_RTL,
        HR_DIRECTION_RTL => HR_DIRECTION_LTR,
        HR_DIRECTION_TTB => HR_DIRECTION_BTT,
        HR_DIRECTION_BTT => HR_DIRECTION_TTB,
        other => other,
    }
}

/// An ISO 15924 script, held as its four byte tag.
///
/// HarfBuzz declares `hb_script_t` as an enum; here it is a tag typedef so
/// that arbitrary scripts round-trip without a cast. The `HR_SCRIPT_*`
/// constants below carry the same values as HarfBuzz's enumerators.
pub type hr_script_t = hr_tag_t;

/// The script matching no script at all.
pub const HR_SCRIPT_INVALID: hr_script_t = HR_TAG_NONE;

/// The Common script (`Zyyy`).
pub const HR_SCRIPT_COMMON: hr_script_t = 0x5A79_7979;
/// The Inherited script (`Zinh`).
pub const HR_SCRIPT_INHERITED: hr_script_t = 0x5A69_6E68;
/// The Arabic script (`Arab`).
pub const HR_SCRIPT_ARABIC: hr_script_t = 0x4172_6162;
/// The Armenian script (`Armn`).
pub const HR_SCRIPT_ARMENIAN: hr_script_t = 0x4172_6D6E;
/// The Bengali script (`Beng`).
pub const HR_SCRIPT_BENGALI: hr_script_t = 0x4265_6E67;
/// The Cyrillic script (`Cyrl`).
pub const HR_SCRIPT_CYRILLIC: hr_script_t = 0x4379_726C;
/// The Devanagari script (`Deva`).
pub const HR_SCRIPT_DEVANAGARI: hr_script_t = 0x4465_7661;
/// The Georgian script (`Geor`).
pub const HR_SCRIPT_GEORGIAN: hr_script_t = 0x4765_6F72;
/// The Greek script (`Grek`).
pub const HR_SCRIPT_GREEK: hr_script_t = 0x4772_656B;
/// The Gujarati script (`Gujr`).
pub const HR_SCRIPT_GUJARATI: hr_script_t = 0x4775_6A72;
/// The Gurmukhi script (`Guru`).
pub const HR_SCRIPT_GURMUKHI: hr_script_t = 0x4775_7275;
/// The Hangul script (`Hang`).
pub const HR_SCRIPT_HANGUL: hr_script_t = 0x4861_6E67;
/// The Han script (`Hani`).
pub const HR_SCRIPT_HAN: hr_script_t = 0x4861_6E69;
/// The Hebrew script (`Hebr`).
pub const HR_SCRIPT_HEBREW: hr_script_t = 0x4865_6272;
/// The Hiragana script (`Hira`).
pub const HR_SCRIPT_HIRAGANA: hr_script_t = 0x4869_7261;
/// The Kannada script (`Knda`).
pub const HR_SCRIPT_KANNADA: hr_script_t = 0x4B6E_6461;
/// The Katakana script (`Kana`).
pub const HR_SCRIPT_KATAKANA: hr_script_t = 0x4B61_6E61;
/// The Lao script (`Laoo`).
pub const HR_SCRIPT_LAO: hr_script_t = 0x4C61_6F6F;
/// The Latin script (`Latn`).
pub const HR_SCRIPT_LATIN: hr_script_t = 0x4C61_746E;
/// The Malayalam script (`Mlym`).
pub const HR_SCRIPT_MALAYALAM: hr_script_t = 0x4D6C_796D;
/// The Oriya script (`Orya`).
pub const HR_SCRIPT_ORIYA: hr_script_t = 0x4F72_7961;
/// The Tamil script (`Taml`).
pub const HR_SCRIPT_TAMIL: hr_script_t = 0x5461_6D6C;
/// The Telugu script (`Telu`).
pub const HR_SCRIPT_TELUGU: hr_script_t = 0x5465_6C75;
/// The Thai script (`Thai`).
pub const HR_SCRIPT_THAI: hr_script_t = 0x5468_6169;
/// The Tibetan script (`Tibt`).
pub const HR_SCRIPT_TIBETAN: hr_script_t = 0x5469_6274;
/// The Bopomofo script (`Bopo`).
pub const HR_SCRIPT_BOPOMOFO: hr_script_t = 0x426F_706F;
/// The Braille script (`Brai`).
pub const HR_SCRIPT_BRAILLE: hr_script_t = 0x4272_6169;
/// The Canadian Syllabics script (`Cans`).
pub const HR_SCRIPT_CANADIAN_SYLLABICS: hr_script_t = 0x4361_6E73;
/// The Cherokee script (`Cher`).
pub const HR_SCRIPT_CHEROKEE: hr_script_t = 0x4368_6572;
/// The Ethiopic script (`Ethi`).
pub const HR_SCRIPT_ETHIOPIC: hr_script_t = 0x4574_6869;
/// The Khmer script (`Khmr`).
pub const HR_SCRIPT_KHMER: hr_script_t = 0x4B68_6D72;
/// The Mongolian script (`Mong`).
pub const HR_SCRIPT_MONGOLIAN: hr_script_t = 0x4D6F_6E67;
/// The Myanmar script (`Mymr`).
pub const HR_SCRIPT_MYANMAR: hr_script_t = 0x4D79_6D72;
/// The Ogham script (`Ogam`).
pub const HR_SCRIPT_OGHAM: hr_script_t = 0x4F67_616D;
/// The Runic script (`Runr`).
pub const HR_SCRIPT_RUNIC: hr_script_t = 0x5275_6E72;
/// The Sinhala script (`Sinh`).
pub const HR_SCRIPT_SINHALA: hr_script_t = 0x5369_6E68;
/// The Syriac script (`Syrc`).
pub const HR_SCRIPT_SYRIAC: hr_script_t = 0x5379_7263;
/// The Thaana script (`Thaa`).
pub const HR_SCRIPT_THAANA: hr_script_t = 0x5468_6161;
/// The Yi script (`Yiii`).
pub const HR_SCRIPT_YI: hr_script_t = 0x5969_6969;
/// The Deseret script (`Dsrt`).
pub const HR_SCRIPT_DESERET: hr_script_t = 0x4473_7274;
/// The Gothic script (`Goth`).
pub const HR_SCRIPT_GOTHIC: hr_script_t = 0x476F_7468;
/// The Old Italic script (`Ital`).
pub const HR_SCRIPT_OLD_ITALIC: hr_script_t = 0x4974_616C;
/// The Buhid script (`Buhd`).
pub const HR_SCRIPT_BUHID: hr_script_t = 0x4275_6864;
/// The Hanunoo script (`Hano`).
pub const HR_SCRIPT_HANUNOO: hr_script_t = 0x4861_6E6F;
/// The Tagalog script (`Tglg`).
pub const HR_SCRIPT_TAGALOG: hr_script_t = 0x5467_6C67;
/// The Tagbanwa script (`Tagb`).
pub const HR_SCRIPT_TAGBANWA: hr_script_t = 0x5461_6762;
/// The Cypriot script (`Cprt`).
pub const HR_SCRIPT_CYPRIOT: hr_script_t = 0x4370_7274;
/// The Limbu script (`Limb`).
pub const HR_SCRIPT_LIMBU: hr_script_t = 0x4C69_6D62;
/// The Linear B script (`Linb`).
pub const HR_SCRIPT_LINEAR_B: hr_script_t = 0x4C69_6E62;
/// The Osmanya script (`Osma`).
pub const HR_SCRIPT_OSMANYA: hr_script_t = 0x4F73_6D61;
/// The Shavian script (`Shaw`).
pub const HR_SCRIPT_SHAVIAN: hr_script_t = 0x5368_6177;
/// The Tai Le script (`Tale`).
pub const HR_SCRIPT_TAI_LE: hr_script_t = 0x5461_6C65;
/// The Ugaritic script (`Ugar`).
pub const HR_SCRIPT_UGARITIC: hr_script_t = 0x5567_6172;
/// The Buginese script (`Bugi`).
pub const HR_SCRIPT_BUGINESE: hr_script_t = 0x4275_6769;
/// The Coptic script (`Copt`).
pub const HR_SCRIPT_COPTIC: hr_script_t = 0x436F_7074;
/// The Glagolitic script (`Glag`).
pub const HR_SCRIPT_GLAGOLITIC: hr_script_t = 0x476C_6167;
/// The Kharoshthi script (`Khar`).
pub const HR_SCRIPT_KHAROSHTHI: hr_script_t = 0x4B68_6172;
/// The New Tai Lue script (`Talu`).
pub const HR_SCRIPT_NEW_TAI_LUE: hr_script_t = 0x5461_6C75;
/// The Old Persian script (`Xpeo`).
pub const HR_SCRIPT_OLD_PERSIAN: hr_script_t = 0x5870_656F;
/// The Syloti Nagri script (`Sylo`).
pub const HR_SCRIPT_SYLOTI_NAGRI: hr_script_t = 0x5379_6C6F;
/// The Tifinagh script (`Tfng`).
pub const HR_SCRIPT_TIFINAGH: hr_script_t = 0x5466_6E67;
/// The Unknown script (`Zzzz`).
pub const HR_SCRIPT_UNKNOWN: hr_script_t = 0x5A7A_7A7A;
/// The Balinese script (`Bali`).
pub const HR_SCRIPT_BALINESE: hr_script_t = 0x4261_6C69;
/// The Cuneiform script (`Xsux`).
pub const HR_SCRIPT_CUNEIFORM: hr_script_t = 0x5873_7578;
/// The Nko script (`Nkoo`).
pub const HR_SCRIPT_NKO: hr_script_t = 0x4E6B_6F6F;
/// The Phags Pa script (`Phag`).
pub const HR_SCRIPT_PHAGS_PA: hr_script_t = 0x5068_6167;
/// The Phoenician script (`Phnx`).
pub const HR_SCRIPT_PHOENICIAN: hr_script_t = 0x5068_6E78;
/// The Carian script (`Cari`).
pub const HR_SCRIPT_CARIAN: hr_script_t = 0x4361_7269;
/// The Cham script (`Cham`).
pub const HR_SCRIPT_CHAM: hr_script_t = 0x4368_616D;
/// The Kayah Li script (`Kali`).
pub const HR_SCRIPT_KAYAH_LI: hr_script_t = 0x4B61_6C69;
/// The Lepcha script (`Lepc`).
pub const HR_SCRIPT_LEPCHA: hr_script_t = 0x4C65_7063;
/// The Lycian script (`Lyci`).
pub const HR_SCRIPT_LYCIAN: hr_script_t = 0x4C79_6369;
/// The Lydian script (`Lydi`).
pub const HR_SCRIPT_LYDIAN: hr_script_t = 0x4C79_6469;
/// The Ol Chiki script (`Olck`).
pub const HR_SCRIPT_OL_CHIKI: hr_script_t = 0x4F6C_636B;
/// The Rejang script (`Rjng`).
pub const HR_SCRIPT_REJANG: hr_script_t = 0x526A_6E67;
/// The Saurashtra script (`Saur`).
pub const HR_SCRIPT_SAURASHTRA: hr_script_t = 0x5361_7572;
/// The Sundanese script (`Sund`).
pub const HR_SCRIPT_SUNDANESE: hr_script_t = 0x5375_6E64;
/// The Vai script (`Vaii`).
pub const HR_SCRIPT_VAI: hr_script_t = 0x5661_6969;
/// The Avestan script (`Avst`).
pub const HR_SCRIPT_AVESTAN: hr_script_t = 0x4176_7374;
/// The Bamum script (`Bamu`).
pub const HR_SCRIPT_BAMUM: hr_script_t = 0x4261_6D75;
/// The Egyptian Hieroglyphs script (`Egyp`).
pub const HR_SCRIPT_EGYPTIAN_HIEROGLYPHS: hr_script_t = 0x4567_7970;
/// The Imperial Aramaic script (`Armi`).
pub const HR_SCRIPT_IMPERIAL_ARAMAIC: hr_script_t = 0x4172_6D69;
/// The Inscriptional Pahlavi script (`Phli`).
pub const HR_SCRIPT_INSCRIPTIONAL_PAHLAVI: hr_script_t = 0x5068_6C69;
/// The Inscriptional Parthian script (`Prti`).
pub const HR_SCRIPT_INSCRIPTIONAL_PARTHIAN: hr_script_t = 0x5072_7469;
/// The Javanese script (`Java`).
pub const HR_SCRIPT_JAVANESE: hr_script_t = 0x4A61_7661;
/// The Kaithi script (`Kthi`).
pub const HR_SCRIPT_KAITHI: hr_script_t = 0x4B74_6869;
/// The Lisu script (`Lisu`).
pub const HR_SCRIPT_LISU: hr_script_t = 0x4C69_7375;
/// The Meetei Mayek script (`Mtei`).
pub const HR_SCRIPT_MEETEI_MAYEK: hr_script_t = 0x4D74_6569;
/// The Old South Arabian script (`Sarb`).
pub const HR_SCRIPT_OLD_SOUTH_ARABIAN: hr_script_t = 0x5361_7262;
/// The Old Turkic script (`Orkh`).
pub const HR_SCRIPT_OLD_TURKIC: hr_script_t = 0x4F72_6B68;
/// The Samaritan script (`Samr`).
pub const HR_SCRIPT_SAMARITAN: hr_script_t = 0x5361_6D72;
/// The Tai Tham script (`Lana`).
pub const HR_SCRIPT_TAI_THAM: hr_script_t = 0x4C61_6E61;
/// The Tai Viet script (`Tavt`).
pub const HR_SCRIPT_TAI_VIET: hr_script_t = 0x5461_7674;
/// The Batak script (`Batk`).
pub const HR_SCRIPT_BATAK: hr_script_t = 0x4261_746B;
/// The Brahmi script (`Brah`).
pub const HR_SCRIPT_BRAHMI: hr_script_t = 0x4272_6168;
/// The Mandaic script (`Mand`).
pub const HR_SCRIPT_MANDAIC: hr_script_t = 0x4D61_6E64;
/// The Chakma script (`Cakm`).
pub const HR_SCRIPT_CHAKMA: hr_script_t = 0x4361_6B6D;
/// The Meroitic Cursive script (`Merc`).
pub const HR_SCRIPT_MEROITIC_CURSIVE: hr_script_t = 0x4D65_7263;
/// The Meroitic Hieroglyphs script (`Mero`).
pub const HR_SCRIPT_MEROITIC_HIEROGLYPHS: hr_script_t = 0x4D65_726F;
/// The Miao script (`Plrd`).
pub const HR_SCRIPT_MIAO: hr_script_t = 0x506C_7264;
/// The Sharada script (`Shrd`).
pub const HR_SCRIPT_SHARADA: hr_script_t = 0x5368_7264;
/// The Sora Sompeng script (`Sora`).
pub const HR_SCRIPT_SORA_SOMPENG: hr_script_t = 0x536F_7261;
/// The Takri script (`Takr`).
pub const HR_SCRIPT_TAKRI: hr_script_t = 0x5461_6B72;
/// The Bassa Vah script (`Bass`).
pub const HR_SCRIPT_BASSA_VAH: hr_script_t = 0x4261_7373;
/// The Caucasian Albanian script (`Aghb`).
pub const HR_SCRIPT_CAUCASIAN_ALBANIAN: hr_script_t = 0x4167_6862;
/// The Duployan script (`Dupl`).
pub const HR_SCRIPT_DUPLOYAN: hr_script_t = 0x4475_706C;
/// The Elbasan script (`Elba`).
pub const HR_SCRIPT_ELBASAN: hr_script_t = 0x456C_6261;
/// The Grantha script (`Gran`).
pub const HR_SCRIPT_GRANTHA: hr_script_t = 0x4772_616E;
/// The Khojki script (`Khoj`).
pub const HR_SCRIPT_KHOJKI: hr_script_t = 0x4B68_6F6A;
/// The Khudawadi script (`Sind`).
pub const HR_SCRIPT_KHUDAWADI: hr_script_t = 0x5369_6E64;
/// The Linear A script (`Lina`).
pub const HR_SCRIPT_LINEAR_A: hr_script_t = 0x4C69_6E61;
/// The Mahajani script (`Mahj`).
pub const HR_SCRIPT_MAHAJANI: hr_script_t = 0x4D61_686A;
/// The Manichaean script (`Mani`).
pub const HR_SCRIPT_MANICHAEAN: hr_script_t = 0x4D61_6E69;
/// The Mende Kikakui script (`Mend`).
pub const HR_SCRIPT_MENDE_KIKAKUI: hr_script_t = 0x4D65_6E64;
/// The Modi script (`Modi`).
pub const HR_SCRIPT_MODI: hr_script_t = 0x4D6F_6469;
/// The Mro script (`Mroo`).
pub const HR_SCRIPT_MRO: hr_script_t = 0x4D72_6F6F;
/// The Nabataean script (`Nbat`).
pub const HR_SCRIPT_NABATAEAN: hr_script_t = 0x4E62_6174;
/// The Old North Arabian script (`Narb`).
pub const HR_SCRIPT_OLD_NORTH_ARABIAN: hr_script_t = 0x4E61_7262;
/// The Old Permic script (`Perm`).
pub const HR_SCRIPT_OLD_PERMIC: hr_script_t = 0x5065_726D;
/// The Pahawh Hmong script (`Hmng`).
pub const HR_SCRIPT_PAHAWH_HMONG: hr_script_t = 0x486D_6E67;
/// The Palmyrene script (`Palm`).
pub const HR_SCRIPT_PALMYRENE: hr_script_t = 0x5061_6C6D;
/// The Pau Cin Hau script (`Pauc`).
pub const HR_SCRIPT_PAU_CIN_HAU: hr_script_t = 0x5061_7563;
/// The Psalter Pahlavi script (`Phlp`).
pub const HR_SCRIPT_PSALTER_PAHLAVI: hr_script_t = 0x5068_6C70;
/// The Siddham script (`Sidd`).
pub const HR_SCRIPT_SIDDHAM: hr_script_t = 0x5369_6464;
/// The Tirhuta script (`Tirh`).
pub const HR_SCRIPT_TIRHUTA: hr_script_t = 0x5469_7268;
/// The Warang Citi script (`Wara`).
pub const HR_SCRIPT_WARANG_CITI: hr_script_t = 0x5761_7261;
/// The Ahom script (`Ahom`).
pub const HR_SCRIPT_AHOM: hr_script_t = 0x4168_6F6D;
/// The Anatolian Hieroglyphs script (`Hluw`).
pub const HR_SCRIPT_ANATOLIAN_HIEROGLYPHS: hr_script_t = 0x486C_7577;
/// The Hatran script (`Hatr`).
pub const HR_SCRIPT_HATRAN: hr_script_t = 0x4861_7472;
/// The Multani script (`Mult`).
pub const HR_SCRIPT_MULTANI: hr_script_t = 0x4D75_6C74;
/// The Old Hungarian script (`Hung`).
pub const HR_SCRIPT_OLD_HUNGARIAN: hr_script_t = 0x4875_6E67;
/// The Signwriting script (`Sgnw`).
pub const HR_SCRIPT_SIGNWRITING: hr_script_t = 0x5367_6E77;
/// The Adlam script (`Adlm`).
pub const HR_SCRIPT_ADLAM: hr_script_t = 0x4164_6C6D;
/// The Bhaiksuki script (`Bhks`).
pub const HR_SCRIPT_BHAIKSUKI: hr_script_t = 0x4268_6B73;
/// The Marchen script (`Marc`).
pub const HR_SCRIPT_MARCHEN: hr_script_t = 0x4D61_7263;
/// The Osage script (`Osge`).
pub const HR_SCRIPT_OSAGE: hr_script_t = 0x4F73_6765;
/// The Tangut script (`Tang`).
pub const HR_SCRIPT_TANGUT: hr_script_t = 0x5461_6E67;
/// The Newa script (`Newa`).
pub const HR_SCRIPT_NEWA: hr_script_t = 0x4E65_7761;
/// The Masaram Gondi script (`Gonm`).
pub const HR_SCRIPT_MASARAM_GONDI: hr_script_t = 0x476F_6E6D;
/// The Nushu script (`Nshu`).
pub const HR_SCRIPT_NUSHU: hr_script_t = 0x4E73_6875;
/// The Soyombo script (`Soyo`).
pub const HR_SCRIPT_SOYOMBO: hr_script_t = 0x536F_796F;
/// The Zanabazar Square script (`Zanb`).
pub const HR_SCRIPT_ZANABAZAR_SQUARE: hr_script_t = 0x5A61_6E62;
/// The Dogra script (`Dogr`).
pub const HR_SCRIPT_DOGRA: hr_script_t = 0x446F_6772;
/// The Gunjala Gondi script (`Gong`).
pub const HR_SCRIPT_GUNJALA_GONDI: hr_script_t = 0x476F_6E67;
/// The Hanifi Rohingya script (`Rohg`).
pub const HR_SCRIPT_HANIFI_ROHINGYA: hr_script_t = 0x526F_6867;
/// The Makasar script (`Maka`).
pub const HR_SCRIPT_MAKASAR: hr_script_t = 0x4D61_6B61;
/// The Medefaidrin script (`Medf`).
pub const HR_SCRIPT_MEDEFAIDRIN: hr_script_t = 0x4D65_6466;
/// The Old Sogdian script (`Sogo`).
pub const HR_SCRIPT_OLD_SOGDIAN: hr_script_t = 0x536F_676F;
/// The Sogdian script (`Sogd`).
pub const HR_SCRIPT_SOGDIAN: hr_script_t = 0x536F_6764;
/// The Elymaic script (`Elym`).
pub const HR_SCRIPT_ELYMAIC: hr_script_t = 0x456C_796D;
/// The Nandinagari script (`Nand`).
pub const HR_SCRIPT_NANDINAGARI: hr_script_t = 0x4E61_6E64;
/// The Nyiakeng Puachue Hmong script (`Hmnp`).
pub const HR_SCRIPT_NYIAKENG_PUACHUE_HMONG: hr_script_t = 0x486D_6E70;
/// The Wancho script (`Wcho`).
pub const HR_SCRIPT_WANCHO: hr_script_t = 0x5763_686F;
/// The Chorasmian script (`Chrs`).
pub const HR_SCRIPT_CHORASMIAN: hr_script_t = 0x4368_7273;
/// The Dives Akuru script (`Diak`).
pub const HR_SCRIPT_DIVES_AKURU: hr_script_t = 0x4469_616B;
/// The Khitan Small Script script (`Kits`).
pub const HR_SCRIPT_KHITAN_SMALL_SCRIPT: hr_script_t = 0x4B69_7473;
/// The Yezidi script (`Yezi`).
pub const HR_SCRIPT_YEZIDI: hr_script_t = 0x5965_7A69;
/// The Cypro Minoan script (`Cpmn`).
pub const HR_SCRIPT_CYPRO_MINOAN: hr_script_t = 0x4370_6D6E;
/// The Old Uyghur script (`Ougr`).
pub const HR_SCRIPT_OLD_UYGHUR: hr_script_t = 0x4F75_6772;
/// The Tangsa script (`Tnsa`).
pub const HR_SCRIPT_TANGSA: hr_script_t = 0x546E_7361;
/// The Toto script (`Toto`).
pub const HR_SCRIPT_TOTO: hr_script_t = 0x546F_746F;
/// The Vithkuqi script (`Vith`).
pub const HR_SCRIPT_VITHKUQI: hr_script_t = 0x5669_7468;
/// The Kawi script (`Kawi`).
pub const HR_SCRIPT_KAWI: hr_script_t = 0x4B61_7769;
/// The Nag Mundari script (`Nagm`).
pub const HR_SCRIPT_NAG_MUNDARI: hr_script_t = 0x4E61_676D;
/// The Garay script (`Gara`).
pub const HR_SCRIPT_GARAY: hr_script_t = 0x4761_7261;
/// The Gurung Khema script (`Gukh`).
pub const HR_SCRIPT_GURUNG_KHEMA: hr_script_t = 0x4775_6B68;
/// The Kirat Rai script (`Krai`).
pub const HR_SCRIPT_KIRAT_RAI: hr_script_t = 0x4B72_6169;
/// The Ol Onal script (`Onao`).
pub const HR_SCRIPT_OL_ONAL: hr_script_t = 0x4F6E_616F;
/// The Sunuwar script (`Sunu`).
pub const HR_SCRIPT_SUNUWAR: hr_script_t = 0x5375_6E75;
/// The Todhri script (`Todr`).
pub const HR_SCRIPT_TODHRI: hr_script_t = 0x546F_6472;
/// The Tulu Tigalari script (`Tutg`).
pub const HR_SCRIPT_TULU_TIGALARI: hr_script_t = 0x5475_7467;
/// The Beria Erfe script (`Berf`).
pub const HR_SCRIPT_BERIA_ERFE: hr_script_t = 0x4265_7266;
/// The Sidetic script (`Sidt`).
pub const HR_SCRIPT_SIDETIC: hr_script_t = 0x5369_6474;
/// The Tai Yo script (`Tayo`).
pub const HR_SCRIPT_TAI_YO: hr_script_t = 0x5461_796F;
/// The Tolong Siki script (`Tols`).
pub const HR_SCRIPT_TOLONG_SIKI: hr_script_t = 0x546F_6C73;
/// The Math script (`Zmth`).
pub const HR_SCRIPT_MATH: hr_script_t = 0x5A6D_7468;
/// The Myanmar Zawgyi script (`Qaag`).
pub const HR_SCRIPT_MYANMAR_ZAWGYI: hr_script_t = 0x5161_6167;

pub(crate) fn script_to_rust(script: hr_script_t) -> Option<Script> {
    Script::from_iso15924_tag(tag_to_rust(script))
}

pub(crate) fn script_from_rust(script: Script) -> hr_script_t {
    tag_from_rust(script.tag())
}

/// Converts an ISO 15924 tag into a script.
#[no_mangle]
pub extern "C" fn hr_script_from_iso15924_tag(tag: hr_tag_t) -> hr_script_t {
    script_to_rust(tag).map_or(HR_SCRIPT_INVALID, script_from_rust)
}

/// Parses a script from an ISO 15924 tag written as a string.
///
/// # Safety
///
/// See [`str_from_raw`].
#[no_mangle]
pub unsafe extern "C" fn hr_script_from_string(str_: *const c_char, len: c_int) -> hr_script_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return HR_SCRIPT_INVALID;
    };
    Script::from_str(s).map_or(HR_SCRIPT_INVALID, script_from_rust)
}

/// Returns a script's ISO 15924 tag.
#[no_mangle]
pub extern "C" fn hr_script_to_iso15924_tag(script: hr_script_t) -> hr_tag_t {
    script
}

/// Returns the direction text in this script is usually set in.
#[no_mangle]
pub extern "C" fn hr_script_get_horizontal_direction(script: hr_script_t) -> hr_direction_t {
    let Some(script) = script_to_rust(script) else {
        return HR_DIRECTION_INVALID;
    };
    // `guess_segment_properties` derives the direction from the script, which
    // is exactly what this needs and keeps the two in step.
    let mut buffer = harfrust::Buffer::new();
    buffer.set_script(script);
    buffer.guess_segment_properties();
    direction_from_rust(buffer.direction())
}

/// An interned language tag.
///
/// Language values are interned for the lifetime of the process, so they may
/// be compared by pointer and never need to be freed.
pub struct hr_language_impl_t {
    lang: Language,
    /// NUL-terminated name, so it can be handed straight back to C.
    name: Box<[u8]>,
}

/// A language, as an interned pointer. `NULL` means "unset".
pub type hr_language_t = *const hr_language_impl_t;

static LANGUAGES: Mutex<Vec<&'static hr_language_impl_t>> = Mutex::new(Vec::new());

fn intern_language(lang: Language) -> hr_language_t {
    let Ok(mut languages) = LANGUAGES.lock() else {
        return core::ptr::null();
    };
    if let Some(found) = languages.iter().find(|entry| entry.lang == lang) {
        return *found as hr_language_t;
    }
    let mut name = lang.as_bytes().to_vec();
    name.push(0);
    let entry: &'static hr_language_impl_t = Box::leak(Box::new(hr_language_impl_t {
        lang,
        name: name.into_boxed_slice(),
    }));
    languages.push(entry);
    entry as hr_language_t
}

/// # Safety
///
/// `language` must be `NULL` or a value returned by `hr_language_from_string`.
pub(crate) unsafe fn language_to_rust(language: hr_language_t) -> Option<Language> {
    unsafe { language.as_ref() }.map(|entry| entry.lang.clone())
}

pub(crate) fn language_from_rust(language: Option<Language>) -> hr_language_t {
    language.map_or(core::ptr::null(), intern_language)
}

/// Interns a language from a BCP 47 tag.
///
/// The returned value lives for the lifetime of the process and must not be
/// freed.
///
/// # Safety
///
/// See [`str_from_raw`].
#[no_mangle]
pub unsafe extern "C" fn hr_language_from_string(str_: *const c_char, len: c_int) -> hr_language_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return core::ptr::null();
    };
    Language::new(s).map_or(core::ptr::null(), intern_language)
}

/// Returns a language's tag as a NUL-terminated string, or `NULL` when the
/// language is unset.
/// # Safety
///
/// `language` must be `NULL` or a value returned by `hr_language_from_string`.
#[no_mangle]
pub unsafe extern "C" fn hr_language_to_string(language: hr_language_t) -> *const c_char {
    unsafe { language.as_ref() }.map_or(core::ptr::null(), |entry| {
        entry.name.as_ptr().cast::<c_char>()
    })
}

/// Returns the process's default language, taken from the environment.
#[no_mangle]
pub extern "C" fn hr_language_get_default() -> hr_language_t {
    static DEFAULT: Mutex<Option<usize>> = Mutex::new(None);
    let Ok(mut cached) = DEFAULT.lock() else {
        return core::ptr::null();
    };
    *cached.get_or_insert_with(|| {
        let from_env = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok())
            .and_then(|value| {
                // Trim the codeset and modifier: "en_US.UTF-8" -> "en_US".
                let tag = value.split(['.', '@']).next().unwrap_or_default();
                Language::new(tag)
            });
        language_from_rust(from_env.or_else(|| Language::new("x-hbot"))) as usize
    }) as hr_language_t
}

/// Returns whether `language` is the same as, or a more specific form of,
/// `specific`.
/// # Safety
///
/// Both arguments must be `NULL` or values returned by
/// `hr_language_from_string`.
#[no_mangle]
pub unsafe extern "C" fn hr_language_matches(
    language: hr_language_t,
    specific: hr_language_t,
) -> hr_bool_t {
    if language == specific {
        return true.into();
    }
    let (Some(language), Some(specific)) =
        (unsafe { language.as_ref() }, unsafe { specific.as_ref() })
    else {
        return false.into();
    };
    let (lang, spec) = (language.lang.as_bytes(), specific.lang.as_bytes());
    if spec.is_empty() {
        return true.into();
    }
    // A match requires a prefix ending on a subtag boundary.
    (lang.len() > spec.len() && lang.starts_with(spec) && lang[spec.len()] == b'-').into()
}

/// A feature tag with the value to apply and the range to apply it over.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct hr_feature_t {
    /// The feature's OpenType tag.
    pub tag: hr_tag_t,
    /// The value to apply. Zero disables the feature; one enables it.
    pub value: u32,
    /// Index of the first item the feature applies to.
    pub start: c_uint,
    /// Index one past the last item the feature applies to.
    pub end: c_uint,
}

impl From<hr_feature_t> for Feature {
    fn from(value: hr_feature_t) -> Self {
        Feature {
            tag: tag_to_rust(value.tag),
            value: value.value,
            start: value.start,
            end: value.end,
        }
    }
}

impl From<Feature> for hr_feature_t {
    fn from(value: Feature) -> Self {
        hr_feature_t {
            tag: tag_from_rust(value.tag),
            value: value.value,
            start: value.start,
            end: value.end,
        }
    }
}

/// Parses a feature from its string form, such as `kern`, `-liga` or
/// `aalt[3:5]=2`.
///
/// Returns false, leaving `feature` untouched, if the string does not parse.
///
/// # Safety
///
/// See [`str_from_raw`]. `feature` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_feature_from_string(
    str_: *const c_char,
    len: c_int,
    feature: *mut hr_feature_t,
) -> hr_bool_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return false.into();
    };
    let Ok(parsed) = Feature::from_str(s) else {
        return false.into();
    };
    if let Some(out) = unsafe { feature.as_mut() } {
        *out = parsed.into();
    }
    true.into()
}

/// Writes a feature's string form into `buf`, always NUL-terminating it.
///
/// # Safety
///
/// `feature` must be `NULL` or readable, and `buf` must point to `size`
/// writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hr_feature_to_string(
    feature: *const hr_feature_t,
    buf: *mut c_char,
    size: c_uint,
) {
    let Some(feature) = (unsafe { feature.as_ref() }) else {
        return;
    };
    let mut s = String::with_capacity(32);
    if feature.value == 0 {
        s.push('-');
    }
    s.push_str(tag_to_rust(feature.tag).to_string().trim_end());
    if feature.start != HR_FEATURE_GLOBAL_START || feature.end != HR_FEATURE_GLOBAL_END {
        s.push('[');
        if feature.start != HR_FEATURE_GLOBAL_START {
            s.push_str(&feature.start.to_string());
        }
        s.push(':');
        if feature.end != HR_FEATURE_GLOBAL_END {
            s.push_str(&feature.end.to_string());
        }
        s.push(']');
    }
    if feature.value > 1 {
        s.push('=');
        s.push_str(&feature.value.to_string());
    }
    unsafe { write_c_string(&s, buf, size) };
}

/// A variation axis tag and the value to set it to, in user space.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct hr_variation_t {
    /// The axis's OpenType tag.
    pub tag: hr_tag_t,
    /// The value to set the axis to, in user space.
    pub value: f32,
}

impl From<hr_variation_t> for Variation {
    fn from(value: hr_variation_t) -> Self {
        Variation {
            tag: tag_to_rust(value.tag),
            value: value.value,
        }
    }
}

impl From<Variation> for hr_variation_t {
    fn from(value: Variation) -> Self {
        hr_variation_t {
            tag: tag_from_rust(value.tag),
            value: value.value,
        }
    }
}

/// Parses a variation from its string form, such as `wght=700`.
///
/// Returns false, leaving `variation` untouched, if the string does not parse.
///
/// # Safety
///
/// See [`str_from_raw`]. `variation` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_variation_from_string(
    str_: *const c_char,
    len: c_int,
    variation: *mut hr_variation_t,
) -> hr_bool_t {
    let Some(s) = (unsafe { str_from_raw(str_, len) }) else {
        return false.into();
    };
    let Ok(parsed) = Variation::from_str(s) else {
        return false.into();
    };
    if let Some(out) = unsafe { variation.as_mut() } {
        *out = parsed.into();
    }
    true.into()
}

/// Writes a variation's string form into `buf`, always NUL-terminating it.
///
/// # Safety
///
/// `variation` must be `NULL` or readable, and `buf` must point to `size`
/// writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hr_variation_to_string(
    variation: *const hr_variation_t,
    buf: *mut c_char,
    size: c_uint,
) {
    let Some(variation) = (unsafe { variation.as_ref() }) else {
        return;
    };
    let s = format!(
        "{}={}",
        tag_to_rust(variation.tag).to_string().trim_end(),
        variation.value
    );
    unsafe { write_c_string(&s, buf, size) };
}

/// The ink extents of a glyph, in the font's scaled units.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct hr_glyph_extents_t {
    /// Horizontal bearing from the glyph origin to the left of the ink box.
    pub x_bearing: hr_position_t,
    /// Vertical bearing from the glyph origin to the top of the ink box.
    pub y_bearing: hr_position_t,
    /// Width of the ink box.
    pub width: hr_position_t,
    /// Height of the ink box, measured downwards.
    pub height: hr_position_t,
}

/// Copies `s` into `buf`, truncating to `size` bytes and always writing a
/// terminating NUL.
///
/// # Safety
///
/// `buf` must point to `size` writable bytes.
pub(crate) unsafe fn write_c_string(s: &str, buf: *mut c_char, size: c_uint) {
    if buf.is_null() || size == 0 {
        return;
    }
    let capacity = size as usize - 1;
    // Truncate on a UTF-8 boundary so we never split a character.
    let mut len = s.len().min(capacity);
    while len > 0 && !s.is_char_boundary(len) {
        len -= 1;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), buf, len);
        buf.add(len).write(0);
    }
}
