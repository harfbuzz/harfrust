use crate::hb::buffer::hb_glyph_info_t;
use crate::hb::ot::coverage_index_cached;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{
    ligate_input, match_glyph, match_input, may_skip_t, skipping_iterator_t, Apply,
    SubtableExternalCache, WouldApply, WouldApplyContext,
};
use read_fonts::tables::gsub::{Ligature, LigatureSet, LigatureSubstFormat1};
use read_fonts::types::GlyphId;

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

        let SubtableExternalCache::MappingCache(cache) = external_cache else {
            return None;
        };
        let index = coverage_index_cached(self.coverage(), glyph, cache)?;
        self.ligature_sets()
            .get(index as usize)
            .ok()
            .and_then(|set| set.apply(ctx))
    }
}
