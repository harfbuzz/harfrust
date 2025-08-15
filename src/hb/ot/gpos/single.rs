use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::{ot::gpos::apply_value_to_pos, ot_layout_gsubgpos::Apply};
use read_fonts::tables::gpos::{SinglePosFormat1, SinglePosFormat2, Value};

impl Apply for SinglePosFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        self.coverage().ok()?.get(glyph)?;
        let value = Value::read(
            self.offset_data(),
            self.shape().value_record_byte_range().start,
            self.value_format(),
            &ctx.face.ot_tables.value_context,
        )
        .ok()?;
        apply_value_to_pos(ctx, ctx.buffer.idx, &value);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl Apply for SinglePosFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().ok()?.get(glyph)? as usize;
        let format = self.value_format();
        let format_len = format.record_byte_len();
        let offset = self.shape().value_records_byte_range().start + index * format_len;
        let value = Value::read(
            self.offset_data(),
            offset,
            format,
            &ctx.face.ot_tables.value_context,
        )
        .ok()?;
        apply_value_to_pos(ctx, ctx.buffer.idx, &value);
        ctx.buffer.idx += 1;
        Some(())
    }
}
