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
//! * [`LengthEffect`] — whether applying can change the glyph count. Knowing it
//!   statically is what lets substitution run in place with no output buffer.
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
use core::sync::atomic::{AtomicBool, Ordering};
use read_fonts::tables::gpos::Gpos;
use read_fonts::tables::gsub::Gsub;
use read_fonts::{FontData, FontRead};

use super::set::{ClassMap, Coverage, Digest, GlyphSet, Interner};
use super::Table;

/// Whether applying a lookup can change the number of glyphs.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LengthEffect {
    /// 1 -> 1. Writes in place; positions never move.
    Preserving,
    /// n -> 1. Writes the result into the first slot and marks the rest dead.
    Shrinking,
    /// 1 -> n. Needs room, so it takes the rebuild path.
    Growing,
}

impl LengthEffect {
    /// The combined effect of a set of subtables.
    pub fn join(self, other: Self) -> Self {
        use LengthEffect::{Growing, Preserving, Shrinking};
        match (self, other) {
            (Growing, _) | (_, Growing) => Growing,
            (Shrinking, _) | (_, Shrinking) => Shrinking,
            _ => Preserving,
        }
    }
}

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

/// A context subtable's per-set and per-rule indexes, together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleIndex {
    /// One word per rule set. See [`SetDigests`].
    pub digests: SetDigests,
    /// One byte per rule. See [`RuleFirsts`].
    pub firsts: RuleFirsts,
}

impl RuleIndex {
    pub fn heap_bytes(&self) -> usize {
        self.digests.heap_bytes() + self.firsts.heap_bytes()
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
    pub kind: SubtableKind,
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
    /// Multiple substitution: one glyph becomes a sequence.
    ///
    /// The sequences stay in the font. `effect` is scanned at compile time
    /// because the format only grows when some sequence has more than one
    /// glyph — HarfBuzz special-cases a single-glyph sequence to substitute in
    /// place, and an empty one to delete — so most of these never need the
    /// rebuild path at all.
    Multiple { offset: u32, effect: LengthEffect },
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
        /// Unlike every other format, this one is applied straight from the
        /// font rather than from the fields below -- see `gpos::at_mark_to` for
        /// why. The rest is kept because it is what the shaper this came from
        /// applies, and because dropping it would make the two copies diverge
        /// for no gain.
        offset: u32,
        /// Coverage of what they attach to: bases, or other marks.
        bases: Arc<Coverage>,
        /// Offset of the mark array.
        mark_array: u32,
        /// Offset of the base or mark2 array.
        base_array: u32,
        class_count: u16,
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
        /// One word per rule set, and one byte per rule. Boxed together: they
        /// are the widest thing any kind carries, and every subtable of every
        /// format is as wide as the widest.
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
        if self.rank {
            self.cov.index(glyph)
        } else {
            self.cov.contains(glyph).then_some(0)
        }
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

    pub fn effect(&self) -> LengthEffect {
        match &self.kind {
            SubtableKind::SingleDelta { .. } | SubtableKind::SingleList { .. } => {
                LengthEffect::Preserving
            }
            SubtableKind::Ligature { .. } => LengthEffect::Shrinking,
            SubtableKind::Multiple { effect, .. } => *effect,
            // Positioning never changes the glyph count.
            SubtableKind::SinglePos { .. }
            | SubtableKind::PairPos1 { .. }
            | SubtableKind::PairPos2 { .. }
            | SubtableKind::MarkTo { .. }
            | SubtableKind::Cursive { .. }
            | SubtableKind::Alternate { .. } => LengthEffect::Preserving,
            // As with chain context, the effect is whatever the nested lookups
            // do.
            SubtableKind::Rules { .. } => LengthEffect::Preserving,
            // The effect is whatever its nested lookups do; resolved by the
            // program once every lookup is compiled.
            SubtableKind::ChainCtx3 { .. } => LengthEffect::Preserving,
            // One glyph in, one glyph out, and no nested lookups to change
            // that.
            SubtableKind::ReverseChain { .. } => LengthEffect::Preserving,
        }
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
    pub effect: LengthEffect,
    pub subtables: Vec<Subtable>,
    /// Union of every subtable's coverage: the lookup-level candidate filter.
    /// Tested, never indexed, so it carries no rank table.
    pub reach: GlyphSet,
    /// Three-word summary of `reach`, so a lookup that cannot touch a buffer is
    /// thrown away before the buffer is scanned at all.
    pub digest: Digest,
    /// Three-word summary of `pair_key`, when there is one. A lookup whose
    /// second component cannot appear in the buffer at all is thrown away
    /// without the scan that would otherwise discover it -- which is every
    /// `ccmp` and `liga` lookup on a line of English, since their second
    /// components are combining marks.
    pub pair_digest: Digest,
    /// Glyph-to-subtable index, for lookups with enough subtables to want one.
    pub dispatch: Option<Dispatch>,
    /// Which glyphs can follow which, for a lookup that is entirely pair
    /// positioning. See [`PairFilter`].
    ///
    /// Set by the compiler rather than derived here: building it well needs the
    /// pairs as the font lists them, and what reaches this point is already
    /// folded into summaries.
    pub pair_filter: Option<PairFilter>,
    /// Whether applying this lookup can move positions or kill them.
    ///
    /// Only a context can: its nested lookups may be anything, including a
    /// multiplication that splices. Everything else writes where it stands, so
    /// the candidate loop need not re-check that a position is still live or
    /// that the buffer is still the length it was. Kerning is the case that
    /// cares -- it is nearly every position of a line of Latin.
    pub unsettling: bool,
    /// Whether the candidate loop must run from the end of the buffer towards
    /// the start. True only for reverse chaining substitution, which is
    /// defined that way -- see [`SubtableKind::ReverseChain`].
    pub reverse: bool,
    /// Union of every subtable's second-component set, present only when *all*
    /// subtables are pair-keyable. A lookup mixing ligature and single subtables
    /// must not use it, or the single subtable's candidates would be rejected.
    pub pair_key: Option<GlyphSet>,
}

impl CompiledLookup {
    /// Convenience for tests and one-off use; allocates its own scratch.
    pub fn new(props: u32, subtables: Vec<Subtable>) -> Self {
        Self::new_in(props, subtables, &mut Vec::new())
    }

    /// Build reusing `scratch`, so compiling a whole font does not allocate a
    /// fresh buffer per lookup.
    pub fn new_in(props: u32, subtables: Vec<Subtable>, scratch: &mut Vec<u32>) -> Self {
        Self::new_with(props, subtables, scratch, true)
    }

    /// Build, optionally without the glyph-to-subtable dispatch index.
    pub fn new_with(
        props: u32,
        mut subtables: Vec<Subtable>,
        scratch: &mut Vec<u32>,
        accelerate: bool,
    ) -> Self {
        // Built by pushing, so the capacity is rounded up to a power of two and
        // a lookup with five subtables has paid for eight. These live for as
        // long as the font cache does, and at 104 bytes each the slack is the
        // largest single thing this holds -- the interpreted form shrinks its
        // own subtable vector for the same reason.
        subtables.shrink_to_fit();
        let effect = subtables
            .iter()
            .map(Subtable::effect)
            .reduce(LengthEffect::join)
            .unwrap_or(LengthEffect::Preserving);

        scratch.clear();
        for sub in &subtables {
            sub.extend_reach(scratch);
        }
        scratch.sort_unstable();
        scratch.dedup();
        let reach = GlyphSet::build(scratch);
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
        let pair_key = subtables
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
            .flatten();
        let unsettling = effect != LengthEffect::Preserving
            || subtables.iter().any(|s| {
                matches!(
                    s.kind,
                    SubtableKind::Rules { .. } | SubtableKind::ChainCtx3 { .. }
                )
            });
        Self {
            props,
            effect,
            subtables,
            reach,
            digest,
            pair_digest,
            pair_key,
            dispatch,
            // Filled by the compiler, which can see the font the pairs live in.
            pair_filter: None,
            unsettling,
            reverse,
        }
    }

    pub fn heap_bytes(&self) -> usize {
        // The vector holding the subtables, not just what they point at. Each
        // `Subtable` lives inside it, so its own size is counted here and not
        // in `Subtable::heap_bytes`.
        self.subtables.capacity() * size_of::<Subtable>()
            + self
                .subtables
                .iter()
                .map(Subtable::heap_bytes)
                .sum::<usize>()
            + self.reach.heap_bytes()
            + self.pair_key.as_ref().map_or(0, GlyphSet::heap_bytes)
            + self.dispatch.as_ref().map_or(0, Dispatch::heap_bytes)
            + self.pair_filter.as_ref().map_or(0, PairFilter::heap_bytes)
    }
}

/// One byte per rule: the value its first input step demands, folded to six
/// bits.
///
/// The set-level summary throws away a whole rule set when nothing in it can
/// match. What is left is sets that contain a live rule among many dead ones,
/// and those still cost a header parse each -- variable-length, so reaching the
/// first input value means walking the backtrack sequence out of the font.
/// Measured on Nastaliq: of 1291 headers parsed, 828 are parsed only to find
/// the rule wants something the buffer does not offer.
///
/// A byte per rule answers that without touching the font. It is the same fold
/// as the set summary, so the two use one offered-mask between them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RuleFirsts {
    /// Where each set's bytes begin, with a terminator.
    starts: Box<[u32]>,
    firsts: Box<[u8]>,
}

/// A rule that constrains nothing at its first input step, and so must always
/// be tried. Six-bit values never reach it.
pub const RULE_ALWAYS: u8 = 0xFF;

impl RuleFirsts {
    pub fn new(starts: Vec<u32>, firsts: Vec<u8>) -> Self {
        Self {
            starts: starts.into_boxed_slice(),
            firsts: firsts.into_boxed_slice(),
        }
    }

    /// The per-rule bytes for one set, or an empty slice if there is no index.
    #[inline]
    pub fn row(&self, set: u32) -> &[u8] {
        let (Some(&from), Some(&to)) = (
            self.starts.get(set as usize),
            self.starts.get(set as usize + 1),
        ) else {
            return &[];
        };
        self.firsts.get(from as usize..to as usize).unwrap_or(&[])
    }

    /// Whether rule `i` of `row` could match a buffer offering `offered`.
    #[inline]
    pub fn may_match(row: &[u8], i: usize, offered: u64) -> bool {
        match row.get(i) {
            Some(&RULE_ALWAYS) | None => true,
            Some(&b) => offered & (1u64 << b) != 0,
        }
    }

    pub fn heap_bytes(&self) -> usize {
        self.starts.len() * 4 + self.firsts.len()
    }
}

/// For each first glyph, which glyphs it can be paired with.
///
/// Measured on Roboto and a line of English: of 166 candidate positions a kern
/// lookup is handed, seven produce a kern. The rest are rejected inside the
/// subtables, but only after a dispatch, two coverage probes and two class
/// lookups apiece. This answers the same question during candidate selection,
/// in two loads and an `and`.
///
/// Direct-mapped on the low bits of the first glyph, and a bitmap of the second
/// folded to eight bits. Both fold, so a collision merges two glyphs' answers
/// -- conservative in the only direction that matters: it may keep a position
/// that will not kern, never drop one that will.
///
/// The width is not incidental. Roboto lists its kern pairs explicitly and a
/// busy letter has forty of them; forty entries in a sixty-four bit word is a
/// saturated word, and a saturated word says yes to everything. At sixty-four
/// bits this rejected sixteen percent of Roboto's candidates against
/// eighty-four percent of NotoSans's, which kerns by class and has sparse rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairFilter {
    mask: u32,
    /// [`PAIR_WORDS`] words per first-glyph slot.
    slots: Box<[u64]>,
}

/// Words per slot, so the second glyph folds to `6 + log2` bits.
pub const PAIR_WORDS: usize = 4;

impl PairFilter {
    /// `slots` holds [`PAIR_WORDS`] words for each of a power-of-two number of
    /// first-glyph slots.
    pub fn new(slots: Vec<u64>) -> Self {
        let count = slots.len() / PAIR_WORDS;
        debug_assert!(count.is_power_of_two());
        Self {
            mask: count as u32 - 1,
            slots: slots.into_boxed_slice(),
        }
    }

    /// Where a pair's bit lives: the word, and the bit within it.
    #[inline]
    pub fn locate(mask: u32, first: u32, second: u32) -> (usize, u64) {
        let word = (first & mask) as usize * PAIR_WORDS + (second >> 6) as usize % PAIR_WORDS;
        (word, 1u64 << (second & 63))
    }

    #[inline]
    pub fn may_pair(&self, first: u32, second: u32) -> bool {
        let (word, bit) = Self::locate(self.mask, first, second);
        self.slots[word] & bit != 0
    }

    pub fn heap_bytes(&self) -> usize {
        self.slots.len() * 8
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
    /// Indexed over the union of every subtable's reach.
    cov: Coverage,
    /// Where each covered glyph's list starts, with a terminator.
    starts: Box<[u32]>,
    /// Subtable indices, ascending within each glyph's list.
    subs: Box<[u16]>,
}

/// Below this many subtables the linear walk is cheaper than the indirection.
const DISPATCH_MIN_SUBTABLES: usize = 4;

/// Cap on the index's size, in (glyph, subtable) pairs.
const DISPATCH_MAX_ENTRIES: usize = 4096;

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

        let mut starts = Vec::with_capacity(union.len() + 1);
        let mut subs = Vec::with_capacity(pairs.len());
        let mut p = 0;
        for &g in union {
            starts.push(subs.len() as u32);
            while let Some(&packed) = pairs.get(p) {
                if (packed >> 16) as u32 != g {
                    break;
                }
                subs.push((packed & 0xFFFF) as u16);
                p += 1;
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
            cov: Coverage::build(union),
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
pub type CompiledLookupSlot = OnceLock<Option<Box<CompiledLookup>>>;

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
                let slot = OnceLock::new();
                let _ = slot.set(l.map(Box::new));
                slot
            })
            .collect();
        p
    }

    fn with_table(count: u16, pool: Arc<Interner>, table: Table, detail: super::Detail) -> Self {
        let compiler = Mutex::new(Compiler::with_detail(Arc::clone(&pool), detail));
        let mut lookups = Vec::new();
        lookups.resize_with(usize::from(count), OnceLock::new);
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
    pub fn get(&self, index: u16, data: &[u8]) -> Option<&CompiledLookup> {
        let slot = self.lookups.get(index as usize)?;
        // The hit path: no lock, no font parsing, just a load.
        if let Some(compiled) = slot.get() {
            return compiled.as_deref();
        }
        slot.get_or_init(|| self.compile(index, data).map(Box::new))
            .as_deref()
    }

    #[cold]
    fn compile(&self, index: u16, data: &[u8]) -> Option<CompiledLookup> {
        let data = FontData::new(data);
        let mut compiler = self.compiler.lock();
        self.scratch_held.store(true, Ordering::Relaxed);
        match self.table {
            Table::Gsub => compiler.gsub(&Gsub::read(data).ok()?, index).ok(),
            Table::Gpos => compiler.gpos(&Gpos::read(data).ok()?, index).ok(),
        }
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
    fn length_effect_joins_to_the_widest() {
        use LengthEffect::*;
        assert_eq!(Preserving.join(Preserving), Preserving);
        assert_eq!(Preserving.join(Shrinking), Shrinking);
        assert_eq!(Shrinking.join(Growing), Growing);
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
    fn single_lookup_is_preserving_and_has_no_pair_key() {
        let l = CompiledLookup::new(0, vec![single(&[5, 6])]);
        assert_eq!(l.effect, LengthEffect::Preserving);
        assert!(l.pair_key.is_none());
    }

    #[test]
    fn ligature_lookup_is_shrinking_and_pair_keyed() {
        let l = CompiledLookup::new(0, vec![lig(&[1], &[2])]);
        assert_eq!(l.effect, LengthEffect::Shrinking);
        assert_eq!(l.pair_key.as_ref().map(|c| c.len()), Some(1));
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
