use crate::{text_parser::TextParser, Tag};
use core::str::FromStr;

/// A font variation.
#[repr(C)]
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Variation {
    pub tag: Tag,
    pub value: f32,
}

impl FromStr for Variation {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        fn parse(s: &str) -> Option<Variation> {
            if s.is_empty() {
                return None;
            }

            let mut p = TextParser::new(s);

            // Parse tag.
            p.skip_spaces();
            let quote = p.consume_quote();

            let tag = p.consume_tag()?;

            // Force closing quote.
            if let Some(quote) = quote {
                p.consume_byte(quote)?;
            }

            let _ = p.consume_byte(b'=');
            let value = p.consume_f32()?;
            p.skip_spaces();

            if !p.at_end() {
                return None;
            }

            Some(Variation { tag, value })
        }

        parse(s).ok_or("invalid variation")
    }
}

// The following From impls are designed to match the convenience
// impls in skrifa which have proven to be fairly useful in practice.
impl From<&Variation> for Variation {
    fn from(value: &Variation) -> Self {
        *value
    }
}

impl From<(&str, f32)> for Variation {
    fn from(value: (&str, f32)) -> Self {
        Self {
            tag: Tag::from_str(value.0).unwrap_or_default(),
            value: value.1,
        }
    }
}

impl From<&(&str, f32)> for Variation {
    fn from(value: &(&str, f32)) -> Self {
        (*value).into()
    }
}

impl From<(Tag, f32)> for Variation {
    fn from(value: (Tag, f32)) -> Self {
        Self {
            tag: value.0,
            value: value.1,
        }
    }
}

impl From<&(Tag, f32)> for Variation {
    fn from(value: &(Tag, f32)) -> Self {
        (*value).into()
    }
}
