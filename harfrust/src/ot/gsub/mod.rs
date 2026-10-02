//! OpenType GSUB lookups.

mod alternate;
mod ligature;
pub(crate) use ligature::collect_seconds;
mod multiple;
mod reverse_chain;
mod single;

use crate::buffer::Buffer;
use crate::font_funcs::FontFuncsDispatch;
use crate::ot::layout::*;
use crate::ot::shape::plan::ShapePlan;
use crate::Shaper;

pub fn substitute(
    plan: &ShapePlan,
    face: &Shaper,
    font_funcs: &mut FontFuncsDispatch,
    buffer: &mut Buffer,
) {
    apply_layout_table(plan, face, font_funcs, buffer, face.ot_tables.gsub.as_ref());
}
