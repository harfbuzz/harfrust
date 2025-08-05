#![feature(test)]
#![allow(dead_code)]
#![allow(unused_imports)]

extern crate test;

use harfrust::Tag;
use read_fonts::{
    types::{F2Dot14, Fixed},
    TableProvider,
};

#[derive(Copy, Clone)]
struct CustomVariation {
    tag: Tag,
    value: f32,
}

impl Into<harfrust::Variation> for CustomVariation {
    fn into(self) -> harfrust::Variation {
        harfrust::Variation { tag: self.tag, value: self.value }
    }
}

impl Into<harfbuzz_rs::Variation> for CustomVariation {
    fn into(self) -> harfbuzz_rs::Variation {
        harfbuzz_rs::Variation::new(harfbuzz_rs::Tag(u32::from_be_bytes(self.tag.to_be_bytes())), self.value)
    }
}

macro_rules! simple_bench {
    ($name:ident, $font_path:expr, $text_path:expr) => {
        simple_bench!($name, $font_path, $text_path, []);
    };

    ($name:ident, $font_path:expr, $text_path:expr, $variations:expr) => {
        mod $name {
            use super::*;
            use test::Bencher;

            #[bench]
            fn cold_hr(bencher: &mut Bencher) {
                let font_data = std::fs::read($font_path).unwrap();
                let text = std::fs::read_to_string($text_path).unwrap().trim().to_string();
                bencher.iter(|| {
                    test::black_box({
                        let font = harfrust::FontRef::from_index(&font_data, 0).unwrap();
                        let data = harfrust::ShaperData::new(&font);
                        let vars: &[CustomVariation] = $variations.as_slice();
                        let instance = harfrust::ShaperInstance::from_variations(&font, vars.iter().map(|var| (var.tag, var.value)));
                        let shaper = data.shaper(&font).instance(Some(&instance)).build();
                        let mut buffer = harfrust::UnicodeBuffer::new();
                        buffer.push_str(&text);
                        buffer.guess_segment_properties();
                        let shape_plan = harfrust::ShapePlan::new(&shaper, buffer.direction(), Some(buffer.script()), buffer.language().as_ref(), &[]);
                        shaper.shape_with_plan(&shape_plan, buffer, &[])
                    });
                })
            }

            #[cfg(feature = "hb")]
            #[bench]
            fn cold_hb(bencher: &mut Bencher) {
                let font_data = std::fs::read($font_path).unwrap();
                let text = std::fs::read_to_string($text_path).unwrap().trim().to_string();
                bencher.iter(|| {
                    test::black_box({
                        let face = harfbuzz_rs::Face::from_bytes(&font_data, 0);
                        let mut font = harfbuzz_rs::Font::new(face);
                        let vars: &[CustomVariation] = $variations.as_slice();
                        let vars = vars.iter().copied().map(|var| var.into()).collect::<Vec<harfbuzz_rs::Variation>>();
                        font.set_variations(&vars);
                        let buffer = harfbuzz_rs::UnicodeBuffer::new().add_str(&text);
                        harfbuzz_rs::shape(&font, buffer, &[])
                    });
                })
            }

            #[bench]
            fn warm_hr(bencher: &mut Bencher) {
                let text = std::fs::read_to_string($text_path).unwrap().trim().to_string();
                let font_data = std::fs::read($font_path).unwrap();
                let font = harfrust::FontRef::from_index(&font_data, 0).unwrap();
                let data = harfrust::ShaperData::new(&font);
                let vars: &[CustomVariation] = $variations.as_slice();
                let instance = harfrust::ShaperInstance::from_variations(&font, vars.iter().map(|var| (var.tag, var.value)));
                let shaper = data.shaper(&font).instance(Some(&instance)).build();
                let mut buffer = harfrust::UnicodeBuffer::new();
                buffer.push_str(&text);
                buffer.guess_segment_properties();
                let shape_plan = harfrust::ShapePlan::new(&shaper, buffer.direction(), Some(buffer.script()), buffer.language().as_ref(), &[]);
                let mut buffer = Some(harfrust::UnicodeBuffer::new());
                bencher.iter(|| {
                    test::black_box({
                        let mut filled_buffer = buffer.take().unwrap();
                        filled_buffer.push_str(&text);
                        let glyph_buffer = shaper.shape_with_plan(&shape_plan, filled_buffer, &[]);
                        buffer = Some(glyph_buffer.clear());
                    });
                })
            }

            #[cfg(feature = "hb")]
            #[bench]
            fn warm_hb(bencher: &mut Bencher) {
                let font_data = std::fs::read($font_path).unwrap();
                let face = harfbuzz_rs::Face::from_bytes(&font_data, 0);
                let mut font = harfbuzz_rs::Font::new(face);
                let vars: &[CustomVariation] = $variations.as_slice();
                let vars = vars.iter().copied().map(|var| var.into()).collect::<Vec<harfbuzz_rs::Variation>>();
                font.set_variations(&vars);
                let text = std::fs::read_to_string($text_path).unwrap().trim().to_string();
                let mut buffer = Some(harfbuzz_rs::UnicodeBuffer::new());
                bencher.iter(|| {
                    test::black_box({
                        let filled_buffer = buffer.take().unwrap().add_str(&text);
                        let glyph_buffer = harfbuzz_rs::shape(&font, filled_buffer, &[]);
                        buffer = Some(glyph_buffer.clear());
                    });
                })
            }
        }
    };
}

simple_bench!(nastaliq_urdu_little_prince, "fonts/NotoNastaliqUrdu-Regular.ttf", "texts/fa-thelittleprince.txt");
simple_bench!(nastaliq_urdu_words, "fonts/NotoNastaliqUrdu-Regular.ttf", "texts/fa-words.txt");

simple_bench!(amiri_little_prince, "fonts/Amiri-Regular.ttf", "texts/fa-thelittleprince.txt");

simple_bench!(devanagari_words, "fonts/NotoSansDevanagari-Regular.ttf", "texts/hi-words.txt");

simple_bench!(roboto_little_prince, "fonts/Roboto-Regular.ttf", "texts/en-thelittleprince.txt");
simple_bench!(roboto_words, "fonts/Roboto-Regular.ttf", "texts/en-words.txt");

simple_bench!(source_serif_variable_react_dom, "fonts/SourceSerifVariable-Roman.ttf", "texts/react-dom.txt");
