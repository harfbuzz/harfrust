use std::cell::Cell;
use std::fs;
use std::path::PathBuf;

use harfrust::{Advances, Buffer, Direction, FontFuncs, ShaperFont};
use read_fonts::types::GlyphId;

fn test_font_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("rb_custom")
        .join("OpenSans.subset1.ttf")
}

fn with_test_shaper<T>(f: impl FnOnce(&ShaperFont) -> T) -> T {
    let font_data = fs::read(test_font_path()).expect("failed to read test font");
    let font = harfrust::font::Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    f(&shaper)
}

fn with_test_shaper_from_path<T>(font_path: PathBuf, f: impl FnOnce(&ShaperFont) -> T) -> T {
    let font_data = fs::read(font_path).expect("failed to read test font");
    let font = harfrust::font::Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    f(&shaper)
}

fn buffer_with_text(text: &str) -> Buffer {
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer
}

#[derive(Default)]
struct TestOptions<'a> {
    funcs: Option<&'a dyn FontFuncs>,
}

impl<'a> TestOptions<'a> {
    fn new() -> Self {
        Self::default()
    }
    fn font_funcs(mut self, funcs: Option<&'a dyn FontFuncs>) -> Self {
        self.funcs = funcs;
        self
    }
}

fn shape_test(shaper: &ShaperFont<'_, '_>, mut buffer: Buffer, options: TestOptions<'_>) -> Buffer {
    let mut font = ShaperFont::new(shaper);
    font.set_font_funcs(options.funcs);
    harfrust::shape(&font, &mut buffer, harfrust::ShapeOptions::new()).unwrap();
    buffer
}

#[test]
fn shape_font_allows_cross_query_callbacks() {
    struct CrossQuery {
        extents_calls: Cell<usize>,
    }

    impl FontFuncs for CrossQuery {
        fn glyph_v_origin(&self, font: &ShaperFont, glyph: GlyphId) -> (i32, i32) {
            let _ = font.glyph_extents(glyph);
            font.default_glyph_v_origin(glyph)
        }

        fn glyph_extents(
            &self,
            font: &ShaperFont,
            glyph: GlyphId,
        ) -> Option<harfrust::GlyphExtents> {
            self.extents_calls.set(self.extents_calls.get() + 1);
            font.default_glyph_extents(glyph)
        }
    }

    with_test_shaper(|shaper| {
        let funcs = CrossQuery {
            extents_calls: Cell::new(0),
        };
        let mut font = ShaperFont::new(shaper);
        let glyph = GlyphId::new(1);
        let default_advance = font.default_glyph_h_advance(glyph);
        font.set_scale(i32::from(shaper.units_per_em()) * 2);
        assert_eq!(font.default_glyph_h_advance(glyph), default_advance * 2);
        font.set_font_funcs(Some(&funcs));
        let _ = font.glyph_v_origin(glyph);
        // The callback's own query and the default vertical-origin fallback
        // both consult the overridden extents.
        assert_eq!(funcs.extents_calls.get(), 2);
    });
}

#[test]
fn font_funcs_nominal_override_is_used() {
    struct ForceNotdef {
        nominal_calls: Cell<usize>,
    }

    impl FontFuncs for ForceNotdef {
        fn nominal_glyph(&self, _: &ShaperFont, _: u32) -> Option<GlyphId> {
            self.nominal_calls.set(self.nominal_calls.get() + 1);
            Some(GlyphId::new(0))
        }
    }

    let funcs = ForceNotdef {
        nominal_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("abc"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert!(funcs.nominal_calls.get() > 0);
    assert!(!glyphs.glyph_infos().is_empty());
    assert!(glyphs.glyph_infos().iter().all(|info| info.glyph_id == 0));
}

#[test]
fn font_funcs_default_fallback_is_available() {
    struct DelegatingFuncs {
        nominal_calls: Cell<usize>,
    }

    impl FontFuncs for DelegatingFuncs {
        fn nominal_glyph(&self, builtin: &ShaperFont, c: u32) -> Option<GlyphId> {
            self.nominal_calls.set(self.nominal_calls.get() + 1);
            builtin.default_nominal_glyph(c)
        }
    }

    let funcs = DelegatingFuncs {
        nominal_calls: Cell::new(0),
    };

    let (baseline, with_funcs) = with_test_shaper(|shaper| {
        let baseline = shape_test(shaper, buffer_with_text("abc"), TestOptions::new());
        let with_funcs = shape_test(
            shaper,
            buffer_with_text("abc"),
            TestOptions::new().font_funcs(Some(&funcs)),
        );
        (baseline, with_funcs)
    });

    assert!(funcs.nominal_calls.get() > 0);
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
fn font_funcs_nominal_override_bypasses_cmap_cache() {
    struct RangeFuncs {
        max_char: u32,
        nominal_calls: Cell<usize>,
    }

    impl FontFuncs for RangeFuncs {
        fn nominal_glyph(&self, builtin: &ShaperFont, c: u32) -> Option<GlyphId> {
            self.nominal_calls.set(self.nominal_calls.get() + 1);
            if c <= self.max_char {
                builtin.default_nominal_glyph(c)
            } else {
                None
            }
        }
    }

    let maps_abc = RangeFuncs {
        max_char: u32::from('c'),
        nominal_calls: Cell::new(0),
    };
    let maps_a = RangeFuncs {
        max_char: u32::from('a'),
        nominal_calls: Cell::new(0),
    };

    let (first, second) = with_test_shaper_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fonts")
            .join("rb_custom")
            .join("PT_Sans-Caption-Web-Regular.ttf"),
        |shaper| {
            let first = shape_test(
                shaper,
                buffer_with_text("abc"),
                TestOptions::new().font_funcs(Some(&maps_abc)),
            );
            let second = shape_test(
                shaper,
                buffer_with_text("abc"),
                TestOptions::new().font_funcs(Some(&maps_a)),
            );
            (first, second)
        },
    );

    assert!(maps_abc.nominal_calls.get() > 0);
    assert!(maps_a.nominal_calls.get() > 0);
    assert_ne!(
        first
            .glyph_infos()
            .iter()
            .map(|g| g.glyph_id)
            .collect::<Vec<_>>(),
        second
            .glyph_infos()
            .iter()
            .map(|g| g.glyph_id)
            .collect::<Vec<_>>()
    );
    assert!(second.glyph_infos().iter().any(|info| info.glyph_id == 0));
}

#[test]
fn arabic_win1256_fallback_is_applied() {
    struct Win1256Funcs;

    impl FontFuncs for Win1256Funcs {
        fn nominal_glyph(&self, _: &ShaperFont, c: u32) -> Option<GlyphId> {
            let glyph = match c {
                0x0627 => 199, // ALEF
                0x0644 => 225, // LAM
                0x0645 => 229, // MEEM
                0x0649 => 236, // ALEF MAKSURA
                0x064A => 237, // YEH
                0x0652 => 250, // SUKUN
                _ => return None,
            };
            Some(GlyphId::new(glyph))
        }
    }

    let funcs = Win1256Funcs;
    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("لم"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert_eq!(
        glyphs
            .glyph_infos()
            .iter()
            .map(|info| info.glyph_id)
            .collect::<Vec<_>>(),
        [152, 141],
    );
}

#[test]
fn font_funcs_batch_advance_override_is_used() {
    struct BatchAdvanceFuncs {
        batch_calls: Cell<usize>,
    }

    impl FontFuncs for BatchAdvanceFuncs {
        fn glyph_h_advances(&self, _: &ShaperFont, batch: Advances) {
            self.batch_calls.set(self.batch_calls.get() + 1);
            assert!(!batch.is_empty());
            for (_, advance) in batch {
                *advance = 777;
            }
        }
    }

    let funcs = BatchAdvanceFuncs {
        batch_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("abc"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert!(funcs.batch_calls.get() > 0);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.x_advance == 777));
}

#[test]
fn font_funcs_batch_advance_uses_single_glyph_override_by_default() {
    struct AdvanceOnlyFuncs {
        advance_width_calls: Cell<usize>,
    }

    impl FontFuncs for AdvanceOnlyFuncs {
        fn glyph_h_advance(&self, _: &ShaperFont, _: GlyphId) -> i32 {
            self.advance_width_calls
                .set(self.advance_width_calls.get() + 1);
            333
        }
    }

    let funcs = AdvanceOnlyFuncs {
        advance_width_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("abc"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert!(funcs.advance_width_calls.get() >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.x_advance == 333));
}

#[test]
fn font_funcs_batch_hb_raw_view_is_available() {
    struct HbRawFuncs {
        batch_calls: Cell<usize>,
    }

    impl FontFuncs for HbRawFuncs {
        fn glyph_h_advances(&self, _: &ShaperFont, batch: Advances) {
            self.batch_calls.set(self.batch_calls.get() + 1);
            let raw = batch.into_raw();
            assert_eq!(raw.len, 3);
            assert!(!raw.gids.is_null());
            assert!(!raw.advances.is_null());
            assert!(raw.gid_stride > 0);
            assert!(raw.advance_stride > 0);
        }
    }

    let funcs = HbRawFuncs {
        batch_calls: Cell::new(0),
    };

    let _ = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("abc"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert!(funcs.batch_calls.get() > 0);
}

#[test]
fn font_funcs_vertical_origin_override_is_used() {
    struct VOriginFuncs {
        v_origin_calls: Cell<usize>,
    }

    impl FontFuncs for VOriginFuncs {
        fn glyph_v_origin(&self, builtin: &ShaperFont, glyph: GlyphId) -> (i32, i32) {
            self.v_origin_calls.set(self.v_origin_calls.get() + 1);
            builtin.default_glyph_v_origin(glyph)
        }
    }

    let funcs = VOriginFuncs {
        v_origin_calls: Cell::new(0),
    };

    let mut buffer = Buffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::TopToBottom);
    buffer.guess_segment_properties();
    buffer.set_direction(Direction::TopToBottom);

    let _ = with_test_shaper(|shaper| {
        shape_test(shaper, buffer, TestOptions::new().font_funcs(Some(&funcs)))
    });

    assert!(funcs.v_origin_calls.get() >= 2);
}

#[test]
fn font_funcs_batch_advance_not_called_for_empty_buffer() {
    struct BatchAdvanceFuncs {
        batch_calls: Cell<usize>,
    }

    impl FontFuncs for BatchAdvanceFuncs {
        fn glyph_h_advances(&self, _: &ShaperFont, _: Advances) {
            self.batch_calls.set(self.batch_calls.get() + 1);
        }
    }

    let funcs = BatchAdvanceFuncs {
        batch_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text(""),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert_eq!(funcs.batch_calls.get(), 0);
    assert!(glyphs.glyph_infos().is_empty());
}

#[test]
fn font_funcs_variant_glyph_override_is_used() {
    struct VariantFuncs {
        variant_calls: Cell<usize>,
    }

    impl FontFuncs for VariantFuncs {
        fn variation_glyph(&self, _: &ShaperFont, _: u32, _: u32) -> Option<GlyphId> {
            self.variant_calls.set(self.variant_calls.get() + 1);
            Some(GlyphId::new(1))
        }
    }

    let funcs = VariantFuncs {
        variant_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper(|shaper| {
        shape_test(
            shaper,
            buffer_with_text("a\u{FE0F}"),
            TestOptions::new().font_funcs(Some(&funcs)),
        )
    });

    assert!(funcs.variant_calls.get() > 0);
    assert_eq!(glyphs.glyph_infos().len(), 1);
    assert_eq!(glyphs.glyph_infos()[0].glyph_id, 1);
}

#[test]
fn font_funcs_advance_width_override_is_used() {
    struct AdvanceFuncs {
        advance_width_calls: Cell<usize>,
    }

    impl FontFuncs for AdvanceFuncs {
        fn glyph_h_advance(&self, _: &ShaperFont, _: GlyphId) -> i32 {
            self.advance_width_calls
                .set(self.advance_width_calls.get() + 1);
            100
        }
    }

    let funcs = AdvanceFuncs {
        advance_width_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fonts")
            .join("in-house")
            .join("d9b8bc10985f24796826c29f7ccba3d0ae11ec02.ttf"),
        |shaper| {
            shape_test(
                shaper,
                buffer_with_text("\u{0718}\u{070F}\u{0718}\u{0718}\u{002E}"),
                TestOptions::new().font_funcs(Some(&funcs)),
            )
        },
    );

    assert!(funcs.advance_width_calls.get() >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .any(|pos| pos.x_advance != 0));
}

#[test]
fn font_funcs_advance_height_override_is_used() {
    struct AdvanceHeightFuncs {
        advance_height_calls: Cell<usize>,
    }

    impl FontFuncs for AdvanceHeightFuncs {
        fn glyph_v_advance(&self, _: &ShaperFont, _: GlyphId) -> i32 {
            self.advance_height_calls
                .set(self.advance_height_calls.get() + 1);
            50
        }
    }

    let funcs = AdvanceHeightFuncs {
        advance_height_calls: Cell::new(0),
    };

    let mut buffer = Buffer::new();
    buffer.push_str("abc");
    buffer.set_direction(Direction::TopToBottom);
    buffer.guess_segment_properties();

    let glyphs = with_test_shaper(|shaper| {
        shape_test(shaper, buffer, TestOptions::new().font_funcs(Some(&funcs)))
    });

    assert!(funcs.advance_height_calls.get() >= 2);
    assert!(!glyphs.glyph_positions().is_empty());
    assert!(glyphs
        .glyph_positions()
        .iter()
        .all(|pos| pos.y_advance == 50));
}

#[test]
fn font_funcs_extents_override_is_used() {
    struct ExtentsFuncs {
        extents_calls: Cell<usize>,
    }

    impl FontFuncs for ExtentsFuncs {
        fn glyph_extents(
            &self,
            default: &ShaperFont,
            glyph: GlyphId,
        ) -> Option<harfrust::GlyphExtents> {
            self.extents_calls.set(self.extents_calls.get() + 1);
            default.default_glyph_extents(glyph).map(|mut e| {
                e.width = 999;
                e
            })
        }
    }

    let funcs = ExtentsFuncs {
        extents_calls: Cell::new(0),
    };

    let glyphs = with_test_shaper_from_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fonts")
            .join("in-house")
            .join("8228d035fcd65d62ec9728fb34f42c63be93a5d3.ttf"),
        |shaper| {
            shape_test(
                shaper,
                buffer_with_text("x\u{0301}X\u{0301}"),
                TestOptions::new().font_funcs(Some(&funcs)),
            )
        },
    );

    assert!(funcs.extents_calls.get() >= 2);
    assert_eq!(glyphs.glyph_positions().len(), 4);
    assert!(glyphs
        .glyph_positions()
        .iter()
        .any(|pos| pos.x_offset != 0 || pos.y_offset != 0));
}

#[test]
fn no_advance_past_the_last_glyph_the_face_has() {
    // A malformed cmap can point shaping at a glyph the face does not have.
    // HarfBuzz answers no advance for one, rather than repeating the last
    // advance it does have.
    with_test_shaper(|shaper| {
        assert!(shaper.default_glyph_h_advance(GlyphId::from(1u32)) > 0);
        assert_eq!(shaper.default_glyph_h_advance(GlyphId::from(60_000u32)), 0);
    });
}

#[test]
fn glyph_extents_start_at_the_side_bearing() {
    // Undocumented rasterizer behaviour that HarfBuzz matches: the glyph is
    // shifted left by (lsb - xMin), so the ink starts at the left side
    // bearing and not at the bounding box the glyph carries. In this face
    // the two differ: the box starts at 258 and the bearing at 0.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("in-house")
        .join("ffa0f5d2d9025486d8469d8b1fdd983e7632499b.ttf");
    with_test_shaper_from_path(path, |shaper| {
        let extents = shaper
            .default_glyph_extents(GlyphId::from(6u32))
            .expect("the face has this glyph");
        assert_eq!(extents.x_bearing, 0);
        assert_eq!(extents.y_bearing, 1505);
        assert_eq!(extents.width, 752);
        assert_eq!(extents.height, -264);
    });
}
