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

use super::lookup::{Apply, CompiledLookup, Subtable, SubtableKind};

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
/// coverage index -- so all three arrays are compiled to sets without rank
/// tables. The first input coverage is not among them: it is the gate, and it
/// lives on the [`Subtable`] alongside every other format's.
pub fn at_chain3(
    _ctx: &mut Apply,
    _lookup: &CompiledLookup,
    sub: &Subtable,
    _index: u32,
) -> Option<()> {
    let SubtableKind::ChainCtx3 { .. } = &sub.kind else {
        return None;
    };
    None
}
