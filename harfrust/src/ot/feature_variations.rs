use read_fonts::tables::layout::{Condition, FeatureVariations};
use read_fonts::tables::variations::{DeltaSetIndex, ItemVariationStore};
use read_fonts::types::F2Dot14;
use read_fonts::{FontData, FontRead, TableProvider};

use super::layout::MAX_NESTING_LEVEL;
use crate::Font;

pub(crate) fn feature_variation_indices(font: &Font) -> [Option<u32>; 2] {
    let tables = font.tables();
    let gsub = tables.gsub().ok();
    let gpos = tables.gpos().ok();
    let variations = [
        gsub.and_then(|table| table.feature_variations().transpose().ok().flatten()),
        gpos.and_then(|table| table.feature_variations().transpose().ok().flatten()),
    ];
    if variations.iter().all(Option::is_none) {
        return [None; 2];
    }
    let store = tables
        .gdef()
        .ok()
        .and_then(|gdef| gdef.item_var_store().transpose().ok().flatten());
    variations.map(|variations| find_index(&variations?, font.normalized_coords(), store.as_ref()))
}

fn find_index(
    variations: &FeatureVariations,
    coords: &[F2Dot14],
    store: Option<&ItemVariationStore>,
) -> Option<u32> {
    if variations.version().major != 1 {
        return None;
    }
    variations
        .feature_variation_records()
        .iter()
        .position(|record| {
            if record.condition_set_offset().is_null() {
                return true;
            }
            let Some(Ok(set)) = record.condition_set(variations.offset_data()) else {
                return false;
            };
            let offsets = set.condition_offsets();
            if offsets.len() != usize::from(set.condition_count()) {
                return false;
            }
            let mut evaluator = ConditionEvaluator::new(coords, store);
            offsets.iter().all(|offset| {
                evaluator.evaluate_offset(
                    set.offset_data(),
                    offset.get().to_u32(),
                    MAX_NESTING_LEVEL,
                ) == Some(true)
            })
        })
        .map(|index| index as u32)
}

struct ConditionEvaluator<'a, 'f> {
    coords: &'a [F2Dot14],
    store: Option<&'a ItemVariationStore<'f>>,
    remaining_ops: usize,
}

impl<'a, 'f> ConditionEvaluator<'a, 'f> {
    fn new(coords: &'a [F2Dot14], store: Option<&'a ItemVariationStore<'f>>) -> Self {
        Self {
            coords,
            store,
            remaining_ops: 65536,
        }
    }

    fn evaluate_offset(&mut self, data: FontData, offset: u32, depth: usize) -> Option<bool> {
        if offset == 0 {
            return Some(true);
        }
        self.evaluate(
            &Condition::read(data.split_off(offset as usize)?).ok()?,
            depth,
        )
    }

    fn evaluate(&mut self, condition: &Condition, depth: usize) -> Option<bool> {
        // Bound both recursion and repeated visits: many offsets can refer to
        // the same child, making even a shallow condition tree expensive.
        if depth == 0 || self.remaining_ops == 0 {
            return None;
        }
        self.remaining_ops -= 1;
        Some(match condition {
            Condition::Format1AxisRange(condition) => {
                let coord = self
                    .coords
                    .get(usize::from(condition.axis_index()))
                    .copied()
                    .unwrap_or_default();
                coord >= condition.filter_range_min_value()
                    && coord <= condition.filter_range_max_value()
            }
            Condition::Format2VariableValue(condition) => {
                let index = condition.var_index();
                // Match the other layout variation evaluators: absent
                // coordinates use the default value without blending deltas.
                let delta = if self.coords.is_empty() || index == u32::MAX {
                    0.0
                } else {
                    self.store
                        .and_then(|store| {
                            store.compute_delta(
                                DeltaSetIndex {
                                    outer: (index >> 16) as u16,
                                    inner: index as u16,
                                },
                                self.coords,
                            )
                        })
                        .unwrap_or_default()
                        .to_f64()
                };
                f64::from(condition.default_value()) + delta > 0.0
            }
            Condition::Format3And(condition) => {
                let offsets = condition.condition_offsets();
                if offsets.len() != usize::from(condition.condition_count()) {
                    return None;
                }
                for offset in offsets {
                    if !self.evaluate_offset(
                        condition.offset_data(),
                        offset.get().to_u32(),
                        depth - 1,
                    )? {
                        return Some(false);
                    }
                }
                true
            }
            Condition::Format4Or(condition) => {
                let offsets = condition.condition_offsets();
                if offsets.len() != usize::from(condition.condition_count()) {
                    return None;
                }
                for offset in offsets {
                    if self.evaluate_offset(
                        condition.offset_data(),
                        offset.get().to_u32(),
                        depth - 1,
                    )? {
                        return Some(true);
                    }
                }
                false
            }
            Condition::Format5Negate(condition) => !self.evaluate_offset(
                condition.offset_data(),
                condition.condition_offset().to_u32(),
                depth - 1,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn evaluate(bytes: &[u8], coords: &[F2Dot14]) -> Option<bool> {
        ConditionEvaluator::new(coords, None).evaluate(
            &Condition::read(FontData::new(bytes)).ok()?,
            MAX_NESTING_LEVEL,
        )
    }

    fn axis_range(axis: u16, min: f32, max: f32) -> Vec<u8> {
        let mut bytes = vec![0, 1];
        bytes.extend_from_slice(&axis.to_be_bytes());
        bytes.extend_from_slice(&F2Dot14::from_f32(min).to_be_bytes());
        bytes.extend_from_slice(&F2Dot14::from_f32(max).to_be_bytes());
        bytes
    }

    #[test]
    fn axis_ranges_include_the_endpoints_and_missing_axes_are_zero() {
        let condition = axis_range(0, 0.25, 0.75);
        for (coord, matches) in [
            (0.0, false),
            (0.25, true),
            (0.5, true),
            (0.75, true),
            (1.0, false),
        ] {
            assert_eq!(
                evaluate(&condition, &[F2Dot14::from_f32(coord)]),
                Some(matches)
            );
        }
        assert_eq!(evaluate(&condition, &[]), Some(false));
        assert_eq!(evaluate(&axis_range(5, 0.0, 0.0), &[]), Some(true));
    }

    #[test]
    fn variable_values_keep_fractional_deltas_and_use_a_strict_positive_test() {
        let bytes = [
            0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22, // ItemVariationStore.
            0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0, // One axis and one region.
            0, 1, 0, 1, 0, 1, 0, 0, 0, 1, // One delta of 1.
        ];
        let store = ItemVariationStore::read(FontData::new(&bytes)).unwrap();
        let condition = Condition::read(FontData::new(&[0, 2, 0, 0, 0, 0, 0, 0])).unwrap();
        for (coord, matches) in [(-1.0, false), (0.0, false), (0.5, true), (1.0, true)] {
            let coords = [F2Dot14::from_f32(coord)];
            assert_eq!(
                ConditionEvaluator::new(&coords, Some(&store))
                    .evaluate(&condition, MAX_NESTING_LEVEL),
                Some(matches)
            );
        }
        for default_value in [-1i16, 0, 1] {
            let mut bytes = vec![0, 2];
            bytes.extend_from_slice(&default_value.to_be_bytes());
            bytes.extend_from_slice(&u32::MAX.to_be_bytes());
            assert_eq!(evaluate(&bytes, &[]), Some(default_value > 0));
        }
    }

    #[test]
    fn compound_conditions_and_null_children() {
        let mut condition = vec![0, 3, 2, 0, 0, 9, 0, 0, 17];
        condition.extend_from_slice(&axis_range(0, 0.25, 0.75));
        condition.extend_from_slice(&axis_range(1, -1.0, -0.25));
        assert_eq!(
            evaluate(
                &condition,
                &[F2Dot14::from_f32(0.5), F2Dot14::from_f32(-0.5)]
            ),
            Some(true)
        );
        assert_eq!(evaluate(&condition, &[F2Dot14::from_f32(0.5)]), Some(false));
        condition[1] = 4;
        assert_eq!(evaluate(&condition, &[F2Dot14::from_f32(0.5)]), Some(true));
        assert_eq!(evaluate(&condition, &[]), Some(false));

        let mut negated = vec![0, 5, 0, 0, 5];
        negated.extend_from_slice(&condition);
        assert_eq!(evaluate(&negated, &[]), Some(true));
        assert_eq!(evaluate(&negated, &[F2Dot14::from_f32(0.5)]), Some(false));
        assert_eq!(evaluate(&[0, 3, 1, 0, 0, 0], &[]), Some(true));
        assert_eq!(evaluate(&[0, 4, 1, 0, 0, 0], &[]), Some(true));
        assert_eq!(evaluate(&[0, 5, 0, 0, 0], &[]), Some(false));
        assert_eq!(evaluate(&[0, 3, 0], &[]), Some(true));
        assert_eq!(evaluate(&[0, 4, 0], &[]), Some(false));
    }

    #[test]
    fn malformed_and_expensive_conditions_fail_closed() {
        assert_eq!(evaluate(&[0, 3, 1], &[]), None);
        assert_eq!(evaluate(&[0, 5, 0xFF, 0xFF, 0xFF], &[]), None);
        let mut condition = vec![0, 5, 0, 0, 5];
        condition.extend_from_slice(&[0, 99]);
        assert_eq!(evaluate(&condition, &[]), None);
        let mut deep = Vec::new();
        for _ in 0..MAX_NESTING_LEVEL {
            deep.extend_from_slice(&[0, 5, 0, 0, 5]);
        }
        deep.extend_from_slice(&axis_range(0, 0.0, 0.0));
        assert_eq!(evaluate(&deep, &[]), None);
        let mut repeated = Vec::new();
        for _ in 0..17 {
            repeated.extend_from_slice(&[0, 3, 2, 0, 0, 9, 0, 0, 9]);
        }
        repeated.extend_from_slice(&axis_range(0, 0.0, 0.0));
        assert_eq!(evaluate(&repeated, &[]), None);
    }

    #[test]
    fn feature_variations_select_the_first_matching_record_at_default_too() {
        let mut bytes = vec![0, 1, 0, 0, 0, 0, 0, 2];
        bytes.extend_from_slice(&[0, 0, 0, 24, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 1, 0, 0, 0, 6]);
        bytes.extend_from_slice(&[0, 5, 0, 0, 5]);
        bytes.extend_from_slice(&axis_range(0, 0.25, 0.75));
        let variations = FeatureVariations::read(FontData::new(&bytes)).unwrap();
        assert_eq!(find_index(&variations, &[], None), Some(0));
        assert_eq!(
            find_index(&variations, &[F2Dot14::from_f32(0.5)], None),
            Some(1)
        );
    }
}
