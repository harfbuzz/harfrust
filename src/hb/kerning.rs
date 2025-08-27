use read_fonts::{
    tables::{aat, kern},
    types::GlyphId,
};

use super::aat::layout_common::AatApplyContext;
use super::aat::layout_kerx_table::SimpleKerning;
use super::buffer::*;
use super::ot_layout::TableIndex;
use super::ot_layout_common::lookup_flags;
use super::ot_layout_gpos_table::attach_type;
use super::ot_layout_gsubgpos::{skipping_iterator_t, OT::hb_ot_apply_context_t};
use super::ot_shape_plan::hb_ot_shape_plan_t;
use super::{hb_font_t, hb_mask_t};
use crate::U32Set;

pub fn hb_ot_layout_kern(plan: &hb_ot_shape_plan_t, face: &hb_font_t, buffer: &mut hb_buffer_t) {
    let mut c = AatApplyContext::new(plan, face, buffer);

    let subtables = match face.aat_tables.kern.as_ref() {
        Some(table) => table.subtables(),
        None => return,
    };

    let mut seen_cross_stream = false;
    for subtable in subtables {
        let Ok(subtable) = subtable else {
            return;
        };

        if subtable.is_variable() {
            continue;
        }

        if c.buffer.direction.is_horizontal() != subtable.is_horizontal() {
            continue;
        }

        let Ok(kind) = subtable.kind() else {
            continue;
        };

        let reverse = c.buffer.direction.is_backward();
        let is_cross_stream = subtable.is_cross_stream();

        if !seen_cross_stream && is_cross_stream {
            seen_cross_stream = true;

            // Attach all glyphs into a chain.
            for pos in &mut c.buffer.pos {
                pos.set_attach_type(attach_type::CURSIVE);
                pos.set_attach_chain(if c.buffer.direction.is_forward() {
                    -1
                } else {
                    1
                });
                // We intentionally don't set BufferScratchFlags::HAS_GPOS_ATTACHMENT,
                // since there needs to be a non-zero attachment for post-positioning to
                // be needed.
            }
        }

        if reverse {
            c.buffer.reverse();
        }

        match kind {
            kern::SubtableKind::Format0(format0) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format0, is_cross_stream);
            }
            kern::SubtableKind::Format1(format1) => {
                apply_state_machine_kerning(&mut c, &format1, is_cross_stream);
            }
            kern::SubtableKind::Format2(format2) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format2, is_cross_stream);
            }
            kern::SubtableKind::Format3(format3) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format3, is_cross_stream);
            }
            _ => {}
        }

        if reverse {
            c.buffer.reverse();
        }
    }
}

fn machine_kern(
    face: &hb_font_t,
    buffer: &mut hb_buffer_t,
    kern_mask: hb_mask_t,
    cross_stream: bool,
    get_kerning: impl Fn(u32, u32) -> i32,
) {
    buffer.unsafe_to_concat(None, None);
    let mut ctx = hb_ot_apply_context_t::new(TableIndex::GPOS, face, buffer);
    ctx.set_lookup_mask(kern_mask);
    ctx.lookup_props = u32::from(lookup_flags::IGNORE_MARKS);
    ctx.update_matchers();

    let horizontal = ctx.buffer.direction.is_horizontal();

    let mut i = 0;
    while i < ctx.buffer.len {
        if (ctx.buffer.info[i].mask & kern_mask) == 0 {
            i += 1;
            continue;
        }

        let mut iter = skipping_iterator_t::new(&mut ctx, false);
        iter.reset(i);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            i += 1;
            continue;
        }

        let j = iter.index();

        let info = &ctx.buffer.info;
        let kern = get_kerning(info[i].glyph_id, info[j].glyph_id);

        let pos = &mut ctx.buffer.pos;
        if kern != 0 {
            if horizontal {
                if cross_stream {
                    pos[j].y_offset = kern;
                    ctx.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].x_advance += kern1;
                    pos[j].x_advance += kern2;
                    pos[j].x_offset += kern2;
                }
            } else {
                if cross_stream {
                    pos[j].x_offset = kern;
                    ctx.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].y_advance += kern1;
                    pos[j].y_advance += kern2;
                    pos[j].y_offset += kern2;
                }
            }

            ctx.buffer.unsafe_to_break(Some(i), Some(j + 1));
        }

        i = j;
    }
}

fn apply_simple_kerning<T: SimpleKerning>(
    c: &mut AatApplyContext,
    subtable: &T,
    is_cross_stream: bool,
) {
    machine_kern(
        c.face,
        c.buffer,
        c.plan.kern_mask,
        is_cross_stream,
        |left, right| {
            subtable
                .simple_kerning(left.into(), right.into())
                .unwrap_or(0)
        },
    );
}

struct StateMachineDriver {
    stack: [usize; 8],
    depth: usize,
}

const START_OF_TEXT: u16 = 0;

fn apply_state_machine_kerning(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    is_cross_stream: bool,
) {
    let mut driver = StateMachineDriver {
        stack: [0; 8],
        depth: 0,
    };

    let mut state = START_OF_TEXT;
    c.buffer.idx = 0;
    loop {
        let class = if c.buffer.idx < c.buffer.len {
            c.buffer.info[c.buffer.idx]
                .as_gid16()
                .and_then(|gid| subtable.class(gid).ok())
                .unwrap_or(1)
        } else {
            aat::class::END_OF_TEXT
        };

        let Ok(entry) = subtable.entry(state, class) else {
            break;
        };

        // Unsafe-to-break before this if not in state 0, as things might
        // go differently if we start from state 0 here.
        if state != START_OF_TEXT && c.buffer.backtrack_len() != 0 && c.buffer.idx < c.buffer.len {
            // If there's no value and we're just epsilon-transitioning to state 0, safe to break.
            if entry.has_offset() || entry.new_state != START_OF_TEXT || entry.has_advance() {
                c.buffer.unsafe_to_break_from_outbuffer(
                    Some(c.buffer.backtrack_len() - 1),
                    Some(c.buffer.idx + 1),
                );
            }
        }

        // Unsafe-to-break if end-of-text would kick in here.
        if c.buffer.idx + 2 <= c.buffer.len {
            let Ok(end_entry) = subtable.entry(state, aat::class::END_OF_TEXT) else {
                break;
            };

            if end_entry.has_offset() {
                c.buffer
                    .unsafe_to_break(Some(c.buffer.idx), Some(c.buffer.idx + 2));
            }
        }

        state_machine_transition(c, subtable, &entry, is_cross_stream, &mut driver);

        state = entry.new_state;

        if c.buffer.idx >= c.buffer.len {
            break;
        }

        c.buffer.max_ops -= 1;
        if entry.has_advance() || c.buffer.max_ops <= 0 {
            c.buffer.next_glyph();
        }
    }
}

fn state_machine_transition(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    entry: &aat::StateEntry,
    is_cross_stream: bool,
    driver: &mut StateMachineDriver,
) {
    let buffer = &mut c.buffer;
    let kern_mask = c.plan.kern_mask;

    if entry.has_push() {
        if driver.depth < driver.stack.len() {
            driver.stack[driver.depth] = buffer.idx;
            driver.depth += 1;
        } else {
            driver.depth = 0; // Probably not what CoreText does, but better?
        }
    }

    if entry.has_offset() && driver.depth != 0 {
        let mut value_offset = entry.value_offset();
        let Ok(mut value) = subtable.read_value::<i16>(value_offset as usize) else {
            driver.depth = 0;
            return;
        };

        // From Apple 'kern' spec:
        // "Each pops one glyph from the kerning stack and applies the kerning value to it.
        // The end of the list is marked by an odd value...
        let mut last = false;
        while !last && driver.depth != 0 {
            driver.depth -= 1;
            let idx = driver.stack[driver.depth];
            let mut v = value as i32;
            value_offset = value_offset.wrapping_add(2);
            value = subtable
                .read_value::<i16>(value_offset as usize)
                .unwrap_or(0);
            if idx >= buffer.len {
                continue;
            }

            // "The end of the list is marked by an odd value..."
            last = v & 1 != 0;
            v &= !1;

            // Testing shows that CoreText only applies kern (cross-stream or not)
            // if none has been applied by previous subtables. That is, it does
            // NOT seem to accumulate as otherwise implied by specs.

            let mut has_gpos_attachment = false;
            let glyph_mask = buffer.info[idx].mask;
            let pos = &mut buffer.pos[idx];

            if buffer.direction.is_horizontal() {
                if is_cross_stream {
                    // The following flag is undocumented in the spec, but described
                    // in the 'kern' table example.
                    if v == -0x8000 {
                        pos.set_attach_type(0);
                        pos.set_attach_chain(0);
                        pos.y_offset = 0;
                    } else if pos.attach_type() != 0 {
                        pos.y_offset += v;
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    pos.x_advance += v;
                    pos.x_offset += v;
                }
            } else {
                if is_cross_stream {
                    // CoreText doesn't do crossStream kerning in vertical. We do.
                    if v == -0x8000 {
                        pos.set_attach_type(0);
                        pos.set_attach_chain(0);
                        pos.x_offset = 0;
                    } else if pos.attach_type() != 0 {
                        pos.x_offset += v;
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    if pos.y_offset == 0 {
                        pos.y_advance += v;
                        pos.y_offset += v;
                    }
                }
            }

            if has_gpos_attachment {
                buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
            }
        }
    }
}

trait KernStateEntryExt {
    fn flags(&self) -> u16;

    fn has_offset(&self) -> bool {
        self.flags() & 0x3FFF != 0
    }

    fn value_offset(&self) -> u16 {
        self.flags() & 0x3FFF
    }

    fn has_advance(&self) -> bool {
        self.flags() & 0x4000 == 0
    }

    fn has_push(&self) -> bool {
        self.flags() & 0x8000 != 0
    }
}

impl<T> KernStateEntryExt for aat::StateEntry<T> {
    fn flags(&self) -> u16 {
        self.flags
    }
}

impl SimpleKerning for kern::Subtable0<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut U32Set, second_set: &mut U32Set, _num_glyphs: u32) {
        for &pair in self.pairs() {
            first_set.insert(pair.left.get().to_u32());
            second_set.insert(pair.right.get().to_u32());
        }
    }
}

impl SimpleKerning for kern::Subtable2<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut U32Set, second_set: &mut U32Set, _num_glyphs: u32) {
        let left_classes = &self.left_offset_table;
        let right_classes = &self.right_offset_table;

        let first_glyph = left_classes.first_glyph().to_u32();
        let last_glyphs = first_glyph + left_classes.n_glyphs().saturating_sub(1) as u32;
        first_set.insert_range(first_glyph..=last_glyphs);

        let first_glyph = right_classes.first_glyph().to_u32();
        let last_glyphs = first_glyph + right_classes.n_glyphs().saturating_sub(1) as u32;
        second_set.insert_range(first_glyph..=last_glyphs);
    }
}

impl SimpleKerning for kern::Subtable3<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut U32Set, second_set: &mut U32Set, _num_glyphs: u32) {
        first_set.insert_range(0..=self.glyph_count().saturating_sub(1) as u32);
        second_set.insert_range(0..=self.glyph_count().saturating_sub(1) as u32);
    }
}
