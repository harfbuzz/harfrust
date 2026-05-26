use std::fs;
use std::path::PathBuf;

use harfrust::{
    funcs::{AdvanceWidthBatch, BuiltinFontFuncs, FontFuncs},
    FontRef, ShapeOptions, ShaperData, UnicodeBuffer,
};
use read_fonts::types::GlyphId;

fn test_font_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("rb_custom")
        .join("OpenSans.subset1.ttf")
}

fn with_test_shaper<T>(f: impl FnOnce(&harfrust::Shaper) -> T) -> T {
    let font_data = fs::read(test_font_path()).expect("failed to read test font");
    let font = FontRef::new(&font_data).expect("failed to parse test font");
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();
    f(&shaper)
}

fn with_test_shaper_from_path<T>(font_path: PathBuf, f: impl FnOnce(&harfrust::Shaper) -> T) -> T {
    let font_data = fs::read(font_path).expect("failed to read test font");
    let font = FontRef::new(&font_data).expect("failed to parse test font");
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();
    f(&shaper)
}

fn buffer_with_text(text: &str) -> UnicodeBuffer {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer
}

fn assert_positions_scaled(
    baseline: &[harfrust::GlyphPosition],
    scaled: &[harfrust::GlyphPosition],
    factor: i32,
) {
    assert_eq!(baseline.len(), scaled.len());
    for (baseline, scaled) in baseline.iter().zip(scaled) {
        assert_eq!(scaled.x_advance, baseline.x_advance * factor);
        assert_eq!(scaled.y_advance, baseline.y_advance * factor);
        assert!((scaled.x_offset - baseline.x_offset * factor).abs() <= 1);
        assert!((scaled.y_offset - baseline.y_offset * factor).abs() <= 1);
    }
}

#[test]
fn font_funcs_batch_advance_override_is_used_with_scale() {
    struct BatchAdvanceFuncs {
        batch_calls: usize,
    }

    impl FontFuncs for BatchAdvanceFuncs {
        fn populate_advance_widths(&mut self, _: &BuiltinFontFuncs, batch: AdvanceWidthBatch) {
            self.batch_calls += 1;
            assert!(!batch.is_empty());
            for (_, advance) in batch {
                *advance = 777;
            }
        }
    }

    let mut funcs = BatchAdvanceFuncs { batch_calls: 0 };

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new()
                .scale(Some(shaper.units_per_em() * 2))
                .font_funcs(Some(&mut funcs)),
        )
    });

    assert!(funcs.batch_calls > 0);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.x_advance == 777));
}

#[test]
fn font_funcs_advance_width_override_is_not_scaled() {
    struct AdvanceFuncs;

    impl FontFuncs for AdvanceFuncs {
        fn advance_width(&mut self, _: &BuiltinFontFuncs, _: GlyphId) -> i32 {
            100
        }
    }

    let mut funcs = AdvanceFuncs;

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new()
                .scale(Some(shaper.units_per_em() * 2))
                .font_funcs(Some(&mut funcs)),
        )
    });

    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.x_advance == 100));
}

fn aat_kern_font_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("text-rendering-tests")
        .join("TestKERNOne.otf")
}

#[test]
fn aat_kern_scale_doubles_advances_and_offsets() {
    // TestKERNOne.otf: the u-T and T-u pairs carry an AAT kern value, so
    // both advances and any x-offsets must be doubled at 2× scale.
    let (baseline, scaled) = with_test_shaper_from_path(aat_kern_font_path(), |shaper| {
        let text = "\u{0131}\u{0054}\u{0075}\u{0054}\u{0075}\u{0054}\u{0131}";
        let baseline = shaper.shape(buffer_with_text(text), ShapeOptions::new());
        let scaled = shaper.shape(
            buffer_with_text(text),
            ShapeOptions::new().scale(Some(shaper.units_per_em() * 2)),
        );
        (baseline, scaled)
    });

    assert_eq!(baseline.glyph_infos().len(), scaled.glyph_infos().len());
    assert_positions_scaled(baseline.glyph_positions(), scaled.glyph_positions(), 2);
}

#[test]
fn aat_kern_negative_scale_flips_advances() {
    let (baseline, scaled) = with_test_shaper_from_path(aat_kern_font_path(), |shaper| {
        let text = "\u{0131}\u{0054}\u{0075}\u{0054}\u{0075}\u{0054}\u{0131}";
        let baseline = shaper.shape(buffer_with_text(text), ShapeOptions::new());
        let scaled = shaper.shape(
            buffer_with_text(text),
            ShapeOptions::new().scale(Some(-(shaper.units_per_em() * 2))),
        );
        (baseline, scaled)
    });

    assert_eq!(baseline.glyph_infos().len(), scaled.glyph_infos().len());
    for (b, s) in baseline
        .glyph_positions()
        .iter()
        .zip(scaled.glyph_positions())
    {
        assert_eq!(s.x_advance, -(b.x_advance * 2));
        assert_eq!(s.y_advance, -(b.y_advance * 2));
    }
}

#[test]
fn shape_scale_doubles_positioned_output() {
    let font_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("in-house")
        .join("8228d035fcd65d62ec9728fb34f42c63be93a5d3.ttf");

    let (baseline, scaled) = with_test_shaper_from_path(font_path, |shaper| {
        let text = "x\u{0301}AVX\u{0301}";
        let baseline = shaper.shape(buffer_with_text(text), ShapeOptions::new());
        let scaled = shaper.shape(
            buffer_with_text(text),
            ShapeOptions::new().scale(Some(shaper.units_per_em() * 2)),
        );
        (baseline, scaled)
    });

    assert_eq!(baseline.glyph_infos().len(), scaled.glyph_infos().len());
    assert_eq!(
        baseline
            .glyph_infos()
            .iter()
            .map(|info| info.glyph_id)
            .collect::<Vec<_>>(),
        scaled
            .glyph_infos()
            .iter()
            .map(|info| info.glyph_id)
            .collect::<Vec<_>>()
    );
    assert_positions_scaled(baseline.glyph_positions(), scaled.glyph_positions(), 2);
}

#[test]
fn shape_negative_scale_flips_and_doubles_advances() {
    let (baseline, scaled) = with_test_shaper(|shaper| {
        let text = "abc";
        let baseline = shaper.shape(buffer_with_text(text), ShapeOptions::new());
        let scaled = shaper.shape(
            buffer_with_text(text),
            ShapeOptions::new().scale(Some(-(shaper.units_per_em() * 2))),
        );
        (baseline, scaled)
    });

    assert_eq!(baseline.glyph_infos().len(), scaled.glyph_infos().len());
    for (baseline, scaled) in baseline
        .glyph_positions()
        .iter()
        .zip(scaled.glyph_positions())
    {
        assert_eq!(scaled.x_advance, -(baseline.x_advance * 2));
        assert_eq!(scaled.y_advance, -(baseline.y_advance * 2));
    }
}
