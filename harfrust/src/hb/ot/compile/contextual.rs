//! Applying the contextual formats.
//!
//! These are the formats that match a pattern of positions and then invoke
//! other lookups at the positions they matched, and they are where a shaping
//! run of a complex script spends most of its time. Two routines cover all six
//! OpenType formats:
//!
//! * [`at_rules`] takes the rule-based ones -- sequence context formats 1 and
//!   2, and chained formats 1 and 2. A rule set is selected by the first
//!   glyph's coverage index or class, and its rules are tried in font order.
//! * [`at_chain3`] takes format 3 of both, where there are no rules at all:
//!   the pattern is one run of coverages, so the subtable *is* the rule.
//!
//! On the pointer these are reached through, and on the flags each owes the
//! buffer, see [`super::gsub`].

use super::apply::recurse;
use super::be16;
use super::lookup::{Apply, CompiledLookup, SeqRecord, Subtable, SubtableKind};
use super::set::ClassMap;
use crate::hb::buffer::GlyphInfo;
use crate::hb::ot::contextual::{apply_chain_context_rules, apply_context_rules};
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{apply_lookup, match_backtrack, match_input, match_lookahead};
use read_fonts::types::{BigEndian, Offset16};
use read_fonts::FontData;

/// The rule-based contexts: sequence context 1 and 2, chained 1 and 2.
///
/// One routine covers four formats because they differ in only two ways, and
/// both are settled when the subtable is compiled: whether a position is
/// compared against a glyph id or against a class, and whether a rule carries
/// backtrack and lookahead. The rules themselves are variable-length arrays,
/// so they stay in the font.
///
/// What is compiled is the class definitions, which every probe reads. A
/// class-based context resolves a class per rule per position and a rule set
/// can hold hundreds; this crate answers that from a cache in front of the
/// font, and here it is an owned map with no font access at all.
///
/// The nested lookups a matching rule invokes recurse through the compiled
/// program rather than the font's own path -- see [`super::apply::recurse`].
///
/// The rule-set walk is this crate's own, and deliberately. It probes a rule's
/// first two input values without parsing it, skips ahead over runs of rules
/// sharing a first value, and accumulates exactly which concat hazard to
/// report -- and that last part is a function of *which* probes ran, so it is
/// not something to reimplement next to a filter that changes them. Layering
/// the per-set and per-rule summaries on top is the next step, and it has to
/// answer for those flags rather than merely be faster.
///
/// Flags: owed entirely by that walk, which reports a break hazard over a
/// matched span and a concat hazard over the longest prefix any rule agreed
/// with.
pub fn at_rules(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    index: u32,
) -> Option<()> {
    let SubtableKind::Rules {
        input_classes,
        backtrack_classes,
        lookahead_classes,
        rule_sets,
        base,
        rule_set_count,
        chained,
        index: rule_index,
        ..
    } = &sub.kind
    else {
        return None;
    };

    // Which rule set. Format 1 indexes them by coverage rank -- which the gate
    // already computed, and is the only reason that format ranks -- and format
    // 2 by the first glyph's class, throwing the rank away.
    let set_index = match input_classes {
        Some(classes) => u32::from(classes.get(ctx.glyph())),
        None => index,
    };
    if set_index >= u32::from(*rule_set_count) {
        return None;
    }
    let offset = be16(ctx.table, *rule_sets as usize + set_index as usize * 2)?;
    // A null offset means this glyph or class has no rules, which is most of
    // them: the array has an entry per class whether or not it is used.
    if offset == 0 {
        return None;
    }

    let set = FontData::new(ctx.table.get(*base as usize + usize::from(offset)..)?);
    let count = usize::from(set.read_at::<u16>(0).ok()?);
    let rules: &[BigEndian<Offset16>] = set.read_array(2..2 + count * 2).ok()?;

    // Copied out before the context is borrowed mutably, so the nested lookups
    // can be found on the compiled path rather than the font's.
    let (table, program) = (ctx.table, ctx.program);
    let recurse = |host: &mut hb_ot_apply_context_t, index| recurse(host, table, program, index);

    // One word per rule set, saying which values its rules accept at the
    // position every one of them begins by testing. Nothing in a rule set says
    // that, so this is the one filter here with no counterpart in the font --
    // and it earns its place because a set that survives coverage still mostly
    // does not match: measured on Nastaliq, half the entered rule sets are dead
    // this way, and they hold half of every rule that would otherwise be
    // parsed.
    //
    // Answering it with a single glyph's value rather than a window of them is
    // deliberate. It is the same question the walk inside asks per rule, at the
    // same position, so dismissing a set here reaches exactly the conclusion
    // that walk would have reached -- including the concat hazard it would have
    // reported. A wider window would reject more and would then owe a flag it
    // no longer sets.
    let digests = &rule_index.digests;
    let input = input_classes.as_deref();
    let dismiss =
        |info: &mut GlyphInfo| !digests.may_hold(set_index, class_of(input, info.glyph_id));

    match chained {
        false => apply_context_rules(
            &mut *ctx.host,
            set,
            rules,
            |info, value| class_of(input_classes.as_deref(), info.glyph_id) == value,
            &recurse,
            Some(&dismiss),
        ),
        true => apply_chain_context_rules(
            &mut *ctx.host,
            set,
            rules,
            (
                |info: &mut GlyphInfo, value| {
                    class_of(backtrack_classes.as_deref(), info.glyph_id) == value
                },
                |info: &mut GlyphInfo, value| {
                    class_of(input_classes.as_deref(), info.glyph_id) == value
                },
                |info: &mut GlyphInfo, value| {
                    class_of(lookahead_classes.as_deref(), info.glyph_id) == value
                },
            ),
            &recurse,
            Some(&dismiss),
        ),
    }
}

/// What a rule compares a position against.
///
/// Format 2 compares classes; format 1 compares glyph ids, which is the same
/// question with the identity map, and is why there is one routine here rather
/// than four.
#[inline]
fn class_of(classes: Option<&ClassMap>, glyph: u32) -> u32 {
    match classes {
        Some(map) => u32::from(map.get(glyph)),
        None => glyph,
    }
}

/// Format 3 of both contexts: one run of coverages, no rules.
///
/// Every coverage here is a pure membership test -- the format never uses a
/// coverage index -- so all three arrays compile to sets without rank tables.
/// The first input coverage is not among them: it is the gate, and it lives on
/// the [`Subtable`] alongside every other format's, which is why the input run
/// below starts at the second.
///
/// One routine covers the plain and the chained format, since a plain one is
/// just a chained one with nothing on either side. They part company only in
/// which flag calls a match makes, which is what `chained` records.
///
/// Flags: **owes its whole matched span.** A match means these positions were
/// chosen because of each other, so a caller may not break inside the span;
/// and a *failed* match still owes a concat hazard over what it did match,
/// because appending text could complete the pattern.
pub fn at_chain3(
    ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::ChainCtx3 {
        backtrack,
        input,
        lookahead,
        records,
        chained,
    } = &sub.kind
    else {
        return None;
    };

    // The input run, from the second coverage on. `end` tracks how far the
    // pattern got, because a failure has to report it.
    let mut end = ctx.host.buffer.idx;
    let mut match_end = 0;
    let matched = match_input(
        &mut *ctx.host,
        input.len() as u16,
        |info: &mut GlyphInfo, i: u32| {
            input
                .get(i as usize)
                .is_some_and(|set| set.contains(info.glyph_id))
        },
        &mut match_end,
        None,
    );
    if matched {
        end = match_end;
    }
    let matched = matched
        && match_lookahead(
            &mut *ctx.host,
            lookahead.len() as u16,
            |info: &mut GlyphInfo, i: u32| {
                lookahead
                    .get(i as usize)
                    .is_some_and(|set| set.contains(info.glyph_id))
            },
            match_end,
            &mut end,
        );

    if !matched {
        let idx = ctx.host.buffer.idx;
        match chained {
            true => ctx.host.buffer.unsafe_to_concat(Some(idx), Some(end)),
            false => ctx.host.buffer.unsafe_to_concat(Some(idx), Some(match_end)),
        }
        return None;
    }

    if !chained {
        let idx = ctx.host.buffer.idx;
        ctx.host.buffer.unsafe_to_break(Some(idx), Some(match_end));
        apply_nested(ctx, input.len(), match_end, records);
        return Some(());
    }

    let mut start = ctx.host.buffer.out_len;
    if !match_backtrack(
        &mut *ctx.host,
        backtrack.len() as u16,
        |info: &mut GlyphInfo, i: u32| {
            backtrack
                .get(i as usize)
                .is_some_and(|set| set.contains(info.glyph_id))
        },
        &mut start,
    ) {
        ctx.host
            .buffer
            .unsafe_to_concat_from_outbuffer(Some(start), Some(end));
        return None;
    }

    ctx.host
        .buffer
        .unsafe_to_break_from_outbuffer(Some(start), Some(end));
    apply_nested(ctx, input.len(), match_end, records);
    Some(())
}

/// Run a context's nested lookups at the positions it matched.
///
/// The renumbering around this is `apply_lookup`, which is this crate's own
/// and now takes the records as pairs so both paths can share it. That is
/// deliberate: what it does is fix up match positions when a nested lookup
/// changes the buffer length, the hard cases are documented in its TODOs, and
/// duplicating it to save a `map` would be trading correctness for nothing.
///
/// The nested lookups recurse through the compiled program rather than the
/// font's own path -- see [`super::apply::recurse`]. Without that a context
/// would be a hole through which everything fell back, and on a cursive script
/// that is most of the work.
fn apply_nested(ctx: &mut Apply, input_len: usize, match_end: usize, records: &[SeqRecord]) {
    let (table, program) = (ctx.table, ctx.program);
    apply_lookup(
        &mut *ctx.host,
        input_len,
        match_end,
        records.iter().map(|r| (r.seq_index, r.lookup_index)),
        &|host: &mut hb_ot_apply_context_t, index| recurse(host, table, program, index),
    );
}
