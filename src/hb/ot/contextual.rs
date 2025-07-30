use super::{coverage_index, covered, glyph_class};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{
    apply_lookup, match_backtrack, match_glyph, match_input, match_lookahead, Apply, WouldApply,
    WouldApplyContext,
};
use read_fonts::tables::gsub::ClassDef;
use read_fonts::tables::layout::{
    ChainedSequenceContextFormat1, ChainedSequenceContextFormat2, ChainedSequenceContextFormat3,
    SequenceContextFormat1, SequenceContextFormat2, SequenceContextFormat3, SequenceLookupRecord,
};
use read_fonts::types::{BigEndian, GlyphId};

impl WouldApply for SequenceContextFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        coverage_index(self.coverage(), ctx.glyphs[0])
            .and_then(|index| {
                self.seq_rule_sets()
                    .get(index as usize)
                    .transpose()
                    .ok()
                    .flatten()
            })
            .is_some_and(|set| {
                set.seq_rules().iter().any(|rule| {
                    rule.map(|rule| {
                        let input = rule.input_sequence();
                        ctx.glyphs.len() == input.len() + 1
                            && input.iter().enumerate().all(|(i, value)| {
                                match_glyph(ctx.glyphs[i + 1], value.get().to_u16())
                            })
                    })
                    .unwrap_or(false)
                })
            })
    }
}

impl Apply for SequenceContextFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().ok()?.get(glyph)? as usize;
        let set = self.seq_rule_sets().get(index)?.ok()?;
        for rule in set.seq_rules().iter().filter_map(|rule| rule.ok()) {
            let input = rule.input_sequence();
            if apply_context(ctx, input, match_glyph, rule.seq_lookup_records()).is_some() {
                return Some(());
            }
        }
        None
    }
}

impl WouldApply for SequenceContextFormat2<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let class_def = self.class_def().ok();
        let match_fn = &match_class(&class_def);
        let class = glyph_class(self.class_def(), ctx.glyphs[0]);
        self.class_seq_rule_sets()
            .get(class as usize)
            .transpose()
            .ok()
            .flatten()
            .is_some_and(|set| {
                set.class_seq_rules().iter().any(|rule| {
                    rule.map(|rule| {
                        let input = rule.input_sequence();
                        ctx.glyphs.len() == input.len() + 1
                            && input
                                .iter()
                                .enumerate()
                                .all(|(i, value)| match_fn(ctx.glyphs[i + 1], value.get()))
                    })
                    .unwrap_or(false)
                })
            })
    }
}

impl Apply for SequenceContextFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let input_classes = self.class_def().ok();
        let glyph = ctx.buffer.cur(0).as_gid16()?;
        self.coverage().ok()?.get(glyph)?;
        let index = input_classes.as_ref()?.get(glyph) as usize;
        let set = self.class_seq_rule_sets().get(index)?.ok()?;
        for rule in set.class_seq_rules().iter().filter_map(|rule| rule.ok()) {
            let input = rule.input_sequence();
            if apply_context(
                ctx,
                input,
                match_class(&input_classes),
                rule.seq_lookup_records(),
            )
            .is_some()
            {
                return Some(());
            }
        }
        None
    }
}

impl WouldApply for SequenceContextFormat3<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let coverages = self.coverages();
        ctx.glyphs.len() == coverages.len() + 1
            && coverages
                .iter()
                .enumerate()
                .all(|(i, coverage)| covered(coverage, ctx.glyphs[i + 1]))
    }
}

impl Apply for SequenceContextFormat3<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let input_coverages = self.coverages();
        input_coverages.get(0).ok()?.get(glyph)?;
        let input = |glyph: GlyphId, index: u16| {
            input_coverages
                .get(index as usize + 1)
                .is_ok_and(|cov| cov.get(glyph).is_some())
        };
        let mut match_end = 0;
        let mut match_positions = smallvec::SmallVec::from_elem(0, 4);
        if match_input(
            ctx,
            input_coverages.len() as u16 - 1,
            &input,
            &mut match_end,
            &mut match_positions,
            None,
        ) {
            ctx.buffer
                .unsafe_to_break_from_outbuffer(Some(ctx.buffer.idx), Some(match_end));
            apply_lookup(
                ctx,
                input_coverages.len() - 1,
                &mut match_positions,
                match_end,
                self.seq_lookup_records(),
            );
            Some(())
        } else {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
            None
        }
    }
}

impl WouldApply for ChainedSequenceContextFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        coverage_index(self.coverage(), ctx.glyphs[0])
            .and_then(|index| {
                self.chained_seq_rule_sets()
                    .get(index as usize)
                    .transpose()
                    .ok()
                    .flatten()
            })
            .is_some_and(|set| {
                set.chained_seq_rules().iter().any(|rule| {
                    rule.map(|rule| {
                        let input = rule.input_sequence();
                        (!ctx.zero_context
                            || (rule.backtrack_glyph_count() == 0
                                && rule.lookahead_glyph_count() == 0))
                            && ctx.glyphs.len() == input.len() + 1
                            && input.iter().enumerate().all(|(i, value)| {
                                match_glyph(ctx.glyphs[i + 1], value.get().to_u16())
                            })
                    })
                    .unwrap_or(false)
                })
            })
    }
}

impl Apply for ChainedSequenceContextFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().ok()?.get(glyph)? as usize;
        let set = self.chained_seq_rule_sets().get(index)?.ok()?;
        for rule in set.chained_seq_rules().iter().filter_map(|rule| rule.ok()) {
            let backtrack = rule.backtrack_sequence();
            let input = rule.input_sequence();
            let lookahead = rule.lookahead_sequence();
            if apply_chain_context(
                ctx,
                backtrack,
                input,
                lookahead,
                [match_glyph; 3],
                rule.seq_lookup_records(),
            )
            .is_some()
            {
                return Some(());
            }
        }
        None
    }
}

impl WouldApply for ChainedSequenceContextFormat2<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let class_def = self.input_class_def().ok();
        let match_fn = &match_class(&class_def);
        let class = glyph_class(self.input_class_def(), ctx.glyphs[0]);
        self.chained_class_seq_rule_sets()
            .get(class as usize)
            .transpose()
            .ok()
            .flatten()
            .is_some_and(|set| {
                set.chained_class_seq_rules().iter().any(|rule| {
                    rule.map(|rule| {
                        let input = rule.input_sequence();
                        (!ctx.zero_context
                            || (rule.backtrack_glyph_count() == 0
                                && rule.lookahead_glyph_count() == 0))
                            && ctx.glyphs.len() == input.len() + 1
                            && input
                                .iter()
                                .enumerate()
                                .all(|(i, value)| match_fn(ctx.glyphs[i + 1], value.get()))
                    })
                    .unwrap_or(false)
                })
            })
    }
}

fn get_class(class_def: &ClassDef, gid: GlyphId) -> u16 {
    let Ok(gid16) = gid.try_into() else {
        return 0;
    };
    class_def.get(gid16)
}

/// Value represents glyph class.
#[inline(always)]
fn match_class<'a>(class_def: &'a Option<ClassDef<'a>>) -> impl Fn(GlyphId, u16) -> bool + 'a {
    |glyph, value| {
        class_def
            .as_ref()
            .is_some_and(|class_def| get_class(class_def, glyph) == value)
    }
}

impl Apply for ChainedSequenceContextFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let backtrack_classes = self.backtrack_class_def().ok();
        let input_classes = self.input_class_def().ok();
        let lookahead_classes = self.lookahead_class_def().ok();
        let glyph = ctx.buffer.cur(0).as_gid16()?;
        self.coverage().ok()?.get(glyph)?;
        let index = input_classes.as_ref()?.get(glyph) as usize;
        let set = self.chained_class_seq_rule_sets().get(index)?.ok()?;
        for rule in set
            .chained_class_seq_rules()
            .iter()
            .filter_map(|rule| rule.ok())
        {
            let backtrack = rule.backtrack_sequence();
            let input = rule.input_sequence();
            let lookahead = rule.lookahead_sequence();
            if apply_chain_context(
                ctx,
                backtrack,
                input,
                lookahead,
                [
                    match_class(&backtrack_classes),
                    match_class(&input_classes),
                    match_class(&lookahead_classes),
                ],
                rule.seq_lookup_records(),
            )
            .is_some()
            {
                return Some(());
            }
        }
        None
    }
}

impl WouldApply for ChainedSequenceContextFormat3<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let input_coverages = self.input_coverages();
        (!ctx.zero_context
            || (self.backtrack_coverage_offsets().is_empty()
                && self.lookahead_coverage_offsets().is_empty()))
            && (ctx.glyphs.len() == input_coverages.len() + 1
                && input_coverages.iter().enumerate().all(|(i, coverage)| {
                    coverage
                        .map(|cov| cov.get(ctx.glyphs[i + 1]).is_some())
                        .unwrap_or(false)
                }))
    }
}

impl Apply for ChainedSequenceContextFormat3<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();

        let input_coverages = self.input_coverages();
        input_coverages.get(0).ok()?.get(glyph)?;

        let backtrack_coverages = self.backtrack_coverages();
        let lookahead_coverages = self.lookahead_coverages();

        let back = |glyph: GlyphId, index: u16| {
            backtrack_coverages
                .get(index as usize)
                .is_ok_and(|cov| cov.get(glyph).is_some())
        };

        let ahead = |glyph: GlyphId, index: u16| {
            lookahead_coverages
                .get(index as usize)
                .is_ok_and(|cov| cov.get(glyph).is_some())
        };

        let input = |glyph: GlyphId, index: u16| {
            input_coverages
                .get(index as usize + 1)
                .is_ok_and(|cov| cov.get(glyph).is_some())
        };

        let mut end_index = ctx.buffer.idx;
        let mut match_end = 0;
        let mut match_positions = smallvec::SmallVec::from_elem(0, 4);

        let input_matches = match_input(
            ctx,
            input_coverages.len() as u16 - 1,
            input,
            &mut match_end,
            &mut match_positions,
            None,
        );

        if input_matches {
            end_index = match_end;
        }

        if !(input_matches
            && match_lookahead(
                ctx,
                lookahead_coverages.len() as u16,
                &ahead,
                match_end,
                &mut end_index,
            ))
        {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_index));
            return None;
        }

        let mut start_index = ctx.buffer.out_len;

        if !match_backtrack(
            ctx,
            backtrack_coverages.len() as u16,
            &back,
            &mut start_index,
        ) {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
            return None;
        }

        ctx.buffer
            .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
        apply_lookup(
            ctx,
            input_coverages.len() - 1,
            &mut match_positions,
            match_end,
            self.seq_lookup_records(),
        );

        Some(())
    }
}

trait ToU16: Copy {
    fn to_u16(self) -> u16;
}

impl ToU16 for BigEndian<read_fonts::types::GlyphId16> {
    fn to_u16(self) -> u16 {
        self.get().to_u16()
    }
}

impl ToU16 for BigEndian<u16> {
    fn to_u16(self) -> u16 {
        self.get()
    }
}

fn apply_context<T: ToU16>(
    ctx: &mut hb_ot_apply_context_t,
    input: &[T],
    match_func: impl Fn(GlyphId, u16) -> bool,
    lookups: &[SequenceLookupRecord],
) -> Option<()> {
    let match_func = |glyph, index| {
        let value = input.get(index as usize).unwrap().to_u16();
        match_func(glyph, value)
    };

    let mut match_end = 0;
    let mut match_positions = smallvec::SmallVec::from_elem(0, 4);

    if match_input(
        ctx,
        input.len() as _,
        match_func,
        &mut match_end,
        &mut match_positions,
        None,
    ) {
        ctx.buffer
            .unsafe_to_break(Some(ctx.buffer.idx), Some(match_end));
        apply_lookup(ctx, input.len(), &mut match_positions, match_end, lookups);
        return Some(());
    }

    None
}

fn apply_chain_context<T: ToU16, F: Fn(GlyphId, u16) -> bool>(
    ctx: &mut hb_ot_apply_context_t,
    backtrack: &[T],
    input: &[T],
    lookahead: &[T],
    match_funcs: [F; 3],
    lookups: &[SequenceLookupRecord],
) -> Option<()> {
    // NOTE: Whenever something in this method changes, we also need to
    // change it in the `apply` implementation for ChainedContextLookup.
    let f1 = |glyph, index| {
        let value = (*backtrack.get(index as usize).unwrap()).to_u16();
        match_funcs[0](glyph, value)
    };

    let f2 = |glyph, index| {
        let value = (*lookahead.get(index as usize).unwrap()).to_u16();
        match_funcs[2](glyph, value)
    };

    let f3 = |glyph, index| {
        let value = (*input.get(index as usize).unwrap()).to_u16();
        match_funcs[1](glyph, value)
    };

    let mut end_index = ctx.buffer.idx;
    let mut match_end = 0;
    let mut match_positions = smallvec::SmallVec::from_elem(0, 4);

    let input_matches = match_input(
        ctx,
        input.len() as u16,
        &f3,
        &mut match_end,
        &mut match_positions,
        None,
    );

    if input_matches {
        end_index = match_end;
    }

    if !(input_matches
        && match_lookahead(ctx, lookahead.len() as u16, &f2, match_end, &mut end_index))
    {
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_index));
        return None;
    }

    let mut start_index = ctx.buffer.out_len;

    if !match_backtrack(ctx, backtrack.len() as u16, &f1, &mut start_index) {
        ctx.buffer
            .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
        return None;
    }

    ctx.buffer
        .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
    apply_lookup(ctx, input.len(), &mut match_positions, match_end, lookups);

    Some(())
}
