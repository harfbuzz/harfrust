//! OpenType GSUB lookups.

mod alternate;
mod ligature;
pub(crate) use ligature::collect_seconds;
mod multiple;
mod reverse_chain;
mod single;

use crate::buffer::Buffer;
use crate::ot::layout::*;
use crate::shape::plan::ShapePlan;
use crate::shape::ShaperFont;

pub fn substitute(plan: &ShapePlan, font: &ShaperFont<'_, '_>, buffer: &mut Buffer) {
    let table = font.layout.ot.gsub.clone();
    apply_layout_table(plan, font, buffer, table.as_ref());
}
