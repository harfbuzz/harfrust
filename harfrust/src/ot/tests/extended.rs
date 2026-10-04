use super::apply::ApplyContext;
use super::*;
use crate::{Buffer, Direction, LayoutData, Scale};
use alloc::vec;

fn apply_subtable(lookup_type: u16, is_subst: bool, subtable: &[u8], glyphs: &[u32]) -> Buffer {
    apply_subtable_configured(lookup_type, is_subst, subtable, glyphs, |_| {})
}

fn apply_subtable_configured(
    lookup_type: u16,
    is_subst: bool,
    subtable: &[u8],
    glyphs: &[u32],
    configure: impl FnOnce(&mut ApplyContext),
) -> Buffer {
    let mut bytes = vec![0; 8];
    bytes[..2].copy_from_slice(&lookup_type.to_be_bytes());
    bytes[4..6].copy_from_slice(&1u16.to_be_bytes());
    bytes[6..8].copy_from_slice(&8u16.to_be_bytes());
    bytes.extend_from_slice(subtable);
    let host = lookup::LookupData {
        offset: 0,
        is_subst,
        table_data: FontData::new(&bytes),
    };
    let lookup = LookupInfo::new(&host).unwrap();
    assert_eq!(lookup.subtables.len(), 1);
    let mut buffer = Buffer::new();
    assert!(buffer.set_length(glyphs.len()));
    buffer.set_direction(Direction::LeftToRight);
    buffer.allocate_gsubgpos_vars();
    for (info, glyph) in buffer.glyph_infos_mut().iter_mut().zip(glyphs) {
        info.glyph_id = *glyph;
        info.mask = 1;
    }
    if is_subst && !lookup.is_reverse() {
        buffer.clear_output();
    } else if !is_subst {
        buffer.clear_positions();
    }
    let layout = LayoutData {
        ot: &EMPTY_OT_DATA,
        aat: &aat::EMPTY_AAT_DATA,
        units_per_em: 1000,
        apply_trak: false,
    };
    let mut ctx = ApplyContext::new(
        if is_subst {
            LayoutTableKind::Gsub
        } else {
            LayoutTableKind::Gpos
        },
        layout,
        Scale::default(),
        &mut buffer,
    );
    ctx.lookup_props = lookup.props();
    configure(&mut ctx);
    ctx.update_matchers();
    if lookup.is_reverse() {
        for index in (0..ctx.buffer.len).rev() {
            ctx.buffer.idx = index;
            lookup.apply(&mut ctx, &bytes, false);
        }
    } else {
        while ctx.buffer.idx < ctx.buffer.len {
            if lookup.apply(&mut ctx, &bytes, false).is_none() {
                ctx.buffer.next_glyph();
            }
        }
    }
    if is_subst && !lookup.is_reverse() {
        assert!(buffer.sync());
    }
    buffer
}

fn table_data(bytes: &[u8]) -> Vec<u8> {
    let mut data = vec![0; 2];
    data.extend_from_slice(bytes);
    data
}

#[test]
fn wide_coverage_indices_and_counts_are_not_truncated() {
    let bytes = table_data(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 1, 0, 0]);
    let data = FontData::new(&bytes);
    let coverage = CoverageInfo::new(&data, 2u32).unwrap();
    assert_eq!(coverage.index(&data, GlyphId::new(65538)), Some(65538));
    assert_eq!(coverage.index(&data, GlyphId::new(2)), None);

    let mut bytes = table_data(&[0, 3, 1, 0, 1]);
    for gid in 0..=65536 {
        bytes.extend_from_slice(&Uint24::new(gid).to_be_bytes());
    }
    let data = FontData::new(&bytes);
    let coverage = CoverageInfo::new(&data, 2u32).unwrap();
    assert_eq!(coverage.count, 65537);
    assert_eq!(coverage.index(&data, GlyphId::new(65536)), Some(65536));
}

#[test]
fn wide_classes_do_not_alias_narrow_classes() {
    let bytes = table_data(&[0, 3, 1, 0, 0, 0, 0, 2, 1, 0, 1, 0xFF, 0xFF, 0xFF]);
    let data = FontData::new(&bytes);
    let class_def = ClassDefInfo::new(&data, 2u32).unwrap();
    assert_eq!(class_def.class(&data, GlyphId::new(65536)), 65537);
    assert_eq!(class_def.class(&data, GlyphId::new(65537)), 0x00FF_FFFF);
    assert_eq!(class_def.class(&data, GlyphId::new(0)), 0);

    let bytes = table_data(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0xFF, 0xFF]);
    let data = FontData::new(&bytes);
    let class_def = ClassDefInfo::new(&data, 2u32).unwrap();
    assert_eq!(class_def.class(&data, GlyphId::new(65538)), 65535);
    assert_eq!(class_def.class(&data, GlyphId::new(2)), 0);
}

#[test]
fn wide_values_bypass_compact_mapping_caches() {
    let cache = MappingCache::new();
    let glyph = GlyphId::new(7);
    let wide = 65537;
    assert_eq!(
        coverage_index_cached(|_| Some(wide), glyph, &cache),
        Some(wide)
    );
    assert_eq!(cache.get(glyph.into()), None);
    assert_eq!(coverage_index_cached(|_| None, glyph, &cache), None);
    assert_eq!(cache.get(glyph.into()), Some(MappingCache::MAX_VALUE));

    let cache = MappingCache::new();
    assert_eq!(glyph_class_cached(|_| wide, glyph, &cache), wide);
    assert_eq!(cache.get(glyph.into()), None);
    assert_eq!(glyph_class_cached(|_| 1, glyph, &cache), 1);
    assert_eq!(cache.get(glyph.into()), Some(1));
    assert_eq!(glyph_class_cached(|_| 2, GlyphId::new(65543), &cache), 2);
    assert_eq!(cache.get(glyph.into()), Some(1));
}

#[test]
fn digests_and_mark_bitmaps_cover_wide_glyphs() {
    for bytes in [
        &[0, 3, 0, 0, 2, 1, 0, 0, 1, 0, 2][..],
        &[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 0, 0][..],
    ] {
        let coverage = CoverageTable::read(FontData::new(bytes)).unwrap();
        let digest = SetDigest::from_coverage(&coverage);
        let bitmap = MarkGlyphSetBitmap::from_coverage(&coverage);
        assert!(matches!(
            bitmap,
            MarkGlyphSetBitmap::Page { bias: 65536, .. }
        ));
        for glyph in coverage.iter() {
            assert!(digest.may_have(glyph.to_u32()));
            assert_eq!(bitmap.covers(glyph.to_u32()), Some(true));
        }
        assert_eq!(bitmap.covers(0), Some(false));
        assert_eq!(bitmap.covers(65539), Some(false));
    }
}

#[test]
fn gdef_14_caches_resolved_wide_tables() {
    let mut bytes = vec![0; 65536];
    bytes[..4].copy_from_slice(&[0, 1, 0, 4]);
    bytes[18..22].copy_from_slice(&65536u32.to_be_bytes());
    bytes.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 3]);
    let mark_class_offset = bytes.len() as u32;
    bytes[30..34].copy_from_slice(&mark_class_offset.to_be_bytes());
    bytes.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 7]);
    let mark_sets_offset = bytes.len() as u32;
    bytes[34..38].copy_from_slice(&mark_sets_offset.to_be_bytes());
    bytes.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 8]);
    bytes.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 0, 0]);
    let gdef = Gdef::read(FontData::new(&bytes)).unwrap();
    let cache = GdefCache::new(&gdef);
    let data = gdef.offset_data();
    assert_eq!(cache.classes.unwrap().class(&data, GlyphId::new(65538)), 3);
    assert_eq!(
        cache
            .mark_classes
            .unwrap()
            .class(&data, GlyphId::new(65538)),
        7
    );
    assert_eq!(cache.mark_set_offsets, [mark_sets_offset + 8]);
    assert_eq!(cache.mark_set_bitmaps[0].covers(65538), Some(true));
}

#[test]
fn truncated_wide_arrays_do_not_match() {
    for bytes in [&[0, 3, 0, 0, 1][..], &[0, 4, 0, 0, 1][..]] {
        let bytes = table_data(bytes);
        let data = FontData::new(&bytes);
        let coverage = CoverageInfo::new(&data, 2u32).unwrap();
        assert_eq!(coverage.index(&data, GlyphId::new(0)), None);
    }
    for bytes in [&[0, 3, 1, 0, 0, 0, 0, 1][..], &[0, 4, 0, 0, 1][..]] {
        let bytes = table_data(bytes);
        let data = FontData::new(&bytes);
        let class_def = ClassDefInfo::new(&data, 2u32).unwrap();
        assert_eq!(class_def.class(&data, GlyphId::new(65536)), 0);
    }
}

#[test]
fn invalid_offsets_and_overflowing_legacy_indices_do_not_match() {
    let bytes = table_data(&[0, 2, 0, 1, 0, 0, 0, 2, 0xFF, 0xFF]);
    let data = FontData::new(&bytes);
    let coverage = CoverageInfo::new(&data, 2u32).unwrap();
    assert_eq!(coverage.index(&data, GlyphId::new(0)), Some(65535));
    assert_eq!(coverage.index(&data, GlyphId::new(2)), None);
    assert!(CoverageInfo::new(&data, u32::MAX).is_none());
    assert!(ClassDefInfo::new(&data, u32::MAX).is_none());
}

#[test]
fn single_subst3_wraps_signed_deltas_in_24_bits() {
    use read_fonts::types::Int24;
    for (glyph, delta, expected) in [
        (65535, 1, 65536),
        (65536, -1, 65535),
        (0x00FF_FFFF, 1, 0),
        (0, -1, 0x00FF_FFFF),
    ] {
        let mut subtable = vec![0, 3, 0, 0, 0, 9];
        subtable.extend_from_slice(&Int24::new(delta).to_be_bytes());
        subtable.extend_from_slice(&[0, 3, 0, 0, 1]);
        subtable.extend_from_slice(&Uint24::new(glyph).to_be_bytes());
        let output = apply_subtable(1, true, &subtable, &[glyph]);
        assert_eq!(output.glyph_infos()[0].glyph_id, expected);
        let table =
            read_fonts::tables::gsub::SingleSubstFormat3::read(FontData::new(&subtable)).unwrap();
        use super::apply::{WouldApply, WouldApplyContext};
        assert!(table.would_apply(&WouldApplyContext {
            glyphs: &[GlyphId::new(glyph)],
            zero_context: false,
        }));
    }
}

#[test]
fn single_subst4_keeps_wide_coverage_indices_and_replacements() {
    let mut subtable = vec![0, 4, 0, 0, 0, 12, 0, 0, 1, 2, 0, 1];
    subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
    let output = apply_subtable(1, true, &subtable, &[65536, 1]);
    assert_eq!(
        output
            .glyph_infos()
            .iter()
            .map(|info| info.glyph_id)
            .collect::<Vec<_>>(),
        [131_073, 1]
    );

    // The selected replacement is at index 65536, not index zero.
    let count = 65537;
    subtable = vec![0, 4];
    subtable.extend_from_slice(&(9 + count * 3u32).to_be_bytes());
    subtable.extend_from_slice(&Uint24::new(count).to_be_bytes());
    for replacement in 0..count {
        subtable.extend_from_slice(&Uint24::new(replacement + 1).to_be_bytes());
    }
    subtable.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0]);
    let output = apply_subtable(1, true, &subtable, &[65536]);
    assert_eq!(output.glyph_infos()[0].glyph_id, 65537);
}

#[test]
fn extension_lookup_applies_wide_single_substitution() {
    let mut subtable = vec![0, 1, 0, 1, 0, 0, 0, 8];
    subtable.extend_from_slice(&[0, 3, 0, 0, 0, 9, 0, 0, 1]);
    subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
    let output = apply_subtable(7, true, &subtable, &[65536]);
    assert_eq!(output.glyph_infos()[0].glyph_id, 65537);
}

#[test]
fn multiple_subst2_preserves_expansion_and_single_glyph_behavior() {
    for replacements in [&[65537, 131_073][..], &[131_073][..], &[][..]] {
        let mut subtable = vec![0, 2, 0, 0, 0, 12, 0, 0, 1, 0, 0, 20];
        subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
        subtable.extend_from_slice(&(replacements.len() as u16).to_be_bytes());
        for glyph in replacements {
            subtable.extend_from_slice(&Uint24::new(*glyph).to_be_bytes());
        }
        let output = apply_subtable(2, true, &subtable, &[65536, 1]);
        let mut expected = replacements.to_vec();
        expected.push(1);
        assert_eq!(
            output
                .glyph_infos()
                .iter()
                .map(|info| info.glyph_id)
                .collect::<Vec<_>>(),
            expected
        );
        if replacements.len() > 1 {
            assert!(output.glyph_infos()[0].multiplied());
            assert!(output.glyph_infos()[1].multiplied());
            assert_eq!(output.glyph_infos()[0].lig_comp(), 0);
            assert_eq!(output.glyph_infos()[1].lig_comp(), 1);
        } else if replacements.len() == 1 {
            assert!(!output.glyph_infos()[0].multiplied());
        }
    }
}

#[test]
fn multiple_subst2_resolves_large_sequence_offsets() {
    let mut subtable = vec![0, 2, 0, 0, 0, 12, 0, 0, 1, 1, 0, 0];
    subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
    subtable.resize(65536, 0);
    subtable.extend_from_slice(&[0, 1, 2, 0, 1]);
    let output = apply_subtable(2, true, &subtable, &[65536]);
    assert_eq!(output.glyph_infos()[0].glyph_id, 131_073);
}

#[test]
fn alternate_subst2_uses_feature_values_to_select_wide_glyphs() {
    let subtable = [
        0, 2, 0, 0, 0, 12, 0, 0, 1, 0, 0, 20, // Header and set offset.
        0, 3, 0, 0, 1, 1, 0, 0, // Coverage.
        0, 2, 1, 0, 1, 2, 0, 1, // Two alternates.
    ];
    for (value, expected) in [(0, 65536), (1, 65537), (2, 131_073), (3, 65536)] {
        let output = apply_subtable_configured(3, true, &subtable, &[65536], |ctx| {
            ctx.set_lookup_mask(3);
            ctx.buffer.glyph_infos_mut()[0].mask = value;
        });
        assert_eq!(output.glyph_infos()[0].glyph_id, expected);
    }

    let output = apply_subtable_configured(3, true, &subtable, &[65536], |ctx| {
        ctx.set_lookup_mask(map::OtMap::MAX_VALUE);
        ctx.buffer.glyph_infos_mut()[0].mask = map::OtMap::MAX_VALUE;
        ctx.random = true;
    });
    // The initial random state yields 48271, so choose the second alternate.
    assert_eq!(output.glyph_infos()[0].glyph_id, 131_073);
}

#[test]
fn alternate_subst2_resolves_large_set_offsets() {
    let mut subtable = vec![0, 2, 0, 0, 0, 12, 0, 0, 1, 1, 0, 0];
    subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
    subtable.resize(65536, 0);
    subtable.extend_from_slice(&[0, 1, 2, 0, 1]);
    let output = apply_subtable(3, true, &subtable, &[65536]);
    assert_eq!(output.glyph_infos()[0].glyph_id, 131_073);
}

#[test]
fn reverse_chain_subst2_matches_wide_backtrack_and_lookahead() {
    let subtable = [
        0, 2, 0, 0, 0, 22, // Header and coverage offset.
        0, 1, 0, 0, 30, // Backtrack coverage.
        0, 1, 0, 0, 38, // Lookahead coverage.
        0, 0, 1, 2, 0, 1, // One replacement.
        0, 3, 0, 0, 1, 1, 0, 0, // Input glyph.
        0, 3, 0, 0, 1, 1, 0, 1, // Backtrack glyph.
        0, 3, 0, 0, 1, 1, 0, 2, // Lookahead glyph.
    ];
    for (glyphs, expected) in [
        ([65537, 65536, 65538], [65537, 131_073, 65538]),
        ([1, 65536, 65538], [1, 65536, 65538]),
        ([65537, 65536, 1], [65537, 65536, 1]),
    ] {
        let output = apply_subtable(8, true, &subtable, &glyphs);
        assert_eq!(
            output
                .glyph_infos()
                .iter()
                .map(|info| info.glyph_id)
                .collect::<Vec<_>>(),
            expected
        );
    }

    let mut extension = vec![0, 1, 0, 8, 0, 0, 0, 8];
    extension.extend_from_slice(&subtable);
    let output = apply_subtable(7, true, &extension, &[65537, 65536, 65538]);
    assert_eq!(output.glyph_infos()[1].glyph_id, 131_073);
}

#[test]
fn reverse_chain_subst2_applies_in_reverse_order() {
    let subtable = [
        0, 2, 0, 0, 0, 19, // Header and coverage offset.
        0, 0, // No backtrack glyphs.
        0, 1, 0, 0, 19, // Lookahead uses the same coverage.
        0, 0, 1, 1, 0, 1, // One replacement.
        0, 3, 0, 0, 1, 1, 0, 0, // Coverage.
    ];
    let output = apply_subtable(8, true, &subtable, &[65536, 65536, 65536]);
    assert_eq!(
        output
            .glyph_infos()
            .iter()
            .map(|info| info.glyph_id)
            .collect::<Vec<_>>(),
        [65536, 65537, 65536]
    );
}

#[test]
fn ligature_subst2_matches_wide_components_and_results() {
    let subtable = [
        0, 2, 0, 0, 0, 12, 0, 0, 1, 0, 0, 20, // Header and set offset.
        0, 3, 0, 0, 1, 1, 0, 0, // Coverage.
        0, 2, 0, 0, 8, 0, 0, 16, // Two ligature offsets.
        1, 0, 1, 0, 2, 1, 0, 2, // First ligature.
        2, 0, 1, 0, 2, 1, 0, 3, // Second ligature.
    ];
    for (input, expected) in [
        ([65536, 65538], vec![65537]),
        ([65536, 65539], vec![131_073]),
        ([65536, 65540], vec![65536, 65540]),
    ] {
        let output = apply_subtable(4, true, &subtable, &input);
        assert_eq!(
            output
                .glyph_infos()
                .iter()
                .map(|info| info.glyph_id)
                .collect::<Vec<_>>(),
            expected
        );
    }
    use super::apply::{WouldApply, WouldApplyContext};
    let table =
        read_fonts::tables::gsub::LigatureSubstFormat2::read(FontData::new(&subtable)).unwrap();
    assert!(table.would_apply(&WouldApplyContext {
        glyphs: &[GlyphId::new(65536), GlyphId::new(65539)],
        zero_context: false,
    }));
    assert!(!table.would_apply(&WouldApplyContext {
        glyphs: &[GlyphId::new(65536), GlyphId::new(3)],
        zero_context: false,
    }));
}

#[test]
fn ligature_subst2_resolves_large_ligature_offsets() {
    let mut subtable = vec![0, 2, 0, 0, 0, 12, 0, 0, 1, 0, 0, 20];
    subtable.extend_from_slice(&[0, 3, 0, 0, 1, 1, 0, 0]);
    subtable.extend_from_slice(&[0, 1, 1, 0, 0]);
    subtable.resize(20 + 65536, 0);
    subtable.extend_from_slice(&[2, 0, 1, 0, 2, 1, 0, 2]);
    let output = apply_subtable(4, true, &subtable, &[65536, 65538]);
    assert_eq!(output.glyph_infos().len(), 1);
    assert_eq!(output.glyph_infos()[0].glyph_id, 131_073);
}

#[test]
fn ligature_subst2_keeps_wide_set_counts_and_coverage_indices() {
    let count = 65537;
    let coverage_offset = 9 + count * 3u32;
    let mut subtable = vec![0, 2];
    subtable.extend_from_slice(&coverage_offset.to_be_bytes());
    subtable.extend_from_slice(&Uint24::new(count).to_be_bytes());
    subtable.resize(coverage_offset as usize - 3, 0);
    subtable.extend_from_slice(&Uint24::new(coverage_offset + 14).to_be_bytes());
    subtable.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0]);
    subtable.extend_from_slice(&[0, 1, 0, 0, 5, 2, 0, 1, 0, 2, 1, 0, 2]);
    let output = apply_subtable(4, true, &subtable, &[65536, 65538]);
    assert_eq!(output.glyph_infos().len(), 1);
    assert_eq!(output.glyph_infos()[0].glyph_id, 131_073);
}

#[test]
fn single_pos3_applies_scaled_values_in_both_directions() {
    let subtable = [
        0, 3, 0, 0, 0, 16, 0, 15, // Header and value format.
        0, 10, 0xFF, 0xEC, 0, 30, 0xFF, 0xD8, // Placement and advances.
        0, 3, 0, 0, 1, 1, 0, 0, // Coverage.
    ];
    for (direction, expected) in [
        (Direction::LeftToRight, [20, -10, 60, 0]),
        (Direction::TopToBottom, [20, -10, 0, 20]),
    ] {
        let output = apply_subtable_configured(1, false, &subtable, &[65536, 1], |ctx| {
            ctx.buffer.set_direction(direction);
            ctx.scale = Scale::new(Some((2000, 500)), 1000);
        });
        let pos = &output.glyph_positions()[0];
        assert_eq!(
            [pos.x_offset, pos.y_offset, pos.x_advance, pos.y_advance],
            expected
        );
        let pos = &output.glyph_positions()[1];
        assert_eq!(
            [pos.x_offset, pos.y_offset, pos.x_advance, pos.y_advance],
            [0; 4]
        );
    }
}

#[test]
fn single_pos4_keeps_wide_record_counts_and_indices() {
    let count = 65537;
    let mut subtable = vec![0, 4];
    subtable.extend_from_slice(&(11 + count * 2u32).to_be_bytes());
    subtable.extend_from_slice(&4u16.to_be_bytes()); // X advance.
    subtable.extend_from_slice(&Uint24::new(count).to_be_bytes());
    subtable.resize(11 + (count as usize - 1) * 2, 0);
    subtable.extend_from_slice(&(-10i16).to_be_bytes());
    subtable.extend_from_slice(&[0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0]);
    let output = apply_subtable(1, false, &subtable, &[65536]);
    assert_eq!(output.glyph_positions()[0].x_advance, -10);

    let mut extension = vec![0, 1, 0, 1, 0, 0, 0, 8];
    extension.extend_from_slice(&subtable);
    let output = apply_subtable(9, false, &extension, &[65536]);
    assert_eq!(output.glyph_positions()[0].x_advance, -10);
}

#[test]
fn single_pos4_rejects_coverage_indices_outside_the_value_array() {
    let subtable = [
        0, 4, 0, 0, 0, 13, 0, 4, 0, 0, 1, 0, 20, // Header and one value.
        0, 4, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 1, // Index 1 is out of bounds.
    ];
    let output = apply_subtable(1, false, &subtable, &[65536]);
    assert_eq!(output.glyph_positions()[0].x_advance, 0);
}

#[test]
fn cursive_pos2_attaches_wide_glyphs_in_all_directions() {
    let subtable = [
        0, 2, 0, 0, 0, 21, 0, 0, 2, // Header and two records.
        0, 0, 0, 0, 0, 32, // First glyph's exit anchor.
        0, 0, 38, 0, 0, 0, // Second glyph's entry anchor.
        0, 3, 0, 0, 2, 1, 0, 0, 1, 0, 1, // Coverage.
        0, 1, 0, 100, 0, 50, // Exit anchor.
        0, 1, 0, 20, 0, 10, // Entry anchor.
    ];
    for (direction, expected) in [
        (Direction::LeftToRight, [[0, 0, 100, 0], [-20, 40, -20, 0]]),
        (Direction::RightToLeft, [[-100, 0, -100, 0], [0, 40, 20, 0]]),
        (Direction::TopToBottom, [[0, 0, 0, 50], [80, -10, 0, -10]]),
        (Direction::BottomToTop, [[0, -50, 0, -50], [80, 0, 0, 10]]),
    ] {
        let output = apply_subtable_configured(3, false, &subtable, &[65536, 65537], |ctx| {
            ctx.buffer.set_direction(direction);
        });
        for (pos, expected) in output.glyph_positions().iter().zip(expected) {
            assert_eq!(
                [pos.x_offset, pos.y_offset, pos.x_advance, pos.y_advance],
                expected
            );
        }
        assert_eq!(output.glyph_positions()[1].attach_chain(), -1);
        assert_eq!(
            output.glyph_positions()[1].attach_type(),
            gpos::attach_type::CURSIVE
        );
    }

    let output = apply_subtable_configured(3, false, &subtable, &[65536, 65537], |ctx| {
        ctx.lookup_props |= u32::from(lookup_flags::RIGHT_TO_LEFT);
    });
    assert_eq!(output.glyph_positions()[0].attach_chain(), 1);
    assert_eq!(output.glyph_positions()[0].y_offset, -40);
}

#[test]
fn cursive_pos2_resolves_large_and_nullable_anchor_offsets() {
    let mut subtable = vec![
        0, 2, 0, 0, 0, 21, 0, 0, 2, // Header and two records.
        0, 0, 0, 1, 0, 0, // First glyph's exit anchor at 65536.
        1, 0, 6, 0, 0, 0, // Second glyph's entry anchor at 65542.
        0, 3, 0, 0, 2, 1, 0, 0, 1, 0, 1, // Coverage.
    ];
    subtable.resize(65536, 0);
    subtable.extend_from_slice(&[0, 1, 0, 100, 0, 50, 0, 1, 0, 20, 0, 10]);
    let output = apply_subtable(3, false, &subtable, &[65536, 65537]);
    assert_eq!(output.glyph_positions()[0].x_advance, 100);
    assert_eq!(output.glyph_positions()[1].y_offset, 40);

    subtable[15..18].fill(0);
    let output = apply_subtable(3, false, &subtable, &[65536, 65537]);
    assert_eq!(output.glyph_positions()[0].x_advance, 0);
    assert_eq!(output.glyph_positions()[1].attach_chain(), 0);
}

#[test]
fn pair_class_indices_cannot_alias_another_matrix_row() {
    for class in [2, 65536] {
        let mut subtable = vec![
            0, 2, 0, 24, 0, 4, 0, 0, 0, 32, 0, 43, 0, 2, 0, 2, // Header.
            0, 0, 0, 0, 0, 100, 0, 0, // A nonzero value in the next row.
            0, 3, 0, 0, 1, 1, 0, 0, // Coverage.
            0, 3, 1, 0, 0, 0, 0, 1, 0, 0, 0, // First class is zero.
            0, 3, 1, 0, 1, 0, 0, 1, // Second class is outside the matrix.
        ];
        subtable.extend_from_slice(&Uint24::new(class).to_be_bytes());
        let output = apply_subtable(2, false, &subtable, &[65536, 65537]);
        assert_eq!(output.glyph_positions()[0].x_advance, 0);
    }
}

fn class_pair_pos4(large_offsets: bool) -> Vec<u8> {
    let coverage_offset = if large_offsets { 65536u32 } else { 38 };
    let mut subtable = vec![0, 4];
    subtable.extend_from_slice(&coverage_offset.to_be_bytes());
    subtable.extend_from_slice(&[0, 4, 0, 2]); // X advance and Y placement.
    subtable.extend_from_slice(&(coverage_offset + 11).to_be_bytes());
    subtable.extend_from_slice(&(coverage_offset + 25).to_be_bytes());
    subtable.extend_from_slice(&[0, 2, 0, 2]); // Class counts remain 16-bit.
    subtable.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 0xFF, 0xEC]);
    subtable.resize(coverage_offset as usize, 0);
    subtable.extend_from_slice(&[0, 3, 0, 0, 2, 1, 0, 0, 1, 0, 1]);
    subtable.extend_from_slice(&[0, 3, 1, 0, 0, 0, 0, 2, 0, 0, 1, 0, 0, 0]);
    subtable.extend_from_slice(&[0, 3, 1, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 1]);
    subtable
}

#[test]
fn pair_pos4_applies_wide_class_pairs_with_both_cache_sizes() {
    use super::apply::{Apply, SubtableExternalCache, SubtableExternalCacheMode};
    for large_offsets in [false, true] {
        let subtable = class_pair_pos4(large_offsets);
        let output = apply_subtable(2, false, &subtable, &[65536, 65537]);
        assert_eq!(output.glyph_positions()[0].x_advance, 100);
        assert_eq!(output.glyph_positions()[1].y_offset, -20);

        let table =
            read_fonts::tables::gpos::PairPosFormat4::read(FontData::new(&subtable)).unwrap();
        let SubtableExternalCache::PairPosFormat2SmallCache(cache) =
            table.external_cache_create(SubtableExternalCacheMode::Small)
        else {
            panic!("missing small class-pair cache");
        };
        let data = table.offset_data();
        assert_eq!(cache.coverage.index(&data, GlyphId::new(65536)), Some(0));
        assert_eq!(cache.first.class(&data, GlyphId::new(65536)), 1);
        assert_eq!(cache.second.class(&data, GlyphId::new(65537)), 1);
    }
}

#[test]
fn pair_pos4_does_not_truncate_wide_classes_to_matrix_indices() {
    let mut subtable = class_pair_pos4(false);
    subtable[57..60].copy_from_slice(&Uint24::new(65537).to_be_bytes());
    let output = apply_subtable(2, false, &subtable, &[65536, 65537]);
    assert_eq!(output.glyph_positions()[0].x_advance, 0);
    assert_eq!(output.glyph_positions()[1].y_offset, 0);
}
