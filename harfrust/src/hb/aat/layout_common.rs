use super::layout::DELETED_GLYPH;
use super::map::RangeFlags;
use crate::hb::buffer::{hb_buffer_t, HB_BUFFER_SCRATCH_FLAG_SHAPER0};
use crate::hb::face::hb_font_t;
use crate::hb::face::Scale;
use crate::hb::hb_mask_t;
use crate::hb::ot_layout_gsubgpos::MappingCache;
use crate::hb::ot_shape_plan::hb_ot_shape_plan_t;
use super::glyph_set::GlyphSet;
use alloc::vec::Vec;
use read_fonts::tables::aat::*;
use read_fonts::types::{FixedSize, GlyphId};
use read_fonts::FontData;

pub const HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED: u32 = HB_BUFFER_SCRATCH_FLAG_SHAPER0;

pub(crate) const START_OF_TEXT: u16 = 0;

pub(crate) type ClassCache = MappingCache;

pub(crate) fn get_class<T: bytemuck::AnyBitPattern + FixedSize>(
    machine: &ExtendedStateTable<'_, T>,
    glyph_id: GlyphId,
    cache: &ClassCache,
) -> u16 {
    if let Some(klass) = cache.get(glyph_id.to_u32()) {
        return klass as u16;
    }
    let klass = machine
        .class(glyph_id)
        .unwrap_or(class::OUT_OF_BOUNDS as u16);
    cache.set(glyph_id.to_u32(), klass as u32);
    klass
}

/// HB: hb_aat_apply_context_t
///
/// See <https://github.com/harfbuzz/harfbuzz/blob/2c22a65f0cb99544c36580b9703a43b5dc97a9e1/src/hb-aat-layout-common.hh#L108>
#[doc(alias = "hb_aat_apply_context_t")]
pub struct AatApplyContext<'a> {
    pub plan: &'a hb_ot_shape_plan_t,
    pub face: &'a hb_font_t<'a>,
    pub scale: Scale,
    pub buffer: &'a mut hb_buffer_t,
    pub has_glyph_classes: bool,
    pub range_flags: Option<&'a [RangeFlags]>,
    pub subtable_flags: hb_mask_t,
    pub(crate) buffer_is_reversed: bool,
    // Caches
    using_buffer_glyph_set: bool,
    pub(crate) first_set: Option<&'a GlyphSet>,
    pub(crate) second_set: Option<&'a GlyphSet>,
    pub(crate) machine_class_cache: Option<&'a ClassCache>,
    pub(crate) start_end_safe_to_break: u64,
}

impl<'a> AatApplyContext<'a> {
    pub fn new(
        plan: &'a hb_ot_shape_plan_t,
        face: &'a hb_font_t<'a>,
        scale: Scale,
        buffer: &'a mut hb_buffer_t,
    ) -> Self {
        Self {
            plan,
            face,
            scale,
            buffer,
            has_glyph_classes: face.ot_tables.has_glyph_classes(),
            range_flags: None,
            subtable_flags: 0,
            buffer_is_reversed: false,
            using_buffer_glyph_set: false,
            first_set: None,
            second_set: None,
            machine_class_cache: None,
            start_end_safe_to_break: 0,
        }
    }

    #[inline(always)]
    pub(crate) fn scale_x(&self, value: i32) -> i32 {
        self.scale.scale_x(value)
    }

    #[inline(always)]
    pub(crate) fn scale_y(&self, value: i32) -> i32 {
        self.scale.scale_y(value)
    }

    pub(crate) fn reverse_buffer(&mut self) {
        self.buffer.reverse();
        self.buffer_is_reversed = !self.buffer_is_reversed;
    }

    pub(crate) fn setup_buffer_glyph_set(&mut self) {
        self.using_buffer_glyph_set = self.buffer.len >= 4;

        if self.using_buffer_glyph_set {
            self.buffer.update_glyph_set();
        }
    }

    pub(crate) fn buffer_intersects_machine(&self) -> bool {
        if let Some(first_set) = &self.first_set {
            if self.using_buffer_glyph_set {
                return self.buffer.glyph_set.intersects_set(first_set);
            }
            for info in &self.buffer.info {
                if first_set.contains(info.glyph_id) {
                    return true;
                }
            }
            false
        } else {
            true
        }
    }

    pub fn output_glyph(&mut self, glyph: u32) {
        if self.using_buffer_glyph_set {
            self.buffer.glyph_set.insert(glyph);
        }

        // Insertion state machines can emit glyphs during the end-of-text
        // transition, when there is no current input glyph. output_glyph()
        // supports that by copying the previous output glyph's metadata.
        // Apply the AAT metadata to that new output glyph instead of indexing
        // past the end of the input buffer.
        let at_end = self.buffer.idx == self.buffer.len;
        if at_end {
            let out_len = self.buffer.out_len;
            self.buffer.output_glyph(glyph);
            if self.buffer.out_len == out_len {
                return;
            }
        }

        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            let info = if at_end {
                self.buffer.prev_mut()
            } else {
                self.buffer.cur_mut(0)
            };
            info.set_aat_deleted();
        } else {
            if self.has_glyph_classes {
                let glyph_props = self.face.ot_tables.glyph_props(glyph.into());
                let info = if at_end {
                    self.buffer.prev_mut()
                } else {
                    self.buffer.cur_mut(0)
                };
                info.set_glyph_props(glyph_props);
            }
        }
        if !at_end {
            self.buffer.output_glyph(glyph);
        }
    }

    pub fn replace_glyph(&mut self, glyph: u32) {
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            self.buffer.cur_mut(0).set_aat_deleted();
        }

        if self.using_buffer_glyph_set {
            self.buffer.glyph_set.insert(glyph);
        }
        if self.has_glyph_classes {
            self.buffer
                .cur_mut(0)
                .set_glyph_props(self.face.ot_tables.glyph_props(glyph.into()));
        }
        self.buffer.replace_glyph(glyph);
    }

    pub fn delete_glyph(&mut self) {
        self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
        self.buffer.cur_mut(0).set_aat_deleted();
        self.buffer.replace_glyph(DELETED_GLYPH);
    }

    pub fn replace_glyph_inplace(&mut self, i: usize, glyph: u32) {
        self.buffer.info[i].glyph_id = glyph;
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            self.buffer.info[i].set_aat_deleted();
        }
        if self.using_buffer_glyph_set {
            self.buffer.glyph_set.insert(glyph);
        }
        if self.has_glyph_classes {
            self.buffer.info[i].set_glyph_props(self.face.ot_tables.glyph_props(glyph.into()));
        }
    }
}

pub trait TypedCollectGlyphs<T: LookupValue> {
    /// Add all indices into `set`.
    fn collect_glyphs(&self, set: &mut GlyphSet, num_glyphs: u32) {
        self.collect_glyphs_filtered::<_>(set, num_glyphs, |_| true);
    }

    /// For each valid index, read the value of type `T`.
    /// If `filter(&value)` returns true, insert the index into `set`.
    fn collect_glyphs_filtered<F>(&self, _set: &mut GlyphSet, _num_glyphs: u32, _filter: F)
    where
        F: Fn(T) -> bool;
}

impl<T> TypedCollectGlyphs<T> for TypedLookup<'_, T>
where
    T: LookupValue,
{
    fn collect_glyphs(&self, set: &mut GlyphSet, num_glyphs: u32) {
        self.lookup.collect_glyphs::<T>(set, num_glyphs);
    }
    fn collect_glyphs_filtered<F>(&self, set: &mut GlyphSet, num_glyphs: u32, filter: F)
    where
        F: Fn(T) -> bool,
    {
        self.lookup
            .collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
    }
}

pub trait CollectGlyphs {
    /// Add all indices into `set`.
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, num_glyphs: u32)
    where
        T: LookupValue,
    {
        self.collect_glyphs_filtered::<T, _>(set, num_glyphs, |_| true);
    }

    /// For each valid index, read the value of type `T`.
    /// If `filter(&value)` returns true, insert the index into `set`.
    fn collect_glyphs_filtered<T, F>(&self, _set: &mut GlyphSet, _num_glyphs: u32, _filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool;
}

impl CollectGlyphs for Lookup<'_> {
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, num_glyphs: u32)
    where
        T: LookupValue,
    {
        match self {
            Lookup::Format0(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
            Lookup::Format2(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
            Lookup::Format4(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
            Lookup::Format6(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
            Lookup::Format8(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
            Lookup::Format10(lookup) => lookup.collect_glyphs::<T>(set, num_glyphs),
        }
    }
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        match self {
            Lookup::Format0(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
            Lookup::Format2(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
            Lookup::Format4(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
            Lookup::Format6(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
            Lookup::Format8(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
            Lookup::Format10(lookup) => {
                lookup.collect_glyphs_filtered::<T, F>(set, num_glyphs, filter);
            }
        }
    }
}

impl CollectGlyphs for Lookup0<'_> {
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, num_glyphs: u32)
    where
        T: LookupValue,
    {
        set.insert_range(0..=num_glyphs.saturating_sub(1));
    }
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        if let Ok(values) = self.values::<T>() {
            for (i, value) in values.iter().take(num_glyphs as usize).enumerate() {
                if filter(value.get()) {
                    set.insert(i as u32);
                }
            }
        }
    }
}
impl CollectGlyphs for Lookup2<'_> {
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        if let Ok(segments) = self.segments::<T>() {
            for segment in segments {
                let value = segment.value;
                if filter(value.get()) {
                    if segment.first_glyph.get() as u32 == DELETED_GLYPH {
                        continue;
                    }
                    set.insert_range(
                        segment.first_glyph.get() as u32..=segment.last_glyph.get() as u32,
                    );
                }
            }
        }
    }
}
impl CollectGlyphs for Lookup4<'_> {
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, _num_glyphs: u32)
    where
        T: LookupValue,
    {
        for segment in self.segments() {
            if segment.first_glyph.get() as u32 == DELETED_GLYPH {
                continue;
            }
            set.insert_range(segment.first_glyph.get() as u32..=segment.last_glyph.get() as u32);
        }
    }
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        for (segment_idx, segment) in self.segments().iter().enumerate() {
            if segment.first_glyph.get() as u32 == DELETED_GLYPH {
                continue;
            }
            let segment_values = self.segment_values(segment_idx);
            if let Ok(segment_values) = segment_values {
                for (i, value) in segment_values.iter().enumerate() {
                    if filter(value.get()) {
                        set.insert(segment.first_glyph.get() as u32 + i as u32);
                    }
                }
            }
        }
    }
}
impl CollectGlyphs for Lookup6<'_> {
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        let entries = self.entries();
        if let Ok(entries) = entries {
            for entry in entries {
                let value = entry.value;
                if filter(value.get()) {
                    if entry.glyph.get() as u32 == DELETED_GLYPH {
                        continue;
                    }
                    set.insert(entry.glyph.get() as u32);
                }
            }
        }
    }
}
impl CollectGlyphs for Lookup8<'_> {
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, _num_glyphs: u32)
    where
        T: LookupValue,
    {
        let n_values = self.value_array().len();
        let first_glyph = self.first_glyph();
        if first_glyph as u32 == DELETED_GLYPH {
            return;
        }
        set.insert_range(
            first_glyph as u32..=first_glyph as u32 + n_values.saturating_sub(1) as u32,
        );
    }
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        let values = self.value_array();
        let first_glyph = self.first_glyph();
        if first_glyph as u32 == DELETED_GLYPH {
            return;
        }
        for (i, value) in values.iter().enumerate() {
            if filter(T::from_u16(value.get())) {
                set.insert(first_glyph as u32 + i as u32);
            }
        }
    }
}
impl CollectGlyphs for Lookup10<'_> {
    fn collect_glyphs<T>(&self, set: &mut GlyphSet, _num_glyphs: u32)
    where
        T: LookupValue,
    {
        let n_values = self.glyph_count();
        let first_glyph = self.first_glyph();
        if first_glyph as u32 == DELETED_GLYPH {
            return;
        }
        set.insert_range(
            first_glyph as u32..=first_glyph as u32 + n_values.saturating_sub(1) as u32,
        );
    }
    fn collect_glyphs_filtered<T, F>(&self, set: &mut GlyphSet, _num_glyphs: u32, filter: F)
    where
        T: LookupValue,
        F: Fn(T) -> bool,
    {
        let first_glyph = self.first_glyph();
        if first_glyph as u32 == DELETED_GLYPH {
            return;
        }
        for i in 0..self.glyph_count() {
            let idx = first_glyph as u32 + i as u32;
            // TODO: Speed up by accessing the value array directly
            let value = self.value::<T>(idx as u16);
            if let Ok(value) = value {
                if filter(value) {
                    set.insert(idx);
                }
            }
        }
    }
}

pub(crate) const MACHINE_META_SAFE: u8 = 1;
pub(crate) const MACHINE_META_POISON: u8 = 2;

/// An extended state machine decoded into flat per-(state, class) arrays,
/// built once per face. The drive loops replace the per-transition
/// `entry()` parsing — plus the extra `entry()` probes behind the
/// safe-to-break computation — with a single indexed load; everything
/// stored here depends only on `(state, class)`.
///
/// The bounds-checked lookup reproduces `entry()`'s error semantics: an
/// out-of-range row fails the lookup and breaks the drive exactly where
/// the raw path would, and a per-slot decode failure carries a poison
/// flag in `meta`.
pub(crate) struct DecodedExtMachine<T> {
    n_classes: usize,
    entries: Vec<StateEntry<T>>,
    meta: Vec<u8>,
}

impl<T: bytemuck::AnyBitPattern + FixedSize> DecodedExtMachine<T> {
    #[inline(never)]
    pub(crate) fn build(
        machine: &ExtendedStateTable<T>,
        data: &[u8],
        start_end_safe_to_break: u64,
        is_actionable: &dyn Fn(&StateEntry<T>) -> bool,
        can_advance: &dyn Fn(&StateEntry<T>) -> bool,
    ) -> Option<Self> {
        // Guard against hostile geometry: bound the decode size, and
        // require OUT_OF_BOUNDS to be a valid class so the clamp in
        // `get` can never alias into a neighboring state row (the raw
        // path handles such degenerate machines).
        const MAX_CELLS: usize = 1 << 18;

        let n_classes = machine.n_classes;
        // The state array runs from its offset to the end of the
        // subtable, matching how the table reader slices it.
        let parts = StateTableParts::read(FontData::new(data)).ok()?;
        let n_cells = (data.len().checked_sub(parts.state_array_offset as usize)?)
            / u16::RAW_BYTE_LEN;
        if n_classes <= class::OUT_OF_BOUNDS as usize
            || n_cells == 0
            || n_cells > MAX_CELLS
            || n_cells.div_ceil(n_classes) > u16::MAX as usize
        {
            return None;
        }

        // Pass 1: decode every cell once; a failed decode is poisoned so
        // the lookup fails exactly where the raw entry() would.
        let mut entries = Vec::with_capacity(n_cells);
        let mut meta = Vec::with_capacity(n_cells);
        for i in 0..n_cells {
            let state = (i / n_classes) as u16;
            let class = (i % n_classes) as u16;
            if let Ok(entry) = machine.entry(state, class) {
                entries.push(entry);
                meta.push(0);
            } else {
                entries.push(StateEntry {
                    new_state: 0,
                    flags: 0,
                    payload: T::zeroed(),
                });
                meta.push(MACHINE_META_POISON);
            }
        }

        // Pass 2: the safe-to-break conditions (see the walk-through in
        // the morx drive loop), reading the probe entries — start-row
        // "wouldbe" and end-of-text — from the decoded array instead of
        // re-parsing them. A poisoned or out-of-range probe answers
        // false, exactly like the raw path's Err.
        fn probe<T: Clone>(
            entries: &[StateEntry<T>],
            meta: &[u8],
            ix: usize,
        ) -> Option<StateEntry<T>> {
            (meta.get(ix).copied()? & MACHINE_META_POISON == 0).then(|| entries[ix].clone())
        }
        for i in 0..n_cells {
            if meta[i] & MACHINE_META_POISON != 0 {
                continue;
            }
            let state = (i / n_classes) as u16;
            let class = i % n_classes;
            let entry = entries[i].clone();
            let next_state = entry.new_state;

            let is_safe_to_break =
                // 1
                !is_actionable(&entry) &&

                // 2
                (
                    state == START_OF_TEXT
                    || (!can_advance(&entry) && next_state == START_OF_TEXT)
                    ||
                    {
                        // 2c
                        if let Some(wouldbe_entry) = probe(&entries, &meta, class) {
                            // 2c'
                            !is_actionable(&wouldbe_entry) &&

                            // 2c"
                            (
                                next_state == wouldbe_entry.new_state &&
                                can_advance(&entry) == can_advance(&wouldbe_entry)
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
                        if let Some(end_entry) = probe(
                            &entries,
                            &meta,
                            state as usize * n_classes + class::END_OF_TEXT as usize,
                        ) {
                            !is_actionable(&end_entry)
                        } else {
                            false
                        }
                    }
                )
            ;

            meta[i] |= (is_safe_to_break as u8) * MACHINE_META_SAFE;
        }
        Some(DecodedExtMachine {
            n_classes,
            entries,
            meta,
        })
    }

    /// Mirrors `ExtendedStateTable::entry`: the same class clamp, and
    /// `None` exactly where it would error.
    #[inline(always)]
    pub(crate) fn get(&self, state: u16, class: u16) -> Option<(&StateEntry<T>, u8)> {
        let mut class = class as usize;
        if class >= self.n_classes {
            class = class::OUT_OF_BOUNDS as usize;
        }
        let ix = state as usize * self.n_classes + class;
        let entry = self.entries.get(ix)?;
        let meta = *self.meta.get(ix)?;
        if meta & MACHINE_META_POISON != 0 {
            return None;
        }
        Some((entry, meta))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Direction, FontRef, ShapePlan, ShaperData, UnicodeBuffer};

    #[test]
    fn output_deleted_glyph_at_end_of_text_marks_output() {
        let font_data = include_bytes!("../../../tests/fonts/text-rendering-tests/TestMORXOne.ttf");
        let font = FontRef::new(font_data).unwrap();
        let shaper_data = ShaperData::new(&font);
        let shaper = shaper_data.shaper(&font).build();
        let plan = ShapePlan::new(&shaper, Direction::LeftToRight, None, None, &[]);

        let mut unicode_buffer = UnicodeBuffer::new();
        unicode_buffer.add('A', 0);
        let mut buffer = unicode_buffer.0;
        buffer.clear_output();
        buffer.next_glyph();

        let mut context = AatApplyContext::new(&plan, &shaper, Scale::default(), &mut buffer);
        context.output_glyph(DELETED_GLYPH);

        assert_eq!(context.buffer.out_len, 2);
        assert_eq!(context.buffer.prev().glyph_id, DELETED_GLYPH);
        assert!(context.buffer.prev().is_aat_deleted());
    }
}
