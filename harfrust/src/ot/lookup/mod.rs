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
use crate::budget::{Budget, Exhausted};
use crate::once::Once;
use crate::set_digest::SetDigest;
use crate::GlyphInfo;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
pub(crate) use coverage::{ClassDef, Coverage};
use gpos::SecondGlyph;
use read_fonts::model::Tables;
use read_fonts::types::{BigEndian, GlyphId, Nullable, Offset16, Scalar};
use read_fonts::{FontData, FontRead};
use yoke::erased::ErasedArcCart;
use yoke::{Yoke, Yokeable};

/// The lookups of GSUB or GPOS, each parsed on first use.
pub(crate) struct LookupCache {
    lookups: Vec<Once<Option<Box<CachedLookupRef>>>>,
    /// The font's shared tables, which lookups are cached against.
    tables: Option<Arc<Tables>>,
    is_subst: bool,
    budget: Option<Arc<Budget>>,
}

impl LookupCache {
    pub const EMPTY: Self = Self {
        lookups: Vec::new(),
        tables: None,
        is_subst: false,
        budget: None,
    };

    pub fn new(tables: Option<Arc<Tables>>, is_subst: bool, budget: Arc<Budget>) -> Self {
        let count = tables
            .as_deref()
            .map_or(0, |tables| build::lookup_count(tables, is_subst));
        let count = usize::from(count);
        if budget.work(count).is_err()
            || budget
                .alloc::<Once<Option<Box<CachedLookupRef>>>>(count)
                .is_err()
        {
            return Self::EMPTY;
        }
        let mut lookups = Vec::with_capacity(count);
        lookups.resize_with(count, Once::new);
        Self {
            lookups,
            tables,
            is_subst,
            budget: Some(budget),
        }
    }

    pub fn get(&self, index: u16) -> Option<&CachedLookup<'_>> {
        let cell = self.lookups.get(usize::from(index))?;
        // Checked before `get_or_init`, whose initializer would otherwise
        // weigh down the path taken every time.
        let lookup = match cell.get() {
            Some(lookup) => lookup,
            None => cell.get_or_init(|| {
                let budget = self.budget.as_deref()?;
                budget.alloc::<CachedLookupRef>(1).ok()?;
                cache_lookup(self.tables.clone()?, self.is_subst, index, budget).map(Box::new)
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
    budget: &Budget,
) -> Option<CachedLookupRef> {
    Yoke::try_attach_to_cart(tables, |tables| {
        build::build_lookup(tables, is_subst, index, budget)
    })
    .ok()
    .map(Yoke::erase_arc_cart)
}

/// Parses a substitution lookup serialized on its own, as the Arabic
/// fallback makes them.
pub(crate) fn cache_synthesized_subst(data: Vec<u8>) -> Option<CachedLookupRef> {
    Yoke::try_attach_to_cart(Arc::new(data), |data: &Vec<u8>| {
        build::build_lookup_at(
            FontData::new(data),
            0,
            true,
            &Budget::for_table_bytes(data.len()),
        )
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
    fn coverage_index(&self, glyph: u32) -> Option<u16> {
        match &self.coverage_cache {
            Some(cache) => coverage_index_cached(|gid| self.coverage.get(gid), glyph.into(), cache),
            None => self.coverage.get(glyph),
        }
    }
}

/// Applies a subtable of one kind at the given coverage index, returning
/// `None` for any other kind.
type ApplyFn = fn(&CachedSubtable, &mut ApplyContext, u16) -> Option<()>;

/// Applies a subtable of one kind, sharing the search for a pair's second
/// glyph with the other subtables of its lookup.
type ApplySharingFn = fn(&CachedSubtable, &mut ApplyContext, u16, &mut SecondGlyph) -> Option<()>;

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
    _: u16,
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
    fn digest_second(&self, _budget: &Budget) -> Result<SetDigest, Exhausted> {
        Ok(SetDigest::full())
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
#[inline(always)]
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

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::ot::OtCache;

    #[test]
    fn exhausted_lookups_are_memoized() {
        let tables = Arc::new(
            Tables::new(
                include_bytes!("../../../tests/fonts/rb_custom/PT_Sans-Caption-Web-Regular.ttf")
                    .to_vec(),
                0,
            )
            .unwrap(),
        );
        let budget = Arc::new(Budget::new(1 << 20, 1 << 20));
        let cache = LookupCache::new(Some(tables), true, budget.clone());
        assert!(!cache.lookups.is_empty());
        budget.work(budget.remaining().0).unwrap();
        assert!(cache.get(0).is_none());
        assert!(matches!(cache.lookups[0].get(), Some(None)));
        let remaining = budget.remaining();
        assert!(cache.get(0).is_none());
        assert_eq!(budget.remaining(), remaining);
    }

    #[test]
    fn overlapping_pair_sets_exhaust_the_shared_budget() {
        let data = include_bytes!("../../../../fuzz/corpus/fuzz_shape/clusterfuzz-5665051575648256-overlapping-pair-sets.otf");
        let tables = Arc::new(Tables::new(data.to_vec(), 0).unwrap());
        let cache = OtCache::new(Some(tables));
        // Many lookup indices point at the same expensive lookup.
        for index in [53, 87, 118] {
            assert!(cache.gpos.get(index).is_none());
            assert!(matches!(
                cache.gpos.lookups[index as usize].get(),
                Some(None)
            ));
        }
        assert!(Arc::ptr_eq(
            cache.gsub.budget.as_ref().unwrap(),
            cache.gpos.budget.as_ref().unwrap()
        ));
    }

    /// Allows checking every lookup, including ones sample text won't select.
    #[test]
    #[ignore = "set HARFRUST_BUDGET_FONTS to font files or directories for calibration"]
    fn real_font_budget_headroom() {
        fn collect(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            if path.is_dir() {
                for entry in std::fs::read_dir(path).unwrap() {
                    collect(&entry.unwrap().path(), files);
                }
            } else if path
                .extension()
                .is_some_and(|ext| ext == "ttf" || ext == "otf")
            {
                files.push(path.to_owned());
            }
        }
        let paths = std::env::var_os("HARFRUST_BUDGET_FONTS").expect("set HARFRUST_BUDGET_FONTS");
        let mut files = Vec::new();
        for path in std::env::split_paths(&paths) {
            collect(&path, &mut files);
        }
        files.sort();
        assert!(
            !files.is_empty(),
            "no font files found in HARFRUST_BUDGET_FONTS"
        );
        for path in files {
            let tables = Arc::new(Tables::new(std::fs::read(&path).unwrap(), 0).unwrap());
            let bytes = [tables.gsub_data(), tables.gpos_data(), tables.gdef_data()]
                .into_iter()
                .flatten()
                .map(<[u8]>::len)
                .sum::<usize>();
            let cache = OtCache::new(Some(tables));
            for lookups in [&cache.gsub, &cache.gpos] {
                for index in 0..lookups.lookups.len() {
                    assert!(
                        lookups.get(index as u16).is_some(),
                        "lookup {index}: {}",
                        path.display()
                    );
                }
            }
            let budget = cache.gsub.budget.as_ref().unwrap();
            let (work, memory) = budget.remaining();
            let (work_limit, memory_limit) = Budget::for_table_bytes(bytes).remaining();
            std::println!(
                "{}: work {}/{work_limit}, bytes {}/{memory_limit}",
                path.display(),
                work_limit - work,
                memory_limit - memory
            );
        }
    }
}
