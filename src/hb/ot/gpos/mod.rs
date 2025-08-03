//! OpenType GPOS lookups.

use crate::{hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t, GlyphPosition};
use read_fonts::{
    tables::{
        gpos::{DeviceOrVariationIndex, ValueFormat, ValueRecord},
        variations::DeltaSetIndex,
    },
    FontData, ReadError,
};

mod cursive;
mod mark;
mod pair;
mod single;

pub(crate) use pair::{apply_pair_pos1, apply_pair_pos2};

struct ValueReader<'a> {
    data: FontData<'a>,
    parent_offset: usize,
    offset: usize,
    format: ValueFormat,
}

impl<'a> ValueReader<'a> {
    pub fn new(
        data: FontData<'a>,
        parent_offset: usize,
        offset: usize,
        format: ValueFormat,
    ) -> Self {
        Self {
            data,
            parent_offset,
            offset,
            format,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.format.is_empty()
    }

    fn apply(&self, ctx: &mut hb_ot_apply_context_t, idx: usize) -> bool {
        let mut pos = ctx.buffer.pos[idx];
        let worked = self.apply_to_pos(ctx, &mut pos);
        ctx.buffer.pos[idx] = pos;
        worked == Some(true)
    }

    fn apply_to_pos(
        &self,
        ctx: &mut hb_ot_apply_context_t,
        pos: &mut GlyphPosition,
    ) -> Option<bool> {
        let horizontal = ctx.buffer.direction.is_horizontal();
        let mut worked = false;
        let mut offset = self.offset;
        if self.format.contains(ValueFormat::X_PLACEMENT) {
            let value = self.data.read_at::<i16>(offset).ok()?;
            offset += 2;
            if value != 0 {
                pos.x_offset += value as i32;
                worked = true;
            }
        }
        if self.format.contains(ValueFormat::Y_PLACEMENT) {
            let value = self.data.read_at::<i16>(offset).ok()?;
            offset += 2;
            if value != 0 {
                pos.y_offset += value as i32;
                worked = true;
            }
        }
        if self.format.contains(ValueFormat::X_ADVANCE) {
            if horizontal {
                let value = self.data.read_at::<i16>(offset).ok()?;
                if value != 0 {
                    pos.x_advance += value as i32;
                    worked = true;
                }
            }
            offset += 2;
        }
        if self.format.contains(ValueFormat::Y_ADVANCE) {
            if !horizontal {
                let value = self.data.read_at::<i16>(offset).ok()?;
                if value != 0 {
                    // y_advance values grow downward but font-space grows upward, hence negation
                    pos.y_advance -= value as i32;
                    worked = true;
                }
            }
            offset += 2;
        }
        if let (false, Some(vs)) = (
            ctx.face.ot_tables.coords.is_empty(),
            ctx.face.ot_tables.var_store.as_ref(),
        ) {
            let coords = ctx.face.ot_tables.coords;
            let delta = |offset: usize| {
                let rec_offset =
                    self.parent_offset + self.data.read_at::<u16>(offset).ok()? as usize;
                let format = self.data.read_at::<u16>(rec_offset + 4).ok()?;
                if format != 0x8000 {
                    return Some(0);
                }
                let outer = self.data.read_at::<u16>(rec_offset).ok()?;
                let inner = self.data.read_at::<u16>(rec_offset + 2).ok()?;
                vs.compute_delta(DeltaSetIndex { outer, inner }, coords)
                    .ok()
            };
            if self.format.contains(ValueFormat::X_PLACEMENT_DEVICE) {
                pos.x_offset += delta(offset).unwrap_or_default();
                offset += 2;
                worked = true;
            }
            if self.format.contains(ValueFormat::Y_PLACEMENT_DEVICE) {
                pos.y_offset += delta(offset).unwrap_or_default();
                offset += 2;
                worked = true;
            }
            if self.format.contains(ValueFormat::X_ADVANCE_DEVICE) {
                if horizontal {
                    pos.x_advance += delta(offset).unwrap_or_default();
                    worked = true;
                }
                offset += 2;
            }
            if self.format.contains(ValueFormat::Y_ADVANCE_DEVICE) {
                if !horizontal {
                    // y_advance values grow downward but face-space grows upward, hence negation
                    pos.y_advance -= delta(offset).unwrap_or_default();
                    worked = true;
                }
            }
        }
        Some(worked)
    }
}

struct Value<'a> {
    record: ValueRecord,
    data: FontData<'a>,
}

impl Value<'_> {
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
