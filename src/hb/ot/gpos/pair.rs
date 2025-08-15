use crate::hb::ot::{coverage_index, coverage_index_cached};
use crate::hb::ot::{glyph_class, glyph_class_cached};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{
    skipping_iterator_t, Apply, PairPosFormat1Cache, PairPosFormat2Cache, SubtableExternalCache,
};
use alloc::boxed::Box;
use read_fonts::tables::gpos::{PairPosFormat1, PairPosFormat2, Value, ValueContext};
use read_fonts::types::GlyphId;

use super::apply_value_to_pos;

impl Apply for PairPosFormat1<'_> {
    fn apply_with_external_cache(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        external_cache: &SubtableExternalCache,
    ) -> Option<()> {
        let first_glyph = ctx.buffer.cur(0).as_glyph();

        let first_glyph_coverage_index =
            if let SubtableExternalCache::PairPosFormat1Cache(cache) = external_cache {
                coverage_index_cached(
                    |gid| self.coverage().ok()?.get(gid),
                    first_glyph,
                    &cache.coverage,
                )?
            } else {
                coverage_index(self.coverage(), first_glyph)?
            };

        let mut iter = skipping_iterator_t::new(ctx, false);
        iter.reset(iter.buffer.idx);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
            return None;
        }

        let second_glyph_index = iter.index();
        let second_glyph = iter.buffer.info[second_glyph_index].as_glyph();

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
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, records: [Value; 2]| {
                let flag1 = apply_value_to_pos(ctx, ctx.buffer.idx, &records[0]);
                let flag2 = apply_value_to_pos(ctx, second_glyph_index, &records[1]);

                let has_record2 = !records[1].format.is_empty();
                success(ctx, iter_index, flag1, flag2, has_record2)
            };

        let mut buf_idx = iter.buf_idx;
        let values = pair_pos1_values(
            self,
            first_glyph_coverage_index as usize,
            second_glyph,
            &ctx.face.ot_tables.value_context,
        )?;
        bail(ctx, &mut buf_idx, values)
    }

    fn external_cache_create(&self) -> SubtableExternalCache {
        SubtableExternalCache::PairPosFormat1Cache(Box::new(PairPosFormat1Cache::new()))
    }
}

fn pair_pos1_values(
    pair_pos: &PairPosFormat1,
    set_index: usize,
    second_glyph: GlyphId,
    value_context: &ValueContext,
) -> Option<[Value; 2]> {
    let set_offset = pair_pos.pair_set_offsets().get(set_index)?.get().to_u32() as usize;
    let format1 = pair_pos.value_format1();
    let format2 = pair_pos.value_format2();
    let format1_len = format1.record_byte_len();
    let record_size = format1_len + format2.record_byte_len() + 2;
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
            return Some([
                Value::read(base_data, record_offset + 2, format1, value_context).ok()?,
                Value::read(
                    base_data,
                    record_offset + 2 + format1_len,
                    format2,
                    value_context,
                )
                .ok()?,
            ]);
        }
    }
    None
}

impl Apply for PairPosFormat2<'_> {
    fn apply_with_external_cache(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        external_cache: &SubtableExternalCache,
    ) -> Option<()> {
        let first_glyph = ctx.buffer.cur(0).as_glyph();

        let _ = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
            coverage_index_cached(
                |gid| self.coverage().ok()?.get(gid),
                first_glyph,
                &cache.coverage,
            )?
        } else {
            coverage_index(self.coverage(), first_glyph)?
        };

        let mut iter = skipping_iterator_t::new(ctx, false);
        iter.reset(iter.buffer.idx);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
            return None;
        }

        let second_glyph_index = iter.index();
        let second_glyph = iter.buffer.info[second_glyph_index].as_glyph();

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
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, records: &[Value; 2]| {
                let flag1 = apply_value_to_pos(ctx, ctx.buffer.idx, &records[0]);
                let flag2 = apply_value_to_pos(ctx, second_glyph_index, &records[1]);

                let has_record2 = !records[1].format.is_empty();
                success(ctx, iter_index, flag1, flag2, has_record2)
            };

        let class1 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
            glyph_class_cached(
                |gid| glyph_class(self.class_def1(), gid),
                first_glyph,
                &cache.first,
            )
        } else {
            glyph_class(self.class_def1(), first_glyph)
        };
        let class2 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
            glyph_class_cached(
                |gid| glyph_class(self.class_def2(), gid),
                second_glyph,
                &cache.second,
            )
        } else {
            glyph_class(self.class_def2(), second_glyph)
        };
        let mut buf_idx = iter.buf_idx;
        let end_idx = iter.index() + 1;
        if let Ok(values) = self.values(class1, class2, &ctx.face.ot_tables.value_context) {
            bail(ctx, &mut buf_idx, &values)
        } else {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_idx));
            None
        }
    }

    fn external_cache_create(&self) -> SubtableExternalCache {
        SubtableExternalCache::PairPosFormat2Cache(Box::new(PairPosFormat2Cache::new()))
    }
}
