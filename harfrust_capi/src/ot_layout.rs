//! OpenType layout metadata, lookup glyph collection, and BASE baseline queries.

use core::ffi::c_uint;

use harfrust::Tag;
use read_fonts::tables::base::{BaseAxis, BaseInstance};
use read_fonts::tables::gpos::{PairPos, PositionSubtables};
use read_fonts::tables::gsub::{SingleSubst, SubstitutionLookupList, SubstitutionSubtables};
use read_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, CoverageTable, FeatureList, ScriptList, SequenceContext,
    SequenceLookupRecord,
};
use read_fonts::types::F48Dot16;
use read_fonts::{ReadError, TableProvider};

use crate::common::hr_glyph_extents_t;
use crate::common::{
    hr_bool_t, hr_direction_t, hr_language_t, hr_position_t, hr_script_t, hr_tag_t, script_to_rust,
    tag_from_rust, tag_to_rust, HR_DIRECTION_LTR, HR_DIRECTION_RTL,
};
use crate::face::hr_face_t;
use crate::font::{hr_font_get_glyph_extents, hr_font_get_nominal_glyph, hr_font_t};
use crate::object;
use crate::set::hr_set_t;

/// The OpenType substitution table tag.
pub const HR_OT_TAG_GSUB: hr_tag_t = 0x4753_5542u32;
/// The OpenType positioning table tag.
pub const HR_OT_TAG_GPOS: hr_tag_t = 0x4750_4F53u32;
/// The default OpenType script tag, `DFLT`.
pub const HR_OT_TAG_DEFAULT_SCRIPT: hr_tag_t = 0x4446_4C54u32;
/// The default OpenType language tag, `dflt`.
pub const HR_OT_TAG_DEFAULT_LANGUAGE: hr_tag_t = 0x6466_6C74u32;
/// No script was selected.
pub const HR_OT_LAYOUT_NO_SCRIPT_INDEX: c_uint = 0xFFFFu32;
/// No feature was found.
pub const HR_OT_LAYOUT_NO_FEATURE_INDEX: c_uint = 0xFFFFu32;
/// Selects a script's default language system.
pub const HR_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX: c_uint = 0xFFFFu32;

const GSUB_TAG: hr_tag_t = HR_OT_TAG_GSUB;
const GPOS_TAG: hr_tag_t = HR_OT_TAG_GPOS;

fn script_list(face: &hr_face_t, table_tag: hr_tag_t) -> Option<ScriptList<'_>> {
    let font = face.font()?;
    match table_tag {
        GSUB_TAG => font.tables().gsub().ok()?.script_list().ok(),
        GPOS_TAG => font.tables().gpos().ok()?.script_list().ok(),
        _ => None,
    }
}

fn feature_list(face: &hr_face_t, table_tag: hr_tag_t) -> Option<FeatureList<'_>> {
    let font = face.font()?;
    match table_tag {
        GSUB_TAG => font.tables().gsub().ok()?.feature_list().ok(),
        GPOS_TAG => font.tables().gpos().ok()?.feature_list().ok(),
        _ => None,
    }
}

/// Selects the first available requested script in GSUB or GPOS.
///
/// If none matches, tries `DFLT`, `dflt`, then `latn`. Returns true only for
/// a requested script, even when a fallback was selected. With no match,
/// writes `HR_OT_LAYOUT_NO_SCRIPT_INDEX` and a zero tag to the outputs.
///
/// # Safety
/// `face` must be null or live; `script_tags` must hold `script_count` tags
/// when non-null. Both output pointers must be null or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_table_select_script(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
    script_count: c_uint,
    script_tags: *const hr_tag_t,
    script_index: *mut c_uint,
    chosen_script: *mut hr_tag_t,
) -> hr_bool_t {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let list = script_list(face, table_tag);
    let records = list.as_ref().map_or(&[][..], |list| list.script_records());
    let requested = if script_tags.is_null() {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(script_tags, script_count as usize) }
    };
    let fallbacks = [
        HR_OT_TAG_DEFAULT_SCRIPT,
        HR_OT_TAG_DEFAULT_LANGUAGE,
        u32::from_be_bytes(*b"latn"),
    ];
    let selected = requested
        .iter()
        .map(|tag| (*tag, true))
        .chain(fallbacks.into_iter().map(|tag| (tag, false)))
        .find_map(|(tag, requested)| {
            records
                .binary_search_by_key(&tag_to_rust(tag), |record| record.script_tag())
                .ok()
                .map(|index| (index as c_uint, tag, requested))
        });
    let (index, tag, requested) = selected.unwrap_or((HR_OT_LAYOUT_NO_SCRIPT_INDEX, 0, false));
    if let Some(output) = unsafe { script_index.as_mut() } {
        *output = index;
    }
    if let Some(output) = unsafe { chosen_script.as_mut() } {
        *output = tag;
    }
    requested.into()
}

/// Returns the total number of feature records in GSUB or GPOS.
///
/// Copies tags starting at `start_offset`, including duplicates. When both
/// array and count are non-null, the count gives capacity on entry and the
/// number written on return. A null array leaves the count unchanged.
///
/// # Safety
/// `face` must be null or live; `feature_count` must be null or writable;
/// `feature_tags` must hold the input capacity when non-null.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_table_get_feature_tags(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
    start_offset: c_uint,
    feature_count: *mut c_uint,
    feature_tags: *mut hr_tag_t,
) -> c_uint {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let list = feature_list(face, table_tag);
    let records = list.as_ref().map_or(&[][..], |list| list.feature_records());
    if !feature_tags.is_null() {
        if let Some(count) = unsafe { feature_count.as_mut() } {
            let mut written = 0;
            for record in records
                .iter()
                .skip(start_offset as usize)
                .take(*count as usize)
            {
                unsafe {
                    feature_tags
                        .add(written)
                        .write(tag_from_rust(record.feature_tag()));
                };
                written += 1;
            }
            *count = written as c_uint;
        }
    }
    records.len() as c_uint
}

/// Finds an optional feature in a script's language system in GSUB or GPOS.
///
/// Use `HR_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX` for the default language system.
/// A required feature is not included in this search. Writes the feature's
/// table-wide index, or `HR_OT_LAYOUT_NO_FEATURE_INDEX` on failure.
///
/// # Safety
/// `face` must be null or live; `feature_index` must be null or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_language_find_feature(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
    script_index: c_uint,
    language_index: c_uint,
    feature_tag: hr_tag_t,
    feature_index: *mut c_uint,
) -> hr_bool_t {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let found = (|| {
        let scripts = script_list(face, table_tag)?;
        let script = scripts
            .script_records()
            .get(script_index as usize)?
            .script(scripts.offset_data())
            .ok()?;
        let language = if language_index == HR_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX {
            script.default_lang_sys()?.ok()?
        } else {
            script
                .lang_sys_records()
                .get(language_index as usize)?
                .lang_sys(script.offset_data())
                .ok()?
        };
        let features = feature_list(face, table_tag)?;
        language.feature_indices().iter().find_map(|index| {
            let index = index.get() as usize;
            let tag = features
                .feature_records()
                .get(index)
                .map_or(0, |record| tag_from_rust(record.feature_tag()));
            (tag == feature_tag).then_some(index as c_uint)
        })
    })();
    if let Some(output) = unsafe { feature_index.as_mut() } {
        *output = found.unwrap_or(HR_OT_LAYOUT_NO_FEATURE_INDEX);
    }
    found.is_some().into()
}

/// Returns whether the face has a readable GSUB table.
///
/// # Safety
/// `face` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_has_substitution(face: *mut hr_face_t) -> hr_bool_t {
    let face = unsafe { object::or_empty(face.cast_const()) };
    face.font()
        .is_some_and(|font| font.tables().gsub().is_ok())
        .into()
}

/// Returns whether the face has a readable GPOS table.
///
/// # Safety
/// `face` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_has_positioning(face: *mut hr_face_t) -> hr_bool_t {
    let face = unsafe { object::or_empty(face.cast_const()) };
    face.font()
        .is_some_and(|font| font.tables().gpos().is_ok())
        .into()
}

/// Returns the number of lookups in GSUB or GPOS.
///
/// # Safety
/// `face` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_table_get_lookup_count(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
) -> c_uint {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let Some(font) = face.font() else { return 0 };
    match table_tag {
        GSUB_TAG => font
            .tables()
            .gsub()
            .ok()
            .and_then(|table| table.lookup_list().ok())
            .map_or(0, |list| list.lookups().len() as c_uint),
        GPOS_TAG => font
            .tables()
            .gpos()
            .ok()
            .and_then(|table| table.lookup_list().ok())
            .map_or(0, |list| list.lookups().len() as c_uint),
        _ => 0,
    }
}

#[derive(Clone, Copy)]
struct SetTarget(*mut hr_set_t);

impl SetTarget {
    fn add(self, glyph: u32) {
        if let Some(set) = unsafe { object::as_mutable(self.0) } {
            set.add(glyph);
        }
    }

    fn include_all_glyphs(self) {
        if let Some(set) = unsafe { object::as_mutable(self.0) } {
            set.include_all_glyphs(65536);
        }
    }
}

fn add_coverage(set: SetTarget, coverage: CoverageTable<'_>) {
    for glyph in coverage.iter() {
        set.add(glyph.to_u32());
    }
}

fn add_class_coverage(set: SetTarget, class_def: ClassDef<'_>) {
    match class_def {
        ClassDef::Format1(table) => {
            let start = table.start_glyph_id().to_u32();
            let mut previous_nonzero = false;
            for (index, class) in table.class_value_array().iter().enumerate() {
                let nonzero = class.get() != 0;
                if nonzero || previous_nonzero {
                    set.add(start + index as u32);
                }
                previous_nonzero = nonzero;
            }
            if previous_nonzero {
                set.add(start + table.class_value_array().len() as u32);
            }
        }
        ClassDef::Format2(table) => {
            for (glyph, class) in table.iter() {
                if class != 0 {
                    set.add(glyph.to_u32());
                }
            }
        }
    }
}

fn add_class(set: SetTarget, class_def: &ClassDef<'_>, selected: u16) {
    for (glyph, class) in class_def.iter() {
        if class == selected {
            set.add(glyph.to_u32());
        }
    }
}

fn collect_nested_substitutions(
    records: &[SequenceLookupRecord],
    lookup_list: &SubstitutionLookupList<'_>,
    output: SetTarget,
    depth: u8,
) -> Result<bool, ReadError> {
    for record in records {
        let nested = lookup_list
            .lookups()
            .get(record.lookup_list_index() as usize)?;
        if !collect_substitution(
            nested,
            lookup_list,
            SetTarget(core::ptr::null_mut()),
            SetTarget(core::ptr::null_mut()),
            SetTarget(core::ptr::null_mut()),
            output,
            depth + 1,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn collect_substitution(
    lookup: read_fonts::tables::gsub::SubstitutionLookup<'_>,
    lookup_list: &SubstitutionLookupList<'_>,
    before: SetTarget,
    input: SetTarget,
    after: SetTarget,
    output: SetTarget,
    depth: u8,
) -> Result<bool, ReadError> {
    if depth >= 8 {
        return Ok(false);
    }
    match lookup.subtables()? {
        SubstitutionSubtables::Single(subtables) => {
            for subtable in subtables.iter() {
                match subtable? {
                    SingleSubst::Format1(table) => {
                        let coverage = table.coverage()?;
                        for glyph in coverage.iter() {
                            let id = glyph.to_u32();
                            input.add(id);
                            output.add((id as u16).wrapping_add_signed(table.delta_glyph_id()) as u32);
                        }
                    }
                    SingleSubst::Format2(table) => {
                        add_coverage(input, table.coverage()?);
                        for glyph in table.substitute_glyph_ids() {
                            output.add(glyph.get().to_u32());
                        }
                    }
                }
            }
        }
        SubstitutionSubtables::Multiple(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.coverage()?);
                for sequence in table.sequences().iter() {
                    for glyph in sequence?.substitute_glyph_ids() {
                        output.add(glyph.get().to_u32());
                    }
                }
            }
        }
        SubstitutionSubtables::Alternate(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.coverage()?);
                for alternate in table.alternate_sets().iter() {
                    for glyph in alternate?.alternate_glyph_ids() {
                        output.add(glyph.get().to_u32());
                    }
                }
            }
        }
        SubstitutionSubtables::Ligature(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.coverage()?);
                for set in table.ligature_sets().iter() {
                    for ligature in set?.ligatures().iter() {
                        let ligature = ligature?;
                        output.add(ligature.ligature_glyph().to_u32());
                        for glyph in ligature.component_glyph_ids() {
                            input.add(glyph.get().to_u32());
                        }
                    }
                }
            }
        }
        SubstitutionSubtables::Reverse(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.coverage()?);
                for coverage in table.backtrack_coverages().iter() {
                    add_coverage(before, coverage?);
                }
                for coverage in table.lookahead_coverages().iter() {
                    add_coverage(after, coverage?);
                }
                for glyph in table.substitute_glyph_ids() {
                    output.add(glyph.get().to_u32());
                }
            }
        }
        SubstitutionSubtables::Contextual(subtables) => {
            for subtable in subtables.iter() {
                match subtable? {
                    SequenceContext::Format1(table) => {
                        add_coverage(input, table.coverage()?);
                        for rule_set in table.seq_rule_sets().iter().flatten() {
                            for rule in rule_set?.seq_rules().iter() {
                                let rule = rule?;
                                for glyph in rule.input_sequence() {
                                    input.add(glyph.get().to_u32());
                                }
                                if !collect_nested_substitutions(
                                    rule.seq_lookup_records(),
                                    lookup_list,
                                    output,
                                    depth,
                                )? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    SequenceContext::Format2(table) => {
                        add_coverage(input, table.coverage()?);
                        let class_def = table.class_def()?;
                        for rule_set in table.class_seq_rule_sets().iter().flatten() {
                            for rule in rule_set?.class_seq_rules().iter() {
                                let rule = rule?;
                                for class in rule.input_sequence() {
                                    add_class(input, &class_def, class.get());
                                }
                                if !collect_nested_substitutions(
                                    rule.seq_lookup_records(),
                                    lookup_list,
                                    output,
                                    depth,
                                )? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    SequenceContext::Format3(table) => {
                        for coverage in table.coverages().iter() {
                            add_coverage(input, coverage?);
                        }
                        if !collect_nested_substitutions(
                            table.seq_lookup_records(),
                            lookup_list,
                            output,
                            depth,
                        )? {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        SubstitutionSubtables::ChainContextual(subtables) => {
            for subtable in subtables.iter() {
                match subtable? {
                    ChainedSequenceContext::Format1(table) => {
                        add_coverage(input, table.coverage()?);
                        for rule_set in table.chained_seq_rule_sets().iter().flatten() {
                            for rule in rule_set?.chained_seq_rules().iter() {
                                let rule = rule?;
                                for glyph in rule.backtrack_sequence() {
                                    before.add(glyph.get().to_u32());
                                }
                                for glyph in rule.input_sequence() {
                                    input.add(glyph.get().to_u32());
                                }
                                for glyph in rule.lookahead_sequence() {
                                    after.add(glyph.get().to_u32());
                                }
                                if !collect_nested_substitutions(
                                    rule.seq_lookup_records(),
                                    lookup_list,
                                    output,
                                    depth,
                                )? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    ChainedSequenceContext::Format2(table) => {
                        add_coverage(input, table.coverage()?);
                        let backtrack_class_def = table.backtrack_class_def()?;
                        let input_class_def = table.input_class_def()?;
                        let lookahead_class_def = table.lookahead_class_def()?;
                        for rule_set in table.chained_class_seq_rule_sets().iter().flatten() {
                            for rule in rule_set?.chained_class_seq_rules().iter() {
                                let rule = rule?;
                                for class in rule.backtrack_sequence() {
                                    add_class(before, &backtrack_class_def, class.get());
                                }
                                for class in rule.input_sequence() {
                                    add_class(input, &input_class_def, class.get());
                                }
                                for class in rule.lookahead_sequence() {
                                    add_class(after, &lookahead_class_def, class.get());
                                }
                                if !collect_nested_substitutions(
                                    rule.seq_lookup_records(),
                                    lookup_list,
                                    output,
                                    depth,
                                )? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    ChainedSequenceContext::Format3(table) => {
                        let mut coverages = table.input_coverages().iter();
                        add_coverage(input, coverages.next().ok_or(ReadError::OutOfBounds)??);
                        for coverage in coverages {
                            add_coverage(input, coverage?);
                        }
                        for coverage in table.backtrack_coverages().iter() {
                            add_coverage(before, coverage?);
                        }
                        for coverage in table.lookahead_coverages().iter() {
                            add_coverage(after, coverage?);
                        }
                        if !collect_nested_substitutions(
                            table.seq_lookup_records(),
                            lookup_list,
                            output,
                            depth,
                        )? {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        SubstitutionSubtables::EmptyExtension => {}
    }
    Ok(true)
}

fn collect_positioning(
    lookup: read_fonts::tables::gpos::PositionLookup<'_>,
    input: SetTarget,
) -> Result<bool, ReadError> {
    match lookup.subtables()? {
        PositionSubtables::Single(subtables) => {
            for table in subtables.iter() {
                match table? {
                    read_fonts::tables::gpos::SinglePos::Format1(table) => {
                        add_coverage(input, table.coverage()?);
                    }
                    read_fonts::tables::gpos::SinglePos::Format2(table) => {
                        add_coverage(input, table.coverage()?);
                    }
                }
            }
        }
        PositionSubtables::Pair(subtables) => {
            for table in subtables.iter() {
                match table? {
                    PairPos::Format1(table) => {
                        add_coverage(input, table.coverage()?);
                        for pair_set in table.pair_sets().iter() {
                            for record in pair_set?.pair_value_records().iter() {
                                input.add(record?.second_glyph().to_u32());
                            }
                        }
                    }
                    PairPos::Format2(table) => {
                        add_coverage(input, table.coverage()?);
                        add_class_coverage(input, table.class_def2()?);
                    }
                }
            }
        }
        PositionSubtables::Cursive(subtables) => {
            for table in subtables.iter() {
                add_coverage(input, table?.coverage()?);
            }
        }
        PositionSubtables::MarkToBase(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.mark_coverage()?);
                add_coverage(input, table.base_coverage()?);
            }
        }
        PositionSubtables::MarkToLig(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.mark_coverage()?);
                add_coverage(input, table.ligature_coverage()?);
            }
        }
        PositionSubtables::MarkToMark(subtables) => {
            for table in subtables.iter() {
                let table = table?;
                add_coverage(input, table.mark1_coverage()?);
                add_coverage(input, table.mark2_coverage()?);
            }
        }
        PositionSubtables::EmptyExtension => {}
        _ => return Ok(false),
    }
    Ok(true)
}

/// Collects glyphs touched by one GSUB or GPOS lookup into the supplied sets.
/// Forms not yet enumerated conservatively include every OpenType glyph ID.
///
/// # Safety
/// `face` and each non-`NULL` set must be live. Output sets may be the same.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_lookup_collect_glyphs(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
    lookup_index: c_uint,
    glyphs_before: *mut hr_set_t,
    glyphs_input: *mut hr_set_t,
    glyphs_after: *mut hr_set_t,
    glyphs_output: *mut hr_set_t,
) {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let Some(font) = face.font() else { return };
    // Each insertion borrows its destination separately, so output pointers may alias.
    let before = SetTarget(glyphs_before);
    let input = SetTarget(glyphs_input);
    let after = SetTarget(glyphs_after);
    let output = SetTarget(glyphs_output);
    let complete = match table_tag {
        GSUB_TAG => font.tables().gsub().ok().and_then(|table| {
            let list = table.lookup_list().ok()?;
            let lookup = list.lookups().get(lookup_index as usize).ok()?;
            Some(collect_substitution(
                lookup, &list, before, input, after, output, 0,
            ))
        }),
        GPOS_TAG => font
            .tables()
            .gpos()
            .ok()
            .and_then(|table| table.lookup_list().ok())
            .and_then(|list| list.lookups().get(lookup_index as usize).ok())
            .map(|lookup| collect_positioning(lookup, input)),
        _ => None,
    };
    let Some(complete) = complete else { return };
    if !matches!(complete, Ok(true)) {
        for set in [before, input, after, output] {
            set.include_all_glyphs();
        }
    }
}

/// A registered OpenType BASE baseline tag. The numeric value is the tag itself.
pub type hr_ot_layout_baseline_tag_t = hr_tag_t;

pub const HR_OT_LAYOUT_BASELINE_TAG_ROMAN: hr_ot_layout_baseline_tag_t = 0x726F_6D6Eu32;
pub const HR_OT_LAYOUT_BASELINE_TAG_HANGING: hr_ot_layout_baseline_tag_t = 0x6861_6E67u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT: hr_ot_layout_baseline_tag_t =
    0x6963_6662u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT: hr_ot_layout_baseline_tag_t =
    0x6963_6674u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_CENTRAL: hr_ot_layout_baseline_tag_t = 0x4963_6663u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT: hr_ot_layout_baseline_tag_t =
    0x6964_656Fu32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT: hr_ot_layout_baseline_tag_t =
    0x6964_7470u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_CENTRAL: hr_ot_layout_baseline_tag_t =
    0x4964_6365u32;
pub const HR_OT_LAYOUT_BASELINE_TAG_MATH: hr_ot_layout_baseline_tag_t = 0x6D61_7468u32;

/// Returns the dominant horizontal baseline for a Unicode script.
#[no_mangle]
pub extern "C" fn hr_ot_layout_get_horizontal_baseline_tag_for_script(
    script: hr_script_t,
) -> hr_ot_layout_baseline_tag_t {
    tag_from_rust(read_fonts::tables::base::horizontal_baseline_tag_for_script(tag_to_rust(script)))
}

fn axis(direction: hr_direction_t) -> BaseAxis {
    if matches!(direction, HR_DIRECTION_LTR | HR_DIRECTION_RTL) {
        BaseAxis::Horizontal
    } else {
        BaseAxis::Vertical
    }
}

fn scale_metric(value: F48Dot16, scale: i32, upem: i32) -> i32 {
    (value.to_f64() * f64::from(scale) / f64::from(upem)).round() as i32
}

fn baseline(
    font: &hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script_tag: hr_tag_t,
) -> Option<hr_position_t> {
    let instance = font.instance()?;
    let base = instance.tables().base().ok()?;
    let base = BaseInstance::with_coords(base, instance.normalized_coords());
    let horizontal = matches!(axis(direction), BaseAxis::Horizontal);
    // The baseline is a y coordinate in horizontal text and an x coordinate
    // in vertical text, so use the ppem and scale of that coordinate axis.
    let ppem = if horizontal { font.y_ppem } else { font.x_ppem };
    // OpenType device tables address a 16-bit ppem. A larger setting cannot
    // select an entry, but the design-unit coordinate remains available.
    let device_ppem = u16::try_from(ppem).unwrap_or(0);
    let value = base.baseline_for_ppem(
        tag_to_rust(baseline_tag),
        axis(direction),
        tag_to_rust(script_tag),
        device_ppem,
    )?;
    let scale = if horizontal {
        font.y_scale
    } else {
        font.x_scale
    };
    let scaled = scale_metric(value.value, scale, font.upem());
    let device = if device_ppem == 0 {
        0
    } else {
        (f64::from(value.delta_px) * f64::from(scale) / f64::from(device_ppem)).round() as i32
    };
    Some(scaled.saturating_add(device))
}

/// Reads a baseline from `BASE`. The language tag is currently unused.
/// Returns false without changing `coord` when the font has no such baseline.
///
/// # Safety
/// `font` must be `NULL` or a live font, and `coord` must be `NULL` or writable.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_get_baseline(
    font: *mut hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script_tag: hr_tag_t,
    _language_tag: hr_tag_t,
    coord: *mut hr_position_t,
) -> hr_bool_t {
    let font = unsafe { object::or_empty(font.cast_const()) };
    let Some(value) = baseline(font, baseline_tag, direction, script_tag) else {
        return 0;
    };
    if let Some(coord) = unsafe { coord.as_mut() } {
        *coord = value;
    }
    1
}

fn has_base_script(font: &hr_font_t, direction: hr_direction_t, tag: Tag) -> bool {
    font.instance()
        .and_then(|instance| instance.tables().base().ok())
        .and_then(|base| base.axis(axis(direction)))
        .and_then(|axis| axis.base_script_list().ok())
        .is_some_and(|list| {
            list.base_script_records()
                .iter()
                .any(|record| record.base_script_tag() == tag)
        })
}

fn selected_base_script(font: &hr_font_t, direction: hr_direction_t, script: hr_script_t) -> Tag {
    let Some(script) = script_to_rust(script) else {
        return Tag::from_u32(0);
    };
    let tags = script.tags();
    tags.as_slice()
        .iter()
        .copied()
        .find(|tag| has_base_script(font, direction, *tag))
        .unwrap_or_else(|| tags.as_slice().first().copied().unwrap_or(Tag::from_u32(0)))
}

/// Reads a baseline using a Unicode script instead of an OpenType script tag.
///
/// # Safety
/// As for [`hr_ot_layout_get_baseline`].
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_get_baseline2(
    font: *mut hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script: hr_script_t,
    _language: hr_language_t,
    coord: *mut hr_position_t,
) -> hr_bool_t {
    let state = unsafe { object::or_empty(font.cast_const()) };
    let selected = selected_base_script(state, direction, script);
    if let Some(value) = baseline(state, baseline_tag, direction, tag_from_rust(selected)) {
        if let Some(coord) = unsafe { coord.as_mut() } {
            *coord = value;
        }
        1
    } else {
        0
    }
}

fn extents(font: &hr_font_t, direction: hr_direction_t) -> (i32, i32) {
    let horizontal = matches!(axis(direction), BaseAxis::Horizontal);
    let Some(instance) = font.instance() else {
        return (0, 0);
    };
    if horizontal {
        instance.metrics().hhea_line.map_or((0, 0), |line| {
            (
                scale_metric(line.ascender, font.y_scale, font.upem()),
                scale_metric(line.descender, font.y_scale, font.upem()),
            )
        })
    } else {
        instance
            .metrics()
            .vhea_line
            .map_or((font.x_scale / 2, -font.x_scale / 2), |line| {
                (
                    scale_metric(line.ascender, font.x_scale, font.upem()),
                    scale_metric(line.descender, font.x_scale, font.upem()),
                )
            })
    }
}

unsafe fn glyph_top(font: *mut hr_font_t, ch: u32) -> Option<hr_glyph_extents_t> {
    let mut glyph = 0;
    if unsafe { hr_font_get_nominal_glyph(font, ch, &raw mut glyph) } == 0 {
        return None;
    }
    let mut extents = hr_glyph_extents_t::default();
    if unsafe { hr_font_get_glyph_extents(font, glyph, &raw mut extents) } == 0 {
        return None;
    }
    Some(extents)
}

/// Synthesizes a baseline when `BASE` does not provide one.
unsafe fn baseline_with_fallback(
    font_ptr: *mut hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script_tag: hr_tag_t,
) -> hr_position_t {
    let font = unsafe { object::or_empty(font_ptr.cast_const()) };
    if let Some(value) = baseline(font, baseline_tag, direction, script_tag) {
        return value;
    }
    let horizontal = matches!(axis(direction), BaseAxis::Horizontal);
    let span = if horizontal {
        font.y_scale
    } else {
        font.x_scale
    };
    match baseline_tag {
        HR_OT_LAYOUT_BASELINE_TAG_ROMAN => 0,
        HR_OT_LAYOUT_BASELINE_TAG_MATH => {
            if horizontal {
                if let Some(extents) = unsafe {
                    glyph_top(font_ptr, 0x2212).or_else(|| glyph_top(font_ptr, b'-' as u32))
                } {
                    return extents.y_bearing.saturating_add(extents.height / 2);
                }
            }
            let x_height = font
                .instance()
                .and_then(|instance| instance.metrics().x_height)
                .map_or(font.y_scale / 2, |height| {
                    scale_metric(height, font.y_scale, font.upem())
                });
            x_height / 2
        }
        HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT => {
            if let Some(bottom) = baseline(
                font,
                HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT,
                direction,
                script_tag,
            ) {
                bottom.saturating_add(span)
            } else {
                extents(font, direction).0
            }
        }
        HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT => {
            if let Some(top) = baseline(
                font,
                HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT,
                direction,
                script_tag,
            ) {
                top.saturating_sub(span)
            } else {
                extents(font, direction).1
            }
        }
        HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT
        | HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT => {
            let top = unsafe {
                baseline_with_fallback(
                    font_ptr,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT,
                    direction,
                    script_tag,
                )
            };
            let bottom = unsafe {
                baseline_with_fallback(
                    font_ptr,
                    HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT,
                    direction,
                    script_tag,
                )
            };
            if baseline_tag == HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT {
                top.saturating_add(
                    (i64::from(bottom) - i64::from(top)).clamp(i32::MIN as i64, i32::MAX as i64)
                        as i32
                        / 10,
                )
            } else {
                bottom.saturating_add(
                    (i64::from(top) - i64::from(bottom)).clamp(i32::MIN as i64, i32::MAX as i64)
                        as i32
                        / 10,
                )
            }
        }
        HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_CENTRAL
        | HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_CENTRAL => {
            let (top_tag, bottom_tag) =
                if baseline_tag == HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_CENTRAL {
                    (
                        HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT,
                        HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT,
                    )
                } else {
                    (
                        HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT,
                        HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT,
                    )
                };
            let top = unsafe { baseline_with_fallback(font_ptr, top_tag, direction, script_tag) };
            let bottom =
                unsafe { baseline_with_fallback(font_ptr, bottom_tag, direction, script_tag) };
            ((i64::from(top) + i64::from(bottom)) / 2) as i32
        }
        HR_OT_LAYOUT_BASELINE_TAG_HANGING => {
            let ch = match script_tag.to_be_bytes() {
                [b'B', b'e', b'n', b'g'] => 0x0995,
                [b'D', b'e', b'v', b'a'] => 0x0915,
                [b'G', b'u', b'j', b'r'] => 0x0a95,
                [b'G', b'u', b'r', b'u'] => 0x0a15,
                [b'T', b'i', b'b', b't'] => 0x0f40,
                _ => 0,
            };
            if horizontal && ch != 0 {
                if let Some(extents) = unsafe { glyph_top(font_ptr, ch) } {
                    return extents.y_bearing;
                }
            }
            span.saturating_mul(6) / 10
        }
        _ => 0,
    }
}

/// Reads a baseline or synthesizes one when absent from `BASE`.
///
/// # Safety
/// `font` must be `NULL` or a live font; `coord` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_get_baseline_with_fallback(
    font: *mut hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script_tag: hr_tag_t,
    _language_tag: hr_tag_t,
    coord: *mut hr_position_t,
) {
    if let Some(coord) = unsafe { coord.as_mut() } {
        *coord = unsafe { baseline_with_fallback(font, baseline_tag, direction, script_tag) };
    }
}

/// Reads or synthesizes a baseline using a Unicode script.
///
/// # Safety
/// As for [`hr_ot_layout_get_baseline_with_fallback`].
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_get_baseline_with_fallback2(
    font: *mut hr_font_t,
    baseline_tag: hr_ot_layout_baseline_tag_t,
    direction: hr_direction_t,
    script: hr_script_t,
    _language: hr_language_t,
    coord: *mut hr_position_t,
) {
    let Some(coord) = (unsafe { coord.as_mut() }) else {
        return;
    };
    let state = unsafe { object::or_empty(font.cast_const()) };
    let selected = selected_base_script(state, direction, script);
    if let Some(value) = baseline(state, baseline_tag, direction, tag_from_rust(selected)) {
        *coord = value;
        return;
    }
    *coord = unsafe { baseline_with_fallback(font, baseline_tag, direction, script) };
}
