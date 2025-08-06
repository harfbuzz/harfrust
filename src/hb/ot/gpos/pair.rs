use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{skipping_iterator_t, Apply, ApplyState};
use read_fonts::tables::gpos::{PairPosFormat1, PairPosFormat2, PairValueRecord};
use read_fonts::types::GlyphId;
use read_fonts::FontData;

use super::Value;

// TODO: HarfBuzz uses two class caches, for left and right, as well as coverage.

impl Apply for PairPosFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, state: &ApplyState) -> Option<()> {
        let first_glyph_coverage_index = state.index;

        let mut iter = skipping_iterator_t::new(ctx, false);
        iter.reset(ctx.buffer.idx);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
            return None;
        }

        let second_glyph_index = iter.index();
        let second_glyph = ctx.buffer.info[second_glyph_index].as_glyph();

        let finish = |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, has_record2| {
            if has_record2 {
                *iter_index += 1;
                // https://github.com/harfbuzz/harfbuzz/issues/3824
                // https://github.com/harfbuzz/harfbuzz/issues/3888#issuecomment-1326781116
                ctx.buffer
                    .unsafe_to_break(Some(ctx.buffer.idx), Some(*iter_index + 1));
            }

            ctx.buffer.idx = *iter_index;

            Some(())
        };

        let boring = |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, has_record2| {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_glyph_index + 1));
            finish(ctx, iter_index, has_record2)
        };

        let success =
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, flag1, flag2, has_record2| {
                if flag1 || flag2 {
                    ctx.buffer
                        .unsafe_to_break(Some(ctx.buffer.idx), Some(second_glyph_index + 1));
                    finish(ctx, iter_index, has_record2)
                } else {
                    boring(ctx, iter_index, has_record2)
                }
            };

        let bail =
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, records: (Value, Value)| {
                let flag1 = records.0.apply(ctx, ctx.buffer.idx);
                let flag2 = records.1.apply(ctx, second_glyph_index);

                let has_record2 = !records.1.is_empty();
                success(ctx, iter_index, flag1, flag2, has_record2)
            };

        let (pair, data) =
            find_second_glyph(self, first_glyph_coverage_index as usize, second_glyph)?;
        // let sets = self.pair_sets();
        // let data = sets.offset_data();
        // let sets = self.pair_sets();
        // let pair_sets = sets.get(first_glyph_coverage_index as usize).ok()?;
        // let pair = pair_sets
        //     .pair_value_records()
        //     .iter()
        //     .filter_map(|value| value.ok())
        //     .find(|value| value.second_glyph() == second_glyph)?;
        let values = (
            Value {
                record: pair.value_record1,
                data,
            },
            Value {
                record: pair.value_record2,
                data,
            },
        );
        bail(ctx, &mut iter.buf_idx, values)
    }
}

fn find_second_glyph<'a>(
    pair_pos: &PairPosFormat1<'a>,
    set_index: usize,
    second_glyph: GlyphId,
) -> Option<(PairValueRecord, FontData<'a>)> {
    let set_offset = pair_pos.pair_set_offsets().get(set_index)?.get().to_u32() as usize;
    let format1 = pair_pos.value_format1();
    let format2 = pair_pos.value_format2();
    let record_size = format1.record_byte_len() + format2.record_byte_len() + 2;
    let base_data = pair_pos.offset_data();
    let pair_value_count = base_data.read_at::<u16>(set_offset).ok()? as usize;
    let mut hi = pair_value_count;
    let mut lo = 0;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let record_offset = set_offset + 2 + mid * record_size;
        let glyph_id = base_data
            .read_at::<read_fonts::types::GlyphId16>(record_offset)
            .ok()?;
        if glyph_id < second_glyph {
            lo = mid + 1;
        } else if glyph_id > second_glyph {
            hi = mid;
        } else {
            let set = pair_pos.pair_sets().get(set_index).ok()?;
            return Some((set.pair_value_records().get(mid).ok()?, set.offset_data()));
        }
    }
    None
}

impl Apply for PairPosFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, state: &ApplyState) -> Option<()> {
        let first_glyph = state.glyph;

        let mut iter = skipping_iterator_t::new(ctx, false);
        iter.reset(ctx.buffer.idx);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
            return None;
        }

        let second_glyph_index = iter.index();
        let second_glyph = ctx.buffer.info[second_glyph_index].as_glyph();

        let finish = |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, has_record2| {
            if has_record2 {
                *iter_index += 1;
                // https://github.com/harfbuzz/harfbuzz/issues/3824
                // https://github.com/harfbuzz/harfbuzz/issues/3888#issuecomment-1326781116
                ctx.buffer
                    .unsafe_to_break(Some(ctx.buffer.idx), Some(*iter_index + 1));
            }

            ctx.buffer.idx = *iter_index;

            Some(())
        };

        let boring = |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, has_record2| {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(second_glyph_index + 1));
            finish(ctx, iter_index, has_record2)
        };

        let success =
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, flag1, flag2, has_record2| {
                if flag1 || flag2 {
                    ctx.buffer
                        .unsafe_to_break(Some(ctx.buffer.idx), Some(second_glyph_index + 1));
                    finish(ctx, iter_index, has_record2)
                } else {
                    boring(ctx, iter_index, has_record2)
                }
            };

        let bail =
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, records: (Value, Value)| {
                let flag1 = records.0.apply(ctx, ctx.buffer.idx);
                let flag2 = records.1.apply(ctx, second_glyph_index);

                let has_record2 = !records.1.is_empty();
                success(ctx, iter_index, flag1, flag2, has_record2)
            };

        let class1 = super::super::glyph_class(self.class_def1(), first_glyph);
        let class2 = super::super::glyph_class(self.class_def2(), second_glyph);

        let data = self.offset_data();
        if let Ok(class2_record) = self
            .class1_records()
            .get(class1 as usize)
            .and_then(|rec| rec.class2_records().get(class2 as usize))
        {
            let values = (
                Value {
                    record: class2_record.value_record1,
                    data,
                },
                Value {
                    record: class2_record.value_record2,
                    data,
                },
            );
            bail(ctx, &mut iter.buf_idx, values)
        } else {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(iter.index() + 1));
            None
        }
    }
}
