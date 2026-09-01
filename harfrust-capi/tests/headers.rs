//! Checks on the two generated headers, both of which are committed and so
//! can fall behind the source they are generated from.

const HR_H: &str = include_str!("../include/hr.h");
const HR_HB_H: &str = include_str!("../include/hr-hb.h");

/// Strips C comments, so that prose mentioning a name does not count as
/// declaring one.
fn without_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            match source[i + 2..].find("*/") {
                Some(end) => i += 2 + end + 2,
                None => break,
            }
            out.push(' ');
        } else if bytes[i..].starts_with(b"//") {
            match source[i..].find('\n') {
                Some(end) => i += end,
                None => break,
            }
            out.push(' ');
        } else {
            out.push(source[i..].chars().next().unwrap());
            i += source[i..].chars().next().unwrap().len_utf8();
        }
    }
    out
}

/// Returns every `hr_` and `HR_` identifier declared in `source`.
fn declared_names(source: &str) -> Vec<String> {
    let stripped = without_comments(source);
    let mut names = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            // Only a whole word starting the identifier counts, so `foo_hr_bar`
            // is not mistaken for one.
            let word = &stripped[start..i];
            let preceded_by_word =
                start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
            if !preceded_by_word && (word.starts_with("hr_") || word.starts_with("HR_")) {
                names.push(word.to_string());
            }
        } else {
            i += 1;
        }
    }
    names.sort();
    names.dedup();
    names
}

#[test]
fn the_compatibility_header_covers_every_name() {
    let missing: Vec<String> = declared_names(HR_H)
        .into_iter()
        .filter(|name| {
            let hb = match (name.strip_prefix("hr_"), name.strip_prefix("HR_")) {
                (Some(rest), _) => format!("hb_{rest}"),
                (_, Some(rest)) => format!("HB_{rest}"),
                _ => unreachable!("only hr_ and HR_ names are collected"),
            };
            // The generator writes one `#define` per name, padded to a column.
            !HR_HB_H
                .lines()
                .any(|line| line.starts_with(&format!("#define {hb} ")))
        })
        .collect();

    assert!(
        missing.is_empty(),
        "include/hr-hb.h is out of date, missing {} name(s): {}. \
         Regenerate it with scripts/gen-hb-compat-header.py.",
        missing.len(),
        missing.join(", ")
    );
}

#[test]
fn the_compatibility_header_maps_nothing_that_does_not_exist() {
    let declared = declared_names(HR_H);
    let stray: Vec<&str> = HR_HB_H
        .lines()
        .filter_map(|line| line.strip_prefix("#define "))
        .filter_map(|rest| rest.split_whitespace().nth(1))
        .filter(|target| !declared.iter().any(|name| name == target))
        .collect();

    assert!(
        stray.is_empty(),
        "include/hr-hb.h maps to {} name(s) that hr.h no longer declares: {}. \
         Regenerate it with scripts/gen-hb-compat-header.py.",
        stray.len(),
        stray.join(", ")
    );
}

#[test]
fn the_compatibility_header_maps_the_names_callers_reach_for_first() {
    // A spot check, so that a wholesale failure of the generator is obvious
    // rather than showing up only as an empty diff.
    for expected in [
        "#define hb_shape ",
        "#define hb_shape_full ",
        "#define hb_buffer_create ",
        "#define hb_buffer_add_utf8 ",
        "#define hb_face_create ",
        "#define hb_font_create ",
        "#define hb_blob_create_from_file ",
        "#define hb_glyph_info_t ",
        "#define hb_glyph_position_t ",
        "#define hb_segment_properties_t ",
        "#define hb_shape_plan_create_cached ",
        "#define HB_TAG ",
        "#define HB_SCRIPT_LATIN ",
        "#define HB_DIRECTION_LTR ",
        "#define HB_DIRECTION_IS_HORIZONTAL ",
        "#define HB_FEATURE_GLOBAL_START ",
        "#define HB_FEATURE_GLOBAL_END ",
        "#define HB_SEGMENT_PROPERTIES_DEFAULT ",
        "#define HB_VERSION_ATLEAST ",
    ] {
        assert!(
            HR_HB_H.lines().any(|line| line.starts_with(expected)),
            "include/hr-hb.h is missing `{}`",
            expected.trim()
        );
    }
}
