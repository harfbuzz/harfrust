use core::convert::TryFrom;

use super::ucd_table::ucd::*;
use crate::hb::algs::*;
use crate::Script;

// Space estimates based on:
// https://unicode.org/charts/PDF/U2000.pdf
// https://docs.microsoft.com/en-us/typography/develop/character-design-standards/whitespace
pub mod hb_unicode_funcs_t {
    pub type space_t = u8;
    pub const NOT_SPACE: u8 = 0;
    pub const SPACE_EM: u8 = 1;
    pub const SPACE_EM_2: u8 = 2;
    pub const SPACE_EM_3: u8 = 3;
    pub const SPACE_EM_4: u8 = 4;
    pub const SPACE_EM_5: u8 = 5;
    pub const SPACE_EM_6: u8 = 6;
    pub const SPACE_EM_16: u8 = 16;
    pub const SPACE_4_EM_18: u8 = 17; // 4/18th of an EM!
    pub const SPACE: u8 = 18;
    pub const SPACE_FIGURE: u8 = 19;
    pub const SPACE_PUNCTUATION: u8 = 20;
    pub const SPACE_NARROW: u8 = 21;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum hb_unicode_general_category_t {
    ClosePunctuation,
    ConnectorPunctuation,
    Control,
    CurrencySymbol,
    DashPunctuation,
    DecimalNumber,
    EnclosingMark,
    FinalPunctuation,
    Format,
    InitialPunctuation,
    LetterNumber,
    LineSeparator,
    LowercaseLetter,
    MathSymbol,
    ModifierLetter,
    ModifierSymbol,
    NonspacingMark,
    OpenPunctuation,
    OtherLetter,
    OtherNumber,
    OtherPunctuation,
    OtherSymbol,
    ParagraphSeparator,
    PrivateUse,
    SpaceSeparator,
    SpacingMark,
    Surrogate,
    TitlecaseLetter,
    Unassigned,
    UppercaseLetter,
}

#[allow(dead_code)]
pub mod combining_class {
    pub const NotReordered: u8 = 0;
    pub const Overlay: u8 = 1;
    pub const Nukta: u8 = 7;
    pub const KanaVoicing: u8 = 8;
    pub const Virama: u8 = 9;

    /* Hebrew */
    pub const CCC10: u8 = 10;
    pub const CCC11: u8 = 11;
    pub const CCC12: u8 = 12;
    pub const CCC13: u8 = 13;
    pub const CCC14: u8 = 14;
    pub const CCC15: u8 = 15;
    pub const CCC16: u8 = 16;
    pub const CCC17: u8 = 17;
    pub const CCC18: u8 = 18;
    pub const CCC19: u8 = 19;
    pub const CCC20: u8 = 20;
    pub const CCC21: u8 = 21;
    pub const CCC22: u8 = 22;
    pub const CCC23: u8 = 23;
    pub const CCC24: u8 = 24;
    pub const CCC25: u8 = 25;
    pub const CCC26: u8 = 26;

    /* Arabic */
    pub const CCC27: u8 = 27;
    pub const CCC28: u8 = 28;
    pub const CCC29: u8 = 29;
    pub const CCC30: u8 = 30;
    pub const CCC31: u8 = 31;
    pub const CCC32: u8 = 32;
    pub const CCC33: u8 = 33;
    pub const CCC34: u8 = 34;
    pub const CCC35: u8 = 35;

    /* Syriac */
    pub const CCC36: u8 = 36;

    /* Telugu */
    pub const CCC84: u8 = 84;
    pub const CCC91: u8 = 91;

    /* Thai */
    pub const CCC103: u8 = 103;
    pub const CCC107: u8 = 107;

    /* Lao */
    pub const CCC118: u8 = 118;
    pub const CCC122: u8 = 122;

    /* Tibetan */
    pub const CCC129: u8 = 129;
    pub const CCC130: u8 = 130;
    pub const CCC132: u8 = 132;

    pub const AttachedBelowLeft: u8 = 200;
    pub const AttachedBelow: u8 = 202;
    pub const AttachedAbove: u8 = 214;
    pub const AttachedAboveRight: u8 = 216;
    pub const BelowLeft: u8 = 218;
    pub const Below: u8 = 220;
    pub const BelowRight: u8 = 222;
    pub const Left: u8 = 224;
    pub const Right: u8 = 226;
    pub const AboveLeft: u8 = 228;
    pub const Above: u8 = 230;
    pub const AboveRight: u8 = 232;
    pub const DoubleBelow: u8 = 233;
    pub const DoubleAbove: u8 = 234;

    pub const IotaSubscript: u8 = 240;

    pub const Invalid: u8 = 255;
}

#[allow(dead_code)]
pub mod modified_combining_class {
    // Hebrew
    //
    // We permute the "fixed-position" classes 10-26 into the order
    // described in the SBL Hebrew manual:
    //
    // https://www.sbl-site.org/Fonts/SBLHebrewUserManual1.5x.pdf
    //
    // (as recommended by:
    //  https://forum.fontlab.com/archive-old-microsoft-volt-group/vista-and-diacritic-ordering/msg22823/)
    //
    // More details here:
    // https://bugzilla.mozilla.org/show_bug.cgi?id=662055
    pub const CCC10: u8 = 22; // sheva
    pub const CCC11: u8 = 15; // hataf segol
    pub const CCC12: u8 = 16; // hataf patah
    pub const CCC13: u8 = 17; // hataf qamats
    pub const CCC14: u8 = 23; // hiriq
    pub const CCC15: u8 = 18; // tsere
    pub const CCC16: u8 = 19; // segol
    pub const CCC17: u8 = 20; // patah
    pub const CCC18: u8 = 21; // qamats & qamats qatan
    pub const CCC19: u8 = 14; // holam & holam haser for vav
    pub const CCC20: u8 = 24; // qubuts
    pub const CCC21: u8 = 12; // dagesh
    pub const CCC22: u8 = 25; // meteg
    pub const CCC23: u8 = 13; // rafe
    pub const CCC24: u8 = 10; // shin dot
    pub const CCC25: u8 = 11; // sin dot
    pub const CCC26: u8 = 26; // point varika

    // Arabic
    //
    // Modify to move Shadda (ccc=33) before other marks.  See:
    // https://unicode.org/faq/normalization.html#8
    // https://unicode.org/faq/normalization.html#9
    pub const CCC27: u8 = 28; // fathatan
    pub const CCC28: u8 = 29; // dammatan
    pub const CCC29: u8 = 30; // kasratan
    pub const CCC30: u8 = 31; // fatha
    pub const CCC31: u8 = 32; // damma
    pub const CCC32: u8 = 33; // kasra
    pub const CCC33: u8 = 27; // shadda
    pub const CCC34: u8 = 34; // sukun
    pub const CCC35: u8 = 35; // superscript alef

    // Syriac
    pub const CCC36: u8 = 36; // superscript alaph

    // Telugu
    //
    // Modify Telugu length marks (ccc=84, ccc=91).
    // These are the only matras in the main Indic scripts range that have
    // a non-zero ccc.  That makes them reorder with the Halant that is
    // ccc=9.  Just zero them, we don't need them in our Indic shaper.
    pub const CCC84: u8 = 0; // length mark
    pub const CCC91: u8 = 0; // ai length mark

    // Thai
    //
    // Modify U+0E38 and U+0E39 (ccc=103) to be reordered before U+0E3A (ccc=9).
    // Assign 3, which is unassigned otherwise.
    // Uniscribe does this reordering too.
    pub const CCC103: u8 = 3; // sara u / sara uu
    pub const CCC107: u8 = 107; // mai *

    // Lao
    pub const CCC118: u8 = 118; // sign u / sign uu
    pub const CCC122: u8 = 122; // mai *

    // Tibetan
    //
    // In case of multiple vowel-signs, use u first (but after achung)
    // this allows Dzongkha multi-vowel shortcuts to render correctly
    pub const CCC129: u8 = 129; // sign aa
    pub const CCC130: u8 = 132; // sign i
    pub const CCC132: u8 = 131; // sign u
}

#[rustfmt::skip]
const MODIFIED_COMBINING_CLASS: &[u8; 256] = &[
    combining_class::NotReordered,
    combining_class::Overlay,
    2, 3, 4, 5, 6,
    combining_class::Nukta,
    combining_class::KanaVoicing,
    combining_class::Virama,

    // Hebrew
    modified_combining_class::CCC10,
    modified_combining_class::CCC11,
    modified_combining_class::CCC12,
    modified_combining_class::CCC13,
    modified_combining_class::CCC14,
    modified_combining_class::CCC15,
    modified_combining_class::CCC16,
    modified_combining_class::CCC17,
    modified_combining_class::CCC18,
    modified_combining_class::CCC19,
    modified_combining_class::CCC20,
    modified_combining_class::CCC21,
    modified_combining_class::CCC22,
    modified_combining_class::CCC23,
    modified_combining_class::CCC24,
    modified_combining_class::CCC25,
    modified_combining_class::CCC26,

    // Arabic
    modified_combining_class::CCC27,
    modified_combining_class::CCC28,
    modified_combining_class::CCC29,
    modified_combining_class::CCC30,
    modified_combining_class::CCC31,
    modified_combining_class::CCC32,
    modified_combining_class::CCC33,
    modified_combining_class::CCC34,
    modified_combining_class::CCC35,

    // Syriac
    modified_combining_class::CCC36,

    37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83,

    // Telugu
    modified_combining_class::CCC84,
    85, 86, 87, 88, 89, 90,
    modified_combining_class::CCC91,
    92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102,

    // Thai
    modified_combining_class::CCC103,
    104, 105, 106,
    modified_combining_class::CCC107,
    108, 109, 110, 111, 112, 113, 114, 115, 116, 117,

    // Lao
    modified_combining_class::CCC118,
    119, 120, 121,
    modified_combining_class::CCC122,
    123, 124, 125, 126, 127, 128,

    // Tibetan
    modified_combining_class::CCC129,
    modified_combining_class::CCC130,
    131,
    modified_combining_class::CCC132,
    133, 134, 135, 136, 137, 138, 139,


    140, 141, 142, 143, 144, 145, 146, 147, 148, 149,
    150, 151, 152, 153, 154, 155, 156, 157, 158, 159,
    160, 161, 162, 163, 164, 165, 166, 167, 168, 169,
    170, 171, 172, 173, 174, 175, 176, 177, 178, 179,
    180, 181, 182, 183, 184, 185, 186, 187, 188, 189,
    190, 191, 192, 193, 194, 195, 196, 197, 198, 199,

    combining_class::AttachedBelowLeft,
    201,
    combining_class::AttachedBelow,
    203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
    combining_class::AttachedAbove,
    215,
    combining_class::AttachedAboveRight,
    217,
    combining_class::BelowLeft,
    219,
    combining_class::Below,
    221,
    combining_class::BelowRight,
    223,
    combining_class::Left,
    225,
    combining_class::Right,
    227,
    combining_class::AboveLeft,
    229,
    combining_class::Above,
    231,
    combining_class::AboveRight,
    combining_class::DoubleBelow,
    combining_class::DoubleAbove,
    235, 236, 237, 238, 239,
    combining_class::IotaSubscript,
    241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254,
    combining_class::Invalid,
];

pub trait GeneralCategoryExt {
    fn to_u32(&self) -> u32;
    fn from_u32(gc: u32) -> Self;
    fn is_mark(&self) -> bool;
    fn is_letter(&self) -> bool;
}

#[rustfmt::skip]
impl GeneralCategoryExt for hb_unicode_general_category_t {
    fn to_u32(&self) -> u32 {
        match *self {
            hb_unicode_general_category_t::ClosePunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_CLOSE_PUNCTUATION,
            hb_unicode_general_category_t::ConnectorPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_CONNECT_PUNCTUATION,
            hb_unicode_general_category_t::Control => hb_gc::HB_UNICODE_GENERAL_CATEGORY_CONTROL,
            hb_unicode_general_category_t::CurrencySymbol => hb_gc::HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL,
            hb_unicode_general_category_t::DashPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_DASH_PUNCTUATION,
            hb_unicode_general_category_t::DecimalNumber => hb_gc::HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER,
            hb_unicode_general_category_t::EnclosingMark => hb_gc::HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK,
            hb_unicode_general_category_t::FinalPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_FINAL_PUNCTUATION,
            hb_unicode_general_category_t::Format => hb_gc::HB_UNICODE_GENERAL_CATEGORY_FORMAT,
            hb_unicode_general_category_t::InitialPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_INITIAL_PUNCTUATION,
            hb_unicode_general_category_t::LetterNumber => hb_gc::HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER,
            hb_unicode_general_category_t::LineSeparator => hb_gc::HB_UNICODE_GENERAL_CATEGORY_LINE_SEPARATOR,
            hb_unicode_general_category_t::LowercaseLetter => hb_gc::HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER,
            hb_unicode_general_category_t::MathSymbol => hb_gc::HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL,
            hb_unicode_general_category_t::ModifierLetter => hb_gc::HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER,
            hb_unicode_general_category_t::ModifierSymbol => hb_gc::HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL,
            hb_unicode_general_category_t::NonspacingMark => hb_gc::HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK,
            hb_unicode_general_category_t::OpenPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_OPEN_PUNCTUATION,
            hb_unicode_general_category_t::OtherLetter => hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER,
            hb_unicode_general_category_t::OtherNumber => hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER,
            hb_unicode_general_category_t::OtherPunctuation => hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_PUNCTUATION,
            hb_unicode_general_category_t::OtherSymbol => hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL,
            hb_unicode_general_category_t::ParagraphSeparator => hb_gc::HB_UNICODE_GENERAL_CATEGORY_PARAGRAPH_SEPARATOR,
            hb_unicode_general_category_t::PrivateUse => hb_gc::HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE,
            hb_unicode_general_category_t::SpaceSeparator => hb_gc::HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR,
            hb_unicode_general_category_t::SpacingMark => hb_gc::HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK,
            hb_unicode_general_category_t::Surrogate => hb_gc::HB_UNICODE_GENERAL_CATEGORY_SURROGATE,
            hb_unicode_general_category_t::TitlecaseLetter => hb_gc::HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER,
            hb_unicode_general_category_t::Unassigned => hb_gc::HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED,
            hb_unicode_general_category_t::UppercaseLetter => hb_gc::HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER
        }
    }

    fn from_u32(gc: u32) -> Self {
        match gc {
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_CLOSE_PUNCTUATION => hb_unicode_general_category_t::ClosePunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_CONNECT_PUNCTUATION => hb_unicode_general_category_t::ConnectorPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_CONTROL => hb_unicode_general_category_t::Control,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL => hb_unicode_general_category_t::CurrencySymbol,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_DASH_PUNCTUATION => hb_unicode_general_category_t::DashPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER => hb_unicode_general_category_t::DecimalNumber,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK => hb_unicode_general_category_t::EnclosingMark,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_FINAL_PUNCTUATION => hb_unicode_general_category_t::FinalPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_FORMAT => hb_unicode_general_category_t::Format,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_INITIAL_PUNCTUATION => hb_unicode_general_category_t::InitialPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER => hb_unicode_general_category_t::LetterNumber,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_LINE_SEPARATOR => hb_unicode_general_category_t::LineSeparator,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER => hb_unicode_general_category_t::LowercaseLetter,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL => hb_unicode_general_category_t::MathSymbol,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER => hb_unicode_general_category_t::ModifierLetter,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL => hb_unicode_general_category_t::ModifierSymbol,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK => hb_unicode_general_category_t::NonspacingMark,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_OPEN_PUNCTUATION => hb_unicode_general_category_t::OpenPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER => hb_unicode_general_category_t::OtherLetter,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER => hb_unicode_general_category_t::OtherNumber,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_PUNCTUATION => hb_unicode_general_category_t::OtherPunctuation,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL => hb_unicode_general_category_t::OtherSymbol,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_PARAGRAPH_SEPARATOR => hb_unicode_general_category_t::ParagraphSeparator,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE => hb_unicode_general_category_t::PrivateUse,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR => hb_unicode_general_category_t::SpaceSeparator,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK => hb_unicode_general_category_t::SpacingMark,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_SURROGATE => hb_unicode_general_category_t::Surrogate,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER => hb_unicode_general_category_t::TitlecaseLetter,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED => hb_unicode_general_category_t::Unassigned,
            hb_gc::HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER => hb_unicode_general_category_t::UppercaseLetter,
            _ => unreachable!()
        }
    }

    fn is_mark(&self) -> bool {
        matches!(*self, 
            hb_unicode_general_category_t::SpacingMark |
            hb_unicode_general_category_t::EnclosingMark |
            hb_unicode_general_category_t::NonspacingMark)
    }

    fn is_letter(&self) -> bool {
        matches!(*self, 
            hb_unicode_general_category_t::LowercaseLetter |
            hb_unicode_general_category_t::ModifierLetter |
            hb_unicode_general_category_t::OtherLetter |
            hb_unicode_general_category_t::TitlecaseLetter |
            hb_unicode_general_category_t::UppercaseLetter)
    }
}

pub trait CharExt {
    fn script(self) -> Script;
    fn general_category(self) -> hb_unicode_general_category_t;
    fn space_fallback(self) -> hb_unicode_funcs_t::space_t;
    fn combining_class(self) -> u8;
    fn modified_combining_class(self) -> u8;
    fn mirrored(self) -> Option<char>;
    fn is_emoji_extended_pictographic(self) -> bool;
    fn is_default_ignorable(self) -> bool;
    fn is_variation_selector(self) -> bool;
    fn vertical(self) -> Option<char>;
}

impl CharExt for char {
    fn script(self) -> Script {
        _hb_ucd_sc_map[_hb_ucd_sc(self as usize) as usize]
    }

    fn general_category(self) -> hb_unicode_general_category_t {
        hb_unicode_general_category_t::from_u32(_hb_ucd_gc(self as usize) as u32)
    }

    fn space_fallback(self) -> hb_unicode_funcs_t::space_t {
        use hb_unicode_funcs_t::*;

        // All GC=Zs chars that can use a fallback.
        match self {
            '\u{0020}' => SPACE,             // SPACE
            '\u{00A0}' => SPACE,             // NO-BREAK SPACE
            '\u{2000}' => SPACE_EM_2,        // EN QUAD
            '\u{2001}' => SPACE_EM,          // EM QUAD
            '\u{2002}' => SPACE_EM_2,        // EN SPACE
            '\u{2003}' => SPACE_EM,          // EM SPACE
            '\u{2004}' => SPACE_EM_3,        // THREE-PER-EM SPACE
            '\u{2005}' => SPACE_EM_4,        // FOUR-PER-EM SPACE
            '\u{2006}' => SPACE_EM_6,        // SIX-PER-EM SPACE
            '\u{2007}' => SPACE_FIGURE,      // FIGURE SPACE
            '\u{2008}' => SPACE_PUNCTUATION, // PUNCTUATION SPACE
            '\u{2009}' => SPACE_EM_5,        // THIN SPACE
            '\u{200A}' => SPACE_EM_16,       // HAIR SPACE
            '\u{202F}' => SPACE_NARROW,      // NARROW NO-BREAK SPACE
            '\u{205F}' => SPACE_4_EM_18,     // MEDIUM MATHEMATICAL SPACE
            '\u{3000}' => SPACE_EM,          // IDEOGRAPHIC SPACE
            _ => NOT_SPACE,                  // OGHAM SPACE MARK
        }
    }

    fn combining_class(self) -> u8 {
        _hb_ucd_ccc(self as usize)
    }

    fn modified_combining_class(self) -> u8 {
        let u = self;

        // Reorder SAKOT to ensure it comes after any tone marks.
        if u == '\u{1A60}' {
            return 254;
        }

        // Reorder PADMA to ensure it comes after any vowel marks.
        if u == '\u{0FC6}' {
            return 254;
        }

        // Reorder TSA -PHRU to reorder before U+0F74
        if u == '\u{0F39}' {
            return 127;
        }

        let k = u.combining_class();

        MODIFIED_COMBINING_CLASS[k as usize]
    }

    fn mirrored(self) -> Option<char> {
        let delta = _hb_ucd_bmg(self as usize);
        if delta == 0 {
            None
        } else {
            char::from_u32(((self as i32).wrapping_add(delta as i32)) as u32)
        }
    }

    fn is_emoji_extended_pictographic(self) -> bool {
        // Generated by scripts/gen-unicode-is-emoji-ext-pict.py
        match self as u32 {
            0x00A9 => true,
            0x00AE => true,
            0x203C => true,
            0x2049 => true,
            0x2122 => true,
            0x2139 => true,
            0x2194..=0x2199 => true,
            0x21A9..=0x21AA => true,
            0x231A..=0x231B => true,
            0x2328 => true,
            0x2388 => true,
            0x23CF => true,
            0x23E9..=0x23F3 => true,
            0x23F8..=0x23FA => true,
            0x24C2 => true,
            0x25AA..=0x25AB => true,
            0x25B6 => true,
            0x25C0 => true,
            0x25FB..=0x25FE => true,
            0x2600..=0x2605 => true,
            0x2607..=0x2612 => true,
            0x2614..=0x2685 => true,
            0x2690..=0x2705 => true,
            0x2708..=0x2712 => true,
            0x2714 => true,
            0x2716 => true,
            0x271D => true,
            0x2721 => true,
            0x2728 => true,
            0x2733..=0x2734 => true,
            0x2744 => true,
            0x2747 => true,
            0x274C => true,
            0x274E => true,
            0x2753..=0x2755 => true,
            0x2757 => true,
            0x2763..=0x2767 => true,
            0x2795..=0x2797 => true,
            0x27A1 => true,
            0x27B0 => true,
            0x27BF => true,
            0x2934..=0x2935 => true,
            0x2B05..=0x2B07 => true,
            0x2B1B..=0x2B1C => true,
            0x2B50 => true,
            0x2B55 => true,
            0x3030 => true,
            0x303D => true,
            0x3297 => true,
            0x3299 => true,
            0x1F000..=0x1F0FF => true,
            0x1F10D..=0x1F10F => true,
            0x1F12F => true,
            0x1F16C..=0x1F171 => true,
            0x1F17E..=0x1F17F => true,
            0x1F18E => true,
            0x1F191..=0x1F19A => true,
            0x1F1AD..=0x1F1E5 => true,
            0x1F201..=0x1F20F => true,
            0x1F21A => true,
            0x1F22F => true,
            0x1F232..=0x1F23A => true,
            0x1F23C..=0x1F23F => true,
            0x1F249..=0x1F3FA => true,
            0x1F400..=0x1F53D => true,
            0x1F546..=0x1F64F => true,
            0x1F680..=0x1F6FF => true,
            0x1F774..=0x1F77F => true,
            0x1F7D5..=0x1F7FF => true,
            0x1F80C..=0x1F80F => true,
            0x1F848..=0x1F84F => true,
            0x1F85A..=0x1F85F => true,
            0x1F888..=0x1F88F => true,
            0x1F8AE..=0x1F8FF => true,
            0x1F90C..=0x1F93A => true,
            0x1F93C..=0x1F945 => true,
            0x1F947..=0x1FAFF => true,
            0x1FC00..=0x1FFFD => true,
            _ => false,
        }
    }

    /// Default_Ignorable codepoints:
    ///
    /// Note: While U+115F, U+1160, U+3164 and U+FFA0 are Default_Ignorable,
    /// we do NOT want to hide them, as the way Uniscribe has implemented them
    /// is with regular spacing glyphs, and that's the way fonts are made to work.
    /// As such, we make exceptions for those four.
    /// Also ignoring U+1BCA0..1BCA3. https://github.com/harfbuzz/harfbuzz/issues/503
    ///
    /// Unicode 14.0:
    /// $ grep '; Default_Ignorable_Code_Point ' DerivedCoreProperties.txt | sed 's/;.*#/#/'
    /// 00AD          # Cf       SOFT HYPHEN
    /// 034F          # Mn       COMBINING GRAPHEME JOINER
    /// 061C          # Cf       ARABIC LETTER MARK
    /// 115F..1160    # Lo   [2] HANGUL CHOSEONG FILLER..HANGUL JUNGSEONG FILLER
    /// 17B4..17B5    # Mn   [2] KHMER VOWEL INHERENT AQ..KHMER VOWEL INHERENT AA
    /// 180B..180D    # Mn   [3] MONGOLIAN FREE VARIATION SELECTOR ONE..MONGOLIAN FREE VARIATION SELECTOR THREE
    /// 180E          # Cf       MONGOLIAN VOWEL SEPARATOR
    /// 180F          # Mn       MONGOLIAN FREE VARIATION SELECTOR FOUR
    /// 200B..200F    # Cf   [5] ZERO WIDTH SPACE..RIGHT-TO-LEFT MARK
    /// 202A..202E    # Cf   [5] LEFT-TO-RIGHT EMBEDDING..RIGHT-TO-LEFT OVERRIDE
    /// 2060..2064    # Cf   [5] WORD JOINER..INVISIBLE PLUS
    /// 2065          # Cn       <reserved-2065>
    /// 2066..206F    # Cf  [10] LEFT-TO-RIGHT ISOLATE..NOMINAL DIGIT SHAPES
    /// 3164          # Lo       HANGUL FILLER
    /// FE00..FE0F    # Mn  [16] VARIATION SELECTOR-1..VARIATION SELECTOR-16
    /// FEFF          # Cf       ZERO WIDTH NO-BREAK SPACE
    /// FFA0          # Lo       HALFWIDTH HANGUL FILLER
    /// FFF0..FFF8    # Cn   [9] <reserved-FFF0>..<reserved-FFF8>
    /// 1BCA0..1BCA3  # Cf   [4] SHORTHAND FORMAT LETTER OVERLAP..SHORTHAND FORMAT UP STEP
    /// 1D173..1D17A  # Cf   [8] MUSICAL SYMBOL BEGIN BEAM..MUSICAL SYMBOL END PHRASE
    /// E0000         # Cn       <reserved-E0000>
    /// E0001         # Cf       LANGUAGE TAG
    /// E0002..E001F  # Cn  [30] <reserved-E0002>..<reserved-E001F>
    /// E0020..E007F  # Cf  [96] TAG SPACE..CANCEL TAG
    /// E0080..E00FF  # Cn [128] <reserved-E0080>..<reserved-E00FF>
    /// E0100..E01EF  # Mn [240] VARIATION SELECTOR-17..VARIATION SELECTOR-256
    /// E01F0..E0FFF  # Cn [3600] <reserved-E01F0>..<reserved-E0FFF>
    fn is_default_ignorable(self) -> bool {
        let ch = u32::from(self);
        let plane = ch >> 16;
        if plane == 0 {
            // BMP
            let page = ch >> 8;
            match page {
                0x00 => ch == 0x00AD,
                0x03 => ch == 0x034F,
                0x06 => ch == 0x061C,
                0x17 => (0x17B4..=0x17B5).contains(&ch),
                0x18 => (0x180B..=0x180E).contains(&ch),
                0x20 => {
                    (0x200B..=0x200F).contains(&ch)
                        || (0x202A..=0x202E).contains(&ch)
                        || (0x2060..=0x206F).contains(&ch)
                }
                0xFE => (0xFE00..=0xFE0F).contains(&ch) || ch == 0xFEFF,
                0xFF => (0xFFF0..=0xFFF8).contains(&ch),
                _ => false,
            }
        } else {
            // Other planes
            match plane {
                0x01 => (0x1D173..=0x1D17A).contains(&ch),
                0x0E => (0xE0000..=0xE0FFF).contains(&ch),
                _ => false,
            }
        }
    }

    fn is_variation_selector(self) -> bool {
        // U+180B..180D, U+180F MONGOLIAN FREE VARIATION SELECTORs are handled in the
        //Arabic shaper. No need to match them here.
        let ch = u32::from(self);
        (0x0FE00..=0x0FE0F).contains(&ch) || // VARIATION SELECTOR - 1..16
        (0xE0100..=0xE01EF).contains(&ch) // VARIATION SELECTOR - 17..256
    }

    fn vertical(self) -> Option<char> {
        Some(match u32::from(self) >> 8 {
            0x20 => match self {
                '\u{2013}' => '\u{fe32}', // EN DASH
                '\u{2014}' => '\u{fe31}', // EM DASH
                '\u{2025}' => '\u{fe30}', // TWO DOT LEADER
                '\u{2026}' => '\u{fe19}', // HORIZONTAL ELLIPSIS
                _ => return None,
            },
            0x30 => match self {
                '\u{3001}' => '\u{fe11}', // IDEOGRAPHIC COMMA
                '\u{3002}' => '\u{fe12}', // IDEOGRAPHIC FULL STOP
                '\u{3008}' => '\u{fe3f}', // LEFT ANGLE BRACKET
                '\u{3009}' => '\u{fe40}', // RIGHT ANGLE BRACKET
                '\u{300a}' => '\u{fe3d}', // LEFT DOUBLE ANGLE BRACKET
                '\u{300b}' => '\u{fe3e}', // RIGHT DOUBLE ANGLE BRACKET
                '\u{300c}' => '\u{fe41}', // LEFT CORNER BRACKET
                '\u{300d}' => '\u{fe42}', // RIGHT CORNER BRACKET
                '\u{300e}' => '\u{fe43}', // LEFT WHITE CORNER BRACKET
                '\u{300f}' => '\u{fe44}', // RIGHT WHITE CORNER BRACKET
                '\u{3010}' => '\u{fe3b}', // LEFT BLACK LENTICULAR BRACKET
                '\u{3011}' => '\u{fe3c}', // RIGHT BLACK LENTICULAR BRACKET
                '\u{3014}' => '\u{fe39}', // LEFT TORTOISE SHELL BRACKET
                '\u{3015}' => '\u{fe3a}', // RIGHT TORTOISE SHELL BRACKET
                '\u{3016}' => '\u{fe17}', // LEFT WHITE LENTICULAR BRACKET
                '\u{3017}' => '\u{fe18}', // RIGHT WHITE LENTICULAR BRACKET
                _ => return None,
            },
            0xfe => match self {
                '\u{fe4f}' => '\u{fe34}', // WAVY LOW LINE
                _ => return None,
            },
            0xff => match self {
                '\u{ff01}' => '\u{fe15}', // FULLWIDTH EXCLAMATION MARK
                '\u{ff08}' => '\u{fe35}', // FULLWIDTH LEFT PARENTHESIS
                '\u{ff09}' => '\u{fe36}', // FULLWIDTH RIGHT PARENTHESIS
                '\u{ff0c}' => '\u{fe10}', // FULLWIDTH COMMA
                '\u{ff1a}' => '\u{fe13}', // FULLWIDTH COLON
                '\u{ff1b}' => '\u{fe14}', // FULLWIDTH SEMICOLON
                '\u{ff1f}' => '\u{fe16}', // FULLWIDTH QUESTION MARK
                '\u{ff3b}' => '\u{fe47}', // FULLWIDTH LEFT SQUARE BRACKET
                '\u{ff3d}' => '\u{fe48}', // FULLWIDTH RIGHT SQUARE BRACKET
                '\u{ff3f}' => '\u{fe33}', // FULLWIDTH LOW LINE
                '\u{ff5b}' => '\u{fe37}', // FULLWIDTH LEFT CURLY BRACKET
                '\u{ff5d}' => '\u{fe38}', // FULLWIDTH RIGHT CURLY BRACKET
                _ => return None,
            },
            _ => return None,
        })
    }
}

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

pub fn compose(a: char, b: char) -> Option<char> {
    // Hangul is handled algorithmically.
    if let Some(ab) = compose_hangul(a, b) {
        return Some(ab);
    }

    let a = a as u32;
    let b = b as u32;
    let u: u32;

    if (a & 0xFFFF_F800) == 0x0000 && (b & 0xFFFF_FF80) == 0x0300 {
        /* If "a" is small enough and "b" is in the U+0300 range,
         * the composition data is encoded in a 32bit array sorted
         * by "a,b" pair. */
        let k = HB_CODEPOINT_ENCODE3_11_7_14(a, b, 0);
        let v = _hb_ucd_dm2_u32_map
            .binary_search_by(|probe| {
                let key = probe & HB_CODEPOINT_ENCODE3_11_7_14(0x001F_FFFF, 0x001F_FFFF, 0);
                key.cmp(&k)
            })
            .ok()
            .map(|index| _hb_ucd_dm2_u32_map[index]);

        if let Some(value) = v {
            u = HB_CODEPOINT_DECODE3_11_7_14_3(value);
        } else {
            return None;
        }
    } else {
        /* Otherwise it is stored in a 64bit array sorted by
         * "a,b" pair. */
        let k = HB_CODEPOINT_ENCODE3(a, b, 0);
        let v = _hb_ucd_dm2_u64_map
            .binary_search_by(|probe| {
                let key = probe & HB_CODEPOINT_ENCODE3(0x001F_FFFF, 0x001F_FFFF, 0);
                key.cmp(&k)
            })
            .ok()
            .map(|index| _hb_ucd_dm2_u64_map[index]);

        if let Some(value) = v {
            u = HB_CODEPOINT_DECODE3_3(value);
        } else {
            return None;
        }
    }

    if u == 0 {
        None
    } else {
        char::from_u32(u)
    }
}

fn compose_hangul(a: char, b: char) -> Option<char> {
    let l = u32::from(a);
    let v = u32::from(b);
    if L_BASE <= l && l < (L_BASE + L_COUNT) && V_BASE <= v && v < (V_BASE + V_COUNT) {
        let r = S_BASE + (l - L_BASE) * N_COUNT + (v - V_BASE) * T_COUNT;
        Some(char::try_from(r).unwrap())
    } else if S_BASE <= l
        && l <= (S_BASE + S_COUNT - T_COUNT)
        && T_BASE <= v
        && v < (T_BASE + T_COUNT)
        && (l - S_BASE) % T_COUNT == 0
    {
        let r = l + (v - T_BASE);
        Some(char::try_from(r).unwrap())
    } else {
        None
    }
}

pub fn decompose(ab: char) -> Option<(char, char)> {
    if let Some((a, b)) = decompose_hangul(ab) {
        return Some((a, b));
    }

    let mut i = _hb_ucd_dm(ab as usize) as usize;

    // If no data, there's no decomposition.
    if i == 0 {
        return None;
    }
    i -= 1;

    if i < _hb_ucd_dm1_p0_map.len() + _hb_ucd_dm1_p2_map.len() {
        let a = if i < _hb_ucd_dm1_p0_map.len() {
            _hb_ucd_dm1_p0_map[i] as u32
        } else {
            let j = i - _hb_ucd_dm1_p0_map.len();
            0x20000 | _hb_ucd_dm1_p2_map[j] as u32
        };
        return char::from_u32(a).map(|a_char| (a_char, '\0'));
    }

    i -= _hb_ucd_dm1_p0_map.len() + _hb_ucd_dm1_p2_map.len();

    if i < _hb_ucd_dm2_u32_map.len() {
        let v = _hb_ucd_dm2_u32_map[i];
        let a = HB_CODEPOINT_DECODE3_11_7_14_1(v);
        let b = HB_CODEPOINT_DECODE3_11_7_14_2(v);
        return match (char::from_u32(a), char::from_u32(b)) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };
    }

    i -= _hb_ucd_dm2_u32_map.len();

    let v = _hb_ucd_dm2_u64_map[i];
    let a = HB_CODEPOINT_DECODE3_1(v);
    let b = HB_CODEPOINT_DECODE3_2(v);
    match (char::from_u32(a), char::from_u32(b)) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    }
}

pub fn decompose_hangul(ab: char) -> Option<(char, char)> {
    let si = u32::from(ab).wrapping_sub(S_BASE);
    if si >= S_COUNT {
        return None;
    }

    let (a, b) = if si % T_COUNT != 0 {
        // LV,T
        (S_BASE + (si / T_COUNT) * T_COUNT, T_BASE + (si % T_COUNT))
    } else {
        // L,V
        (L_BASE + (si / N_COUNT), V_BASE + (si % N_COUNT) / T_COUNT)
    };

    Some((char::try_from(a).unwrap(), char::try_from(b).unwrap()))
}

pub mod hb_gc {
    pub const HB_UNICODE_GENERAL_CATEGORY_CONTROL: u32 = 0;
    pub const HB_UNICODE_GENERAL_CATEGORY_FORMAT: u32 = 1;
    pub const HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED: u32 = 2;
    pub const HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE: u32 = 3;
    pub const HB_UNICODE_GENERAL_CATEGORY_SURROGATE: u32 = 4;
    pub const HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER: u32 = 5;
    pub const HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER: u32 = 6;
    pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER: u32 = 7;
    pub const HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER: u32 = 8;
    pub const HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER: u32 = 9;
    pub const HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK: u32 = 10;
    pub const HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK: u32 = 11;
    pub const HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK: u32 = 12;
    pub const HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER: u32 = 13;
    pub const HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER: u32 = 14;
    pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER: u32 = 15;
    pub const HB_UNICODE_GENERAL_CATEGORY_CONNECT_PUNCTUATION: u32 = 16;
    pub const HB_UNICODE_GENERAL_CATEGORY_DASH_PUNCTUATION: u32 = 17;
    pub const HB_UNICODE_GENERAL_CATEGORY_CLOSE_PUNCTUATION: u32 = 18;
    pub const HB_UNICODE_GENERAL_CATEGORY_FINAL_PUNCTUATION: u32 = 19;
    pub const HB_UNICODE_GENERAL_CATEGORY_INITIAL_PUNCTUATION: u32 = 20;
    pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_PUNCTUATION: u32 = 21;
    pub const HB_UNICODE_GENERAL_CATEGORY_OPEN_PUNCTUATION: u32 = 22;
    pub const HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL: u32 = 23;
    pub const HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL: u32 = 24;
    pub const HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL: u32 = 25;
    pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL: u32 = 26;
    pub const HB_UNICODE_GENERAL_CATEGORY_LINE_SEPARATOR: u32 = 27;
    pub const HB_UNICODE_GENERAL_CATEGORY_PARAGRAPH_SEPARATOR: u32 = 28;
    pub const HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR: u32 = 29;
}
