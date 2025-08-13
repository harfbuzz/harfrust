//! OpenType GPOS lookups.

use crate::{hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t, GlyphPosition};
use read_fonts::{
    tables::{
        gpos::{DeviceOrVariationIndex, Value, ValueFormat, ValueRecord},
        variations::DeltaSetIndex,
    },
    FontData, ReadError,
};

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

// TODO: remove me
struct ValueResolver<'a> {
    record: ValueRecord,
    data: FontData<'a>,
}

impl ValueResolver<'_> {
    fn is_empty(&self) -> bool {
        self.record.format.is_empty()
    }

    fn apply(&self, ctx: &mut hb_ot_apply_context_t, idx: usize) -> bool {
        let mut pos = ctx.buffer.pos[idx];
        let worked = self.apply_to_pos(ctx, &mut pos);
        ctx.buffer.pos[idx] = pos;
        worked
    }

    fn apply_to_pos(&self, ctx: &mut hb_ot_apply_context_t, pos: &mut GlyphPosition) -> bool {
        let horizontal = ctx.buffer.direction.is_horizontal();
        let mut worked = false;

        if let Some(value) = self.record.x_placement() {
            if value != 0 {
                pos.x_offset += i32::from(value);
                worked = true;
            }
        }

        if let Some(value) = self.record.y_placement() {
            if value != 0 {
                pos.y_offset += i32::from(value);
                worked = true;
            }
        }

        if horizontal {
            if let Some(value) = self.record.x_advance() {
                if value != 0 {
                    pos.x_advance += i32::from(value);
                    worked = true;
                }
            }
        } else {
            if let Some(value) = self.record.y_advance() {
                if value != 0 {
                    // y_advance values grow downward but font-space grows upward, hence negation
                    pos.y_advance -= i32::from(value);
                    worked = true;
                }
            }
        }

        if let Some(vs) = ctx.face.ot_tables.var_store.as_ref() {
            let coords = ctx.face.ot_tables.coords;
            let delta = |val: Result<DeviceOrVariationIndex<'_>, ReadError>| match val {
                Ok(DeviceOrVariationIndex::VariationIndex(varix)) => vs
                    .compute_delta(
                        DeltaSetIndex {
                            outer: varix.delta_set_outer_index(),
                            inner: varix.delta_set_inner_index(),
                        },
                        coords,
                    )
                    .unwrap_or_default(),
                _ => 0,
            };

            let (ppem_x, ppem_y) = ctx.face.pixels_per_em().unwrap_or((0, 0));
            let coords = coords.len();
            let use_x_device = ppem_x != 0 || coords != 0;
            let use_y_device = ppem_y != 0 || coords != 0;

            if use_x_device {
                if let Some(device) = self.record.x_placement_device(self.data) {
                    pos.x_offset += delta(device);
                    worked = true; // TODO: even when 0?
                }
            }

            if use_y_device {
                if let Some(device) = self.record.y_placement_device(self.data) {
                    pos.y_offset += delta(device);
                    worked = true;
                }
            }

            if horizontal && use_x_device {
                if let Some(device) = self.record.x_advance_device(self.data) {
                    pos.x_advance += delta(device);
                    worked = true;
                }
            }

            if !horizontal && use_y_device {
                if let Some(device) = self.record.y_advance_device(self.data) {
                    // y_advance values grow downward but face-space grows upward, hence negation
                    pos.y_advance -= delta(device);
                    worked = true;
                }
            }
        }

        worked
    }
}
