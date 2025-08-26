use super::get_class;
use super::layout::DELETED_GLYPH;
use crate::hb::aat::layout_common::{AatApplyContext, TypedCollectGlyphs, START_OF_TEXT};
use crate::hb::{
    buffer::*,
    ot_layout::TableIndex,
    ot_layout_common::lookup_flags,
    ot_layout_gpos_table::attach_type,
    ot_layout_gsubgpos::{skipping_iterator_t, OT::hb_ot_apply_context_t},
    ot_shape_plan::hb_ot_shape_plan_t,
};
use crate::U32Set;
use core::convert::TryFrom;
use read_fonts::{
    tables::{
        aat,
        ankr::Ankr,
        kerx::{
            Subtable, Subtable0, Subtable1, Subtable2, Subtable4, Subtable4Actions, Subtable6,
            SubtableKind,
        },
    },
    types::{BigEndian, FixedSize, GlyphId},
};

pub(crate) fn apply(c: &mut AatApplyContext) -> Option<()> {
    c.buffer.unsafe_to_concat(None, None);

    c.setup_buffer_glyph_set();

    let (kerx, subtable_caches) = c.face.aat_tables.kerx.as_ref()?;

    let mut subtable_idx = 0;

    let mut seen_cross_stream = false;
    for subtable in kerx.subtables().iter() {
        let Ok(subtable) = subtable else {
            continue;
        };

        let subtable_cache = subtable_caches.get(subtable_idx);
        let subtable_cache = subtable_cache.as_ref().unwrap();
        subtable_idx += 1;

        // We don't handle variations
        if subtable.is_variable() {
            continue;
        }

        if c.buffer.direction.is_horizontal() != subtable.is_horizontal() {
            continue;
        }

        let Ok(kind) = subtable.kind() else {
            continue;
        };

        c.first_set = Some(&subtable_cache.first_set);
        c.second_set = Some(&subtable_cache.second_set);
        c.machine_class_cache = Some(&subtable_cache.class_cache);

        if !c.buffer_intersects_machine() {
            continue;
        }

        let reverse = c.buffer.direction.is_backward();

        if !seen_cross_stream && subtable.is_cross_stream() {
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

        match &kind {
            SubtableKind::Format0(format0) => {
                if !c.plan.requested_kerning {
                    continue;
                }
                apply_simple_kerning(c, &subtable, format0);
            }
            SubtableKind::Format1(format1) => {
                let mut driver = Driver1 {
                    stack: [0; 8],
                    depth: 0,
                };
                apply_state_machine_kerning(
                    c,
                    &subtable,
                    format1,
                    &format1.state_table,
                    &mut driver,
                );
            }
            SubtableKind::Format2(format2) => {
                if !c.plan.requested_kerning {
                    continue;
                }
                c.buffer.unsafe_to_concat(None, None);
                apply_simple_kerning(c, &subtable, format2);
            }
            SubtableKind::Format4(format4) => {
                let mut driver = Driver4 {
                    mark_set: false,
                    mark: 0,
                    ankr_table: c.face.aat_tables.ankr.clone(),
                };
                apply_state_machine_kerning(
                    c,
                    &subtable,
                    format4,
                    &format4.state_table,
                    &mut driver,
                );
            }
            SubtableKind::Format6(format6) => {
                if !c.plan.requested_kerning {
                    continue;
                }
                apply_simple_kerning(c, &subtable, format6);
            }
        }

        if reverse {
            c.buffer.reverse();
        }
    }

    Some(())
}

pub trait SimpleKerning {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32>;
    fn collect_glyphs(&self, _first_set: &mut U32Set, _second_set: &mut U32Set, _num_glyphs: u32);
}

impl SimpleKerning for Subtable0<'_> {
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

impl SimpleKerning for Subtable2<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut U32Set, second_set: &mut U32Set, num_glyphs: u32) {
        let left_classes = &self.left_offset_table;
        let right_classes = &self.right_offset_table;

        left_classes.collect_glyphs(first_set, num_glyphs);
        right_classes.collect_glyphs(second_set, num_glyphs);
    }
}

impl SimpleKerning for Subtable6<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut U32Set, second_set: &mut U32Set, num_glyphs: u32) {
        match &self {
            Self::ShortValues(rows, columns, ..) => {
                rows.collect_glyphs(first_set, num_glyphs);
                columns.collect_glyphs(second_set, num_glyphs);
            }
            Self::LongValues(rows, columns, ..) => {
                rows.collect_glyphs(first_set, num_glyphs);
                columns.collect_glyphs(second_set, num_glyphs);
            }
        }
    }
}

fn apply_simple_kerning<T: SimpleKerning>(c: &mut AatApplyContext, subtable: &Subtable, kind: &T) {
    let mut ctx = hb_ot_apply_context_t::new(TableIndex::GPOS, c.face, c.buffer);
    ctx.set_lookup_mask(c.plan.kern_mask);
    ctx.lookup_props = u32::from(lookup_flags::IGNORE_MARKS);
    ctx.update_matchers();

    let horizontal = ctx.buffer.direction.is_horizontal();
    let cross_stream = subtable.is_cross_stream();

    let first_set = c.first_set.as_ref().unwrap();
    let second_set = c.second_set.as_ref().unwrap();

    let mut i = 0;
    while i < ctx.buffer.len {
        if (ctx.buffer.info[i].mask & c.plan.kern_mask) == 0 {
            i += 1;
            continue;
        }

        let mut iter = skipping_iterator_t::new(&mut ctx, false);
        iter.reset(i);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            ctx.buffer.unsafe_to_concat(Some(i), Some(unsafe_to));
            i += 1;
            continue;
        }

        let j = iter.index();

        let info = &ctx.buffer.info;
        let a = info[i].as_glyph();
        let b = info[j].as_glyph();
        let kern = if !first_set.contains(a.to_u32()) || !second_set.contains(b.to_u32()) {
            0
        } else {
            kind.simple_kerning(a, b).unwrap_or(0)
        };

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

pub(crate) trait KerxStateEntryExt {
    fn flags(&self) -> u16;
    fn action_index(&self) -> u16;

    fn is_action_initiable(&self) -> bool {
        self.flags() & 0x8000 != 0
    }

    fn is_actionable(&self) -> bool {
        self.action_index() != 0xFFFF
    }

    fn has_advance(&self) -> bool {
        self.flags() & 0x4000 == 0
    }

    fn has_reset(&self) -> bool {
        self.flags() & 0x2000 != 0
    }

    fn has_push(&self) -> bool {
        self.flags() & 0x8000 != 0
    }

    fn has_mark(&self) -> bool {
        self.flags() & 0x8000 != 0
    }
}

impl KerxStateEntryExt for aat::StateEntry<BigEndian<u16>> {
    fn flags(&self) -> u16 {
        self.flags
    }

    fn action_index(&self) -> u16 {
        self.payload.get()
    }
}

pub(crate) fn collect_initial_glyphs<T>(
    machine: &aat::ExtendedStateTable<T>,
    glyphs: &mut U32Set,
    num_glyphs: u32,
) where
    T: FixedSize + bytemuck::AnyBitPattern,
    aat::StateEntry<T>: KerxStateEntryExt,
{
    let mut classes = U32Set::default();

    let class_table = &machine.class_table;
    for i in 0..machine.n_classes {
        if let Ok(entry) = machine.entry(START_OF_TEXT, i as u16) {
            if entry.new_state == START_OF_TEXT
                && !entry.is_action_initiable()
                && !entry.is_actionable()
            {
                continue;
            }
            classes.insert(i as u32);
        }
    }

    // And glyphs in those classes.

    let filter = |class: u16| classes.contains(class as u32);

    if filter(aat::class::DELETED_GLYPH as u16) {
        glyphs.insert(DELETED_GLYPH);
    }

    class_table.collect_glyphs_filtered(glyphs, num_glyphs, filter);
}

fn apply_state_machine_kerning<T, E>(
    c: &mut AatApplyContext,
    subtable: &Subtable,
    kind: &T,
    state_table: &aat::ExtendedStateTable<E>,
    driver: &mut dyn StateTableDriver<T, E>,
) where
    E: FixedSize + bytemuck::AnyBitPattern,
    aat::StateEntry<E>: KerxStateEntryExt,
{
    let mut state = START_OF_TEXT;
    c.buffer.idx = 0;
    loop {
        let class = if c.buffer.idx < c.buffer.len {
            get_class(
                state_table,
                c.buffer.cur(0).as_glyph(),
                c.machine_class_cache.unwrap(),
            )
        } else {
            u16::from(aat::class::END_OF_TEXT)
        };

        let Ok(entry) = state_table.entry(state, class) else {
            break;
        };

        // Unsafe-to-break before this if not in state 0, as things might
        // go differently if we start from state 0 here.
        if state != START_OF_TEXT && c.buffer.backtrack_len() != 0 && c.buffer.idx < c.buffer.len {
            // If there's no value and we're just epsilon-transitioning to state 0, safe to break.
            if entry.is_actionable() || entry.new_state != START_OF_TEXT || entry.has_advance() {
                c.buffer.unsafe_to_break_from_outbuffer(
                    Some(c.buffer.backtrack_len() - 1),
                    Some(c.buffer.idx + 1),
                );
            }
        }

        // Unsafe-to-break if end-of-text would kick in here.
        if c.buffer.idx + 2 <= c.buffer.len {
            let end_entry = state_table
                .entry(state, u16::from(aat::class::END_OF_TEXT))
                .ok();
            let Some(end_entry) = end_entry else { break };

            if end_entry.is_actionable() {
                c.buffer
                    .unsafe_to_break(Some(c.buffer.idx), Some(c.buffer.idx + 2));
            }
        }

        let _ = driver.transition(
            kind,
            &entry,
            subtable.is_cross_stream(),
            subtable.tuple_count(),
            c.plan,
            c.buffer,
        );

        state = entry.new_state;

        if c.buffer.idx >= c.buffer.len {
            break;
        }

        if entry.has_advance() || c.buffer.max_ops <= 0 {
            c.buffer.next_glyph();
        }
        c.buffer.max_ops -= 1;
    }
}

trait StateTableDriver<T, E> {
    fn transition(
        &mut self,
        aat: &T,
        entry: &aat::StateEntry<E>,
        has_cross_stream: bool,
        tuple_count: u32,
        plan: &hb_ot_shape_plan_t,
        buffer: &mut hb_buffer_t,
    ) -> Option<()>;
}

struct Driver1 {
    stack: [usize; 8],
    depth: usize,
}

impl StateTableDriver<Subtable1<'_>, BigEndian<u16>> for Driver1 {
    fn transition(
        &mut self,
        aat: &Subtable1,
        entry: &aat::StateEntry<BigEndian<u16>>,
        has_cross_stream: bool,
        tuple_count: u32,
        plan: &hb_ot_shape_plan_t,
        buffer: &mut hb_buffer_t,
    ) -> Option<()> {
        if entry.has_reset() {
            self.depth = 0;
        }

        if entry.has_push() {
            if self.depth < self.stack.len() {
                self.stack[self.depth] = buffer.idx;
                self.depth += 1;
            } else {
                self.depth = 0; // Probably not what CoreText does, but better?
            }
        }

        if entry.is_actionable() && self.depth != 0 {
            let tuple_count = u16::try_from(tuple_count.max(1)).ok()?;

            let mut action_index = entry.action_index();

            // From Apple 'kern' spec:
            // "Each pops one glyph from the kerning stack and applies the kerning value to it.
            // The end of the list is marked by an odd value...
            let mut last = false;
            while !last && self.depth != 0 {
                self.depth -= 1;
                let idx = self.stack[self.depth];
                let mut v = aat.values.get(action_index as usize)?.get() as i32;
                action_index = action_index.checked_add(tuple_count)?;
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
                    } else if glyph_mask & plan.kern_mask != 0 {
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
                    } else if glyph_mask & plan.kern_mask != 0 {
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

        Some(())
    }
}
struct Driver4<'a> {
    mark_set: bool,
    mark: usize,
    ankr_table: Option<Ankr<'a>>,
}

impl StateTableDriver<Subtable4<'_>, BigEndian<u16>> for Driver4<'_> {
    fn transition(
        &mut self,
        aat: &Subtable4,
        entry: &aat::StateEntry<BigEndian<u16>>,
        _has_cross_stream: bool,
        _tuple_count: u32,
        _opt: &hb_ot_shape_plan_t,
        buffer: &mut hb_buffer_t,
    ) -> Option<()> {
        if self.mark_set && entry.is_actionable() && buffer.idx < buffer.len {
            match (self.ankr_table.as_ref(), &aat.actions) {
                (Some(ankr_table), Subtable4Actions::AnchorPoints(ankr_data)) => {
                    let action_idx = entry.action_index() as usize * 2;
                    let mark_action_idx = ankr_data.get(action_idx)?.get() as usize;
                    let curr_action_idx = ankr_data.get(action_idx + 1)?.get() as usize;
                    let mark_idx = buffer.info[self.mark].as_glyph();
                    let mark_anchor = ankr_table
                        .anchor_points(mark_idx)
                        .ok()
                        .and_then(|list| list.get(mark_action_idx))
                        .map(|point| (point.x(), point.y()))
                        .unwrap_or_default();

                    let curr_idx = buffer.cur(0).as_glyph();
                    let curr_anchor = ankr_table
                        .anchor_points(curr_idx)
                        .ok()
                        .and_then(|list| list.get(curr_action_idx))
                        .map(|point| (point.x(), point.y()))
                        .unwrap_or_default();

                    let pos = buffer.cur_pos_mut();
                    pos.x_offset = i32::from(mark_anchor.0 - curr_anchor.0);
                    pos.y_offset = i32::from(mark_anchor.1 - curr_anchor.1);
                }
                (_, Subtable4Actions::ControlPointCoords(coords)) => {
                    let action_idx = entry.action_index() as usize * 4;
                    let mark_x = coords.get(action_idx)?.get() as i32;
                    let mark_y = coords.get(action_idx + 1)?.get() as i32;
                    let curr_x = coords.get(action_idx + 2)?.get() as i32;
                    let curr_y = coords.get(action_idx + 3)?.get() as i32;
                    let pos = buffer.cur_pos_mut();
                    pos.x_offset = mark_x - curr_x;
                    pos.y_offset = mark_y - curr_y;
                }
                _ => {}
            }

            buffer.cur_pos_mut().set_attach_type(attach_type::MARK);
            let idx = buffer.idx;
            buffer
                .cur_pos_mut()
                .set_attach_chain(self.mark as i16 - idx as i16);
            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
        }

        if entry.has_mark() {
            self.mark_set = true;
            self.mark = buffer.idx;
        }

        Some(())
    }
}
