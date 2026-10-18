//! Substitution subtables.

use super::coverage::{Coverage, CoverageArray};
use super::{Offsets, SubtableDigests};
use crate::buffer::{GlyphInfo, GlyphPropsFlags};
use crate::hint::Likely;
use crate::ot::apply::{
    ligate_input, match_always, match_backtrack, match_glyph, match_input, match_lookahead,
    ApplyContext, MaySkip, SkippingIterator, WouldApplyContext,
};
use crate::ot::layout::MAX_NESTING_LEVEL;
use crate::ot::map::OtMap;
use crate::set_digest::SetDigest;
#[cfg(feature = "tracing")]
use alloc::string::String;
use read_fonts::tables::gsub::{
    AlternateSet, AlternateSubstFormat1, Ligature, LigatureSet, LigatureSubstFormat1,
    MultipleSubstFormat1, ReverseChainSingleSubstFormat1, Sequence, SingleSubstFormat1,
    SingleSubstFormat2,
};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16};

/// Single substitution by a fixed delta.
pub(crate) struct SingleSubst1 {
    delta: i16,
}

impl SingleSubst1 {
    pub fn new(table: &SingleSubstFormat1) -> Option<Self> {
        Some(Self {
            delta: table.delta_glyph_id(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, _index: u16) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        // The delta wraps around within 16 bits.
        let substitute = (glyph.to_u32() as i32 + i32::from(self.delta)) as u16;
        message!(
            ctx,
            "replacing glyph at {} (single substitution)",
            ctx.buffer.message_idx()
        );
        ctx.replace_glyph(substitute.into());
        message!(
            ctx,
            "replaced glyph at {} (single substitution)",
            ctx.buffer.message_idx() - 1
        );
        Some(())
    }
}

impl SubtableDigests<'_> for SingleSubst1 {}

/// Single substitution from an array of substitutes.
pub(crate) struct SingleSubst2<'a> {
    substitutes: &'a [BigEndian<GlyphId16>],
}

impl<'a> SingleSubst2<'a> {
    pub fn new(table: &SingleSubstFormat2<'a>) -> Option<Self> {
        Some(Self {
            substitutes: table.substitute_glyph_ids(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let index = usize::from(index);
        let substitute = self.substitutes.get(index)?.get();
        message!(
            ctx,
            "replacing glyph at {} (single substitution)",
            ctx.buffer.message_idx()
        );
        ctx.replace_glyph(substitute.into());
        message!(
            ctx,
            "replaced glyph at {} (single substitution)",
            ctx.buffer.message_idx() - 1
        );
        Some(())
    }
}

impl<'a> SubtableDigests<'a> for SingleSubst2<'a> {}

/// Multiple substitution: one glyph into a sequence.
pub(crate) struct MultipleSubst1<'a> {
    sequences: Offsets<'a>,
}

impl<'a> MultipleSubst1<'a> {
    pub fn new(table: &MultipleSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            sequences: Offsets::new(table.offset_data(), table.sequence_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let index = usize::from(index);
        let sequence: Sequence = self.sequences.get(index)?;
        let substitutes = sequence.substitute_glyph_ids();
        match substitutes {
            // The spec disallows empty sequences, but Uniscribe deletes the
            // glyph. https://github.com/harfbuzz/harfbuzz/issues/253
            [] => {
                message!(
                    ctx,
                    "deleting glyph at {} (multiple substitution)",
                    ctx.buffer.message_idx()
                );
                ctx.buffer.delete_glyph();
                message!(
                    ctx,
                    "deleted glyph at {} (multiple substitution)",
                    ctx.buffer.message_idx()
                );
            }
            // A single substitute is an in-place replacement, not a
            // multiplication.
            [substitute] => {
                message!(
                    ctx,
                    "replacing glyph at {} (multiple substitution)",
                    ctx.buffer.message_idx()
                );
                ctx.replace_glyph(substitute.get().into());
                message!(
                    ctx,
                    "replaced glyph at {} (multiple substitution)",
                    ctx.buffer.message_idx() - 1
                );
            }
            _ => {
                message!(ctx, "multiplying glyph at {}", ctx.buffer.message_idx());
                let class = if ctx.buffer.cur(0)?.is_ligature() {
                    GlyphPropsFlags::BASE_GLYPH
                } else {
                    GlyphPropsFlags::empty()
                };
                let lig_id = ctx.buffer.cur(0)?.lig_id();
                for (component, substitute) in substitutes.iter().enumerate() {
                    // Leave a glyph attached to a ligature as it is.
                    // https://github.com/harfbuzz/harfbuzz/issues/3069
                    if lig_id == 0 {
                        // The component is stored in four bits.
                        ctx.buffer
                            .cur_mut(0)?
                            .set_lig_props_for_component(component as u8);
                    }
                    ctx.output_glyph_for_component(substitute.get().into(), class);
                }
                ctx.buffer.skip_glyph();
                #[cfg(feature = "tracing")]
                if ctx.buffer.messaging() && ctx.buffer.successful {
                    let count = substitutes.len();
                    let end = ctx.buffer.message_idx();
                    let mut msg = String::from("multiplied glyphs at ");
                    for i in (end - count)..end {
                        if i > (end - count) {
                            msg.push(',');
                        }
                        use core::fmt::Write;
                        let _ = write!(msg, "{i}");
                    }
                    ctx.buffer.message(ctx.font, &msg);
                }
            }
        }
        Some(())
    }
}

impl<'a> SubtableDigests<'a> for MultipleSubst1<'a> {}

/// Alternate substitution, chosen by the feature value.
pub(crate) struct AlternateSubst1<'a> {
    alternate_sets: Offsets<'a>,
}

impl<'a> AlternateSubst1<'a> {
    pub fn new(table: &AlternateSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            alternate_sets: Offsets::new(table.offset_data(), table.alternate_set_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let index = usize::from(index);
        apply_alternate_set(ctx, &self.alternate_sets.get(index)?)
    }
}

impl<'a> SubtableDigests<'a> for AlternateSubst1<'a> {}

fn apply_alternate_set(ctx: &mut ApplyContext, set: &AlternateSet) -> Option<()> {
    let alternates = set.alternate_glyph_ids();
    if alternates.is_empty() {
        return None;
    }
    let glyph_mask = ctx.buffer.cur(0)?.mask;
    let lookup_mask = ctx.lookup_mask();
    // This breaks if two features enable the lookup together.
    let mut alt_index = (lookup_mask & glyph_mask) >> lookup_mask.trailing_zeros();
    // The maximum value picks a random alternate under the `rand` feature.
    if alt_index == OtMap::MAX_VALUE && ctx.random {
        // Changing the random state makes every position unsafe to break.
        ctx.buffer.unsafe_to_break(Some(0), Some(ctx.buffer.len));
        alt_index = ctx.random_number() % alternates.len() as u32 + 1;
    }
    let index = u16::try_from(alt_index).ok()?.checked_sub(1)?;
    message!(
        ctx,
        "replacing glyph at {} (alternate substitution)",
        ctx.buffer.message_idx()
    );
    ctx.replace_glyph(alternates.get(index as usize)?.get().into());
    message!(
        ctx,
        "replaced glyph at {} (alternate substitution)",
        ctx.buffer.message_idx() - 1
    );
    Some(())
}

/// Ligature substitution.
pub(crate) struct LigatureSubst1<'a> {
    /// The second components of all ligatures.
    seconds: SetDigest,
    ligature_sets: Offsets<'a>,
}

impl<'a> LigatureSubst1<'a> {
    pub fn new(table: &LigatureSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            seconds: collect_seconds(table),
            ligature_sets: Offsets::new(table.offset_data(), table.ligature_set_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        apply_ligature_set(
            ctx,
            &self.ligature_sets.get(usize::from(index))?,
            &self.seconds,
        )
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext, coverage: &Coverage) -> bool {
        coverage
            .get(ctx.glyphs[0])
            .and_then(|index| self.ligature_sets.get::<LigatureSet>(index as usize))
            .is_some_and(|set| {
                set.ligatures()
                    .iter()
                    .filter_map(Result::ok)
                    .any(|ligature| ligature_would_apply(&ligature, ctx))
            })
    }
}

impl<'a> SubtableDigests<'a> for LigatureSubst1<'a> {
    const CACHES_COVERAGE: bool = true;

    fn digest_second(&self) -> SetDigest {
        self.seconds.clone()
    }
}

/// HarfBuzz builds the ligature digest from sanitized tables. Bound the walk
/// over unsanitized data and fall back to a full digest past it.
const MAX_LIGATURE_DIGEST_WORK: usize = 16_384;

/// The digest of every ligature's second glyph, or a full digest if a
/// ligature has none or the walk is too long.
fn collect_seconds(table: &LigatureSubstFormat1) -> SetDigest {
    let mut seconds = SetDigest::new();
    let mut remaining_work = MAX_LIGATURE_DIGEST_WORK;
    let sets = table.ligature_sets();
    let Some(work) = remaining_work.checked_sub(sets.len()) else {
        return SetDigest::full();
    };
    remaining_work = work;
    for set in sets.iter().filter_map(Result::ok) {
        let ligatures = set.ligatures();
        let Some(work) = remaining_work.checked_sub(ligatures.len()) else {
            return SetDigest::full();
        };
        remaining_work = work;
        for ligature in ligatures.iter().filter_map(Result::ok) {
            let Some(second) = ligature.component_glyph_ids().first() else {
                return SetDigest::full();
            };
            seconds.add(second.get().into());
        }
    }
    seconds
}

fn ligature_would_apply(ligature: &Ligature, ctx: &WouldApplyContext) -> bool {
    let components = ligature.component_glyph_ids();
    ctx.glyphs.len() == components.len() + 1
        && components
            .iter()
            .zip(&ctx.glyphs[1..])
            .all(|(component, glyph)| GlyphId::from(component.get()) == *glyph)
}

/// Applies the first ligature of `set` that matches.
fn apply_ligature_set(
    ctx: &mut ApplyContext,
    set: &LigatureSet,
    seconds: &SetDigest,
) -> Option<()> {
    let ligatures = set.ligatures();
    // Find the glyph after the current one, to reject ligatures by their
    // first component without matching them.
    let mut second = None;
    let mut unsafe_to = 0;
    if ligatures.len() > 1 {
        let mut iter = SkippingIterator::with_match_fn(ctx, true, Some(match_always));
        if iter.reset(iter.buffer.idx).is_none() {
            iter.buffer.index_failed();
        }
        if iter.next(Some(&mut unsafe_to)) {
            let info = iter.buffer.info.get(iter.index()).likely()?;
            unsafe_to = iter.index() + 1;
            // A skippable next glyph, such as a default ignorable, may not
            // be the second component.
            if iter.may_skip(info) == MaySkip::No {
                second = Some(info.as_glyph());
            }
        }
    }

    let Some(second) = second else {
        return ligatures
            .iter()
            .filter_map(Result::ok)
            .find_map(|ligature| apply_ligature(ctx, &ligature));
    };
    if !seconds.may_have(second.to_u32()) {
        // Every ligature here has a second component and none of them is
        // this glyph; `collect_seconds` gives up on any ligature without
        // one.
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
        return None;
    }
    let mut unsafe_to_concat = false;
    for ligature in ligatures.iter().filter_map(Result::ok) {
        match ligature.component_glyph_ids().first() {
            Some(first) if GlyphId::from(first.get()) != second => unsafe_to_concat = true,
            _ => {
                if apply_ligature(ctx, &ligature).is_some() {
                    if unsafe_to_concat {
                        ctx.buffer
                            .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
                    }
                    return Some(());
                }
            }
        }
    }
    if unsafe_to_concat {
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
    }
    None
}

fn apply_ligature(ctx: &mut ApplyContext, ligature: &Ligature) -> Option<()> {
    let components = ligature.component_glyph_ids();
    let ligature_glyph = ligature.ligature_glyph().into();
    // A ligature of one glyph is an in-place replacement, not a ligation.
    if components.is_empty() {
        message!(
            ctx,
            "replacing glyph at {} (ligature substitution)",
            ctx.buffer.message_idx()
        );
        ctx.replace_glyph(ligature_glyph);
        message!(
            ctx,
            "replaced glyph at {} (ligature substitution)",
            ctx.buffer.message_idx() - 1
        );
        return Some(());
    }
    let matches = |info: &mut GlyphInfo, index: u32| {
        components
            .get(index as usize)
            .is_some_and(|component| match_glyph(info, component.get().to_u32()))
    };
    let mut match_end = 0;
    let mut total_component_count = 0;
    if !match_input(
        ctx,
        components.len() as u16,
        matches,
        &mut match_end,
        Some(&mut total_component_count),
    )? {
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
        return None;
    }
    #[cfg(feature = "tracing")]
    let mut pos = 0;
    #[cfg(feature = "tracing")]
    if ctx.buffer.messaging() {
        pos = ctx.buffer.message_idx();
        let mut msg = String::from("ligating glyphs at ");
        for i in 0..components.len() + 1 {
            let index = ctx.match_positions[i] as usize - ctx.buffer.idx + pos;
            if i > 0 {
                msg.push(',');
            }
            use core::fmt::Write;
            let _ = write!(msg, "{index}");
        }
        ctx.buffer.message(ctx.font, &msg);
    }
    ligate_input(
        ctx,
        components.len() + 1,
        match_end,
        total_component_count,
        ligature_glyph,
    )?;
    message!(ctx, "ligated glyph at {}", pos);
    Some(())
}

/// Reverse chaining single substitution.
pub(crate) struct ReverseChainSubst1<'a> {
    substitutes: &'a [BigEndian<GlyphId16>],
    /// The backtrack coverages, `None` where one can't be read.
    backtrack: CoverageArray<'a>,
    lookahead: CoverageArray<'a>,
}

impl<'a> ReverseChainSubst1<'a> {
    pub fn new(table: &ReverseChainSingleSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            substitutes: table.substitute_glyph_ids(),
            backtrack: CoverageArray::new(table.offset_data(), table.backtrack_coverage_offsets()),
            lookahead: CoverageArray::new(table.offset_data(), table.lookahead_coverage_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        // Reverse chaining lookups can't be nested.
        if ctx.nesting_level_left != MAX_NESTING_LEVEL {
            return None;
        }
        let index = usize::from(index);
        let substitute = self.substitutes.get(index)?.get();

        let mut start_index = 0;
        let mut end_index = 0;
        if match_backtrack(
            ctx,
            self.backtrack.len() as u16,
            |info, index| covered_at(&self.backtrack, index, info),
            &mut start_index,
        ) && match_lookahead(
            ctx,
            self.lookahead.len() as u16,
            |info, index| covered_at(&self.lookahead, index, info),
            ctx.buffer.idx + 1,
            &mut end_index,
        ) {
            message!(
                ctx,
                "replacing glyph at {} (reverse chaining substitution)",
                ctx.buffer.message_idx()
            );
            ctx.buffer
                .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
            if ctx.replace_glyph_inplace(substitute.into()).is_none() {
                ctx.buffer.index_failed();
            }
            message!(
                ctx,
                "replaced glyph at {} (reverse chaining substitution)",
                ctx.buffer.message_idx()
            );
            // The caller moves back to the previous glyph, which keeps a
            // call through a context lookup from skipping one.
            return Some(());
        }
        ctx.buffer
            .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
        None
    }
}

impl<'a> SubtableDigests<'a> for ReverseChainSubst1<'a> {}

/// Whether coverage `index` of `coverages` exists and covers the glyph.
#[inline]
pub(super) fn covered_at(coverages: &CoverageArray, index: u32, info: &GlyphInfo) -> bool {
    coverages.get(index as usize).covers(info.glyph_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use read_fonts::{FontData, FontRead};

    fn assert_full_digest(digest: SetDigest) {
        assert!(digest.may_have(0));
        assert!(digest.may_have(12345));
        assert!(digest.may_have(u16::MAX as u32));
    }

    #[test]
    fn collect_seconds_bails_on_too_many_ligature_sets() {
        let ligature_set_count = MAX_LIGATURE_DIGEST_WORK + 1;
        let mut data = vec![0; 6 + ligature_set_count * 2];
        data[0..2].copy_from_slice(&1u16.to_be_bytes());
        data[4..6].copy_from_slice(&(ligature_set_count as u16).to_be_bytes());

        let lig_subst = LigatureSubstFormat1::read(FontData::new(&data)).unwrap();
        assert_full_digest(collect_seconds(&lig_subst));
    }

    #[test]
    fn collect_seconds_bails_on_too_many_ligatures() {
        let ligature_count = MAX_LIGATURE_DIGEST_WORK;
        let mut data = vec![0; 10 + ligature_count * 2];
        data[0..2].copy_from_slice(&1u16.to_be_bytes());
        data[4..6].copy_from_slice(&1u16.to_be_bytes());
        data[6..8].copy_from_slice(&8u16.to_be_bytes());
        data[8..10].copy_from_slice(&(ligature_count as u16).to_be_bytes());

        let lig_subst = LigatureSubstFormat1::read(FontData::new(&data)).unwrap();
        assert_full_digest(collect_seconds(&lig_subst));
    }
}
