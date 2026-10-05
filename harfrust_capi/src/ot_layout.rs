//! OpenType layout and BASE queries.

use core::ffi::c_uint;

use harfrust::Tag;
use read_fonts::tables::base::{BaseAxis, BaseInstance};
use read_fonts::tables::layout::{FeatureList, ScriptList};
use read_fonts::types::F48Dot16;
use read_fonts::TableProvider;

use crate::common::hr_glyph_extents_t;
use crate::common::{
    hr_bool_t, hr_direction_t, hr_language_t, hr_position_t, hr_script_t, hr_tag_t, script_to_rust,
    tag_from_rust, tag_to_rust, HR_DIRECTION_LTR, HR_DIRECTION_RTL,
};
use crate::face::hr_face_t;
use crate::font::{hr_font_get_glyph_extents, hr_font_get_nominal_glyph, hr_font_t};
use crate::object;

const GSUB_TAG: hr_tag_t = 0x4753_5542;
const GPOS_TAG: hr_tag_t = 0x4750_4F53;
const NO_INDEX: c_uint = 0xFFFF;

pub const HR_OT_TAG_GSUB: hr_tag_t = 0x4753_5542;
pub const HR_OT_TAG_GPOS: hr_tag_t = 0x4750_4F53;
pub const HR_OT_LAYOUT_NO_SCRIPT_INDEX: c_uint = 0xFFFF;
pub const HR_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX: c_uint = 0xFFFF;
pub const HR_OT_LAYOUT_NO_FEATURE_INDEX: c_uint = 0xFFFF;

fn script_list(face: &hr_face_t, table_tag: hr_tag_t) -> Option<ScriptList<'_>> {
    let font = face.font()?;
    match table_tag {
        GSUB_TAG => font.tables().gsub().ok()?.script_list().ok(),
        GPOS_TAG => font.tables().gpos().ok()?.script_list().ok(),
        _ => None,
    }
}

fn layout_lists(
    face: &hr_face_t,
    table_tag: hr_tag_t,
) -> Option<(ScriptList<'_>, FeatureList<'_>)> {
    let font = face.font()?;
    match table_tag {
        GSUB_TAG => {
            let table = font.tables().gsub().ok()?;
            Some((table.script_list().ok()?, table.feature_list().ok()?))
        }
        GPOS_TAG => {
            let table = font.tables().gpos().ok()?;
            Some((table.script_list().ok()?, table.feature_list().ok()?))
        }
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

/// Selects an OpenType script, falling back to DFLT, dflt, or latn.
///
/// # Safety
/// `face` must be NULL or live. `script_tags` must contain `script_count` tags
/// when non-NULL; output pointers must be writable when non-NULL.
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
    let selected = script_list(face, table_tag).and_then(|list| {
        const BATCH_SIZE: usize = 16;
        let mut tags = [Tag::new(b"    "); BATCH_SIZE];
        let mut fallback = None;
        if !script_tags.is_null() {
            let mut offset = 0;
            while offset < script_count as usize {
                let count = (script_count as usize - offset).min(BATCH_SIZE);
                for (i, tag) in tags[..count].iter_mut().enumerate() {
                    *tag = tag_to_rust(unsafe { *script_tags.add(offset + i) });
                }
                if let Some(found) = list.select(&tags[..count]) {
                    if !found.is_fallback {
                        return Some(found);
                    }
                    fallback = Some(found);
                }
                offset += count;
            }
        }
        fallback.or_else(|| list.select(&[]))
    });
    if let Some(index) = unsafe { script_index.as_mut() } {
        *index = selected.map_or(NO_INDEX, |value| c_uint::from(value.index));
    }
    if let Some(tag) = unsafe { chosen_script.as_mut() } {
        *tag = selected.map_or(0, |value| tag_from_rust(value.tag));
    }
    selected.is_some_and(|value| !value.is_fallback).into()
}

/// Finds a feature in one script's language system.
/// Language index 0xFFFF selects its default language system.
///
/// # Safety
/// `face` must be NULL or live; `feature_index` must be writable when non-NULL.
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
    let found = layout_lists(face, table_tag).and_then(|(scripts, features)| {
        let script = scripts.get(u16::try_from(script_index).ok()?).ok()?;
        let language = if language_index == NO_INDEX {
            script.default_lang_sys()?.ok()?
        } else {
            script
                .lang_sys(u16::try_from(language_index).ok()?)
                .ok()?
                .element
        };
        language.feature_index_for_tag(&features, tag_to_rust(feature_tag))
    });
    if let Some(index) = unsafe { feature_index.as_mut() } {
        *index = found.map_or(NO_INDEX, c_uint::from);
    }
    found.is_some().into()
}

/// Enumerates feature tags in their original table order, including duplicates.
///
/// # Safety
/// `face` must be NULL or live; `feature_count` must be writable when non-NULL,
/// and `feature_tags` must have its input capacity when non-NULL.
#[no_mangle]
pub unsafe extern "C" fn hr_ot_layout_table_get_feature_tags(
    face: *mut hr_face_t,
    table_tag: hr_tag_t,
    start_offset: c_uint,
    feature_count: *mut c_uint,
    feature_tags: *mut hr_tag_t,
) -> c_uint {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let Some(features) = feature_list(face, table_tag) else {
        if let Some(count) = unsafe { feature_count.as_mut() } {
            *count = 0;
        }
        return 0;
    };
    let records = features.feature_records();
    let total = records.len() as c_uint;
    if let Some(count) = unsafe { feature_count.as_mut() } {
        let start = (start_offset as usize).min(records.len());
        let written = (*count as usize).min(records.len() - start);
        if !feature_tags.is_null() {
            for (i, record) in records[start..start + written].iter().enumerate() {
                unsafe { *feature_tags.add(i) = tag_from_rust(record.feature_tag()) };
            }
        }
        *count = written as c_uint;
    }
    total
}

/// A registered OpenType BASE baseline tag. The numeric value is the tag itself.
pub type hr_ot_layout_baseline_tag_t = hr_tag_t;

pub const HR_OT_LAYOUT_BASELINE_TAG_ROMAN: hr_ot_layout_baseline_tag_t = 0x726F_6D6E;
pub const HR_OT_LAYOUT_BASELINE_TAG_HANGING: hr_ot_layout_baseline_tag_t = 0x6861_6E67;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_BOTTOM_OR_LEFT: hr_ot_layout_baseline_tag_t =
    0x6963_6662;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_TOP_OR_RIGHT: hr_ot_layout_baseline_tag_t =
    0x6963_6674;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_FACE_CENTRAL: hr_ot_layout_baseline_tag_t = 0x4963_6663;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_BOTTOM_OR_LEFT: hr_ot_layout_baseline_tag_t =
    0x6964_656F;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_TOP_OR_RIGHT: hr_ot_layout_baseline_tag_t =
    0x6964_7470;
pub const HR_OT_LAYOUT_BASELINE_TAG_IDEO_EMBOX_CENTRAL: hr_ot_layout_baseline_tag_t = 0x4964_6365;
pub const HR_OT_LAYOUT_BASELINE_TAG_MATH: hr_ot_layout_baseline_tag_t = 0x6D61_7468;

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
