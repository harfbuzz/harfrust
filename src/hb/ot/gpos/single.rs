use super::Value;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{Apply, ApplyState};
use read_fonts::tables::gpos::{SinglePosFormat1, SinglePosFormat2};

impl Apply for SinglePosFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, _state: &ApplyState) -> Option<()> {
        let record = self.value_record();
        let value = Value {
            record,
            data: self.offset_data(),
        };
        value.apply(ctx, ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}

impl Apply for SinglePosFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, state: &ApplyState) -> Option<()> {
        let record = self.value_records().get(state.index).ok()?;
        let value = Value {
            record,
            data: self.offset_data(),
        };
        value.apply(ctx, ctx.buffer.idx);
        ctx.buffer.idx += 1;
        Some(())
    }
}
