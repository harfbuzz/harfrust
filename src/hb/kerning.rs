use read_fonts::{
    tables::{aat, kern},
    types::GlyphId,
};

use super::aat_layout_kerx_table::SimpleKerning;
use super::buffer::*;
use super::ot_layout::TableIndex;
use super::ot_layout_common::lookup_flags;
use super::ot_layout_gpos_table::attach_type;
use super::ot_layout_gsubgpos::{skipping_iterator_t, OT::hb_ot_apply_context_t};
use super::ot_shape_plan::hb_ot_shape_plan_t;
use super::{hb_font_t, hb_mask_t};

pub fn hb_ot_layout_kern(plan: &hb_ot_shape_plan_t, face: &hb_font_t, buffer: &mut hb_buffer_t) {
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

        if buffer.direction.is_horizontal() != subtable.is_horizontal() {
            continue;
        }

        let Ok(kind) = subtable.kind() else {
            continue;
        };

        let reverse = buffer.direction.is_backward();
        let is_cross_stream = subtable.is_cross_stream();

        if !seen_cross_stream && is_cross_stream {
            seen_cross_stream = true;

            // Attach all glyphs into a chain.
            for pos in &mut buffer.pos {
                pos.set_attach_type(attach_type::CURSIVE);
                pos.set_attach_chain(if buffer.direction.is_forward() { -1 } else { 1 });
                // We intentionally don't set BufferScratchFlags::HAS_GPOS_ATTACHMENT,
                // since there needs to be a non-zero attachment for post-positioning to
                // be needed.
            }
        }

        if reverse {
            buffer.reverse();
        }

        match kind {
            kern::SubtableKind::Format0(format0) if plan.requested_kerning => {
                apply_simple_kerning(&format0, is_cross_stream, face, plan.kern_mask, buffer);
            }
            kern::SubtableKind::Format1(format1) => {
                apply_state_machine_kerning(&format1, is_cross_stream, plan.kern_mask, buffer);
            }
            kern::SubtableKind::Format2(format2) if plan.requested_kerning => {
                apply_simple_kerning(&format2, is_cross_stream, face, plan.kern_mask, buffer);
            }
            kern::SubtableKind::Format3(format3) if plan.requested_kerning => {
                apply_simple_kerning(&format3, is_cross_stream, face, plan.kern_mask, buffer);
            }
            _ => {}
        }

        if reverse {
            buffer.reverse();
        }
    }
}

// TODO: remove
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
    subtable: &T,
    is_cross_stream: bool,
    face: &hb_font_t,
    kern_mask: hb_mask_t,
    buffer: &mut hb_buffer_t,
) {
    machine_kern(face, buffer, kern_mask, is_cross_stream, |left, right| {
        subtable
            .simple_kerning(left.into(), right.into())
            .unwrap_or(0)
    });
}

struct StateMachineDriver {
    stack: [usize; 8],
    depth: usize,
}

const START_OF_TEXT: u16 = 0;

fn apply_state_machine_kerning(
    subtable: &aat::StateTable,
    is_cross_stream: bool,
    kern_mask: hb_mask_t,
    buffer: &mut hb_buffer_t,
) {
    let mut driver = StateMachineDriver {
        stack: [0; 8],
        depth: 0,
    };

    let mut state = START_OF_TEXT;
    buffer.idx = 0;
    loop {
        let class = if buffer.idx < buffer.len {
            buffer.info[buffer.idx]
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
        if state != START_OF_TEXT && buffer.backtrack_len() != 0 && buffer.idx < buffer.len {
            // If there's no value and we're just epsilon-transitioning to state 0, safe to break.
            if entry.has_offset() || entry.new_state != START_OF_TEXT || entry.has_advance() {
                buffer.unsafe_to_break_from_outbuffer(
                    Some(buffer.backtrack_len() - 1),
                    Some(buffer.idx + 1),
                );
            }
        }

        // Unsafe-to-break if end-of-text would kick in here.
        if buffer.idx + 2 <= buffer.len {
            let Ok(end_entry) = subtable.entry(state, aat::class::END_OF_TEXT) else {
                break;
            };

            if end_entry.has_offset() {
                buffer.unsafe_to_break(Some(buffer.idx), Some(buffer.idx + 2));
            }
        }

        state_machine_transition(
            subtable,
            &entry,
            is_cross_stream,
            kern_mask,
            &mut driver,
            buffer,
        );

        state = entry.new_state;

        if buffer.idx >= buffer.len {
            break;
        }

        buffer.max_ops -= 1;
        if entry.has_advance() || buffer.max_ops <= 0 {
            buffer.next_glyph();
        }
    }
}

fn state_machine_transition(
    subtable: &aat::StateTable,
    entry: &aat::StateEntry,
    has_cross_stream: bool,
    kern_mask: hb_mask_t,
    driver: &mut StateMachineDriver,
    buffer: &mut hb_buffer_t,
) {
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
                if has_cross_stream {
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
                if has_cross_stream {
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
}

impl SimpleKerning for kern::Subtable2<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
}

impl SimpleKerning for kern::Subtable3<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
}
