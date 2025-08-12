use crate::hb::buffer::hb_glyph_info_t;
use crate::hb::ot::{coverage_index, coverage_index_cached, read_coverage_index};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{
    ligate_input, match_glyph, match_input, may_skip_t, skipping_iterator_t, Apply,
    SubtableExternalCache, WouldApply, WouldApplyContext,
};
use read_fonts::tables::gsub::{Ligature, LigatureSet, LigatureSubstFormat1};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16};
use read_fonts::FontData;

impl WouldApply for Ligature<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let components = self.component_glyph_ids();
        ctx.glyphs.len() == components.len() + 1
            && components
                .iter()
                .map(|comp| GlyphId::from(comp.get()))
                .enumerate()
                .all(|(i, comp)| ctx.glyphs[i + 1] == comp)
    }
}

impl Apply for Ligature<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        // Special-case to make it in-place and not consider this
        // as a "ligated" substitution.
        let components = self.component_glyph_ids();
        if components.is_empty() {
            ctx.replace_glyph(self.ligature_glyph().into());
            Some(())
        } else {
            let f = |info: &mut hb_glyph_info_t, index| {
                let value = components.get(index as usize).unwrap().get().to_u16();
                match_glyph(info, value)
            };

            let mut match_end = 0;
            let mut match_positions = smallvec::SmallVec::from_elem(0, 4);
            let mut total_component_count = 0;

            if !match_input(
                ctx,
                components.len() as u16,
                f,
                &mut match_end,
                &mut match_positions,
                Some(&mut total_component_count),
            ) {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
                return None;
            }
            let count = components.len() + 1;
            ligate_input(
                ctx,
                count,
                &match_positions,
                match_end,
                total_component_count,
                self.ligature_glyph().into(),
            );
            Some(())
        }
    }
}

impl WouldApply for LigatureSet<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        self.ligatures()
            .iter()
            .filter_map(|lig| lig.ok())
            .any(|lig| lig.would_apply(ctx))
    }
}

impl Apply for LigatureSet<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let mut first = GlyphId::new(u32::MAX);
        let mut unsafe_to = 0;
        let slow_path = if self.ligatures().len() <= 4 {
            true
        } else {
            let mut iter = skipping_iterator_t::new(ctx, false);
            iter.reset(iter.buffer.idx);
            let matched = iter.next(Some(&mut unsafe_to));
            if !matched {
                true
            } else {
                first = iter.buffer.info[iter.index()].glyph_id.into();
                unsafe_to = iter.index() + 1;

                // Can't use the fast path if eg. the next char is a default-ignorable
                // or other skippable.
                iter.may_skip(&iter.buffer.info[iter.index()]) != may_skip_t::SKIP_NO
            }
        };

        if slow_path {
            // Slow path
            for lig in self.ligatures().iter().filter_map(|lig| lig.ok()) {
                if lig.apply(ctx).is_some() {
                    return Some(());
                }
            }
        } else {
            // Fast path
            let mut unsafe_to_concat = false;
            for lig in self.ligatures().iter().filter_map(|lig| lig.ok()) {
                let components = lig.component_glyph_ids();
                if components.is_empty() || components[0].get() == first {
                    if lig.apply(ctx).is_some() {
                        if unsafe_to_concat {
                            ctx.buffer
                                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
                        }
                        return Some(());
                    }
                } else if !components.is_empty() {
                    unsafe_to_concat = true;
                }
            }
            if unsafe_to_concat {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
            }
        }
        None
    }
}

impl WouldApply for LigatureSubstFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        self.coverage()
            .ok()
            .and_then(|coverage| coverage.get(ctx.glyphs[0]))
            .and_then(|index| self.ligature_sets().get(index as usize).ok())
            .is_some_and(|set| set.would_apply(ctx))
    }
}

impl Apply for LigatureSubstFormat1<'_> {
    fn apply_with_external_cache(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        external_cache: &SubtableExternalCache,
    ) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();

        let index = if let SubtableExternalCache::MappingCache(cache) = external_cache {
            coverage_index_cached(|gid| self.coverage().ok()?.get(gid), glyph, cache)?
        } else {
            coverage_index(self.coverage(), glyph)?
        };
        self.ligature_sets()
            .get(index as usize)
            .ok()
            .and_then(|set| set.apply(ctx))
    }
}

struct LigSet<'a> {
    data: FontData<'a>,
    base: usize,
    count: usize,
}

impl<'a> LigSet<'a> {
    fn new(table_data: &'a [u8], base: usize, coverage_index: usize) -> Option<Self> {
        let data = FontData::new(table_data);
        let base = base + data.read_at::<u16>(base + 6 + coverage_index * 2).ok()? as usize;
        let count = data.read_at::<u16>(base).ok()? as usize;
        Some(Self { data, base, count })
    }

    fn check_first_component(
        &self,
        index: usize,
        glyph_id: GlyphId,
        out_comp_count: &mut usize,
    ) -> Option<bool> {
        let lig_base =
            self.base + self.data.read_at::<u16>(self.base + 2 + index * 2).ok()? as usize;
        let comp_count = (self.data.read_at::<u16>(lig_base + 2).ok()? as usize).checked_sub(1)?;
        *out_comp_count = comp_count;
        if comp_count == 0
            || self.data.read_at::<u16>(lig_base + 4).ok()? as u32 == glyph_id.to_u32()
        {
            return Some(true);
        }
        None
    }

    fn apply(&self, ctx: &mut hb_ot_apply_context_t, index: usize) -> Option<()> {
        let lig_base =
            self.base + self.data.read_at::<u16>(self.base + 2 + index * 2).ok()? as usize;
        let comp_count = (self.data.read_at::<u16>(lig_base + 2).ok()? as usize).checked_sub(1)?;
        let components = self
            .data
            .read_array::<BigEndian<GlyphId16>>(lig_base + 4..lig_base + 4 + comp_count * 2)
            .ok()?;
        if components.is_empty() {
            let liga_glyph = self.data.read_at::<u16>(lig_base).ok()? as u32;
            ctx.replace_glyph(liga_glyph.into());
            Some(())
        } else {
            let f = |info: &mut hb_glyph_info_t, index| {
                let value = components.get(index as usize).unwrap().get().to_u16();
                match_glyph(info, value)
            };
            let mut match_end = 0;
            let mut match_positions = smallvec::SmallVec::from_elem(0, 4);
            let mut total_component_count = 0;
            if !match_input(
                ctx,
                components.len() as u16,
                f,
                &mut match_end,
                &mut match_positions,
                Some(&mut total_component_count),
            ) {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
                return None;
            }
            let liga_glyph = self.data.read_at::<u16>(lig_base).ok()? as u32;
            let count = components.len() + 1;
            ligate_input(
                ctx,
                count,
                &match_positions,
                match_end,
                total_component_count,
                liga_glyph.into(),
            );
            Some(())
        }
    }
}

pub fn apply_lig_subst1(
    ctx: &mut hb_ot_apply_context_t,
    table_data: &[u8],
    base: usize,
    external_cache: &SubtableExternalCache,
) -> Option<()> {
    let glyph = ctx.buffer.cur(0).as_glyph();
    let index = if let SubtableExternalCache::MappingCache(cache) = external_cache {
        coverage_index_cached(
            |gid| read_coverage_index(table_data, base, 2, gid),
            glyph,
            cache,
        )?
    } else {
        read_coverage_index(table_data, base, 2, glyph)?
    };
    let set = LigSet::new(table_data, base, index as usize)?;
    let mut first = GlyphId::new(u32::MAX);
    let mut unsafe_to = 0;
    let slow_path = if set.count <= 4 {
        true
    } else {
        let mut iter = skipping_iterator_t::new(ctx, false);
        iter.reset(iter.buffer.idx);
        let matched = iter.next(Some(&mut unsafe_to));
        if !matched {
            true
        } else {
            first = iter.buffer.info[iter.index()].glyph_id.into();
            unsafe_to = iter.index() + 1;

            // Can't use the fast path if eg. the next char is a default-ignorable
            // or other skippable.
            iter.may_skip(&iter.buffer.info[iter.index()]) != may_skip_t::SKIP_NO
        }
    };

    if slow_path {
        // Slow path
        for i in 0..set.count {
            if set.apply(ctx, i).is_some() {
                return Some(());
            }
        }
    } else {
        // Fast path
        let mut unsafe_to_concat = false;
        for i in 0..set.count {
            let mut comp_count = 0;
            if set.check_first_component(i, first, &mut comp_count) == Some(true) {
                if set.apply(ctx, i).is_some() {
                    if unsafe_to_concat {
                        ctx.buffer
                            .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
                    }
                    return Some(());
                }
            } else if comp_count != 0 {
                unsafe_to_concat = true;
            }
        }
        if unsafe_to_concat {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
        }
    }
    None
}
