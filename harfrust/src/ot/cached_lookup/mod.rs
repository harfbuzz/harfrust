//! Lookups whose subtables are parsed once and kept with the font tables.
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

use super::apply::{ApplyContext, BinaryCache, MappingCache, WouldApplyContext};
use crate::set_digest::SetDigest;
use crate::GlyphInfo;
use alloc::{sync::Arc, vec::Vec};
use coverage::Coverage;
use gpos::{Search, SecondGlyph};
use read_fonts::model::Tables;
use read_fonts::types::{BigEndian, GlyphId, Nullable, Offset16, Scalar};
use read_fonts::{FontData, FontRead};
use yoke::{Yoke, Yokeable};

/// A cached lookup kept with the tables it borrows.
pub(crate) type CachedLookupRef = Yoke<CachedLookup<'static>, Arc<Tables>>;

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
}

/// A lookup with its subtables parsed.
#[derive(Yokeable)]
pub(crate) struct CachedLookup<'a> {
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
            if subtable.digest.may_have(glyph) && subtable.kind.would_apply(ctx) == Some(true) {
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
    let subtable = lookup.subtables.first()?;
    let is_cached = use_hot_subtable_cache && lookup.hot_subtable == Some(0);
    subtable.apply(ctx, is_cached)
}

/// Applies subtables in turn.
fn walk_each(
    lookup: &CachedLookup,
    ctx: &mut ApplyContext,
    use_hot_subtable_cache: bool,
) -> Option<()> {
    let glyph = ctx.buffer.cur(0)?.glyph_id;
    for (index, subtable) in lookup.subtables.iter().enumerate() {
        if !subtable.digest.may_have(glyph) {
            continue;
        }
        let is_cached = use_hot_subtable_cache && lookup.hot_subtable == Some(index);
        if subtable.apply(ctx, is_cached).is_some() {
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
        if subtable.digest.may_have(glyph)
            && (subtable.apply_sharing)(&subtable.kind, ctx, &mut second).is_some()
        {
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
    /// Applies `kind` without and with the glyph-info class cache.
    apply: [ApplyFn; 2],
    apply_sharing: ApplySharingFn,
}

impl CachedSubtable<'_> {
    #[inline]
    pub(crate) fn apply(&self, ctx: &mut ApplyContext, is_cached: bool) -> Option<()> {
        self.apply[is_cached as usize](&self.kind, ctx)
    }
}

/// Applies a subtable of one kind, returning `None` for any other kind.
type ApplyFn = fn(&CachedSubtableKind, &mut ApplyContext) -> Option<()>;

/// Applies a subtable of one kind, sharing the search for a pair's second
/// glyph with the other subtables of its lookup.
type ApplySharingFn = fn(&CachedSubtableKind, &mut ApplyContext, &mut SecondGlyph) -> Option<()>;

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
    ($kind:ident) => {
        &KindFns {
            apply: [
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply(ctx),
                    _ => None,
                },
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply(ctx),
                    _ => None,
                },
            ],
            apply_sharing: |kind, ctx, _| match kind {
                CachedSubtableKind::$kind(subtable) => subtable.apply(ctx),
                _ => None,
            },
            shares_second_glyph: false,
        }
    };
    ($kind:ident adjusts_pairs) => {
        &KindFns {
            apply: [
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply(ctx, &mut Search),
                    _ => None,
                },
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply(ctx, &mut Search),
                    _ => None,
                },
            ],
            apply_sharing: |kind, ctx, second| match kind {
                CachedSubtableKind::$kind(subtable) => subtable.apply(ctx, second),
                _ => None,
            },
            shares_second_glyph: true,
        }
    };
    ($kind:ident uses_class_cache) => {
        &KindFns {
            apply: [
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply(ctx),
                    _ => None,
                },
                |kind, ctx| match kind {
                    CachedSubtableKind::$kind(subtable) => subtable.apply_with_class_cache(ctx),
                    _ => None,
                },
            ],
            apply_sharing: |kind, ctx, _| match kind {
                CachedSubtableKind::$kind(subtable) => subtable.apply(ctx),
                _ => None,
            },
            shares_second_glyph: false,
        }
    };
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
    SingleSubst1(gsub::SingleSubst1<'a>);
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
    Context1(context::Context1<'a>);
    Context2(context::Context2<'a>) uses_class_cache;
    Context3(context::Context3<'a>);
    ChainContext1(context::ChainContext1<'a>);
    ChainContext2(context::ChainContext2<'a>) uses_class_cache;
    ChainContext3(context::ChainContext3<'a>);
}

impl CachedSubtableKind<'_> {
    /// Returns whether a substitution subtable would apply, or `None` for a
    /// positioning subtable.
    fn would_apply(&self, ctx: &WouldApplyContext) -> Option<bool> {
        Some(match self {
            Self::SingleSubst1(t) => t.would_apply(ctx),
            Self::SingleSubst2(t) => t.would_apply(ctx),
            Self::MultipleSubst1(t) => t.would_apply(ctx),
            Self::AlternateSubst1(t) => t.would_apply(ctx),
            Self::LigatureSubst1(t) => t.would_apply(ctx),
            Self::ReverseChainSubst1(t) => t.would_apply(ctx),
            Self::Context1(t) => t.would_apply(ctx),
            Self::Context2(t) => t.would_apply(ctx),
            Self::Context3(t) => t.would_apply(ctx),
            Self::ChainContext1(t) => t.would_apply(ctx),
            Self::ChainContext2(t) => t.would_apply(ctx),
            Self::ChainContext3(t) => t.would_apply(ctx),
            _ => return None,
        })
    }
}

/// What a lookup needs to know about one of its subtables.
trait SubtableDigests<'a> {
    /// The coverage of the first glyph of a match.
    fn coverage(&self) -> &Coverage<'a>;

    /// The glyphs that may follow the first one in a match.
    fn digest_second(&self) -> SetDigest {
        SetDigest::full()
    }

    /// How much the subtable gains from the glyph-info class cache.
    fn cache_cost(&self) -> u32 {
        0
    }
}

/// A 16-bit offset to a child table, which may be nullable.
trait ChildOffset: Scalar + Copy {
    fn to_usize(self) -> usize;
}

impl ChildOffset for Offset16 {
    fn to_usize(self) -> usize {
        self.to_u32() as usize
    }
}

impl ChildOffset for Nullable<Offset16> {
    fn to_usize(self) -> usize {
        self.offset().to_u32() as usize
    }
}

/// An array of offsets to child tables, with the data they are relative to.
#[derive(Clone, Copy)]
struct Offsets<'a, O: ChildOffset = Offset16> {
    data: FontData<'a>,
    offsets: &'a [BigEndian<O>],
}

impl<'a, O: ChildOffset> Offsets<'a, O> {
    fn new(data: FontData<'a>, offsets: &'a [BigEndian<O>]) -> Self {
        Self { data, offsets }
    }

    fn is_null(&self, index: usize) -> bool {
        self.offsets
            .get(index)
            .is_some_and(|offset| offset.get().to_usize() == 0)
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    /// The data of the child table at `index`, or `None` if its offset is
    /// null or out of bounds.
    fn data(&self, index: usize) -> Option<FontData<'a>> {
        let offset = self.offsets.get(index)?.get().to_usize();
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
fn coverage_index_cached(
    coverage: impl Fn(GlyphId) -> Option<u16>,
    gid: GlyphId,
    cache: &MappingCache,
) -> Option<u16> {
    if let Some(index) = cache.get(gid.into()) {
        if index == MappingCache::MAX_VALUE {
            None
        } else {
            Some(index as u16)
        }
    } else {
        let index = coverage(gid);
        if let Some(index) = index {
            if (index as u32) < MappingCache::MAX_VALUE {
                cache.set(gid.into(), index as u32);
            }
            Some(index)
        } else {
            cache.set(gid.into(), MappingCache::MAX_VALUE);
            None
        }
    }
}

/// Returns `Some` if `gid` is covered, remembering the answer in `cache`.
fn covered_cached(
    coverage: impl Fn(GlyphId) -> Option<u16>,
    gid: GlyphId,
    cache: &BinaryCache,
) -> Option<()> {
    if let Some(value) = cache.get(gid.into()) {
        (value != BinaryCache::MAX_VALUE).then_some(())
    } else if coverage(gid).is_some() {
        cache.set(gid.into(), 0);
        Some(())
    } else {
        cache.set(gid.into(), BinaryCache::MAX_VALUE);
        None
    }
}

/// Returns the class of `gid`, remembering it in `cache`.
fn glyph_class_cached(
    class_def: impl Fn(GlyphId) -> u16,
    gid: GlyphId,
    cache: &MappingCache,
) -> u16 {
    if let Some(class) = cache.get(gid.into()) {
        class as u16
    } else {
        let class = class_def(gid);
        cache.set(gid.into(), class as u32);
        class
    }
}
