use harfrust::{font::Font, shape, Buffer, FontFuncs, GlyphId, ShapeOptions, ShaperFont};

fn font() -> Font {
    Font::new(
        include_bytes!("fonts/negative-var-advances.ttf").to_vec(),
        0,
    )
    .unwrap()
}

#[test]
fn negative_varied_advances_are_clamped_at_every_scale() {
    let font = font();
    for (weight, expected_h, expected_v) in
        [(0.0, [500, 500], [1000, 1000]), (1.0, [0, 600], [0, 1200])]
    {
        let instance = font
            .instance_builder()
            .variations([("wght", weight)])
            .build();
        for factor in [-2, -1, 0, 1, 2] {
            let shaper = ShaperFont::new(&instance).with_scale(factor * 1000);
            for (index, glyph) in [GlyphId::new(1), GlyphId::new(2)].into_iter().enumerate() {
                assert_eq!(shaper.glyph_h_advance(glyph), expected_h[index] * factor);
                assert_eq!(shaper.glyph_v_advance(glyph), -expected_v[index] * factor);
            }

            // Horizontal shaping uses the batched advance query.
            let mut buffer = Buffer::new();
            buffer.push_str("AB");
            buffer.guess_segment_properties();
            shape(&shaper, &mut buffer, ShapeOptions::new()).unwrap();
            let advances: Vec<_> = buffer
                .glyph_positions()
                .iter()
                .map(|p| p.x_advance)
                .collect();
            assert_eq!(advances, expected_h.map(|advance| advance * factor));
        }
    }
}

#[test]
fn signed_callback_advances_are_preserved() {
    struct SignedAdvances;
    impl FontFuncs for SignedAdvances {
        fn glyph_h_advance(&self, _: &ShaperFont, _: GlyphId) -> i32 {
            -17
        }
        fn glyph_v_advance(&self, _: &ShaperFont, _: GlyphId) -> i32 {
            19
        }
    }
    let font = font();
    let instance = font.instance_builder().variations([("wght", 1.0)]).build();
    let shaper = ShaperFont::new(&instance).with_font_funcs(Some(&SignedAdvances));
    assert_eq!(shaper.glyph_h_advance(GlyphId::new(1)), -17);
    assert_eq!(shaper.glyph_v_advance(GlyphId::new(1)), 19);

    let mut buffer = Buffer::new();
    buffer.push_str("AB");
    buffer.guess_segment_properties();
    shape(&shaper, &mut buffer, ShapeOptions::new()).unwrap();
    assert!(buffer.glyph_positions().iter().all(|p| p.x_advance == -17));
}
