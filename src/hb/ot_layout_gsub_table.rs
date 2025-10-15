use super::buffer::hb_buffer_t;
use super::hb_font_t;
use super::ot_layout::*;
use super::ot_shape_plan::hb_ot_shape_plan_t;

pub fn substitute(plan: &hb_ot_shape_plan_t, face: &hb_font_t, buffer: &mut hb_buffer_t) {
    #[cfg(feature = "std")]
    let tag = plan
        .ot_map
        .chosen_script(TableIndex::GSUB)
        .map_or("none".to_string(), |x| x.to_string());
    #[cfg(feature = "std")]
    if !buffer.message(face, &format!("start table GSUB script tag '{tag}'")) {
        return;
    }
    apply_layout_table(plan, face, buffer, face.ot_tables.gsub.as_ref());
    #[cfg(feature = "std")]
    buffer.message(face, &format!("end table GSUB script tag '{tag}'"));
}
