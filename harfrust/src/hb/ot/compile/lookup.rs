//! Compiled lookups.
//!
//! HarfBuzz interprets font bytes on every probe: for each glyph, for each
//! subtable, re-read the header, binary-search coverage, try to apply. We
//! compile each lookup once and then run a small machine over the result.
//!
//! **Compile the index, borrow the payload.** Coverage and class tables are
//! probed for every glyph, so they are worth compiling into a fast owned form.
//! Substitute arrays, ligature sets, value records and anchors are read only
//! when a lookup actually applies, which measurement puts near zero — copying
//! those would be memory spent to speed up something that hardly happens. They
//! stay in the font, and we keep a byte offset to them.
//!
//! Offsets, not references: a compiled font cache outlives any particular borrow
//! of the font bytes, so it cannot carry a lifetime. Every offset here is
//! relative to the start of the layout table, which the caller supplies again at
//! apply time.
//!
//! That the caller supplies the table at apply time is also what makes the cache
//! lazy. A [`Program`] starts as one empty slot per lookup and compiles a slot
//! the first time something asks for it, because a plan reaches a small part of
//! a font: five of NotoSans's 404 lookups shape Latin text, and thirty-one of
//! Nastaliq's 195 shape a word of Urdu. Compiling the list up front is nine
//! times the time to first shape, spent almost entirely on lookups that never
//! run.
//!
//! The compile step also derives two things the font does not contain:
//!
//! * The **pair key** — for a ligature lookup, the set of glyphs that can appear
//!   as a *second* component. HarfBuzz filters on the first glyph, but that is
//!   the non-discriminating half: Roboto's `ccmp` covers 72% of English text by
//!   first glyph and ligates none of it, because the second component is always
//!   a combining mark. Filtering on the pair rejects all of it up front.

use super::sync::{Mutex, OnceLock};
use super::Compiler;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use read_fonts::tables::gpos::Gpos;
use read_fonts::tables::gsub::Gsub;
use read_fonts::{FontData, FontRead};

use super::set::{scan_budget, ClassMap, Coverage, Digest, GlyphSet, Interner, DEFAULT_BUDGET};
use super::Table;

/// A big-endian `u16` array living in the font, addressed by byte offset from
/// the start of the layout table.
///
/// Eight bytes here replaces a copy of the array. Reads are bounds checked and
/// return `None` on a malformed offset rather than trusting the font.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct U16Array {
    pub offset: u32,
    pub len: u16,
}

impl U16Array {
    #[inline]
    pub fn get(&self, table: &[u8], i: u32) -> Option<u16> {
        if i >= u32::from(self.len) {
            return None;
        }
        let at = self.offset as usize + i as usize * 2;
        let bytes: [u8; 2] = table.get(at..at + 2)?.try_into().ok()?;
        Some(u16::from_be_bytes(bytes))
    }

    #[inline]
    pub fn len(&self) -> u16 {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One word per set, summarising the values that set can accept.
///
/// The font is full of the same shape: a list of alternatives, keyed by the
/// glyph a lookup is anchored on, of which a run takes at most one. A rule set
/// holds hundreds of rules that each begin by testing the same next position; a
/// pair set holds every glyph its first glyph kerns with. Either way the
/// alternatives fit in a word -- bit `v & 63` for each -- and if that word
/// shares no bit with what the buffer offers, the whole set is dead and none of
/// it needs to come out of the font.
///
/// Measured on Nastaliq: half the rule sets a run enters are wholly dead, and
/// they hold half of every rule parsed. HarfBuzz has set digests for lookups
/// and for subtables, but not for these.
///
/// Built when the lookup is compiled, which is already the first time anything
/// asks for it -- compilation is lazy per lookup, so there is nothing to gain
/// from deferring further, and the array is allocated either way. Plain data:
/// no atomics, no "not built yet" state to test on every visit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetDigests {
    slots: Box<[u64]>,
}

impl SetDigests {
    /// Summarise `count` sets, calling `of` for each. `None` from `of` means the
    /// set could not be summarised and must stay open.
    pub fn build(count: u16, mut of: impl FnMut(u32) -> Option<u64>) -> Self {
        let slots = (0..u32::from(count))
            .map(|i| of(i).unwrap_or(u64::MAX))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { slots }
    }

    /// Whether a set could hold `value`. True for a set we have no summary of,
    /// since a filter may only ever reject what truly cannot match.
    #[inline]
    pub fn may_hold(&self, set: u32, value: u32) -> bool {
        match self.slots.get(set as usize) {
            Some(d) => d & Self::bit(value) != 0,
            None => true,
        }
    }

    /// Whether a set could hold any of the values in a summary.
    #[inline]
    pub fn may_hold_any(&self, set: u32, offered: u64) -> bool {
        match self.slots.get(set as usize) {
            Some(d) => d & offered != 0,
            None => true,
        }
    }

    /// The raw summary word for one set, for callers building a coarser filter
    /// out of these.
    #[inline]
    pub fn word(&self, set: u32) -> u64 {
        self.slots.get(set as usize).copied().unwrap_or(u64::MAX)
    }

    /// The bit one value occupies in a summary.
    #[inline]
    pub fn bit(value: u32) -> u64 {
        1u64 << (value & 63)
    }

    pub fn heap_bytes(&self) -> usize {
        self.slots.len() * 8
    }
}

/// The only name in this module that reaches outside it.
///
/// Everything else under `compile` describes a font and nothing else: sets,
/// class maps, offsets, summaries. What breaks that is the function pointer a
/// [`Subtable`] carries -- it has to name the applying side's context type, and
/// it has to be resolved when the subtable is built.
///
/// That is worth paying for: an indirect call beats a match on a dozen-variant
/// enum, and it lets each format's routine be sized for its own work rather
/// than for the widest arm of a shared dispatch. The tie is gathered here
/// rather than scattered through the tree, so moving this module between
/// shapers is one edit.
///
/// In the shaper this came from the block named five things: a context, a
/// buffer, a matcher, a match-position array and the dispatch. HarfBuzz bundles
/// all of those into its apply context, so here it names one -- which [`Apply`]
/// then pairs with the layout table and the program.
pub(crate) mod host {
    pub use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
}

use host::hb_ot_apply_context_t;

/// What every format needs beyond the payload it borrowed.
///
/// Built once per lookup, not once per position. Three things, and the reason
/// each is here rather than fetched:
///
/// * `host` is the shaper's own context, which already carries the buffer, the
///   two matchers, the match-position array and the nesting budget.
/// * `table` is the layout table every compiled offset is relative to. It is
///   reachable as `host.face.ot_tables.table_data(host.table_index)`, but that
///   is an `Option` and two derefs to answer a question whose answer cannot
///   change while a lookup runs.
/// * `program` is how a context reaches the lookups it invokes. Holding it
///   here is also what lets the compiled form stay free of lifetimes: the
///   offsets in it mean nothing without `table`, and this is where the two are
///   brought back together.
pub struct Apply<'a, 'f> {
    pub host: &'a mut hb_ot_apply_context_t<'f>,
    pub table: &'a [u8],
    pub program: &'a Program,
}

impl Apply<'_, '_> {
    /// The glyph at the cursor, which is the position a format was gated on.
    #[inline]
    pub fn glyph(&self) -> u32 {
        self.host.buffer.cur(0).glyph_id
    }
}

/// How to apply one subtable.
///
/// `index` is the coverage rank for the formats that read it, and zero for
/// those that only needed to know the glyph was covered -- the gate has already
/// run, so this is a value the callee would otherwise recompute.
///
/// Everything else a format needs is on the context: the position is
/// `ctx.buffer.idx`, the matcher and the match-position array are fields, and
/// the nesting budget is `ctx.nesting_level_left`. Returning `Option<()>`
/// rather than an end position follows the convention here, where a format
/// advances the cursor itself.
pub type ApplyFn = fn(&mut Apply, &CompiledLookup, &Subtable, u32) -> Option<()>;

/// Which function applies this format. Resolved once, when the subtable is
/// compiled, so no format is ever matched on at apply time.
///
/// This lives inside the module rather than outside it -- unlike the shaper
/// this came from, where the applying side owned it -- because the routines it
/// names are here too, in [`super::gsub`], [`super::gpos`] and
/// [`super::contextual`]. That is what shrinks the tie to the host down to a
/// single type.
pub fn dispatch_for(kind: &SubtableKind) -> ApplyFn {
    use super::{contextual, gpos, gsub};
    match kind {
        SubtableKind::SingleDelta { .. } => gsub::at_single_delta,
        SubtableKind::SingleList { .. } => gsub::at_single_list,
        SubtableKind::Multiple { .. } => gsub::at_multiple,
        SubtableKind::Ligature { .. } => gsub::at_ligature,
        SubtableKind::Alternate { .. } => gsub::at_alternate,
        SubtableKind::ReverseChain { .. } => gsub::at_reverse_chain,
        SubtableKind::SinglePos { .. } => gpos::at_single_pos,
        SubtableKind::PairPos1 { .. } => gpos::at_pair1,
        SubtableKind::PairPos2 { .. } => gpos::at_pair2,
        SubtableKind::Cursive { .. } => gpos::at_cursive,
        SubtableKind::MarkTo { .. } => gpos::at_mark_to,
        SubtableKind::Rules { .. } => contextual::at_rules,
        SubtableKind::ChainCtx3 { .. } => contextual::at_chain3,
    }
}

/// A context subtable's rule-set index: one word per set. See [`SetDigests`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleIndex {
    /// One word per rule set. See [`SetDigests`].
    pub digests: SetDigests,
}

impl RuleIndex {
    pub fn heap_bytes(&self) -> usize {
        self.digests.heap_bytes()
    }
}

/// One subtable, split into what every format has and what only one does.
///
/// Every format is gated by a coverage: a glyph either starts a match here or
/// it does not. Keeping that in the outer struct means the gate is a field
/// load rather than a walk through a dozen variants to find out where each one
/// keeps its coverage -- and the applying code, which has to match on the kind
/// anyway, gets to do that exactly once.
#[derive(Clone, Debug)]
pub struct Subtable {
    /// What decides whether this subtable can start at a glyph.
    pub cov: Arc<Coverage>,
    /// How to apply this format, resolved when the subtable is compiled.
    ///
    /// A pointer rather than a match on the kind: the applying code has to
    /// branch on the variant anyway to reach its payload, and this way it does
    /// so once, inside a function sized for that one format -- rather than in a
    /// dispatch whose stack frame has to fit the largest arm.
    pub apply: ApplyFn,
    /// What may occupy the position after the start, when the format says
    /// anything about it. Resolved when the subtable is compiled: a ligature
    /// keeps its second components here, and a chain context whichever of its
    /// second input or first lookahead applies.
    pub next: Option<Arc<GlyphSet>>,
    /// Whether the *index* into that coverage is used, or only membership. On
    /// a bitmap the two differ by a load and a popcount, and several formats
    /// never read the number.
    pub rank: bool,
    /// The gate, with the coverage's shape already chosen. See `gate_for`.
    gate: GateFn,
    pub kind: SubtableKind,
}

/// Whether this subtable can start at a glyph, and where in its coverage.
///
/// A function pointer for the same reason `apply` is one. `Coverage` has five
/// shapes and `rank` two readings of them, and which pair applies was settled
/// when the subtable was compiled -- but written as a match it is re-decided
/// per candidate, and the compiler turns ten arms into a jump table: a load of
/// the discriminant, a load from the table, an indirect jump. The pointer is
/// the same indirect jump with the two loads gone, and its target is stable
/// for as long as a lookup runs, which is what the branch predictor wants.
pub type GateFn = fn(&Coverage, u32) -> Option<u32>;

/// Pick the gate for a coverage of this shape, read this way.
///
/// Each of these matches the one shape it was chosen for and hands the rest
/// back to the general path -- a single compare that always goes the same way,
/// rather than a table.
fn gate_for(cov: &Coverage, rank: bool) -> GateFn {
    match (cov, rank) {
        (Coverage::Range { .. }, true) => |c, g| match c {
            Coverage::Range { first, len } => {
                let o = g.checked_sub(*first)?;
                (o < *len).then_some(o)
            }
            _ => c.index(g),
        },
        (Coverage::Range { .. }, false) => |c, g| match c {
            Coverage::Range { first, len } => (g.wrapping_sub(*first) < *len).then_some(0),
            _ => c.contains(g).then_some(0),
        },
        (Coverage::Bitmap { .. }, true) => |c, g| match c {
            Coverage::Bitmap { base, words, rank } => {
                let o = g.checked_sub(*base)? as usize;
                let w = *words.get(o / 64)?;
                let bit = o % 64;
                if (w >> bit) & 1 == 0 {
                    return None;
                }
                let below = if bit == 0 {
                    0
                } else {
                    (w & ((1u64 << bit) - 1)).count_ones()
                };
                Some(rank[o / 64] + below)
            }
            _ => c.index(g),
        },
        (Coverage::Bitmap { .. }, false) => |c, g| match c {
            Coverage::Bitmap { base, words, .. } => {
                let o = g.checked_sub(*base)? as usize;
                matches!(words.get(o / 64), Some(w) if (w >> (o % 64)) & 1 != 0).then_some(0)
            }
            _ => c.contains(g).then_some(0),
        },
        (_, true) => Coverage::index,
        (_, false) => |c, g| c.contains(g).then_some(0),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubtableKind {
    /// Format 1. The payload is a single delta, so there is nothing to borrow:
    /// the substitute is `(glyph + delta) & 0xFFFF`.
    SingleDelta { delta: i32 },
    /// Format 2. Substitute glyph ids stay in the font; applying is one indexed
    /// big-endian read at the coverage index.
    SingleList { subst: U16Array },
    /// Ligature sets stay in the font in their entirety. What we compile is
    /// `seconds` — the pair key, which the font does not contain.
    Ligature {
        /// Offset of the `LigatureSubstFormat1` subtable within the layout
        /// table. Ligature set offsets follow its six-byte header.
        offset: u32,
    },
    /// Multiple substitution: one glyph becomes a sequence, which stays in
    /// the font.
    Multiple { offset: u32 },
    /// Single positioning. One value record per covered glyph, or one shared
    /// by all of them, depending on the format.
    SinglePos {
        /// Which fields each record carries.
        value_format: u16,
        /// Offset of the record, or of the array of them.
        values: u32,
        /// Format 1 shares a single record; format 2 indexes by coverage.
        shared: bool,
    },
    /// Pair positioning, format 1: the pairs are listed explicitly.
    PairPos1 {
        /// One word per pair set: which glyphs its first glyph pairs with.
        /// See [`SetDigests`].
        ///
        /// The union across all pair sets is useless -- Roboto's kern lookup
        /// admits 569 glyphs as a second, and on English it rejects two
        /// candidates in thirty-four. Per first glyph it is a handful.
        seconds: SetDigests,
        offset: u32,
        first_format: u16,
        second_format: u16,
    },
    /// Pair positioning, format 2: a class matrix.
    ///
    /// The classes are compiled because every probe reads them; the matrix
    /// itself stays in the font, and it is the larger of the two by far -- a
    /// kerning subtable holds one record per class pair, thousands of them, of
    /// which a run touches a few dozen.
    PairPos2 {
        /// One word per class-1 row: which class-2 values that row has a
        /// non-zero record for. A class matrix is mostly zeros -- it has an
        /// entry for every pair of classes, and few pairs kern -- and a zero
        /// record moves nothing. See [`SetDigests`].
        rows: SetDigests,
        class1: Arc<ClassMap>,
        class2: Arc<ClassMap>,
        /// Offset of the first class-1 record.
        matrix: u32,
        first_format: u16,
        second_format: u16,
        class2_count: u16,
    },
    /// Cursive attachment: joins a glyph's entry anchor to the previous
    /// glyph's exit anchor.
    ///
    /// What makes a cursive script actually cursive. Nastaliq stacks its
    /// letters along a descending baseline this way, so without it the letters
    /// are all correct and all sitting on the wrong line.
    Cursive {
        /// Offset of the entry/exit record array.
        records: u32,
        /// What the anchor offsets inside those records are relative to.
        base: u32,
        count: u16,
    },
    /// Alternate substitution: one of several variants, chosen by the feature's
    /// value.
    ///
    /// The value rides in the mask, which is why this needs the lookup mask as
    /// well as the glyph's: the bits selecting the alternate are the lookup's
    /// own bits, shifted down.
    Alternate { offset: u32 },
    /// Mark-to-base and mark-to-mark attachment.
    ///
    /// Both have the same shape: a coverage of marks, a coverage of things to
    /// attach to, and two arrays of anchors indexed by mark class. Only the
    /// coverages compile; the anchors stay in the font, since a run reads a
    /// handful of the hundreds a font carries.
    MarkTo {
        /// Offset of the subtable itself.
        ///
        /// The one format applied straight from the font rather than from
        /// compiled fields -- see `gpos::at_mark_to`. What is compiled is the
        /// outer gate: a mark either starts an attachment here or it does not,
        /// and a run asks that of every glyph.
        offset: u32,
        /// What the marks attach to.
        to: AttachTo,
    },
    /// Rule-based context: sequence and chained-sequence contexts, formats 1
    /// and 2.
    ///
    /// One variant covers four formats because they differ only in how a
    /// position is compared — against a glyph id or against a class — and in
    /// whether the rule carries backtrack and lookahead. The rules themselves
    /// are variable-length arrays, so they stay in the font and only the
    /// coverage and class definitions are compiled.
    Rules {
        /// `None` means the rule compares glyph ids directly, which is format 1.
        input_classes: Option<Arc<ClassMap>>,
        /// Only chained format 2 has these.
        backtrack_classes: Option<Arc<ClassMap>>,
        lookahead_classes: Option<Arc<ClassMap>>,
        /// Offset of the rule-set offset array.
        rule_sets: u32,
        /// What those offsets are relative to: the subtable itself.
        base: u32,
        rule_set_count: u16,
        /// One word per rule set. Boxed because every subtable of every
        /// format is as wide as the widest kind, and this is otherwise it.
        index: Box<RuleIndex>,
        /// Whether rules carry backtrack and lookahead sequences.
        chained: bool,
    },
    /// Chained sequence context, format 3.
    ///
    /// Every coverage here is a pure membership test — the format never uses a
    /// coverage index — so all three arrays are [`GlyphSet`], and none of them
    /// carries a rank table.
    ChainCtx3 {
        /// Offset of the subtable itself, so a caller that needs the format's
        /// own reading of a glyph sequence -- `would_apply` does -- can reach
        /// it rather than reproduce it. Free: it fits in the padding the
        /// `chained` flag leaves behind.
        offset: u32,
        /// Nearest preceding glyph first.
        backtrack: Box<[Arc<GlyphSet>]>,
        /// The input coverages *after* the first, which is the gate and lives
        /// in the outer struct.
        input: Box<[Arc<GlyphSet>]>,
        lookahead: Box<[Arc<GlyphSet>]>,
        records: Box<[SeqRecord]>,
        /// Whether this came from a *chained* format 3 rather than a plain one.
        ///
        /// The two are otherwise the same subtable -- a plain one simply has no
        /// backtrack or lookahead -- and the shaper this came from collapsed
        /// them for exactly that reason. They part company on one thing only:
        /// which flag calls a match makes. A chained context reports its span
        /// through the out-buffer variants, a plain one does not, and a chained
        /// context with empty backtrack and lookahead is legal, so the emptiness
        /// cannot stand in for this.
        chained: bool,
    },
    /// Reverse chaining contextual single substitution, GSUB type 8.
    ///
    /// A single-glyph input with a context on both sides, and the one format
    /// that has to run from the end of the buffer towards the start. That is
    /// not an implementation detail: it is what makes the format expressible.
    /// A fraction font substitutes a digit for its numerator form when what
    /// follows is the fraction slash *or another numerator* -- so the decision
    /// at each position depends on the decision one to its right having already
    /// been made. Running forwards, the chain could never propagate.
    ///
    /// Like format 3 of the chain contexts, every coverage here is a pure
    /// membership test, so backtrack and lookahead are [`GlyphSet`]s. The
    /// substitutes are indexed by the gate's coverage index and stay in the
    /// font, exactly as [`SubtableKind::SingleList`] leaves them.
    ReverseChain {
        /// Nearest preceding glyph first.
        backtrack: Box<[Arc<GlyphSet>]>,
        /// Nearest following glyph first.
        lookahead: Box<[Arc<GlyphSet>]>,
        subst: U16Array,
    },
}

/// What a mark attaches to, which decides both how to find the target and how
/// its anchors are laid out.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AttachTo {
    /// A letter, found by looking past any intervening marks.
    Base,
    /// The mark immediately before, which is what stacks diacritics.
    Mark,
    /// A ligature, whose anchors are per component: a mark attaches to the
    /// component it came from, not to the ligature as a whole.
    Ligature,
}

/// "Apply lookup `lookup_index` at input position `seq_index`."
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SeqRecord {
    pub seq_index: u16,
    pub lookup_index: u16,
}

/// Equality skips `apply`: it is a function of the kind, and comparing
/// function addresses is not meaningful.
impl PartialEq for Subtable {
    fn eq(&self, other: &Self) -> bool {
        self.cov == other.cov
            && self.next == other.next
            && self.rank == other.rank
            && self.kind == other.kind
    }
}

impl Eq for Subtable {}

impl Subtable {
    /// A subtable whose coverage index is read, not just tested.
    pub fn ranked(cov: Arc<Coverage>, kind: SubtableKind) -> Self {
        Self {
            gate: gate_for(&cov, true),
            cov,
            apply: dispatch_for(&kind),
            next: None,
            rank: true,
            kind,
        }
    }

    /// A subtable that only asks whether the glyph is covered.
    pub fn member(cov: Arc<Coverage>, kind: SubtableKind) -> Self {
        Self {
            gate: gate_for(&cov, false),
            cov,
            apply: dispatch_for(&kind),
            next: None,
            rank: false,
            kind,
        }
    }

    /// Record what may follow the starting position.
    pub fn following(mut self, next: Option<Arc<GlyphSet>>) -> Self {
        self.next = next;
        self
    }

    /// Whether this subtable can start at `glyph`, and where in its coverage.
    ///
    /// A field load and one branch. The gate lives in the outer struct because
    /// every format has one, and whether the *index* is wanted or only
    /// membership is settled when the subtable is compiled: on a bitmap the
    /// two differ by a load and a popcount, and several formats never read the
    /// number. Pair positioning format 2 keys off class definitions, single
    /// positioning format 1 shares one record among everything it covers, and
    /// a rule set indexes by class when it has one.
    #[inline]
    pub fn gate(&self, glyph: u32) -> Option<u32> {
        (self.gate)(&self.cov, glyph)
    }

    /// Append every glyph this subtable can match at the starting position.
    fn extend_reach(&self, out: &mut Vec<u32>) {
        self.cov.extend_into(out);
    }

    /// The glyphs that may occupy the position immediately after a match
    /// starts, if the format constrains it at all.
    ///
    /// This is the pair key, generalised. For a ligature it is the set of
    /// second components. For chain context it is the second input coverage,
    /// or -- when the input is a single glyph -- the first lookahead coverage,
    /// which is exactly the case Roboto's `ccmp` hits: one input glyph
    /// followed by a combining mark that ASCII never produces.
    ///
    /// Pair positioning constrains it too, but measurement says the constraint
    /// is worthless: Roboto's kern lookup admits 569 glyphs as a second, and on
    /// English prose a pair key keeps 32 candidates of 34.
    fn next_set(&self) -> Option<&GlyphSet> {
        self.next.as_deref()
    }

    /// Heap bytes this subtable owns.
    ///
    /// Excludes two things by design: whatever stayed in the font, and whatever
    /// is shared through the interner. A subtable holding an `Arc` is charged
    /// for the pointer, and the table it points at is counted once, where it
    /// actually lives.
    pub fn heap_bytes(&self) -> usize {
        let shared = size_of::<Arc<Coverage>>();
        // The gate and the pair key, which live out here, plus the kind's own.
        shared + self.next.as_ref().map_or(0, |_| shared) + self.kind.heap_bytes()
    }
}

impl SubtableKind {
    /// Heap bytes the kind owns, beyond the gate and the pair key.
    pub fn heap_bytes(&self) -> usize {
        let shared = size_of::<Arc<Coverage>>();
        match self {
            SubtableKind::SingleDelta { .. }
            | SubtableKind::SingleList { .. }
            | SubtableKind::Multiple { .. }
            | SubtableKind::SinglePos { .. }
            | SubtableKind::Ligature { .. }
            | SubtableKind::Cursive { .. }
            | SubtableKind::Alternate { .. } => 0,
            SubtableKind::PairPos1 { seconds, .. } => seconds.heap_bytes(),
            SubtableKind::PairPos2 { rows, .. } => shared * 2 + rows.heap_bytes(),
            SubtableKind::Rules { index, .. } => shared * 3 + index.heap_bytes(),
            SubtableKind::MarkTo { .. } => shared,
            SubtableKind::ChainCtx3 {
                backtrack,
                input,
                lookahead,
                records,
                ..
            } => {
                (backtrack.len() + input.len() + lookahead.len()) * size_of::<Arc<GlyphSet>>()
                    + records.len() * size_of::<SeqRecord>()
            }
            SubtableKind::ReverseChain {
                backtrack,
                lookahead,
                ..
            } => (backtrack.len() + lookahead.len()) * size_of::<Arc<GlyphSet>>(),
        }
    }
}

/// A lookup, resolved into owned filters plus offsets into the font.
#[derive(Clone, Debug)]
pub struct CompiledLookup {
    pub props: u32,
    pub subtables: Box<[Subtable]>,
    /// Union of every subtable's coverage: the lookup-level candidate filter.
    /// Tested, never indexed, so it carries no rank table.
    ///
    /// For a one-subtable lookup this holds the same glyphs as that subtable's
    /// coverage -- 20KiB of them on Amiri -- and is still worth keeping. A
    /// coverage has to answer *where* a glyph is, so the picker gives it a
    /// shape that can; a set only has to answer *whether*. The scan asks the
    /// cheaper question far more often than the gate asks the dearer one, and
    /// scanning the coverage instead costs 14% on a line of English.
    reach: GlyphSet,
    /// Three-word summary of `reach`, so a lookup that cannot touch a buffer is
    /// thrown away before the buffer is scanned at all.
    pub digest: Digest,
    /// Three-word summary of what may follow a start, when the formats in this
    /// lookup all constrain it. A lookup whose
    /// second component cannot appear in the buffer at all is thrown away
    /// without the scan that would otherwise discover it -- which is every
    /// `ccmp` and `liga` lookup on a line of English, since their second
    /// components are combining marks.
    pub pair_digest: Digest,
    /// Glyph-to-subtable index, for lookups with enough subtables to want one.
    /// Behind a pointer because most lookups have none. Inline it is the
    /// largest field here by some way -- seventy-two bytes of vectors and
    /// bounds against twenty-four for the reach -- and a font's lookups are
    /// mostly one or two subtables, which want no index at all. The
    /// indirection is paid only where there is something to point at, and
    /// there by a path that was going to chase pointers anyway.
    pub dispatch: Option<Box<Dispatch>>,
    /// Whether the candidate loop must run from the end of the buffer towards
    /// the start. True only for reverse chaining substitution, which is
    /// defined that way -- see [`SubtableKind::ReverseChain`].
    pub reverse: bool,
}

impl CompiledLookup {
    /// The union set, for a lookup that has one. Empty for a one-subtable
    /// lookup, whose reach is its subtable's coverage.
    #[inline]
    pub fn reach(&self) -> &GlyphSet {
        &self.reach
    }

    /// Whether this lookup could apply at `glyph`.
    ///
    /// A lookup with one subtable has a reach which *is* that subtable's
    /// coverage, so it asks the coverage and the union set is not consulted.
    /// The branch is on the subtable count, which cannot change under a scan.
    #[inline]
    pub fn may_reach(&self, glyph: u32) -> bool {
        match &self.subtables[..] {
            [sub] => sub.cov.contains(glyph),
            _ => self.reach.contains(glyph),
        }
    }

    /// Every glyph this lookup could apply at, appended to `out`.
    pub fn reach_into(&self, out: &mut Vec<u32>) {
        match &self.subtables[..] {
            [sub] => sub.cov.extend_into(out),
            _ => self.reach.extend_into(out),
        }
    }

    /// How many glyphs it could apply at.
    pub fn reach_len(&self) -> usize {
        match &self.subtables[..] {
            [sub] => sub.cov.len(),
            _ => self.reach.len(),
        }
    }

    /// Convenience for tests and one-off use; allocates its own scratch.
    pub fn new(props: u32, subtables: Vec<Subtable>) -> Self {
        Self::new_in(props, subtables, &mut Vec::new())
    }

    /// Build reusing `scratch`, so compiling a whole font does not allocate a
    /// fresh buffer per lookup.
    pub fn new_in(props: u32, subtables: Vec<Subtable>, scratch: &mut Vec<u32>) -> Self {
        Self::new_with(props, subtables, scratch, true, scan_budget(DEFAULT_BUDGET))
    }

    /// Build, optionally without the glyph-to-subtable dispatch index.
    pub fn new_with(
        props: u32,
        subtables: Vec<Subtable>,
        scratch: &mut Vec<u32>,
        accelerate: bool,
        scan_budget: usize,
    ) -> Self {
        // Built by pushing, so the capacity is rounded up to a power of two and
        // a lookup with five subtables has paid for eight. These live for as
        // long as the font cache does, and at 104 bytes each the slack is the
        // largest single thing this holds -- the interpreted form shrinks its
        // own subtable vector for the same reason. Boxed rather than shrunk,
        // which does the same and drops the capacity field with it.
        let subtables = subtables.into_boxed_slice();
        scratch.clear();
        for sub in &subtables {
            sub.extend_reach(scratch);
        }
        scratch.sort_unstable();
        scratch.dedup();
        // One subtable means the union is that subtable's coverage. Keep the
        // glyph list, which the rest of this needs, but not a second set.
        let reach = GlyphSet::build_with_budget(scratch, scan_budget);
        let digest = Digest::from_glyphs(scratch.iter().copied());
        // Keep the union: the dispatch index is built over it, and building it
        // reuses `scratch` for each subtable's own reach.
        let scratch_union = scratch.clone();

        let reverse = subtables
            .iter()
            .any(|s| matches!(s.kind, SubtableKind::ReverseChain { .. }));

        // Pair-keyable only if *every* subtable constrains the next position. A
        // subtable that does not can match regardless of what follows, and its
        // candidates must survive.
        //
        // A reverse lookup is never pair-keyable, and the reason is the whole
        // point of the format: it rewrites the glyph a candidate's key was read
        // from. Filtering `1` out of `123/` because a plain `2` follows it is
        // exactly wrong -- by the time the loop reaches position 0 that `2` is
        // a numerator, and the rule does match. The compiler leaves `next`
        // unset on these subtables, so this collect yields `None` on its own;
        // the assert is here because the invariant is not local to it.
        // Built to be summarised and then dropped. The exact set is worth
        // keeping per *subtable*, where a format consults it after its own
        // gate has admitted a candidate; at the lookup level it is a union
        // over subtables, so it is both weaker and asked earlier -- and three
        // words are enough to answer at the only point anything asks.
        let pair_key: Option<GlyphSet> = subtables
            .iter()
            .map(Subtable::next_set)
            .collect::<Option<Vec<_>>>()
            .filter(|v| !v.is_empty())
            .map(|v| union_sets_in(v.into_iter(), scratch));
        // No pair key means no extra constraint, so the digest must admit
        // everything rather than reject.
        let pair_digest = match &pair_key {
            Some(k) => {
                scratch.clear();
                k.extend_into(scratch);
                Digest::from_glyphs(scratch.iter().copied())
            }
            None => Digest::FULL,
        };

        debug_assert!(!(reverse && pair_key.is_some()));

        let dispatch = accelerate
            .then(|| Dispatch::build(&subtables, &scratch_union, scratch))
            .flatten()
            .map(Box::new);
        Self {
            props,
            subtables,
            reach,
            digest,
            pair_digest,
            dispatch,
            reverse,
        }
    }

    pub fn heap_bytes(&self) -> usize {
        // The vector holding the subtables, not just what they point at. Each
        // `Subtable` lives inside it, so its own size is counted here and not
        // in `Subtable::heap_bytes`.
        self.subtables.len() * size_of::<Subtable>()
            + self
                .subtables
                .iter()
                .map(Subtable::heap_bytes)
                .sum::<usize>()
            + self.reach.heap_bytes()
            + self
                .dispatch
                .as_ref()
                .map_or(0, |d| size_of::<Dispatch>() + d.heap_bytes())
    }
}

/// Which subtables of a lookup can start at a given glyph.
///
/// A lookup tries its subtables in order and takes the first that applies, so
/// the obvious loop asks every one of them "do you cover this glyph?". That is
/// fine for the two or three subtables most lookups have. Nastaliq has a
/// chained-context lookup with a hundred and four, and walking all of them per
/// candidate is the single largest cost in shaping Urdu.
///
/// The answer is already known at compile time: a subtable's reach is exactly
/// what gates it. So this inverts the question once -- glyph to subtables,
/// rather than subtable to glyphs -- and dispatch becomes one indexed probe and
/// a short list, usually of length one. Order is preserved, because the lists
/// are built by walking the subtables in order.
///
/// Only worth its own memory when there are enough subtables to amortise it,
/// and only built when the lists stay small: a lookup whose subtables each
/// cover thousands of glyphs would spend more on the index than it saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    /// Turns a glyph into a row of `starts`. See [`Dispatch::build`].
    cov: Coverage,
    /// Where each row starts, with a terminator.
    starts: Box<[u32]>,
    /// Subtable indices, ascending within each glyph's list.
    subs: Box<[u16]>,
}

/// Below this many subtables the linear walk is cheaper than the indirection.
const DISPATCH_MIN_SUBTABLES: usize = 4;

/// Cap on the index's size, in (glyph, subtable) pairs.
const DISPATCH_MAX_ENTRIES: usize = 4096;

/// How much wider than the glyphs it covers a span-indexed row table may be
/// before ranking is worth its cost. See [`Dispatch::build`].
const DISPATCH_SPAN_SLACK: usize = 2;

impl Dispatch {
    fn build(subtables: &[Subtable], union: &[u32], scratch: &mut Vec<u32>) -> Option<Self> {
        if subtables.len() < DISPATCH_MIN_SUBTABLES || subtables.len() > usize::from(u16::MAX) {
            return None;
        }
        // (glyph, subtable) packed so one sort puts every glyph's list together
        // and in subtable order.
        let mut pairs: Vec<u64> = Vec::new();
        for (i, sub) in subtables.iter().enumerate() {
            scratch.clear();
            sub.extend_reach(scratch);
            if pairs.len() + scratch.len() > DISPATCH_MAX_ENTRIES {
                return None;
            }
            pairs.extend(scratch.iter().map(|&g| u64::from(g) << 16 | i as u64));
        }
        pairs.sort_unstable();

        let (&base, &last) = (union.first()?, union.last()?);
        let span = (last - base) as usize + 1;
        // What turns a glyph into a row. Ranking within the union is the
        // general answer and costs a popcount per candidate; but when the
        // union is nearly contiguous, a coverage of the whole span answers the
        // same question with a subtract, and the glyphs it admits that no
        // subtable covers get an empty row, which is the answer they wanted.
        //
        // A word per glyph of span is the price, so it is only offered while
        // the span stays close to the count. Both forms earn their keep:
        // SourceSerif's kerning is five subtables over a contiguous thousand
        // glyphs and asked about nearly every position -- ranking it was six
        // percent of a run -- while Amiri's contexts are scattered, and
        // spanning those instead costs 150KiB and gains nothing.
        let dense = span <= union.len() * DISPATCH_SPAN_SLACK;
        let cov = if dense {
            Coverage::Range {
                first: base,
                len: span as u32,
            }
        } else {
            Coverage::build(union)
        };

        let mut starts = Vec::with_capacity(if dense { span } else { union.len() } + 1);
        let mut subs = Vec::with_capacity(pairs.len());
        let mut p = 0;
        let mut push_row = |g: u32, starts: &mut Vec<u32>, subs: &mut Vec<u16>| {
            starts.push(subs.len() as u32);
            while let Some(&packed) = pairs.get(p) {
                if (packed >> 16) as u32 != g {
                    break;
                }
                subs.push((packed & 0xFFFF) as u16);
                p += 1;
            }
        };
        if dense {
            for g in base..=last {
                push_row(g, &mut starts, &mut subs);
            }
        } else {
            for &g in union {
                push_row(g, &mut starts, &mut subs);
            }
        }
        starts.push(subs.len() as u32);
        // Does it actually narrow anything? Without the index a glyph tries
        // every subtable; with it, its row plus one coverage probe to find the
        // row. A lookup whose subtables mostly cover the same glyphs has rows
        // nearly as long as the list, and then the index is a second probe
        // buying nothing -- which is not hypothetical: building these
        // unconditionally is measurably slower on two of seven benchmarks.
        if subs.len() * 2 > union.len() * subtables.len() {
            return None;
        }
        Some(Self {
            cov,
            starts: starts.into_boxed_slice(),
            subs: subs.into_boxed_slice(),
        })
    }

    /// The subtables that can start at `glyph`, in the order they must be tried.
    #[inline]
    pub fn row(&self, glyph: u32) -> &[u16] {
        let Some(i) = self.cov.index(glyph) else {
            return &[];
        };
        let (Some(&from), Some(&to)) =
            (self.starts.get(i as usize), self.starts.get(i as usize + 1))
        else {
            return &[];
        };
        self.subs.get(from as usize..to as usize).unwrap_or(&[])
    }

    pub fn heap_bytes(&self) -> usize {
        self.cov.heap_bytes() + self.starts.len() * 4 + self.subs.len() * 2
    }
}

/// Every compiled lookup of a layout table, addressable by lookup-list index.
///
/// Chain context recurses by index, so applying one lookup needs the whole set.
/// Entries that could not be compiled are `None`, which the runtime treats as
/// "does not apply" — wrong output is preferable to a panic only if it is loud,
/// so callers check [`Program::missing`] to know what they are missing.
/// One slot of a [`Program`], named so its size can be accounted for.
///
/// Boxed, and that is the whole reason this is a named type. A slot holding a
/// `CompiledLookup` inline is 232 bytes, and there is one per lookup in the
/// font whether or not a plan ever reaches it -- 394 of them in NotoSans
/// Devanagari, of which shaping Hindi touches 54. Behind a box the empty slot
/// costs a pointer and the reached ones cost what they always did.
/// One lookup's slot: the compiled form once it exists, and the cheap summary
/// that decides whether it ever should.
///
/// The two live together because the hot loop asks about both, in that order,
/// for every lookup of every buffer. Kept in separate arrays they were two
/// cache lines per lookup and the second one was paid on every buffer for
/// every lookup the summary keeps declining -- which on Latin is most of them.
#[derive(Default, Debug)]
pub struct CompiledLookupSlot {
    lookup: OnceLock<Option<Box<CompiledLookup>>>,
    digest: AtomicDigest,
}

/// What [`Program::compiled`] found in a slot.
pub enum Compiled<'a> {
    /// Compiled already. `None` inside means it compiled to nothing, which is
    /// a lookup that does nothing -- see [`Program::compile`].
    Already(Option<&'a CompiledLookup>),
    /// Not compiled yet, and the only state in which the cheap summary is
    /// worth consulting.
    NotYet,
}

impl CompiledLookupSlot {
    #[inline]
    fn get(&self) -> Option<&Option<Box<CompiledLookup>>> {
        self.lookup.get()
    }
}

/// A lookup's summary, as three words that can be written from any thread.
///
/// Not a `OnceLock`: one of those around a `Digest` is thirty-two bytes and
/// there is one per lookup, which triples the array the pass indexes and costs
/// more in cache misses than the summary saves. Three relaxed words are
/// twenty-four with no state beside them, and need none -- every thread that
/// builds this builds the same value, so a race writes the same bits twice.
///
/// All zero means "not built yet". A lookup that really summarises to nothing
/// can never match anything, so it is summarised again each time and skipped
/// each time, which costs nothing anyone will find.
#[derive(Default, Debug)]
pub struct AtomicDigest([AtomicU64; 3]);

impl AtomicDigest {
    #[inline]
    fn get(&self) -> [u64; 3] {
        [
            self.0[0].load(Ordering::Relaxed),
            self.0[1].load(Ordering::Relaxed),
            self.0[2].load(Ordering::Relaxed),
        ]
    }

    #[inline]
    fn set(&self, words: &[u64; 3]) {
        for (slot, word) in self.0.iter().zip(words) {
            slot.store(*word, Ordering::Relaxed);
        }
    }
}

/// Every compiled lookup of a layout table, addressable by lookup-list index.
///
/// Chain context recurses by index, so applying one lookup needs the whole set.
/// Entries that could not be compiled are `None`, which the runtime treats as
/// "does not apply" — wrong output is preferable to a panic only if it is loud,
/// so callers check [`Program::missing`] to know what they are missing.
/// One slot of a [`Program`], named so its size can be accounted for.
///
/// Boxed, and that is the whole reason this is a named type. A slot holding a
/// `CompiledLookup` inline is 232 bytes, and there is one per lookup in the
/// font whether or not a plan ever reaches it -- 394 of them in NotoSans
/// Devanagari, of which shaping Hindi touches 54. Behind a box the empty slot
/// costs a pointer and the reached ones cost what they always did.
#[derive(Debug)]
pub struct Program {
    /// One slot per lookup in the font, each compiled on first use.
    ///
    /// A plan touches a small part of a font: five of NotoSans's 404 lookups
    /// shape Latin text. Compiling all 404 to run five is most of the cost of
    /// the first shape, and all of it is avoidable, so a slot stays empty until
    /// something asks for it.
    lookups: Vec<CompiledLookupSlot>,
    /// The interning index, shared with every lookup compiled into this program
    /// and with anything else built against the same font. Shaping never reads
    /// it: subtables hold their tables directly.
    pool: Arc<Interner>,
    /// Whether the compiler is holding working buffers worth giving back. Read
    /// once per shaping call, so it must not be a lock.
    scratch_held: AtomicBool,
    /// Held for the lookups not compiled yet. Behind a lock because compiling
    /// happens through a shared reference, from whichever thread shapes first --
    /// and because the compiler carries scratch buffers whose whole point is to
    /// be reused across lookups rather than reallocated per lookup.
    ///
    /// The lock is only taken on a miss. A compiled lookup is read straight out
    /// of its slot.
    compiler: Mutex<Compiler>,
    /// Which table these came from. Positioning steps over glyphs that
    /// substitution may not, so the matcher needs to know.
    table: Table,
}

impl Default for Program {
    fn default() -> Self {
        Self::new(0, Arc::default())
    }
}

impl Program {
    /// A program over `count` lookups, none of them compiled yet.
    pub fn new(count: u16, pool: Arc<Interner>) -> Self {
        Self::with_table(count, pool, Table::Gsub, super::Detail::Full)
    }

    pub fn new_gpos(count: u16, pool: Arc<Interner>) -> Self {
        Self::with_table(count, pool, Table::Gpos, super::Detail::Full)
    }

    /// The same, at a chosen level of precomputation. See
    /// [`Detail`](super::Detail).
    pub fn new_with_detail(count: u16, pool: Arc<Interner>, detail: super::Detail) -> Self {
        Self::with_table(count, pool, Table::Gsub, detail)
    }

    pub fn new_gpos_with_detail(count: u16, pool: Arc<Interner>, detail: super::Detail) -> Self {
        Self::with_table(count, pool, Table::Gpos, detail)
    }

    /// A program over lookups that are already compiled.
    ///
    /// For callers holding compiled forms from somewhere other than this font's
    /// lookup list -- tests that build a lookup by hand, mostly. `get` never
    /// compiles here: every slot is already filled.
    pub fn prebuilt(lookups: Vec<Option<CompiledLookup>>, pool: Arc<Interner>) -> Self {
        let mut p = Self::with_table(0, pool, Table::Gsub, super::Detail::Full);
        p.lookups = lookups
            .into_iter()
            .map(|l| {
                let slot = CompiledLookupSlot::default();
                let _ = slot.lookup.set(l.map(Box::new));
                // Prebuilt lookups are never gated: nothing can be summarised
                // from a font that was not read.
                slot.digest.set(Digest::FULL.words());
                slot
            })
            .collect();
        p
    }

    fn with_table(count: u16, pool: Arc<Interner>, table: Table, detail: super::Detail) -> Self {
        let compiler = Mutex::new(Compiler::with_detail(Arc::clone(&pool), detail));
        let mut lookups = Vec::new();
        lookups.resize_with(usize::from(count), CompiledLookupSlot::default);
        Self {
            lookups,
            pool,
            compiler,
            table,
            scratch_held: AtomicBool::new(false),
        }
    }

    #[inline]
    pub fn table(&self) -> Table {
        self.table
    }

    /// The interning index, for compiling further lookups later.
    #[inline]
    /// Words of scratch the compiler is holding, so a caller accounting for
    /// this program can see what compilation left behind.
    pub fn scratch_capacity(&self) -> usize {
        self.compiler.lock().scratch_capacity()
    }

    pub fn pool(&self) -> &Arc<Interner> {
        &self.pool
    }

    /// The compiled form of lookup `index`, compiling it if this is the first
    /// ask.
    ///
    /// `data` is the layout table, needed only on a miss: the compiled form
    /// holds offsets into it, so the caller has to supply it at apply time
    /// anyway. Re-reading the table header per compiled lookup is a few
    /// bounds checks, and it is what keeps this cache free of a lifetime.
    ///
    /// A lookup that fails to compile caches its failure, so a malformed
    /// subtable is diagnosed once rather than on every glyph.
    /// The compiled lookup, if it has been compiled already.
    ///
    /// `None` means not yet, which is the only state in which the cheap
    /// summary is worth consulting: once a lookup is compiled, it carries the
    /// same summary and reading it costs nothing extra.
    #[inline]
    pub fn compiled(&self, index: u16) -> Compiled<'_> {
        match self.lookups.get(index as usize).and_then(|s| s.get()) {
            Some(compiled) => Compiled::Already(compiled.as_deref()),
            None => Compiled::NotYet,
        }
    }

    /// Whether this lookup could touch a buffer holding `seen`.
    ///
    /// Answered from the font rather than from the compiled form, so a lookup
    /// that cannot is never compiled. See [`CompiledLookupSlot::digest`].
    ///
    /// The summary is built once and read on every buffer after that -- a
    /// lookup this keeps declining is never compiled, so it is this that keeps
    /// answering for it. Reading it has to be a load and three `and`s, which
    /// is why building it lives out of line.
    #[inline]
    pub fn may_touch(&self, index: u16, data: &[u8], seen: &[u64; 3]) -> bool {
        let Some(slot) = self.lookups.get(index as usize) else {
            return false;
        };
        let mut words = slot.digest.get();
        if words == [0; 3] {
            words = self.build_digest(&slot.digest, index, data);
        }
        words[0] & seen[0] != 0 && words[1] & seen[1] != 0 && words[2] & seen[2] != 0
    }

    /// Summarise a lookup from the font. Once per lookup, near enough.
    #[cold]
    fn build_digest(&self, slot: &AtomicDigest, index: u16, data: &[u8]) -> [u64; 3] {
        let data = FontData::new(data);
        let built = match self.table {
            Table::Gsub => Gsub::read(data)
                .ok()
                .and_then(|t| super::lookup_digest_gsub(&t, index)),
            Table::Gpos => Gpos::read(data)
                .ok()
                .and_then(|t| super::lookup_digest_gpos(&t, index)),
        };
        // A lookup that cannot be summarised is one nothing can be ruled out
        // about.
        let words = *built.unwrap_or(Digest::FULL).words();
        slot.set(&words);
        words
    }

    pub fn get(&self, index: u16, data: &[u8]) -> Option<&CompiledLookup> {
        let slot = self.lookups.get(index as usize)?;
        // The hit path: no lock, no font parsing, just a load.
        if let Some(compiled) = slot.get() {
            return compiled.as_deref();
        }
        slot.lookup
            .get_or_init(|| self.compile(index, data).map(Box::new))
            .as_deref()
    }

    #[cold]
    fn compile(&self, index: u16, data: &[u8]) -> Option<CompiledLookup> {
        let data = FontData::new(data);
        let mut compiler = self.compiler.lock();
        self.scratch_held.store(true, Ordering::Relaxed);
        let compiled = match self.table {
            Table::Gsub => compiler.gsub(&Gsub::read(data).ok()?, index).ok(),
            Table::Gpos => compiler.gpos(&Gpos::read(data).ok()?, index).ok(),
        };
        // A lookup left with no subtables is an empty slot, and an empty slot
        // is a lookup that does nothing.
        //
        // That is the whole contract, and it is worth stating because the
        // alternative -- falling back to reading the font for a lookup that
        // would not compile -- is what this deliberately does not do. A
        // subtable no reader can make sense of is one no reader can apply, so
        // compiling drops it and keeps the rest of the lookup; if that leaves
        // nothing, there is nothing to apply and nothing to fall back to. The
        // shaper this came from reaches the same place by a different route,
        // declining to sanitize a malformed subtable.
        //
        // The point is that every caller agrees. A nested lookup invoked by a
        // context sees an empty slot and applies nothing; so does the pass
        // over the buffer. There is no path on which one of them reads the
        // font and the other does not.
        compiled.filter(|lookup| !lookup.subtables.is_empty())
    }

    /// Drop the compiler's working buffers.
    ///
    /// Called when a shaping call finishes, which is the natural boundary:
    /// everything a plan reaches has been compiled by then, and what is left
    /// is a compiled form that never needs them again. See
    /// [`Compiler::release`](super::Compiler::release).
    ///
    /// Gated on a flag rather than taking the lock every time. A caller shapes
    /// a line at a time and compiles on almost none of them, so the common
    /// case has to be an atomic read: two lock acquisitions per line would be
    /// a real cost on a benchmark of short lines, to free nothing.
    pub fn release_scratch(&self) {
        if self.scratch_held.swap(false, Ordering::Relaxed) {
            self.compiler.lock().release();
        }
    }

    /// Compile every lookup, for callers that want the whole font rather than
    /// what a plan reaches: memory accounting, coverage checks, benchmarks.
    pub fn compile_all(&self, data: &[u8]) {
        for i in 0..self.lookups.len() as u16 {
            self.get(i, data);
        }
    }

    /// Whether this slot has been filled, for callers measuring how much of a
    /// font a plan actually reaches.
    pub fn is_compiled(&self, index: u16) -> bool {
        self.lookups
            .get(index as usize)
            .is_some_and(|s| s.get().is_some())
    }

    /// How many lookups have been compiled so far, which is the measure of what
    /// laziness actually saved.
    pub fn compiled_count(&self) -> usize {
        self.lookups.iter().filter(|l| l.get().is_some()).count()
    }

    pub fn len(&self) -> usize {
        self.lookups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lookups.is_empty()
    }

    /// Lookup-list indices that failed to compile. Forces the whole font, since
    /// an uncompiled lookup has no verdict yet.
    pub fn missing(&self, data: &[u8]) -> Vec<u16> {
        self.compile_all(data);
        self.lookups
            .iter()
            .enumerate()
            .filter(|(_, l)| matches!(l.get(), Some(None)))
            .map(|(i, _)| i as u16)
            .collect()
    }

    /// Everything this program owns: the slot vector, whatever has been
    /// compiled into it, the shared interner, and the compiler's own scratch.
    ///
    /// The slot vector is sized for every lookup in the table whether or not
    /// one has been compiled, so an empty program is not free -- that is the
    /// price of being able to fill a slot behind `&self`.
    pub fn heap_bytes(&self) -> usize {
        self.lookups.capacity() * size_of::<CompiledLookupSlot>()
            + self
                .lookups
                .iter()
                .filter_map(|l| l.get()?.as_deref())
                // The box itself, then what the lookup inside it owns.
                .map(|l| size_of::<CompiledLookup>() + l.heap_bytes())
                .sum::<usize>()
            + self.pool.heap_bytes()
            + self.pool.key_bytes()
            + self.compiler.lock().scratch_capacity() * size_of::<u32>()
    }
}

/// Union of several sets, reusing `scratch`.
fn union_sets_in<'a>(sets: impl Iterator<Item = &'a GlyphSet>, scratch: &mut Vec<u32>) -> GlyphSet {
    scratch.clear();
    for c in sets {
        c.extend_into(scratch);
    }
    scratch.sort_unstable();
    scratch.dedup();
    GlyphSet::build(scratch)
}

#[cfg(test)]
mod malformed {
    use super::*;

    /// A GSUB whose only lookup has one subtable, at an offset past the end of
    /// the table.
    ///
    /// Hand-built because the corpus has nothing like it: a font this broken
    /// is what a fuzzer produces, not what a foundry ships.
    fn gsub_with_bad_subtable_offset() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // version 1.0
        d.extend_from_slice(&10u16.to_be_bytes()); // scriptList
        d.extend_from_slice(&12u16.to_be_bytes()); // featureList
        d.extend_from_slice(&14u16.to_be_bytes()); // lookupList
        d.extend_from_slice(&0u16.to_be_bytes()); // empty script list
        d.extend_from_slice(&0u16.to_be_bytes()); // empty feature list
        d.extend_from_slice(&1u16.to_be_bytes()); // one lookup
        d.extend_from_slice(&4u16.to_be_bytes()); // at +4 from the list
        d.extend_from_slice(&1u16.to_be_bytes()); // type 1, single
        d.extend_from_slice(&0u16.to_be_bytes()); // no flags
        d.extend_from_slice(&1u16.to_be_bytes()); // one subtable
        d.extend_from_slice(&0xF000u16.to_be_bytes()); // ...far past the end
        d
    }

    /// The lookup compiles to an empty slot, and an empty slot is a lookup
    /// that does nothing.
    ///
    /// The thing being pinned is that it does not instead become a lookup the
    /// font gets read for. Every caller has to agree about a slot it cannot
    /// fill -- the pass over the buffer and a context invoking it as a nested
    /// lookup both see `None` and both apply nothing -- and the way to keep
    /// them agreeing is for there to be no other path.
    #[test]
    fn a_subtable_that_cannot_be_read_leaves_an_empty_slot() {
        let data = gsub_with_bad_subtable_offset();
        Gsub::read(FontData::new(&data)).expect("only the subtable is malformed, not the header");
        let program = Program::new(1, Arc::default());
        assert!(
            program.get(0, &data).is_none(),
            "a lookup with no readable subtable must not compile"
        );
        assert_eq!(program.len(), 1, "the slot still exists, it is just empty");
    }

    /// The same, one level down: a lookup whose subtables are all unreadable
    /// is empty however it is reached.
    #[test]
    fn an_empty_slot_is_empty_from_every_caller() {
        let data = gsub_with_bad_subtable_offset();
        let program = Program::new(1, Arc::default());
        // Twice, because the first call fills the slot and the second reads
        // what it filled -- the two paths through `get`.
        assert!(program.get(0, &data).is_none());
        assert!(program.get(0, &data).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Candidate filtering reads only the compiled sets, so a test subtable
    /// needs no font behind it.
    fn lig(firsts: &[u32], seconds: &[u32]) -> Subtable {
        Subtable::ranked(
            Arc::new(Coverage::build(firsts)),
            SubtableKind::Ligature { offset: 0 },
        )
        .following(Some(Arc::new(GlyphSet::build(seconds))))
    }

    fn single(cov: &[u32]) -> Subtable {
        Subtable::member(
            Arc::new(Coverage::build(cov)),
            SubtableKind::SingleDelta { delta: 5 },
        )
    }
    #[test]
    fn u16_array_reads_big_endian() {
        // Two bytes of padding, then [0x0102, 0x0304].
        let table = [0xff, 0xff, 0x01, 0x02, 0x03, 0x04];
        let a = U16Array { offset: 2, len: 2 };
        assert_eq!(a.get(&table, 0), Some(0x0102));
        assert_eq!(a.get(&table, 1), Some(0x0304));
        assert_eq!(a.get(&table, 2), None, "past len");
    }

    #[test]
    fn u16_array_refuses_to_read_past_the_table() {
        // A malformed font must yield None, not a panic and not garbage.
        let table = [0x00, 0x01];
        assert_eq!(U16Array { offset: 1, len: 4 }.get(&table, 0), None);
        assert_eq!(
            U16Array {
                offset: 9999,
                len: 1
            }
            .get(&table, 0),
            None
        );
    }

    #[test]
    fn borrowed_payload_costs_nothing() {
        // The point of holding an offset: a ligature subtable with hundreds of
        // rules weighs only its compiled filters.
        let s = lig(&[1, 2, 3], &[9]);
        assert_eq!(
            s.heap_bytes(),
            2 * size_of::<Arc<Coverage>>(),
            "the gate and the pair key, and nothing per rule"
        );
        assert_eq!(size_of::<U16Array>(), 8);
    }

    #[test]
    fn single_lookup_has_no_pair_key() {
        let l = CompiledLookup::new(0, vec![single(&[5, 6])]);
        assert_eq!(l.pair_digest, Digest::FULL, "nothing to key on");
    }

    #[test]
    fn ligature_lookup_is_pair_keyed() {
        let l = CompiledLookup::new(0, vec![lig(&[1], &[2])]);
        assert_ne!(
            l.pair_digest,
            Digest::FULL,
            "keyed on its second components"
        );
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    #[test]
    fn report_sizes() {
        println!("Subtable = {} bytes", size_of::<Subtable>());
        println!("SubtableKind = {} bytes", size_of::<SubtableKind>());
        println!("CompiledLookup = {} bytes", size_of::<CompiledLookup>());
        println!("Coverage = {} bytes", size_of::<Coverage>());
    }
}
