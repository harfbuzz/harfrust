use crate::hb::ot_layout_gsubgpos::Apply;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use read_fonts::tables::gpos::{SinglePosFormat1Sanitized, SinglePosFormat2Sanitized};

impl Apply for SinglePosFormat1Sanitized<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        self.coverage().get(glyph)?;
        let format = self.value_format();
        let data = self.value_record().offset_ptr().into_font_data();
        super::apply_value(ctx, ctx.buffer.idx, &data, 0, format);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl Apply for SinglePosFormat2Sanitized<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().get(glyph)? as usize;
        let format = self.value_format();
        let record = self.value_records().get(index);
        let data = record.offset_ptr().into_font_data();
        super::apply_value(ctx, ctx.buffer.idx, &data, 0, format);
        ctx.buffer.idx += 1;
        Some(())
    }
}
