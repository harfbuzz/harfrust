//! Compiled glyph sets and class maps.
//!
//! A coverage table answers two questions: is this glyph covered, and if so what
//! is its index. HarfBuzz answers both by binary-searching font bytes on every
//! probe. We compile each set once per font into whichever representation is
//! cheapest to probe *that still fits a byte budget*, so a shaper never does
//! worse on memory than reading the font directly but usually does much better
//! on speed.
//!
//! The budget matters because bitmap cost scales with the glyph-id *span*, not
//! the member count: a 50-glyph coverage is 10 bytes in a small font and 3.7 KB
//! in a 30k-glyph font if the members are spread out. [`Coverage::build`] falls
//! back to range or sorted forms in exactly that case.

use super::sync::Mutex;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::hash::BuildHasher;

use hashbrown::{DefaultHashBuilder, HashTable};

/// Rough per-subtable ceiling, chosen so a compiled subtable stays in the same
/// range as the font bytes HarfBuzz would read instead.
pub const DEFAULT_BUDGET: usize = 384;

/// How much larger than the smallest form a faster one may be, when picking a
/// representation.
pub const SLACK: usize = 8;

/// How much more a set the candidate scan probes may spend than one probed
/// once per candidate.
///
/// A lookup's reach is asked about every glyph of every buffer the lookup is
/// entered for; a coverage is asked only about the glyphs the reach admitted,
/// which on English is one position in five. So the reach earns a bigger
/// allowance, and this is how much bigger. Measured on Amiri: at the plain
/// budget 38 of its 160 reached lookups scan by binary search, and giving them
/// bitmaps instead costs 23KiB and buys 10% of the run.
///
/// It scales the caller's budget rather than replacing it, so a caller who
/// asks for a smaller one still gets a smaller one.
pub const SCAN_FACTOR: usize = 11;

/// The budget for a set the candidate scan probes.
pub fn scan_budget(budget: usize) -> usize {
    budget.saturating_mul(SCAN_FACTOR)
}

/// A conservative summary of a set of glyphs, in three machine words.
///
/// Answers one question -- "could these two sets possibly overlap?" -- with
/// three ands and no memory traffic, and is allowed to say yes when the answer
/// is no. That is enough to throw away a whole lookup before touching the
/// buffer, which is what makes it worth having: a font's GPOS plan can be a
/// dozen mark-attachment lookups that cannot match a line of English, and
/// without this each one costs a scan of the buffer to discover that.
///
/// Three different shifts of the glyph id, each folded into 64 bits. One shift
/// alone would collide on any font whose glyphs run past 64; taking the low
/// bits, the bits above them, and a high slice makes an accidental overlap in
/// all three unlikely. This is HarfBuzz's set digest, and it earns its place
/// there for the same reason.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub struct Digest {
    words: [u64; 3],
}

/// Three views of a glyph id, each folded into six bits: bits 4 upward, the
/// low six, and bits 6 upward.
///
/// The same three the host's own digest uses, deliberately. A digest is only
/// useful against another digest, and the one this has to meet is the buffer's
/// -- maintained there as glyphs are substituted, which is not something this
/// module can do from outside. Same views, same bits, so the two intersect
/// word for word. `digests_agree` in `ot_layout` holds the two tables to it.
pub(crate) const DIGEST_SHIFTS: [u32; 3] = [4, 0, 6];

impl Digest {
    pub const EMPTY: Self = Self { words: [0; 3] };

    /// A digest that claims to contain everything, for the cases where nothing
    /// is known and rejecting would be wrong.
    pub const FULL: Self = Self {
        words: [u64::MAX; 3],
    };

    /// The raw words, for a host that has to meet this against a digest of its
    /// own. Meaningful only against one built over [`DIGEST_SHIFTS`].
    #[inline]
    pub fn words(&self) -> &[u64; 3] {
        &self.words
    }

    /// Whether this digest and one built over the same views can share a glyph.
    #[inline]
    pub fn may_intersect_raw(&self, other: &[u64; 3]) -> bool {
        self.words[0] & other[0] != 0
            && self.words[1] & other[1] != 0
            && self.words[2] & other[2] != 0
    }

    #[inline]
    pub fn insert(&mut self, gid: u32) {
        for (word, shift) in self.words.iter_mut().zip(DIGEST_SHIFTS) {
            *word |= 1u64 << ((gid >> shift) & 63);
        }
    }

    /// Add every glyph from `first` to `last` inclusive.
    ///
    /// Without visiting them: a run long enough to cover every bit of one view
    /// sets that view outright, and a shorter one sets the bits between its
    /// ends. Coverage format 2 states its glyphs as runs, so a summary built
    /// this way costs a handful of operations per run rather than one per
    /// glyph -- which is what makes it cheap enough to build for a lookup that
    /// is then not compiled at all.
    pub fn insert_range(&mut self, first: u32, last: u32) {
        for (word, shift) in self.words.iter_mut().zip(DIGEST_SHIFTS) {
            let (lo, hi) = (first >> shift, last >> shift);
            if hi.wrapping_sub(lo) >= 63 {
                *word = u64::MAX;
            } else {
                let (lo, hi) = (lo & 63, hi & 63);
                // A run that wraps within the view covers both ends of it.
                *word |= if lo <= hi {
                    let span = hi - lo;
                    if span == 63 {
                        u64::MAX
                    } else {
                        ((1u64 << (span + 1)) - 1) << lo
                    }
                } else {
                    !0u64 << lo | ((1u64 << (hi + 1)) - 1)
                };
            }
        }
    }

    pub fn from_glyphs(glyphs: impl IntoIterator<Item = u32>) -> Self {
        let mut d = Self::EMPTY;
        for g in glyphs {
            d.insert(g);
        }
        d
    }

    #[inline]
    pub fn union(&mut self, other: &Self) {
        for (a, b) in self.words.iter_mut().zip(other.words) {
            *a |= b;
        }
    }

    /// Whether the two sets might share a glyph. False is certain; true is not.
    #[inline]
    pub fn may_intersect(&self, other: &Self) -> bool {
        self.words[0] & other.words[0] != 0
            && self.words[1] & other.words[1] != 0
            && self.words[2] & other.words[2] != 0
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words == [0; 3]
    }
}

/// A set of glyphs, membership only.
///
/// Most coverage tables in a font are read purely as sets: chain context format
/// 3 tests backtrack, input and lookahead coverages and ignores the index
/// entirely, and so do the derived filters this shaper builds. Only a subtable
/// that indexes a parallel array — a substitute list, a value array — actually
/// needs [`Coverage`].
///
/// Dropping the index is not free to ignore: it is a third of a bitmap's cost
/// (a `u32` rank per 64-bit word) and a third of a range record's. So the two
/// are separate types, and asking for an index you never use is something the
/// type system stops rather than something you pay for silently.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum GlyphSet {
    #[default]
    Empty,
    Range {
        first: u32,
        len: u32,
    },
    /// No rank table: membership is one bit test.
    Bitmap {
        base: u32,
        words: Box<[u64]>,
    },
    /// Inclusive `(start, end)` runs, binary searched. Eight bytes per run
    /// against twelve for the indexed form.
    Ranges(Box<[(u32, u32)]>),
    Sorted(Box<[u32]>),
}

impl GlyphSet {
    /// Build from ascending, deduplicated glyph ids.
    pub fn build(glyphs: &[u32]) -> Self {
        Self::build_with_budget(glyphs, DEFAULT_BUDGET)
    }

    pub fn build_with_budget(glyphs: &[u32], budget: usize) -> Self {
        debug_assert!(
            glyphs.windows(2).all(|w| w[0] < w[1]),
            "must be ascending and deduped"
        );
        let (Some(&first), Some(&last)) = (glyphs.first(), glyphs.last()) else {
            return GlyphSet::Empty;
        };
        let span = (last - first + 1) as usize;
        if span == glyphs.len() {
            return GlyphSet::Range {
                first,
                len: glyphs.len() as u32,
            };
        }

        let n_words = span.div_ceil(64);
        let choice = pick(
            &[
                (Pick::Bitmap, 0, n_words * 8),
                (Pick::Ranges, 1, count_runs(glyphs) * 8),
                (Pick::Sorted, 1, glyphs.len() * 4),
            ],
            budget,
            SLACK,
        );

        match choice {
            Pick::Bitmap => {
                let mut words = vec![0u64; n_words];
                for &g in glyphs {
                    let o = (g - first) as usize;
                    words[o / 64] |= 1 << (o % 64);
                }
                GlyphSet::Bitmap {
                    base: first,
                    words: words.into_boxed_slice(),
                }
            }
            Pick::Ranges => {
                let mut runs: Vec<(u32, u32)> = Vec::new();
                for &g in glyphs {
                    match runs.last_mut() {
                        Some(r) if r.1 + 1 == g => r.1 = g,
                        _ => runs.push((g, g)),
                    }
                }
                GlyphSet::Ranges(runs.into_boxed_slice())
            }
            _ => GlyphSet::Sorted(glyphs.to_vec().into_boxed_slice()),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            GlyphSet::Empty => 0,
            GlyphSet::Range { len, .. } => *len as usize,
            GlyphSet::Bitmap { words, .. } => words.iter().map(|w| w.count_ones() as usize).sum(),
            GlyphSet::Ranges(r) => r.iter().map(|(a, b)| (b - a + 1) as usize).sum(),
            GlyphSet::Sorted(s) => s.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, GlyphSet::Empty)
    }

    #[inline]
    pub fn contains(&self, g: u32) -> bool {
        match self {
            GlyphSet::Empty => false,
            GlyphSet::Range { first, len } => g.wrapping_sub(*first) < *len,
            GlyphSet::Bitmap { base, words } => {
                let Some(o) = g.checked_sub(*base) else {
                    return false;
                };
                let o = o as usize;
                matches!(words.get(o / 64), Some(w) if (w >> (o % 64)) & 1 != 0)
            }
            GlyphSet::Ranges(r) => r
                .binary_search_by(|(a, b)| {
                    if g < *a {
                        core::cmp::Ordering::Greater
                    } else if g > *b {
                        core::cmp::Ordering::Less
                    } else {
                        core::cmp::Ordering::Equal
                    }
                })
                .is_ok(),
            GlyphSet::Sorted(s) => s.binary_search(&g).is_ok(),
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            GlyphSet::Empty | GlyphSet::Range { .. } => 0,
            GlyphSet::Bitmap { words, .. } => words.len() * 8,
            GlyphSet::Ranges(r) => r.len() * size_of::<(u32, u32)>(),
            GlyphSet::Sorted(s) => s.len() * 4,
        }
    }

    pub fn extend_into(&self, out: &mut Vec<u32>) {
        match self {
            GlyphSet::Empty => {}
            GlyphSet::Range { first, len } => out.extend(*first..*first + *len),
            GlyphSet::Bitmap { base, words } => {
                for (w, word) in words.iter().enumerate() {
                    let mut cur = *word;
                    while cur != 0 {
                        let b = cur.trailing_zeros() as usize;
                        cur &= cur - 1;
                        out.push(base + (w * 64 + b) as u32);
                    }
                }
            }
            GlyphSet::Ranges(r) => {
                for (a, b) in r {
                    out.extend(*a..=*b);
                }
            }
            GlyphSet::Sorted(s) => out.extend_from_slice(s),
        }
    }

    pub fn to_vec(&self) -> Vec<u32> {
        let mut v = Vec::with_capacity(self.len());
        self.extend_into(&mut v);
        v
    }
}

/// Compiled tables shared across every subtable of a font.
///
/// Fonts reference the same coverage and class tables over and over. Noto
/// Nastaliq Urdu makes 16,006 coverage references to 148 distinct tables;
/// NotoSans 524 to 186, Amiri 625 to 180. Compiling one per reference would
/// spend most of its memory on copies.
///
/// Interning is keyed by the table's own font bytes, which matters twice over:
/// a repeat reference is answered without decoding the table at all, and
/// `HashTable` probes with a hash and an equality closure over the borrowed
/// bytes, so a hit allocates nothing — a `HashMap` would need an owned key
/// constructed purely to ask the question.
///
/// Compiled subtables hold `Arc`s rather than indices into this. Shaping then
/// never touches the interner or its lock: a pointer dereference replaces a
/// bounds-checked lookup. And because a reader clones the `Arc` out rather than
/// borrowing through the lock, lookups can be compiled lazily behind `&self` and
/// shared across plans, which an index into a growing pool could not support.
///
/// The three kinds are separate tables because the same bytes compile to
/// different structures: a coverage read as a set carries no rank data, while
/// one read for its index does.
#[derive(Default)]
pub struct Interner {
    sets: Mutex<Table<GlyphSet>>,
    coverages: Mutex<Table<Coverage>>,
    classes: Mutex<Table<ClassMap>>,
}

struct Table<T> {
    entries: HashTable<Entry<T>>,
    hasher: DefaultHashBuilder,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            entries: HashTable::new(),
            hasher: DefaultHashBuilder::default(),
        }
    }
}

/// One interned table.
///
/// Keyed by where it sits -- which layout table, and how far into it -- and
/// bucketed by what it holds. The two answer different halves of the question
/// and the entry needs both.
///
/// Position is what makes the key *exact*. The bytes were the original key and
/// cost more than the thing they identified: on Nastaliq 24.7KiB against
/// 46.8KiB of compiled tables, so a third of what the interner held was there
/// only to recognise a table it had already seen. They were replaced by two
/// 64-bit hashes of those bytes, which was smaller and wrong -- two hashes are
/// not equality, and the failure is not a crash but the wrong compiled
/// coverage handed back for a table that merely hashed the same. A font is
/// untrusted input, and `foldhash` promises nothing to an adversary who can
/// see what it produced. The same offset in the same table is the same bytes
/// by construction, and costs one `u64`.
///
/// Content is what keeps the *sharing*. Position alone would file two tables
/// that hold the same bytes at different offsets as two tables, and that is
/// not a rare case: keying Nastaliq by position alone grows what the interner
/// holds from 46.8KiB to 70.3KiB. So the bucket is chosen by content, and an
/// entry whose position is new but whose value equals one already there is
/// given a second key onto the same `Arc`. Deciding that means building the
/// value to compare it, which happens once per distinct position rather than
/// once per reference -- a few hundred times against Nastaliq's sixteen
/// thousand.
struct Entry<T> {
    /// Hash of the bytes this was built from, kept so a resize can rehash
    /// without them. It is what buckets the entry, and what brings two
    /// spellings of the same table together.
    hash: u64,
    /// Which layout table, and how far into it. See `origin` in the parent
    /// module.
    key: u64,
    value: Arc<T>,
}

impl<T: PartialEq> Table<T> {
    fn intern(&mut self, key: Option<u64>, bytes: &[u8], build: impl FnOnce() -> T) -> Arc<T> {
        let hash = self.hasher.hash_one(bytes);
        let Some(key) = key else {
            // Nowhere to file it under, so it is built and not shared. Always
            // correct, and not reached by a font whose offsets are what they
            // say they are.
            return Arc::new(build());
        };
        // Seen this exact table before: same bytes, at the same place.
        if let Some(entry) = self.entries.find(hash, |e| e.key == key) {
            return Arc::clone(&entry.value);
        }
        // A table this has not seen, whose bytes hash like one it has. Build it
        // and ask whether they compile to the same thing, which is the question
        // that matters -- two coverages that describe the same glyphs are
        // interchangeable however they were spelled. A hash that collided
        // without matching simply fails here, which is the whole point.
        let built = build();
        let value = match self.entries.find(hash, |e| *e.value == built) {
            Some(entry) => Arc::clone(&entry.value),
            None => Arc::new(built),
        };
        self.entries.insert_unique(
            hash,
            Entry {
                hash,
                key,
                value: Arc::clone(&value),
            },
            // Rehashing on resize needs the bytes, which are gone by then;
            // keeping their hash on the entry is what makes it possible.
            |e| e.hash,
        );
        value
    }
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    /// A membership-only set, for coverages read as sets.
    pub fn set(
        &self,
        key: Option<u64>,
        bytes: &[u8],
        build: impl FnOnce() -> GlyphSet,
    ) -> Arc<GlyphSet> {
        self.sets.lock().intern(key, bytes, build)
    }

    /// An indexed coverage, for subtables that index a parallel array.
    pub fn coverage(
        &self,
        key: Option<u64>,
        bytes: &[u8],
        build: impl FnOnce() -> Coverage,
    ) -> Arc<Coverage> {
        self.coverages.lock().intern(key, bytes, build)
    }

    pub fn class_map(
        &self,
        key: Option<u64>,
        bytes: &[u8],
        build: impl FnOnce() -> ClassMap,
    ) -> Arc<ClassMap> {
        self.classes.lock().intern(key, bytes, build)
    }

    /// Distinct *values* held, against the number of positions that name
    /// them. The gap is what interning by content buys over interning by
    /// position alone.
    pub fn sharing(&self) -> (usize, usize) {
        fn count<T>(entries: &HashTable<Entry<T>>) -> (usize, usize) {
            let mut at: Vec<*const T> = entries.iter().map(|e| Arc::as_ptr(&e.value)).collect();
            let positions = at.len();
            at.sort_unstable();
            at.dedup();
            (positions, at.len())
        }
        let (ps, vs) = count(&self.sets.lock().entries);
        let (pc, vc) = count(&self.coverages.lock().entries);
        let (pk, vk) = count(&self.classes.lock().entries);
        (ps + pc + pk, vs + vc + vk)
    }

    /// Distinct tables interned, as `(sets, coverages, classes)`.
    pub fn len(&self) -> (usize, usize, usize) {
        (
            self.sets.lock().entries.len(),
            self.coverages.lock().entries.len(),
            self.classes.lock().entries.len(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.len() == (0, 0, 0)
    }

    /// Bytes held by the interning keys themselves.
    ///
    /// Separate from `heap_bytes` because it is not what the compiled form
    /// costs to *use* -- it is what the compiler keeps in order to recognise a
    /// table it has already seen. Sixteen bytes an entry.
    pub fn key_bytes(&self) -> usize {
        let (sets, coverages, classes) = self.len();
        (sets + coverages + classes) * 16
    }

    /// Bytes held by the compiled tables. The interning keys are compiler
    /// bookkeeping and are not counted.
    ///
    /// Counted per distinct table, not per entry. Two positions holding the
    /// same table share one `Arc`, which is the whole point of interning by
    /// content, and summing over entries would count what is shared as often
    /// as it is referenced -- reporting Nastaliq at 70.3KiB for 46.8KiB of
    /// tables.
    pub fn heap_bytes(&self) -> usize {
        /// Each interned table once, however many positions name it.
        fn distinct<T>(
            entries: &HashTable<Entry<T>>,
            inline: usize,
            size: impl Fn(&T) -> usize,
        ) -> usize {
            let mut tables: Vec<(*const T, usize)> = entries
                .iter()
                .map(|e| (Arc::as_ptr(&e.value), size(&e.value) + inline))
                .collect();
            tables.sort_unstable_by_key(|&(at, _)| at);
            tables.dedup_by_key(|&mut (at, _)| at);
            tables.iter().map(|&(_, bytes)| bytes).sum()
        }

        let sets = {
            let t = self.sets.lock();
            distinct(&t.entries, size_of::<GlyphSet>(), GlyphSet::heap_bytes)
        };
        let covs = {
            let t = self.coverages.lock();
            distinct(&t.entries, size_of::<Coverage>(), Coverage::heap_bytes)
        };
        let classes = {
            let t = self.classes.lock();
            distinct(&t.entries, size_of::<ClassMap>(), ClassMap::heap_bytes)
        };
        sets + covs + classes
    }
}

impl core::fmt::Debug for Interner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (s, c, k) = self.len();
        write!(f, "Interner({s} sets, {c} coverages, {k} classes)")
    }
}

/// One run of consecutive covered glyphs, mirroring OpenType coverage format 2.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CovRange {
    pub start: u32,
    /// Inclusive.
    pub end: u32,
    /// Coverage index of `start`.
    pub first_index: u32,
}

/// A set of glyphs that also assigns each member a dense index.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Coverage {
    #[default]
    Empty,
    /// Contiguous: index is `g - first`. Eight bytes and no memory traffic.
    Range { first: u32, len: u32 },
    /// Rank-select bitmap: `contains` is one bit test, `index` is a rank lookup
    /// plus a popcount. Chosen when the span is tight enough to afford.
    Bitmap {
        base: u32,
        words: Box<[u64]>,
        /// Set bits below each word, so index lookups stay O(1).
        rank: Box<[u32]>,
    },
    /// Sorted runs, binary searched. The compact fallback for wide sparse sets.
    Ranges(Box<[CovRange]>),
    /// Sorted glyph ids, binary searched. Used when members are so scattered
    /// that runs cost more than the ids themselves.
    Sorted(Box<[u32]>),
}

impl Coverage {
    /// Compile from ascending, deduplicated glyph ids.
    pub fn build(glyphs: &[u32]) -> Self {
        Self::build_with_budget(glyphs, DEFAULT_BUDGET)
    }

    pub fn build_with_budget(glyphs: &[u32], budget: usize) -> Self {
        debug_assert!(
            glyphs.windows(2).all(|w| w[0] < w[1]),
            "must be ascending and deduped"
        );
        let (Some(&first), Some(&last)) = (glyphs.first(), glyphs.last()) else {
            return Coverage::Empty;
        };
        let span = (last - first + 1) as usize;
        if span == glyphs.len() {
            return Coverage::Range {
                first,
                len: glyphs.len() as u32,
            };
        }

        // Size every candidate before building any of them, so a rejected
        // representation costs nothing. Then prefer the fastest form that fits
        // the budget; if none fits, take the smallest, since overshooting by
        // more than we have to is never right.
        let n_words = span.div_ceil(64);
        let bitmap_bytes = n_words * 12;
        let ranges_bytes = count_runs(glyphs) * size_of::<CovRange>();
        let sorted_bytes = glyphs.len() * 4;

        let choice = pick(
            &[
                (Pick::Bitmap, 0, bitmap_bytes),
                (Pick::Ranges, 1, ranges_bytes),
                (Pick::Sorted, 1, sorted_bytes),
            ],
            budget,
            SLACK,
        );

        match choice {
            Pick::Bitmap => {
                let mut words = vec![0u64; n_words];
                for &g in glyphs {
                    let o = (g - first) as usize;
                    words[o / 64] |= 1 << (o % 64);
                }
                let mut rank = Vec::with_capacity(n_words);
                let mut acc = 0u32;
                for w in &words {
                    rank.push(acc);
                    acc += w.count_ones();
                }
                Coverage::Bitmap {
                    base: first,
                    words: words.into_boxed_slice(),
                    rank: rank.into_boxed_slice(),
                }
            }
            Pick::Ranges => Coverage::Ranges(to_ranges(glyphs).into_boxed_slice()),
            Pick::Sorted => Coverage::Sorted(glyphs.to_vec().into_boxed_slice()),
            Pick::Dense8 | Pick::Dense16 => unreachable!("not coverage forms"),
        }
    }

    /// Number of covered glyphs.
    pub fn len(&self) -> usize {
        match self {
            Coverage::Empty => 0,
            Coverage::Range { len, .. } => *len as usize,
            Coverage::Bitmap { words, .. } => words.iter().map(|w| w.count_ones() as usize).sum(),
            Coverage::Ranges(r) => r.iter().map(|r| (r.end - r.start + 1) as usize).sum(),
            Coverage::Sorted(s) => s.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Coverage::Empty)
    }

    #[inline]
    pub fn contains(&self, g: u32) -> bool {
        match self {
            Coverage::Empty => false,
            Coverage::Range { first, len } => g >= *first && g - *first < *len,
            Coverage::Bitmap { base, words, .. } => {
                if g < *base {
                    return false;
                }
                let o = (g - *base) as usize;
                matches!(words.get(o / 64), Some(w) if (w >> (o % 64)) & 1 != 0)
            }
            Coverage::Ranges(r) => find_range(r, g).is_some(),
            Coverage::Sorted(s) => s.binary_search(&g).is_ok(),
        }
    }

    /// The coverage index of `g`, or `None` if not covered.
    #[inline]
    pub fn index(&self, g: u32) -> Option<u32> {
        match self {
            Coverage::Empty => None,
            Coverage::Range { first, len } => {
                let o = g.checked_sub(*first)?;
                (o < *len).then_some(o)
            }
            Coverage::Bitmap { base, words, rank } => {
                let o = g.checked_sub(*base)? as usize;
                let w = *words.get(o / 64)?;
                let bit = o % 64;
                if (w >> bit) & 1 == 0 {
                    return None;
                }
                // Rank of this bit: whole words below it, plus set bits below
                // it within its own word.
                let below = if bit == 0 {
                    0
                } else {
                    (w & ((1u64 << bit) - 1)).count_ones()
                };
                Some(rank[o / 64] + below)
            }
            Coverage::Ranges(r) => {
                let i = find_range(r, g)?;
                Some(r[i].first_index + (g - r[i].start))
            }
            Coverage::Sorted(s) => s.binary_search(&g).ok().map(|i| i as u32),
        }
    }

    /// Heap bytes held, for budget accounting.
    pub fn heap_bytes(&self) -> usize {
        match self {
            Coverage::Empty | Coverage::Range { .. } => 0,
            Coverage::Bitmap { words, rank, .. } => words.len() * 8 + rank.len() * 4,
            Coverage::Ranges(r) => r.len() * size_of::<CovRange>(),
            Coverage::Sorted(s) => s.len() * 4,
        }
    }

    /// Append every covered glyph, ascending, to `out`.
    ///
    /// Deliberately not an `Iterator`: the only callers collect into a buffer,
    /// and a trait object here would heap-allocate once per coverage and block
    /// inlining, on a path that runs for every subtable of every lookup at
    /// compile time. Time to first shape is dominated by exactly this kind of
    /// incidental allocation.
    pub fn extend_into(&self, out: &mut Vec<u32>) {
        match self {
            Coverage::Empty => {}
            Coverage::Range { first, len } => out.extend(*first..*first + *len),
            Coverage::Bitmap { base, words, .. } => {
                for (w, word) in words.iter().enumerate() {
                    let mut cur = *word;
                    while cur != 0 {
                        let b = cur.trailing_zeros() as usize;
                        cur &= cur - 1;
                        out.push(base + (w * 64 + b) as u32);
                    }
                }
            }
            Coverage::Ranges(r) => {
                for r in r {
                    out.extend(r.start..=r.end);
                }
            }
            Coverage::Sorted(s) => out.extend_from_slice(s),
        }
    }

    /// Rebuild as a membership-only set, dropping the index.
    pub fn to_set(&self) -> GlyphSet {
        GlyphSet::build(&self.to_vec())
    }

    /// Convenience for tests and tools; allocates.
    pub fn to_vec(&self) -> Vec<u32> {
        let mut v = Vec::with_capacity(self.len());
        self.extend_into(&mut v);
        v
    }
}

/// One run of glyphs sharing a class.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClassRange {
    pub start: u32,
    /// Inclusive.
    pub end: u32,
    pub class: u16,
}

/// Maps a glyph to its class, with class 0 as the implicit default.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ClassMap {
    /// Every glyph is class 0.
    #[default]
    Empty,
    /// Direct index, one byte per glyph. Class counts are small in practice
    /// (kern classes rarely reach a few dozen), so this is usually both the
    /// smallest and the fastest form.
    Dense8 { first: u32, classes: Box<[u8]> },
    /// Direct index: one load, no search.
    Dense { first: u32, classes: Box<[u16]> },
    /// Sorted runs, binary searched. Class definitions routinely span most of a
    /// font, so this is the common choice under any sane budget.
    Ranges(Box<[ClassRange]>),
}

impl ClassMap {
    /// Compile from ascending `(glyph, class)` pairs. Entries with class 0 are
    /// dropped, since class 0 is the default.
    pub fn build(entries: &[(u32, u16)]) -> Self {
        Self::build_with_budget(entries, DEFAULT_BUDGET)
    }

    pub fn build_with_budget(entries: &[(u32, u16)], budget: usize) -> Self {
        Self::build_with_slack(entries, budget, SLACK)
    }

    /// Build under an explicit slack, for tests that need to pin one form.
    pub fn build_with_slack(entries: &[(u32, u16)], budget: usize, slack: usize) -> Self {
        let nz: Vec<(u32, u16)> = entries.iter().copied().filter(|(_, c)| *c != 0).collect();
        let (Some(first), Some(last)) = (nz.first(), nz.last()) else {
            return ClassMap::Empty;
        };
        let (first, last) = (first.0, last.0);
        let span = (last - first + 1) as usize;
        let max_class = nz.iter().map(|(_, c)| *c).max().unwrap_or(0);

        let mut n_ranges = 0usize;
        let mut prev: Option<(u32, u16)> = None;
        for &(g, c) in &nz {
            if !matches!(prev, Some((pg, pc)) if pc == c && pg + 1 == g) {
                n_ranges += 1;
            }
            prev = Some((g, c));
        }

        let mut cands = vec![
            (Pick::Dense16, 0, span * 2),
            (Pick::Ranges, 1, n_ranges * size_of::<ClassRange>()),
        ];
        if u8::try_from(max_class).is_ok() {
            cands.push((Pick::Dense8, 0, span));
        }

        match pick(&cands, budget, slack) {
            Pick::Dense8 => {
                let mut classes = vec![0u8; span];
                for &(g, c) in &nz {
                    classes[(g - first) as usize] = c as u8;
                }
                ClassMap::Dense8 {
                    first,
                    classes: classes.into_boxed_slice(),
                }
            }
            Pick::Dense16 => {
                let mut classes = vec![0u16; span];
                for &(g, c) in &nz {
                    classes[(g - first) as usize] = c;
                }
                ClassMap::Dense {
                    first,
                    classes: classes.into_boxed_slice(),
                }
            }
            _ => {
                let mut ranges: Vec<ClassRange> = Vec::new();
                for &(g, c) in &nz {
                    match ranges.last_mut() {
                        Some(r) if r.class == c && r.end + 1 == g => r.end = g,
                        _ => ranges.push(ClassRange {
                            start: g,
                            end: g,
                            class: c,
                        }),
                    }
                }
                ClassMap::Ranges(ranges.into_boxed_slice())
            }
        }
    }

    #[inline]
    /// Every glyph the definition names, with its class.
    ///
    /// Glyphs it does not name are class zero, and there is no bound on how
    /// many of those there are -- so a caller reasoning about class zero has to
    /// treat it as holding everything.
    pub fn for_each(&self, mut f: impl FnMut(u32, u16)) {
        match self {
            ClassMap::Empty => {}
            ClassMap::Dense8 { first, classes } => {
                for (o, &c) in classes.iter().enumerate() {
                    f(first + o as u32, u16::from(c));
                }
            }
            ClassMap::Dense { first, classes } => {
                for (o, &c) in classes.iter().enumerate() {
                    f(first + o as u32, c);
                }
            }
            ClassMap::Ranges(r) => {
                for e in r {
                    for g in e.start..=e.end {
                        f(g, e.class);
                    }
                }
            }
        }
    }

    pub fn get(&self, g: u32) -> u16 {
        match self {
            ClassMap::Empty => 0,
            ClassMap::Dense8 { first, classes } => g
                .checked_sub(*first)
                .and_then(|o| classes.get(o as usize).copied())
                .unwrap_or(0) as u16,
            ClassMap::Dense { first, classes } => g
                .checked_sub(*first)
                .and_then(|o| classes.get(o as usize).copied())
                .unwrap_or(0),
            ClassMap::Ranges(r) => {
                match r.binary_search_by(|e| {
                    if g < e.start {
                        core::cmp::Ordering::Greater
                    } else if g > e.end {
                        core::cmp::Ordering::Less
                    } else {
                        core::cmp::Ordering::Equal
                    }
                }) {
                    Ok(i) => r[i].class,
                    Err(_) => 0,
                }
            }
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            ClassMap::Empty => 0,
            ClassMap::Dense8 { classes, .. } => classes.len(),
            ClassMap::Dense { classes, .. } => classes.len() * 2,
            ClassMap::Ranges(r) => r.len() * size_of::<ClassRange>(),
        }
    }
}

/// Which representation the size comparison settled on.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Pick {
    Bitmap,
    Ranges,
    Sorted,
    Dense8,
    Dense16,
}

/// Choose among `(form, speed_rank, bytes)` candidates. Lower `speed_rank` is
/// faster.
///
/// A flat byte budget is the wrong shape for this. Whether a direct table is
/// extravagant does not depend on how many bytes it is, but on how many bytes
/// the alternative would be: one byte per glyph of span is a bargain for a
/// class definition covering most of a font, and a scandal for one covering
/// three glyphs spread across it. So the rule is relative -- take the fastest
/// form costing no more than `SLACK` times the smallest -- with `budget` left
/// as a floor under which everything is affordable, since a fixed cost that
/// small is not worth reasoning about. A flat budget instead of a relative one
/// puts NotoSans's kern class definitions into binary search, which costs
/// about fifteen percent of shaping throughput on Latin text.
fn pick(cands: &[(Pick, u8, usize)], budget: usize, slack: usize) -> Pick {
    let smallest = cands.iter().map(|(_, _, b)| *b).min().unwrap_or(0);
    let ceiling = budget.max(smallest.saturating_mul(slack));
    cands
        .iter()
        .filter(|(_, _, bytes)| *bytes <= ceiling)
        .min_by_key(|(_, rank, bytes)| (*rank, *bytes))
        .or_else(|| cands.iter().min_by_key(|(_, _, bytes)| *bytes))
        .map(|(p, _, _)| *p)
        .expect("at least one candidate")
}

/// Number of runs of consecutive glyphs, without building them.
fn count_runs(glyphs: &[u32]) -> usize {
    let mut n = 0usize;
    let mut prev: Option<u32> = None;
    for &g in glyphs {
        if !matches!(prev, Some(p) if p + 1 == g) {
            n += 1;
        }
        prev = Some(g);
    }
    n
}

fn to_ranges(glyphs: &[u32]) -> Vec<CovRange> {
    let mut out: Vec<CovRange> = Vec::new();
    for (i, &g) in glyphs.iter().enumerate() {
        match out.last_mut() {
            Some(r) if r.end + 1 == g => r.end = g,
            _ => out.push(CovRange {
                start: g,
                end: g,
                first_index: i as u32,
            }),
        }
    }
    out
}

#[inline]
fn find_range(ranges: &[CovRange], g: u32) -> Option<usize> {
    ranges
        .binary_search_by(|r| {
            if g < r.start {
                core::cmp::Ordering::Greater
            } else if g > r.end {
                core::cmp::Ordering::Less
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .ok()
}

#[cfg(test)]
mod tests {
    /// The interner recognises a table it has seen and distinguishes ones it
    /// has not -- which is the whole of its contract, and is now decided by a
    /// digest rather than by comparing the bytes.
    #[test]
    fn interning_is_by_content() {
        let pool = Interner::new();
        let a = pool.set(Some(0), b"k0", || GlyphSet::build(&[1]));
        let again = pool.set(Some(0), b"k0", || GlyphSet::build(&[99]));
        // The second build closure must never have run.
        assert!(Arc::ptr_eq(&a, &again));
        assert_eq!(again.to_vec(), vec![1]);

        let b = pool.set(Some(1), b"k1", || GlyphSet::build(&[2]));
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(pool.len().0, 2);

        // Same bytes, different kind: separate tables, because a coverage read
        // as a set compiles to something else than one read for its index.
        let _ = pool.coverage(Some(0), b"k0", || Coverage::build(&[1]));
        assert_eq!(pool.len(), (2, 1, 0));
    }

    use super::*;

    /// Every representation must agree with the set it was built from, for both
    /// membership and index, across the whole probe range.
    fn check(glyphs: &[u32], budget: usize, expect: fn(&Coverage) -> bool) {
        let cov = Coverage::build_with_budget(glyphs, budget);
        assert!(expect(&cov), "unexpected representation: {cov:?}");
        assert_eq!(cov.len(), glyphs.len());
        assert_eq!(cov.to_vec(), glyphs);

        let hi = glyphs.last().copied().unwrap_or(0) + 3;
        for g in 0..hi {
            let want = glyphs.iter().position(|&x| x == g).map(|i| i as u32);
            assert_eq!(cov.index(g), want, "index({g}) in {cov:?}");
            assert_eq!(cov.contains(g), want.is_some(), "contains({g}) in {cov:?}");
        }
    }

    #[test]
    fn empty_set() {
        check(&[], DEFAULT_BUDGET, |c| matches!(c, Coverage::Empty));
    }

    #[test]
    fn contiguous_becomes_a_range() {
        check(&[10, 11, 12, 13], DEFAULT_BUDGET, |c| {
            matches!(c, Coverage::Range { .. })
        });
        let c = Coverage::build(&[10, 11, 12, 13]);
        assert_eq!(c.heap_bytes(), 0, "a range should hold no heap memory");
    }

    #[test]
    fn single_glyph_is_a_range() {
        check(&[42], DEFAULT_BUDGET, |c| {
            matches!(c, Coverage::Range { len: 1, .. })
        });
    }

    #[test]
    fn tight_span_becomes_a_bitmap() {
        let g: Vec<u32> = (100..300).filter(|x| x % 3 == 0).collect();
        check(&g, DEFAULT_BUDGET, |c| matches!(c, Coverage::Bitmap { .. }));
    }

    #[test]
    fn bitmap_index_is_a_rank_across_words() {
        // Spans several words so the rank table is actually exercised.
        let g: Vec<u32> = (0..250).filter(|x| x % 7 == 1).collect();
        check(&g, DEFAULT_BUDGET, |c| matches!(c, Coverage::Bitmap { .. }));
    }

    #[test]
    fn wide_sparse_set_falls_back_within_budget() {
        // 60 glyphs spread over 30k: a bitmap would be ~5.6 KB.
        let g: Vec<u32> = (0..60).map(|i| i * 500).collect();
        let cov = Coverage::build(&g);
        assert!(
            !matches!(cov, Coverage::Bitmap { .. }),
            "should not blow the budget on a bitmap"
        );
        assert!(
            cov.heap_bytes() <= DEFAULT_BUDGET,
            "fallback took {} bytes",
            cov.heap_bytes()
        );
        check(&g, DEFAULT_BUDGET, |c| {
            !matches!(c, Coverage::Bitmap { .. })
        });
    }

    #[test]
    fn clustered_sparse_set_prefers_ranges() {
        // Three tight clusters, far apart: few runs, so runs win.
        let mut g = Vec::new();
        for base in [0u32, 10_000, 20_000] {
            g.extend(base..base + 40);
        }
        let cov = Coverage::build(&g);
        assert!(matches!(cov, Coverage::Ranges(_)), "got {cov:?}");
        check(&g, DEFAULT_BUDGET, |c| matches!(c, Coverage::Ranges(_)));
    }

    #[test]
    fn a_set_too_sparse_to_index_is_not_indexed() {
        // The pathology the relative rule exists to catch: three glyphs spread
        // across a whole font. A bitmap over that span is kilobytes to describe
        // twelve bytes of information, and no amount of speed justifies it.
        let g = [0u32, 30_000, 60_000];
        let cov = Coverage::build(&g);
        assert!(!matches!(cov, Coverage::Bitmap { .. }), "got {cov:?}");
        assert!(
            cov.heap_bytes() < 512,
            "{} bytes for three glyphs",
            cov.heap_bytes()
        );
        check(&g, DEFAULT_BUDGET, |c| {
            !matches!(c, Coverage::Bitmap { .. })
        });

        // Scattered but dense enough to be worth indexing: a hundred glyphs
        // over ten thousand is 4.5x the bytes of a sorted list, which buys a
        // constant-time probe on a structure hit once per glyph per lookup.
        // Under the old flat budget nothing fitted and this fell back to the
        // smallest form, which was also the slowest.
        let g: Vec<u32> = (0..100).map(|i| i * 97).collect();
        let cov = Coverage::build(&g);
        assert!(matches!(cov, Coverage::Bitmap { .. }), "got {cov:?}");
    }

    #[test]
    fn slack_is_measured_against_the_smallest_form_not_a_fixed_size() {
        // Same shape at two scales: the choice should not change, because
        // whether a direct table is extravagant depends on the alternative and
        // not on how many bytes it happens to be.
        let small: Vec<u32> = (0..50).map(|i| i * 2).collect();
        let large: Vec<u32> = (0..5000).map(|i| i * 2).collect();
        let a = Coverage::build(&small);
        let b = Coverage::build(&large);
        assert!(matches!(a, Coverage::Bitmap { .. }), "got {a:?}");
        assert!(matches!(b, Coverage::Bitmap { .. }), "got {b:?}");
    }

    #[test]
    fn all_representations_agree() {
        // Force each representation over the same set by varying the budget,
        // then require identical answers from all of them.
        let g: Vec<u32> = (0..400).filter(|x| x % 5 == 2 || x % 11 == 0).collect();
        let reps: Vec<Coverage> = [0, 64, 4096]
            .iter()
            .map(|&b| Coverage::build_with_budget(&g, b))
            .collect();
        for probe in 0..420 {
            let want = reps[0].index(probe);
            for r in &reps[1..] {
                assert_eq!(r.index(probe), want, "disagreement at {probe}: {r:?}");
            }
        }
    }

    #[test]
    fn coverage_range_does_not_wrap_below_first() {
        // The Range fast path subtracts `first` with wrapping arithmetic, so a
        // glyph below it must not come back round into the range.
        let cov = Coverage::build(&[10, 11, 12]);
        assert!(matches!(cov, Coverage::Range { .. }));
        for g in [0, 9, 13, u32::MAX] {
            assert!(!cov.contains(g), "glyph {g}");
            assert_eq!(cov.index(g), None, "glyph {g}");
        }
        assert_eq!(cov.index(10), Some(0));
        assert_eq!(cov.index(12), Some(2));
    }

    #[test]
    fn glyph_set_drops_the_index_and_the_memory_for_it() {
        let g: Vec<u32> = (0..250).filter(|x| x % 7 == 1).collect();
        let cov = Coverage::build(&g);
        let set = GlyphSet::build(&g);
        assert!(matches!(cov, Coverage::Bitmap { .. }));
        assert!(matches!(set, GlyphSet::Bitmap { .. }));
        // A rank table is a u32 per 64-bit word: a third of the cost.
        assert_eq!(set.heap_bytes() * 3, cov.heap_bytes() * 2);
    }

    #[test]
    fn glyph_set_agrees_with_coverage_on_membership() {
        let g: Vec<u32> = (0..400).filter(|x| x % 5 == 2 || x % 11 == 0).collect();
        for budget in [0, 32, 64, 4096] {
            let cov = Coverage::build_with_budget(&g, budget);
            let set = GlyphSet::build_with_budget(&g, budget);
            assert_eq!(set.to_vec(), g, "budget {budget}");
            assert_eq!(set.len(), g.len());
            for probe in 0..420 {
                assert_eq!(
                    set.contains(probe),
                    cov.contains(probe),
                    "budget {budget}, glyph {probe}: {set:?} vs {cov:?}"
                );
            }
        }
    }

    #[test]
    fn glyph_set_range_does_not_wrap_below_first() {
        let set = GlyphSet::build(&[10, 11, 12]);
        assert!(matches!(set, GlyphSet::Range { .. }));
        assert!(!set.contains(0));
        assert!(!set.contains(9));
        assert!(set.contains(10));
        assert!(!set.contains(13));
    }

    #[test]
    fn empty_glyph_set() {
        let set = GlyphSet::build(&[]);
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert!(!set.contains(0));
        assert_eq!(set.heap_bytes(), 0);
    }

    #[test]
    fn interning_returns_the_same_table_for_the_same_bytes() {
        let pool = Interner::new();
        let mut built = 0;
        let a = pool.set(Some(0), b"k0", || {
            built += 1;
            GlyphSet::build(&[1, 2, 3])
        });
        let b = pool.set(Some(0), b"k0", || {
            built += 1;
            GlyphSet::build(&[1, 2, 3])
        });
        assert_eq!(built, 1, "a hit must not build");
        assert!(Arc::ptr_eq(&a, &b), "a hit must share, not copy");
        assert_eq!(pool.len().0, 1);
    }

    #[test]
    fn different_bytes_intern_separately() {
        let pool = Interner::new();
        let a = pool.set(Some(0), b"k0", || GlyphSet::build(&[1]));
        let b = pool.set(Some(1), b"k1", || GlyphSet::build(&[2]));
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(pool.len().0, 2);
    }

    #[test]
    fn the_same_bytes_compile_differently_for_different_uses() {
        // A coverage read as a set carries no rank table; one read for its index
        // does. They cannot share an entry, so they do not share a table.
        let pool = Interner::new();
        let glyphs: Vec<u32> = (0..200).filter(|x| x % 3 == 0).collect();
        let set = pool.set(Some(2), b"k2", || GlyphSet::build(&glyphs));
        let cov = pool.coverage(Some(2), b"k2", || Coverage::build(&glyphs));
        assert_eq!(pool.len(), (1, 1, 0));
        assert!(cov.heap_bytes() > set.heap_bytes(), "the index costs extra");
        assert_eq!(cov.index(0), Some(0));
    }

    #[test]
    fn class_maps_intern_too() {
        let pool = Interner::new();
        let entries: Vec<(u32, u16)> = (0..50).map(|g| (g, (g % 4) as u16)).collect();
        let a = pool.class_map(Some(3), b"k3", || ClassMap::build(&entries));
        let b = pool.class_map(Some(3), b"k3", || ClassMap::build(&entries));
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(pool.len(), (0, 0, 1));
    }

    #[test]
    fn interning_is_shared_across_threads() {
        // Taking &self is what lets a font cache fill this in lazily from
        // whichever plan reaches a lookup first.
        let pool = Arc::new(Interner::new());
        let sets: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let pool = Arc::clone(&pool);
                    scope.spawn(move || pool.set(Some(4), b"k4", || GlyphSet::build(&[4, 5, 6])))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(pool.len().0, 1, "every thread must reach the same table");
        for set in &sets {
            assert!(Arc::ptr_eq(set, &sets[0]));
        }
    }

    #[test]
    fn class_map_empty_when_all_zero() {
        assert_eq!(ClassMap::build(&[(1, 0), (2, 0)]), ClassMap::Empty);
        assert_eq!(ClassMap::build(&[]).get(7), 0);
    }

    #[test]
    fn small_classes_prefer_the_byte_table() {
        // Max class 4 fits a byte, so the 200-byte table must beat the 400-byte
        // one even though both are dense and both fit the budget.
        let entries: Vec<(u32, u16)> = (0..200).map(|g| (g, ((g / 7) % 5) as u16)).collect();
        let m = ClassMap::build_with_budget(&entries, 4096);
        assert!(matches!(m, ClassMap::Dense8 { .. }), "got {m:?}");

        // Same set, but one class pushed past a byte: the only difference is the
        // element width, so the u16 table must cost exactly twice as much.
        let mut wide_entries = entries.clone();
        wide_entries.last_mut().unwrap().1 = 900;
        let wide = ClassMap::build_with_budget(&wide_entries, 4096);
        assert!(matches!(wide, ClassMap::Dense { .. }), "got {wide:?}");
        assert_eq!(wide.heap_bytes(), m.heap_bytes() * 2);
    }

    #[test]
    fn wide_classes_need_the_u16_table() {
        // A class past 255 rules out the byte table.
        let mut entries: Vec<(u32, u16)> = (0..200).map(|g| (g, ((g / 7) % 5) as u16)).collect();
        entries[10].1 = 900;
        let m = ClassMap::build_with_budget(&entries, 4096);
        assert!(matches!(m, ClassMap::Dense { .. }), "got {m:?}");
        assert_eq!(m.get(10), 900);
    }

    #[test]
    fn every_class_map_form_agrees() {
        let mut entries: Vec<(u32, u16)> = (0..200).map(|g| (g, ((g / 7) % 5) as u16)).collect();
        entries[10].1 = 900;
        // The budget is a floor, so raising it cannot change a choice the slack
        // already allowed. To force the slow form, deny the slack instead.
        let forms = [
            ClassMap::build_with_budget(&entries, 4096),
            ClassMap::build_with_slack(&entries, 0, 1),
        ];
        assert!(
            matches!(forms[0], ClassMap::Dense { .. }),
            "got {:?}",
            forms[0]
        );
        assert!(
            matches!(forms[1], ClassMap::Ranges(_)),
            "got {:?}",
            forms[1]
        );
        for g in 0..220 {
            let want = entries.iter().find(|(x, _)| *x == g).map_or(0, |(_, c)| *c);
            for m in &forms {
                assert_eq!(m.get(g), want, "{m:?} at {g}");
            }
        }
    }

    #[test]
    fn picker_never_takes_a_bigger_form_than_it_has_to() {
        // The failure this guards: falling back past a representation that was
        // both smaller and faster just because it missed the budget.
        let entries: Vec<(u32, u16)> = (0..1200)
            .filter(|g| g % 3 != 0)
            .map(|g| (g, (g % 37) as u16))
            .collect();
        let m = ClassMap::build(&entries);
        let ranges = ClassMap::build_with_budget(&entries, 0);
        assert!(
            m.heap_bytes() <= ranges.heap_bytes(),
            "picked {} bytes when {} was available",
            m.heap_bytes(),
            ranges.heap_bytes()
        );
        for g in 0..1250 {
            let want = entries.iter().find(|(x, _)| *x == g).map_or(0, |(_, c)| *c);
            assert_eq!(m.get(g), want, "at {g}");
        }
    }

    #[test]
    fn class_map_ranges_coalesce() {
        let entries: Vec<(u32, u16)> = (0..100).map(|g| (g, 3)).collect();
        let ranges = ClassMap::build_with_budget(&entries, 0);
        match &ranges {
            ClassMap::Ranges(r) => assert_eq!(r.len(), 1, "one run expected, got {r:?}"),
            other => panic!("expected ranges, got {other:?}"),
        }
        assert_eq!(ranges.get(50), 3);
        assert_eq!(ranges.get(100), 0);
    }

    #[test]
    fn a_digest_never_denies_a_glyph_it_holds() {
        // The only property that matters for correctness: a false negative is a
        // silently dropped lookup, a false positive is a wasted scan.
        let mut d = Digest::EMPTY;
        let members: Vec<u32> = (0..500).map(|i| i * 37 % 9001).collect();
        for &g in &members {
            d.insert(g);
        }
        for &g in &members {
            let mut one = Digest::EMPTY;
            one.insert(g);
            assert!(
                d.may_intersect(&one),
                "digest denied glyph {g} it was given"
            );
        }
    }

    #[test]
    fn disjoint_sets_usually_read_as_disjoint() {
        // Not guaranteed -- it is allowed to say yes -- but it has to reject
        // most of the time or it is not worth having.
        let low = Digest::from_glyphs(0..40u32);
        let high = Digest::from_glyphs(3000..3040u32);
        assert!(!low.may_intersect(&high));

        let mut rejected = 0;
        for base in (0..8000u32).step_by(97) {
            let a = Digest::from_glyphs(base..base + 8);
            let b = Digest::from_glyphs(base + 4000..base + 4008);
            if !a.may_intersect(&b) {
                rejected += 1;
            }
        }
        assert!(
            rejected > 70,
            "only {rejected} of 83 disjoint pairs rejected"
        );
    }

    #[test]
    fn an_empty_digest_matches_nothing_and_a_full_one_matches_all() {
        let some = Digest::from_glyphs([1u32, 2, 3]);
        assert!(!Digest::EMPTY.may_intersect(&some));
        assert!(!some.may_intersect(&Digest::EMPTY));
        assert!(Digest::FULL.may_intersect(&some));
        assert!(Digest::EMPTY.is_empty());
        assert!(!some.is_empty());
    }

    #[test]
    fn a_union_holds_both_sides() {
        let mut a = Digest::from_glyphs([10u32, 20]);
        let b = Digest::from_glyphs([3000u32]);
        a.union(&b);
        assert!(a.may_intersect(&b));
        assert!(a.may_intersect(&Digest::from_glyphs([10u32])));
    }
}

#[cfg(test)]
mod digest_ranges {
    use super::Digest;

    /// A range summary must hold every glyph the range holds. It may hold
    /// more -- it is a summary -- but never less, or a lookup that could match
    /// would be thrown away.
    #[test]
    fn a_range_holds_everything_in_it() {
        for (first, last) in [
            (0u32, 0u32),
            (0, 1),
            (5, 5),
            (3, 70),
            (64, 64),
            (63, 65),
            (100, 4000),
            (0, 65535),
            (500, 501),
            (1, 62),
        ] {
            let mut range = Digest::EMPTY;
            range.insert_range(first, last);
            let mut one_at_a_time = Digest::EMPTY;
            for g in first..=last {
                one_at_a_time.insert(g);
            }
            // Every glyph in the range must still be admitted, which is
            // what a summary of it promises.
            for g in first..=last {
                let mut one = Digest::EMPTY;
                one.insert(g);
                assert!(range.may_intersect(&one), "range {first}..={last} lost {g}");
            }
            assert!(range.may_intersect(&one_at_a_time));
        }
    }
}
