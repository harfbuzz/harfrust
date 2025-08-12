use core::cmp::Ordering;

use crate::hb::ot::{coverage_index, coverage_index_cached, read_class_def, read_coverage_index};
use crate::hb::ot::{glyph_class, glyph_class_cached};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{skipping_iterator_t, Apply, SubtableExternalCache};
use read_fonts::tables::gpos::{PairPosFormat1, PairPosFormat2, PairValueRecord, ValueFormat};
use read_fonts::types::GlyphId;
use read_fonts::FontData;

use super::{Value, ValueReader};

impl Apply for PairPosFormat1<'_> {
    fn apply_with_external_cache(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        external_cache: &SubtableExternalCache,
    ) -> Option<()> {
        let first_glyph = ctx.buffer.cur(0).as_glyph();

        let first_glyph_coverage_index =
            if let SubtableExternalCache::MappingCache(cache) = external_cache {
                coverage_index_cached(|gid| self.coverage().ok()?.get(gid), first_glyph, cache)?
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
        let mut buf_idx = iter.buf_idx;
        bail(ctx, &mut buf_idx, values)
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
            |ctx: &mut hb_ot_apply_context_t, iter_index: &mut usize, records: (Value, Value)| {
                let flag1 = records.0.apply(ctx, ctx.buffer.idx);
                let flag2 = records.1.apply(ctx, second_glyph_index);

                let has_record2 = !records.1.is_empty();
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
            let mut buf_idx = iter.buf_idx;
            bail(ctx, &mut buf_idx, values)
        } else {
            iter.buffer
                .unsafe_to_concat(Some(iter.buffer.idx), Some(iter.index() + 1));
            None
        }
    }
}

pub fn apply_pair_pos1(
    ctx: &mut hb_ot_apply_context_t,
    table_data: &[u8],
    base: usize,
    external_cache: &SubtableExternalCache,
) -> Option<()> {
    let first_glyph = ctx.buffer.cur(0).as_glyph();
    let first_glyph_coverage_index =
        if let SubtableExternalCache::MappingCache(cache) = external_cache {
            coverage_index_cached(
                |gid| read_coverage_index(table_data, base, 2, gid),
                first_glyph,
                cache,
            )?
        } else {
            read_coverage_index(table_data, base, 2, first_glyph)?
        };

    let mut iter = skipping_iterator_t::new(ctx, false);
    iter.reset(iter.buffer.idx);

    let mut unsafe_to = 0;
    if !iter.next(Some(&mut unsafe_to)) {
        iter.buffer
            .unsafe_to_concat(Some(iter.buffer.idx), Some(unsafe_to));
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

    let bail = |ctx: &mut hb_ot_apply_context_t,
                iter_index: &mut usize,
                records: (ValueReader, ValueReader)| {
        let flag1 = records.0.apply(ctx, ctx.buffer.idx);
        let flag2 = records.1.apply(ctx, second_glyph_index);

        let has_record2 = !records.1.is_empty();
        success(ctx, iter_index, flag1, flag2, has_record2)
    };

    let data = FontData::new(table_data);
    let format1 = data.read_at::<ValueFormat>(base + 4).ok()?;
    let format2 = data.read_at::<ValueFormat>(base + 6).ok()?;
    let len1 = format1.record_byte_len();
    let record_size = len1 + format2.record_byte_len() + 2;
    let set_base = base
        + data
            .read_at::<u16>(base + 10 + first_glyph_coverage_index as usize * 2)
            .ok()? as usize;
    let count = data.read_at::<u16>(set_base).ok()? as usize;
    let val_base = set_base + 2;
    let mut lo = 0;
    let mut hi = count;
    let mut values = None;
    while lo < hi {
        let index = (lo + hi) / 2;
        let rec_offset = val_base + index * record_size;
        let glyph: GlyphId = data.read_at::<u16>(rec_offset).ok()?.into();
        match second_glyph.cmp(&glyph) {
            Ordering::Greater => lo = index + 1,
            Ordering::Less => hi = index,
            Ordering::Equal => {
                values = Some((
                    ValueReader::new(data, set_base, rec_offset + 2, format1),
                    ValueReader::new(data, set_base, rec_offset + 2 + len1, format2),
                ));
                break;
            }
        }
    }
    let mut buf_idx = iter.buf_idx;
    bail(ctx, &mut buf_idx, values?)
}

pub fn apply_pair_pos2(
    ctx: &mut hb_ot_apply_context_t,
    table_data: &[u8],
    base: usize,
    external_cache: &SubtableExternalCache,
) -> Option<()> {
    let first_glyph = ctx.buffer.cur(0).as_glyph();
    let _ = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
        coverage_index_cached(
            |gid| read_coverage_index(table_data, base, 2, gid),
            first_glyph,
            &cache.coverage,
        )?
    } else {
        read_coverage_index(table_data, base, 2, first_glyph)?
    };
    let mut iter = skipping_iterator_t::new(ctx, false);
    iter.reset(iter.buffer.idx);

    let mut unsafe_to = 0;
    if !iter.next(Some(&mut unsafe_to)) {
        iter.buffer
            .unsafe_to_concat(Some(iter.buffer.idx), Some(unsafe_to));
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

    let bail = |ctx: &mut hb_ot_apply_context_t,
                iter_index: &mut usize,
                records: (ValueReader, ValueReader)| {
        let flag1 = records.0.apply(ctx, ctx.buffer.idx);
        let flag2 = records.1.apply(ctx, second_glyph_index);

        let has_record2 = !records.1.is_empty();
        success(ctx, iter_index, flag1, flag2, has_record2)
    };

    let data = FontData::new(table_data);
    let format1 = data.read_at::<ValueFormat>(base + 4).ok()?;
    let format2 = data.read_at::<ValueFormat>(base + 6).ok()?;
    let len1 = format1.record_byte_len();
    let record_size = len1 + format2.record_byte_len();

    let class1 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
        glyph_class_cached(
            |gid| read_class_def(table_data, base, 8, gid).unwrap_or(0),
            first_glyph,
            &cache.first,
        )
    } else {
        read_class_def(table_data, base, 8, first_glyph).unwrap_or(0)
    };
    let class2 = if let SubtableExternalCache::PairPosFormat2Cache(cache) = external_cache {
        glyph_class_cached(
            |gid| read_class_def(table_data, base, 10, gid).unwrap_or(0),
            second_glyph,
            &cache.second,
        )
    } else {
        read_class_def(table_data, base, 10, second_glyph).unwrap_or(0)
    };
    let class2_count = data.read_at::<u16>(base + 14).ok()? as usize;
    let rec_offset = base
        + 16
        + (class1 as usize * record_size * class2_count)
        + (class2 as usize * record_size);
    let values = (
        ValueReader::new(data, base, rec_offset, format1),
        ValueReader::new(data, base, rec_offset + len1, format2),
    );
    let mut buf_idx = iter.buf_idx;
    bail(ctx, &mut buf_idx, values)
}
