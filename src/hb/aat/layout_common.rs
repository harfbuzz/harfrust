use super::layout::DELETED_GLYPH;
use super::map::RangeFlags;
use super::ClassCache;
use crate::hb::buffer::{hb_buffer_t, HB_BUFFER_SCRATCH_FLAG_SHAPER0};
use crate::hb::face::hb_font_t;
use crate::hb::hb_mask_t;
use crate::hb::ot_shape_plan::hb_ot_shape_plan_t;
use read_fonts::collections::int_set::U32Set;
use read_fonts::tables::aat::*;

pub const HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED: u32 = HB_BUFFER_SCRATCH_FLAG_SHAPER0;

/// HB: hb_aat_apply_context_t
///
/// See <https://github.com/harfbuzz/harfbuzz/blob/2c22a65f0cb99544c36580b9703a43b5dc97a9e1/src/hb-aat-layout-common.hh#L108>
#[doc(alias = "hb_aat_apply_context_t")]
pub struct AatApplyContext<'a> {
    pub plan: &'a hb_ot_shape_plan_t,
    pub face: &'a hb_font_t<'a>,
    pub buffer: &'a mut hb_buffer_t,
    pub range_flags: Option<&'a mut [RangeFlags]>,
    pub subtable_flags: hb_mask_t,
    pub has_glyph_classes: bool,
    // Caches
    pub(crate) left_set: Option<&'a U32Set>,
    pub(crate) right_set: Option<&'a U32Set>,
    pub(crate) machine_class_cache: Option<&'a ClassCache>,
}

impl<'a> AatApplyContext<'a> {
    pub fn new(
        plan: &'a hb_ot_shape_plan_t,
        face: &'a hb_font_t<'a>,
        buffer: &'a mut hb_buffer_t,
    ) -> Self {
        Self {
            plan,
            face,
            buffer,
            range_flags: None,
            subtable_flags: 0,
            has_glyph_classes: face.ot_tables.has_glyph_classes(),
            left_set: None,
            right_set: None,
            machine_class_cache: None,
        }
    }

    pub fn output_glyph(&mut self, glyph: u32) {
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            self.buffer.cur_mut(0).set_aat_deleted();
        } else {
            if self.has_glyph_classes {
                self.buffer
                    .cur_mut(0)
                    .set_glyph_props(self.face.ot_tables.glyph_props(glyph.into()));
            }
        }
        self.buffer.output_glyph(glyph);
    }

    pub fn replace_glyph(&mut self, glyph: u32) {
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            self.buffer.cur_mut(0).set_aat_deleted();
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
        if self.has_glyph_classes {
            self.buffer.info[i].set_glyph_props(self.face.ot_tables.glyph_props(glyph.into()));
        }
    }
}

pub trait TypedCollectIndices<T: LookupValue> {
    /// Add all indices into `set`.
    fn collect_indices(&self, set: &mut U32Set) {
        self.filter_indices::<_>(set, |_| true);
    }

    /// For each valid index, read the value of type `T`.
    /// If `filter(&value)` returns true, insert the index into `set`.
    fn filter_indices<F>(&self, _set: &mut U32Set, _filter: F)
    where
        F: Fn(&T) -> bool,
    {
        /* TODO remove me. */
    }
}

impl<'a, T> TypedCollectIndices<T> for TypedLookup<'a, T>
where
    T: LookupValue,
{
    fn filter_indices<F>(&self, set: &mut U32Set, filter: F)
    where
        F: Fn(&T) -> bool,
    {
        self.lookup.filter_indices::<T, F>(set, filter);
    }
}

pub trait CollectIndices {
    /// Add all indices into `set`.
    fn collect_indices<T>(&self, set: &mut U32Set)
    where
        T: LookupValue,
    {
        self.filter_indices::<T, _>(set, |_| true);
    }

    /// For each valid index, read the value of type `T`.
    /// If `filter(&value)` returns true, insert the index into `set`.
    fn filter_indices<T, F>(&self, _set: &mut U32Set, _filter: F)
    where
        T: LookupValue,
        F: Fn(&T) -> bool,
    {
        /* TODO remove me. */
    }
}

impl<'a> CollectIndices for Lookup<'a> {
    fn filter_indices<T, F>(&self, set: &mut U32Set, filter: F)
    where
        T: LookupValue,
        F: Fn(&T) -> bool,
    {
        match self {
            Lookup::Format0(lookup) => lookup.filter_indices::<T, F>(set, filter),
            Lookup::Format2(lookup) => lookup.filter_indices::<T, F>(set, filter),
            Lookup::Format4(lookup) => lookup.filter_indices::<T, F>(set, filter),
            Lookup::Format6(lookup) => lookup.filter_indices::<T, F>(set, filter),
            Lookup::Format8(lookup) => lookup.filter_indices::<T, F>(set, filter),
            Lookup::Format10(lookup) => lookup.filter_indices::<T, F>(set, filter),
        }
    }
}

impl<'a> CollectIndices for Lookup0<'a> {}
impl<'a> CollectIndices for Lookup2<'a> {}
impl<'a> CollectIndices for Lookup4<'a> {}
impl<'a> CollectIndices for Lookup6<'a> {}
impl<'a> CollectIndices for Lookup8<'a> {}
impl<'a> CollectIndices for Lookup10<'a> {}

/*
impl<'a, T: LookupValue> CollectGlyphs for Lookup2<'a> {
    fn collect_glyphs(&self, glyphs: &mut U32Set) {
        if let Ok(segments) = self.segments::<T>() {
            for segment in segments {
            }
        }

    }
}
*/
