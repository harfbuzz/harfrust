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
//! Every one of these is a stub. See [`super::gsub`].

use super::lookup::{Apply, CompiledLookup, SeqRecord, Subtable, SubtableKind};
use crate::hb::buffer::GlyphInfo;
use crate::hb::ot_layout_gsubgpos::{apply_lookup, match_backtrack, match_input, match_lookahead};

/// The rule-based contexts: sequence context 1 and 2, chained 1 and 2.
///
/// The compiled part is two indexes the font does not contain, and they are
/// what makes this cheap on the fonts that lean on it hardest:
///
/// * [`super::lookup::SetDigests`], one word per rule set, so a set that
///   cannot match is thrown away before a single rule header is parsed.
/// * [`super::lookup::RuleFirsts`], one byte per rule, so a rule whose first
///   input step wants something the buffer does not offer is thrown away
///   without parsing its header -- which is variable-length, so reaching that
///   first value otherwise means walking the backtrack sequence out of the
///   font.
pub fn at_rules(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::Rules { .. } = &sub.kind else {
        return None;
    };
    None
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
/// The nested lookups themselves still recurse through the host's own path
/// rather than the compiled program. That is the next step and the reason
/// [`Apply::program`] exists; until then this isolates what is under test --
/// the context match -- from the application of whatever it invokes.
fn apply_nested(ctx: &mut Apply, input_len: usize, match_end: usize, records: &[SeqRecord]) {
    apply_lookup(
        &mut *ctx.host,
        input_len,
        match_end,
        records.iter().map(|r| (r.seq_index, r.lookup_index)),
    );
}
