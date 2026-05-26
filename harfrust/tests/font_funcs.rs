use std::fs;
use std::path::PathBuf;

use harfrust::{
    funcs::{AdvanceWidthBatch, BuiltinFontFuncs, FontFuncs},
    Direction, FontRef, ShapeOptions, ShaperData, UnicodeBuffer,
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

#[test]
fn font_funcs_nominal_override_is_used() {
    struct ForceNotdef {
        nominal_calls: usize,
    }

    impl FontFuncs for ForceNotdef {
        fn nominal_glyph(&mut self, _: &BuiltinFontFuncs, _: u32) -> Option<GlyphId> {
            self.nominal_calls += 1;
            Some(GlyphId::new(0))
        }
    }

    let mut funcs = ForceNotdef { nominal_calls: 0 };

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        )
    });

    assert!(funcs.nominal_calls > 0);
    assert!(!glyphs.glyph_infos().is_empty());
    assert!(glyphs.glyph_infos().iter().all(|info| info.glyph_id == 0));
}

#[test]
fn font_funcs_default_fallback_is_available() {
    struct DelegatingFuncs {
        nominal_calls: usize,
    }

    impl FontFuncs for DelegatingFuncs {
        fn nominal_glyph(&mut self, builtin: &BuiltinFontFuncs, c: u32) -> Option<GlyphId> {
            self.nominal_calls += 1;
            builtin.nominal_glyph(c)
        }
    }

    let mut funcs = DelegatingFuncs { nominal_calls: 0 };

    let (baseline, with_funcs) = with_test_shaper(|shaper| {
        let baseline = shaper.shape(buffer_with_text("abc"), ShapeOptions::new());
        let with_funcs = shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        );
        (baseline, with_funcs)
    });

    assert!(funcs.nominal_calls > 0);
    assert_eq!(
        baseline
            .glyph_infos()
            .iter()
            .map(|g| g.glyph_id)
            .collect::<Vec<_>>(),
        with_funcs
            .glyph_infos()
            .iter()
            .map(|g| g.glyph_id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn font_funcs_batch_advance_override_is_used() {
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
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
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
fn font_funcs_batch_advance_uses_single_glyph_override_by_default() {
    struct AdvanceOnlyFuncs {
        advance_width_calls: usize,
    }

    impl FontFuncs for AdvanceOnlyFuncs {
        fn advance_width(&mut self, _: &BuiltinFontFuncs, _: GlyphId) -> i32 {
            self.advance_width_calls += 1;
            333
        }
    }

    let mut funcs = AdvanceOnlyFuncs {
        advance_width_calls: 0,
    };

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        )
    });

    assert!(funcs.advance_width_calls >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.x_advance == 333));
}

#[test]
fn font_funcs_batch_hb_raw_view_is_available() {
    struct HbRawFuncs {
        batch_calls: usize,
    }

    impl FontFuncs for HbRawFuncs {
        fn populate_advance_widths(&mut self, _: &BuiltinFontFuncs, batch: AdvanceWidthBatch) {
            self.batch_calls += 1;
            let raw = batch.into_raw();
            assert_eq!(raw.len, 3);
            assert!(!raw.gids.is_null());
            assert!(!raw.advances.is_null());
            assert!(raw.gid_stride > 0);
            assert!(raw.advance_stride > 0);
        }
    }

    let mut funcs = HbRawFuncs { batch_calls: 0 };

    let _ = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("abc"),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        )
    });

    assert!(funcs.batch_calls > 0);
}

#[test]
fn font_funcs_vertical_origin_override_is_used() {
    struct VOriginFuncs {
        v_origin_calls: usize,
    }

    impl FontFuncs for VOriginFuncs {
        fn vertical_origin(&mut self, builtin: &BuiltinFontFuncs, glyph: GlyphId) -> (i32, i32) {
            self.v_origin_calls += 1;
            builtin.vertical_origin(glyph)
        }
    }

    let mut funcs = VOriginFuncs { v_origin_calls: 0 };

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::TopToBottom);
    buffer.guess_segment_properties();
    buffer.set_direction(Direction::TopToBottom);

    let _ = with_test_shaper(|shaper| {
        shaper.shape(buffer, ShapeOptions::new().font_funcs(Some(&mut funcs)))
    });

    assert!(funcs.v_origin_calls >= 2);
}

#[test]
fn font_funcs_batch_advance_not_called_for_empty_buffer() {
    struct BatchAdvanceFuncs {
        batch_calls: usize,
    }

    impl FontFuncs for BatchAdvanceFuncs {
        fn populate_advance_widths(&mut self, _: &BuiltinFontFuncs, _: AdvanceWidthBatch) {
            self.batch_calls += 1;
        }
    }

    let mut funcs = BatchAdvanceFuncs { batch_calls: 0 };

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text(""),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        )
    });

    assert_eq!(funcs.batch_calls, 0);
    assert!(glyphs.glyph_infos().is_empty());
}

#[test]
fn font_funcs_variant_glyph_override_is_used() {
    struct VariantFuncs {
        variant_calls: usize,
    }

    impl FontFuncs for VariantFuncs {
        fn variant_glyph(&mut self, _: &BuiltinFontFuncs, _: u32, _: u32) -> Option<GlyphId> {
            self.variant_calls += 1;
            Some(GlyphId::new(1))
        }
    }

    let mut funcs = VariantFuncs { variant_calls: 0 };

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(
            buffer_with_text("a\u{FE0F}"),
            ShapeOptions::new().font_funcs(Some(&mut funcs)),
        )
    });

    assert!(funcs.variant_calls > 0);
    assert_eq!(glyphs.glyph_infos().len(), 1);
    assert_eq!(glyphs.glyph_infos()[0].glyph_id, 1);
}

#[test]
fn font_funcs_advance_width_override_is_used() {
    struct AdvanceFuncs {
        advance_width_calls: usize,
    }

    impl FontFuncs for AdvanceFuncs {
        fn advance_width(&mut self, _: &BuiltinFontFuncs, _: GlyphId) -> i32 {
            self.advance_width_calls += 1;
            100
        }
    }

    let mut funcs = AdvanceFuncs {
        advance_width_calls: 0,
    };

    let glyphs = with_test_shaper_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fonts")
            .join("in-house")
            .join("d9b8bc10985f24796826c29f7ccba3d0ae11ec02.ttf"),
        |shaper| {
            shaper.shape(
                buffer_with_text("\u{0718}\u{070F}\u{0718}\u{0718}\u{002E}"),
                ShapeOptions::new().font_funcs(Some(&mut funcs)),
            )
        },
    );

    assert!(funcs.advance_width_calls >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .any(|pos| pos.x_advance != 0));
}

#[test]
fn font_funcs_advance_height_override_is_used() {
    struct AdvanceHeightFuncs {
        advance_height_calls: usize,
    }

    impl FontFuncs for AdvanceHeightFuncs {
        fn advance_height(&mut self, _: &BuiltinFontFuncs, _: GlyphId) -> i32 {
            self.advance_height_calls += 1;
            50
        }
    }

    let mut funcs = AdvanceHeightFuncs {
        advance_height_calls: 0,
    };

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::TopToBottom);
    buffer.guess_segment_properties();

    let glyphs = with_test_shaper(|shaper| {
        shaper.shape(buffer, ShapeOptions::new().font_funcs(Some(&mut funcs)))
    });

    assert!(funcs.advance_height_calls >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.y_advance == 50));
}

#[test]
fn font_funcs_extents_override_is_used() {
    struct ExtentsFuncs {
        extents_calls: usize,
    }

    impl FontFuncs for ExtentsFuncs {
        fn extents(
            &mut self,
            default: &BuiltinFontFuncs,
            glyph: GlyphId,
        ) -> Option<harfrust::GlyphExtents> {
            self.extents_calls += 1;
            default.extents(glyph).map(|mut e| {
                e.width = 999;
                e
            })
        }
    }

    let mut funcs = ExtentsFuncs { extents_calls: 0 };

    let glyphs = with_test_shaper_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fonts")
            .join("in-house")
            .join("8228d035fcd65d62ec9728fb34f42c63be93a5d3.ttf"),
        |shaper| {
            shaper.shape(
                buffer_with_text("x\u{0301}X\u{0301}"),
                ShapeOptions::new().font_funcs(Some(&mut funcs)),
            )
        },
    );

    assert!(funcs.extents_calls >= 2);
    assert_eq!(glyphs.glyph_positions().len(), 4);
    assert!(glyphs
        .glyph_positions()
        .iter()
        .any(|pos| pos.x_offset != 0 || pos.y_offset != 0));
}
