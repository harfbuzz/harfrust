//! Turning AAT tables into something worth running twice.
//!
//! The same bargain as [`crate::hb::ot::compile`], one table family over. A
//! `morx` or `kerx` subtable is a finite state machine stored as three arrays
//! in the font: a lookup from glyph to class, a state array indexed by state
//! and class, and a table of the entries it names. Driving it from the font
//! means, per glyph and per subtable, a search for the class and a chain of
//! bounds-checked big-endian reads for the entry. None of that depends on the
//! text, so none of it needs doing more than once.
//!
//! # Isolation
//!
//! This module describes fonts. It knows nothing about a buffer, a cursor, or
//! the shaping context; the applying side stays where it is, in
//! `layout_morx_table` and `layout_kerx_table`, which read what is built here
//! and drive it. What it does share is the vocabulary in
//! [`crate::hb::ot::compile::set`]: a class definition is a class definition
//! whether a `GPOS` kerning subtable or a `morx` ligature machine names it,
//! and the representation picker there has already been measured against real
//! fonts.
//!
//! # What is deliberately not here
//!
//! Two things that look like obvious candidates and were measured not to be.
//! Both are recorded at length because the reasoning that suggests them is
//! sound, and someone will have it again.
//!
//! ## Which subtables a buffer can skip
//!
//! Every subtable carries a set of the glyphs its machine can start on, and
//! the runtime intersects that with the buffer -- eight to twenty percent of a
//! run across the AAT benchmark fonts. Four ways of compiling it, all slower:
//!
//! * a three-word digest of the set, kept current as glyphs are written:
//!   1.017, because three `or`s per written glyph grew the substitution loop
//!   past what the compiler would inline it into, costing Geeza Pro 11%;
//! * the same digest, rebuilt after any subtable that ran instead: 1.039,
//!   trading that for a pass over the buffer per subtable;
//! * walking the buffer and probing the exact set per glyph: 1.110, because
//!   reaching a bit in a sparse integer set means finding the page it is in;
//! * walking the buffer and probing a flattened bitmap of the same set: 1.032,
//!   which wins on Zapfino and Menlo and loses badly on Devanagari MT, whose
//!   many subtables mostly answer *no* -- and a walk is a poor way to answer
//!   no, where intersecting two sparse bitmaps is a good one, since disjoint
//!   pages settle it without comparing bits.
//!
//! The set intersection is already the right shape for the question. What
//! would beat it is an O(1) rejection costing nothing to maintain, and the
//! only sound form of that needs each machine's *output* glyphs compiled too,
//! so a buffer summary can stay a superset without being rebuilt.
//!
//! ## The glyph-to-class lookup
//!
//! `Lookup::value` is 5% of a run of Devanagari MT and 13% of Geeza Pro, and
//! the lookup is four searched formats behind a 256-entry direct-mapped cache.
//! Replacing it with a compiled table looks free. It is not:
//!
//! * as a general [`crate::hb::ot::compile::set::ClassMap`]: 1.045, and 1.036
//!   once given a budget that keeps it out of searched ranges. Reading it is a
//!   match over four shapes, which becomes a jump table in a loop that runs
//!   once per glyph per subtable;
//! * with that match resolved to a function pointer per subtable: 1.028. The
//!   call would not inline, and on Zapfino the two together came to 16% of the
//!   run against 7% for the font's own lookup with the cache in front;
//! * as a plain byte table, no enum and no pointer: 1.000, dead level;
//! * the same, built only for faces above 256 glyphs, where the cache can
//!   collide at all: 0.999.
//!
//! The last is the interesting one. It wins where the cache thrashes -- Menlo
//! at 3157 glyphs, 0.976; Lucida Grande at 2826, 0.974 -- and loses where it
//! does not, and the two cancel. A 256-entry cache is a good fit for this
//! question: a line of text uses few distinct glyphs, so the search behind it
//! runs about once per glyph per face rather than once per glyph per buffer.
//! Beating it needs a structure that is both O(1) *and* smaller than the cache
//! it replaces, and a byte per glyph of span is not that.
//!
//! Where the time actually is: the state machine step itself, which is
//! [`machine::States`].

pub mod machine;

#[cfg(all(test, feature = "std"))]
mod heap_cost {
    use crate::{FontRef, ShaperData};

    /// What decoding the state machines costs, against the table they came
    /// from.
    ///
    /// The state array is copied wholesale -- the same entries, in the
    /// machine's byte order rather than the font's -- so this lands near the
    /// size of the `morx` table itself, and a font with a large one pays for
    /// it twice. That is the trade the timing pays for, and it is the number
    /// to look at before widening this to `kerx`.
    #[allow(clippy::cast_precision_loss)]
    #[test]
    fn report() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/fonts");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut fonts: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e == "ttf" || e == "ttc" || e == "otf")
            })
            .collect();
        fonts.sort();
        println!();
        println!("{:<34} {:>10} {:>12}", "font", "morx KiB", "decoded KiB");
        for path in fonts {
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let Ok(font) = FontRef::from_index(&data, 0) else {
                continue;
            };
            let Some(morx) = font
                .table_data(read_fonts::types::Tag::new(b"morx"))
                .map(|d| d.len())
            else {
                continue;
            };
            let shaper_data = ShaperData::new(&font);
            let shaper = shaper_data.shaper(&font).build();
            let decoded = shaper.aat_tables.morph_heap_bytes();
            println!(
                "{:<34} {:>10.1} {:>12.1}",
                path.file_name().unwrap().to_string_lossy(),
                morx as f64 / 1024.0,
                decoded as f64 / 1024.0,
            );
        }
        println!();
    }
}
