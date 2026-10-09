use alloc::vec::Vec;
use read_fonts::tables::layout::{FeatureLookupsFlags, FeatureVariations};
use read_fonts::tables::variations::{DeltaSetIndex, ItemVariationStore};
use read_fonts::types::F2Dot14;
use read_fonts::TableProvider;

use super::layout::LayoutTableKind;
use super::LayoutTable;
use crate::{Font, U32Set};

#[cfg(test)]
#[path = "tests/lookup_variations.rs"]
mod tests;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct FeatureVariationState {
    pub indices: [Option<u32>; 2],
    lookups: [Vec<FeatureLookupState>; 2],
}

#[derive(Clone, PartialEq, Eq)]
struct FeatureLookupState {
    feature_index: u16,
    lookups: Vec<u16>,
}

impl FeatureVariationState {
    pub const EMPTY: Self = Self {
        indices: [None; 2],
        lookups: [Vec::new(), Vec::new()],
    };

    pub fn lookup_indices(&self, table: LayoutTableKind, feature: u16) -> Option<&[u16]> {
        let states = &self.lookups[table];
        let index = states
            .binary_search_by_key(&feature, |state| state.feature_index)
            .ok()?;
        Some(&states[index].lookups)
    }
}

pub(crate) fn variation_state(font: &Font) -> FeatureVariationState {
    let tables = font.tables();
    let layout_tables = [
        tables.gsub().ok().map(LayoutTable::Gsub),
        tables.gpos().ok().map(LayoutTable::Gpos),
    ];
    let variations = layout_tables
        .each_ref()
        .map(|table| table.as_ref().and_then(LayoutTable::feature_variations));
    if variations.iter().all(Option::is_none) {
        return FeatureVariationState::EMPTY;
    }
    let store = tables
        .gdef()
        .ok()
        .and_then(|gdef| gdef.item_var_store().transpose().ok().flatten());
    let coords = font.normalized_coords();
    let indices = variations.each_ref().map(|variations| {
        variations
            .as_ref()?
            .index_for_coords_with_delta(coords, |index| delta(store.as_ref(), coords, index))
    });
    let lookups = core::array::from_fn(|index| match (&layout_tables[index], &variations[index]) {
        (Some(table), Some(variations)) => {
            resolve_lookups(table, variations, indices[index], coords, store.as_ref())
        }
        _ => Vec::new(),
    });
    FeatureVariationState { indices, lookups }
}

fn resolve_lookups(
    table: &LayoutTable,
    variations: &FeatureVariations,
    variation_index: Option<u32>,
    coords: &[F2Dot14],
    store: Option<&ItemVariationStore>,
) -> Vec<FeatureLookupState> {
    let Some(records) = variations.lookup_variation_records() else {
        return Vec::new();
    };
    if variations.version().major != 1
        || records
            .windows(2)
            .any(|records| records[0].feature_index() >= records[1].feature_index())
    {
        return Vec::new();
    }
    records
        .iter()
        .filter_map(|record| {
            let feature_index = record.feature_index();
            let feature = variation_index
                .and_then(|index| table.feature_substitution(index, feature_index))
                .or_else(|| table.feature(feature_index))?;
            if record.feature_lookups_offset().to_u32() == 0 {
                return None;
            }
            let conditional = record.feature_lookups(variations.offset_data()).ok()?;
            let conditions = conditional.lookup_condition_records();
            if conditional.version().major != 1
                || conditions.len() != conditional.lookup_condition_count() as usize
            {
                return None;
            }
            let mut lookups = U32Set::default();
            if conditional
                .flags()
                .contains(FeatureLookupsFlags::ADD_DEFAULT_LOOKUPS)
            {
                // "Default" means the current feature, including an alternate
                // selected by the older FeatureVariationRecord mechanism.
                lookups.extend(
                    feature
                        .lookup_list_indices()
                        .iter()
                        .map(|index| u32::from(index.get())),
                );
            }
            for condition in conditions {
                if condition
                    .evaluate(conditional.offset_data(), coords, |index| {
                        delta(store, coords, index)
                    })
                    .ok()
                    != Some(true)
                {
                    continue;
                }
                if condition.lookup_index_list_offset().to_u32() == 0 {
                    continue;
                }
                let Ok(indices) = condition.lookup_index_list(conditional.offset_data()) else {
                    continue;
                };
                if indices.lookup_indices().len() != usize::from(indices.lookup_index_count()) {
                    continue;
                }
                lookups.extend(
                    indices
                        .lookup_indices()
                        .iter()
                        .map(|index| u32::from(index.get())),
                );
            }
            Some(FeatureLookupState {
                feature_index,
                lookups: lookups.iter().map(|index| index as u16).collect(),
            })
        })
        .collect()
}

fn delta(
    store: Option<&ItemVariationStore>,
    coords: &[F2Dot14],
    index: u32,
) -> Result<f64, core::convert::Infallible> {
    Ok(store
        .and_then(|store| {
            store.compute_delta(
                DeltaSetIndex {
                    outer: (index >> 16) as u16,
                    inner: index as u16,
                },
                coords,
            )
        })
        .unwrap_or_default()
        .to_f64())
}
