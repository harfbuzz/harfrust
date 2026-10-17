//! Contextual and chained contextual subtables, shared by GSUB and GPOS.
//!
//! A contextual subtable is a chained one without backtrack or lookahead,
//! so both run through the same walks. Only the layout of their rules
//! differs, which [`Rules`] and [`ParsedRule`] record.

use super::coverage::{ClassDef, Coverage, CoverageArray};
use super::gsub::covered_at;
use super::{glyph_class_cached, Offsets, SubtableDigests};
use crate::buffer::GlyphInfo;
use crate::hint::Likely;
use crate::ot::apply::{
    apply_lookup, match_always, match_backtrack, match_glyph, match_input, match_lookahead,
    ApplyContext, MappingCache, MaySkip, RuleSetDigest, SkippingIterator, WouldApplyContext,
};
use crate::set_digest::SetDigest;
use alloc::boxed::Box;
use read_fonts::tables::layout::{
    ChainedSequenceContextFormat1, ChainedSequenceContextFormat2, ChainedSequenceContextFormat3,
    ChainedSequenceRule, SequenceContextFormat1, SequenceContextFormat2, SequenceContextFormat3,
    SequenceLookupRecord, SequenceRule,
};
use read_fonts::types::{BigEndian, FixedSize, GlyphId, Nullable, Offset16};
use read_fonts::{FontData, Offset};

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

/// Rule set `index` of `sets`, or `None` if it is null or can't be read.
fn rule_set<'a>(sets: &RuleSets<'a>, index: usize, chained: bool) -> Option<Rules<'a>> {
    Rules::read(sets.data(index)?, chained)
}

/// Calls `f` with each rule of each rule set present in `sets`, stopping
/// with `None` at one that can't be read.
fn for_each_rule<'a>(
    sets: &RuleSets<'a>,
    chained: bool,
    mut f: impl FnMut(ParsedRule<'a>) -> Option<()>,
) -> Option<()> {
    for index in 0..sets.len() {
        if sets.is_null(index) {
            continue;
        }
        let rules = rule_set(sets, index, chained)?;
        for index in 0..rules.len() {
            f(ParsedRule::try_parse(rules.rule(index)?, chained)?)?;
        }
    }
    Some(())
}

/// The digest of the first input value of each rule set's rules.
fn rule_set_digests(sets: &RuleSets, chained: bool) -> Box<[RuleSetDigest]> {
    (0..sets.len())
        .map(|index| {
            if sets.is_null(index) {
                return RuleSetDigest::default();
            }
            let Some(rules) = rule_set(sets, index, chained) else {
                return RuleSetDigest::full();
            };
            let mut digest = RuleSetDigest::default();
            for index in 0..rules.len() {
                let Some(value) = rules
                    .rule(index)
                    .and_then(|data| rule_first_input(&data, chained))
                else {
                    return RuleSetDigest::full();
                };
                digest.add(value);
            }
            digest
        })
        .collect()
}

/// Chained context by glyph sequences, or context by glyph sequences.
pub(crate) struct ChainContext1<'a> {
    rule_sets: RuleSets<'a>,
    chained: bool,
}

impl<'a> ChainContext1<'a> {
    pub fn new(table: &ChainedSequenceContextFormat1<'a>) -> Option<Self> {
        Some(Self {
            rule_sets: RuleSets::new(table.offset_data(), table.chained_seq_rule_set_offsets()),
            chained: true,
        })
    }

    pub fn from_plain(table: &SequenceContextFormat1<'a>) -> Option<Self> {
        Some(Self {
            rule_sets: RuleSets::new(table.offset_data(), table.seq_rule_set_offsets()),
            chained: false,
        })
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        rule_set(&self.rule_sets, index, self.chained)
    }

    pub fn apply(&self, ctx: &mut ApplyContext, index: u16) -> Option<()> {
        let rules = self.rule_set(usize::from(index))?;
        let matchers = ChainMatchers {
            backtrack: &match_glyph,
            input: &match_glyph,
            lookahead: &match_glyph,
        };
        apply_chain_rules(ctx, rules, &matchers, None, &|_| 0)
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext, coverage: &Coverage) -> bool {
        coverage
            .get(ctx.glyphs[0])
            .and_then(|index| self.rule_set(index as usize))
            .is_some_and(|rules| would_apply_chain_rules(ctx, rules, &match_glyph))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext1<'a> {
    fn digest_second(&self) -> SetDigest {
        let mut digest = SetDigest::new();
        for_each_rule(&self.rule_sets, self.chained, |rule| {
            let second = rule.input.first().or_else(|| rule.lookahead.first())?;
            digest.add(u32::from(second.get()));
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }
}

/// Chained context by glyph class sequences, or context by glyph class
/// sequences.
pub(crate) struct ChainContext2<'a> {
    backtrack: ClassDef<'a>,
    input: ClassDef<'a>,
    lookahead: ClassDef<'a>,
    class_caches: Option<Box<ChainClassCaches>>,
    rule_sets: RuleSets<'a>,
    chained: bool,
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
        Some(Self::from_parts(
            ClassDef::new(table.backtrack_class_def()),
            ClassDef::new(table.input_class_def()),
            ClassDef::new(table.lookahead_class_def()),
            RuleSets::new(
                table.offset_data(),
                table.chained_class_seq_rule_set_offsets(),
            ),
            true,
            with_caches,
        ))
    }

    pub fn from_plain(table: &SequenceContextFormat2<'a>, with_caches: bool) -> Option<Self> {
        Some(Self::from_parts(
            ClassDef::None,
            ClassDef::new(table.class_def()),
            ClassDef::None,
            RuleSets::new(table.offset_data(), table.class_seq_rule_set_offsets()),
            false,
            with_caches,
        ))
    }

    fn from_parts(
        backtrack: ClassDef<'a>,
        input: ClassDef<'a>,
        lookahead: ClassDef<'a>,
        rule_sets: RuleSets<'a>,
        chained: bool,
        with_caches: bool,
    ) -> Self {
        Self {
            backtrack,
            input,
            lookahead,
            class_caches: with_caches.then(|| {
                Box::new(ChainClassCaches {
                    input: MappingCache::new(),
                    lookahead: MappingCache::new(),
                })
            }),
            rule_set_digests: rule_set_digests(&rule_sets, chained),
            rule_sets,
            chained,
        }
    }

    fn rule_set(&self, index: usize) -> Option<Rules<'a>> {
        rule_set(&self.rule_sets, index, self.chained)
    }

    /// The input class of a glyph, kept in its syllable byte if
    /// `class_cache` is set.
    #[inline(always)]
    fn input_class(&self, info: &mut GlyphInfo, class_cache: bool) -> u16 {
        let class_of = |gid| self.input.get(gid);
        if !class_cache {
            self.mapped_input_class(info.as_glyph())
        } else if self.chained {
            // The lookahead class takes the low nibble.
            SyllableSlot::HIGH.class(&class_of, info)
        } else {
            SyllableSlot::FULL.class(&class_of, info)
        }
    }

    #[inline(always)]
    fn lookahead_class(&self, info: &mut GlyphInfo, class_cache: bool) -> u16 {
        if class_cache {
            SyllableSlot::LOW.class(&|gid| self.lookahead.get(gid), info)
        } else {
            self.mapped_lookahead_class(info.as_glyph())
        }
    }

    // The mapping-cache paths stay out of line so that the syllable-byte
    // reads inline into the matchers.
    #[inline(never)]
    fn mapped_input_class(&self, gid: GlyphId) -> u16 {
        let cache = self.class_caches.as_deref().map(|c| &c.input);
        cached_class(&self.input, gid, cache)
    }

    #[inline(never)]
    fn mapped_lookahead_class(&self, gid: GlyphId) -> u16 {
        let cache = self.class_caches.as_deref().map(|c| &c.lookahead);
        cached_class(&self.lookahead, gid, cache)
    }

    /// Applies, keeping glyph classes in the syllable byte if `class_cache`
    /// is set, and in the subtable's mapping caches otherwise.
    #[inline(never)]
    pub fn apply(&self, ctx: &mut ApplyContext, _index: u16, class_cache: bool) -> Option<()> {
        let info = ctx.buffer.info.get_mut(ctx.buffer.idx).likely()?;
        let index = self.input_class(info, class_cache) as usize;
        let matchers = ChainMatchers {
            backtrack: &|info: &mut GlyphInfo, value| {
                u32::from(self.backtrack.get(info.as_glyph())) == value
            },
            input: &|info: &mut GlyphInfo, value| {
                u32::from(self.input_class(info, class_cache)) == value
            },
            lookahead: &|info: &mut GlyphInfo, value| {
                u32::from(self.lookahead_class(info, class_cache)) == value
            },
        };
        apply_chain_rules(
            ctx,
            self.rule_set(index)?,
            &matchers,
            self.rule_set_digests.get(index).copied(),
            &|info| self.input_class(info, class_cache),
        )
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext, _coverage: &Coverage) -> bool {
        let matches =
            |info: &mut GlyphInfo, value| u32::from(self.input.get(info.as_glyph())) == value;
        self.rule_set(self.input.get(ctx.glyphs[0]) as usize)
            .is_some_and(|rules| would_apply_chain_rules(ctx, rules, &matches))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext2<'a> {
    const CACHES_COVERAGE: bool = true;

    fn digest_second(&self) -> SetDigest {
        let (input, lookahead) = (&self.input, &self.lookahead);
        if input.is_none() {
            return SetDigest::full();
        }
        let mut digest = SetDigest::new();
        for_each_rule(&self.rule_sets, self.chained, |rule| {
            if let Some(second) = rule.input.first() {
                input.add_class_to(&mut digest, second.get());
            } else {
                let second = rule.lookahead.first()?;
                // A missing class definition puts every glyph in class 0.
                (!lookahead.is_none()).then_some(())?;
                lookahead.add_class_to(&mut digest, second.get());
            }
            Some(())
        })
        .map_or_else(SetDigest::full, |()| digest)
    }

    fn cache_cost(&self) -> u32 {
        self.input.cost() + self.lookahead.cost()
    }
}

/// Chained context by a coverage per glyph, or context by a coverage per
/// glyph.
pub(crate) struct ChainContext3<'a> {
    /// The coverages of the input glyphs after the first.
    inputs: CoverageArray<'a>,
    backtrack: CoverageArray<'a>,
    lookahead: CoverageArray<'a>,
    records: &'a [SequenceLookupRecord],
    chained: bool,
}

impl<'a> ChainContext3<'a> {
    pub fn new(table: &ChainedSequenceContextFormat3<'a>) -> Option<Self> {
        Some(Self {
            inputs: CoverageArray::new(
                table.offset_data(),
                table.input_coverage_offsets().get(1..).unwrap_or_default(),
            ),
            backtrack: CoverageArray::new(table.offset_data(), table.backtrack_coverage_offsets()),
            lookahead: CoverageArray::new(table.offset_data(), table.lookahead_coverage_offsets()),
            records: table.seq_lookup_records(),
            chained: true,
        })
    }

    pub fn from_plain(table: &SequenceContextFormat3<'a>) -> Option<Self> {
        Some(Self {
            inputs: CoverageArray::new(
                table.offset_data(),
                table.coverage_offsets().get(1..).unwrap_or_default(),
            ),
            backtrack: CoverageArray::default(),
            lookahead: CoverageArray::default(),
            records: table.seq_lookup_records(),
            chained: false,
        })
    }

    pub fn apply(&self, ctx: &mut ApplyContext, _index: u16) -> Option<()> {
        let mut end_index = ctx.buffer.idx;
        let mut match_end = 0;
        let input_matches = match_input(
            ctx,
            self.inputs.len() as u16,
            |info, index| covered_at(&self.inputs, index, info),
            &mut match_end,
            None,
        )?;
        if !self.chained {
            if !input_matches {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(match_end));
                return None;
            }
            ctx.buffer
                .unsafe_to_break(Some(ctx.buffer.idx), Some(match_end));
            apply_lookup(ctx, self.inputs.len(), match_end, self.records);
            return Some(());
        }
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

    pub fn would_apply(&self, ctx: &WouldApplyContext, _coverage: &Coverage) -> bool {
        (!ctx.zero_context || (self.backtrack.is_empty() && self.lookahead.is_empty()))
            && ctx.glyphs.len() == self.inputs.len() + 1
            && ctx.glyphs[1..]
                .iter()
                .enumerate()
                .all(|(index, glyph)| self.inputs.get(index).covers(*glyph))
    }
}

impl<'a> SubtableDigests<'a> for ChainContext3<'a> {
    fn digest_second(&self) -> SetDigest {
        if self.inputs.is_empty() {
            self.lookahead.get(0).digest()
        } else {
            self.inputs.get(0).digest()
        }
    }
}

/// Where a glyph's class is kept in its syllable byte while a lookup
/// applies: in the bits of `unknown` shifted left by `shift`, where the
/// value `unknown` means the class isn't known yet.
#[derive(Clone, Copy)]
struct SyllableSlot {
    shift: u8,
    unknown: u8,
}

impl SyllableSlot {
    /// The input class of a chained context.
    const HIGH: Self = Self {
        shift: 4,
        unknown: 15,
    };
    /// The lookahead class of a chained context.
    const LOW: Self = Self {
        shift: 0,
        unknown: 15,
    };
    /// The input class of a context, which has no lookahead.
    const FULL: Self = Self {
        shift: 0,
        unknown: 255,
    };

    /// The class of the glyph, looked up with `class_def` the first time.
    #[inline(always)]
    fn class(self, class_def: &impl Fn(GlyphId) -> u16, info: &mut GlyphInfo) -> u16 {
        let cached = (info.syllable() >> self.shift) & self.unknown;
        if cached < self.unknown {
            return u16::from(cached);
        }
        let class = class_def(info.as_glyph());
        if class < u16::from(self.unknown) {
            let mask = self.unknown << self.shift;
            info.set_syllable((info.syllable() & !mask) | ((class as u8) << self.shift));
        }
        class
    }
}

/// The rules of a rule set, read on demand from their offsets.
#[derive(Clone, Copy)]
struct Rules<'a> {
    data: FontData<'a>,
    offsets: &'a [BigEndian<Offset16>],
    /// Whether the rules are chained rules rather than sequence rules.
    chained: bool,
}

impl<'a> Rules<'a> {
    /// Reads a rule set, whose layout is the same for every kind of rule.
    fn read(data: FontData<'a>, chained: bool) -> Option<Self> {
        let count = usize::from(data.read_at::<u16>(0).ok()?);
        let offsets = data
            .read_array(2..2 + count * Offset16::RAW_BYTE_LEN)
            .ok()?;
        Some(Self {
            data,
            offsets,
            chained,
        })
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    /// The data of rule `index`, if it holds at least the fixed fields of
    /// a rule, as reading the rule table would require.
    fn rule(&self, index: usize) -> Option<FontData<'a>> {
        let min_size = if self.chained {
            ChainedSequenceRule::MIN_SIZE
        } else {
            SequenceRule::MIN_SIZE
        };
        let data = self
            .data
            .split_off(self.offsets.get(index)?.get().to_usize())?;
        (data.len() >= min_size).then_some(data)
    }
}

/// A rule's fields, parsed in one pass. Glyph IDs and classes are both
/// read as plain 16-bit values; a sequence rule has no backtrack or
/// lookahead.
#[derive(Clone, Copy, Default)]
struct ParsedRule<'a> {
    backtrack: &'a [BigEndian<u16>],
    input: &'a [BigEndian<u16>],
    lookahead: &'a [BigEndian<u16>],
    records: &'a [SequenceLookupRecord],
    chained: bool,
}

impl<'a> ParsedRule<'a> {
    /// Parses a rule; an unreadable one is empty.
    fn parse(data: FontData<'a>, chained: bool) -> Self {
        Self::try_parse(data, chained).unwrap_or_else(|| Self {
            chained,
            ..Self::default()
        })
    }

    fn try_parse(data: FontData<'a>, chained: bool) -> Option<Self> {
        if chained {
            Self::try_chain(data)
        } else {
            Self::try_plain(data)
        }
    }

    /// Parses a sequence rule or class sequence rule.
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

    /// Parses a chained sequence rule or chained class sequence rule.
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
            chained: true,
        })
    }

    /// Matches the input, lookahead and backtrack sequences in that order
    /// and applies the lookup records.
    fn apply(
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
            if self.chained {
                ctx.buffer
                    .unsafe_to_concat(Some(ctx.buffer.idx), Some(end_index));
            }
            return None;
        }
        if !self.chained {
            ctx.buffer
                .unsafe_to_break(Some(ctx.buffer.idx), Some(match_end));
            apply_lookup(ctx, self.input.len(), match_end, self.records);
            return Some(());
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

/// The first input value of a rule, after any backtrack: the second glyph
/// of a match.
fn rule_first_input(data: &FontData, chained: bool) -> Option<u16> {
    RuleProbe::new(*data, chained)
        .filter(|probe| probe.input_count > 1)?
        .input(0)
}

/// Reads the values of a rule that the pre-match needs, without parsing
/// the rule.
struct RuleProbe<'a> {
    data: FontData<'a>,
    input_start: usize,
    input_end: usize,
    input_count: u16,
    chained: bool,
}

impl<'a> RuleProbe<'a> {
    fn new(data: FontData<'a>, chained: bool) -> Option<Self> {
        // A sequence rule's lookup count sits between its glyph count and
        // its input.
        let (count_pos, input_start) = if chained {
            let backtrack_count = usize::from(data.read_at::<u16>(0).ok()?);
            let count_pos = 2 + backtrack_count * u16::RAW_BYTE_LEN;
            (count_pos, count_pos + 2)
        } else {
            (0, 4)
        };
        let input_count: u16 = data.read_at(count_pos).ok()?;
        let input_end =
            input_start + usize::from(input_count).saturating_sub(1) * u16::RAW_BYTE_LEN;
        Some(Self {
            data,
            input_start,
            input_end,
            input_count,
            chained,
        })
    }

    /// The length of the input sequence, including the first glyph.
    fn input_len(&self) -> usize {
        usize::from(self.input_count).max(1)
    }

    fn input(&self, index: usize) -> Option<u16> {
        self.data
            .read_at(self.input_start + index * u16::RAW_BYTE_LEN)
            .ok()
    }

    fn lookahead_len(&self) -> usize {
        if !self.chained {
            return 0;
        }
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

/// Returns the index of the first rule after `index` whose first input
/// isn't `first`. Rules are sorted by it, so the ones skipped would fail the
/// same way; unreadable rules are skipped too.
fn skip_rules(rules: Rules, mut index: usize, first: u16) -> usize {
    loop {
        index += 1;
        if index >= rules.len()
            || rules
                .rule(index)
                .is_some_and(|data| rule_first_input(&data, rules.chained) != Some(first))
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

/// Applies the first rule of a rule set that matches.
///
/// Rules are pre-matched on the first two values after the current glyph,
/// from the input and then the lookahead, which rejects most of them
/// without parsing. HarfBuzz skips the pre-match for small rule sets; here
/// it is faster at every size.
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
                .filter_map(|index| rules.rule(index))
                .find_map(|data| ParsedRule::parse(data, rules.chained).apply(ctx, matchers));
        }
        Prematch::End { unsafe_to } => {
            // Only rules without further input or lookahead can match.
            let mut unsafe_to_concat = false;
            for data in (0..rules.len()).filter_map(|index| rules.rule(index)) {
                let rule = ParsedRule::parse(data, rules.chained);
                if !rule.input.is_empty() || !rule.lookahead.is_empty() {
                    unsafe_to_concat = true;
                } else if rule.apply(ctx, matchers).is_some() {
                    return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), Some(()));
                }
            }
            return finish_rules(ctx, unsafe_to_concat.then_some(unsafe_to), None);
        }
    };
    let mut unsafe_to = None;
    let mut index = 0;
    while index < rules.len() {
        let Some(data) = rules.rule(index) else {
            index += 1;
            continue;
        };
        let Some(probe) = RuleProbe::new(data, rules.chained) else {
            // An unreadable header parses as an empty rule.
            if ParsedRule::parse(data, rules.chained)
                .apply(ctx, matchers)
                .is_some()
            {
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
            } else if ParsedRule::parse(data, rules.chained)
                .apply(ctx, matchers)
                .is_some()
            {
                return finish_rules(ctx, unsafe_to, Some(()));
            }
            index += 1;
        } else {
            unsafe_to.get_or_insert(lookahead.unsafe_to1);
            index = match (input_len > 1).then(|| probe.input(0)).flatten() {
                Some(first) => skip_rules(rules, index, first),
                None => index + 1,
            };
        }
    }
    finish_rules(ctx, unsafe_to, None)
}

/// Whether a rule of `rules` matches the glyphs of `ctx`.
fn would_apply_chain_rules(ctx: &WouldApplyContext, rules: Rules, matches: MatchFn) -> bool {
    (0..rules.len())
        .filter_map(|index| ParsedRule::try_parse(rules.rule(index)?, rules.chained))
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
