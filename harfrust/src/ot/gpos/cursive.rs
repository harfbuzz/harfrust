use crate::buffer::HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
use crate::hint::{Likely, LikelyOk};
use crate::ot::apply::ApplyContext;
use crate::ot::apply::{Apply, SkippingIterator};
use crate::ot::gpos::attach_type;
use crate::ot::lookup_flags;
use crate::{Direction, GlyphPosition};
use read_fonts::tables::gpos::CursivePosFormat1;

impl Apply for CursivePosFormat1<'_> {
    fn apply(&self, ctx: &mut ApplyContext) -> Option<()> {
        let this = ctx.buffer.cur(0)?.as_glyph();

        let coverage = self.coverage().likely_ok()?;
        let index_this = coverage.get(this)? as usize;
        let records = self.entry_exit_record();
        let offset_data = self.offset_data();
        let entry_this = records.get(index_this)?.entry_anchor(offset_data)?.ok()?;

        let mut iter = SkippingIterator::new(ctx, false);
        iter.reset_fast(iter.buffer.idx);

        let mut unsafe_from = 0;
        if !iter.prev(Some(&mut unsafe_from)) {
            ctx.buffer
                .unsafe_to_concat_from_outbuffer(Some(unsafe_from), Some(ctx.buffer.idx + 1));
            return None;
        }

        let i = iter.index();
        let prev = iter.buffer.info.get(i).likely()?.as_glyph();
        let index_prev = coverage.get(prev)? as usize;
        let Some(exit_prev) = records
            .get(index_prev)
            .and_then(|rec| rec.exit_anchor(offset_data).transpose().ok().flatten())
        else {
            iter.buffer
                .unsafe_to_concat_from_outbuffer(Some(iter.index()), Some(iter.buffer.idx + 1));
            return None;
        };

        let (exit_x, exit_y) = ctx.layout.ot.resolve_anchor(&exit_prev);
        let (entry_x, entry_y) = ctx.layout.ot.resolve_anchor(&entry_this);
        let exit_x = ctx.scale_x(exit_x);
        let exit_y = ctx.scale_y(exit_y);
        let entry_x = ctx.scale_x(entry_x);
        let entry_y = ctx.scale_y(entry_y);

        let direction = ctx.buffer.direction;
        let j = ctx.buffer.idx;
        ctx.buffer.unsafe_to_break(Some(i), Some(j + 1));

        let pos = &mut ctx.buffer.pos;
        // Both glyphs are adjusted together, so work on copies and store them
        // back rather than holding two mutable borrows of the buffer.
        let mut prev_pos = *pos.get(i).likely()?;
        let mut this_pos = *pos.get(j).likely()?;
        match direction {
            Direction::LeftToRight => {
                prev_pos.x_advance = exit_x.saturating_add(prev_pos.x_offset);
                let d = entry_x.saturating_add(this_pos.x_offset);
                this_pos.x_advance = this_pos.x_advance.saturating_sub(d);
                this_pos.x_offset = this_pos.x_offset.saturating_sub(d);
            }
            Direction::RightToLeft => {
                let d = exit_x.saturating_add(prev_pos.x_offset);
                prev_pos.x_advance = prev_pos.x_advance.saturating_sub(d);
                prev_pos.x_offset = prev_pos.x_offset.saturating_sub(d);
                this_pos.x_advance = entry_x.saturating_add(this_pos.x_offset);
            }
            Direction::TopToBottom => {
                prev_pos.y_advance = exit_y.saturating_add(prev_pos.y_offset);
                let d = entry_y.saturating_add(this_pos.y_offset);
                this_pos.y_advance = this_pos.y_advance.saturating_sub(d);
                this_pos.y_offset = this_pos.y_offset.saturating_sub(d);
            }
            Direction::BottomToTop => {
                let d = exit_y.saturating_add(prev_pos.y_offset);
                prev_pos.y_advance = prev_pos.y_advance.saturating_sub(d);
                prev_pos.y_offset = prev_pos.y_offset.saturating_sub(d);
                this_pos.y_advance = entry_y;
            }
            Direction::Invalid => {}
        }
        *pos.get_mut(i).likely()? = prev_pos;
        *pos.get_mut(j).likely()? = this_pos;

        // Cross-direction adjustment

        // We attach child to parent (think graph theory and rooted trees whereas
        // the root stays on baseline and each node aligns itself against its
        // parent.
        //
        // Optimize things for the case of RightToLeft, as that's most common in
        // Arabic.
        let mut child = i;
        let mut parent = j;
        let mut x_offset = entry_x.saturating_sub(exit_x);
        let mut y_offset = entry_y.saturating_sub(exit_y);

        // Low bits are lookup flags, so we want to truncate.
        if ctx.lookup_props as u16 & lookup_flags::RIGHT_TO_LEFT == 0 {
            core::mem::swap(&mut child, &mut parent);
            x_offset = x_offset.saturating_neg();
            y_offset = y_offset.saturating_neg();
        }

        // If child was already connected to someone else, walk through its old
        // chain and reverse the link direction, such that the whole tree of its
        // previous connection now attaches to new parent.  Watch out for case
        // where new parent is on the path from old chain...
        reverse_cursive_minor_offset(pos, child, direction, parent)?;

        let child_pos = pos.get_mut(child).likely()?;
        child_pos.set_attach_type(attach_type::CURSIVE);
        let chain = parent as isize - child as isize;
        child_pos.set_attach_chain(chain as i16);
        // If the distance between the two glyphs does not fit in the i16 chain
        // field it would be truncated to a bogus value; leave the glyph
        // unattached instead of storing a poisoned chain. Matches HarfBuzz.
        if isize::from(child_pos.attach_chain()) != chain {
            child_pos.set_attach_chain(0);
        }
        if direction.is_horizontal() {
            child_pos.y_offset = y_offset;
        } else {
            child_pos.x_offset = x_offset;
        }
        let child_chain = child_pos.attach_chain();

        ctx.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;

        // If parent was attached to child, separate them.
        // https://github.com/harfbuzz/harfbuzz/issues/2469
        let parent_pos = ctx.buffer.pos.get_mut(parent).likely()?;
        if parent_pos.attach_chain() == -child_chain {
            parent_pos.set_attach_chain(0);

            if direction.is_horizontal() {
                parent_pos.y_offset = 0;
            } else {
                parent_pos.x_offset = 0;
            }
        }

        ctx.buffer.idx += 1;
        Some(())
    }
}

/// Returns `None` only when `i` is outside `pos`; stopping at the end of the
/// chain is success.
fn reverse_cursive_minor_offset(
    pos: &mut [GlyphPosition],
    i: usize,
    direction: Direction,
    new_parent: usize,
) -> Option<()> {
    let this = pos.get_mut(i).likely()?;
    let chain = this.attach_chain();
    let attach_type = this.attach_type();
    if chain == 0 || attach_type & attach_type::CURSIVE == 0 {
        return Some(());
    }

    this.set_attach_chain(0);

    let j = (i as isize + isize::from(chain)) as usize;

    // The chain is an i16 distance that can be truncated for very long buffers,
    // so `i + chain` may fall outside the buffer; stop instead of indexing out
    // of bounds. Matches HarfBuzz's `if (j >= len) return;`.
    if j >= pos.len() {
        return Some(());
    }

    // Stop if we see new parent in the chain.
    if j == new_parent {
        return Some(());
    }

    reverse_cursive_minor_offset(pos, j, direction, new_parent)?;

    // Read only after the recursion, since a chain that loops back can
    // rewrite this glyph.
    let this = pos.get(i).likely()?;
    let offset = if direction.is_horizontal() {
        this.y_offset
    } else {
        this.x_offset
    };
    let next = pos.get_mut(j).likely()?;
    if direction.is_horizontal() {
        next.y_offset = offset.saturating_neg();
    } else {
        next.x_offset = offset.saturating_neg();
    }
    next.set_attach_chain(-chain);
    next.set_attach_type(attach_type);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_cursive_minor_offset_ignores_out_of_range_chain() {
        // A truncated attach_chain can make `i + chain` point past the end of
        // the buffer; reverse_cursive_minor_offset must not index out of
        // bounds. See https://github.com/harfbuzz/harfrust/issues/410.
        let mut pos = vec![GlyphPosition::default(); 4];
        pos[0].set_attach_type(attach_type::CURSIVE);
        pos[0].set_attach_chain(30000); // 0 + 30000 is far past the buffer end

        reverse_cursive_minor_offset(&mut pos, 0, Direction::LeftToRight, 3);

        // The out-of-range link is cleared and no other glyph is touched.
        assert_eq!(pos[0].attach_chain(), 0);
    }

    #[test]
    fn reverse_cursive_minor_offset_saturates_negation() {
        let mut pos = vec![GlyphPosition::default(); 2];
        pos[0].set_attach_type(attach_type::CURSIVE);
        pos[0].set_attach_chain(1);
        pos[0].y_offset = i32::MIN;

        reverse_cursive_minor_offset(&mut pos, 0, Direction::LeftToRight, usize::MAX);

        assert_eq!(pos[1].y_offset, i32::MAX);
    }
}
