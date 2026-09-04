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
//! summarising it looks obvious. Four ways were tried, from 1.017 to 1.110.
//! Most of those subtables answer *no*, and intersecting two sparse bitmaps
//! answers no cheaply, since disjoint pages settle it without comparing bits.
//! What would beat it is an O(1) rejection costing nothing to maintain, which
//! needs each machine's *output* glyphs compiled too, so a buffer summary can
//! stay a superset without being rebuilt.
//!
//! The module is kept for the one place a compiled form still looks worth
//! having, which is not `morx` at all: `kern` and `kerx` probe two sparse
//! integer sets per adjacent pair, and a per-element probe is what a flat
//! bitmap is for.
