use super::*;
use alloc::vec;

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
