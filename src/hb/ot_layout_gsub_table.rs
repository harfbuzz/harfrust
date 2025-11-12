use super::buffer::Buffer;
use super::hb_font_t;
use super::ot_layout::*;
use super::ot_shape_plan::hb_ot_shape_plan_t;

pub fn substitute(plan: &hb_ot_shape_plan_t, face: &hb_font_t, buffer: &mut Buffer) {
    apply_layout_table(plan, face, buffer, face.ot_tables.gsub.as_ref());
}
