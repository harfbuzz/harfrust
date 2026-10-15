//! Substitution subtables.

use super::coverage::Coverage;
use super::{coverage_index_cached, Offsets, SubtableDigests};
use crate::buffer::{GlyphInfo, GlyphPropsFlags};
use crate::hint::Likely;
use crate::ot::apply::{
    ligate_input, match_always, match_backtrack, match_glyph, match_input, match_lookahead,
    ApplyContext, MappingCache, MaySkip, SkippingIterator, WouldApplyContext,
};
use crate::ot::layout::MAX_NESTING_LEVEL;
use crate::ot::map::OtMap;
use crate::set_digest::SetDigest;
use alloc::boxed::Box;
use read_fonts::tables::gsub::{
    AlternateSet, AlternateSubstFormat1, Ligature, LigatureSet, LigatureSubstFormat1,
    MultipleSubstFormat1, ReverseChainSingleSubstFormat1, Sequence, SingleSubstFormat1,
    SingleSubstFormat2,
};
use read_fonts::types::{BigEndian, GlyphId, GlyphId16};

/// Whether a single glyph `ctx` would be substituted by a subtable with
/// this coverage.
fn covers_single(coverage: &Coverage, ctx: &WouldApplyContext) -> bool {
    ctx.glyphs.len() == 1 && coverage.covers(ctx.glyphs[0])
}

/// Single substitution by a fixed delta.
pub(crate) struct SingleSubst1<'a> {
    coverage: Coverage<'a>,
    delta: i16,
}

impl<'a> SingleSubst1<'a> {
    pub fn new(table: &SingleSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            delta: table.delta_glyph_id(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        self.coverage.get(glyph)?;
        // The delta wraps around within 16 bits.
        let substitute = (glyph.to_u32() as i32 + i32::from(self.delta)) as u16;
        ctx.replace_glyph(substitute.into());
        Some(())
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        covers_single(&self.coverage, ctx)
    }
}

impl<'a> SubtableDigests<'a> for SingleSubst1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }
}

/// Single substitution from an array of substitutes.
pub(crate) struct SingleSubst2<'a> {
    coverage: Coverage<'a>,
    substitutes: &'a [BigEndian<GlyphId16>],
}

impl<'a> SingleSubst2<'a> {
    pub fn new(table: &SingleSubstFormat2<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            substitutes: table.substitute_glyph_ids(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let index = self.coverage.get(glyph)? as usize;
        let substitute = self.substitutes.get(index)?.get();
        ctx.replace_glyph(substitute.into());
        Some(())
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        covers_single(&self.coverage, ctx)
    }
}

impl<'a> SubtableDigests<'a> for SingleSubst2<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }
}

/// Multiple substitution: one glyph into a sequence.
pub(crate) struct MultipleSubst1<'a> {
    coverage: Coverage<'a>,
    sequences: Offsets<'a>,
}

impl<'a> MultipleSubst1<'a> {
    pub fn new(table: &MultipleSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            sequences: Offsets::new(table.offset_data(), table.sequence_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let index = self.coverage.get(glyph)? as usize;
        let sequence: Sequence = self.sequences.get(index)?;
        let substitutes = sequence.substitute_glyph_ids();
        match substitutes {
            // The spec disallows empty sequences, but Uniscribe deletes the
            // glyph. https://github.com/harfbuzz/harfbuzz/issues/253
            [] => ctx.buffer.delete_glyph(),
            // A single substitute is an in-place replacement, not a
            // multiplication.
            [substitute] => ctx.replace_glyph(substitute.get().into()),
            _ => {
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
            }
        }
        Some(())
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        covers_single(&self.coverage, ctx)
    }
}

impl<'a> SubtableDigests<'a> for MultipleSubst1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }
}

/// Alternate substitution, chosen by the feature value.
pub(crate) struct AlternateSubst1<'a> {
    coverage: Coverage<'a>,
    alternate_sets: Offsets<'a>,
}

impl<'a> AlternateSubst1<'a> {
    pub fn new(table: &AlternateSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            alternate_sets: Offsets::new(table.offset_data(), table.alternate_set_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let index = self.coverage.get(glyph)? as usize;
        apply_alternate_set(ctx, &self.alternate_sets.get(index)?)
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        covers_single(&self.coverage, ctx)
    }
}

impl<'a> SubtableDigests<'a> for AlternateSubst1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }
}

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
    ctx.replace_glyph(alternates.get(index as usize)?.get().into());
    Some(())
}

/// Ligature substitution.
pub(crate) struct LigatureSubst1<'a> {
    coverage: Coverage<'a>,
    coverage_cache: Option<Box<MappingCache>>,
    /// The second components of all ligatures.
    seconds: SetDigest,
    ligature_sets: Offsets<'a>,
}

impl<'a> LigatureSubst1<'a> {
    pub fn new(table: &LigatureSubstFormat1<'a>, with_caches: bool) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            coverage_cache: with_caches.then(|| Box::new(MappingCache::new())),
            seconds: collect_seconds(table),
            ligature_sets: Offsets::new(table.offset_data(), table.ligature_set_offsets()),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let index = match &self.coverage_cache {
            Some(cache) => coverage_index_cached(|gid| self.coverage.get(gid), glyph, cache)?,
            None => self.coverage.get(glyph)?,
        };
        apply_ligature_set(ctx, &self.ligature_sets.get(index as usize)?, &self.seconds)
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        self.coverage
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
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

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
        ctx.replace_glyph(ligature_glyph);
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
    ligate_input(
        ctx,
        components.len() + 1,
        match_end,
        total_component_count,
        ligature_glyph,
    )
}

/// Reverse chaining single substitution.
pub(crate) struct ReverseChainSubst1<'a> {
    coverage: Coverage<'a>,
    substitutes: &'a [BigEndian<GlyphId16>],
    /// The backtrack coverages, `None` where one can't be read.
    backtrack: Box<[Coverage<'a>]>,
    lookahead: Box<[Coverage<'a>]>,
}

impl<'a> ReverseChainSubst1<'a> {
    pub fn new(table: &ReverseChainSingleSubstFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            substitutes: table.substitute_glyph_ids(),
            backtrack: table
                .backtrack_coverages()
                .iter()
                .map(Coverage::new)
                .collect(),
            lookahead: table
                .lookahead_coverages()
                .iter()
                .map(Coverage::new)
                .collect(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        // Reverse chaining lookups can't be nested.
        if ctx.nesting_level_left != MAX_NESTING_LEVEL {
            return None;
        }
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let index = self.coverage.get(glyph)? as usize;
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
            ctx.buffer
                .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
            if ctx.replace_glyph_inplace(substitute.into()).is_none() {
                ctx.buffer.index_failed();
            }
            // The caller moves back to the previous glyph, which keeps a
            // call through a context lookup from skipping one.
            return Some(());
        }
        ctx.buffer
            .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
        None
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        covers_single(&self.coverage, ctx)
    }
}

impl<'a> SubtableDigests<'a> for ReverseChainSubst1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }
}

/// Whether coverage `index` of `coverages` exists and covers the glyph.
#[inline]
pub(super) fn covered_at(coverages: &[Coverage], index: u32, info: &GlyphInfo) -> bool {
    coverages
        .get(index as usize)
        .is_some_and(|coverage| coverage.covers(info.glyph_id))
}
