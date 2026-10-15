//! Contextual and chained contextual subtables, shared by GSUB and GPOS.

use super::coverage::{ClassDef, Coverage};
use super::gsub::covered_at;
use super::{covered_cached, glyph_class_cached, Offsets, SubtableDigests};
use crate::buffer::GlyphInfo;
use crate::hint::Likely;
use crate::ot::apply::{
    apply_lookup, match_always, match_backtrack, match_glyph, match_input, match_lookahead,
    ApplyContext, BinaryCache, MappingCache, MaySkip, RuleSetDigest, SkippingIterator,
    WouldApplyContext,
};
use crate::set_digest::SetDigest;
use alloc::boxed::Box;
use read_fonts::tables::layout::{
    ChainedClassSequenceRuleSet, ChainedSequenceContextFormat1, ChainedSequenceContextFormat2,
    ChainedSequenceContextFormat3, ChainedSequenceRule, ChainedSequenceRuleSet,
    ClassSequenceRuleSet, SequenceContextFormat1, SequenceContextFormat2, SequenceContextFormat3,
    SequenceLookupRecord, SequenceRule, SequenceRuleSet,
};
use read_fonts::types::{BigEndian, FixedSize, GlyphId, Nullable, Offset16};
use read_fonts::{FontData, FontRead, Offset};

/// Matches a glyph against a rule value: a glyph ID or a class. The rule
/// walks are generic over it so that matching inlines; `would_apply`, which
/// is cold, takes it by reference.
type MatchFn<'a> = &'a dyn Fn(&mut GlyphInfo, u32) -> bool;

/// Offsets to rule sets, which are null for glyphs or classes without rules.
type RuleSets<'a> = Offsets<'a, Nullable<Offset16>>;

/// The class of `gid`, remembered in `cache` if there is one.
fn cached_class(class_def: &ClassDef, gid: GlyphId, cache: Option<&MappingCache>) -> u16 {
    match cache {
        Some(cache) => glyph_class_cached(|gid| class_def.get(gid), gid, cache),
        None => class_def.get(gid),
    }
}

/// Calls `f` with each rule set present in `sets`, stopping with `None` at
/// one that can't be read.
fn for_each_rule_set<'a, T: FontRead<'a, Args = ()>>(
    sets: &RuleSets<'a>,
    mut f: impl FnMut(T) -> Option<()>,
) -> Option<()> {
    for index in 0..sets.len() {
        if !sets.is_null(index) {
            f(sets.get(index)?)?;
        }
    }
    Some(())
}

/// The digest of the first input value of each rule set's rules.
fn rule_set_digests<'a, T: FontRead<'a, Args = ()>>(
    sets: &RuleSets<'a>,
    rules_of: impl Fn(&T) -> Rules<'a>,
    first_input: fn(&FontData) -> Option<u16>,
    min_rule_size: usize,
) -> Box<[RuleSetDigest]> {
    (0..sets.len())
        .map(|index| {
            if sets.is_null(index) {
                return RuleSetDigest::default();
            }
            let Some(set) = sets.get::<T>(index) else {
                return RuleSetDigest::full();
            };
            let rules = rules_of(&set);
            let mut digest = RuleSetDigest::default();
            for index in 0..rules.len() {
                let Some(value) = rules
                    .data_at(index, min_rule_size)
                    .and_then(|data| first_input(&data))
                else {
                    return RuleSetDigest::full();
                };
                digest.add(value);
            }
            digest
        })
        .collect()
}

/// Context by glyph sequences.
pub(crate) struct Context1<'a> {
    coverage: Coverage<'a>,
    rule_sets: RuleSets<'a>,
}

impl<'a> Context1<'a> {
    pub fn new(table: &SequenceContextFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            rule_sets: RuleSets::new(table.offset_data(), table.seq_rule_set_offsets()),
        })
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        let set: SequenceRuleSet = self.rule_sets.get(index)?;
        Some(Rules::new(set.offset_data(), set.seq_rule_offsets()))
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let rules = self.rule_set(self.coverage.get(glyph)? as usize)?;
        apply_context_rules(ctx, rules, &match_glyph, None, &|_| 0)
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        self.coverage
            .get(ctx.glyphs[0])
            .and_then(|index| self.rule_set(index as usize))
            .is_some_and(|rules| would_apply_rules(ctx, rules, &match_glyph))
    }
}

impl<'a> SubtableDigests<'a> for Context1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        let mut digest = SetDigest::new();
        for_each_rule_set(&self.rule_sets, |set: SequenceRuleSet| {
            for rule in set.seq_rules().iter() {
                digest.add(rule.ok()?.input_sequence().first()?.get().to_u32());
            }
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }
}

/// Context by glyph class sequences.
pub(crate) struct Context2<'a> {
    coverage: Coverage<'a>,
    coverage_cache: BinaryCache,
    class_def: ClassDef<'a>,
    rule_sets: RuleSets<'a>,
    /// The first input class of each rule set's rules.
    rule_set_digests: Box<[RuleSetDigest]>,
}

impl<'a> Context2<'a> {
    pub fn new(table: &SequenceContextFormat2<'a>) -> Option<Self> {
        let rule_sets = RuleSets::new(table.offset_data(), table.class_seq_rule_set_offsets());
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            coverage_cache: BinaryCache::new(),
            class_def: ClassDef::new(table.class_def()),
            rule_set_digests: rule_set_digests(
                &rule_sets,
                |set: &ClassSequenceRuleSet| {
                    Rules::new(set.offset_data(), set.class_seq_rule_offsets())
                },
                plain_rule_first_input,
                SequenceRule::MIN_SIZE,
            ),
            rule_sets,
        })
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        let set: ClassSequenceRuleSet = self.rule_sets.get(index)?;
        Some(Rules::new(set.offset_data(), set.class_seq_rule_offsets()))
    }

    #[inline]
    fn class(&self, gid: GlyphId) -> u16 {
        self.class_def.get(gid)
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        covered_cached(|gid| self.coverage.get(gid), glyph, &self.coverage_cache)?;
        let index = self.class(glyph) as usize;
        apply_context_rules(
            ctx,
            self.rule_set(index)?,
            &|info, value| u32::from(self.class(info.as_glyph())) == value,
            self.rule_set_digests.get(index).copied(),
            &|info| self.class(info.as_glyph()),
        )
    }

    /// Applies, keeping each glyph's class in its syllable byte.
    pub fn apply_with_class_cache(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        covered_cached(|gid| self.coverage.get(gid), glyph, &self.coverage_cache)?;
        let class = |gid| self.class(gid);
        let info = ctx.buffer.info.get_mut(ctx.buffer.idx).likely()?;
        let index = syllable_class(&class, info) as usize;
        apply_context_rules(
            ctx,
            self.rule_set(index)?,
            &|info, value| u32::from(syllable_class(&class, info)) == value,
            self.rule_set_digests.get(index).copied(),
            &|info| syllable_class(&class, info),
        )
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let matches = |info: &mut GlyphInfo, value| u32::from(self.class(info.as_glyph())) == value;
        self.rule_set(self.class(ctx.glyphs[0]) as usize)
            .is_some_and(|rules| would_apply_rules(ctx, rules, &matches))
    }
}

impl<'a> SubtableDigests<'a> for Context2<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        let class_def = &self.class_def;
        if class_def.is_none() {
            return SetDigest::full();
        }
        let mut digest = SetDigest::new();
        for_each_rule_set(&self.rule_sets, |set: ClassSequenceRuleSet| {
            for rule in set.class_seq_rules().iter() {
                let second = rule.ok()?.input_sequence().first()?.get();
                class_def.add_class_to(&mut digest, second);
            }
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }

    fn cache_cost(&self) -> u32 {
        self.class_def.cost()
    }
}

/// Context by a coverage per glyph.
pub(crate) struct Context3<'a> {
    coverage: Coverage<'a>,
    /// The coverages of the glyphs after the first.
    inputs: Box<[Coverage<'a>]>,
    records: &'a [SequenceLookupRecord],
}

impl<'a> Context3<'a> {
    pub fn new(table: &SequenceContextFormat3<'a>) -> Option<Self> {
        let coverages = table.coverages();
        Some(Self {
            coverage: Coverage::required(coverages.get(0))?,
            inputs: coverages.iter().skip(1).map(Coverage::new).collect(),
            records: table.seq_lookup_records(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        self.coverage.get(glyph)?;
        let mut match_end = 0;
        if match_input(
            ctx,
            self.inputs.len() as u16,
            |info, index| covered_at(&self.inputs, index, info),
            &mut match_end,
            None,
        )? {
            ctx.buffer
                .unsafe_to_break(Some(ctx.buffer.idx), Some(match_end));
            apply_lookup(ctx, self.inputs.len(), match_end, self.records);
            Some(())
        } else {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
            None
        }
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        // The first coverage is the first glyph, which the caller has
        // already matched.
        ctx.glyphs.len() == self.inputs.len() + 1
            && self
                .inputs
                .iter()
                .zip(&ctx.glyphs[1..])
                .all(|(coverage, glyph)| coverage.covers(*glyph))
    }
}

impl<'a> SubtableDigests<'a> for Context3<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        coverage_digest(self.inputs.first())
    }
}

fn coverage_digest(coverage: Option<&Coverage>) -> SetDigest {
    coverage.map_or_else(SetDigest::full, Coverage::digest)
}

/// Chained context by glyph sequences.
pub(crate) struct ChainContext1<'a> {
    coverage: Coverage<'a>,
    rule_sets: RuleSets<'a>,
}

impl<'a> ChainContext1<'a> {
    pub fn new(table: &ChainedSequenceContextFormat1<'a>) -> Option<Self> {
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            rule_sets: RuleSets::new(table.offset_data(), table.chained_seq_rule_set_offsets()),
        })
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        let set: ChainedSequenceRuleSet = self.rule_sets.get(index)?;
        Some(Rules::new(
            set.offset_data(),
            set.chained_seq_rule_offsets(),
        ))
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        let rules = self.rule_set(self.coverage.get(glyph)? as usize)?;
        let matchers = ChainMatchers {
            backtrack: &match_glyph,
            input: &match_glyph,
            lookahead: &match_glyph,
        };
        apply_chain_rules(ctx, rules, &matchers, None, &|_| 0)
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        self.coverage
            .get(ctx.glyphs[0])
            .and_then(|index| self.rule_set(index as usize))
            .is_some_and(|rules| would_apply_chain_rules(ctx, rules, &match_glyph))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext1<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        let mut digest = SetDigest::new();
        for_each_rule_set(&self.rule_sets, |set: ChainedSequenceRuleSet| {
            for rule in set.chained_seq_rules().iter() {
                let rule = rule.ok()?;
                let input = rule.input_sequence();
                let second = input
                    .first()
                    .or_else(|| rule.lookahead_sequence().first())?;
                digest.add(second.get().to_u32());
            }
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }
}

/// Chained context by glyph class sequences.
pub(crate) struct ChainContext2<'a> {
    coverage: Coverage<'a>,
    coverage_cache: BinaryCache,
    backtrack: ClassDef<'a>,
    input: ClassDef<'a>,
    lookahead: ClassDef<'a>,
    class_caches: Option<Box<ChainClassCaches>>,
    rule_sets: RuleSets<'a>,
    /// The first input class of each rule set's rules.
    rule_set_digests: Box<[RuleSetDigest]>,
}

/// Caches of the input and lookahead classes of a glyph.
struct ChainClassCaches {
    input: MappingCache,
    lookahead: MappingCache,
}

impl<'a> ChainContext2<'a> {
    pub fn new(table: &ChainedSequenceContextFormat2<'a>, with_caches: bool) -> Option<Self> {
        let rule_sets = RuleSets::new(
            table.offset_data(),
            table.chained_class_seq_rule_set_offsets(),
        );
        Some(Self {
            coverage: Coverage::required(table.coverage())?,
            coverage_cache: BinaryCache::new(),
            backtrack: ClassDef::new(table.backtrack_class_def()),
            input: ClassDef::new(table.input_class_def()),
            lookahead: ClassDef::new(table.lookahead_class_def()),
            class_caches: with_caches.then(|| {
                Box::new(ChainClassCaches {
                    input: MappingCache::new(),
                    lookahead: MappingCache::new(),
                })
            }),
            rule_set_digests: rule_set_digests(
                &rule_sets,
                |set: &ChainedClassSequenceRuleSet| {
                    Rules::new(set.offset_data(), set.chained_class_seq_rule_offsets())
                },
                chain_rule_first_input,
                ChainedSequenceRule::MIN_SIZE,
            ),
            rule_sets,
        })
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        let set: ChainedClassSequenceRuleSet = self.rule_sets.get(index)?;
        Some(Rules::new(
            set.offset_data(),
            set.chained_class_seq_rule_offsets(),
        ))
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        covered_cached(|gid| self.coverage.get(gid), glyph, &self.coverage_cache)?;
        let caches = self.class_caches.as_deref();
        let input_class = |gid| cached_class(&self.input, gid, caches.map(|c| &c.input));
        let lookahead_class =
            |gid| cached_class(&self.lookahead, gid, caches.map(|c| &c.lookahead));
        let index = input_class(glyph) as usize;
        let matchers = ChainMatchers {
            backtrack: &|info: &mut GlyphInfo, value| {
                u32::from(self.backtrack.get(info.as_glyph())) == value
            },
            input: &|info: &mut GlyphInfo, value| u32::from(input_class(info.as_glyph())) == value,
            lookahead: &|info: &mut GlyphInfo, value| {
                u32::from(lookahead_class(info.as_glyph())) == value
            },
        };
        apply_chain_rules(
            ctx,
            self.rule_set(index)?,
            &matchers,
            self.rule_set_digests.get(index).copied(),
            &|info| input_class(info.as_glyph()),
        )
    }

    /// Applies, keeping each glyph's input class in the high nibble of its
    /// syllable byte and its lookahead class in the low one.
    pub fn apply_with_class_cache(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        covered_cached(|gid| self.coverage.get(gid), glyph, &self.coverage_cache)?;
        let input_class = |gid| self.input.get(gid);
        let lookahead_class = |gid| self.lookahead.get(gid);
        let info = ctx.buffer.info.get_mut(ctx.buffer.idx).likely()?;
        let index = syllable_class_high(&input_class, info) as usize;
        let matchers = ChainMatchers {
            backtrack: &|info: &mut GlyphInfo, value| {
                u32::from(self.backtrack.get(info.as_glyph())) == value
            },
            input: &|info: &mut GlyphInfo, value| {
                u32::from(syllable_class_high(&input_class, info)) == value
            },
            lookahead: &|info: &mut GlyphInfo, value| {
                u32::from(syllable_class_low(&lookahead_class, info)) == value
            },
        };
        apply_chain_rules(
            ctx,
            self.rule_set(index)?,
            &matchers,
            self.rule_set_digests.get(index).copied(),
            &|info| syllable_class_high(&input_class, info),
        )
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let matches =
            |info: &mut GlyphInfo, value| u32::from(self.input.get(info.as_glyph())) == value;
        self.rule_set(self.input.get(ctx.glyphs[0]) as usize)
            .is_some_and(|rules| would_apply_chain_rules(ctx, rules, &matches))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext2<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        let (input, lookahead) = (&self.input, &self.lookahead);
        if input.is_none() || lookahead.is_none() {
            return SetDigest::full();
        }
        let mut digest = SetDigest::new();
        for_each_rule_set(&self.rule_sets, |set: ChainedClassSequenceRuleSet| {
            for rule in set.chained_class_seq_rules().iter() {
                let rule = rule.ok()?;
                if let Some(second) = rule.input_sequence().first() {
                    input.add_class_to(&mut digest, second.get());
                } else {
                    let second = rule.lookahead_sequence().first()?;
                    lookahead.add_class_to(&mut digest, second.get());
                }
            }
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }

    fn cache_cost(&self) -> u32 {
        self.input.cost() + self.lookahead.cost()
    }
}

/// Chained context by a coverage per glyph.
pub(crate) struct ChainContext3<'a> {
    coverage: Coverage<'a>,
    /// The coverages of the input glyphs after the first.
    inputs: Box<[Coverage<'a>]>,
    backtrack: Box<[Coverage<'a>]>,
    lookahead: Box<[Coverage<'a>]>,
    records: &'a [SequenceLookupRecord],
}

impl<'a> ChainContext3<'a> {
    pub fn new(table: &ChainedSequenceContextFormat3<'a>) -> Option<Self> {
        let inputs = table.input_coverages();
        Some(Self {
            coverage: Coverage::required(inputs.get(0))?,
            inputs: inputs.iter().skip(1).map(Coverage::new).collect(),
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
            records: table.seq_lookup_records(),
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.as_glyph();
        self.coverage.get(glyph)?;
        let mut end_index = ctx.buffer.idx;
        let mut match_end = 0;
        let input_matches = match_input(
            ctx,
            self.inputs.len() as u16,
            |info, index| covered_at(&self.inputs, index, info),
            &mut match_end,
            None,
        )?;
        if input_matches {
            end_index = match_end;
        }
        if !(input_matches
            && match_lookahead(
                ctx,
                self.lookahead.len() as u16,
                |info, index| covered_at(&self.lookahead, index, info),
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
            self.backtrack.len() as u16,
            |info, index| covered_at(&self.backtrack, index, info),
            &mut start_index,
        ) {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
            return None;
        }
        ctx.buffer
            .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
        apply_lookup(ctx, self.inputs.len(), match_end, self.records);
        Some(())
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        (!ctx.zero_context || (self.backtrack.is_empty() && self.lookahead.is_empty()))
            && ctx.glyphs.len() == self.inputs.len() + 1
            && self
                .inputs
                .iter()
                .zip(&ctx.glyphs[1..])
                .all(|(coverage, glyph)| coverage.covers(*glyph))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext3<'a> {
    fn coverage(&self) -> &Coverage<'a> {
        &self.coverage
    }

    fn digest_second(&self) -> SetDigest {
        if self.inputs.is_empty() {
            coverage_digest(self.lookahead.first())
        } else {
            coverage_digest(self.inputs.first())
        }
    }
}

/// The class of a glyph, kept in its syllable byte once known; 255 means
/// unknown.
#[inline]
fn syllable_class(class_def: &impl Fn(GlyphId) -> u16, info: &mut GlyphInfo) -> u16 {
    let cached = u16::from(info.syllable());
    if cached < 255 {
        return cached;
    }
    let class = class_def(info.as_glyph());
    if class < 255 {
        info.set_syllable(class as u8);
    }
    class
}

/// The class of a glyph, kept in the low nibble of its syllable byte once
/// known; 15 means unknown.
#[inline]
fn syllable_class_low(class_def: &impl Fn(GlyphId) -> u16, info: &mut GlyphInfo) -> u16 {
    let cached = u16::from(info.syllable() & 0x0F);
    if cached < 15 {
        return cached;
    }
    let class = class_def(info.as_glyph());
    if class < 15 {
        info.set_syllable((info.syllable() & 0xF0) | class as u8);
    }
    class
}

/// The class of a glyph, kept in the high nibble of its syllable byte once
/// known; 15 means unknown.
#[inline]
fn syllable_class_high(class_def: &impl Fn(GlyphId) -> u16, info: &mut GlyphInfo) -> u16 {
    let cached = u16::from(info.syllable() >> 4);
    if cached < 15 {
        return cached;
    }
    let class = class_def(info.as_glyph());
    if class < 15 {
        info.set_syllable((info.syllable() & 0x0F) | ((class as u8) << 4));
    }
    class
}

/// The rules of a rule set, read on demand from their offsets.
#[derive(Clone, Copy)]
struct Rules<'a> {
    data: FontData<'a>,
    offsets: &'a [BigEndian<Offset16>],
}

impl<'a> Rules<'a> {
    fn new(data: FontData<'a>, offsets: &'a [BigEndian<Offset16>]) -> Self {
        Self { data, offsets }
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    /// The data of rule `index`, if it holds at least `min_size` bytes, as
    /// reading the rule table would require.
    fn data_at(&self, index: usize, min_size: usize) -> Option<FontData<'a>> {
        let data = self
            .data
            .split_off(self.offsets.get(index)?.get().to_usize())?;
        (data.len() >= min_size).then_some(data)
    }

    fn plain(&self, index: usize) -> Option<FontData<'a>> {
        self.data_at(index, SequenceRule::MIN_SIZE)
    }

    fn chain(&self, index: usize) -> Option<FontData<'a>> {
        self.data_at(index, ChainedSequenceRule::MIN_SIZE)
    }
}

/// A rule's fields, parsed in one pass. Glyph IDs and classes are both
/// read as plain 16-bit values.
#[derive(Clone, Copy, Default)]
struct ParsedRule<'a> {
    backtrack: &'a [BigEndian<u16>],
    input: &'a [BigEndian<u16>],
    lookahead: &'a [BigEndian<u16>],
    records: &'a [SequenceLookupRecord],
}

impl<'a> ParsedRule<'a> {
    /// Parses a sequence rule or class sequence rule; an unreadable one is
    /// empty.
    fn plain(data: FontData<'a>) -> Self {
        Self::try_plain(data).unwrap_or_default()
    }

    fn try_plain(data: FontData<'a>) -> Option<Self> {
        let glyph_count = usize::from(data.read_at::<u16>(0).ok()?);
        let record_count = usize::from(data.read_at::<u16>(2).ok()?);
        let input_end = 4 + glyph_count.saturating_sub(1) * u16::RAW_BYTE_LEN;
        let records_end = input_end + record_count * SequenceLookupRecord::RAW_BYTE_LEN;
        Some(Self {
            input: data.read_array(4..input_end).ok()?,
            records: data.read_array(input_end..records_end).ok()?,
            ..Self::default()
        })
    }

    /// Parses a chained sequence rule or chained class sequence rule; an
    /// unreadable one is empty.
    fn chain(data: FontData<'a>) -> Self {
        Self::try_chain(data).unwrap_or_default()
    }

    fn try_chain(data: FontData<'a>) -> Option<Self> {
        let backtrack_count = usize::from(data.read_at::<u16>(0).ok()?);
        let backtrack_end = 2 + backtrack_count * u16::RAW_BYTE_LEN;
        let input_count = usize::from(data.read_at::<u16>(backtrack_end).ok()?);
        let input_end = backtrack_end + 2 + input_count.saturating_sub(1) * u16::RAW_BYTE_LEN;
        let lookahead_count = usize::from(data.read_at::<u16>(input_end).ok()?);
        let lookahead_end = input_end + 2 + lookahead_count * u16::RAW_BYTE_LEN;
        let record_count = usize::from(data.read_at::<u16>(lookahead_end).ok()?);
        let records_end = lookahead_end + 2 + record_count * SequenceLookupRecord::RAW_BYTE_LEN;
        Some(Self {
            backtrack: data.read_array(2..backtrack_end).ok()?,
            input: data.read_array(backtrack_end + 2..input_end).ok()?,
            lookahead: data.read_array(input_end + 2..lookahead_end).ok()?,
            records: data.read_array(lookahead_end + 2..records_end).ok()?,
        })
    }

    /// Matches the input sequence and applies the lookup records, ignoring
    /// any backtrack and lookahead.
    fn apply_plain(
        &self,
        ctx: &mut ApplyContext,
        matches: &impl Fn(&mut GlyphInfo, u32) -> bool,
    ) -> Option<()> {
        let input = self.input;
        let mut match_end = 0;
        if match_input(
            ctx,
            input.len() as u16,
            |info, index| sequence_matches(input, index, info, matches),
            &mut match_end,
            None,
        )? {
            ctx.buffer
                .unsafe_to_break(Some(ctx.buffer.idx), Some(match_end));
            apply_lookup(ctx, input.len(), match_end, self.records);
            return Some(());
        }
        None
    }

    /// Matches the input, lookahead and backtrack sequences in that order
    /// and applies the lookup records.
    fn apply_chain(
        &self,
        ctx: &mut ApplyContext,
        matchers: &ChainMatchers<impl Matcher, impl Matcher, impl Matcher>,
    ) -> Option<()> {
        let mut end_index = ctx.buffer.idx;
        let mut match_end = 0;
        if !match_input(
            ctx,
            self.input.len() as u16,
            |info, index| sequence_matches(self.input, index, info, &matchers.input),
            &mut match_end,
            None,
        )? {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_index));
            return None;
        }
        end_index = match_end;
        if !match_lookahead(
            ctx,
            self.lookahead.len() as u16,
            |info, index| sequence_matches(self.lookahead, index, info, &matchers.lookahead),
            match_end,
            &mut end_index,
        ) {
            ctx.buffer
                .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_index));
            return None;
        }
        let mut start_index = ctx.buffer.out_len;
        if !match_backtrack(
            ctx,
            self.backtrack.len() as u16,
            |info, index| sequence_matches(self.backtrack, index, info, &matchers.backtrack),
            &mut start_index,
        ) {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(start_index), Some(end_index));
            return None;
        }
        ctx.buffer
            .unsafe_to_break_from_outbuffer(Some(start_index), Some(end_index));
        apply_lookup(ctx, self.input.len(), match_end, self.records);
        Some(())
    }
}

/// Whether the glyph matches value `index` of `sequence`.
fn sequence_matches(
    sequence: &[BigEndian<u16>],
    index: u32,
    info: &mut GlyphInfo,
    matches: &impl Fn(&mut GlyphInfo, u32) -> bool,
) -> bool {
    sequence
        .get(index as usize)
        .is_some_and(|value| matches(info, u32::from(value.get())))
}

/// How a chain rule matches each of its three sequences.
struct ChainMatchers<B, I, L> {
    backtrack: B,
    input: I,
    lookahead: L,
}

/// Matches a glyph against a rule value in a rule walk.
trait Matcher: Fn(&mut GlyphInfo, u32) -> bool {}

impl<F: Fn(&mut GlyphInfo, u32) -> bool> Matcher for F {}

/// The first input value of a sequence rule: the second glyph of a match.
fn plain_rule_first_input(data: &FontData) -> Option<u16> {
    (data.read_at::<u16>(0).ok()? > 1)
        .then(|| data.read_at(4).ok())
        .flatten()
}

/// The second input value of a sequence rule.
fn plain_rule_second_input(data: &FontData) -> Option<u16> {
    (data.read_at::<u16>(0).ok()? > 2)
        .then(|| data.read_at(6).ok())
        .flatten()
}

/// The first input value of a chained rule, after the backtrack sequence.
fn chain_rule_first_input(data: &FontData) -> Option<u16> {
    ChainRuleProbe::new(*data)
        .filter(|probe| probe.input_count > 1)?
        .input(0)
}

/// Reads the values of a chained rule that the pre-match needs, without
/// parsing the rule.
struct ChainRuleProbe<'a> {
    data: FontData<'a>,
    count_pos: usize,
    input_end: usize,
    input_count: u16,
}

impl<'a> ChainRuleProbe<'a> {
    fn new(data: FontData<'a>) -> Option<Self> {
        let backtrack_count = usize::from(data.read_at::<u16>(0).ok()?);
        let count_pos = 2 + backtrack_count * u16::RAW_BYTE_LEN;
        let input_count: u16 = data.read_at(count_pos).ok()?;
        let input_end =
            count_pos + 2 + usize::from(input_count).saturating_sub(1) * u16::RAW_BYTE_LEN;
        Some(Self {
            data,
            count_pos,
            input_end,
            input_count,
        })
    }

    /// The length of the input sequence, including the first glyph.
    fn input_len(&self) -> usize {
        usize::from(self.input_count).max(1)
    }

    fn input(&self, index: usize) -> Option<u16> {
        self.data
            .read_at(self.count_pos + 2 + index * u16::RAW_BYTE_LEN)
            .ok()
    }

    fn lookahead_len(&self) -> usize {
        usize::from(self.data.read_at::<u16>(self.input_end).unwrap_or(0))
    }

    fn lookahead(&self, index: usize) -> Option<u16> {
        self.data
            .read_at(self.input_end + 2 + index * u16::RAW_BYTE_LEN)
            .ok()
    }
}

/// Whether the glyph at `idx` matches `value`; a glyph outside the buffer
/// never matches.
fn matches_at(
    info: &mut [GlyphInfo],
    idx: usize,
    value: u16,
    matches: &impl Fn(&mut GlyphInfo, u32) -> bool,
) -> bool {
    info.get_mut(idx)
        .likely()
        .is_some_and(|info| matches(info, u32::from(value)))
}

/// The glyphs after the current one that rules are pre-matched against.
struct Lookahead {
    /// The glyph after the current one, and the end of the range a failed
    /// match on it makes unsafe to concatenate.
    first: usize,
    unsafe_to1: usize,
    /// The glyph after that, if there is one.
    second: Option<usize>,
    unsafe_to2: usize,
}

/// What the pre-match found after the current glyph.
enum Prematch {
    /// The rules are matched against these glyphs.
    Glyphs(Lookahead),
    /// A skippable glyph follows, so every rule must be tried in full.
    TryAll,
    /// Nothing follows; `unsafe_to` ends the range a match would depend on.
    End { unsafe_to: usize },
    /// The rule set digest rules out every rule.
    Rejected,
}

/// Finds the glyphs that rules are pre-matched against. The context
/// iterator, unlike the input one, doesn't skip default ignorables.
/// https://github.com/harfbuzz/harfbuzz/issues/4813
#[inline]
fn prematch(
    ctx: &mut ApplyContext,
    has_rules: bool,
    digest: Option<RuleSetDigest>,
    first_value: &impl Fn(&mut GlyphInfo) -> u16,
) -> Option<Prematch> {
    let mut iter = SkippingIterator::with_match_fn(ctx, true, Some(match_always));
    if iter.reset(iter.buffer.idx).is_none() {
        iter.buffer.index_failed();
    }
    iter.set_glyph_data(0);
    let mut unsafe_to1 = 0;
    if !iter.next(Some(&mut unsafe_to1)) {
        return Some(Prematch::End {
            unsafe_to: unsafe_to1,
        });
    }
    let first = iter.index();
    if iter.may_skip(iter.buffer.info.get(first).likely()?) != MaySkip::No {
        return Some(Prematch::TryAll);
    }
    let unsafe_to1 = first + 1;
    if has_rules
        && digest.is_some_and(|digest| {
            !digest.is_full()
                && iter
                    .buffer
                    .info
                    .get_mut(first)
                    .is_some_and(|info| !digest.may_have(first_value(info)))
        })
    {
        iter.buffer
            .unsafe_to_concat(Some(iter.buffer.idx), Some(unsafe_to1));
        return Some(Prematch::Rejected);
    }
    let mut second = None;
    let mut unsafe_to2 = 0;
    if iter.next(None) {
        let index = iter.index();
        if iter.may_skip(iter.buffer.info.get(index).likely()?) != MaySkip::No {
            return Some(Prematch::TryAll);
        }
        second = Some(index);
        unsafe_to2 = index + 1;
    }
    Some(Prematch::Glyphs(Lookahead {
        first,
        unsafe_to1,
        second,
        unsafe_to2,
    }))
}

/// Applies the first rule of a sequence rule set that matches.
///
/// Rules are pre-matched on their first two input values, which rejects most
/// of them without parsing. HarfBuzz skips the pre-match for small rule sets;
/// here it is faster at every size.
#[inline(never)]
fn apply_context_rules(
    ctx: &mut ApplyContext,
    rules: Rules,
    matches: &impl Matcher,
    digest: Option<RuleSetDigest>,
    first_value: &impl Fn(&mut GlyphInfo) -> u16,
) -> Option<()> {
    let lookahead = match prematch(ctx, rules.len() != 0, digest, first_value)? {
        Prematch::Glyphs(lookahead) => lookahead,
        Prematch::Rejected => return None,
        Prematch::TryAll => {
            return (0..rules.len())
                .filter_map(|index| rules.plain(index))
                .find_map(|data| ParsedRule::plain(data).apply_plain(ctx, matches));
        }
        Prematch::End { unsafe_to } => {
            // Only rules without further input can match.
            let mut unsafe_to_concat = false;
            for data in (0..rules.len()).filter_map(|index| rules.plain(index)) {
                let rule = ParsedRule::plain(data);
                if !rule.input.is_empty() {
                    unsafe_to_concat = true;
                } else if rule.apply_plain(ctx, matches).is_some() {
                    return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), Some(()));
                }
            }
            return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), None);
        }
    };
    let mut unsafe_to = None;
    let mut index = 0;
    while index < rules.len() {
        let Some(data) = rules.plain(index) else {
            index += 1;
            continue;
        };
        match plain_rule_first_input(&data) {
            Some(first) if !matches_at(&mut ctx.buffer.info, lookahead.first, first, matches) => {
                unsafe_to.get_or_insert(lookahead.unsafe_to1);
                index = skip_rules(rules, index, Rules::plain, plain_rule_first_input, first);
            }
            _ => {
                let second_matches = lookahead.second.is_none_or(|second| {
                    plain_rule_second_input(&data)
                        .is_none_or(|v| matches_at(&mut ctx.buffer.info, second, v, matches))
                });
                if !second_matches {
                    unsafe_to = Some(lookahead.unsafe_to2);
                } else if ParsedRule::plain(data).apply_plain(ctx, matches).is_some() {
                    return finish_rules(ctx, unsafe_to, Some(()));
                }
                index += 1;
            }
        }
    }
    finish_rules(ctx, unsafe_to, None)
}

/// Returns the index of the first rule after `index` whose first input
/// isn't `first`. Rules are sorted by it, so the ones skipped would fail the
/// same way; unreadable rules are skipped too.
fn skip_rules<'a>(
    rules: Rules<'a>,
    mut index: usize,
    rule_at: fn(&Rules<'a>, usize) -> Option<FontData<'a>>,
    first_input: fn(&FontData) -> Option<u16>,
    first: u16,
) -> usize {
    loop {
        index += 1;
        if index >= rules.len()
            || rule_at(&rules, index).is_some_and(|data| first_input(&data) != Some(first))
        {
            return index;
        }
    }
}

/// Marks the range a rule walk depended on, then returns its result.
fn finish_rules(
    ctx: &mut ApplyContext,
    unsafe_to: Option<usize>,
    result: Option<()>,
) -> Option<()> {
    if let Some(unsafe_to) = unsafe_to {
        ctx.buffer
            .unsafe_to_concat(Some(ctx.buffer.idx), Some(unsafe_to));
    }
    result
}

/// Applies the first rule of a chained rule set that matches, pre-matching
/// as [`apply_context_rules`] does.
#[inline(never)]
fn apply_chain_rules(
    ctx: &mut ApplyContext,
    rules: Rules,
    matchers: &ChainMatchers<impl Matcher, impl Matcher, impl Matcher>,
    digest: Option<RuleSetDigest>,
    first_value: &impl Fn(&mut GlyphInfo) -> u16,
) -> Option<()> {
    let lookahead = match prematch(ctx, rules.len() != 0, digest, first_value)? {
        Prematch::Glyphs(lookahead) => lookahead,
        Prematch::Rejected => return None,
        Prematch::TryAll => {
            return (0..rules.len())
                .filter_map(|index| rules.chain(index))
                .find_map(|data| ParsedRule::chain(data).apply_chain(ctx, matchers));
        }
        Prematch::End { unsafe_to } => {
            // Only rules without further input or lookahead can match.
            let mut unsafe_to_concat = false;
            for data in (0..rules.len()).filter_map(|index| rules.chain(index)) {
                let rule = ParsedRule::chain(data);
                if !rule.input.is_empty() || !rule.lookahead.is_empty() {
                    unsafe_to_concat = true;
                } else if rule.apply_chain(ctx, matchers).is_some() {
                    return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), Some(()));
                }
            }
            return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), None);
        }
    };
    let mut unsafe_to = None;
    let mut index = 0;
    while index < rules.len() {
        let Some(data) = rules.chain(index) else {
            index += 1;
            continue;
        };
        let Some(probe) = ChainRuleProbe::new(data) else {
            // An unreadable header parses as an empty rule.
            if ParsedRule::chain(data).apply_chain(ctx, matchers).is_some() {
                return finish_rules(ctx, unsafe_to, Some(()));
            }
            index += 1;
            continue;
        };
        // A rule with one input glyph is pre-matched on its lookahead.
        let input_len = probe.input_len();
        let info = &mut ctx.buffer.info;
        let first_matches = if input_len > 1 {
            probe
                .input(0)
                .is_some_and(|v| matches_at(info, lookahead.first, v, &matchers.input))
        } else {
            probe.lookahead_len() == 0
                || probe
                    .lookahead(0)
                    .is_some_and(|v| matches_at(info, lookahead.first, v, &matchers.lookahead))
        };
        if first_matches {
            let second_matches = lookahead.second.is_none_or(|second| {
                if input_len > 2 {
                    probe
                        .input(1)
                        .is_some_and(|v| matches_at(info, second, v, &matchers.input))
                } else {
                    probe.lookahead_len() <= 2 - input_len
                        || probe
                            .lookahead(2 - input_len)
                            .is_some_and(|v| matches_at(info, second, v, &matchers.lookahead))
                }
            });
            if !second_matches {
                unsafe_to = Some(lookahead.unsafe_to2);
            } else if ParsedRule::chain(data).apply_chain(ctx, matchers).is_some() {
                return finish_rules(ctx, unsafe_to, Some(()));
            }
            index += 1;
        } else {
            unsafe_to.get_or_insert(lookahead.unsafe_to1);
            index = match (input_len > 1).then(|| probe.input(0)).flatten() {
                Some(first) => {
                    skip_rules(rules, index, Rules::chain, chain_rule_first_input, first)
                }
                None => index + 1,
            };
        }
    }
    finish_rules(ctx, unsafe_to, None)
}

/// Whether a rule of `rules` matches the glyphs of `ctx`.
fn would_apply_rules(ctx: &WouldApplyContext, rules: Rules, matches: MatchFn) -> bool {
    (0..rules.len())
        .filter_map(|index| ParsedRule::try_plain(rules.plain(index)?))
        .any(|rule| sequence_would_apply(ctx, rule.input, matches))
}

/// Whether a chained rule of `rules` matches the glyphs of `ctx`.
fn would_apply_chain_rules(ctx: &WouldApplyContext, rules: Rules, matches: MatchFn) -> bool {
    (0..rules.len())
        .filter_map(|index| ParsedRule::try_chain(rules.chain(index)?))
        .any(|rule| {
            (!ctx.zero_context || (rule.backtrack.is_empty() && rule.lookahead.is_empty()))
                && sequence_would_apply(ctx, rule.input, matches)
        })
}

/// Whether `input` matches the glyphs of `ctx` after the first.
fn sequence_would_apply(
    ctx: &WouldApplyContext,
    input: &[BigEndian<u16>],
    matches: MatchFn,
) -> bool {
    ctx.glyphs.len() == input.len() + 1
        && input.iter().zip(&ctx.glyphs[1..]).all(|(value, glyph)| {
            let mut info = GlyphInfo {
                glyph_id: (*glyph).into(),
                ..GlyphInfo::default()
            };
            matches(&mut info, u32::from(value.get()))
        })
}
