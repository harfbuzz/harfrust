use alloc::boxed::Box;

use crate::hb::{
    hb_font_t, hb_glyph_info_t,
    ot_layout_gsubgpos::{
        Apply, MappingCache, PairPosFormat2Cache, SubtableExternalCache, WouldApply,
        WouldApplyContext, OT::hb_ot_apply_context_t,
    },
    set_digest::hb_set_digest_t,
};

use alloc::vec::Vec;
use core::ops::Range;
use read_fonts::{
    tables::{
        gpos::{
            CursivePosFormat1, Gpos, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1,
            PairPos, PairPosFormat1, PairPosFormat2, SinglePos, SinglePosFormat1, SinglePosFormat2,
        },
        gsub::{
            AlternateSubstFormat1, ExtensionSubstFormat1, Gsub, LigatureSubstFormat1,
            MultipleSubstFormat1, ReverseChainSingleSubstFormat1, SingleSubst, SingleSubstFormat1,
            SingleSubstFormat2,
        },
        layout::{
            ChainedSequenceContext, ChainedSequenceContextFormat1, ChainedSequenceContextFormat2,
            ChainedSequenceContextFormat3, CoverageTable, Lookup, LookupFlag, SequenceContext,
            SequenceContextFormat1, SequenceContextFormat2, SequenceContextFormat3,
        },
    },
    FontData, FontRead, Offset, ReadError,
};

pub trait LookupHost<'a> {
    fn lookup_count(&self) -> u16;
    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError>;
}

impl<'a> LookupHost<'a> for Gsub<'a> {
    fn lookup_count(&self) -> u16 {
        self.lookup_list()
            .map(|list| list.lookup_count())
            .unwrap_or_default()
    }

    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError> {
        let list = self.lookup_list()?;
        let offset = list
            .lookup_offsets()
            .get(index as usize)
            .ok_or(ReadError::OutOfBounds)?
            .get()
            .to_usize()
            + self.lookup_list_offset().to_usize();
        Ok(LookupData {
            offset,
            is_subst: true,
            table_data: self.offset_data(),
        })
    }
}

impl<'a> LookupHost<'a> for Gpos<'a> {
    fn lookup_count(&self) -> u16 {
        self.lookup_list()
            .map(|list| list.lookup_count())
            .unwrap_or_default()
    }

    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError> {
        let list = self.lookup_list()?;
        let offset = list
            .lookup_offsets()
            .get(index as usize)
            .ok_or(ReadError::OutOfBounds)?
            .get()
            .to_usize()
            + self.lookup_list_offset().to_usize();
        Ok(LookupData {
            offset,
            is_subst: false,
            table_data: self.offset_data(),
        })
    }
}

pub struct LookupData<'a> {
    /// Offset of the lookup from the base of the layout table.
    offset: usize,
    /// True if the lookup comes from GSUB.
    is_subst: bool,
    /// Data of the layout table.
    table_data: FontData<'a>,
}

/// Cache containing lookup and subtable information for a single GSUB or
/// GPOS table.
#[derive(Default)]
pub struct LookupCache {
    pub lookups: Vec<LookupInfo>,
    pub subtables: Vec<SubtableInfo>,
}

impl LookupCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.lookups.clear();
        self.subtables.clear();
    }

    pub fn create_all<'a>(&mut self, host: &impl LookupHost<'a>) {
        self.clear();
        let count = host.lookup_count();
        self.lookups.resize(count as usize, Default::default());
        for i in 0..count {
            let _ = self.get_or_create(host, i);
        }
    }

    pub fn get(&self, index: u16) -> Option<&LookupInfo> {
        let entry = self.lookups.get(index as usize)?;
        match entry.state {
            LookupState::Ready => Some(entry),
            _ => None,
        }
    }

    pub fn get_or_create<'a>(
        &mut self,
        cx: &impl LookupHost<'a>,
        index: u16,
    ) -> Result<&LookupInfo, ReadError> {
        let index = index as usize;
        if index >= self.lookups.len() {
            self.lookups.resize(index + 1, LookupInfo::default());
        }
        let entry = &mut self.lookups[index];
        if entry.state != LookupState::Vacant {
            return Ok(entry);
        }
        entry.state = LookupState::Error;
        let data = cx.lookup_data(index as u16)?;
        entry.is_subst = data.is_subst;
        let lookup_data = data
            .table_data
            .split_off(data.offset)
            .ok_or(ReadError::OutOfBounds)?;
        let lookup: Lookup<()> = Lookup::read(lookup_data)?;
        let kind = lookup.lookup_type();
        let lookup_flag = lookup.lookup_flag();
        entry.props = u32::from(lookup.lookup_flag().to_bits());
        if lookup_flag.to_bits() & LookupFlag::USE_MARK_FILTERING_SET.to_bits() != 0 {
            entry.props |= (lookup.mark_filtering_set().unwrap_or_default() as u32) << 16;
        }
        entry.is_rtl = lookup_flag.to_bits() & LookupFlag::RIGHT_TO_LEFT.to_bits() != 0;
        if data.is_subst {
            entry.is_reversed =
                is_reversed(data.table_data, &lookup, data.offset).unwrap_or_default();
        }
        entry.subtables_start = self
            .subtables
            .len()
            .try_into()
            .map_err(|_| ReadError::MalformedData("too many subtables"))?;
        entry.state = LookupState::Ready;
        let mut subtable_cache_user_cost = 0;
        for subtable_offset in lookup.subtable_offsets() {
            let subtable_offset = subtable_offset.get().to_usize() + data.offset;
            if let Some((subtable_info, cache_cost)) = SubtableInfo::new(
                data.table_data,
                subtable_offset as u32,
                data.is_subst,
                kind as u8,
            ) {
                entry.digest.union(&subtable_info.digest);
                if cache_cost > subtable_cache_user_cost {
                    entry.subtable_cache_user_idx = Some(entry.subtables_count as usize);
                    subtable_cache_user_cost = cache_cost;
                }
                self.subtables.push(subtable_info);
                entry.subtables_count += 1;
            }
        }
        Ok(entry)
    }

    pub fn subtables(&self, entry: &LookupInfo) -> Option<&[SubtableInfo]> {
        self.subtables.get(entry.subtables_range())
    }
}

fn is_reversed(table_data: FontData, lookup: &Lookup<()>, lookup_offset: usize) -> Option<bool> {
    match lookup.lookup_type() {
        // Reverse chain context
        8 => Some(true),
        // Extension table
        7 => {
            let offset = lookup_offset + lookup.subtable_offsets().first()?.get().to_usize();
            let data = table_data.split_off(offset)?;
            let ext = ExtensionSubstFormat1::<()>::read(data).ok()?;
            Some(ext.extension_lookup_type() == 8)
        }
        _ => Some(false),
    }
}

/// Current state of a lookup cache entry.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
#[repr(u8)]
pub enum LookupState {
    /// Lookup has not been cached yet. This supports
    /// lazy population of the lookup cache.
    #[default]
    Vacant,
    /// Lookup is available for use.
    Ready,
    /// An error occurred while reading this lookup.
    Error,
}

/// Cached information about a lookup.
#[derive(Clone, Default)]
pub struct LookupInfo {
    /// Current state of this lookup info entry.
    pub state: LookupState,
    pub props: u32,
    pub is_subst: bool,
    /// Indicates RTL processing for cursive lookups.
    pub is_rtl: bool,
    /// True if glyphs should be processed in reverse for this lookup.
    pub is_reversed: bool,
    /// Index of the first subtable in the cache subtables vector.
    pub subtables_start: u32,
    /// Number of subtables in the cache subtables vector.
    pub subtables_count: u16,
    /// Bloom filter representing the set of glyphs from the primary
    /// coverage of all subtables in the lookup.
    pub digest: hb_set_digest_t,
    pub subtable_cache_user_idx: Option<usize>,
}

impl LookupInfo {
    pub fn subtables_range(&self) -> Range<usize> {
        let start = self.subtables_start as usize;
        start..start + self.subtables_count as usize
    }

    pub fn props(&self) -> u32 {
        self.props
    }

    pub fn is_reverse(&self) -> bool {
        self.is_reversed
    }

    pub fn digest(&self) -> &hb_set_digest_t {
        &self.digest
    }
}

impl LookupInfo {
    #[inline]
    pub(crate) fn apply(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        table_data: &[u8],
        cache: &LookupCache,
        use_hot_subtable_cache: bool,
    ) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        for (subtable_idx, subtable_info) in cache.subtables(self)?.iter().enumerate() {
            if !subtable_info.digest.may_have_glyph(glyph) {
                continue;
            }
            let is_cached =
                use_hot_subtable_cache & (self.subtable_cache_user_idx == Some(subtable_idx));
            if subtable_info.apply(ctx, table_data, is_cached).is_some() {
                return Some(());
            }
        }
        None
    }

    pub(crate) fn cache_enter(&self, ctx: &mut hb_ot_apply_context_t, cache: &LookupCache) -> bool {
        let Some(idx) = self.subtable_cache_user_idx else {
            return false;
        };
        let Some(subtable_info) = cache.subtables(self).unwrap_or_default().get(idx) else {
            return false;
        };
        if matches!(
            subtable_info.kind,
            SubtableKind::ContextFormat2 | SubtableKind::ChainedContextFormat2
        ) {
            cache_enter(ctx)
        } else {
            false
        }
    }
    pub(crate) fn cache_leave(&self, ctx: &mut hb_ot_apply_context_t, cache: &LookupCache) {
        let Some(idx) = self.subtable_cache_user_idx else {
            return;
        };
        let Some(subtable_info) = cache.subtables(self).unwrap_or_default().get(idx) else {
            return;
        };
        if matches!(
            subtable_info.kind,
            SubtableKind::ContextFormat2 | SubtableKind::ChainedContextFormat2
        ) {
            cache_leave(ctx);
        }
    }
}

impl LookupInfo {
    pub fn would_apply(&self, face: &hb_font_t, ctx: &WouldApplyContext) -> Option<bool> {
        let glyph = ctx.glyphs[0];
        if !self.digest.may_have_glyph(glyph) {
            return Some(false);
        }
        let (table_data, lookups) = if self.is_subst {
            let table = face.ot_tables.gsub.as_ref()?;
            (table.table.offset_data().as_bytes(), &table.lookups)
        } else {
            let table = face.ot_tables.gpos.as_ref()?;
            (table.table.offset_data().as_bytes(), &table.lookups)
        };
        let subtables = lookups.subtables(self)?;
        for subtable_info in subtables {
            if !subtable_info.digest.may_have_glyph(glyph) {
                continue;
            }
            let Some(data) = table_data.get(subtable_info.offset as usize..) else {
                continue;
            };
            let data = FontData::new(data);
            let result = match subtable_info.kind {
                SubtableKind::SingleSubst1 => {
                    SingleSubstFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::SingleSubst2 => {
                    SingleSubstFormat2::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::MultipleSubst1 => {
                    MultipleSubstFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::AlternateSubst1 => {
                    AlternateSubstFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::LigatureSubst1 => {
                    LigatureSubstFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ReverseChainContext => {
                    ReverseChainSingleSubstFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ContextFormat1 => {
                    SequenceContextFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ContextFormat2 => {
                    SequenceContextFormat2::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ContextFormat3 => {
                    SequenceContextFormat3::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ChainedContextFormat1 => {
                    ChainedSequenceContextFormat1::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ChainedContextFormat2 => {
                    ChainedSequenceContextFormat2::read(data).map(|t| t.would_apply(ctx))
                }
                SubtableKind::ChainedContextFormat3 => {
                    ChainedSequenceContextFormat3::read(data).map(|t| t.would_apply(ctx))
                }
                _ => continue,
            };
            if result == Ok(true) {
                return Some(true);
            }
        }
        None
    }
}

/// Cached information about a subtable.
pub struct SubtableInfo {
    /// The fully resolved type of the subtable.
    pub kind: SubtableKind,
    /// Byte offset to the subtable from the base of the GSUB or GPOS
    /// table.
    pub offset: u32,
    pub digest: hb_set_digest_t,
    pub apply_fns: [SubtableApplyFn; 2],
    pub external_cache: SubtableExternalCache,
}

pub type SubtableApplyFn =
    fn(&mut hb_ot_apply_context_t, &SubtableExternalCache, &[u8]) -> Option<()>;

impl SubtableInfo {
    #[inline]
    pub(crate) fn apply(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        table_data: &[u8],
        is_cached: bool,
    ) -> Option<()> {
        let subtable_data = table_data.get(self.offset as usize..)?;
        self.apply_fns[is_cached as usize](ctx, &self.external_cache, subtable_data)
    }
}

macro_rules! apply_fns {
    ($apply:ident, $apply_cached:ident, $ty:ident) => {
        fn $apply(
            ctx: &mut hb_ot_apply_context_t,
            external_cache: &SubtableExternalCache,
            table_data: &[u8],
        ) -> Option<()> {
            let t = $ty::read(FontData::new(table_data)).ok()?;
            t.apply_with_external_cache(ctx, external_cache)
        }

        fn $apply_cached(
            ctx: &mut hb_ot_apply_context_t,
            external_cache: &SubtableExternalCache,
            table_data: &[u8],
        ) -> Option<()> {
            let t = $ty::read(FontData::new(table_data)).ok()?;
            t.apply_cached(ctx, external_cache)
        }
    };
}

apply_fns!(single_subst1, single_subst1_cached, SingleSubstFormat1);
apply_fns!(single_subst2, single_subst2_cached, SingleSubstFormat2);
apply_fns!(
    multiple_subst1,
    multiple_subst1_cached,
    MultipleSubstFormat1
);
apply_fns!(
    alternate_subst1,
    alternate_subst1_cached,
    AlternateSubstFormat1
);
// apply_fns!(
//     ligature_subst1,
//     ligature_subst1_cached,
//     LigatureSubstFormat1
// );
apply_fns!(single_pos1, single_pos1_cached, SinglePosFormat1);
apply_fns!(single_pos2, single_pos2_cached, SinglePosFormat2);
// apply_fns!(pair_pos1, pair_pos1_cached, PairPosFormat1);
// apply_fns!(pair_pos2, pair_pos2_cached, PairPosFormat2);
apply_fns!(cursive_pos1, cursive_pos1_cached, CursivePosFormat1);
apply_fns!(mark_base_pos1, mark_base_pos1_cached, MarkBasePosFormat1);
apply_fns!(mark_mark_pos1, mark_mark_pos1_cached, MarkMarkPosFormat1);
apply_fns!(mark_lig_pos1, mark_lig_pos1_cached, MarkLigPosFormat1);
apply_fns!(context1, context1_cached, SequenceContextFormat1);
apply_fns!(context2, context2_cached, SequenceContextFormat2);
apply_fns!(context3, context3_cached, SequenceContextFormat3);
apply_fns!(
    chained_context1,
    chained_context1_cached,
    ChainedSequenceContextFormat1
);
apply_fns!(
    chained_context2,
    chained_context2_cached,
    ChainedSequenceContextFormat2
);
apply_fns!(
    chained_context3,
    chained_context3_cached,
    ChainedSequenceContextFormat3
);
apply_fns!(
    rev_chain_single_subst1,
    rev_chain_single_subst1_cached,
    ReverseChainSingleSubstFormat1
);

fn ligature_subst1(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gsub::apply_lig_subst1(ctx, table_data, 0, external_cache)
}

fn ligature_subst1_cached(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gsub::apply_lig_subst1(ctx, table_data, 0, external_cache)
}

fn pair_pos1(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gpos::apply_pair_pos1(ctx, table_data, 0, external_cache)
}

fn pair_pos1_cached(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gpos::apply_pair_pos1(ctx, table_data, 0, external_cache)
}

fn pair_pos2(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gpos::apply_pair_pos2(ctx, table_data, 0, external_cache)
}

fn pair_pos2_cached(
    ctx: &mut hb_ot_apply_context_t,
    external_cache: &SubtableExternalCache,
    table_data: &[u8],
) -> Option<()> {
    super::gpos::apply_pair_pos2(ctx, table_data, 0, external_cache)
}

/// All possible subtables in a lookup.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SubtableKind {
    SingleSubst1,
    SingleSubst2,
    MultipleSubst1,
    AlternateSubst1,
    LigatureSubst1,
    SinglePos1,
    SinglePos2,
    PairPos1,
    PairPos2,
    CursivePos1,
    MarkBasePos1,
    MarkMarkPos1,
    MarkLigPos1,
    ContextFormat1,
    ContextFormat2,
    ContextFormat3,
    ChainedContextFormat1,
    ChainedContextFormat2,
    ChainedContextFormat3,
    ReverseChainContext,
}

impl SubtableInfo {
    fn new(
        table_data: FontData,
        subtable_offset: u32,
        is_subst: bool,
        lookup_type: u8,
    ) -> Option<(Self, u32)> {
        let data = table_data.split_off(subtable_offset as usize)?;
        let (kind, (cache_cost, coverage), apply_fns): (
            SubtableKind,
            (u32, CoverageTable),
            [SubtableApplyFn; 2],
        ) = match (is_subst, lookup_type) {
            (true, 1) => match SingleSubst::read(data).ok()? {
                SingleSubst::Format1(s) => (
                    SubtableKind::SingleSubst1,
                    (s.cache_cost(), s.coverage().ok()?),
                    [single_subst1, single_subst1_cached as _],
                ),
                SingleSubst::Format2(s) => (
                    SubtableKind::SingleSubst2,
                    (s.cache_cost(), s.coverage().ok()?),
                    [single_subst2, single_subst2_cached as _],
                ),
            },
            (false, 1) => match SinglePos::read(data).ok()? {
                SinglePos::Format1(s) => (
                    SubtableKind::SinglePos1,
                    (s.cache_cost(), s.coverage().ok()?),
                    [single_pos1, single_pos1_cached as _],
                ),
                SinglePos::Format2(s) => (
                    SubtableKind::SinglePos2,
                    (s.cache_cost(), s.coverage().ok()?),
                    [single_pos2, single_pos2_cached as _],
                ),
            },
            (true, 2) => (
                SubtableKind::MultipleSubst1,
                MultipleSubstFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.coverage().ok()?)))?,
                [multiple_subst1, multiple_subst1_cached as _],
            ),
            (false, 2) => match PairPos::read(data).ok()? {
                PairPos::Format1(s) => (
                    SubtableKind::PairPos1,
                    (s.cache_cost(), s.coverage().ok()?),
                    [pair_pos1, pair_pos1_cached as _],
                ),
                PairPos::Format2(s) => (
                    SubtableKind::PairPos2,
                    (s.cache_cost(), s.coverage().ok()?),
                    [pair_pos2, pair_pos2_cached as _],
                ),
            },
            (true, 3) => (
                SubtableKind::AlternateSubst1,
                AlternateSubstFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.coverage().ok()?)))?,
                [alternate_subst1, alternate_subst1_cached as _],
            ),
            (false, 3) => (
                SubtableKind::CursivePos1,
                CursivePosFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.coverage().ok()?)))?,
                [cursive_pos1, cursive_pos1_cached as _],
            ),
            (true, 4) => (
                SubtableKind::LigatureSubst1,
                LigatureSubstFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.coverage().ok()?)))?,
                [ligature_subst1, ligature_subst1_cached as _],
            ),
            (false, 4) => (
                SubtableKind::MarkBasePos1,
                MarkBasePosFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.mark_coverage().ok()?)))?,
                [mark_base_pos1, mark_base_pos1_cached as _],
            ),
            (true, 5) | (false, 7) => match SequenceContext::read(data).ok()? {
                SequenceContext::Format1(s) => (
                    SubtableKind::ContextFormat1,
                    (s.cache_cost(), s.coverage().ok()?),
                    [context1, context1_cached as _],
                ),
                SequenceContext::Format2(s) => (
                    SubtableKind::ContextFormat2,
                    (s.cache_cost(), s.coverage().ok()?),
                    [context2, context2_cached as _],
                ),
                SequenceContext::Format3(s) => (
                    SubtableKind::ContextFormat3,
                    (s.cache_cost(), s.coverages().get(0).ok()?),
                    [context3, context3_cached as _],
                ),
            },
            (false, 5) => (
                SubtableKind::MarkLigPos1,
                MarkLigPosFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.mark_coverage().ok()?)))?,
                [mark_lig_pos1, mark_lig_pos1_cached as _],
            ),
            (true, 6) | (false, 8) => match ChainedSequenceContext::read(data).ok()? {
                ChainedSequenceContext::Format1(s) => (
                    SubtableKind::ChainedContextFormat1,
                    (s.cache_cost(), s.coverage().ok()?),
                    [chained_context1, chained_context1_cached as _],
                ),
                ChainedSequenceContext::Format2(s) => (
                    SubtableKind::ChainedContextFormat2,
                    (s.cache_cost(), s.coverage().ok()?),
                    [chained_context2, chained_context2_cached as _],
                ),
                ChainedSequenceContext::Format3(s) => (
                    SubtableKind::ChainedContextFormat3,
                    (s.cache_cost(), s.input_coverages().get(0).ok()?),
                    [chained_context3, chained_context3_cached as _],
                ),
            },
            (true, 7) | (false, 9) => {
                let ext = ExtensionSubstFormat1::<'_, ()>::read(data).ok()?;
                let ext_type = ext.extension_lookup_type() as u8;
                let ext_offset = ext.extension_offset().to_u32();
                return Self::new(
                    table_data,
                    subtable_offset.checked_add(ext_offset)?,
                    is_subst,
                    ext_type,
                );
            }
            (false, 6) => (
                SubtableKind::MarkMarkPos1,
                MarkMarkPosFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.mark1_coverage().ok()?)))?,
                [mark_mark_pos1, mark_mark_pos1_cached as _],
            ),
            (true, 8) => (
                SubtableKind::ReverseChainContext,
                ReverseChainSingleSubstFormat1::read(data)
                    .ok()
                    .and_then(|t| Some((t.cache_cost(), t.coverage().ok()?)))?,
                [rev_chain_single_subst1, rev_chain_single_subst1_cached as _],
            ),
            _ => return None,
        };
        let mut digest = hb_set_digest_t::new();
        digest.add_coverage(&coverage);
        let external_cache = match kind {
            SubtableKind::LigatureSubst1 | SubtableKind::PairPos1 => {
                SubtableExternalCache::MappingCache(Box::new(MappingCache::new()))
            }
            SubtableKind::PairPos2 => {
                SubtableExternalCache::PairPosFormat2Cache(Box::new(PairPosFormat2Cache::new()))
            }
            _ => SubtableExternalCache::None,
        };
        Some((
            SubtableInfo {
                kind,
                offset: subtable_offset,
                digest,
                apply_fns,
                external_cache,
            },
            cache_cost,
        ))
    }
}

fn cache_enter(ctx: &mut hb_ot_apply_context_t) -> bool {
    if !ctx.buffer.try_allocate_var(hb_glyph_info_t::SYLLABLE_VAR) {
        return false;
    }
    for info in &mut ctx.buffer.info {
        info.set_syllable(255);
    }
    ctx.new_syllables = Some(255);
    true
}

fn cache_leave(ctx: &mut hb_ot_apply_context_t) {
    ctx.new_syllables = None;
    ctx.buffer.deallocate_var(hb_glyph_info_t::SYLLABLE_VAR);
}
