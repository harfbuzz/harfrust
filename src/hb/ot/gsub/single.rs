use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{Apply, ApplyState, WouldApply, WouldApplyContext};
use read_fonts::tables::gsub::{SingleSubstFormat1, SingleSubstFormat2};

impl WouldApply for SingleSubstFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let gid = ctx.glyphs[0];
        ctx.glyphs.len() == 1 && self.coverage().is_ok_and(|cov| cov.get(gid).is_some())
    }
}

impl Apply for SingleSubstFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, state: &ApplyState) -> Option<()> {
        let subst = (state.glyph.to_u32() as i32 + self.delta_glyph_id() as i32) as u16;
        ctx.replace_glyph(subst.into());
        Some(())
    }
}

impl WouldApply for SingleSubstFormat2<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        ctx.glyphs.len() == 1
            && self
                .coverage()
                .is_ok_and(|cov| cov.get(ctx.glyphs[0]).is_some())
    }
}

impl Apply for SingleSubstFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t, state: &ApplyState) -> Option<()> {
        let subst = self.substitute_glyph_ids().get(state.index)?.get().to_u16();
        ctx.replace_glyph(subst.into());
        Some(())
    }
}
