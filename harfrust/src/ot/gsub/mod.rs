//! OpenType GSUB lookups.

use crate::buffer::Buffer;
use crate::ot::layout::*;
use crate::plan::ShapePlan;
use crate::ShaperFont;

pub fn substitute(plan: &ShapePlan, font: &ShaperFont<'_, '_>, buffer: &mut Buffer) {
    let table = font.layout().ot.gsub.clone();
    apply_layout_table(plan, font, buffer, table.as_ref());
}
