//! Turning font tables into something worth running twice.
//!
//! This module is the font side of the shaper, and it is deliberately the
//! *whole* of the font side. [`set`] holds the compiled representations --
//! coverages, glyph sets, class maps, digests, the interner that shares them.
//! [`lookup`] holds what a compiled lookup is and how a program of them is
//! stored and lazily filled. This file builds them from `read-fonts` tables.
//!
//! # Three files, then three
//!
//! This module was written in another shaper and lifted here, and it splits
//! along the line that made that possible.
//!
//! [`set`], [`lookup`] and this file are the compiled form. They know nothing
//! about a buffer, a matcher, or this crate at all: they describe a font. That
//! is enforced rather than asserted -- a test below reads their source and
//! checks that no path leaves the module except through `lookup::host`, which
//! is the one type the function pointer in a [`lookup::Subtable`] has to name.
//! Self-containment erodes one convenient import at a time and the compiler
//! never complains about it.
//!
//! [`gsub`], [`gpos`] and [`contextual`] are the applying side: one function
//! per format, reached through that pointer. These name this crate's types
//! freely, because that is their whole job. They are what a port rewrites.
//!
//! So the split is three files that came over as they were, and three written
//! against this crate's buffer.
//!
//! # What it costs to build
//!
//! This runs once per font, but it runs *before the first shape*, so its cost is
//! latency a caller feels directly rather than throughput amortised over a
//! corpus. Two things follow.
//!
//! First, compilation goes through a [`Compiler`] that owns its scratch buffers
//! and clears rather than drops them. The dominant cost is not parsing, it is
//! thousands of short-lived `Vec`s — one per coverage, per class table, per
//! ligature set. Reusing them turns that into a handful of allocations for a
//! whole font.
//!
//! Second, we compile only what gets probed. Substitute arrays and ligature sets
//! stay in the font and are addressed by offset, so compiling a lookup does not
//! walk-and-copy its payload. Offsets are relative to the start of the layout
//! table, and the caller passes that table back at apply time — which is also
//! what keeps the compiled form free of lifetimes, so it can live in a cache.
//!
//! # Extension lookups
//!
//! Lookup type 7 is pure indirection: it exists so a large font can reach past
//! the 16-bit offsets in a lookup table. It carries no semantics of its own, so
//! the only thing it changes here is arithmetic — the real subtable sits at the
//! extension's own offset plus its 32-bit `extensionOffset`. Every per-format
//! routine below therefore takes an explicit absolute offset, and the direct and
//! extension paths share them.

// Much of this module is reached only from tests and from the heap accounting
// the reports use -- `heap_bytes` on every compiled form, the constructors that
// take an explicit budget, the whole-font entry points. Those are not dead in
// the sense the lint means, and the alternative to this is a `cfg(test)` on
// each of them, which would make the measurement code harder to read than the
// thing it measures.
#![allow(dead_code)]

pub mod apply;
pub mod contextual;
pub mod gpos;
pub mod gsub;
pub mod lookup;
pub mod set;
pub mod sync;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use read_fonts::{
    tables::{
        gpos::{
            CursivePosFormat1, ExtensionSubtable as PosExtension, Gpos, MarkBasePosFormat1,
            MarkLigPosFormat1, MarkMarkPosFormat1, PairPos, PositionLookup, SinglePos,
        },
        gsub::{
            ExtensionSubtable, Gsub, LigatureSubstFormat1, MultipleSubstFormat1,
            ReverseChainSingleSubstFormat1, SingleSubst, SubstitutionChainContext,
            SubstitutionLookup,
        },
        layout::{ChainedSequenceContext, ClassDef, CoverageTable, LookupFlag, SequenceContext},
    },
    ArrayOfOffsets, Offset, ReadError,
};

use self::lookup::{
    AttachTo, CompiledLookup, LengthEffect, Program, RuleIndex, SeqRecord, SetDigests, Subtable,
    SubtableKind, U16Array,
};
use self::set::{ClassMap, Coverage, GlyphSet, Interner};

/// How much precomputation to keep alongside the compiled lookups.
///
/// Everything here is optional in the strict sense: dropping any of it changes
/// how long shaping takes and nothing else. What is *not* optional is the
/// compiled coverages and class definitions, which are what a lookup is
/// probed through -- so this dial cannot take the memory to zero, and the
/// levels below are the whole of what it can give back.
///
/// See the `detail_cost` report for what each level costs in time and saves in
/// space; the short version is that the two accelerators are cheap to keep and
/// the rule summaries are not optional in practice.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Detail {
    /// Everything. The default, and what the benchmarks measure.
    #[default]
    Full,
    /// Without the two per-lookup accelerators: the glyph-to-subtable dispatch
    /// index, and the pair filter that rejects a kerning candidate on the two
    /// glyphs together. Both are pure speed -- a lookup without them tries its
    /// subtables in order and asks the font.
    Lean,
    /// Also without the rule-set summaries, so a context enters every rule set
    /// its coverage admits and probes the rules one at a time.
    Minimal,
}

impl Detail {
    /// Whether to build the per-lookup accelerators.
    fn accelerators(self) -> bool {
        matches!(self, Detail::Full)
    }

    /// Whether to summarise rule sets.
    fn rule_summaries(self) -> bool {
        !matches!(self, Detail::Minimal)
    }
}

/// The level to compile at, from the environment.
///
/// A measurement affordance, not an interface: the level belongs on whatever
/// builds a shaper, and putting it there means threading an option through
/// `ShaperData`. Read once, so a process cannot change level halfway.
///
/// `HARFRUST_COMPILE_DETAIL=lean` or `=minimal`; anything else is `full`.
#[cfg(feature = "std")]
pub fn detail_from_env() -> Detail {
    use std::sync::OnceLock;
    static LEVEL: OnceLock<Detail> = OnceLock::new();
    *LEVEL.get_or_init(
        || match std::env::var("HARFRUST_COMPILE_DETAIL").as_deref() {
            Ok("lean") => Detail::Lean,
            Ok("minimal") => Detail::Minimal,
            _ => Detail::Full,
        },
    )
}

#[cfg(not(feature = "std"))]
pub fn detail_from_env() -> Detail {
    Detail::Full
}

/// Which fields a GPOS value record carries.
///
/// Only the *sizes* live here, not the reading: a record's size is what decides
/// where the next thing after it sits, so it is offset arithmetic, and offset
/// arithmetic is this module's whole job. Reading the values needs a buffer
/// position to write into, so it stays outside, in `value`.
pub mod value_format {
    pub const X_PLACEMENT: u16 = 0x0001;
    pub const Y_PLACEMENT: u16 = 0x0002;
    pub const X_ADVANCE: u16 = 0x0004;
    pub const Y_ADVANCE: u16 = 0x0008;
    /// Device and variation-index offsets. Their values are not applied here,
    /// but they occupy space in the record and so affect its size.
    pub const DEVICES: u16 = 0x00F0;
    /// Every bit the format may legitimately set.
    pub const ALL: u16 = 0x00FF;
}

/// Bytes one record of this format occupies: two per field present.
#[inline]
pub fn record_size(format: u16) -> usize {
    (format & value_format::ALL).count_ones() as usize * 2
}

/// Which layout table a program was compiled from.
///
/// The compiler needs it to know which lookup list to read; the matcher needs it
/// because GPOS may step over things GSUB may not.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Table {
    Gsub,
    Gpos,
}

/// Byte offset of the substitute array within a `SingleSubstFormat2` subtable:
/// past `substFormat`, `coverageOffset` and `glyphCount`.
const SINGLE_SUBST_F2_ARRAY: usize = 6;

/// `ReverseChainSingleSubstFormat1` up to and including `glyphCount`, counting
/// both coverage-offset arrays as empty: format, coverage offset, backtrack
/// count, lookahead count, glyph count.
const REVERSE_CHAIN_HEADER: usize = 10;

/// Why a lookup could not be compiled.
///
/// Every lookup type of both tables is now handled, so the only way to fail is
/// a table that does not parse. A malformed subtable still costs only itself:
/// see [`compile_gsub_program`].
#[derive(Debug)]
pub enum CompileError {
    Read(ReadError),
}

impl From<ReadError> for CompileError {
    fn from(e: ReadError) -> Self {
        CompileError::Read(e)
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self {
            glyphs: Vec::new(),
            table_span: 0,
            table_mark: 0,
            seconds: Vec::new(),
            union: Vec::new(),
            pool: Arc::default(),
            budget: set::DEFAULT_BUDGET,
            detail: Detail::Full,
        }
    }
}

/// Compiles lookups, reusing its buffers between them.
///
/// Hold one across a whole font. The buffers grow to the largest subtable seen
/// and are reused from then on.
#[derive(Debug)]
pub struct Compiler {
    /// Glyph ids of the coverage being compiled.
    glyphs: Vec<u32>,
    /// How long the layout table being compiled is, and which one it is.
    ///
    /// Both only so an interned table can be named by where it sits. A
    /// coverage's `offset_data` runs from the coverage to the end of the table
    /// it was read from, so the distance from the start is the difference of
    /// the two lengths -- and the table has to be named as well as the offset,
    /// because one interner serves GSUB and GPOS and offset 100 means a
    /// different table in each. Held as numbers rather than a borrow: the
    /// compiler outlives any one call and cannot name the font's lifetime.
    table_span: usize,
    table_mark: u64,
    /// Second components seen while walking ligature sets.
    seconds: Vec<u32>,
    /// Used by `CompiledLookup::new_in` for set unions.
    union: Vec<u32>,
    /// Interning index for chain-context sets, shared so a font cache can keep
    /// compiling into it as new plans reach new lookups.
    pool: Arc<Interner>,
    /// How much a compiled set may spend before a cheaper representation is
    /// preferred. See [`set::pick`] and [`Compiler::with_budget`].
    budget: usize,
    /// How much precomputation to keep. See [`Detail`].
    detail: Detail,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    /// A compiler that interns into an existing index, so lookups compiled now
    /// share coverages with those compiled earlier.
    pub fn with_pool(pool: Arc<Interner>) -> Self {
        Self {
            pool,
            ..Self::default()
        }
    }

    /// The same, at a chosen level of precomputation. See [`Detail`].
    pub fn with_detail(pool: Arc<Interner>, detail: Detail) -> Self {
        Self {
            pool,
            detail,
            ..Self::default()
        }
    }

    /// The same, with a different representation budget.
    ///
    /// The budget is what a set may spend before the picker prefers a cheaper
    /// shape: a bitmap is a constant-time probe but costs one bit per glyph of
    /// span, a sorted list is a binary search but costs only what it holds. A
    /// smaller budget therefore trades probe time for space, and a larger one
    /// the reverse. See [`set::pick`].
    pub fn with_budget(pool: Arc<Interner>, budget: usize) -> Self {
        Self {
            pool,
            budget,
            ..Self::default()
        }
    }

    /// Total capacity currently held, so a caller can check that reuse is really
    /// happening rather than the buffers being reallocated each time.
    pub fn scratch_capacity(&self) -> usize {
        self.glyphs.capacity() + self.seconds.capacity() + self.union.capacity()
    }

    /// Give the scratch back.
    ///
    /// These buffers exist to be reused, and that reuse is why compiling a
    /// font is a handful of allocations rather than thousands -- so they must
    /// not be dropped between lookups. But they grow to the largest thing ever
    /// put in them and never shrink, and a font cache holds its compiler for
    /// as long as it holds the font, so one lookup with a four-thousand-glyph
    /// coverage would otherwise leave that much scratch behind for good.
    ///
    /// The right lifetime is the shaping call: every lookup a plan reaches is
    /// compiled within one, so the reuse is kept in full, and nothing survives
    /// it. A later call that reaches new lookups pays to grow the buffers
    /// again, which is the same cost it would have paid to compile them.
    pub(super) fn release(&mut self) {
        self.glyphs = Vec::new();
        self.seconds = Vec::new();
        self.union = Vec::new();
    }

    /// The interning index, shared with whatever program this builds.
    pub fn pool(&self) -> &Arc<Interner> {
        &self.pool
    }

    /// Compile lookup `index` of a GSUB table.
    ///
    /// Takes the table rather than a bare lookup because the compiled form
    /// records offsets relative to the table, which a lookup alone cannot know.
    pub fn gsub(&mut self, gsub: &Gsub, index: u16) -> Result<CompiledLookup, CompileError> {
        // Every offset a subtable records is relative to the layout table, so a
        // set summarised at compile time is read the same way it will be read
        // at apply time.
        let data = gsub.offset_data().as_bytes();
        self.table_span = data.len();
        self.table_mark = 0;
        let list = gsub.lookup_list()?;
        let lookup_offset = gsub.lookup_list_offset().to_usize()
            + list
                .lookup_offsets()
                .get(index as usize)
                .ok_or(ReadError::OutOfBounds)?
                .get()
                .to_usize();

        let lookup = list.lookups().get(index as usize)?;
        let (flag, filtering_set) = match &lookup {
            SubstitutionLookup::Single(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Ligature(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Multiple(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Contextual(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Alternate(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::ChainContextual(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Extension(l) => (l.lookup_flag(), l.mark_filtering_set()),
            SubstitutionLookup::Reverse(l) => (l.lookup_flag(), l.mark_filtering_set()),
        };

        let mut subtables = Vec::new();
        match &lookup {
            SubstitutionLookup::Single(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.single(&st, at));
                }
            }
            SubstitutionLookup::Ligature(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.ligature(&st, at));
                }
            }
            SubstitutionLookup::Multiple(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.multiple(&st, at));
                }
            }
            SubstitutionLookup::Alternate(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    let t = st;
                    subtables.push(Subtable::ranked(
                        self.coverage_or_empty(t.coverage()),
                        SubtableKind::Alternate { offset: at as u32 },
                    ));
                }
            }
            SubstitutionLookup::Contextual(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.context(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
            SubstitutionLookup::ChainContextual(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.chain_context(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
            SubstitutionLookup::Extension(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.extension(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
            SubstitutionLookup::Reverse(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.reverse_chain(&st, at) {
                        subtables.push(subtable);
                    }
                }
            }
        }

        Ok(CompiledLookup::new_with(
            props_of(flag, filtering_set),
            subtables,
            &mut self.union,
            self.detail.accelerators(),
            set::scan_budget(self.budget),
        ))
    }

    /// Unwrap a type 7 subtable and compile whatever it points at.
    ///
    /// `at` is the extension subtable itself; the real subtable is that plus the
    /// extension's own 32-bit offset. Getting this arithmetic wrong is silent,
    /// which is why `tests/offsets` resolves it against the font.
    fn extension(
        &mut self,
        ext: &ExtensionSubtable,
        at: usize,
        data: &[u8],
    ) -> Result<Subtable, CompileError> {
        match ext {
            ExtensionSubtable::Single(e) => {
                let inner = at + e.extension_offset().to_usize();
                Ok(self.single(&e.extension()?, inner))
            }
            ExtensionSubtable::Ligature(e) => {
                let inner = at + e.extension_offset().to_usize();
                Ok(self.ligature(&e.extension()?, inner))
            }
            ExtensionSubtable::ChainContextual(e) => {
                let inner = at + e.extension_offset().to_usize();
                self.chain_context(&e.extension()?, inner, data)
            }
            ExtensionSubtable::Contextual(e) => {
                let inner = at + e.extension_offset().to_usize();
                self.context(&e.extension()?, inner, data)
            }
            ExtensionSubtable::Multiple(e) => {
                let inner = at + e.extension_offset().to_usize();
                Ok(self.multiple(&e.extension()?, inner))
            }
            ExtensionSubtable::Alternate(e) => {
                let inner = at + e.extension_offset().to_usize();
                Ok(Subtable::ranked(
                    self.coverage_or_empty(e.extension()?.coverage()),
                    SubtableKind::Alternate {
                        offset: inner as u32,
                    },
                ))
            }
            ExtensionSubtable::Reverse(e) => {
                let inner = at + e.extension_offset().to_usize();
                self.reverse_chain(&e.extension()?, inner)
            }
        }
    }

    fn single(&mut self, t: &SingleSubst, at: usize) -> Subtable {
        match t {
            SingleSubst::Format1(t) => Subtable::member(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::SingleDelta {
                    delta: i32::from(t.delta_glyph_id()),
                },
            ),
            SingleSubst::Format2(t) => Subtable::ranked(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::SingleList {
                    subst: U16Array {
                        offset: (at + SINGLE_SUBST_F2_ARRAY) as u32,
                        len: t.glyph_count(),
                    },
                },
            ),
        }
    }

    /// Reverse chaining contextual single substitution, GSUB type 8.
    ///
    /// The gate is ranked -- the substitute array is indexed by coverage index,
    /// exactly as single substitution format 2 is -- and the substitutes stay
    /// in the font. Its offset is the one variable-length piece of arithmetic
    /// here: two counted coverage-offset arrays sit between the header and the
    /// glyph list, so the array cannot live at a constant.
    ///
    /// Deliberately no `.following(lookahead.first())`. The next-glyph filter
    /// would be wrong here for the reason the format exists: see
    /// [`CompiledLookup::reverse`].
    fn reverse_chain(
        &mut self,
        t: &ReverseChainSingleSubstFormat1,
        at: usize,
    ) -> Result<Subtable, CompileError> {
        let backtrack = self.sets(t.backtrack_coverages())?;
        let lookahead = self.sets(t.lookahead_coverages())?;
        let subst = U16Array {
            offset: (at + REVERSE_CHAIN_HEADER + 2 * (backtrack.len() + lookahead.len())) as u32,
            len: t.glyph_count(),
        };
        // The font's own view of where that array starts, which read-fonts
        // computes from the same counts. Cheap, and the arithmetic above is the
        // kind that fails silently.
        debug_assert_eq!(
            subst.offset as usize - at,
            t.substitute_glyph_ids_byte_range().start
        );
        Ok(Subtable::ranked(
            self.coverage_or_empty(t.coverage()),
            SubtableKind::ReverseChain {
                backtrack,
                lookahead,
                subst,
            },
        ))
    }

    /// Walk the ligature sets once to derive the pair key, then leave them in
    /// the font.
    fn ligature(&mut self, t: &LigatureSubstFormat1, at: usize) -> Subtable {
        let cov = t
            .coverage()
            .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c));
        self.seconds.clear();

        let sets = t.ligature_sets();
        // A ligature declaring one component substitutes the covered glyph on
        // its own, whatever follows. One of those anywhere in the subtable and
        // there is nothing to key on: the filter's promise is that a candidate
        // whose second glyph is absent cannot match, and this one can.
        let mut keyable = true;
        for i in 0..cov.len() {
            // A coverage entry with no ligature set is malformed; treat it as
            // empty so one bad entry cannot poison the rest of the font.
            let Ok(set) = sets.get(i) else { continue };
            for lig in set.ligatures().iter() {
                let Ok(lig) = lig else { continue };
                // Component 2 is what the pair key filters on.
                match lig.component_glyph_ids().first() {
                    Some(second) => self.seconds.push(second.get().to_u32()),
                    None => keyable = false,
                }
            }
        }

        self.seconds.sort_unstable();
        self.seconds.dedup();
        let next = keyable.then(|| Arc::new(GlyphSet::build(&self.seconds)));
        Subtable::ranked(cov, SubtableKind::Ligature { offset: at as u32 }).following(next)
    }

    /// Multiple substitution. The sequences stay in the font; what compilation
    /// derives is the length effect, by scanning how long they are.
    ///
    /// Only a sequence of more than one glyph actually grows the buffer. A
    /// single-glyph sequence substitutes in place and an empty one deletes, so a
    /// subtable made only of those never needs the rebuild path.
    fn multiple(&mut self, t: &MultipleSubstFormat1, at: usize) -> Subtable {
        let cov = t
            .coverage()
            .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c));
        let mut effect = LengthEffect::Preserving;
        for seq in t.sequences().iter().flatten() {
            effect = effect.join(match seq.glyph_count() {
                0 => LengthEffect::Shrinking,
                1 => LengthEffect::Preserving,
                _ => LengthEffect::Growing,
            });
        }
        Subtable::ranked(
            cov,
            SubtableKind::Multiple {
                offset: at as u32,
                effect,
            },
        )
    }

    fn chain_context(
        &mut self,
        t: &SubstitutionChainContext,
        at: usize,
        data: &[u8],
    ) -> Result<Subtable, CompileError> {
        match t {
            // Format 1 lists rules over glyph ids, format 2 over classes; both
            // keep the rules in the font.
            ChainedSequenceContext::Format1(t) => {
                let digests = rule_index(
                    data,
                    (at + 6) as u32,
                    at as u32,
                    t.chained_seq_rule_set_count(),
                    true,
                    self.detail.rule_summaries(),
                );
                Ok(Subtable::ranked(
                    self.coverage_or_empty(t.coverage()),
                    SubtableKind::Rules {
                        input_classes: None,
                        backtrack_classes: None,
                        lookahead_classes: None,
                        // Rule-set offsets follow a six-byte header.
                        rule_sets: (at + 6) as u32,
                        base: at as u32,
                        rule_set_count: t.chained_seq_rule_set_count(),
                        index: Box::new(RuleIndex { digests }),
                        chained: true,
                    },
                ))
            }
            ChainedSequenceContext::Format2(t) => {
                let digests = rule_index(
                    data,
                    (at + 12) as u32,
                    at as u32,
                    t.chained_class_seq_rule_set_count(),
                    true,
                    self.detail.rule_summaries(),
                );
                Ok(Subtable::member(
                    self.coverage_or_empty(t.coverage()),
                    SubtableKind::Rules {
                        input_classes: Some(self.class_or_empty(t.input_class_def())),
                        backtrack_classes: Some(self.class_or_empty(t.backtrack_class_def())),
                        lookahead_classes: Some(self.class_or_empty(t.lookahead_class_def())),
                        // Four offsets and two counts precede the rule sets.
                        rule_sets: (at + 12) as u32,
                        base: at as u32,
                        rule_set_count: t.chained_class_seq_rule_set_count(),
                        index: Box::new(RuleIndex { digests }),
                        chained: true,
                    },
                ))
            }
            ChainedSequenceContext::Format3(t) => self.chain_context3(t, at),
        }
    }

    /// Unchained sequence context.
    ///
    /// Format 3 compiles to the same shape as chained format 3 with no
    /// backtrack and no lookahead, which is exactly what it is.
    fn context(
        &mut self,
        t: &SequenceContext,
        at: usize,
        data: &[u8],
    ) -> Result<Subtable, CompileError> {
        match t {
            SequenceContext::Format1(t) => {
                let digests = rule_index(
                    data,
                    (at + 6) as u32,
                    at as u32,
                    t.seq_rule_set_count(),
                    false,
                    self.detail.rule_summaries(),
                );
                Ok(Subtable::ranked(
                    self.coverage_or_empty(t.coverage()),
                    SubtableKind::Rules {
                        input_classes: None,
                        backtrack_classes: None,
                        lookahead_classes: None,
                        rule_sets: (at + 6) as u32,
                        base: at as u32,
                        rule_set_count: t.seq_rule_set_count(),
                        index: Box::new(RuleIndex { digests }),
                        chained: false,
                    },
                ))
            }
            SequenceContext::Format2(t) => {
                let digests = rule_index(
                    data,
                    (at + 8) as u32,
                    at as u32,
                    t.class_seq_rule_set_count(),
                    false,
                    self.detail.rule_summaries(),
                );
                Ok(Subtable::member(
                    self.coverage_or_empty(t.coverage()),
                    SubtableKind::Rules {
                        input_classes: Some(self.class_or_empty(t.class_def())),
                        backtrack_classes: None,
                        lookahead_classes: None,
                        // One extra offset for the class definition.
                        rule_sets: (at + 8) as u32,
                        base: at as u32,
                        rule_set_count: t.class_seq_rule_set_count(),
                        index: Box::new(RuleIndex { digests }),
                        chained: false,
                    },
                ))
            }
            SequenceContext::Format3(t) => {
                let (cov, input) = self.input_sets(t.coverages())?;
                let records = t
                    .seq_lookup_records()
                    .iter()
                    .map(|r| SeqRecord {
                        seq_index: r.sequence_index(),
                        lookup_index: r.lookup_list_index(),
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                let next = input.first().cloned();
                Ok(Subtable::member(
                    cov,
                    SubtableKind::ChainCtx3 {
                        backtrack: Box::new([]),
                        input,
                        lookahead: Box::new([]),
                        records,
                        offset: at as u32,
                        chained: false,
                    },
                )
                .following(next))
            }
        }
    }

    fn coverage_or_empty(&mut self, c: Result<CoverageTable, ReadError>) -> Arc<Coverage> {
        match c {
            Ok(c) => self.coverage(&c),
            Err(_) => Arc::new(Coverage::Empty),
        }
    }

    fn class_or_empty(&mut self, c: Result<ClassDef, ReadError>) -> Arc<ClassMap> {
        match c {
            Ok(c) => self.class_map(&c),
            Err(_) => Arc::new(ClassMap::Empty),
        }
    }

    /// Chain context format 3 tests its coverages and never indexes them, so
    /// every one compiles to a membership-only set — and none of them holds a
    /// font offset, which is why this routine needs no `at`.
    fn chain_context3(
        &mut self,
        t: &read_fonts::tables::layout::ChainedSequenceContextFormat3,
        at: usize,
    ) -> Result<Subtable, CompileError> {
        let backtrack = self.sets(t.backtrack_coverages())?;
        let (cov, input) = self.input_sets(t.input_coverages())?;
        let lookahead = self.sets(t.lookahead_coverages())?;
        let records = t
            .seq_lookup_records()
            .iter()
            .map(|r| SeqRecord {
                seq_index: r.sequence_index(),
                lookup_index: r.lookup_list_index(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // What has to follow the start: the second input coverage, or the
        // first lookahead when the input is a single glyph.
        let next = input.first().or_else(|| lookahead.first()).cloned();
        Ok(Subtable::member(
            cov,
            SubtableKind::ChainCtx3 {
                backtrack,
                input,
                lookahead,
                records,
                offset: at as u32,
                chained: true,
            },
        )
        .following(next))
    }

    /// Split a run of input coverages into the gate and the rest.
    ///
    /// The first is what makes a position a candidate, so it goes in the outer
    /// subtable alongside every other format's gate. The rest are only ever
    /// tested, so they stay membership-only sets.
    ///
    /// No coverages at all means nothing can start here. That used to fail the
    /// whole lookup; an empty gate says the same thing without taking the rest
    /// of the lookup down with it, and costs one constant-false probe.
    fn input_sets<'a>(
        &mut self,
        covs: ArrayOfOffsets<'a, CoverageTable<'a>>,
    ) -> Result<(Arc<Coverage>, Box<[Arc<GlyphSet>]>), CompileError> {
        let mut iter = covs.iter();
        let Some(first) = iter.next() else {
            return Ok((Arc::new(Coverage::Empty), Box::new([])));
        };
        let gate = self.coverage(&first?);
        let mut rest = Vec::with_capacity(covs.len().saturating_sub(1));
        for c in iter {
            rest.push(self.intern(&c?));
        }
        Ok((gate, rest.into_boxed_slice()))
    }

    /// Compile a run of coverage tables to membership-only sets, reusing the
    /// glyph scratch for each.
    fn sets<'a>(
        &mut self,
        covs: ArrayOfOffsets<'a, CoverageTable<'a>>,
    ) -> Result<Box<[Arc<GlyphSet>]>, CompileError> {
        let mut out = Vec::with_capacity(covs.len());
        for c in covs.iter() {
            out.push(self.intern(&c?));
        }
        Ok(out.into_boxed_slice())
    }

    /// Intern a coverage table, keyed by its bytes.
    ///
    /// Fonts reference the same coverage over and over — Nastaliq makes 4,620
    /// chain-context references to 93 distinct sets. Two things follow from
    /// hashing the raw bytes rather than a decoded glyph list:
    ///
    /// * A hit never decodes. The 4,527 repeat references cost a hash and a
    ///   memcmp instead of walking a coverage table and building a set.
    /// * A hit never allocates. `HashTable` probes with a hash and an equality
    ///   closure, so the borrowed bytes are the key; a `HashMap` would need an
    ///   owned key constructed just to look up.
    fn intern(&mut self, c: &CoverageTable) -> Arc<GlyphSet> {
        let key = origin(self.table_span, self.table_mark, c.offset_data().as_bytes());
        let bytes = coverage_bytes(c);
        // Disjoint field borrows: the build closure fills the glyph scratch
        // while the interner is borrowed.
        let Self {
            glyphs,
            pool,
            budget,
            ..
        } = self;
        let (pool, budget): (&Interner, usize) = (pool, *budget);
        pool.set(key, bytes, || {
            fill_glyphs(glyphs, c);
            GlyphSet::build_with_budget(glyphs, budget)
        })
    }

    /// Coverages duplicate heavily across a font -- Nastaliq references 148
    /// distinct tables 16,006 times -- so these are interned exactly like the
    /// membership-only sets are.
    fn coverage(&mut self, c: &CoverageTable) -> Arc<Coverage> {
        let key = origin(self.table_span, self.table_mark, c.offset_data().as_bytes());
        let bytes = coverage_bytes(c);
        let Self {
            glyphs,
            pool,
            budget,
            ..
        } = self;
        let (pool, budget): (&Interner, usize) = (pool, *budget);
        pool.coverage(key, bytes, || {
            fill_glyphs(glyphs, c);
            Coverage::build_with_budget(glyphs, budget)
        })
    }
}

fn fill_glyphs(glyphs: &mut Vec<u32>, c: &CoverageTable) {
    glyphs.clear();
    glyphs.extend(c.iter().map(|g| g.to_u32()));
    // Coverage tables are required to be ascending, but a malformed font can
    // violate it and every representation depends on the order.
    if !glyphs.windows(2).all(|w| w[0] < w[1]) {
        glyphs.sort_unstable();
        glyphs.dedup();
    }
}

/// The exact bytes of a class definition, for interning.
///
/// Format 1 is a start glyph and a run of classes; format 2 is a run of range
/// records. Either way the extent comes from the count field, since
/// `offset_data` runs to the end of the parent table.
/// Where a subtable sits, as a key nothing else can collide with.
///
/// `None` when the position cannot be worked out, which means the table is not
/// interned and is simply built again -- always correct, and never reached by
/// a font whose offsets are what they say they are.
fn origin(span: usize, mark: u64, tail: &[u8]) -> Option<u64> {
    let at = span.checked_sub(tail.len())?;
    Some(mark | u64::try_from(at).ok()?)
}

fn class_def_bytes<'a>(c: &ClassDef<'a>) -> &'a [u8] {
    let (len, data) = match c {
        ClassDef::Format1(t) => (6 + 2 * t.glyph_count() as usize, t.offset_data()),
        ClassDef::Format2(t) => (4 + 6 * t.class_range_count() as usize, t.offset_data()),
    };
    let all = data.as_bytes();
    all.get(..len).unwrap_or(all)
}

/// The exact bytes of a coverage table.
///
/// `offset_data` runs from the table to the end of its parent, so the extent has
/// to come from the format's own count field. A truncated table falls back to
/// whatever is there, which then simply fails to match anything else.
fn coverage_bytes<'a>(c: &CoverageTable<'a>) -> &'a [u8] {
    let (len, data) = match c {
        CoverageTable::Format1(t) => (4 + 2 * t.glyph_count() as usize, t.offset_data()),
        CoverageTable::Format2(t) => (4 + 6 * t.range_count() as usize, t.offset_data()),
    };
    let all = data.as_bytes();
    all.get(..len).unwrap_or(all)
}

impl Compiler {
    /// Compile lookup `index` of a GPOS table.
    pub fn gpos(&mut self, gpos: &Gpos, index: u16) -> Result<CompiledLookup, CompileError> {
        let data = gpos.offset_data().as_bytes();
        self.table_span = data.len();
        self.table_mark = 1 << 32;
        let list = gpos.lookup_list()?;
        let lookup_offset = gpos.lookup_list_offset().to_usize()
            + list
                .lookup_offsets()
                .get(index as usize)
                .ok_or(ReadError::OutOfBounds)?
                .get()
                .to_usize();

        let lookup = list.lookups().get(index as usize)?;
        let (flag, filtering_set) = match &lookup {
            PositionLookup::Single(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::Pair(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::MarkToBase(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::MarkToMark(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::MarkToLig(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::Cursive(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::Extension(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::Contextual(l) => (l.lookup_flag(), l.mark_filtering_set()),
            PositionLookup::ChainContextual(l) => (l.lookup_flag(), l.mark_filtering_set()),
        };

        let mut subtables = Vec::new();
        match &lookup {
            PositionLookup::Single(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.single_pos(&st, at));
                }
            }
            PositionLookup::Pair(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.pair_pos(&st, at, data));
                }
            }
            // Context lookups are the same machinery in either table: they
            // match positions and recurse, and what the nested lookup does with
            // them is the nested lookup's business.
            PositionLookup::Contextual(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.context(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
            PositionLookup::ChainContextual(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.chain_context(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
            PositionLookup::Cursive(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.cursive(&st, at));
                }
            }
            PositionLookup::MarkToBase(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.mark_base(&st, at));
                }
            }
            PositionLookup::MarkToMark(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.mark_mark(&st, at));
                }
            }
            PositionLookup::MarkToLig(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    subtables.push(self.mark_lig(&st, at));
                }
            }
            PositionLookup::Extension(l) => {
                subtables.reserve(l.sub_table_count() as usize);
                for (i, st) in l.subtables().iter().enumerate() {
                    let (Ok(offset), Ok(st)) = (offset_at(l.subtable_offsets(), i), st) else {
                        continue;
                    };
                    let at = lookup_offset + offset;
                    if let Ok(subtable) = self.pos_extension(&st, at, data) {
                        subtables.push(subtable);
                    }
                }
            }
        }

        let lookup = CompiledLookup::new_with(
            props_of(flag, filtering_set),
            subtables,
            &mut self.union,
            self.detail.accelerators(),
            set::scan_budget(self.budget),
        );
        Ok(lookup)
    }

    /// The value record itself stays in the font; only the coverage and the
    /// format word are compiled.
    fn single_pos(&mut self, t: &SinglePos, at: usize) -> Subtable {
        match t {
            // Format 1: header is format, coverage offset, value format.
            SinglePos::Format1(t) => Subtable::member(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::SinglePos {
                    value_format: t.value_format().bits(),
                    values: (at + 6) as u32,
                    shared: true,
                },
            ),
            // Format 2 adds a count, then one record per covered glyph.
            SinglePos::Format2(t) => Subtable::ranked(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::SinglePos {
                    value_format: t.value_format().bits(),
                    values: (at + 8) as u32,
                    shared: false,
                },
            ),
        }
    }

    fn pair_pos(&mut self, t: &PairPos, at: usize, data: &[u8]) -> Subtable {
        match t {
            PairPos::Format1(t) => Subtable::ranked(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::PairPos1 {
                    seconds: pair_digests(
                        data,
                        at,
                        t.pair_set_count(),
                        t.value_format1().bits(),
                        t.value_format2().bits(),
                    ),
                    offset: at as u32,
                    first_format: t.value_format1().bits(),
                    second_format: t.value_format2().bits(),
                },
            ),
            PairPos::Format2(t) => Subtable::member(
                t.coverage()
                    .map_or_else(|_| Arc::new(Coverage::Empty), |c| self.coverage(&c)),
                SubtableKind::PairPos2 {
                    rows: pair_class_digests(
                        data,
                        at + 16,
                        t.class1_count(),
                        t.class2_count(),
                        t.value_format1().bits(),
                        t.value_format2().bits(),
                    ),
                    class1: t
                        .class_def1()
                        .map_or_else(|_| Arc::new(ClassMap::Empty), |c| self.class_map(&c)),
                    class2: t
                        .class_def2()
                        .map_or_else(|_| Arc::new(ClassMap::Empty), |c| self.class_map(&c)),
                    // Class-1 records follow a sixteen-byte header.
                    matrix: (at + 16) as u32,
                    first_format: t.value_format1().bits(),
                    second_format: t.value_format2().bits(),
                    class2_count: t.class2_count(),
                },
            ),
        }
    }

    /// Cursive attachment. Only the coverage compiles; the entry and exit
    /// anchors stay in the font.
    /// Unwrap a positioning extension: the same indirection GSUB has, and the
    /// same rule that only the arithmetic changes.
    fn pos_extension(
        &mut self,
        ext: &PosExtension,
        at: usize,
        data: &[u8],
    ) -> Result<Subtable, CompileError> {
        macro_rules! inner {
            ($e:expr) => {
                at + $e.extension_offset().to_usize()
            };
        }
        match ext {
            PosExtension::Single(e) => Ok(self.single_pos(&e.extension()?, inner!(e))),
            PosExtension::Pair(e) => Ok(self.pair_pos(&e.extension()?, inner!(e), data)),
            PosExtension::Cursive(e) => Ok(self.cursive(&e.extension()?, inner!(e))),
            PosExtension::MarkToBase(e) => Ok(self.mark_base(&e.extension()?, inner!(e))),
            PosExtension::MarkToLig(e) => Ok(self.mark_lig(&e.extension()?, inner!(e))),
            PosExtension::MarkToMark(e) => Ok(self.mark_mark(&e.extension()?, inner!(e))),
            PosExtension::Contextual(e) => self.context(&e.extension()?, inner!(e), data),
            PosExtension::ChainContextual(e) => {
                self.chain_context(&e.extension()?, inner!(e), data)
            }
        }
    }

    fn cursive(&mut self, t: &CursivePosFormat1, at: usize) -> Subtable {
        Subtable::ranked(
            self.coverage_or_empty(t.coverage()),
            SubtableKind::Cursive {
                // Records follow a six-byte header.
                records: (at + 6) as u32,
                base: at as u32,
                count: t.entry_exit_count(),
            },
        )
    }

    /// Mark-to-base. Only the two coverages compile; every anchor stays in the
    /// font, because a run reads a few of the hundreds a font carries.
    fn mark_base(&mut self, t: &MarkBasePosFormat1, at: usize) -> Subtable {
        Subtable::member(
            self.coverage_or_empty(t.mark_coverage()),
            SubtableKind::MarkTo {
                offset: at as u32,
                bases: self.coverage_or_empty(t.base_coverage()),
                mark_array: (at + t.mark_array_offset().to_usize()) as u32,
                base_array: (at + t.base_array_offset().to_usize()) as u32,
                class_count: t.mark_class_count(),
                to: AttachTo::Base,
            },
        )
    }

    fn mark_mark(&mut self, t: &MarkMarkPosFormat1, at: usize) -> Subtable {
        Subtable::member(
            self.coverage_or_empty(t.mark1_coverage()),
            SubtableKind::MarkTo {
                offset: at as u32,
                bases: self.coverage_or_empty(t.mark2_coverage()),
                mark_array: (at + t.mark1_array_offset().to_usize()) as u32,
                base_array: (at + t.mark2_array_offset().to_usize()) as u32,
                class_count: t.mark_class_count(),
                to: AttachTo::Mark,
            },
        )
    }

    /// Mark-to-ligature. The extra indirection is the point: a ligature has one
    /// set of anchors per component, so a mark lands on the component it came
    /// from rather than on the ligature as a whole.
    fn mark_lig(&mut self, t: &MarkLigPosFormat1, at: usize) -> Subtable {
        Subtable::member(
            self.coverage_or_empty(t.mark_coverage()),
            SubtableKind::MarkTo {
                offset: at as u32,
                bases: self.coverage_or_empty(t.ligature_coverage()),
                mark_array: (at + t.mark_array_offset().to_usize()) as u32,
                base_array: (at + t.ligature_array_offset().to_usize()) as u32,
                class_count: t.mark_class_count(),
                to: AttachTo::Ligature,
            },
        )
    }

    fn class_map(&mut self, c: &ClassDef) -> Arc<ClassMap> {
        let key = origin(self.table_span, self.table_mark, c.offset_data().as_bytes());
        let bytes = class_def_bytes(c);
        let budget = self.budget;
        self.pool.class_map(key, bytes, || {
            let mut entries: Vec<(u32, u16)> = c
                .iter()
                .filter(|(_, cls)| *cls != 0)
                .map(|(g, cls)| (g.to_u32(), cls))
                .collect();
            entries.sort_unstable();
            entries.dedup_by_key(|e| e.0);
            ClassMap::build_with_budget(&entries, budget)
        })
    }
}

fn offset_at(
    offsets: &[read_fonts::types::BigEndian<read_fonts::types::Offset16>],
    i: usize,
) -> Result<usize, CompileError> {
    Ok(offsets
        .get(i)
        .ok_or(ReadError::OutOfBounds)?
        .get()
        .to_usize())
}

fn props_of(flag: LookupFlag, mark_filtering_set: Option<u16>) -> u32 {
    let mut props = u32::from(flag.to_bits());
    if flag.to_bits() & LookupFlag::USE_MARK_FILTERING_SET.to_bits() != 0 {
        props |= u32::from(mark_filtering_set.unwrap_or_default()) << 16;
    }
    props
}

/// Compile every lookup of a GSUB table into a [`Program`].
///
/// Chain context recurses by lookup-list index, so the whole table compiles
/// together even when only part of it is reachable from the enabled features: a
/// nested lookup need not appear in any feature itself.
///
/// Lookups that fail to compile become `None` rather than aborting the font, so
/// one malformed subtable does not cost the rest. Callers that need to know what
/// they are missing ask [`Program::missing`].
/// A [`Program`] over a GPOS table. Nothing is compiled until a lookup is
/// asked for.
pub fn compile_gpos_program(gpos: &Gpos) -> Program {
    let count = gpos.lookup_list().map_or(0, |l| l.lookup_count());
    Program::new_gpos(count, Arc::default())
}

/// A [`Program`] over a GSUB table. Nothing is compiled until a lookup is
/// asked for.
pub fn compile_gsub_program(gsub: &Gsub) -> Program {
    let count = gsub.lookup_list().map_or(0, |l| l.lookup_count());
    Program::new(count, Arc::default())
}

/// Both programs of one font, sharing an interning index.
///
/// Worth using over the two separately: GSUB and GPOS name many of the same
/// coverages, and interning across the pair is what collapses them to one copy.
pub fn compile_font(gsub: Option<&Gsub>, gpos: Option<&Gpos>) -> (Program, Program) {
    compile_font_with_detail(gsub, gpos, Detail::Full)
}

/// The same, keeping less precomputation. See [`Detail`].
pub fn compile_font_with_detail(
    gsub: Option<&Gsub>,
    gpos: Option<&Gpos>,
    detail: Detail,
) -> (Program, Program) {
    let pool = Arc::<Interner>::default();
    let gsub_count = gsub
        .and_then(|t| t.lookup_list().ok())
        .map_or(0, |l| l.lookup_count());
    let gpos_count = gpos
        .and_then(|t| t.lookup_list().ok())
        .map_or(0, |l| l.lookup_count());
    (
        Program::new_with_detail(gsub_count, Arc::clone(&pool), detail),
        Program::new_gpos_with_detail(gpos_count, pool, detail),
    )
}

/// Summarise a context subtable's rule sets, at two granularities.
///
/// Each rule begins by testing the position after the one the lookup is
/// anchored on. Per set, the values its rules accept there fold into a word,
/// which throws away whole sets. Per rule, the same value folds into a byte,
/// which throws away individual rules inside a set that survived -- and that is
/// where most of the remaining header parsing goes.
///
/// One walk builds both, since reaching either value costs the same parse. A
/// rule constraining nothing at that step forces its set open and is marked
/// always-try; so does anything malformed, because a filter may only ever
/// reject what truly cannot match.
fn rule_index(
    data: &[u8],
    rule_sets: u32,
    base: u32,
    count: u16,
    chained: bool,
    summarise: bool,
) -> SetDigests {
    if !summarise {
        // An empty summary admits everything, which is what a filter must do
        // when it knows nothing.
        return SetDigests::default();
    }
    SetDigests::build(count, |set| {
        let set_off = be16(data, rule_sets as usize + set as usize * 2)?;
        if set_off == 0 {
            // A null offset means no rules for this class, so nothing matches.
            return Some(1);
        }
        let set_at = base as usize + set_off as usize;
        let rule_count = be16(data, set_at)?;
        let mut summary = 0u64;
        for i in 0..rule_count as usize {
            let rule_off = be16(data, set_at + 2 + i * 2)?;
            let at = set_at + rule_off as usize;
            // A rule that constrains nothing at its first input step forces
            // the whole set open, and so does anything malformed: a filter may
            // only ever reject what truly cannot match.
            let v = first_input_value(data, at, chained)?;
            summary |= SetDigests::bit(v);
        }
        (summary != 0).then_some(summary)
    })
}

/// The value a rule requires at the position after the one it is anchored on.
///
/// Just enough of the header to reach it: a chained rule puts its backtrack
/// sequence first, and an unchained one puts its lookup count between the input
/// count and the input sequence.
fn first_input_value(data: &[u8], rule_at: usize, chained: bool) -> Option<u32> {
    let mut at = rule_at;
    if chained {
        at += 2 + be16(data, at)? as usize * 2;
    }
    let input_count = be16(data, at)?;
    at += 2;
    if input_count < 2 {
        return None;
    }
    if !chained {
        at += 2;
    }
    Some(u32::from(be16(data, at)?))
}

/// Summarise every pair set of a `PairPosFormat1`: which glyphs each first
/// glyph actually pairs with.
fn pair_digests(
    data: &[u8],
    at: usize,
    count: u16,
    first_format: u16,
    second_format: u16,
) -> SetDigests {
    let stride = 2 + record_size(first_format) + record_size(second_format);
    SetDigests::build(count, |set| {
        let set_off = be16(data, at + PAIR_SET_OFFSETS + set as usize * 2)?;
        let set_at = at + set_off as usize;
        let pair_count = be16(data, set_at)?;
        let mut summary = 0u64;
        for i in 0..pair_count as usize {
            summary |= SetDigests::bit(u32::from(be16(data, set_at + 2 + i * stride)?));
        }
        (summary != 0).then_some(summary)
    })
}

/// Pair set offsets follow a ten-byte header.
const PAIR_SET_OFFSETS: usize = 10;

/// A big-endian `u16` at a byte offset, bounds checked.
pub(super) fn be16(data: &[u8], at: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

/// Summarise which class-2 values each class-1 row has a non-zero record for.
///
/// A class matrix holds one record per pair of classes and most of them are
/// zero: a kerning subtable classifies most of a font and then kerns a small
/// fraction of the pairs. A zero record moves nothing, so reading it and
/// walking its value format is work with no result.
///
/// This may say "maybe" when the answer is no -- values fold to six bits -- but
/// never the reverse, so a row it rules out truly holds nothing.
fn pair_class_digests(
    data: &[u8],
    matrix: usize,
    class1_count: u16,
    class2_count: u16,
    first_format: u16,
    second_format: u16,
) -> SetDigests {
    let pair = record_size(first_format) + record_size(second_format);
    if pair == 0 {
        // No values at all: every record is empty, so nothing is ever holdable.
        return SetDigests::build(class1_count, |_| Some(1));
    }
    SetDigests::build(class1_count, |c1| {
        let row = matrix + c1 as usize * usize::from(class2_count) * pair;
        let mut summary = 0u64;
        for c2 in 0..usize::from(class2_count) {
            let at = row + c2 * pair;
            let bytes = data.get(at..at + pair)?;
            if bytes.iter().any(|&b| b != 0) {
                summary |= SetDigests::bit(c2 as u32);
            }
        }
        // Zero reads as "not built"; a row of all zeros is exactly what we want
        // to record, so give it a bit no class-2 value can collide with.
        Some(match summary {
            0 => 1u64 << 63,
            s => s,
        })
    })
}

#[cfg(test)]
mod self_contained {
    /// The claim at the top of this file, as a test.
    ///
    /// Self-containment is the kind of property that erodes one convenient
    /// import at a time, and nothing in the compiler complains: reaching for
    /// the glyph buffer from in here builds perfectly well. So read our own
    /// source and check.
    ///
    /// Only the shipped code is scanned -- everything up to the first
    /// `#[cfg(test)]` -- because tests are not part of what gets lifted, and a
    /// test that reads source looking for a path is otherwise apt to find its
    /// own. The permitted paths are this tree referring to itself, and whatever
    /// the `host` block declares: the single tie to the applying side, which
    /// exists so that mounting this tree elsewhere is one edit.
    #[test]
    fn no_path_leaves_this_module_except_through_host() {
        // The compiled form only. `gsub`, `gpos` and `contextual` are the
        // applying side and name this crate's types freely -- that is what
        // they are for.
        const FILES: &[(&str, &str)] = &[
            ("compile/mod.rs", include_str!("mod.rs")),
            ("compile/lookup.rs", include_str!("lookup.rs")),
            ("compile/set.rs", include_str!("set.rs")),
        ];
        let mut leaks = Vec::new();
        for (name, src) in FILES {
            let mut in_host = false;
            for (n, line) in src.lines().enumerate() {
                let t = line.trim();
                if t == "#[cfg(test)]" {
                    break;
                }
                if t.ends_with("mod host {") {
                    in_host = true;
                    continue;
                }
                if in_host {
                    in_host = t != "}";
                    continue;
                }
                // Prose may name anything; only code counts.
                if t.starts_with("//") {
                    continue;
                }
                for at in t.match_indices("crate").map(|(i, _)| i) {
                    let tail = &t[at..];
                    if tail.starts_with("crate::") && !tail.starts_with("crate::compile") {
                        leaks.push(format!("{name}:{}: {t}", n + 1));
                        break;
                    }
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "{} path(s) reach outside compile:
{}

If the applying side              genuinely needs this, put it in the filter module or on an impl              outside this tree, and leave the compiled form knowing only fonts.",
            leaks.len(),
            leaks.join("
")
        );
    }
}

#[cfg(all(test, feature = "std"))]
mod heap_cost {
    use super::*;
    use crate::hb::ot::lookup::LookupCache;
    use crate::FontRef;
    use read_fonts::TableProvider;

    /// Both tables of a font, as bytes and as parsed tables.
    struct Font<'a> {
        gsub: Option<Gsub<'a>>,
        gpos: Option<Gpos<'a>>,
        gsub_bytes: Option<Vec<u8>>,
        gpos_bytes: Option<Vec<u8>>,
    }

    fn load<'a>(font: &FontRef<'a>) -> Font<'a> {
        Font {
            gsub: font.gsub().ok(),
            gpos: font.gpos().ok(),
            gsub_bytes: font
                .table_data(read_fonts::types::Tag::new(b"GSUB"))
                .map(|d| d.as_bytes().to_vec()),
            gpos_bytes: font
                .table_data(read_fonts::types::Tag::new(b"GPOS"))
                .map(|d| d.as_bytes().to_vec()),
        }
    }

    /// What the interpreted path holds once every lookup has been read.
    ///
    /// Reading them all is the fair comparison: both sides fill lazily, so the
    /// question is what each costs for a font whose lookups have all been
    /// reached, not what each costs before anything happens.
    fn interpreted(f: &Font<'_>) -> usize {
        let mut total = 0;
        if let Some(gsub) = &f.gsub {
            let cache = LookupCache::new(gsub);
            for i in 0..gsub.lookup_list().map_or(0, |l| l.lookup_count()) {
                let _ = cache.get(gsub, i);
            }
            total += cache.heap_bytes();
        }
        if let Some(gpos) = &f.gpos {
            let cache = LookupCache::new(gpos);
            for i in 0..gpos.lookup_list().map_or(0, |l| l.lookup_count()) {
                let _ = cache.get(gpos, i);
            }
            total += cache.heap_bytes();
        }
        total
    }

    /// Where the compiled path's bytes go, for one font.
    #[derive(Default)]
    struct Parts {
        /// The per-lookup slot vectors, sized for every lookup in the table
        /// whether or not one has been compiled.
        slots: usize,
        /// The compiled lookups themselves, one box per slot actually filled.
        boxes: usize,
        /// The vectors holding the subtables, and the subtable records in them.
        subtable_vecs: usize,
        /// What a compiled lookup owns beyond that: its reach, its dispatch
        /// index, its pair filter, and each kind's own tables.
        owned: usize,
        /// The interned coverages, class maps and sets, shared across lookups.
        interned: usize,
        /// The digests that identify them.
        keys: usize,
        /// The compiler's own working buffers, grown to the largest subtable
        /// seen and kept for the next one.
        scratch: usize,
    }

    impl Parts {
        fn total(&self) -> usize {
            self.slots
                + self.boxes
                + self.subtable_vecs
                + self.owned
                + self.interned
                + self.keys
                + self.scratch
        }
    }

    fn parts(f: &Font<'_>, detail: Detail) -> Parts {
        let (sub, pos) = compile_font_with_detail(f.gsub.as_ref(), f.gpos.as_ref(), detail);
        let mut p = Parts::default();
        for (program, bytes) in [(&sub, &f.gsub_bytes), (&pos, &f.gpos_bytes)] {
            p.slots += program.len() * size_of::<lookup::CompiledLookupSlot>();
            let Some(b) = bytes else { continue };
            for i in 0..program.len() as u16 {
                let Some(l) = program.get(i, b) else { continue };
                let vec = l.subtables.len() * size_of::<Subtable>();
                p.boxes += size_of::<CompiledLookup>();
                p.subtable_vecs += vec;
                p.owned += l.heap_bytes() - vec;
            }
        }
        // Shared, so counted once.
        p.interned = sub.pool().heap_bytes();
        p.keys = sub.pool().key_bytes();
        // What the end of a shaping call does, and therefore what a caller is
        // actually left holding.
        sub.release_scratch();
        pos.release_scratch();
        p.scratch = (sub.scratch_capacity() + pos.scratch_capacity()) * size_of::<u32>();
        p
    }

    /// What the compiled path holds for the same font, at a given set budget.
    fn compiled(f: &Font<'_>, detail: Detail) -> usize {
        let (sub, pos) = compile_font_with_detail(f.gsub.as_ref(), f.gpos.as_ref(), detail);
        if let Some(b) = &f.gsub_bytes {
            for i in 0..sub.len() as u16 {
                let _ = sub.get(i, b);
            }
        }
        if let Some(b) = &f.gpos_bytes {
            for i in 0..pos.len() as u16 {
                let _ = pos.get(i, b);
            }
        }
        // The pool is shared, so counting it once is counting it right; the
        // GPOS program's own `heap_bytes` would double it.
        sub.heap_bytes() + pos.heap_bytes() - pos.pool().heap_bytes() - pos.pool().key_bytes()
    }

    #[allow(clippy::cast_precision_loss)]
    fn kib(bytes: usize) -> f64 {
        bytes as f64 / 1024.0
    }

    #[allow(clippy::cast_precision_loss)]
    fn ratio(a: usize, b: usize) -> f64 {
        a as f64 / b.max(1) as f64
    }

    #[test]
    fn report() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/fonts");
        println!();
        // The two per-subtable records, which is what the vectors above are
        // full of and what most of the difference comes down to.
        println!(
            "per lookup slot: {} bytes ({} of it the compiled lookup itself)",
            size_of::<lookup::CompiledLookupSlot>(),
            size_of::<CompiledLookup>(),
        );
        println!(
            "per subtable record: compiled {} bytes, interpreted {} bytes",
            size_of::<Subtable>(),
            size_of::<crate::hb::ot::lookup::SubtableInfo>(),
        );
        println!(
            "{:<30} {:>9} {:>9} {:>6}  {:>6} {:>6} {:>6} {:>6} {:>8}",
            "font",
            "interp KiB",
            "compiled",
            "ratio",
            "slots",
            "boxes",
            "subtbl",
            "owned",
            "interned"
        );
        let time = |f: &Font<'_>| {
            let reps = 20;
            let start = std::time::Instant::now();
            for _ in 0..reps {
                let _ = parts(f, Detail::Full);
            }
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(reps)
        };
        let mut budgets = Vec::new();
        let mut timings = Vec::new();
        let mut fonts: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ttf" || e == "otf"))
            .collect();
        fonts.sort();
        for path in fonts {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let font = FontRef::new(&data).unwrap();
            let f = load(&font);
            let interp = interpreted(&f);
            let p = parts(&f, Detail::Full);
            println!(
                "{:<30} {:>9.1} {:>9.1} {:>5.2}x  {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>8.1}",
                path.file_name().unwrap().to_string_lossy(),
                kib(interp),
                kib(p.total()),
                ratio(p.total(), interp),
                kib(p.slots),
                kib(p.boxes),
                kib(p.subtable_vecs),
                kib(p.owned),
                kib(p.interned),
            );
            timings.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                time(&f),
            ));
            budgets.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                [Detail::Full, Detail::Lean, Detail::Minimal].map(|d| compiled(&f, d)),
            ));
        }
        println!();
        for (name, ms) in timings {
            println!("compile {name:<32} {ms:>7.2}ms");
        }
        println!();
        println!(
            "{:<32} {:>9} {:>9} {:>9}",
            "detail", "full", "lean", "minimal"
        );
        for (name, at) in budgets {
            println!(
                "{name:<32} {:>9.1} {:>9.1} {:>9.1}",
                kib(at[0]),
                kib(at[1]),
                kib(at[2])
            );
        }
        println!();
    }
}

#[cfg(all(test, feature = "std", feature = "compile-path"))]
mod reached_cost {
    use crate::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};

    /// Font and a text that exercises it, as the shaping benchmark pairs them.
    const CASES: &[(&str, &str)] = &[
        ("Roboto-Regular.ttf", "en-thelittleprince.txt"),
        ("NotoNastaliqUrdu-Regular.ttf", "fa-thelittleprince.txt"),
        ("NotoSansDevanagari-Regular.ttf", "hi-words.txt"),
        ("Amiri-Regular.ttf", "fa-thelittleprince.txt"),
        ("SourceSerifVariable-Roman.ttf", "react-dom.txt"),
    ];

    #[allow(clippy::cast_precision_loss)]
    fn kib(bytes: usize) -> f64 {
        bytes as f64 / 1024.0
    }

    #[allow(clippy::cast_precision_loss)]
    fn pct(a: usize, b: usize) -> f64 {
        100.0 * a as f64 / b.max(1) as f64
    }

    #[allow(clippy::cast_precision_loss)]
    fn ratio(a: usize, b: usize) -> f64 {
        a as f64 / b.max(1) as f64
    }

    /// What each side holds after a plain shaping run.
    ///
    /// This is the number that matters, and it is not the one `heap_cost`
    /// reports. That one compiles every lookup in the font, which is the worst
    /// case and the fair way to compare the two forms *per lookup*. But a plan
    /// reaches a fraction of a font -- 54 of NotoSans Devanagari's 394
    /// substitution lookups shape Hindi -- and both sides fill lazily, so what
    /// a caller actually holds is this.
    ///
    /// Both caches fill on the same run, and over the same lookups: every
    /// lookup the plan lists is read by one side or compiled by the other,
    /// before either knows whether this buffer needs it. So the two columns
    /// price the same work.
    ///
    /// They did not always. While the compiled path still asked the
    /// interpreted one for a lookup's properties, the interpreted column
    /// covered every lookup in the plan and the compiled column only the ones
    /// a buffer had reached -- 54 subtables against 29 on Roboto -- and the
    /// ratio that came out of that was flattering by about the same factor.
    #[test]
    fn report() {
        println!();
        println!(
            "{:<32} {:>9} {:>5} {:>9} {:>11} {:>11} {:>8}",
            "font", "reached", "", "subtables", "interp KiB", "compiled", "ratio"
        );
        for (font_name, text_name) in CASES {
            let font_path = format!("{}/benches/fonts/{font_name}", env!("CARGO_MANIFEST_DIR"));
            let text_path = format!("{}/benches/texts/{text_name}", env!("CARGO_MANIFEST_DIR"));
            let (Ok(data), Ok(text)) = (
                std::fs::read(&font_path),
                std::fs::read_to_string(&text_path),
            ) else {
                continue;
            };
            let font = FontRef::new(&data).unwrap();
            let shaper_data = ShaperData::new(&font);
            let shaper = shaper_data.shaper(&font).build();

            // A default shape: no explicit features, properties guessed from
            // the text, which is what a caller gets without asking.
            let mut buffer = Some(UnicodeBuffer::new());
            for line in text.lines().take(400) {
                let mut b = buffer.take().unwrap();
                b.push_str(line);
                b.guess_segment_properties();
                buffer = Some(shaper.shape(b, ShapeOptions::new()).clear());
            }

            let tables = &shaper.ot_tables;
            // Fill the interpreted cache for exactly the lookups the compiled
            // one holds. Without this the two hold different sets: with the
            // compiled path on, a nested lookup recurses through the compiled
            // program and the interpreted cache never sees it, so a
            // chain-context font like Nastaliq would be compared against a
            // cache holding only its top-level lookups.
            if let Some(t) = tables.gsub.as_ref() {
                for i in 0..tables.gsub_compiled.len() as u16 {
                    if tables.gsub_compiled.is_compiled(i) {
                        let _ = t.lookups.get(&t.table, i);
                    }
                }
            }
            if let Some(t) = tables.gpos.as_ref() {
                for i in 0..tables.gpos_compiled.len() as u16 {
                    if tables.gpos_compiled.is_compiled(i) {
                        let _ = t.lookups.get(&t.table, i);
                    }
                }
            }
            let mut interp = 0;
            let mut total = 0;
            let mut hit = 0;
            for lookups in [
                tables.gsub.as_ref().map(|t| t.lookups),
                tables.gpos.as_ref().map(|t| t.lookups),
            ]
            .into_iter()
            .flatten()
            {
                interp += lookups.heap_bytes();
            }
            let mut compiled = 0;
            for program in [&tables.gsub_compiled, &tables.gpos_compiled] {
                compiled += program.heap_bytes();
                total += program.len();
                hit += (0..program.len() as u16)
                    .filter(|&i| program.is_compiled(i))
                    .count();
            }
            // Sanity: both sides should see the same lookups and the same
            // subtables, or the comparison is not comparing the same work.
            let mut interp_subs = 0;
            for lookups in [
                tables.gsub.as_ref().map(|t| {
                    (
                        t.lookups,
                        t.table.lookup_list().map_or(0, |l| l.lookup_count()),
                    )
                }),
                tables.gpos.as_ref().map(|t| {
                    (
                        t.lookups,
                        t.table.lookup_list().map_or(0, |l| l.lookup_count()),
                    )
                }),
            ]
            .into_iter()
            .flatten()
            {
                let (cache, n) = lookups;
                for i in 0..n {
                    if let Some(info) = cache.get_if_present(i) {
                        interp_subs += info.subtables.len();
                    }
                }
            }
            let mut compiled_subs = 0;
            let mut compiled_hit = 0;
            for (program, bytes) in [
                (
                    &tables.gsub_compiled,
                    tables
                        .gsub
                        .as_ref()
                        .map(|t| t.table.offset_data().as_bytes()),
                ),
                (
                    &tables.gpos_compiled,
                    tables
                        .gpos
                        .as_ref()
                        .map(|t| t.table.offset_data().as_bytes()),
                ),
            ] {
                let Some(b) = bytes else { continue };
                for i in 0..program.len() as u16 {
                    if program.is_compiled(i) {
                        compiled_hit += 1;
                        if let Some(l) = program.get(i, b) {
                            compiled_subs += l.subtables.len();
                        }
                    }
                }
            }
            let _ = compiled_hit;
            println!(
                "{font_name:<32} {:>4}/{:<4} {:>4.0}% {:>4}/{:<4} {:>11.1} {:>11.1} {:>7.2}x",
                hit,
                total,
                pct(hit, total),
                interp_subs,
                compiled_subs,
                kib(interp),
                kib(compiled),
                ratio(compiled, interp),
            );
        }
        println!();
    }
}

#[cfg(all(test, feature = "std", feature = "compile-path"))]
mod lookup_shapes {
    use crate::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};

    /// What the lookups a font actually reaches look like, so an optimisation
    /// aimed at them can be aimed at the right thing.
    #[test]
    fn report() {
        let font_name =
            std::env::var("SHAPES_FONT").unwrap_or_else(|_| "SourceSerifVariable-Roman.ttf".into());
        let text_name = std::env::var("SHAPES_TEXT").unwrap_or_else(|_| "react-dom.txt".into());
        let font_path = format!("{}/benches/fonts/{font_name}", env!("CARGO_MANIFEST_DIR"));
        let text_path = format!("{}/benches/texts/{text_name}", env!("CARGO_MANIFEST_DIR"));
        let (Ok(data), Ok(text)) = (
            std::fs::read(&font_path),
            std::fs::read_to_string(&text_path),
        ) else {
            return;
        };
        let font = FontRef::new(&data).unwrap();
        let shaper_data = ShaperData::new(&font);
        let shaper = shaper_data.shaper(&font).build();
        let mut buffer = Some(UnicodeBuffer::new());
        for line in text.lines().take(400) {
            let mut b = buffer.take().unwrap();
            b.push_str(line);
            b.guess_segment_properties();
            buffer = Some(shaper.shape(b, ShapeOptions::new()).clear());
        }

        println!("\n{font_name} / {text_name}");
        let tables = &shaper.ot_tables;
        for (name, program, bytes) in [
            (
                "GSUB",
                &tables.gsub_compiled,
                tables
                    .gsub
                    .as_ref()
                    .map(|t| t.table.offset_data().as_bytes()),
            ),
            (
                "GPOS",
                &tables.gpos_compiled,
                tables
                    .gpos
                    .as_ref()
                    .map(|t| t.table.offset_data().as_bytes()),
            ),
        ] {
            let Some(b) = bytes else { continue };
            for i in 0..program.len() as u16 {
                if !program.is_compiled(i) {
                    continue;
                }
                let Some(l) = program.get(i, b) else { continue };
                let kinds: Vec<&str> = l.subtables.iter().map(|s| kind_name(&s.kind)).collect();
                let ranked = l.subtables.iter().filter(|s| s.rank).count();
                let shapes: Vec<&str> = l.subtables.iter().map(|s| cov_shape(&s.cov)).collect();
                println!("      gate shapes: {}", shapes.join(", "));
                println!(
                    "      reach shape: {} ({} bytes)",
                    set_shape(l.reach()),
                    l.reach().heap_bytes()
                );
                println!(
                    "  {name} {i:>3}: {} subtable(s), {ranked} ranked, dispatch {}, reach {} :: {}",
                    l.subtables.len(),
                    l.dispatch.is_some(),
                    l.reach_len(),
                    kinds.join(", ")
                );
            }
        }
        println!();
    }

    fn set_shape(g: &super::set::GlyphSet) -> &'static str {
        use super::set::GlyphSet as G;
        match g {
            G::Empty => "Empty",
            G::Range { .. } => "Range",
            G::Bitmap { .. } => "Bitmap",
            G::Sorted { .. } => "Sorted",
            G::Ranges(_) => "Ranges",
        }
    }

    /// Which representation the picker chose, since that is what a probe costs.
    fn cov_shape(c: &super::set::Coverage) -> &'static str {
        use super::set::Coverage as C;
        match c {
            C::Empty => "Empty",
            C::Range { .. } => "Range",
            C::Bitmap { .. } => "Bitmap",
            C::Sorted { .. } => "Sorted",
            C::Ranges(_) => "Ranges",
        }
    }

    fn kind_name(k: &super::SubtableKind) -> &'static str {
        use super::SubtableKind as K;
        match k {
            K::SingleDelta { .. } => "SingleDelta",
            K::SingleList { .. } => "SingleList",
            K::Multiple { .. } => "Multiple",
            K::Ligature { .. } => "Ligature",
            K::Alternate { .. } => "Alternate",
            K::ReverseChain { .. } => "ReverseChain",
            K::SinglePos { .. } => "SinglePos",
            K::PairPos1 { .. } => "PairPos1",
            K::PairPos2 { .. } => "PairPos2",
            K::Cursive { .. } => "Cursive",
            K::MarkTo { .. } => "MarkTo",
            K::Rules { .. } => "Rules",
            K::ChainCtx3 { .. } => "ChainCtx3",
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod interner_key {
    use super::*;
    use crate::FontRef;
    use read_fonts::TableProvider;

    /// Is a coverage table's position in the layout table recoverable from the
    /// table itself?
    ///
    /// It decides what the interner can be keyed by, and the interner's key
    /// has to be exact: a wrong answer there hands back the wrong compiled
    /// coverage for a table that merely hashed the same.
    ///
    /// `offset_data` runs from the table to the end of the data it was read
    /// from, so the distance from the layout table's start is the difference of
    /// the two lengths. This pins that, because it is an assumption about
    /// `read-fonts` rather than about OpenType, and nothing else would notice
    /// if a release changed it.
    #[test]
    fn a_coverage_knows_where_it_is() {
        for (name, data, table) in layout_tables() {
            let Ok(font) = FontRef::new(&data) else {
                continue;
            };
            let Some(bytes) = font.table_data(read_fonts::types::Tag::new(&table)) else {
                continue;
            };
            let bytes = bytes.as_bytes();
            let lookups: Vec<CoverageTable> = match &table {
                b"GSUB" => font
                    .gsub()
                    .ok()
                    .into_iter()
                    .flat_map(gsub_coverages)
                    .collect(),
                _ => font
                    .gpos()
                    .ok()
                    .into_iter()
                    .flat_map(gpos_coverages)
                    .collect(),
            };
            for c in &lookups {
                let at = bytes.len() - c.offset_data().as_bytes().len();
                let own = coverage_bytes(c);
                assert_eq!(
                    bytes.get(at..at + own.len()),
                    Some(own),
                    "{name}: a coverage at {at} does not read back as itself"
                );
            }
        }
    }

    /// What interning by content buys over interning by position alone.
    ///
    /// Position is the exact key -- the same offset in the same table is the
    /// same bytes -- but two tables at different offsets can hold the same
    /// bytes, and filing those separately compiles each of them. This says how
    /// often that happens across a whole font.
    ///
    /// It happens enough to matter. An earlier version of this test looked
    /// only at the coverages the direct formats name, found positions and
    /// contents equal everywhere, and concluded position alone would do. The
    /// contextual formats are where the duplication is, and Nastaliq is almost
    /// entirely contextual: keying it by position alone grew what the interner
    /// holds from 46.8KiB to 70.3KiB.
    #[test]
    fn report() {
        println!();
        println!("{:<34} {:>10} {:>10}", "font", "positions", "tables");
        for (name, data, _) in layout_tables() {
            let Ok(font) = FontRef::new(&data) else {
                continue;
            };
            let (gsub, gpos) = (font.gsub().ok(), font.gpos().ok());
            let (sub, pos) = compile_font(gsub.as_ref(), gpos.as_ref());
            for (program, tag) in [(&sub, b"GSUB"), (&pos, b"GPOS")] {
                let Some(bytes) = font.table_data(read_fonts::types::Tag::new(tag)) else {
                    continue;
                };
                for i in 0..program.len() as u16 {
                    let _ = program.get(i, bytes.as_bytes());
                }
            }
            let (positions, tables) = sub.pool().sharing();
            println!("{name:<34} {positions:>10} {tables:>10}");
        }
        println!();
    }

    /// Each benchmark font, once per layout table it has.
    fn layout_tables() -> Vec<(String, Vec<u8>, [u8; 4])> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/fonts");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ttf" || e == "otf"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, data, *b"GSUB"));
        }
        out
    }

    fn gsub_coverages(gsub: Gsub<'_>) -> Vec<CoverageTable<'_>> {
        let mut out = Vec::new();
        let Ok(list) = gsub.lookup_list() else {
            return out;
        };
        for lookup in list.lookups().iter().flatten() {
            match lookup {
                SubstitutionLookup::Single(l) => {
                    for st in l.subtables().iter().flatten() {
                        out.extend(match st {
                            SingleSubst::Format1(t) => t.coverage().ok(),
                            SingleSubst::Format2(t) => t.coverage().ok(),
                        });
                    }
                }
                SubstitutionLookup::Ligature(l) => {
                    out.extend(
                        l.subtables()
                            .iter()
                            .flatten()
                            .filter_map(|t| t.coverage().ok()),
                    );
                }
                _ => {}
            }
        }
        out
    }

    fn gpos_coverages(gpos: Gpos<'_>) -> Vec<CoverageTable<'_>> {
        let mut out = Vec::new();
        let Ok(list) = gpos.lookup_list() else {
            return out;
        };
        for lookup in list.lookups().iter().flatten() {
            if let PositionLookup::Pair(l) = lookup {
                for st in l.subtables().iter().flatten() {
                    out.extend(match st {
                        PairPos::Format1(t) => t.coverage().ok(),
                        PairPos::Format2(t) => t.coverage().ok(),
                    });
                }
            }
        }
        out
    }
}
