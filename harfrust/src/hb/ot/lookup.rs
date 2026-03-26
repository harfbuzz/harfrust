use crate::hb::{
    hb_font_t,
    ot_layout::TableIndex,
    ot_layout_gsubgpos::{
        Apply, SubtableExternalCache, SubtableExternalCacheMode, WouldApply, WouldApplyContext,
        OT::hb_ot_apply_context_t,
    },
    set_digest::hb_set_digest_t,
    GlyphInfo,
};
use alloc::vec::Vec;
use read_fonts::tables::{
    gpos::{
        CursivePosFormat1Sanitized, MarkBasePosFormat1Sanitized, MarkLigPosFormat1Sanitized,
        MarkMarkPosFormat1Sanitized, PairPosFormat1Sanitized, PairPosFormat2Sanitized,
        SinglePosFormat1Sanitized, SinglePosFormat2Sanitized,
    },
    gsub::SingleSubstFormat2Sanitized,
};
use read_fonts::tables::{
    gpos::{GposSanitized, PairPosSanitized, SinglePosSanitized},
    gsub::{
        AlternateSubstFormat1Sanitized, ExtensionSubstFormat1Sanitized, GsubSanitized,
        LigatureSubstFormat1Sanitized, MultipleSubstFormat1Sanitized,
        ReverseChainSingleSubstFormat1Sanitized, SingleSubstFormat1Sanitized, SingleSubstSanitized,
    },
    layout::{
        ChainedSequenceContextFormat1Sanitized, ChainedSequenceContextFormat2Sanitized,
        ChainedSequenceContextFormat3Sanitized, ChainedSequenceContextSanitized,
        CoverageTableSanitized, SequenceContextFormat1Sanitized, SequenceContextFormat2Sanitized,
        SequenceContextFormat3Sanitized, SequenceContextSanitized,
    },
};
use read_fonts::FontPtr;
use read_fonts::ReadSanitized;
use read_fonts::{
    tables::{
        gsub::ExtensionSubstFormat1,
        layout::{Lookup, LookupFlag},
    },
    FontData, FontRead, Offset, ReadError,
};

pub struct LookupData<'a> {
    /// Offset of the lookup from the base of the layout table.
    offset: usize,
    /// True if the lookup comes from GSUB.
    is_subst: bool,
    /// Data of the layout table.
    table_data: FontData<'a>,
}

pub trait LookupHost<'a> {
    fn lookup_count(&self) -> u16;
    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError>;
}

impl<'a> LookupHost<'a> for GsubSanitized<'a> {
    fn lookup_count(&self) -> u16 {
        self.lookup_list().lookup_count()
    }

    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError> {
        let list = self.lookup_list();
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
            table_data: self.offset_ptr().into_font_data(),
        })
    }
}

impl<'a> LookupHost<'a> for GposSanitized<'a> {
    fn lookup_count(&self) -> u16 {
        self.lookup_list().lookup_count()
    }

    fn lookup_data(&self, index: u16) -> Result<LookupData<'a>, ReadError> {
        let list = self.lookup_list();
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
            table_data: self.offset_ptr().into_font_data(),
        })
    }
}

#[cfg(feature = "std")]
mod cache {
    use super::{LookupHost, LookupInfo};
    use std::sync::OnceLock;

    #[derive(Default)]
    pub(crate) struct LookupCache {
        lookups: Vec<OnceLock<Option<Box<LookupInfo>>>>,
    }

    impl LookupCache {
        pub fn new<'a>(host: &impl LookupHost<'a>) -> Self {
            let mut lookups = Vec::new();
            lookups.resize_with(host.lookup_count() as usize, Default::default);
            Self { lookups }
        }

        pub fn get<'a>(&self, host: &impl LookupHost<'a>, index: u16) -> Option<&LookupInfo> {
            self.lookups
                .get(index as usize)?
                .get_or_init(|| {
                    host.lookup_data(index)
                        .ok()
                        .and_then(|data| LookupInfo::new(&data))
                        .map(Box::new)
                })
                .as_ref()
                .map(|v| &**v)
        }
    }
}

#[cfg(not(feature = "std"))]
mod cache {
    use super::{LookupHost, LookupInfo, Vec};

    #[derive(Default)]
    pub(crate) struct LookupCache {
        lookups: Vec<Option<LookupInfo>>,
    }

    impl LookupCache {
        pub fn new<'a>(host: &impl LookupHost<'a>) -> Self {
            let count = host.lookup_count();
            let mut lookups = Vec::with_capacity(count as usize);
            for i in 0..count {
                lookups.push(
                    host.lookup_data(i)
                        .ok()
                        .and_then(|data| LookupInfo::new(&data)),
                );
            }
            Self { lookups }
        }

        pub fn get<'a>(&self, _host: &impl LookupHost<'a>, index: u16) -> Option<&LookupInfo> {
            self.lookups.get(index as usize)?.as_ref()
        }
    }
}

pub(crate) use cache::LookupCache;

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

/// Cached information about a lookup.
#[derive(Default)]
pub struct LookupInfo {
    pub props: u32,
    pub is_subst: bool,
    pub is_reversed: bool,
    pub digest: hb_set_digest_t,
    pub subtable_cache_user_idx: Option<usize>,
    pub subtables: Vec<SubtableInfo>,
}

impl LookupInfo {
    pub fn new(data: &LookupData) -> Option<Self> {
        let mut info = Self {
            is_subst: data.is_subst,
            ..Default::default()
        };
        let lookup_data = data.table_data.split_off(data.offset)?;
        let lookup: Lookup<()> = Lookup::read(lookup_data).ok()?;
        let lookup_type = lookup.lookup_type();
        let lookup_flag = lookup.lookup_flag();
        info.props = u32::from(lookup.lookup_flag().to_bits());
        if lookup_flag.to_bits() & LookupFlag::USE_MARK_FILTERING_SET.to_bits() != 0 {
            info.props |= (lookup.mark_filtering_set().unwrap_or_default() as u32) << 16;
        }
        if data.is_subst {
            info.is_reversed =
                is_reversed(data.table_data, &lookup, data.offset).unwrap_or_default();
        }
        let mut subtable_cache_user_cost = 0;
        info.subtables.reserve(lookup.sub_table_count() as usize);
        for (idx, subtable_offset) in lookup.subtable_offsets().iter().enumerate() {
            let cache_mode = if idx < 8 {
                SubtableExternalCacheMode::Full
            } else {
                SubtableExternalCacheMode::Small
            };
            let subtable_offset = subtable_offset.get().to_usize() + data.offset;
            if let Some((subtable_info, cache_cost)) = SubtableInfo::new(
                data.table_data,
                subtable_offset as u32,
                data.is_subst,
                lookup_type as u8,
                cache_mode,
            ) {
                info.digest.union(&subtable_info.digest);
                if cache_cost > subtable_cache_user_cost {
                    info.subtable_cache_user_idx = Some(info.subtables.len());
                    subtable_cache_user_cost = cache_cost;
                }
                info.subtables.push(subtable_info);
            }
        }
        info.subtables.shrink_to_fit();
        Some(info)
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
        use_hot_subtable_cache: bool,
    ) -> Option<()> {
        let glyph = ctx.buffer.cur(0).glyph_id;
        for (subtable_idx, subtable_info) in self.subtables.iter().enumerate() {
            if !subtable_info.digest.may_have(glyph) {
                continue;
            }
            let is_cached =
                use_hot_subtable_cache && (self.subtable_cache_user_idx == Some(subtable_idx));
            if subtable_info.apply(ctx, table_data, is_cached).is_some() {
                return Some(());
            }
        }
        None
    }

    pub(crate) fn cache_enter(&self, ctx: &mut hb_ot_apply_context_t) -> bool {
        let Some(idx) = self.subtable_cache_user_idx else {
            return false;
        };
        let Some(subtable_info) = self.subtables.get(idx) else {
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
    pub(crate) fn cache_leave(&self, ctx: &mut hb_ot_apply_context_t) {
        let Some(idx) = self.subtable_cache_user_idx else {
            return;
        };
        let Some(subtable_info) = self.subtables.get(idx) else {
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
        if !self.digest.may_have(glyph.into()) {
            return Some(false);
        }
        let table_index = if self.is_subst {
            TableIndex::GSUB
        } else {
            TableIndex::GPOS
        };
        let table_data = face.ot_tables.table_data(table_index)?;
        for subtable_info in &self.subtables {
            if !subtable_info.digest.may_have(glyph.into()) {
                continue;
            }
            let Some(data) = table_data.get(subtable_info.offset as usize..) else {
                continue;
            };
            let data = FontData::new(data);
            let data = FontPtr::new(data);
            let result = match subtable_info.kind {
                SubtableKind::SingleSubst1 => unsafe {
                    SingleSubstFormat1Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::SingleSubst2 => unsafe {
                    SingleSubstFormat2Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::MultipleSubst1 => unsafe {
                    MultipleSubstFormat1Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::AlternateSubst1 => unsafe {
                    AlternateSubstFormat1Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::LigatureSubst1 => unsafe {
                    LigatureSubstFormat1Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::ReverseChainContext => unsafe {
                    ReverseChainSingleSubstFormat1Sanitized::read_sanitized(data, &())
                        .would_apply(ctx)
                },
                SubtableKind::ContextFormat1 => unsafe {
                    SequenceContextFormat1Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::ContextFormat2 => unsafe {
                    SequenceContextFormat2Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::ContextFormat3 => unsafe {
                    SequenceContextFormat3Sanitized::read_sanitized(data, &()).would_apply(ctx)
                },
                SubtableKind::ChainedContextFormat1 => unsafe {
                    ChainedSequenceContextFormat1Sanitized::read_sanitized(data, &())
                        .would_apply(ctx)
                },
                SubtableKind::ChainedContextFormat2 => unsafe {
                    ChainedSequenceContextFormat2Sanitized::read_sanitized(data, &())
                        .would_apply(ctx)
                },
                SubtableKind::ChainedContextFormat3 => unsafe {
                    ChainedSequenceContextFormat3Sanitized::read_sanitized(data, &())
                        .would_apply(ctx)
                },
                _ => continue,
            };
            if result {
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
    ($apply:ident, $apply_cached:ident, $ty:ty) => {
        fn $apply(
            ctx: &mut hb_ot_apply_context_t,
            external_cache: &SubtableExternalCache,
            table_data: &[u8],
        ) -> Option<()> {
            let t = unsafe { <$ty>::read_sanitized(FontPtr::new(FontData::new(table_data)), &()) };
            t.apply_with_external_cache(ctx, external_cache)
        }

        fn $apply_cached(
            ctx: &mut hb_ot_apply_context_t,
            external_cache: &SubtableExternalCache,
            table_data: &[u8],
        ) -> Option<()> {
            let t = unsafe { <$ty>::read_sanitized(FontPtr::new(FontData::new(table_data)), &()) };
            t.apply_cached(ctx, external_cache)
        }
    };
}

apply_fns!(
    single_subst1,
    single_subst1_cached,
    SingleSubstFormat1Sanitized
);
apply_fns!(
    single_subst2,
    single_subst2_cached,
    SingleSubstFormat2Sanitized
);
apply_fns!(
    multiple_subst1,
    multiple_subst1_cached,
    MultipleSubstFormat1Sanitized
);
apply_fns!(
    alternate_subst1,
    alternate_subst1_cached,
    AlternateSubstFormat1Sanitized
);
apply_fns!(
    ligature_subst1,
    ligature_subst1_cached,
    LigatureSubstFormat1Sanitized
);
apply_fns!(single_pos1, single_pos1_cached, SinglePosFormat1Sanitized);
apply_fns!(single_pos2, single_pos2_cached, SinglePosFormat2Sanitized);
apply_fns!(pair_pos1, pair_pos1_cached, PairPosFormat1Sanitized);
apply_fns!(pair_pos2, pair_pos2_cached, PairPosFormat2Sanitized);
apply_fns!(
    cursive_pos1,
    cursive_pos1_cached,
    CursivePosFormat1Sanitized
);
apply_fns!(
    mark_base_pos1,
    mark_base_pos1_cached,
    MarkBasePosFormat1Sanitized
);
apply_fns!(
    mark_mark_pos1,
    mark_mark_pos1_cached,
    MarkMarkPosFormat1Sanitized
);
apply_fns!(
    mark_lig_pos1,
    mark_lig_pos1_cached,
    MarkLigPosFormat1Sanitized
);
apply_fns!(context1, context1_cached, SequenceContextFormat1Sanitized);
apply_fns!(context2, context2_cached, SequenceContextFormat2Sanitized);
apply_fns!(context3, context3_cached, SequenceContextFormat3Sanitized);
apply_fns!(
    chained_context1,
    chained_context1_cached,
    ChainedSequenceContextFormat1Sanitized
);
apply_fns!(
    chained_context2,
    chained_context2_cached,
    ChainedSequenceContextFormat2Sanitized
);
apply_fns!(
    chained_context3,
    chained_context3_cached,
    ChainedSequenceContextFormat3Sanitized
);
apply_fns!(
    rev_chain_single_subst1,
    rev_chain_single_subst1_cached,
    ReverseChainSingleSubstFormat1Sanitized
);

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
        cache_mode: SubtableExternalCacheMode,
    ) -> Option<(Self, u32)> {
        let data = table_data.split_off(subtable_offset as usize)?;
        let data = FontPtr::new(data);
        let maybe_external_cache = |s: &dyn Apply| s.external_cache_create(cache_mode);
        let (kind, (external_cache, cache_cost, coverage), apply_fns): (
            SubtableKind,
            (SubtableExternalCache, u32, CoverageTableSanitized),
            [SubtableApplyFn; 2],
        ) = match (is_subst, lookup_type) {
            (true, 1) => match unsafe { SingleSubstSanitized::read_sanitized(data, &()) } {
                SingleSubstSanitized::Format1(s) => (
                    SubtableKind::SingleSubst1,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [single_subst1, single_subst1_cached as _],
                ),
                SingleSubstSanitized::Format2(s) => (
                    SubtableKind::SingleSubst2,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [single_subst2, single_subst2_cached as _],
                ),
            },
            (false, 1) => match unsafe { SinglePosSanitized::read_sanitized(data, &()) } {
                SinglePosSanitized::Format1(s) => (
                    SubtableKind::SinglePos1,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [single_pos1, single_pos1_cached as _],
                ),
                SinglePosSanitized::Format2(s) => (
                    SubtableKind::SinglePos2,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [single_pos2, single_pos2_cached as _],
                ),
            },
            (true, 2) => (
                SubtableKind::MultipleSubst1,
                {
                    let t = unsafe { MultipleSubstFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.coverage())
                },
                [multiple_subst1, multiple_subst1_cached as _],
            ),
            (false, 2) => match unsafe { PairPosSanitized::read_sanitized(data, &()) } {
                PairPosSanitized::Format1(s) => (
                    SubtableKind::PairPos1,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [pair_pos1, pair_pos1_cached as _],
                ),
                PairPosSanitized::Format2(s) => (
                    SubtableKind::PairPos2,
                    (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                    [pair_pos2, pair_pos2_cached as _],
                ),
            },
            (true, 3) => (
                SubtableKind::AlternateSubst1,
                {
                    let t = unsafe { AlternateSubstFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.coverage())
                },
                [alternate_subst1, alternate_subst1_cached as _],
            ),
            (false, 3) => (
                SubtableKind::CursivePos1,
                {
                    let t = unsafe { CursivePosFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.coverage())
                },
                [cursive_pos1, cursive_pos1_cached as _],
            ),
            (true, 4) => (
                SubtableKind::LigatureSubst1,
                {
                    let t = unsafe { LigatureSubstFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.coverage())
                },
                [ligature_subst1, ligature_subst1_cached as _],
            ),
            (false, 4) => (
                SubtableKind::MarkBasePos1,
                {
                    let t = unsafe { MarkBasePosFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.mark_coverage())
                },
                [mark_base_pos1, mark_base_pos1_cached as _],
            ),
            (true, 5) | (false, 7) => {
                match unsafe { SequenceContextSanitized::read_sanitized(data, &()) } {
                    SequenceContextSanitized::Format1(s) => (
                        SubtableKind::ContextFormat1,
                        (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                        [context1, context1_cached as _],
                    ),
                    SequenceContextSanitized::Format2(s) => (
                        SubtableKind::ContextFormat2,
                        (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                        [context2, context2_cached as _],
                    ),
                    SequenceContextSanitized::Format3(s) => (
                        SubtableKind::ContextFormat3,
                        (
                            maybe_external_cache(&s),
                            s.cache_cost(),
                            s.coverages().get(0)?,
                        ),
                        [context3, context3_cached as _],
                    ),
                }
            }
            (false, 5) => (
                SubtableKind::MarkLigPos1,
                {
                    let t = unsafe { MarkLigPosFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.mark_coverage())
                },
                [mark_lig_pos1, mark_lig_pos1_cached as _],
            ),
            (true, 6) | (false, 8) => {
                match unsafe { ChainedSequenceContextSanitized::read_sanitized(data, &()) } {
                    ChainedSequenceContextSanitized::Format1(s) => (
                        SubtableKind::ChainedContextFormat1,
                        (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                        [chained_context1, chained_context1_cached as _],
                    ),
                    ChainedSequenceContextSanitized::Format2(s) => (
                        SubtableKind::ChainedContextFormat2,
                        (maybe_external_cache(&s), s.cache_cost(), s.coverage()),
                        [chained_context2, chained_context2_cached as _],
                    ),
                    ChainedSequenceContextSanitized::Format3(s) => (
                        SubtableKind::ChainedContextFormat3,
                        (
                            maybe_external_cache(&s),
                            s.cache_cost(),
                            s.input_coverages().get(0)?,
                        ),
                        [chained_context3, chained_context3_cached as _],
                    ),
                }
            }
            (true, 7) | (false, 9) => {
                let ext =
                    unsafe { ExtensionSubstFormat1Sanitized::<'_, ()>::read_sanitized(data, &()) };
                let ext_type = ext.extension_lookup_type() as u8;
                let ext_offset = ext.extension_offset().to_u32();
                return Self::new(
                    table_data,
                    subtable_offset.checked_add(ext_offset)?,
                    is_subst,
                    ext_type,
                    cache_mode,
                );
            }
            (false, 6) => (
                SubtableKind::MarkMarkPos1,
                {
                    let t = unsafe { MarkMarkPosFormat1Sanitized::read_sanitized(data, &()) };
                    (maybe_external_cache(&t), t.cache_cost(), t.mark1_coverage())
                },
                [mark_mark_pos1, mark_mark_pos1_cached as _],
            ),
            (true, 8) => (
                SubtableKind::ReverseChainContext,
                {
                    let t = unsafe {
                        ReverseChainSingleSubstFormat1Sanitized::read_sanitized(data, &())
                    };
                    (maybe_external_cache(&t), t.cache_cost(), t.coverage())
                },
                [rev_chain_single_subst1, rev_chain_single_subst1_cached as _],
            ),
            _ => return None,
        };
        let mut digest = hb_set_digest_t::new();
        digest.add_coverage(&coverage);
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
    if !ctx.buffer.try_allocate_var(GlyphInfo::SYLLABLE_VAR) {
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
    ctx.buffer.deallocate_var(GlyphInfo::SYLLABLE_VAR);
}
