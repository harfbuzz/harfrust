//! Driving the compiled lookups over a buffer.
//!
//! Two levels, and both are format-agnostic: what makes this file short is
//! that every decision about *which* format runs was already made when the
//! lookup was compiled.
//!
//! * [`apply_at`] tries the subtables that can start on the glyph under the
//!   cursor, in the order they must be tried.
//! * [`apply_forward`] walks the buffer.
//!
//! `apply_forward` deliberately has the shape of this crate's own
//! `ot_layout::apply_forward` -- scan for a candidate, try to apply, advance if
//! it did not. That is so the two can be compared position for position while
//! the formats are filled in. It is not where the compiled form pays off: the
//! shaper this came from replaces the scan with a bitmap intersection built
//! from the lookup's compiled reach, its pair key and the buffer's transposed
//! feature masks, which is a later piece of work and a bigger one.

use super::lookup::{Apply, CompiledLookup};
use crate::hb::ot_layout_gsubgpos::OT::check_glyph_property;

/// Try this lookup at the cursor, reporting whether a subtable applied.
///
/// Where HarfBuzz walks every subtable and asks each to re-read its coverage
/// out of the font, this asks a compiled set -- and when the lookup has enough
/// subtables to be worth an index, only the ones that can start on this glyph
/// are considered at all.
///
/// The gate is asked for exactly what the subtable will use: ranking a bit
/// costs a load and a popcount over the test that answers membership, and
/// several formats never read the number. The result is handed to the format
/// so it does not recompute it.
pub fn apply_at(ctx: &mut Apply, lookup: &CompiledLookup) -> Option<()> {
    let glyph = ctx.glyph();
    // With an index, only the subtables that can start on this glyph, in the
    // order they must be. Without one, all of them.
    let row = lookup.dispatch.as_ref().map(|d| d.row(glyph));
    let count = row.map_or(lookup.subtables.len(), <[u16]>::len);
    for k in 0..count {
        let at = match row {
            Some(r) => r[k] as usize,
            None => k,
        };
        let Some(sub) = lookup.subtables.get(at) else {
            continue;
        };
        let Some(index) = sub.gate(glyph) else {
            continue;
        };
        // An indirect call, not a match. The format was settled when the
        // subtable was compiled.
        if (sub.apply)(ctx, lookup, sub, index).is_some() {
            return Some(());
        }
    }
    None
}

/// One forward pass over the buffer.
///
/// Reports whether anything applied. The caller owns the output buffer
/// discipline -- `clear_output` before, `sync` after, for a table that is not
/// applied in place.
pub fn apply_forward(ctx: &mut Apply, lookup: &CompiledLookup) -> bool {
    let mut applied = false;
    while ctx.host.buffer.successful {
        // Scan to the next position this lookup could touch. Three tests, and
        // the first is the compiled reach rather than a parse of the font.
        let idx = ctx.host.buffer.idx;
        let mut j = idx;
        while j < ctx.host.buffer.len {
            let info = &ctx.host.buffer.info[j];
            if lookup.reach.contains(info.glyph_id)
                && (info.mask & ctx.host.lookup_mask()) != 0
                && check_glyph_property(ctx.host.face, info, ctx.host.lookup_props)
            {
                break;
            }
            j += 1;
        }
        if j > idx {
            ctx.host.buffer.next_glyphs(j - idx);
        }
        if ctx.host.buffer.idx >= ctx.host.buffer.len {
            break;
        }

        // A format that applied has already advanced the cursor past what it
        // consumed; one that did not leaves it to us.
        if apply_at(ctx, lookup).is_some() {
            applied = true;
        } else {
            ctx.host.buffer.next_glyph();
        }
    }
    applied
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::hb::buffer::Buffer;
    use crate::hb::face::Scale;
    use crate::hb::ot::compile::compile_gsub_program;
    use crate::hb::ot::compile::lookup::{Program, SubtableKind};
    use crate::hb::ot_layout::{apply_synthesized_subst_lookup, TableIndex};
    use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
    use crate::{FontRef, ShaperData};
    use read_fonts::TableProvider;

    /// Longest buffer a case builds, so a lookup with a huge coverage does not
    /// turn one case into a benchmark.
    const MAX_GLYPHS: usize = 48;

    /// Whether every subtable of this lookup is one of the formats implemented
    /// so far. A lookup with a stub in it would "differ" for a reason that
    /// says nothing about dispatch.
    fn all_implemented(program: &Program, index: u16, table: &[u8]) -> bool {
        let Some(lookup) = program.get(index, table) else {
            return false;
        };
        !lookup.subtables.is_empty()
            && lookup.subtables.iter().all(|s| {
                matches!(
                    s.kind,
                    SubtableKind::SingleDelta { .. }
                        | SubtableKind::SingleList { .. }
                        | SubtableKind::Alternate { .. }
                )
            })
    }

    /// The glyphs a lookup can act on, plus one it cannot, so a pass that
    /// substitutes indiscriminately is caught as well as one that substitutes
    /// nothing.
    fn probe_glyphs(program: &Program, index: u16, table: &[u8]) -> Vec<u32> {
        let lookup = program.get(index, table).unwrap();
        let mut glyphs = lookup.reach.to_vec();
        glyphs.truncate(MAX_GLYPHS - 1);
        glyphs.push(0);
        glyphs
    }

    /// Apply one GSUB lookup, by whichever path, and return the glyphs left.
    fn run(
        data: &[u8],
        index: u16,
        glyphs: &[u32],
        mine: Option<&Program>,
    ) -> Option<(Vec<u32>, Vec<u32>, Vec<u32>)> {
        let font = FontRef::new(data).ok()?;
        let shaper_data = ShaperData::new(&font);
        let face = shaper_data.shaper(&font).build();
        let (table, info) = face
            .ot_tables
            .table_data_and_lookup(TableIndex::GSUB, index)?;
        if !info.is_subst || info.is_reverse() {
            return None;
        }

        let mut buffer = Buffer::new();
        for (i, &g) in glyphs.iter().enumerate() {
            buffer.push(g, i as u32);
        }
        buffer.reset_masks(1);
        // The vars GSUB writes through: ligature ids and glyph props. Shaping
        // allocates these around the substitution stage, and setting glyph
        // props below asserts they are there.
        buffer.allocate_gsubgpos_vars();
        crate::hb::ot_layout::hb_ot_layout_substitute_start(&face, &mut buffer);

        let mut ctx =
            hb_ot_apply_context_t::new(TableIndex::GSUB, &face, Scale::default(), &mut buffer);
        ctx.lookup_index = index;
        ctx.set_lookup_mask(1);

        match mine {
            // The compiled path, driven the way this crate drives its own.
            Some(program) => {
                let compiled = program.get(index, table)?;
                ctx.lookup_props = info.props();
                ctx.update_matchers();
                ctx.buffer.clear_output();
                ctx.buffer.idx = 0;
                let mut apply = Apply {
                    host: &mut ctx,
                    table,
                    program,
                };
                apply_forward(&mut apply, compiled);
                ctx.buffer.sync();
            }
            None => apply_synthesized_subst_lookup(&mut ctx, info, table),
        }

        let n = buffer.len;
        Some((
            buffer.info[..n].iter().map(|i| i.glyph_id).collect(),
            buffer.info[..n].iter().map(|i| i.cluster).collect(),
            buffer.info[..n].iter().map(|i| i.mask).collect(),
        ))
    }

    /// Every single-substitution lookup in the test corpus, both ways.
    ///
    /// The compiled path has to agree with this crate's own on the glyphs it
    /// leaves *and* on the clusters and flags -- the second half is the part
    /// that is invisible when it is wrong, and it is why this compares masks
    /// rather than just glyph ids.
    #[test]
    fn single_substitution_agrees_with_the_shaper_it_was_lifted_into() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");
        let mut checked = 0usize;
        let mut fonts = 0usize;
        let mut failures = Vec::new();

        for entry in walk(dir) {
            let Ok(data) = std::fs::read(&entry) else {
                continue;
            };
            let Ok(font) = FontRef::new(&data) else {
                continue;
            };
            let Ok(gsub) = font.gsub() else { continue };
            let Some(table) = font
                .table_data(read_fonts::types::Tag::new(b"GSUB"))
                .map(|d| d.as_bytes().to_vec())
            else {
                continue;
            };
            let program = compile_gsub_program(&gsub);
            let mut touched = false;

            for index in 0..program.len() as u16 {
                if !all_implemented(&program, index, &table) {
                    continue;
                }
                let glyphs = probe_glyphs(&program, index, &table);
                if glyphs.len() < 2 {
                    continue;
                }
                let Some(want) = run(&data, index, &glyphs, None) else {
                    continue;
                };
                let Some(got) = run(&data, index, &glyphs, Some(&program)) else {
                    continue;
                };
                checked += 1;
                touched = true;
                if got != want {
                    failures.push(format!(
                        "{}: lookup {index}\n    want {:?}\n    got  {:?}",
                        entry.file_name().unwrap().to_string_lossy(),
                        want,
                        got
                    ));
                }
            }
            if touched {
                fonts += 1;
            }
        }

        assert!(checked > 0, "no single-substitution lookups exercised");
        assert!(
            failures.is_empty(),
            "{} of {checked} lookups differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
        println!("{checked} single-substitution lookups across {fonts} fonts agree");
    }

    fn walk(dir: &str) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .is_some_and(|x| x == "ttf" || x == "otf" || x == "ttc")
                {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }
}
