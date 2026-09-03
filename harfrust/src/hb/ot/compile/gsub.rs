//! Applying the substitution formats.
//!
//! One function per format, reached through the pointer a [`Subtable`] carries
//! rather than by matching on its kind. Each opens with the single branch that
//! extracts its own payload -- a check that cannot fail, since the pointer was
//! resolved from that very variant -- and is then sized for its own work
//! rather than for the widest arm of a shared dispatch.
//!
//! # Cluster and safety flags
//!
//! Every format that consumes more than one position owes the buffer a note
//! about it, and getting that wrong is invisible in the glyphs: it shows up
//! only as a caller breaking a line where it may not, or reusing a shaped run
//! it should have reshaped. So each routine states what it owes.
//!
//! A 1->1 substitution owes nothing -- one position in, the same position out,
//! and `replace_glyph` keeps the cluster. The formats that owe something are
//! ligature (merge the components' clusters), the contexts (see
//! [`super::contextual`]) and reverse chaining.

use super::be16;
use super::lookup::{Apply, CompiledLookup, Subtable, SubtableKind};
use crate::hb::buffer::{GlyphInfo, GlyphPropsFlags};
use crate::hb::ot_layout::MAX_NESTING_LEVEL;
use crate::hb::ot_layout_gsubgpos::{
    ligate_input, match_always, match_backtrack, match_glyph, match_input, match_lookahead,
    may_skip_t, skipping_iterator_t,
};
use crate::hb::ot_map::hb_ot_map_t;
use read_fonts::tables::gsub::{Ligature, LigatureSet};
use read_fonts::types::GlyphId;
use read_fonts::{FontData, FontRead};

/// Single substitution format 1: the substitute is `glyph + delta`, wrapped
/// within the 16-bit glyph space as the format requires.
///
/// Nothing to borrow, so the whole payload is the delta and applying touches
/// no font bytes at all. Membership-gated: the coverage index is not read.
///
/// Flags: nothing owed, 1->1.
pub fn at_single_delta(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::SingleDelta { delta } = &sub.kind else {
        return None;
    };
    let subst = (ctx.glyph() as i32 + delta) as u32 & 0xFFFF;
    ctx.host.replace_glyph(GlyphId::from(subst));
    Some(())
}

/// Single substitution format 2: one indexed big-endian read at the coverage
/// index, straight out of the font.
///
/// The array stayed where it was; what the compiler kept is its offset and its
/// length. This is the whole of "compile the index, borrow the payload" in one
/// function -- the coverage that decided we are here is compiled and exact,
/// and the answer it indexes is still in the font.
///
/// Flags: nothing owed, 1->1.
pub fn at_single_list(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::SingleList { subst } = &sub.kind else {
        return None;
    };
    let glyph = subst.get(ctx.table, index)?;
    ctx.host.replace_glyph(GlyphId::from(u32::from(glyph)));
    Some(())
}

/// Multiple substitution: one glyph becomes a sequence, so this is the only
/// format that can lengthen the buffer -- see [`super::lookup::LengthEffect`].
///
/// Flags: nothing owed beyond what the splice already does. Every piece
/// carries the input's cluster, so a caller cannot break between them, and
/// `output_glyph_for_component` is what arranges that.
pub fn at_multiple(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::Multiple { offset, .. } = &sub.kind else {
        return None;
    };
    // The sequences stay in the font: one per covered glyph, of which a run
    // reads at most one. Two indexed reads to reach ours, then its length.
    let base = *offset as usize;
    if index >= u32::from(be16(ctx.table, base + 4)?) {
        return None;
    }
    let seq = base + usize::from(be16(ctx.table, base + 6 + index as usize * 2)?);
    let count = usize::from(be16(ctx.table, seq)?);
    let subst = |i: usize| be16(ctx.table, seq + 2 + i * 2);

    match count {
        // The spec disallows an empty sequence, but Uniscribe accepts one and
        // fonts ship it, so it deletes.
        0 => ctx.host.buffer.delete_glyph(),
        // One glyph out is an ordinary substitution. In place, and deliberately
        // not recorded as a multiplication -- what follows must not treat the
        // result as a component of anything.
        1 => ctx.host.replace_glyph(GlyphId::from(u32::from(subst(0)?))),
        _ => {
            // A ligature being split yields base glyphs; anything else yields
            // pieces whose class GDEF will decide.
            let class = if ctx.host.buffer.cur(0).is_ligature() {
                GlyphPropsFlags::BASE_GLYPH
            } else {
                GlyphPropsFlags::empty()
            };
            // Whether these pieces may be numbered as components at all. If
            // this glyph is itself attached to a ligature, its component id
            // belongs to that ligature and renumbering would detach the marks
            // that point at it.
            let attached = ctx.host.buffer.cur(0).lig_id() != 0;

            for i in 0..count {
                let glyph = GlyphId::from(u32::from(subst(i)?));
                if !attached {
                    // Truncated to four bits downstream, so the cast is safe
                    // for any sequence a font can express.
                    ctx.host
                        .buffer
                        .cur_mut(0)
                        .set_lig_props_for_component(i as u8);
                }
                ctx.host.output_glyph_for_component(glyph, class);
            }
            // The input is consumed without being written: the pieces above
            // replaced it. Every piece carries the input's cluster, which is
            // what keeps a caller from breaking between them.
            ctx.host.buffer.skip_glyph();
        }
    }
    Some(())
}

/// Ligature substitution: walk the ligature set for the coverage index in font
/// order and apply the first whose components all match.
///
/// The sets stay in the font. What was compiled is the coverage and the pair
/// key -- the set of glyphs that can appear as a *second* component -- which
/// the font does not contain and which is the discriminating half of the
/// filter: `ccmp` covers 72% of English text by first glyph and ligates none
/// of it, because the second component is always a combining mark.
///
/// Flags: **owes `merge_clusters` over the whole matched run.** The components
/// become one glyph, so a caller may not break between the characters they
/// came from.
pub fn at_ligature(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::Ligature { offset } = &sub.kind else {
        return None;
    };
    // Two indexed reads to reach our set, then read-fonts for the walk: a
    // LigatureSet is a count and a list of offsets, so parsing one is a slice
    // and a bounds check, and its offsets are relative to its own start.
    let base = *offset as usize;
    if index >= u32::from(be16(ctx.table, base + 4)?) {
        return None;
    }
    let at = base + usize::from(be16(ctx.table, base + 6 + index as usize * 2)?);
    let set = LigatureSet::read(FontData::new(ctx.table.get(at..)?)).ok()?;
    let ligatures = set.ligatures();

    // What follows the start, if that is even a question with an answer. It is
    // not when the set holds a single ligature (nothing to choose between), and
    // it is not when the next position is skippable -- a default-ignorable
    // between the two means the glyph at `idx + 1` is not the glyph a matcher
    // would see, so filtering on it would be filtering on the wrong glyph.
    let mut second = u32::MAX;
    let mut unsafe_to = 0usize;
    let one_by_one = if ligatures.len() <= 1 {
        true
    } else {
        let mut iter = skipping_iterator_t::with_match_fn(&mut *ctx.host, true, Some(match_always));
        iter.reset(iter.buffer.idx);
        if iter.next(Some(&mut unsafe_to)) {
            let next = iter.index();
            second = iter.buffer.info[next].glyph_id;
            unsafe_to = next + 1;
            iter.may_skip(&iter.buffer.info[next]) != may_skip_t::SKIP_NO
        } else {
            true
        }
    };

    if one_by_one {
        for lig in ligatures.iter().filter_map(Result::ok) {
            if one_ligature(ctx, &lig).is_some() {
                return Some(());
            }
        }
        return None;
    }

    // The pair key. This is the filter the whole format turns on: `ccmp` covers
    // 72% of English text by first glyph and ligates none of it, because what
    // has to follow is a combining mark.
    //
    // This set is exact, where the filter it replaces is a three-word digest,
    // and that difference is observable in exactly one place: a second glyph
    // the digest lets through reaches the loop below, and the loop records a
    // concat hazard. So we mark fewer positions unsafe-to-concat than this
    // crate does -- 7 cases in 3169 of its own test corpus, and only when a
    // caller has asked for those flags at all.
    //
    // Left exact deliberately. The predicate is "could any ligature here start
    // with this glyph", which is what the digest approximates and what this
    // answers; the extra marks are the approximation showing through, not a
    // hazard anyone identified. Two things support that reading: the digest is
    // the *only* test in that path -- there is no exact check behind it, unlike
    // every coverage probe -- and its value depends on whether the subtable
    // happened to be given an external cache, since without one it is
    // `full()`, which passes everything. Reproducing it would mean reproducing
    // a caching artifact.
    //
    // Gating the shortcut on the flag was tried and is worse: it moves the
    // count from 7 to 91 and in the over-marking direction, because skipping
    // the shortcut is not the same as consulting their digest.
    if let Some(seconds) = sub.next.as_deref() {
        if !seconds.contains(second) {
            return None;
        }
    }

    // A ligature that wanted a different second glyph did not fail because of
    // where this run ends -- but appending text could change what follows, so
    // the two cannot be concatenated blind.
    let mut concat_hazard = false;
    for lig in ligatures.iter().filter_map(Result::ok) {
        let components = lig.component_glyph_ids();
        if components.is_empty() || u32::from(components[0].get()) == second {
            if one_ligature(ctx, &lig).is_some() {
                if concat_hazard {
                    let idx = ctx.host.buffer.idx;
                    ctx.host.buffer.unsafe_to_concat(Some(idx), Some(unsafe_to));
                }
                return Some(());
            }
        } else {
            concat_hazard = true;
        }
    }
    if concat_hazard {
        let idx = ctx.host.buffer.idx;
        ctx.host.buffer.unsafe_to_concat(Some(idx), Some(unsafe_to));
    }
    None
}

/// Try one ligature: match its components, then ligate.
///
/// Both halves are this crate's own. `match_input` is the trickiest walk in
/// OpenType -- ligatures may not form across glyphs attached to different
/// components of an earlier ligature -- and `ligate_input` is what merges the
/// clusters and reassigns the component ids the marks depend on. There is
/// nothing to gain by reimplementing either: they run only once a ligature is
/// actually about to happen, which filtering has made rare.
fn one_ligature(ctx: &mut Apply, lig: &Ligature) -> Option<()> {
    let components = lig.component_glyph_ids();
    let glyph = GlyphId::from(lig.ligature_glyph());
    if components.is_empty() {
        // A one-component ligature is an ordinary substitution. In place, and
        // deliberately not recorded as a ligation, so marks can still attach.
        ctx.host.replace_glyph(glyph);
        return Some(());
    }

    let mut match_end = 0;
    let mut total_components = 0u8;
    let matched = match_input(
        &mut *ctx.host,
        components.len() as u16,
        |info: &mut GlyphInfo, i: u32| {
            components
                .get(i as usize)
                .is_some_and(|c| match_glyph(info, c.get().to_u32()))
        },
        &mut match_end,
        Some(&mut total_components),
    );
    if !matched {
        let idx = ctx.host.buffer.idx;
        ctx.host.buffer.unsafe_to_concat(Some(idx), Some(match_end));
        return None;
    }
    ligate_input(
        &mut *ctx.host,
        components.len() + 1,
        match_end,
        total_components,
        glyph,
    );
    Some(())
}

/// Alternate substitution: the feature's value selects which alternate, so
/// this is the one format that reads the lookup mask for more than a test.
///
/// Flags: **owes the whole buffer `unsafe_to_break` when it randomises.** The
/// `rand` feature asks for a different alternate each time, which means the
/// answer depends on how many times this ran before -- so no part of the run
/// can be reshaped in isolation. HarfBuzz notes that it could be narrower and
/// does not bother, and neither does this.
pub fn at_alternate(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::Alternate { offset } = &sub.kind else {
        return None;
    };
    // The alternate sets stay in the font: one per covered glyph, and a run
    // reads at most one of them. Two indexed reads to reach ours.
    let base = *offset as usize;
    if index >= u32::from(be16(ctx.table, base + 4)?) {
        return None;
    }
    let set = base + usize::from(be16(ctx.table, base + 6 + index as usize * 2)?);
    let count = be16(ctx.table, set)?;

    // The feature's value picks the variant, and it rides in this lookup's own
    // bits of the mask, shifted down to a plain index. A value of one -- what
    // enabling a feature normally means -- selects the first alternate.
    let mask = ctx.host.lookup_mask();
    let mut value = (mask & ctx.host.buffer.cur(0).mask) >> mask.trailing_zeros();

    // The saturated value means `rand`: pick one at random rather than by
    // index. That makes the result depend on how many times this lookup has
    // already run, so no part of the run can be reshaped on its own -- hence
    // the whole buffer, not the matched position. HarfBuzz notes that it could
    // be narrower and does not bother, because narrowing it would mean
    // tracking the random state per position.
    if value == hb_ot_map_t::MAX_VALUE && ctx.host.random {
        if count == 0 {
            return None;
        }
        ctx.host
            .buffer
            .unsafe_to_break(Some(0), Some(ctx.host.buffer.len));
        value = ctx.host.random_number() % u32::from(count) + 1;
    }

    let alt = (value as usize).checked_sub(1)?;
    if alt >= usize::from(count) {
        return None;
    }
    let glyph = be16(ctx.table, set + 2 + alt * 2)?;
    ctx.host.replace_glyph(GlyphId::from(u32::from(glyph)));
    Some(())
}

/// Reverse chaining contextual single substitution.
///
/// A single input glyph with a context on both sides. The caller runs these
/// descending, from the end of the buffer towards the start, which is what
/// makes the format expressible: each position's rule depends on the decision
/// one to its right having already been made.
///
/// Flags: **owes `unsafe_to_break` over the context it matched**, backtrack and
/// lookahead included -- the substitution depended on glyphs either side, so
/// neither side can be cut away from it.
pub fn at_reverse_chain(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::ReverseChain {
        backtrack,
        lookahead,
        subst,
    } = &sub.kind
    else {
        return None;
    };
    // Nothing may chain to this type, so being here at any depth but the top
    // means a font asked for something the format does not offer.
    if ctx.host.nesting_level_left != MAX_NESTING_LEVEL {
        return None;
    }
    // Read the substitute before walking the context: one indexed big-endian
    // load that cannot fail for a well-formed font, and a malformed one must
    // not be allowed to match, so bailing here saves the walk rather than
    // discovering the missing glyph after paying for it.
    let glyph = subst.get(ctx.table, index)?;

    let mut start = 0;
    let mut end = 0;
    // Two walks, and the second only if the first got anywhere. Split rather
    // than chained with `&&` because the lookahead has to start after the input
    // glyph, and reading the cursor for that needs the borrow the first walk
    // holds.
    let matched = match_backtrack(
        &mut *ctx.host,
        backtrack.len() as u16,
        |info: &mut GlyphInfo, i: u32| {
            backtrack
                .get(i as usize)
                .is_some_and(|set| set.contains(info.glyph_id))
        },
        &mut start,
    );
    let after = ctx.host.buffer.idx + 1;
    let matched = matched
        && match_lookahead(
            &mut *ctx.host,
            lookahead.len() as u16,
            |info: &mut GlyphInfo, i: u32| {
                lookahead
                    .get(i as usize)
                    .is_some_and(|set| set.contains(info.glyph_id))
            },
            after,
            &mut end,
        );

    if !matched {
        ctx.host
            .buffer
            .unsafe_to_concat_from_outbuffer(Some(start), Some(end));
        return None;
    }

    // The context on both sides decided this substitution, so neither side can
    // be cut away from it -- which is why the whole matched span is marked,
    // not just the position that changed.
    ctx.host
        .buffer
        .unsafe_to_break_from_outbuffer(Some(start), Some(end));
    ctx.host
        .replace_glyph_inplace(GlyphId::from(u32::from(glyph)));
    // Deliberately not advancing: the descending loop owns the cursor. Leaving
    // it alone is also what keeps this harmless if a font does chain to it.
    Some(())
}
