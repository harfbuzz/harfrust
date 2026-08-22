use super::aat::layout::DELETED_GLYPH;
use alloc::boxed::Box;
use alloc::vec::Vec;
use read_fonts::{
    tables::{
        aat,
        kern::{Subtable, Subtable0, Subtable2, Subtable3, SubtableKind},
    },
    types::{GlyphId, GlyphId16},
};

use super::aat::layout_common::{AatApplyContext, ClassCache, START_OF_TEXT};
use super::aat::layout_kerx_table::SimpleKerning;
use super::buffer::*;
use super::face::Scale;
use super::ot_layout::TableIndex;
use super::ot_layout_common::lookup_flags;
use super::ot_layout_gpos_table::attach_type;
use super::ot_layout_gsubgpos::{skipping_iterator_t, OT::hb_ot_apply_context_t};
use super::ot_shape_plan::hb_ot_shape_plan_t;
use super::{hb_font_t, hb_mask_t};
use super::aat::glyph_set::GlyphSet;

pub(crate) fn get_class(machine: &aat::StateTable, glyph_id: GlyphId, cache: &ClassCache) -> u8 {
    if let Some(klass) = cache.get(glyph_id.to_u32()) {
        return klass as u8;
    }
    let klass = machine
        .class(GlyphId16::new(glyph_id.to_u32() as u16))
        .unwrap_or(aat::class::OUT_OF_BOUNDS);
    cache.set(glyph_id.to_u32(), klass as u32);
    klass
}

pub fn hb_ot_layout_kern(
    plan: &hb_ot_shape_plan_t,
    face: &hb_font_t,
    scale: Scale,
    buffer: &mut hb_buffer_t,
) -> Option<()> {
    let mut c = AatApplyContext::new(plan, face, scale, buffer);

    c.setup_buffer_glyph_set();

    let (kern, subtable_caches) = c.face.aat_tables.kern.as_ref()?;

    let mut subtable_idx = 0;

    let mut seen_cross_stream = false;
    for subtable in kern.subtables() {
        let Ok(subtable) = subtable else { continue };

        let subtable_cache = subtable_caches.get(subtable_idx);
        let Some(subtable_cache) = subtable_cache.as_ref() else {
            break;
        };
        subtable_idx += 1;

        if subtable.is_variable() {
            continue;
        }

        if c.buffer.direction.is_horizontal() != subtable.is_horizontal() {
            continue;
        }

        c.first_set = Some(&subtable_cache.first_set);
        c.second_set = Some(&subtable_cache.second_set);
        c.machine_class_cache = Some(&subtable_cache.class_cache);
        c.start_end_safe_to_break = subtable_cache.start_end_safe_to_break;

        if !c.buffer_intersects_machine() {
            continue;
        }

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

        let Ok(kind) = subtable.kind() else {
            continue;
        };

        if reverse != c.buffer_is_reversed {
            c.reverse_buffer();
        }

        match kind {
            SubtableKind::Format0(format0) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format0, is_cross_stream);
            }
            SubtableKind::Format1(format1) => {
                apply_state_machine_kerning(
                    &mut c,
                    &format1,
                    subtable_cache.decoded.as_ref(),
                    is_cross_stream,
                );
            }
            SubtableKind::Format2(format2) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format2, is_cross_stream);
            }
            SubtableKind::Format3(format3) if plan.requested_kerning => {
                apply_simple_kerning(&mut c, &format3, is_cross_stream);
            }
            _ => {}
        }
    }
    if c.buffer_is_reversed {
        c.reverse_buffer();
    }
    Some(())
}

fn machine_kern<F>(
    face: &hb_font_t,
    scale: Scale,
    buffer: &mut hb_buffer_t,
    kern_mask: hb_mask_t,
    cross_stream: bool,
    get_kerning: F,
) where
    F: Fn(u32, u32) -> i32,
{
    buffer.unsafe_to_concat(None, None);
    let mut ctx = hb_ot_apply_context_t::new(TableIndex::GPOS, face, scale, buffer);
    ctx.set_lookup_mask(kern_mask);
    ctx.lookup_props = u32::from(lookup_flags::IGNORE_MARKS);
    ctx.update_matchers();

    let horizontal = ctx.buffer.direction.is_horizontal();
    let use_x_scale = horizontal ^ cross_stream;
    let mut i = 0;
    let mut iter = skipping_iterator_t::new(&mut ctx, false);
    while i < iter.buffer.len {
        if (iter.buffer.info[i].mask & kern_mask) == 0 {
            i += 1;
            continue;
        }

        iter.reset_fast(i);

        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            i = unsafe_to;
            continue;
        }

        let j = iter.index();

        let info = &iter.buffer.info;
        let kern = get_kerning(info[i].glyph_id, info[j].glyph_id);
        let kern = if use_x_scale {
            scale.scale_x(kern)
        } else {
            scale.scale_y(kern)
        };
        let pos = &mut iter.buffer.pos;
        if kern != 0 {
            if horizontal {
                if cross_stream {
                    pos[j].y_offset = kern;
                    iter.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].x_advance = pos[i].x_advance.saturating_add(kern1);
                    pos[j].x_advance = pos[j].x_advance.saturating_add(kern2);
                    pos[j].x_offset = pos[j].x_offset.saturating_add(kern2);
                }
            } else {
                if cross_stream {
                    pos[j].x_offset = kern;
                    iter.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].y_advance = pos[i].y_advance.saturating_add(kern1);
                    pos[j].y_advance = pos[j].y_advance.saturating_add(kern2);
                    pos[j].y_offset = pos[j].y_offset.saturating_add(kern2);
                }
            }

            iter.buffer.unsafe_to_break(Some(i), Some(j + 1));
        }

        i = j;
    }
}

fn apply_simple_kerning<T: SimpleKerning>(
    c: &mut AatApplyContext,
    subtable: &T,
    is_cross_stream: bool,
) {
    let first_set = c.first_set.as_ref().unwrap();
    let second_set = c.second_set.as_ref().unwrap();

    machine_kern(
        c.face,
        c.scale,
        c.buffer,
        c.plan.kern_mask,
        is_cross_stream,
        |left, right| {
            if !first_set.contains(left) || !second_set.contains(right) {
                0
            } else {
                subtable
                    .simple_kerning(left.into(), right.into())
                    .unwrap_or(0)
            }
        },
    );
}

struct StateMachineDriver {
    stack: [usize; 8],
    depth: usize,
}

/// Bit layout of a decoded transition: `[flags:16][safe:1][poison:1][new_state:14]`.
const DECODED_NEW_STATE_MASK: u32 = 0x3FFF;
const DECODED_POISON: u32 = 1 << 14;
const DECODED_SAFE_TO_BREAK: u32 = 1 << 15;
const DECODED_FLAGS_SHIFT: u32 = 16;
/// `new_state` must pack into 14 bits, with 0x3FFF reserved as the
/// always-out-of-bounds sentinel that makes an unrepresentable next
/// state fail the next lookup, exactly like the raw path does.
const DECODED_MAX_STATES: usize = 0x3FFE;

/// A legacy `kern` Format1 state machine decoded into a flat transition
/// array, one packed `u32` per state-array byte in the same linear
/// layout. The drive loop replaces the per-transition `entry()` parsing
/// (offset arithmetic, big-endian reads, new-state division and error
/// plumbing) — plus the extra `entry()` probes the safe-to-break
/// computation needs — with a single indexed load. Everything stored
/// here depends only on `(state, class)`, so it is computed once per
/// face; the machines are tiny, a few hundred entries.
pub(crate) struct DecodedStateMachine {
    n_classes: usize,
    entries: Vec<u32>,
}

impl DecodedStateMachine {
    fn new(machine: &aat::StateTable, start_end_safe_to_break: u64) -> Option<Self> {
        // Guard against hostile geometry: bound the decode size, and
        // require OUT_OF_BOUNDS to be a valid class so the clamp in
        // `transition` can never alias into a neighboring state row
        // (the raw path is kept for such degenerate machines).
        const MAX_ENTRIES: usize = 1 << 20;

        let n_classes = machine.header.state_size() as usize;
        let len = machine.header.state_array().ok()?.data().len();
        if n_classes <= aat::class::OUT_OF_BOUNDS as usize
            || len == 0
            || len > MAX_ENTRIES
            || len.div_ceil(n_classes) > DECODED_MAX_STATES
        {
            return None;
        }

        let mut entries = Vec::with_capacity(len);
        for i in 0..len {
            let state = (i / n_classes) as u16;
            let class = i % n_classes;
            if class > u8::MAX as usize {
                // Classes come from u8 data, so these columns are
                // unreachable; only the row-aligned layout needs them.
                entries.push(DECODED_POISON);
                continue;
            }
            let Ok(entry) = machine.entry(state, class as u8) else {
                entries.push(DECODED_POISON);
                continue;
            };

            let next_state = entry.new_state;

            // The safe-to-break conditions; see the walk-through in
            // `apply_state_machine_kerning_raw`.
            let is_safe_to_break =
                // 1
                !entry.is_actionable() &&

                // 2
                (
                    state == START_OF_TEXT
                    || (!entry.has_advance() && next_state == START_OF_TEXT)
                    ||
                    {
                        // 2c
                        if let Ok(wouldbe_entry) = machine.entry(START_OF_TEXT, class as u8) {
                            // 2c'
                            !wouldbe_entry.is_actionable() &&

                            // 2c"
                            (
                                next_state == wouldbe_entry.new_state &&
                                entry.has_advance() == wouldbe_entry.has_advance()
                            )
                        } else {
                            false
                        }
                    }
                ) &&

                // 3
                (
                    if state < 64 {
                        (start_end_safe_to_break & (1 << state)) != 0
                    } else {
                        if let Ok(end_entry) = machine.entry(state, aat::class::END_OF_TEXT) {
                            !end_entry.is_actionable()
                        } else {
                            false
                        }
                    }
                )
            ;

            entries.push(
                ((entry.flags as u32) << DECODED_FLAGS_SHIFT)
                    | ((is_safe_to_break as u32) << 15)
                    | (next_state as u32).min(DECODED_NEW_STATE_MASK),
            );
        }
        Some(DecodedStateMachine { n_classes, entries })
    }

    /// Mirrors `StateTable::entry`: the same class clamp, and `None`
    /// exactly where it would error (out-of-bounds row, or a poisoned
    /// slot recording that the raw entry failed to decode).
    #[inline(always)]
    fn transition(&self, state: u16, class: u8) -> Option<u32> {
        let mut class = class as usize;
        if class >= self.n_classes {
            class = aat::class::OUT_OF_BOUNDS as usize;
        }
        let packed = *self
            .entries
            .get(state as usize * self.n_classes + class)?;
        if packed & DECODED_POISON != 0 {
            return None;
        }
        Some(packed)
    }
}

pub trait CollectGlyphs {
    /// For each valid index, read the value of type `T`.
    /// If `filter(&value)` returns true, insert the index into `set`.
    fn collect_glyphs_filtered<F>(&self, _set: &mut GlyphSet, _num_glyphs: u32, _filter: F)
    where
        F: Fn(u8) -> bool;
}

impl CollectGlyphs for aat::ClassSubtable<'_> {
    fn collect_glyphs_filtered<F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        F: Fn(u8) -> bool,
    {
        let first_glyph = self.first_glyph() as u32;
        let class_array = self.class_array();
        for (i, class) in class_array.iter().enumerate() {
            let gid = first_glyph + i as u32;
            if filter(*class) {
                set.insert(gid);
            }
        }
    }
}

fn collect_initial_glyphs(machine: &aat::StateTable, glyphs: &mut GlyphSet, num_glyphs: u32) {
    let mut classes = GlyphSet::default();

    let class_table = machine.header.class_table().ok();
    let Some(class_table) = class_table else {
        return;
    };

    let n_classes = machine.header.state_size();
    for i in 0..n_classes {
        if let Ok(entry) = machine.entry(START_OF_TEXT, i as u8) {
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

    let filter = |class: u8| classes.contains(class as u32);

    if filter(aat::class::DELETED_GLYPH) {
        glyphs.insert(DELETED_GLYPH);
    }

    class_table.collect_glyphs_filtered(glyphs, num_glyphs, filter);
}

fn collect_start_end_safe_to_break(machine: &aat::StateTable) -> u64 {
    let mut result = 0u64;
    for state in 0..64 {
        let bit = if let Ok(entry) = machine.entry(state, aat::class::END_OF_TEXT) {
            !entry.is_actionable()
        } else {
            true
        };
        if bit {
            result |= 1 << state;
        }
    }
    result
}

fn apply_state_machine_kerning(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    decoded: Option<&DecodedStateMachine>,
    is_cross_stream: bool,
) {
    if let Some(machine) = decoded {
        apply_state_machine_kerning_decoded(c, subtable, machine, is_cross_stream);
    } else {
        apply_state_machine_kerning_raw(c, subtable, is_cross_stream);
    }
}

/// The same drive loop as `apply_state_machine_kerning_raw`, but over
/// the decoded transition array: one load replaces the `entry()` parse,
/// and the precomputed bit replaces the safe-to-break probe lookups.
fn apply_state_machine_kerning_decoded(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    machine: &DecodedStateMachine,
    is_cross_stream: bool,
) {
    let mut driver = StateMachineDriver {
        stack: [0; 8],
        depth: 0,
    };

    let mut state = START_OF_TEXT;
    // No end-of-text action can fire if we stop while in the start state.
    let start_state_safe_to_break_eot = (c.start_end_safe_to_break & (1 << START_OF_TEXT)) != 0;
    c.buffer.idx = 0;
    'drive: loop {
        let class = if c.buffer.idx < c.buffer.len {
            get_class(
                subtable,
                c.buffer.cur(0).as_glyph(),
                c.machine_class_cache.unwrap(),
            )
        } else {
            aat::class::END_OF_TEXT
        };

        let Some(packed) = machine.transition(state, class) else {
            break;
        };

        let next_state = (packed & DECODED_NEW_STATE_MASK) as u16;
        let entry = (packed >> DECODED_FLAGS_SHIFT) as u16;

        // Fast path for when transitioning from start-state to start-state with
        // no action and advancing. Do so as long as the class remains the same.
        // This is common with runs of non-actionable glyphs.
        if state == START_OF_TEXT
            && next_state == START_OF_TEXT
            && start_state_safe_to_break_eot
            && !entry.is_actionable()
            && entry.has_advance()
        {
            let old_class = class;
            loop {
                state_machine_transition(c, subtable, entry, is_cross_stream, &mut driver);
                if c.buffer.idx >= c.buffer.len {
                    break 'drive;
                }
                c.buffer.max_ops -= 1;
                c.buffer.next_glyph();

                let new_class = if c.buffer.idx < c.buffer.len {
                    get_class(
                        subtable,
                        c.buffer.cur(0).as_glyph(),
                        c.machine_class_cache.unwrap(),
                    )
                } else {
                    aat::class::END_OF_TEXT
                };
                if new_class != old_class {
                    break;
                }
            }
            if c.buffer.idx >= c.buffer.len {
                break 'drive;
            }
            continue 'drive;
        }

        let is_safe_to_break = packed & DECODED_SAFE_TO_BREAK != 0;

        if !is_safe_to_break && c.buffer.backtrack_len() > 0 && c.buffer.idx < c.buffer.len {
            c.buffer.unsafe_to_break_from_outbuffer(
                Some(c.buffer.backtrack_len() - 1),
                Some(c.buffer.idx + 1),
            );
        }

        state_machine_transition(c, subtable, entry, is_cross_stream, &mut driver);

        state = next_state;

        if c.buffer.idx >= c.buffer.len {
            break;
        }

        c.buffer.max_ops -= 1;
        if entry.has_advance() || c.buffer.max_ops <= 0 {
            c.buffer.next_glyph();
        }
    }
}

fn apply_state_machine_kerning_raw(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    is_cross_stream: bool,
) {
    let mut driver = StateMachineDriver {
        stack: [0; 8],
        depth: 0,
    };

    let mut state = START_OF_TEXT;
    // Condition 3 below, precomputed for the start-of-text state: no
    // end-of-text action can fire if we stop while in the start state.
    let start_state_safe_to_break_eot = (c.start_end_safe_to_break & (1 << START_OF_TEXT)) != 0;
    c.buffer.idx = 0;
    'drive: loop {
        let class = if c.buffer.idx < c.buffer.len {
            get_class(
                subtable,
                c.buffer.cur(0).as_glyph(),
                c.machine_class_cache.unwrap(),
            )
        } else {
            aat::class::END_OF_TEXT
        };

        let Ok(entry) = subtable.entry(state, class) else {
            break;
        };

        let next_state = entry.new_state;

        // Fast path for when transitioning from start-state to start-state with
        // no action and advancing. Do so as long as the class remains the same.
        // This is common with runs of non-actionable glyphs.
        if state == START_OF_TEXT
            && next_state == START_OF_TEXT
            && start_state_safe_to_break_eot
            && !entry.is_actionable()
            && entry.has_advance()
        {
            let old_class = class;
            loop {
                state_machine_transition(c, subtable, entry.flags, is_cross_stream, &mut driver);
                if c.buffer.idx >= c.buffer.len {
                    break 'drive;
                }
                c.buffer.max_ops -= 1;
                c.buffer.next_glyph();

                let new_class = if c.buffer.idx < c.buffer.len {
                    get_class(
                        subtable,
                        c.buffer.cur(0).as_glyph(),
                        c.machine_class_cache.unwrap(),
                    )
                } else {
                    aat::class::END_OF_TEXT
                };
                if new_class != old_class {
                    break;
                }
            }
            if c.buffer.idx >= c.buffer.len {
                break 'drive;
            }
            continue 'drive;
        }

        // Conditions under which it's guaranteed safe-to-break before current glyph:
        //
        // 1. There was no action in this transition; and
        //
        // 2. If we break before current glyph, the results will be the same. That
        //    is guaranteed if:
        //
        //    2a. We were already in start-of-text state; or
        //
        //    2b. We are epsilon-transitioning to start-of-text state; or
        //
        //    2c. Starting from start-of-text state seeing current glyph:
        //
        //        2c'. There won't be any actions; and
        //
        //        2c". We would end up in the same state that we were going to end up
        //             in now, including whether epsilon-transitioning.
        //
        //    and
        //
        // 3. If we break before current glyph, there won't be any end-of-text action
        //    after previous glyph.
        //
        // This triples the transitions we need to look up, but is worth returning
        // granular unsafe-to-break results. See eg.:
        //
        //   https://github.com/harfbuzz/harfbuzz/issues/2860

        let is_safe_to_break =
            // 1
            !entry.is_actionable() &&

            // 2
            (
                state == START_OF_TEXT
                || (!entry.has_advance() && next_state == START_OF_TEXT)
                ||
                {
                    // 2c
                    if let Ok(wouldbe_entry) = subtable.entry(START_OF_TEXT, class) {
                        // 2c'
                        !wouldbe_entry.is_actionable() &&

                        // 2c"
                        (
                            next_state == wouldbe_entry.new_state &&
                            entry.has_advance() == wouldbe_entry.has_advance()
                        )
                    } else {
                        false
                    }
                }
            ) &&

            // 3
            (
                if state < 64 {
                    (c.start_end_safe_to_break & (1 << state)) != 0
                } else {
                    if let Ok(end_entry) = subtable.entry(state, aat::class::END_OF_TEXT) {
                        !end_entry.is_actionable()
                    } else {
                        false
                    }
                }
            )
        ;

        if !is_safe_to_break && c.buffer.backtrack_len() > 0 && c.buffer.idx < c.buffer.len {
            c.buffer.unsafe_to_break_from_outbuffer(
                Some(c.buffer.backtrack_len() - 1),
                Some(c.buffer.idx + 1),
            );
        }

        state_machine_transition(c, subtable, entry.flags, is_cross_stream, &mut driver);

        state = next_state;

        if c.buffer.idx >= c.buffer.len {
            break;
        }

        c.buffer.max_ops -= 1;
        if entry.has_advance() || c.buffer.max_ops <= 0 {
            c.buffer.next_glyph();
        }
    }
}

#[inline(always)]
fn state_machine_transition(
    c: &mut AatApplyContext,
    subtable: &aat::StateTable,
    entry: u16,
    is_cross_stream: bool,
    driver: &mut StateMachineDriver,
) {
    let scale = c.scale;
    let use_x_scale = c.buffer.direction.is_horizontal() ^ is_cross_stream;
    let buffer = &mut *c.buffer;
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
            let scaled_v = if use_x_scale {
                scale.scale_x(v)
            } else {
                scale.scale_y(v)
            };

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
                        pos.y_offset = pos.y_offset.saturating_add(scaled_v);
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    pos.x_advance = pos.x_advance.saturating_add(scaled_v);
                    pos.x_offset = pos.x_offset.saturating_add(scaled_v);
                }
            } else {
                if is_cross_stream {
                    // CoreText doesn't do crossStream kerning in vertical. We do.
                    if v == -0x8000 {
                        pos.set_attach_type(0);
                        pos.set_attach_chain(0);
                        pos.x_offset = 0;
                    } else if pos.attach_type() != 0 {
                        pos.x_offset = pos.x_offset.saturating_add(scaled_v);
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    if pos.y_offset == 0 {
                        pos.y_advance = pos.y_advance.saturating_add(scaled_v);
                        pos.y_offset = pos.y_offset.saturating_add(scaled_v);
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

    fn is_action_initiable(&self) -> bool {
        self.flags() & 0x8000 != 0
    }

    fn is_actionable(&self) -> bool {
        self.flags() & 0x3FFF != 0
    }

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

impl KernStateEntryExt for u16 {
    fn flags(&self) -> u16 {
        *self
    }
}

impl SimpleKerning for Subtable0<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut GlyphSet, second_set: &mut GlyphSet, _num_glyphs: u32) {
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
    fn collect_glyphs(&self, first_set: &mut GlyphSet, second_set: &mut GlyphSet, _num_glyphs: u32) {
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

impl SimpleKerning for Subtable3<'_> {
    fn simple_kerning(&self, left: GlyphId, right: GlyphId) -> Option<i32> {
        self.kerning(left, right)
    }
    fn collect_glyphs(&self, first_set: &mut GlyphSet, second_set: &mut GlyphSet, _num_glyphs: u32) {
        first_set.insert_range(0..=self.glyph_count().saturating_sub(1) as u32);
        second_set.insert_range(0..=self.glyph_count().saturating_sub(1) as u32);
    }
}

pub(crate) struct KernSubtableCache {
    start_end_safe_to_break: u64,
    first_set: GlyphSet,
    second_set: GlyphSet,
    class_cache: Box<ClassCache>,
    decoded: Option<DecodedStateMachine>,
}

impl KernSubtableCache {
    pub(crate) fn new(subtable: &Subtable, num_glyphs: u32) -> Self {
        let mut start_end_safe_to_break = 0u64;
        let mut first_set = GlyphSet::default();
        let mut second_set = GlyphSet::default();
        let mut decoded = None;
        if let Ok(kind) = subtable.kind() {
            match &kind {
                SubtableKind::Format0(format0) => {
                    format0.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                }
                SubtableKind::Format1(format1) => {
                    start_end_safe_to_break = collect_start_end_safe_to_break(format1);
                    collect_initial_glyphs(format1, &mut first_set, num_glyphs);
                    decoded = DecodedStateMachine::new(format1, start_end_safe_to_break);
                }
                SubtableKind::Format2(format2) => {
                    format2.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                }
                SubtableKind::Format3(format3) => {
                    format3.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                }
            }
        }
        KernSubtableCache {
            start_end_safe_to_break,
            first_set,
            second_set,
            class_cache: Box::new(ClassCache::new()),
            decoded,
        }
    }
}
