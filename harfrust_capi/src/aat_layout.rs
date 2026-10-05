//! Queries on the Apple Advanced Typography `feat` table.

use core::ffi::c_uint;

use read_fonts::TableProvider;

use crate::face::hr_face_t;
use crate::object;

pub type hr_aat_layout_feature_type_t = c_uint;
pub type hr_aat_layout_feature_selector_t = c_uint;

pub const HR_AAT_LAYOUT_NO_SELECTOR_INDEX: c_uint = 0xFFFF;
pub const HR_AAT_LAYOUT_FEATURE_TYPE_LETTER_CASE: hr_aat_layout_feature_type_t = 3;
pub const HR_AAT_LAYOUT_FEATURE_TYPE_LOWER_CASE: hr_aat_layout_feature_type_t = 37;
pub const HR_AAT_LAYOUT_FEATURE_TYPE_UPPER_CASE: hr_aat_layout_feature_type_t = 38;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_INVALID: hr_aat_layout_feature_selector_t = 0xFFFF;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_SMALL_CAPS: hr_aat_layout_feature_selector_t = 3;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_DEFAULT_LOWER_CASE: hr_aat_layout_feature_selector_t = 0;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_LOWER_CASE_SMALL_CAPS: hr_aat_layout_feature_selector_t =
    1;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_LOWER_CASE_PETITE_CAPS: hr_aat_layout_feature_selector_t =
    2;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_DEFAULT_UPPER_CASE: hr_aat_layout_feature_selector_t = 0;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_UPPER_CASE_SMALL_CAPS: hr_aat_layout_feature_selector_t =
    1;
pub const HR_AAT_LAYOUT_FEATURE_SELECTOR_UPPER_CASE_PETITE_CAPS: hr_aat_layout_feature_selector_t =
    2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct hr_aat_layout_feature_selector_info_t {
    pub name_id: c_uint,
    pub enable: hr_aat_layout_feature_selector_t,
    pub disable: hr_aat_layout_feature_selector_t,
    pub reserved: c_uint,
}

/// Enumerates feature types in the AAT `feat` table.
///
/// # Safety
/// `face` must be NULL or live; `feature_count` must be writable when non-NULL,
/// and `features` must have its input capacity when non-NULL.
#[no_mangle]
pub unsafe extern "C" fn hr_aat_layout_get_feature_types(
    face: *mut hr_face_t,
    start_offset: c_uint,
    feature_count: *mut c_uint,
    features: *mut hr_aat_layout_feature_type_t,
) -> c_uint {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let names = face.font().and_then(|font| font.tables().feat().ok());
    let records = names.as_ref().map_or(&[][..], |feat| feat.names());
    let total = records.len() as c_uint;
    if let Some(count) = unsafe { feature_count.as_mut() } {
        let start = (start_offset as usize).min(records.len());
        let written = (*count as usize).min(records.len() - start);
        if !features.is_null() {
            for (index, record) in records[start..start + written].iter().enumerate() {
                unsafe { *features.add(index) = c_uint::from(record.feature()) };
            }
        }
        *count = written as c_uint;
    }
    total
}

/// Enumerates selectors for an AAT feature type.
///
/// # Safety
/// `face` must be NULL or live; output pointers must be writable when non-NULL,
/// and `selectors` must have `selector_count` entries of capacity when non-NULL.
#[no_mangle]
pub unsafe extern "C" fn hr_aat_layout_feature_type_get_selector_infos(
    face: *mut hr_face_t,
    feature_type: hr_aat_layout_feature_type_t,
    start_offset: c_uint,
    selector_count: *mut c_uint,
    selectors: *mut hr_aat_layout_feature_selector_info_t,
    default_index: *mut c_uint,
) -> c_uint {
    let face = unsafe { object::or_empty(face.cast_const()) };
    let feat = face.font().and_then(|font| font.tables().feat().ok());
    let feature = feat.as_ref().and_then(|feat| {
        u16::try_from(feature_type)
            .ok()
            .and_then(|feature_type| feat.find(feature_type))
    });
    let default = feature
        .filter(|feature| feature.is_exclusive())
        .map_or(HR_AAT_LAYOUT_NO_SELECTOR_INDEX, |feature| {
            c_uint::from(feature.default_setting_index())
        });
    if let Some(index) = unsafe { default_index.as_mut() } {
        *index = default;
    }
    let settings = feat
        .as_ref()
        .zip(feature)
        .and_then(|(feat, feature)| feature.setting_table(feat.offset_data()).ok());
    let records = settings
        .as_ref()
        .map_or(&[][..], |settings| settings.settings());
    let total = records.len() as c_uint;
    if let Some(count) = unsafe { selector_count.as_mut() } {
        let start = (start_offset as usize).min(records.len());
        let written = (*count as usize).min(records.len() - start);
        if !selectors.is_null() {
            let default_selector = records
                .get(default as usize)
                .map(|setting| c_uint::from(setting.setting()));
            for (index, setting) in records[start..start + written].iter().enumerate() {
                let enable = c_uint::from(setting.setting());
                unsafe {
                    *selectors.add(index) = hr_aat_layout_feature_selector_info_t {
                        name_id: c_uint::from(setting.name_index().to_u16()),
                        enable,
                        disable: default_selector.unwrap_or(enable + 1),
                        reserved: 0,
                    }
                };
            }
        }
        *count = written as c_uint;
    }
    total
}
