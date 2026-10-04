use read_fonts::tables::variations::{DeltaSetIndex, ItemVariationStore};
use read_fonts::types::F2Dot14;
use read_fonts::TableProvider;

use crate::Font;

pub(crate) fn feature_variation_indices(font: &Font) -> [Option<u32>; 2] {
    let tables = font.tables();
    let variations = [
        tables
            .gsub()
            .ok()
            .and_then(|table| table.feature_variations().transpose().ok().flatten()),
        tables
            .gpos()
            .ok()
            .and_then(|table| table.feature_variations().transpose().ok().flatten()),
    ];
    if variations.iter().all(Option::is_none) {
        return [None; 2];
    }
    let store = tables
        .gdef()
        .ok()
        .and_then(|gdef| gdef.item_var_store().transpose().ok().flatten());
    let coords = font.normalized_coords();
    variations.map(|variations| {
        variations?
            .index_for_coords_with_delta(coords, |index| delta(store.as_ref(), coords, index))
    })
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
