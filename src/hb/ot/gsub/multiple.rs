use crate::hb::buffer::GlyphPropsFlags;
use crate::hb::ot_layout_gsubgpos::OT::hb_ot_apply_context_t;
use crate::hb::ot_layout_gsubgpos::{Apply, WouldApply, WouldApplyContext};
use read_fonts::tables::gsub::MultipleSubstFormat1;

impl WouldApply for MultipleSubstFormat1<'_> {
    fn would_apply(&self, ctx: &WouldApplyContext) -> bool {
        ctx.glyphs.len() == 1
            && self
                .coverage()
                .is_ok_and(|cov| cov.get(ctx.glyphs[0]).is_some())
    }
}

impl Apply for MultipleSubstFormat1<'_> {
    fn apply(&self, ctx: &mut hb_ot_apply_context_t) -> Option<()> {
        let gid = ctx.buffer.cur(0).as_glyph();
        let index = self.coverage().ok()?.get(gid)? as usize;
        let substs = self.sequences().get(index).ok()?.substitute_glyph_ids();
        match substs.len() {
            // Spec disallows this, but Uniscribe allows it.
            // https://github.com/harfbuzz/harfbuzz/issues/253
            0 => {
                message_sync!(
                    ctx,
                    "deleting glyph at {} (multiple substitution)",
                    ctx.buffer.idx
                );
                ctx.buffer.delete_glyph();
                message!(
                    ctx,
                    "deleted glyph at {} (multiple substitution)",
                    ctx.buffer.idx,
                );
            }

            // Special-case to make it in-place and not consider this
            // as a "multiplied" substitution.
            1 => {
                message_sync!(
                    ctx,
                    "replacing glyph at {} (multiple substitution)",
                    ctx.buffer.idx
                );
                ctx.replace_glyph(substs.first()?.get().into());
                message!(
                    ctx,
                    "replaced glyph at {} (multiple substitution)",
                    ctx.buffer.idx - 1,
                );
            }

            _ => {
                message_sync!(ctx, "multiplying glyph at {}", ctx.buffer.idx);
                let class = if ctx.buffer.cur(0).is_ligature() {
                    GlyphPropsFlags::BASE_GLYPH
                } else {
                    GlyphPropsFlags::empty()
                };
                let lig_id = ctx.buffer.cur(0).lig_id();

                for (i, subst) in substs.iter().enumerate() {
                    let subst = subst.get().into();
                    // If is attached to a ligature, don't disturb that.
                    // https://github.com/harfbuzz/harfbuzz/issues/3069
                    if lig_id == 0 {
                        // Index is truncated to 4 bits anway, so we can safely cast to u8.
                        ctx.buffer.cur_mut(0).set_lig_props_for_component(i as u8);
                    }
                    ctx.output_glyph_for_component(subst, class);
                }

                ctx.buffer.skip_glyph();

                if ctx.buffer.messaging() {
                    ctx.buffer.sync_so_far();
                    let count = substs.len();
                    let mut msg = "Multiplied glyphs at ".to_string();
                    for i in (ctx.buffer.idx - count)..=ctx.buffer.idx {
                        if i > (ctx.buffer.idx - count) {
                            msg.push(',');
                        }
                        msg.push_str(i.to_string().as_str());
                    }
                    ctx.buffer.message(ctx.face, &msg);
                }
            }
        }
        Some(())
    }
}
