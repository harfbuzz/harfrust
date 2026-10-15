//! The lookups of GSUB and GPOS, parsed once and kept with the font tables.
//!
//! When a lookup is first used, each of its subtables is read into a
//! [`CachedSubtableKind`] that holds the parsed table together with
//! everything derived from it: coverage and class tables, digests and glyph
//! lookup caches. Applying a subtable is then one indirect call through
//! [`CachedSubtable`] that checks the kind it expects and runs.
//!
//! The parsed tables borrow the font data, so a lookup is kept in a
//! [`yoke::Yoke`] with a reference to the font's shared tables; see
//! [`CachedLookupRef`].

mod build;
mod context;
mod coverage;
mod gpos;
mod gsub;

use super::apply::{ApplyContext, MappingCache, WouldApplyContext};
use crate::once::Once;
use crate::set_digest::SetDigest;
use crate::GlyphInfo;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::ops::Range;
pub(crate) use coverage::{ClassDef, Coverage};
use gpos::SecondGlyph;
use read_fonts::model::Tables;
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};
use yoke::erased::ErasedArcCart;
use yoke::{Yoke, Yokeable};

/// The lookups of GSUB or GPOS, each parsed on first use.
#[derive(Default)]
pub(crate) struct LookupCache {
    lookups: Vec<Once<Option<Box<CachedLookupRef>>>>,
    /// The font's shared tables, which lookups are cached against.
    tables: Option<Arc<Tables>>,
    is_subst: bool,
}

impl LookupCache {
    pub const EMPTY: Self = Self {
        lookups: Vec::new(),
        tables: None,
        is_subst: false,
    };

    pub fn new(tables: Option<Arc<Tables>>, is_subst: bool) -> Self {
        let count = tables
            .as_deref()
            .map_or(0, |tables| build::lookup_count(tables, is_subst));
        let mut lookups = Vec::new();
        lookups.resize_with(usize::from(count), Once::new);
        Self {
            lookups,
            tables,
            is_subst,
        }
    }

    pub fn get(&self, index: u16) -> Option<&CachedLookup<'_>> {
        let cell = self.lookups.get(usize::from(index))?;
        // Checked before `get_or_init`, whose initializer would otherwise
        // weigh down the path taken every time.
        let lookup = match cell.get() {
            Some(lookup) => lookup,
            None => cell.get_or_init(|| {
                cache_lookup(self.tables.clone()?, self.is_subst, index).map(Box::new)
            }),
        };
        lookup.as_deref().map(Yoke::get)
    }
}

/// A cached lookup kept with the data it borrows: the font's tables, or
/// the bytes of a lookup made up for a font that lacks one.
pub(crate) type CachedLookupRef = Yoke<CachedLookup<'static>, ErasedArcCart>;

/// Parses lookup `index` of GSUB, or GPOS if `is_subst` is false.
pub(crate) fn cache_lookup(
    tables: Arc<Tables>,
    is_subst: bool,
    index: u16,
) -> Option<CachedLookupRef> {
    Yoke::try_attach_to_cart(tables, |tables| {
        build::build_lookup(tables, is_subst, index).ok_or(())
    })
    .ok()
    .map(Yoke::erase_arc_cart)
}

/// Parses a substitution lookup serialized on its own, as the Arabic
/// fallback makes them.
pub(crate) fn cache_synthesized_subst(data: Vec<u8>) -> Option<CachedLookupRef> {
    Yoke::try_attach_to_cart(Arc::new(data), |data: &Vec<u8>| {
        build::build_lookup_at(FontData::new(data), 0, true).ok_or(())
    })
    .ok()
    .map(Yoke::erase_arc_cart)
}

/// A lookup with its subtables parsed.
#[derive(Yokeable)]
pub struct CachedLookup<'a> {
    pub props: u32,
    pub is_subst: bool,
    pub is_reversed: bool,
    pub digest: SetDigest,
    pub digest_second: SetDigest,
    /// The subtable that uses the buffer's glyph-info cache, if any.
    hot_subtable: Option<usize>,
    /// Whether the hot subtable keeps glyph classes in the glyph-info cache.
    uses_class_cache: bool,
    /// Applies the subtables in turn, chosen for their number and kinds.
    walk: WalkFn,
    subtables: Vec<CachedSubtable<'a>>,
}

/// Applies the subtables of a lookup until one applies.
type WalkFn = fn(&CachedLookup, &mut ApplyContext, bool) -> Option<()>;

impl CachedLookup<'_> {
    pub fn props(&self) -> u32 {
        self.props
    }

    pub fn is_reverse(&self) -> bool {
        self.is_reversed
    }

    pub fn digest(&self) -> &SetDigest {
        &self.digest
    }

    pub fn digest_second(&self) -> &SetDigest {
        &self.digest_second
    }

    #[inline]
    pub(crate) fn apply(&self, ctx: &mut ApplyContext, use_hot_subtable_cache: bool) -> Option<()> {
        (self.walk)(self, ctx, use_hot_subtable_cache)
    }

    /// Prepares the glyph-info class cache for the hot subtable, returning
    /// true if it is in use.
    pub(crate) fn cache_enter(&self, ctx: &mut ApplyContext) -> bool {
        if !self.uses_class_cache || !ctx.buffer.try_allocate_var(GlyphInfo::SYLLABLE_VAR) {
            return false;
        }
        for info in &mut ctx.buffer.info {
            info.set_syllable(255);
        }
        ctx.new_syllables = Some(255);
        true
    }

    pub(crate) fn cache_leave(&self, ctx: &mut ApplyContext) {
        if self.uses_class_cache {
            ctx.new_syllables = None;
            ctx.buffer.deallocate_var(GlyphInfo::SYLLABLE_VAR);
        }
    }

    pub fn would_apply(&self, ctx: &WouldApplyContext) -> Option<bool> {
        let glyph = ctx.glyphs[0].into();
        if !self.digest.may_have(glyph) {
            return Some(false);
        }
        for subtable in &self.subtables {
            if subtable.digest.may_have(glyph)
                && subtable.kind.would_apply(ctx, &subtable.coverage) == Some(true)
            {
                return Some(true);
            }
        }
        None
    }
}

/// Applies a lookup's only subtable. Its digest equals the lookup's,
/// which the caller has already tested.
fn walk_single(
    lookup: &CachedLookup,
    ctx: &mut ApplyContext,
    use_hot_subtable_cache: bool,
) -> Option<()> {
    let glyph = ctx.buffer.cur(0)?.glyph_id;
    let subtable = lookup.subtables.first()?;
    let index = subtable.coverage_index(glyph)?;
    let is_cached = use_hot_subtable_cache && lookup.hot_subtable == Some(0);
    subtable.apply[is_cached as usize](subtable, ctx, index)
}

/// Applies subtables in turn.
fn walk_each(
    lookup: &CachedLookup,
    ctx: &mut ApplyContext,
    use_hot_subtable_cache: bool,
) -> Option<()> {
    let glyph = ctx.buffer.cur(0)?.glyph_id;
    for (position, subtable) in lookup.subtables.iter().enumerate() {
        if !subtable.digest.may_have(glyph) {
            continue;
        }
        let Some(index) = subtable.coverage_index(glyph) else {
            continue;
        };
        let is_cached = use_hot_subtable_cache && lookup.hot_subtable == Some(position);
        if subtable.apply[is_cached as usize](subtable, ctx, index).is_some() {
            return Some(());
        }
    }
    None
}

/// Applies subtables that adjust pairs, searching for the second glyph
/// of the pair at most once for all of them.
fn walk_sharing_second_glyph(lookup: &CachedLookup, ctx: &mut ApplyContext, _: bool) -> Option<()> {
    let glyph = ctx.buffer.cur(0)?.glyph_id;
    let mut second = SecondGlyph::Unknown;
    for subtable in &lookup.subtables {
        if !subtable.digest.may_have(glyph) {
            continue;
        }
        let Some(index) = subtable.coverage_index(glyph) else {
            continue;
        };
        if (subtable.apply_sharing)(subtable, ctx, index, &mut second).is_some() {
            return Some(());
        }
    }
    None
}

/// A parsed subtable and the functions that apply it.
pub(crate) struct CachedSubtable<'a> {
    kind: CachedSubtableKind<'a>,
    /// The glyphs the subtable may apply to.
    digest: SetDigest,
    /// The coverage of the glyph that a match starts at.
    coverage: Coverage<'a>,
    coverage_cache: Option<Box<MappingCache>>,
    /// Applies `kind` without and with the glyph-info class cache.
    apply: [ApplyFn; 2],
    apply_sharing: ApplySharingFn,
}

impl CachedSubtable<'_> {
    /// The coverage index of the glyph a match would start at.
    #[inline(always)]
    fn coverage_index(&self, glyph: u32) -> Option<u32> {
        let Some(cache) = &self.coverage_cache else {
            return self.coverage.get(glyph);
        };
        match cache.get(glyph) {
            Some(MappingCache::MAX_VALUE) => None,
            Some(index) => Some(index),
            None => self.coverage_index_uncached(glyph, cache),
        }
    }

    /// Looks up a glyph the cache doesn't hold, and remembers it. Kept out
    /// of line so the walks inline only the cache hit.
    #[inline(never)]
    fn coverage_index_uncached(&self, glyph: u32, cache: &MappingCache) -> Option<u32> {
        coverage_index_cached(|gid| self.coverage.get(gid), glyph.into(), cache)
    }
}

/// Applies a subtable of one kind at the given coverage index, returning
/// `None` for any other kind.
type ApplyFn = fn(&CachedSubtable, &mut ApplyContext, u32) -> Option<()>;

/// Applies a subtable of one kind, sharing the search for a pair's second
/// glyph with the other subtables of its lookup.
type ApplySharingFn = fn(&CachedSubtable, &mut ApplyContext, u32, &mut SecondGlyph) -> Option<()>;

/// How a kind of subtable applies.
struct KindFns {
    /// Applies without and with the glyph-info class cache.
    apply: [ApplyFn; 2],
    apply_sharing: ApplySharingFn,
    /// Whether the kind adjusts pairs and uses the shared second glyph.
    shares_second_glyph: bool,
}

/// The derived data of one kind of subtable.
trait IntoKind<'a>: Sized {
    const FNS: &'static KindFns;

    fn into_kind(self) -> CachedSubtableKind<'a>;
}

/// The functions of a kind that applies plainly, adjusts pairs, or keeps
/// classes in the glyph-info cache.
macro_rules! kind_fns {
    ($kind:ident) => {{
        const APPLY: ApplyFn = |subtable, ctx, index| match &subtable.kind {
            CachedSubtableKind::$kind(data) => data.apply(ctx, index),
            _ => None,
        };
        &KindFns {
            apply: [APPLY, APPLY],
            apply_sharing: never_shares,
            shares_second_glyph: false,
        }
    }};
    ($kind:ident adjusts_pairs) => {{
        const APPLY_SHARING: ApplySharingFn = |subtable, ctx, index, second| match &subtable.kind {
            CachedSubtableKind::$kind(data) => data.apply(ctx, index, second),
            _ => None,
        };
        const APPLY: ApplyFn =
            |subtable, ctx, index| APPLY_SHARING(subtable, ctx, index, &mut SecondGlyph::Unknown);
        &KindFns {
            apply: [APPLY, APPLY],
            apply_sharing: APPLY_SHARING,
            shares_second_glyph: true,
        }
    }};
    ($kind:ident uses_class_cache) => {
        &KindFns {
            apply: [
                |subtable, ctx, index| match &subtable.kind {
                    CachedSubtableKind::$kind(data) => data.apply(ctx, index, false),
                    _ => None,
                },
                |subtable, ctx, index| match &subtable.kind {
                    CachedSubtableKind::$kind(data) => data.apply(ctx, index, true),
                    _ => None,
                },
            ],
            apply_sharing: never_shares,
            shares_second_glyph: false,
        }
    };
}

/// The `apply_sharing` of kinds that don't adjust pairs, which no walk calls.
fn never_shares(
    _: &CachedSubtable,
    _: &mut ApplyContext,
    _: u32,
    _: &mut SecondGlyph,
) -> Option<()> {
    None
}

/// Declares the subtable kinds, the data each holds, and how it applies.
macro_rules! subtable_kinds {
    ($($kind:ident($data:ty) $($how:ident)?;)*) => {
        /// A parsed subtable with the data derived from it.
        pub(crate) enum CachedSubtableKind<'a> {
            $($kind($data),)*
        }

        $(
            impl<'a> IntoKind<'a> for $data {
                const FNS: &'static KindFns = kind_fns!($kind $($how)?);

                fn into_kind(self) -> CachedSubtableKind<'a> {
                    CachedSubtableKind::$kind(self)
                }
            }
        )*
    };
}

subtable_kinds! {
    SingleSubst1(gsub::SingleSubst1);
    SingleSubst2(gsub::SingleSubst2<'a>);
    MultipleSubst1(gsub::MultipleSubst1<'a>);
    AlternateSubst1(gsub::AlternateSubst1<'a>);
    LigatureSubst1(gsub::LigatureSubst1<'a>);
    ReverseChainSubst1(gsub::ReverseChainSubst1<'a>);
    SinglePos1(gpos::SinglePos1<'a>);
    SinglePos2(gpos::SinglePos2<'a>);
    PairPos1(gpos::PairPos1<'a>) adjusts_pairs;
    PairPos2(gpos::PairPos2<'a>) adjusts_pairs;
    PairKern1(gpos::PairKern1<'a>) adjusts_pairs;
    PairKern2(gpos::PairKern2<'a>) adjusts_pairs;
    CursivePos1(gpos::CursivePos1<'a>);
    MarkBasePos1(gpos::MarkBasePos1<'a>);
    MarkLigPos1(gpos::MarkLigPos1<'a>);
    MarkMarkPos1(gpos::MarkMarkPos1<'a>);
    ChainContext1(context::ChainContext1<'a>);
    ChainContext2(context::ChainContext2<'a>) uses_class_cache;
    ChainContext3(context::ChainContext3<'a>);
}

impl CachedSubtableKind<'_> {
    /// Returns whether a substitution subtable would apply, or `None` for a
    /// positioning subtable.
    fn would_apply(&self, ctx: &WouldApplyContext, coverage: &Coverage) -> Option<bool> {
        Some(match self {
            // These substitute a single glyph.
            Self::SingleSubst1(_)
            | Self::SingleSubst2(_)
            | Self::MultipleSubst1(_)
            | Self::AlternateSubst1(_)
            | Self::ReverseChainSubst1(_) => {
                ctx.glyphs.len() == 1 && coverage.covers(ctx.glyphs[0])
            }
            Self::LigatureSubst1(t) => t.would_apply(ctx, coverage),
            Self::ChainContext1(t) => t.would_apply(ctx, coverage),
            Self::ChainContext2(t) => t.would_apply(ctx, coverage),
            Self::ChainContext3(t) => t.would_apply(ctx, coverage),
            _ => return None,
        })
    }
}

/// What a lookup needs to know about one of its subtables.
trait SubtableDigests<'a> {
    /// Whether looking up the first glyph's coverage gains from a cache.
    const CACHES_COVERAGE: bool = false;

    /// The glyphs that may follow the first one in a match.
    fn digest_second(&self) -> SetDigest {
        SetDigest::full()
    }

    /// How much the subtable gains from the glyph-info class cache.
    fn cache_cost(&self) -> u32 {
        0
    }
}

/// Big-endian unsigned integers of one width, two or three bytes, read from
/// an array in place. Glyph IDs, offsets and rule values of the original and
/// extended formats share code through it.
#[derive(Clone, Copy, Default)]
pub(crate) struct Uints<'a> {
    bytes: &'a [u8],
    len: u32,
    width: u8,
}

impl<'a> Uints<'a> {
    /// The integers of `width` bytes in `range` of `data`, or `None` if the
    /// range is out of bounds.
    pub fn new(data: FontData<'a>, range: Range<usize>, width: usize) -> Option<Self> {
        let bytes = data.as_bytes().get(range)?;
        let len = bytes.len() / width;
        Some(Self {
            bytes: &bytes[..len * width],
            len: len as u32,
            width: width as u8,
        })
    }

    /// `count` integers of `width` bytes at `start` of `data`.
    #[inline(always)]
    pub fn at(data: FontData<'a>, start: usize, count: usize, width: usize) -> Option<Self> {
        let end = start.checked_add(count.checked_mul(width)?)?;
        let bytes = data.as_bytes().get(start..end)?;
        Some(Self {
            bytes,
            len: count as u32,
            width: width as u8,
        })
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<u32> {
        // Branching between two fixed widths reads faster than reading any
        // width. Indices are at most 24 bits, so the products can't
        // overflow.
        if self.width == 3 {
            read_be::<3>(self.bytes, index * 3)
        } else {
            read_be::<2>(self.bytes, index * 2)
        }
    }

    #[inline(always)]
    pub fn first(&self) -> Option<u32> {
        self.get(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }
}

/// The unsigned integer of `width` bytes at `pos`, which the caller has
/// checked is in bounds.
#[inline(always)]
pub(crate) fn read_uint(bytes: &[u8], pos: usize, width: usize) -> u32 {
    be_uint(&bytes[pos..pos + width])
}

/// The big-endian unsigned integer of `W` bytes at `pos`, or `None` if it
/// is out of bounds.
#[inline(always)]
pub(crate) fn read_be<const W: usize>(bytes: &[u8], pos: usize) -> Option<u32> {
    let b = bytes.get(pos..pos.checked_add(W)?)?;
    Some(match W {
        2 => u32::from(u16::from_be_bytes([b[0], b[1]])),
        3 => u32::from_be_bytes([0, b[0], b[1], b[2]]),
        _ => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
    })
}

/// The big-endian unsigned integer of two to four bytes.
#[inline(always)]
pub(crate) fn be_uint(bytes: &[u8]) -> u32 {
    match *bytes {
        [a, b] => u32::from(u16::from_be_bytes([a, b])),
        [a, b, c] => u32::from_be_bytes([0, a, b, c]),
        [a, b, c, d] => u32::from_be_bytes([a, b, c, d]),
        _ => 0,
    }
}

/// Unsigned integers of `W` bytes, read in place: glyph IDs, classes or
/// offsets whose width a hot path fixes at compile time.
#[derive(Clone, Copy, Default)]
pub(crate) struct Values<'a, const W: usize> {
    bytes: &'a [u8],
}

impl<'a, const W: usize> Values<'a, W> {
    #[inline(always)]
    pub fn at(data: FontData<'a>, start: usize, count: usize) -> Option<Self> {
        Some(Self {
            bytes: data.as_bytes().get(start..start + count * W)?,
        })
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.bytes.len() / W
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<u32> {
        read_be::<W>(self.bytes, index * W)
    }

    pub fn first(&self) -> Option<u32> {
        self.get(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }
}

/// An array of offsets to child tables, with the data they are relative to.
#[derive(Clone, Copy, Default)]
struct Offsets<'a> {
    data: FontData<'a>,
    offsets: Uints<'a>,
}

impl<'a> Offsets<'a> {
    /// The offsets of `width` bytes in `range` of `data`, relative to it.
    fn new(data: FontData<'a>, range: Range<usize>, width: usize) -> Self {
        Self {
            data,
            offsets: Uints::new(data, range, width).unwrap_or_default(),
        }
    }

    fn is_null(&self, index: usize) -> bool {
        self.offsets.get(index) == Some(0)
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    /// The data of the child table at `index`, or `None` if its offset is
    /// null or out of bounds.
    #[inline(always)]
    fn data(&self, index: usize) -> Option<FontData<'a>> {
        let offset = self.offsets.get(index)? as usize;
        if offset == 0 {
            return None;
        }
        self.data.split_off(offset)
    }

    /// Reads the child table at `index`, or `None` if its offset is null
    /// or it can't be read.
    fn get<T: FontRead<'a, Args = ()>>(&self, index: usize) -> Option<T> {
        T::read(self.data(index)?).ok()
    }
}

/// Returns the coverage index of `gid`, remembering it in `cache`.
#[inline(always)]
fn coverage_index_cached(
    coverage: impl Fn(GlyphId) -> Option<u32>,
    gid: GlyphId,
    cache: &MappingCache,
) -> Option<u32> {
    if let Some(index) = cache.get(gid.into()) {
        if index == MappingCache::MAX_VALUE {
            None
        } else {
            Some(index)
        }
    } else {
        let index = coverage(gid);
        if let Some(index) = index {
            if index < MappingCache::MAX_VALUE {
                cache.set(gid.into(), index);
            }
            Some(index)
        } else {
            cache.set(gid.into(), MappingCache::MAX_VALUE);
            None
        }
    }
}

/// Returns the class of `gid`, remembering it in `cache`.
#[inline(always)]
fn glyph_class_cached(class_def: &ClassDef, gid: GlyphId, cache: &MappingCache) -> u32 {
    match cache.get(gid.into()) {
        Some(class) => class,
        None => glyph_class_uncached(class_def, gid, cache),
    }
}

/// Looks up a class the cache doesn't hold, and remembers it. Kept out of
/// line so callers inline only the cache hit.
#[inline(never)]
fn glyph_class_uncached(class_def: &ClassDef, gid: GlyphId, cache: &MappingCache) -> u32 {
    let class = class_def.get(gid);
    cache.set(gid.into(), class);
    class
}
