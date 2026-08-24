use super::{
    buffer::hb_buffer_t,
    font_funcs::FontFuncsDispatch,
    hb_mask_t, hb_tag_t,
    ot::lookup::LookupInfo,
    ot_layout::{apply_synthesized_subst_lookup, TableIndex},
    ot_layout_common::lookup_flags,
    ot_layout_gsubgpos::OT::hb_ot_apply_context_t,
    ot_shape_plan::hb_ot_shape_plan_t,
    ot_shaper_arabic_table::{
        LIGATURE_3_TABLE, LIGATURE_MARK_TABLE, LIGATURE_TABLE, SHAPING_TABLE,
    },
};
use alloc::vec::Vec;

const FALLBACK_FEATURES: [hb_tag_t; 7] = [
    hb_tag_t::new(b"init"),
    hb_tag_t::new(b"medi"),
    hb_tag_t::new(b"fina"),
    hb_tag_t::new(b"isol"),
    hb_tag_t::new(b"rlig"),
    hb_tag_t::new(b"rlig"),
    hb_tag_t::new(b"rlig"),
];

pub(crate) struct FallbackPlan {
    lookups: Vec<FallbackLookup>,
}

struct FallbackLookup {
    mask: hb_mask_t,
    data: Vec<u8>,
    info: LookupInfo,
}

impl FallbackLookup {
    fn new(mask: hb_mask_t, data: Vec<u8>) -> Option<Self> {
        let info = LookupInfo::new_subst(&data)?;
        Some(Self { mask, data, info })
    }
}

impl FallbackPlan {
    pub(crate) fn new(
        plan: &hb_ot_shape_plan_t,
        font_funcs: &mut FontFuncsDispatch,
    ) -> Option<Self> {
        let mut lookups = Vec::with_capacity(FALLBACK_FEATURES.len());

        for (feature_index, feature) in FALLBACK_FEATURES.iter().enumerate() {
            let mask = plan.ot_map.get_1_mask(*feature);
            if mask == 0 {
                continue;
            }

            let lookup = match feature_index {
                0..=3 => synthesize_single_lookup(font_funcs, feature_index, mask),
                4 => synthesize_ligature_lookup(
                    font_funcs,
                    LIGATURE_3_TABLE,
                    lookup_flags::IGNORE_MARKS,
                    mask,
                ),
                5 => synthesize_ligature_lookup(
                    font_funcs,
                    LIGATURE_TABLE,
                    lookup_flags::IGNORE_MARKS,
                    mask,
                ),
                6 => synthesize_ligature_lookup(font_funcs, LIGATURE_MARK_TABLE, 0, mask),
                _ => unreachable!(),
            };
            if let Some(lookup) = lookup {
                lookups.push(lookup);
            }
        }

        (!lookups.is_empty()).then_some(Self { lookups })
    }

    pub(crate) fn apply(&self, font_funcs: &FontFuncsDispatch, buffer: &mut hb_buffer_t) {
        let mut ctx = hb_ot_apply_context_t::new(
            TableIndex::GSUB,
            font_funcs.font(),
            *font_funcs.scale(),
            buffer,
        );
        for lookup in &self.lookups {
            ctx.set_lookup_mask(lookup.mask);
            apply_synthesized_subst_lookup(&mut ctx, &lookup.info, &lookup.data);
        }
    }
}

fn glyph_for_codepoint(font_funcs: &mut FontFuncsDispatch, codepoint: u16) -> Option<u16> {
    let glyph = font_funcs.nominal_glyph(u32::from(codepoint))?;
    u16::try_from(glyph.to_u32()).ok()
}

fn synthesize_single_lookup(
    font_funcs: &mut FontFuncsDispatch,
    feature_index: usize,
    mask: hb_mask_t,
) -> Option<FallbackLookup> {
    let mut mappings = Vec::with_capacity(SHAPING_TABLE.len());
    for &(codepoint, forms) in SHAPING_TABLE {
        let form = forms[feature_index];
        if form == 0 {
            continue;
        }

        let Some(glyph) = glyph_for_codepoint(font_funcs, codepoint) else {
            continue;
        };
        let Some(substitute) = glyph_for_codepoint(font_funcs, form) else {
            continue;
        };
        if glyph != substitute {
            mappings.push((glyph, substitute));
        }
    }
    if mappings.is_empty() {
        return None;
    }

    mappings.sort_by_key(|(glyph, _)| *glyph);
    FallbackLookup::new(
        mask,
        serialize_single_lookup(&mappings, lookup_flags::IGNORE_MARKS)?,
    )
}

fn synthesize_ligature_lookup<const N: usize>(
    font_funcs: &mut FontFuncsDispatch,
    rules: &[([u16; N], u16)],
    flags: u16,
    mask: hb_mask_t,
) -> Option<FallbackLookup> {
    let mut ligatures = Vec::with_capacity(rules.len());
    for &(components, ligature) in rules {
        let mut component_glyphs = [0; N];
        let mut matched = true;
        for (glyph, &codepoint) in component_glyphs.iter_mut().zip(components.iter()) {
            let Some(component) = glyph_for_codepoint(font_funcs, codepoint) else {
                matched = false;
                break;
            };
            *glyph = component;
        }
        if !matched {
            continue;
        }
        let Some(ligature) = glyph_for_codepoint(font_funcs, ligature) else {
            continue;
        };
        ligatures.push((component_glyphs, ligature));
    }
    if ligatures.is_empty() {
        return None;
    }

    ligatures.sort_by_key(|(components, _)| components[0]);
    FallbackLookup::new(mask, serialize_ligature_lookup(&ligatures, flags)?)
}

fn write_u16(data: &mut Vec<u8>, value: u16) {
    data.extend_from_slice(&value.to_be_bytes());
}

fn patch_u16(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn serialize_single_lookup(mappings: &[(u16, u16)], flags: u16) -> Option<Vec<u8>> {
    let count = u16::try_from(mappings.len()).ok()?;
    let coverage_offset =
        u16::try_from(6usize.checked_add(mappings.len().checked_mul(2)?)?).ok()?;
    let mut data = Vec::with_capacity(12 + mappings.len() * 4);

    // Lookup table: type 1 (single substitution), one subtable at offset 8.
    write_u16(&mut data, 1);
    write_u16(&mut data, flags);
    write_u16(&mut data, 1);
    write_u16(&mut data, 8);

    // SingleSubstFormat2 and its coverage table.
    write_u16(&mut data, 2);
    write_u16(&mut data, coverage_offset);
    write_u16(&mut data, count);
    for &(_, substitute) in mappings {
        write_u16(&mut data, substitute);
    }
    write_u16(&mut data, 1);
    write_u16(&mut data, count);
    for &(glyph, _) in mappings {
        write_u16(&mut data, glyph);
    }

    Some(data)
}

fn serialize_ligature_lookup<const N: usize>(
    ligatures: &[([u16; N], u16)],
    flags: u16,
) -> Option<Vec<u8>> {
    const LOOKUP_SIZE: usize = 8;

    let mut set_starts = Vec::new();
    for (index, (components, _)) in ligatures.iter().enumerate() {
        if index == 0 || components[0] != ligatures[index - 1].0[0] {
            set_starts.push(index);
        }
    }
    let set_count = u16::try_from(set_starts.len()).ok()?;
    let component_count = u16::try_from(N).ok()?;
    let mut data = Vec::new();

    // Lookup table: type 4 (ligature substitution), one subtable at offset 8.
    write_u16(&mut data, 4);
    write_u16(&mut data, flags);
    write_u16(&mut data, 1);
    write_u16(&mut data, LOOKUP_SIZE as u16);

    // LigatureSubstFormat1 header. Fill offsets once their targets are written.
    write_u16(&mut data, 1);
    let coverage_offset_position = data.len();
    write_u16(&mut data, 0);
    write_u16(&mut data, set_count);
    let set_offsets_position = data.len();
    for _ in &set_starts {
        write_u16(&mut data, 0);
    }

    let coverage_offset = u16::try_from(data.len().checked_sub(LOOKUP_SIZE)?).ok()?;
    patch_u16(&mut data, coverage_offset_position, coverage_offset);
    write_u16(&mut data, 1);
    write_u16(&mut data, set_count);
    for &set_start in &set_starts {
        write_u16(&mut data, ligatures[set_start].0[0]);
    }

    for (set_index, &set_start) in set_starts.iter().enumerate() {
        let set_end = set_starts
            .get(set_index + 1)
            .copied()
            .unwrap_or(ligatures.len());
        let set_start_in_data = data.len();
        let set_offset = u16::try_from(set_start_in_data.checked_sub(LOOKUP_SIZE)?).ok()?;
        patch_u16(&mut data, set_offsets_position + set_index * 2, set_offset);

        let ligature_count = u16::try_from(set_end.checked_sub(set_start)?).ok()?;
        write_u16(&mut data, ligature_count);
        let ligature_offsets_position = data.len();
        for _ in set_start..set_end {
            write_u16(&mut data, 0);
        }

        for (ligature_index, &(components, ligature)) in
            ligatures[set_start..set_end].iter().enumerate()
        {
            let ligature_offset = u16::try_from(data.len().checked_sub(set_start_in_data)?).ok()?;
            patch_u16(
                &mut data,
                ligature_offsets_position + ligature_index * 2,
                ligature_offset,
            );
            write_u16(&mut data, ligature);
            write_u16(&mut data, component_count);
            for component in &components[1..] {
                write_u16(&mut data, *component);
            }
        }
    }

    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_single_substitution_lookup() {
        let data = serialize_single_lookup(&[(3, 8), (7, 9)], lookup_flags::IGNORE_MARKS).unwrap();
        assert_eq!(
            data,
            [
                0, 1, 0, 8, 0, 1, 0, 8, // Lookup
                0, 2, 0, 10, 0, 2, 0, 8, 0, 9, // SingleSubstFormat2
                0, 1, 0, 2, 0, 3, 0, 7, // CoverageFormat1
            ]
        );
        assert!(LookupInfo::new_subst(&data).is_some());
    }

    #[test]
    fn serializes_ligature_substitution_lookup() {
        let data = serialize_ligature_lookup(
            &[([3, 4], 9), ([3, 5], 10), ([7, 8], 11)],
            lookup_flags::IGNORE_MARKS,
        )
        .unwrap();
        assert!(LookupInfo::new_subst(&data).is_some());
    }
}
