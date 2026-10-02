use crate::{text_parser::TextParser, Tag};
use core::{
    ops::{Bound, RangeBounds},
    str::FromStr,
};

pub const HB_FEATURE_GLOBAL_START: u32 = 0;
pub const HB_FEATURE_GLOBAL_END: u32 = u32::MAX;

/// A feature tag with an accompanying range specifying on which subslice of
/// `shape`s input it should be applied.
#[repr(C)]
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Hash, Debug)]
pub struct Feature {
    pub tag: Tag,
    pub value: u32,
    pub start: u32,
    pub end: u32,
}

impl Feature {
    /// Create a new `Feature` struct.
    pub fn new(tag: Tag, value: u32, range: impl RangeBounds<usize>) -> Feature {
        let max = u32::MAX as usize;
        let start = match range.start_bound() {
            Bound::Included(&included) => included.min(max) as u32,
            Bound::Excluded(&excluded) => excluded.min(max - 1) as u32 + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&included) => included.min(max) as u32,
            Bound::Excluded(&excluded) => excluded.saturating_sub(1).min(max) as u32,
            Bound::Unbounded => max as u32,
        };

        Feature {
            tag,
            value,
            start,
            end,
        }
    }

    pub(crate) fn is_global(&self) -> bool {
        self.start == 0 && self.end == u32::MAX
    }
}

impl FromStr for Feature {
    type Err = &'static str;

    /// Parses a `Feature` form a string.
    ///
    /// Possible values:
    ///
    /// - `kern` -> kern .. 1
    /// - `+kern` -> kern .. 1
    /// - `-kern` -> kern .. 0
    /// - `kern=0` -> kern .. 0
    /// - `kern=1` -> kern .. 1
    /// - `aalt=2` -> altr .. 2
    /// - `kern[]` -> kern .. 1
    /// - `kern[:]` -> kern .. 1
    /// - `kern[5:]` -> kern 5.. 1
    /// - `kern[:5]` -> kern ..=5 1
    /// - `kern[3:5]` -> kern 3..=5 1
    /// - `kern[3]` -> kern 3..=4 1
    /// - `aalt[3:5]=2` -> kern 3..=5 1
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        fn parse(s: &str) -> Option<Feature> {
            if s.is_empty() {
                return None;
            }

            let mut p = TextParser::new(s);

            // Parse prefix.
            let mut value = 1;
            match p.curr_byte()? {
                b'-' => {
                    value = 0;
                    p.advance(1);
                }
                b'+' => {
                    value = 1;
                    p.advance(1);
                }
                _ => {}
            }

            // Parse tag.
            p.skip_spaces();
            let quote = p.consume_quote();

            let tag = p.consume_tag()?;

            // Force closing quote.
            if let Some(quote) = quote {
                p.consume_byte(quote)?;
            }

            // Parse indices.
            p.skip_spaces();

            let (start, end) = if p.consume_byte(b'[').is_some() {
                let start_opt = p.consume_i32();
                let start = start_opt.unwrap_or(0) as u32; // negative value overflow is ok

                let end = if matches!(p.curr_byte(), Some(b':' | b';')) {
                    p.advance(1);
                    p.consume_i32().unwrap_or(-1) as u32 // negative value overflow is ok
                } else {
                    if start_opt.is_some() && start != u32::MAX {
                        start + 1
                    } else {
                        u32::MAX
                    }
                };

                p.consume_byte(b']')?;

                (start, end)
            } else {
                (0, u32::MAX)
            };

            // Parse postfix.
            let had_equal = p.consume_byte(b'=').is_some();
            let value1 = p
                .consume_i32()
                .or_else(|| p.consume_bool().map(|b| b as i32));

            if had_equal && value1.is_none() {
                return None;
            }

            if let Some(value1) = value1 {
                value = value1 as u32; // negative value overflow is ok
            }

            p.skip_spaces();

            if !p.at_end() {
                return None;
            }

            Some(Feature {
                tag,
                value,
                start,
                end,
            })
        }

        parse(s).ok_or("invalid feature")
    }
}

#[cfg(test)]
mod tests_features {
    use super::*;
    use core::str::FromStr;

    macro_rules! test {
        ($name:ident, $text:expr, $tag:expr, $value:expr, $range:expr) => {
            #[test]
            fn $name() {
                assert_eq!(
                    Feature::from_str($text).unwrap(),
                    Feature::new(Tag::new($tag), $value, $range)
                );
            }
        };
    }

    test!(parse_01, "kern", b"kern", 1, ..);
    test!(parse_02, "+kern", b"kern", 1, ..);
    test!(parse_03, "-kern", b"kern", 0, ..);
    test!(parse_04, "kern=0", b"kern", 0, ..);
    test!(parse_05, "kern=1", b"kern", 1, ..);
    test!(parse_06, "kern=2", b"kern", 2, ..);
    test!(parse_07, "kern[]", b"kern", 1, ..);
    test!(parse_08, "kern[:]", b"kern", 1, ..);
    test!(parse_09, "kern[5:]", b"kern", 1, 5..);
    test!(parse_10, "kern[:5]", b"kern", 1, ..=5);
    test!(parse_11, "kern[3:5]", b"kern", 1, 3..=5);
    test!(parse_12, "kern[3]", b"kern", 1, 3..=4);
    test!(parse_13, "kern[3:5]=2", b"kern", 2, 3..=5);
    test!(parse_14, "kern[3;5]=2", b"kern", 2, 3..=5);
    test!(parse_15, "kern[:-1]", b"kern", 1, ..);
    test!(parse_16, "kern[-1]", b"kern", 1, u32::MAX as usize..);
    test!(parse_17, "kern=on", b"kern", 1, ..);
    test!(parse_18, "kern=off", b"kern", 0, ..);
    test!(parse_19, "kern=oN", b"kern", 1, ..);
    test!(parse_20, "kern=oFf", b"kern", 0, ..);
}
