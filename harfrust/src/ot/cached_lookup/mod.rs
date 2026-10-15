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
    subtables: Vec<CachedSubtable<'a>>,
}

impl CachedLookup<'_> {
    #[inline]
    pub(crate) fn apply(&self, ctx: &mut ApplyContext, use_hot_subtable_cache: bool) -> Option<()> {
        let glyph = ctx.buffer.cur(0)?.glyph_id;
        if let [subtable] = self.subtables.as_slice() {
            // The lookup digest equals this subtable's digest, which the
            // caller has already tested.
            let is_cached = use_hot_subtable_cache && self.hot_subtable == Some(0);
            return subtable.apply(ctx, is_cached);
        }
        for (index, subtable) in self.subtables.iter().enumerate() {
            if !subtable.digest.may_have(glyph) {
                continue;
            }
            let is_cached = use_hot_subtable_cache && self.hot_subtable == Some(index);
            if subtable.apply(ctx, is_cached).is_some() {
                return Some(());
            }
        }
        None
    }

    /// Prepares the glyph-info class cache for the hot subtable, returning
    /// true if it is in use.
    pub(crate) fn cache_enter(&self, ctx: &mut ApplyContext) -> bool {
        if !self.uses_class_cache() || !ctx.buffer.try_allocate_var(GlyphInfo::SYLLABLE_VAR) {
            return false;
        }
        for info in &mut ctx.buffer.info {
            info.set_syllable(255);
        }
        ctx.new_syllables = Some(255);
        true
    }

    pub(crate) fn cache_leave(&self, ctx: &mut ApplyContext) {
        if self.uses_class_cache() {
            ctx.new_syllables = None;
            ctx.buffer.deallocate_var(GlyphInfo::SYLLABLE_VAR);
        }
    }

    fn uses_class_cache(&self) -> bool {
        self.hot_subtable
            .and_then(|index| self.subtables.get(index))
            .is_some_and(|subtable| {
                matches!(
                    subtable.kind,
                    CachedSubtableKind::Context2(_) | CachedSubtableKind::ChainContext2(_)
                )
            })
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

/// A parsed subtable and the functions that apply it.
pub(crate) struct CachedSubtable<'a> {
    kind: CachedSubtableKind<'a>,
    /// The glyphs the subtable may apply to.
    digest: SetDigest,
    /// Applies `kind` without and with the glyph-info class cache.
    apply_fns: [ApplyFn; 2],
}

/// Applies a subtable of one kind, returning `None` for any other kind.
type ApplyFn = fn(&CachedSubtableKind, &mut ApplyContext) -> Option<()>;

impl CachedSubtable<'_> {
    #[inline]
    pub(crate) fn apply(&self, ctx: &mut ApplyContext, is_cached: bool) -> Option<()> {
        self.apply_fns[is_cached as usize](&self.kind, ctx)
    }
}

/// The derived data of one kind of subtable.
trait IntoKind<'a>: Sized {
    /// Applies the kind, without and with the glyph-info class cache.
    const APPLY_FNS: [ApplyFn; 2];

    fn into_kind(self) -> CachedSubtableKind<'a>;
}

/// Declares the subtable kinds: the data each holds and the methods that
/// apply it without and with the glyph-info class cache.
macro_rules! subtable_kinds {
    ($($kind:ident($data:ty) => $apply:ident, $apply_cached:ident;)*) => {
        /// A parsed subtable with the data derived from it.
        pub(crate) enum CachedSubtableKind<'a> {
            $($kind($data),)*
        }

        $(
            impl<'a> IntoKind<'a> for $data {
                const APPLY_FNS: [ApplyFn; 2] = [
                    |kind, ctx| match kind {
                        CachedSubtableKind::$kind(subtable) => subtable.$apply(ctx),
                        _ => None,
                    },
                    |kind, ctx| match kind {
                        CachedSubtableKind::$kind(subtable) => subtable.$apply_cached(ctx),
                        _ => None,
                    },
                ];

                fn into_kind(self) -> CachedSubtableKind<'a> {
                    CachedSubtableKind::$kind(self)
                }
            }
        )*
    };
}

subtable_kinds! {
    SingleSubst1(gsub::SingleSubst1<'a>) => apply, apply;
    SingleSubst2(gsub::SingleSubst2<'a>) => apply, apply;
    MultipleSubst1(gsub::MultipleSubst1<'a>) => apply, apply;
    AlternateSubst1(gsub::AlternateSubst1<'a>) => apply, apply;
    LigatureSubst1(gsub::LigatureSubst1<'a>) => apply, apply;
    ReverseChainSubst1(gsub::ReverseChainSubst1<'a>) => apply, apply;
    SinglePos1(gpos::SinglePos1<'a>) => apply, apply;
    SinglePos2(gpos::SinglePos2<'a>) => apply, apply;
    PairPos1(gpos::PairPos1<'a>) => apply, apply;
    PairPos2(gpos::PairPos2<'a>) => apply, apply;
    CursivePos1(gpos::CursivePos1<'a>) => apply, apply;
    MarkBasePos1(gpos::MarkBasePos1<'a>) => apply, apply;
    MarkLigPos1(gpos::MarkLigPos1<'a>) => apply, apply;
    MarkMarkPos1(gpos::MarkMarkPos1<'a>) => apply, apply;
    Context1(context::Context1<'a>) => apply, apply;
    Context2(context::Context2<'a>) => apply, apply_with_class_cache;
    Context3(context::Context3<'a>) => apply, apply;
    ChainContext1(context::ChainContext1<'a>) => apply, apply;
    ChainContext2(context::ChainContext2<'a>) => apply, apply_with_class_cache;
    ChainContext3(context::ChainContext3<'a>) => apply, apply;
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
