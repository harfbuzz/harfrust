use super::layout::AAT::DELETED_GLYPH;
use super::map::range_flags_t;
use crate::hb::buffer::{hb_buffer_t, HB_BUFFER_SCRATCH_FLAG_SHAPER0};
use crate::hb::face::hb_font_t;
use crate::hb::hb_mask_t;
use crate::hb::ot_layout::_hb_glyph_info_set_aat_deleted;

pub const HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED: u32 = HB_BUFFER_SCRATCH_FLAG_SHAPER0;

pub struct hb_aat_apply_context_t<'a> {
    pub face: &'a hb_font_t<'a>,
    pub buffer: &'a mut hb_buffer_t,
    pub range_flags: Option<&'a mut [range_flags_t]>,
    pub subtable_flags: hb_mask_t,
    pub has_glyph_classes: bool,
}

impl<'a> hb_aat_apply_context_t<'a> {
    pub fn new(face: &'a hb_font_t<'a>, buffer: &'a mut hb_buffer_t) -> Self {
        Self {
            face,
            buffer,
            range_flags: None,
            subtable_flags: 0,
            has_glyph_classes: face.ot_tables.has_glyph_classes(),
        }
    }

    pub fn output_glyph(&mut self, glyph: u32) {
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            _hb_glyph_info_set_aat_deleted(self.buffer.cur_mut(0));
        } else {
            if self.has_glyph_classes {
                self.buffer
                    .cur_mut(0)
                    .set_glyph_props(self.face.glyph_props(glyph.into()));
            }
        }
        self.buffer.output_glyph(glyph);
    }

    pub fn replace_glyph(&mut self, glyph: u32) {
        if glyph == DELETED_GLYPH {
            self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
            _hb_glyph_info_set_aat_deleted(self.buffer.cur_mut(0));
        }

        if self.has_glyph_classes {
            self.buffer
                .cur_mut(0)
                .set_glyph_props(self.face.glyph_props(glyph.into()));
        }
        self.buffer.replace_glyph(glyph);
    }

    pub fn delete_glyph(&mut self) {
        self.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_AAT_HAS_DELETED;
        _hb_glyph_info_set_aat_deleted(self.buffer.cur_mut(0));
        self.buffer.replace_glyph(DELETED_GLYPH);
    }

    pub fn replace_glyph_inplace(&mut self, i: usize, glyph: u32) {
        self.buffer.info[i].glyph_id = glyph;
        if self.has_glyph_classes {
            self.buffer.info[i].set_glyph_props(self.face.glyph_props(glyph.into()));
        }
    }
}
