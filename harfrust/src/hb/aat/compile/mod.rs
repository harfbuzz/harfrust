//! Where a compiled form of the AAT tables would go, and why there is not one.
//!
//! [`crate::hb::ot::compile`] pays off because a `GSUB` or `GPOS` lookup asks
//! the font the same question thousands of times and the answer never changes.
//! A `morx` machine looks like the same shape of problem -- a class lookup and
//! a two-array state table, read per glyph per subtable, and 38 to 51% of a
//! run -- and it is not. Four things were built and measured, and none earned
//! its place.
//!
//! **The state machine, decoded.** Both arrays copied out in the machine's
//! byte order with the entries already parsed, so a step is two indexed loads
//! instead of a bounds-checked big-endian read and a parse. It costs the size
//! of the table -- 88KiB for Lucida Grande, 152 for Devanagari MT -- and buys
//! 1.0%, 0.6% and 0.1% on the three fonts with the largest machines.
//!
//! Decoding only the entry table and leaving the state array in the font cuts
//! that to 6-14KiB and is *slower than not decoding at all* (1.010), so the
//! value was in the state array, which is the memory. There is no middle:
//! capping by size denies the decode to Devanagari MT and Menlo, whose arrays
//! are the largest and which are exactly the fonts that regress without it.
//!
//! **The glyph-to-class lookup, as a direct table.** The applying side keeps a
//! 256-entry cache in front of the font's search, and that cache is a good fit:
//! a line uses few distinct glyphs, so the search behind it runs about once per
//! glyph per *face*. A byte-per-glyph table measured level -- it wins where the
//! cache thrashes, on faces of three thousand glyphs, and loses where it does
//! not.
//!
//! **A summary of which subtables a buffer can skip.** The exact glyph set the
//! applying side intersects with the buffer is 8 to 20% of a run, so
//! summarising it looks obvious, and on one font it is: Lucida Grande, whose
//! `morx` is 396KiB of subtables that a line of English mostly misses, gains
//! 3 to 5%. Every other pairing loses, and the mean is a loss however the
//! summary is kept current:
//!
//! * exact, updated per written glyph: 1.017, and 11% worse on Geeza Pro,
//!   because three `or`s inside the substitution loop grew it past what the
//!   compiler would inline it into;
//! * retaken after any subtable that ran: 1.039, a pass over the buffer per
//!   subtable;
//! * retaken only after one that wrote: 1.030;
//! * unioned with what a machine could have written, three `or`s per subtable
//!   and no pass at all: 1.012.
//!
//! The last is the right design and still does not pay. Disabling only the
//! test, keeping everything that feeds it, measures 1.025 -- so the whole loss
//! is in maintaining the summary, chiefly taking it once per apply, and the
//! test itself is worth about 1.3%. What decides whether that is enough is how
//! often a subtable is skipped, which is a property of the text rather than
//! the font, and so is not something the compiler can decide.
//!
//! **The kerning pair sets, flattened.** Simple kerning probes two sparse
//! integer sets per adjacent pair, and a per-element probe is what a flat
//! bitmap is for -- but these sets are small enough that finding a page costs
//! nothing, and flattening them measured 1.011, 0.996 and 0.999 on the three
//! font and text pairings that have a `kern` table to use.
//!
//! # Why this does not repay what `GSUB` does
//!
//! A layout lookup stores what it knows in shapes that have to be *searched*:
//! binary-searched coverage tables, ranged class definitions, chains of
//! offsets. Compiling replaces a search with an index, and the same search
//! runs for every glyph of every buffer the lookup is entered for.
//!
//! An AAT state table is already an index. The font stores the state array as
//! a dense row-major grid and the class lookup behind a cache, so there is far
//! less between the font and the answer to begin with, and what compiling
//! removes is a byte-swap and a bounds check rather than a search.
