//! Turning AAT tables into something worth running twice.
//!
//! The same bargain as [`crate::hb::ot::compile`], one table family over. A
//! `morx` or `kerx` subtable is a finite state machine stored as three arrays
//! in the font: a lookup from glyph to class, a state array indexed by state
//! and class, and a table of the entries it names. Driving it from the font
//! means, per glyph and per subtable, a search for the class and a chain of
//! bounds-checked big-endian reads for the entry. None of that depends on the
//! text.
//!
//! [`machine`] holds what is decoded. This module describes fonts and knows
//! nothing about a buffer or a cursor; the applying side stays in
//! `layout_morx_table`, which reads what is built here and drives it. The
//! vocabulary in [`crate::hb::ot::compile::set`] is shared where it fits: a
//! class definition is a class definition whether a `GPOS` kerning subtable or
//! a `morx` ligature machine names it.
//!
//! # Scope
//!
//! Two things a reader will expect to find here and will not.
//!
//! The glyph-to-class lookup is left to the font, behind the 256-entry cache
//! the applying side already keeps. A direct table is O(1) where the font's is
//! searched, but the cache is 512 bytes and a line of text uses few distinct
//! glyphs, so the search behind it runs about once per glyph per *face*.
//! Compiling it measured level on the AAT benchmark set -- it wins where the
//! cache thrashes, on faces of three thousand glyphs, and loses where it does
//! not.
//!
//! Which subtables a buffer can skip is left to the exact glyph set the
//! applying side intersects with the buffer. That intersection is eight to
//! twenty percent of a run, so it looks like the obvious thing to summarise,
//! and a summary is a poor fit: most of those subtables answer *no*, and
//! intersecting two sparse bitmaps answers no cheaply, since disjoint pages
//! settle it without comparing bits. What would beat it is an O(1) rejection
//! costing nothing to maintain, which needs each machine's *output* glyphs
//! compiled as well, so a buffer summary can stay a superset without being
//! rebuilt.

pub mod machine;

#[cfg(all(test, feature = "std"))]
mod heap_cost {
    use crate::{FontRef, ShaperData};
    use read_fonts::TableProvider;

    /// What decoding the state machines costs, against the table they came
    /// from.
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
            #[allow(clippy::cast_precision_loss)]
            let kib = |n: usize| n as f64 / 1024.0;
            println!(
                "{:<34} {:>10.1} {:>12.1}",
                path.file_name().unwrap().to_string_lossy(),
                kib(morx),
                kib(shaper.aat_tables.morph_heap_bytes()),
            );
        }
        println!();
    }
}
