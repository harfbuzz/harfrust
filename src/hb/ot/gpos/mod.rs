//! OpenType GPOS lookups.

use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use read_fonts::tables::gpos::{Value, ValueFormat};

mod cursive;
mod mark;
mod pair;
mod single;

fn apply_value_to_pos(ctx: &mut hb_ot_apply_context_t, idx: usize, value: &Value) -> bool {
    let pos = &mut ctx.buffer.pos[idx];
    let is_horizontal = ctx.buffer.direction.is_horizontal();
    pos.x_offset += value.x_placement as i32 + value.x_placement_delta;
    pos.y_offset += value.y_placement as i32 + value.y_placement_delta;
    let advance = if is_horizontal {
        pos.x_advance += value.x_advance as i32 + value.x_advance_delta;
        value.x_advance
    } else {
        pos.y_advance -= value.y_advance as i32 + value.y_advance_delta;
        value.y_advance
    };
    ((value.x_placement | value.y_placement | advance) != 0)
        | value.format.contains(ValueFormat::ANY_DEVICE_OR_VARIDX)
}
