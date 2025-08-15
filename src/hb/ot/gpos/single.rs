use super::ValueResolver;
use crate::hb::ot_layout_gsubgpos::Apply;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use read_fonts::tables::gpos::{SinglePosFormat1, SinglePosFormat2};

impl Apply for SinglePosFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        self.coverage().ok()?.get(glyph)?;
        let record = self.value_record();
        let value = ValueResolver {
            record,
            data: self.offset_data(),
        };
        value.apply(ctx, ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl Apply for SinglePosFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().ok()?.get(glyph)? as usize;
        let record = self.value_records().get(index).ok()?;
        let value = ValueResolver {
            record,
            data: self.offset_data(),
        };
        value.apply(ctx, ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}
