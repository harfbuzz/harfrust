use crate::hb::ot::coverage_index_cached;
use crate::hb::ot::{glyph_class, glyph_class_cached};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{
    skipping_iterator_t, Apply, PairPosFormat1Cache, PairPosFormat2Cache, SubtableExternalCache,
};
use alloc::boxed::Box;
use read_fonts::tables::gpos::{PairPosFormat1, PairPosFormat2};

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
                unreachable!();
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

        let mut buf_idx = iter.buf_idx;
        let set_offset = self
            .pair_set_offsets()
            .get(first_glyph_coverage_index as usize)?
            .get()
            .to_u32() as usize;
        let format1 = self.value_format1();
        let format1_len = format1.record_byte_len();
        let format2 = self.value_format2();
        let record_size = format1_len + format2.record_byte_len() + 2;
        let data = self.offset_data();
        let set_data = data.split_off(set_offset)?;
        let pair_count = set_data.read_at::<u16>(0).ok()? as usize;
        let mut hi = pair_count;
        let mut lo = 0;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let record_offset = 2 + mid * record_size;
            let glyph_id = set_data
                .read_at::<read_fonts::types::GlyphId16>(record_offset)
                .ok()?;
            if glyph_id < second_glyph {
                lo = mid + 1;
            } else if glyph_id > second_glyph {
                hi = mid;
            } else {
                let has_record2 = !format2.is_empty();
                let worked1 = !format1.is_empty()
                    && super::apply_value(
                        ctx,
                        ctx.buffer.idx,
                        &set_data,
                        record_offset + 2,
                        format1,
                    ) == Some(true);
                let worked2 = has_record2
                    && super::apply_value(
                        ctx,
                        second_glyph_index,
                        &set_data,
                        record_offset + format1_len + 2,
                        format2,
                    ) == Some(true);
                return success(ctx, &mut buf_idx, worked1, worked2, has_record2);
            }
        }
        None
    }

    fn external_cache_create(&self) -> SubtableExternalCache {
        SubtableExternalCache::PairPosFormat1Cache(Box::new(PairPosFormat1Cache::new()))
    }
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
            unreachable!();
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

        let class1 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
            glyph_class_cached(
                |gid| glyph_class(self.class_def1(), gid),
                first_glyph,
                &cache.first,
            )
        } else {
            unreachable!();
        };
        let class2 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
            glyph_class_cached(
                |gid| glyph_class(self.class_def2(), gid),
                second_glyph,
                &cache.second,
            )
        } else {
            unreachable!();
        };
        let mut buf_idx = iter.buf_idx;
        let format1 = self.value_format1();
        let format1_len = format1.record_byte_len();
        let format2 = self.value_format2();
        let record_size = format1_len + format2.record_byte_len();
        let data = self.offset_data();
        // Compute an offset into the 2D array of positioning records
        let record_offset = (class1 as usize * record_size * self.class2_count() as usize)
            + (class2 as usize * record_size)
            + self.shape().class1_records_byte_range().start;
        let has_record2 = !format2.is_empty();
        let worked1 = !format1.is_empty()
            && super::apply_value(ctx, ctx.buffer.idx, &data, record_offset, format1) == Some(true);
        let worked2 = has_record2
            && super::apply_value(
                ctx,
                second_glyph_index,
                &data,
                record_offset + format1_len,
                format2,
            ) == Some(true);
        success(ctx, &mut buf_idx, worked1, worked2, has_record2)
    }

    fn external_cache_create(&self) -> SubtableExternalCache {
        SubtableExternalCache::PairPosFormat2Cache(Box::new(PairPosFormat2Cache::new()))
    }
}
