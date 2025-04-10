use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{Apply, WouldApply, WouldApplyContext};
use skrifa::raw::tables::gsub::{SingleSubstFormat1, SingleSubstFormat2};
use ttf_parser::GlyphId;

impl WouldApply for SingleSubstFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        let gid = skrifa::GlyphId::from(ctx.glyphs[0].0);
        ctx.glyphs.len() == 1
            && self
                .coverage()
                .map(|cov| cov.get(gid).is_some())
                .unwrap_or_default()
    }
}

impl Apply for SingleSubstFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_skrifa_glyph16();
        self.coverage().ok()?.get(glyph)?;
        let subst = (glyph.to_u16() as i32 + self.delta_glyph_id() as i32) as u16;
        ctx.replace_glyph(GlyphId(subst));
        Some(())
    }
}

impl WouldApply for SingleSubstFormat2<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        ctx.glyphs.len() == 1
            && self
                .coverage()
                .map(|cov| cov.get(skrifa::GlyphId::from(ctx.glyphs[0].0)).is_some())
                .unwrap_or_default()
    }
}

impl Apply for SingleSubstFormat2<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let glyph = ctx.buffer.cur(0).as_skrifa_glyph();
        let index = self.coverage().ok()?.get(glyph)? as usize;
        let subst = self.substitute_glyph_ids().get(index)?.get().to_u16();
        ctx.replace_glyph(GlyphId(subst));
        Some(())
    }
}
