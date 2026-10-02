//! OpenType GSUB lookups.

mod alternate;
mod ligature;
pub(crate) use ligature::collect_seconds;
mod multiple;
mod reverse_chain;
mod single;

use crate::buffer::Buffer;
use crate::font_funcs::FontFuncsDispatch;
use crate::hb_font_t;
use crate::ot::layout::*;
use crate::ot::shape::plan::hb_ot_shape_plan_t;

pub fn substitute(
    plan: &hb_ot_shape_plan_t,
    face: &hb_font_t,
    font_funcs: &mut FontFuncsDispatch,
    buffer: &mut Buffer,
) {
    apply_layout_table(plan, face, font_funcs, buffer, face.ot_tables.gsub.as_ref());
}
