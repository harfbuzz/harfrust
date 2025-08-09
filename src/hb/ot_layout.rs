//! OpenType layout.

use core::ops::{Index, IndexMut};

use super::buffer::*;
use super::ot::lookup::LookupInfo;
use super::ot_layout_gsubgpos::OT;
use super::ot_shape_plan::hb_ot_shape_plan_t;
use super::unicode::{hb_unicode_funcs_t, hb_unicode_general_category_t, GeneralCategoryExt};
use super::{hb_font_t, hb_glyph_info_t};
use crate::hb::ot_layout_gsubgpos::OT::check_glyph_property;

impl hb_glyph_info_t {
    declare_buffer_var!(u16, 1, 0, GLYPH_PROPS_VAR, glyph_props, set_glyph_props);
    declare_buffer_var!(u8, 1, 2, LIG_PROPS_VAR, lig_props, set_lig_props);
    declare_buffer_var!(u8, 1, 3, SYLLABLE_VAR, syllable, set_syllable);
    declare_buffer_var!(
        u16,
        2,
        0,
        UNICODE_PROPS_VAR,
        unicode_props,
        set_unicode_props
    );
}

impl hb_buffer_t {
    pub(crate) fn allocate_unicode_vars(&mut self) {
        self.allocate_var(hb_glyph_info_t::UNICODE_PROPS_VAR);
    }
    pub(crate) fn deallocate_unicode_vars(&mut self) {
        self.deallocate_var(hb_glyph_info_t::UNICODE_PROPS_VAR);
    }
    pub(crate) fn assert_unicode_vars(&mut self) {
        self.assert_var(hb_glyph_info_t::UNICODE_PROPS_VAR);
    }

    pub(crate) fn allocate_gsubgpos_vars(&mut self) {
        self.allocate_var(hb_glyph_info_t::LIG_PROPS_VAR);
        self.allocate_var(hb_glyph_info_t::GLYPH_PROPS_VAR);
    }
    pub(crate) fn deallocate_gsubgpos_vars(&mut self) {
        self.deallocate_var(hb_glyph_info_t::LIG_PROPS_VAR);
        self.deallocate_var(hb_glyph_info_t::GLYPH_PROPS_VAR);
    }
    pub(crate) fn assert_gsubgpos_vars(&mut self) {
        self.assert_var(hb_glyph_info_t::LIG_PROPS_VAR);
        self.assert_var(hb_glyph_info_t::GLYPH_PROPS_VAR);
    }
}

pub const MAX_NESTING_LEVEL: usize = 64;
pub const MAX_CONTEXT_LENGTH: usize = 64;

pub fn hb_ot_layout_has_kerning(face: &hb_font_t) -> bool {
    face.aat_tables.kern.is_some()
}

pub fn hb_ot_layout_has_machine_kerning(face: &hb_font_t) -> bool {
    match face.aat_tables.kern {
        Some(ref kern) => kern
            .subtables()
            .filter_map(|s| s.ok())
            .any(|s| s.is_state_machine()),
        None => false,
    }
}

pub fn hb_ot_layout_has_cross_kerning(face: &hb_font_t) -> bool {
    match face.aat_tables.kern {
        Some(ref kern) => kern
            .subtables()
            .filter_map(|s| s.ok())
            .any(|s| s.is_cross_stream()),
        None => false,
    }
}

// hb_ot_layout_kern

// OT::GDEF::is_blocklisted unsupported

pub fn _hb_ot_layout_set_glyph_props(face: &hb_font_t, buffer: &mut hb_buffer_t) {
    buffer.assert_gsubgpos_vars();

    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        info.set_glyph_props(face.glyph_props(info.as_glyph()));
        info.set_lig_props(0);
    }
}

pub fn hb_ot_layout_has_glyph_classes(face: &hb_font_t) -> bool {
    face.ot_tables.has_glyph_classes()
}

// get_gsubgpos_table

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableIndex {
    GSUB = 0,
    GPOS = 1,
}

impl TableIndex {
    pub fn iter() -> impl Iterator<Item = TableIndex> {
        [Self::GSUB, Self::GPOS].iter().copied()
    }
}

impl<T> Index<TableIndex> for [T] {
    type Output = T;

    fn index(&self, table_index: TableIndex) -> &Self::Output {
        &self[table_index as usize]
    }
}

impl<T> IndexMut<TableIndex> for [T] {
    fn index_mut(&mut self, table_index: TableIndex) -> &mut Self::Output {
        &mut self[table_index as usize]
    }
}

/// A lookup-based layout table (GSUB or GPOS).
pub trait LayoutTable {
    /// The index of this table.
    const INDEX: TableIndex;

    /// Whether lookups in this table can be applied to the buffer in-place.
    const IN_PLACE: bool;

    /// Get the lookup at the specified index.
    fn get_lookup(&self, index: u16) -> Option<&LookupInfo>;
}

/// Called before substitution lookups are performed, to ensure that glyph
/// class and other properties are set on the glyphs in the buffer.
pub fn hb_ot_layout_substitute_start(face: &hb_font_t, buffer: &mut hb_buffer_t) {
    _hb_ot_layout_set_glyph_props(face, buffer);
}

/// Applies the lookups in the given GSUB or GPOS table.
pub fn apply_layout_table<T: LayoutTable>(
    plan: &hb_ot_shape_plan_t,
    face: &hb_font_t,
    buffer: &mut hb_buffer_t,
    table: Option<&T>,
) {
    let mut ctx = OT::hb_ot_apply_context_t::new(T::INDEX, face, buffer);

    for (stage_index, stage) in plan.ot_map.stages(T::INDEX).iter().enumerate() {
        if let Some(table) = table {
            for lookup_map in plan.ot_map.stage_lookups(T::INDEX, stage_index) {
                let Some(lookup) = table.get_lookup(lookup_map.index) else {
                    continue;
                };

                if lookup.digest().may_intersect(&ctx.digest) {
                    ctx.lookup_index = lookup_map.index;
                    ctx.set_lookup_mask(lookup_map.mask);
                    ctx.auto_zwj = lookup_map.auto_zwj;
                    ctx.auto_zwnj = lookup_map.auto_zwnj;

                    ctx.random = lookup_map.random;
                    ctx.per_syllable = lookup_map.per_syllable;

                    apply_string::<T>(&mut ctx, lookup);
                }
            }
        }

        if let Some(func) = stage.pause_func {
            if func(plan, face, ctx.buffer) {
                ctx.digest = ctx.buffer.digest();
            }
        }
    }
}

fn apply_string<T: LayoutTable>(ctx: &mut OT::hb_ot_apply_context_t, lookup: &LookupInfo) {
    if ctx.buffer.is_empty() || ctx.lookup_mask() == 0 {
        return;
    }

    ctx.lookup_props = lookup.props();
    ctx.update_matchers();

    if !lookup.is_reverse() {
        // in/out forward substitution/positioning
        if !T::IN_PLACE {
            ctx.buffer.clear_output();
        }
        ctx.buffer.idx = 0;
        apply_forward(ctx, lookup);

        if !T::IN_PLACE {
            ctx.buffer.sync();
        }
    } else {
        // in-place backward substitution/positioning
        assert!(!ctx.buffer.have_output);

        ctx.buffer.idx = ctx.buffer.len - 1;
        apply_backward(ctx, lookup);
    }
}

fn apply_forward(ctx: &mut OT::hb_ot_apply_context_t, lookup: &LookupInfo) -> bool {
    let mut ret = false;
    let Some(mut cache) = ctx
        .face
        .ot_tables
        .subtable_cache(ctx.table_index, lookup.clone())
    else {
        return false;
    };

    let use_hot_subtable_cache = lookup.cache_enter(ctx, &mut cache);

    while ctx.buffer.idx < ctx.buffer.len && ctx.buffer.successful {
        let cur = ctx.buffer.cur(0);
        if (cur.mask & ctx.lookup_mask()) != 0
            && check_glyph_property(ctx.face, cur, ctx.lookup_props)
            && lookup
                .apply(ctx, &mut cache, use_hot_subtable_cache)
                .is_some()
        {
            ret = true;
        } else {
            ctx.buffer.next_glyph();
        }
    }

    if use_hot_subtable_cache {
        lookup.cache_leave(ctx, &mut cache);
    }

    ret
}

fn apply_backward(ctx: &mut OT::hb_ot_apply_context_t, lookup: &LookupInfo) -> bool {
    let mut ret = false;
    let Some(mut cache) = ctx
        .face
        .ot_tables
        .subtable_cache(ctx.table_index, lookup.clone())
    else {
        return false;
    };
    loop {
        let cur = ctx.buffer.cur(0);
        ret |= (cur.mask & ctx.lookup_mask()) != 0
            && check_glyph_property(ctx.face, cur, ctx.lookup_props)
            && lookup.apply(ctx, &mut cache, false).is_some();

        if ctx.buffer.idx == 0 {
            break;
        }

        ctx.buffer.idx -= 1;
    }
    ret
}

/* unicode_props */

/* Design:
 * unicode_props() is a two-byte number.  The low byte includes:
 * - Modified General_Category: 5 bits.
 * - A bit each for:
 *   * Is it Default_Ignorable(); we have a modified Default_Ignorable().
 *   * Whether it's one of the four Mongolian Free Variation Selectors,
 *     CGJ, or other characters that are hidden but should not be ignored
 *     like most other Default_Ignorable()s do during GSUB matching.
 *   * Whether it's a grapheme continuation.
 *
 * The high-byte has different meanings, switched by the Gen-Cat:
 * - For Mn,Mc,Me: the modified Combining_Class.
 * - For Cf: whether it's ZWJ, ZWNJ, or something else.
 * - For Ws: index of which space character this is, if space fallback
 *   is needed, ie. we don't set this by default, only if asked to.
 *
 * Above I said "modified" General_Category. This is because we need to
 * remember Variation Selectors, and we don't have bits left. So we
 * change their Gen_Cat from Mn to Cf, and use a bit of the high byte to
 * remember them.
 */

//  enum hb_unicode_props_flags_t {
//     UPROPS_MASK_GEN_CAT	= 0x001Fu,
//     UPROPS_MASK_IGNORABLE	= 0x0020u,
//     UPROPS_MASK_HIDDEN	= 0x0040u, /* MONGOLIAN FREE VARIATION SELECTOR 1..4, or TAG characters */
//     UPROPS_MASK_CONTINUATION=0x0080u,

//     /* If GEN_CAT=FORMAT, top byte masks: */
//     UPROPS_MASK_Cf_ZWJ	= 0x0100u,
//     UPROPS_MASK_Cf_ZWNJ	= 0x0200u
//   };
//   HB_MARK_AS_FLAG_T (hb_unicode_props_flags_t);

//   static inline void
//   _hb_glyph_info_set_unicode_props (hb_glyph_info_t *info, hb_buffer_t *buffer)
//   {
//     hb_unicode_funcs_t *unicode = buffer->unicode;
//     unsigned int u = info->codepoint;
//     unsigned int gen_cat = (unsigned int) unicode->general_category (u);
//     unsigned int props = gen_cat;

//     if (u >= 0x80u)
//     {
//       buffer->scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_NON_ASCII;

//       if (unlikely (unicode->is_default_ignorable (u)))
//       {
//         buffer->scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_DEFAULT_IGNORABLES;
//         props |=  UPROPS_MASK_IGNORABLE;
//         if (u == 0x200Cu) props |= UPROPS_MASK_Cf_ZWNJ;
//         else if (u == 0x200Du) props |= UPROPS_MASK_Cf_ZWJ;
//         /* Mongolian Free Variation Selectors need to be remembered
//          * because although we need to hide them like default-ignorables,
//          * they need to non-ignorable during shaping.  This is similar to
//          * what we do for joiners in Indic-like shapers, but since the
//          * FVSes are GC=Mn, we have use a separate bit to remember them.
//          * Fixes:
//          * https://github.com/harfbuzz/harfbuzz/issues/234 */
//         else if (unlikely (hb_in_ranges<hb_codepoint_t> (u, 0x180Bu, 0x180Du, 0x180Fu, 0x180Fu))) props |= UPROPS_MASK_HIDDEN;
//         /* TAG characters need similar treatment. Fixes:
//          * https://github.com/harfbuzz/harfbuzz/issues/463 */
//         else if (unlikely (hb_in_range<hb_codepoint_t> (u, 0xE0020u, 0xE007Fu))) props |= UPROPS_MASK_HIDDEN;
//         /* COMBINING GRAPHEME JOINER should not be skipped; at least some times.
//          * https://github.com/harfbuzz/harfbuzz/issues/554 */
//         else if (unlikely (u == 0x034Fu))
//         {
//       buffer->scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_CGJ;
//       props |= UPROPS_MASK_HIDDEN;
//         }
//       }

//       if (unlikely (HB_UNICODE_GENERAL_CATEGORY_IS_MARK (gen_cat)))
//       {
//         props |= UPROPS_MASK_CONTINUATION;
//         props |= unicode->modified_combining_class (u)<<8;
//       }
//     }

//     info->unicode_props() = props;
//   }

#[inline]
pub fn _hb_glyph_info_set_general_category(
    info: &mut hb_glyph_info_t,
    gen_cat: hb_unicode_general_category_t,
) {
    /* Clears top-byte. */
    let gen_cat = gen_cat.to_u32();
    let n =
        (gen_cat as u16) | (info.unicode_props() & (0xFF & !UnicodeProps::GENERAL_CATEGORY.bits()));
    info.set_unicode_props(n);
}

#[inline]
pub fn _hb_glyph_info_get_general_category(
    info: &hb_glyph_info_t,
) -> hb_unicode_general_category_t {
    let n = info.unicode_props() & UnicodeProps::GENERAL_CATEGORY.bits();
    hb_unicode_general_category_t::from_u32(n as u32)
}

#[inline]
pub fn _hb_glyph_info_is_unicode_mark(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_get_general_category(info).is_mark()
}

#[inline]
pub(crate) fn _hb_glyph_info_set_modified_combining_class(
    info: &mut hb_glyph_info_t,
    modified_class: u8,
) {
    if !_hb_glyph_info_is_unicode_mark(info) {
        return;
    }

    let n = ((modified_class as u16) << 8) | (info.unicode_props() & 0xFF);
    info.set_unicode_props(n);
}

#[inline]
pub fn _hb_glyph_info_get_modified_combining_class(info: &hb_glyph_info_t) -> u8 {
    if _hb_glyph_info_is_unicode_mark(info) {
        (info.unicode_props() >> 8) as u8
    } else {
        0
    }
}

// TODO: use
// #[inline]
// pub fn info_cc(info: &hb_glyph_info_t) -> u8 {
//     _hb_glyph_info_get_modified_combining_class(info)
// }

#[inline]
pub(crate) fn _hb_glyph_info_is_unicode_space(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_get_general_category(info) == hb_unicode_general_category_t::SpaceSeparator
}

#[inline]
pub(crate) fn _hb_glyph_info_set_unicode_space_fallback_type(
    info: &mut hb_glyph_info_t,
    s: hb_unicode_funcs_t::space_t,
) {
    if !_hb_glyph_info_is_unicode_space(info) {
        return;
    }

    let n = ((s as u16) << 8) | (info.unicode_props() & 0xFF);
    info.set_unicode_props(n);
}

#[inline]
pub(crate) fn _hb_glyph_info_get_unicode_space_fallback_type(
    info: &hb_glyph_info_t,
) -> hb_unicode_funcs_t::space_t {
    if _hb_glyph_info_is_unicode_space(info) {
        (info.unicode_props() >> 8) as u8
    } else {
        hb_unicode_funcs_t::NOT_SPACE
    }
}

#[inline]
pub(crate) fn _hb_glyph_info_is_variation_selector(info: &hb_glyph_info_t) -> bool {
    let a = _hb_glyph_info_get_general_category(info) == hb_unicode_general_category_t::Format;
    let b = (info.unicode_props() & UnicodeProps::CF_VS.bits()) != 0;
    a && b
}

#[inline]
pub(crate) fn _hb_glyph_info_set_variation_selector(info: &mut hb_glyph_info_t, customize: bool) {
    if customize {
        _hb_glyph_info_set_general_category(info, hb_unicode_general_category_t::Format);
        info.set_unicode_props(info.unicode_props() | UnicodeProps::CF_VS.bits());
    } else {
        // Reset to their original condition
        _hb_glyph_info_set_general_category(info, hb_unicode_general_category_t::NonspacingMark);
    }
}

#[inline]
pub(crate) fn _hb_glyph_info_is_default_ignorable(info: &hb_glyph_info_t) -> bool {
    let n = info.unicode_props() & UnicodeProps::IGNORABLE.bits();
    n != 0 && !_hb_glyph_info_substituted(info)
}

#[inline]
pub(crate) fn _hb_glyph_info_set_default_ignorable(info: &mut hb_glyph_info_t) {
    info.set_unicode_props(info.unicode_props() | UnicodeProps::IGNORABLE.bits());
}

#[inline]
pub(crate) fn _hb_glyph_info_clear_default_ignorable(info: &mut hb_glyph_info_t) {
    let mut n = info.unicode_props();
    n &= !UnicodeProps::IGNORABLE.bits();
    info.set_unicode_props(n);
}

#[inline]
pub(crate) fn _hb_glyph_info_is_hidden(info: &hb_glyph_info_t) -> bool {
    (info.unicode_props() & UnicodeProps::HIDDEN.bits()) != 0
}

//   static inline void
//   _hb_glyph_info_unhide (hb_glyph_info_t *info)
//   {
//     info->unicode_props() &= ~ UPROPS_MASK_HIDDEN;
//   }

#[inline]
pub(crate) fn _hb_glyph_info_set_continuation(info: &mut hb_glyph_info_t) {
    let mut n = info.unicode_props();
    n |= UnicodeProps::CONTINUATION.bits();
    info.set_unicode_props(n);
}

#[inline]
pub(crate) fn _hb_glyph_info_clear_continuation(info: &mut hb_glyph_info_t) {
    let mut n = info.unicode_props();
    n &= !UnicodeProps::CONTINUATION.bits();
    info.set_unicode_props(n);
}

#[inline]
pub(crate) fn _hb_glyph_info_is_continuation(info: &hb_glyph_info_t) -> bool {
    info.unicode_props() & UnicodeProps::CONTINUATION.bits() != 0
}

pub(crate) fn _hb_grapheme_group_func(_: &hb_glyph_info_t, b: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_is_continuation(b)
}

pub fn _hb_ot_layout_reverse_graphemes(buffer: &mut hb_buffer_t) {
    // MONOTONE_GRAPHEMES was already applied and is taken care of by _hb_grapheme_group_func.
    // So we just check for MONOTONE_CHARACTERS here.
    buffer.reverse_groups(
        _hb_grapheme_group_func,
        buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS,
    );
}

#[inline]
pub(crate) fn _hb_glyph_info_is_unicode_format(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_get_general_category(info) == hb_unicode_general_category_t::Format
}

#[inline]
pub(crate) fn _hb_glyph_info_is_zwnj(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_is_unicode_format(info)
        && (info.unicode_props() & UnicodeProps::CF_ZWNJ.bits() != 0)
}

#[inline]
pub(crate) fn _hb_glyph_info_is_zwj(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_is_unicode_format(info)
        && (info.unicode_props() & UnicodeProps::CF_ZWJ.bits() != 0)
}

//   static inline bool
//   _hb_glyph_info_is_joiner (const hb_glyph_info_t *info)
//   {
//     return _hb_glyph_info_is_unicode_format (info) && (info->unicode_props() & (UPROPS_MASK_Cf_ZWNJ|UPROPS_MASK_Cf_ZWJ));
//   }

//   static inline void
//   _hb_glyph_info_flip_joiners (hb_glyph_info_t *info)
//   {
//     if (!_hb_glyph_info_is_unicode_format (info))
//       return;
//     info->unicode_props() ^= UPROPS_MASK_Cf_ZWNJ | UPROPS_MASK_Cf_ZWJ;
//   }

#[inline]
pub(crate) fn _hb_glyph_info_is_aat_deleted(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_is_unicode_format(info)
        && (info.unicode_props() & UnicodeProps::CF_AAT_DELETED.bits() != 0)
}

#[inline]
pub(crate) fn _hb_glyph_info_set_aat_deleted(info: &mut hb_glyph_info_t) {
    _hb_glyph_info_set_general_category(info, hb_unicode_general_category_t::Format);
    info.set_unicode_props(
        info.unicode_props() | UnicodeProps::CF_AAT_DELETED.bits() | UnicodeProps::HIDDEN.bits(),
    );
}

//   /* lig_props: aka lig_id / lig_comp
//    *
//    * When a ligature is formed:
//    *
//    *   - The ligature glyph and any marks in between all the same newly allocated
//    *     lig_id,
//    *   - The ligature glyph will get lig_num_comps set to the number of components
//    *   - The marks get lig_comp > 0, reflecting which component of the ligature
//    *     they were applied to.
//    *   - This is used in GPOS to attach marks to the right component of a ligature
//    *     in MarkLigPos,
//    *   - Note that when marks are ligated together, much of the above is skipped
//    *     and the current lig_id reused.
//    *
//    * When a multiple-substitution is done:
//    *
//    *   - All resulting glyphs will have lig_id = 0,
//    *   - The resulting glyphs will have lig_comp = 0, 1, 2, ... respectively.
//    *   - This is used in GPOS to attach marks to the first component of a
//    *     multiple substitution in MarkBasePos.
//    *
//    * The numbers are also used in GPOS to do mark-to-mark positioning only
//    * to marks that belong to the same component of the same ligature.
//    */
//   static inline void
//   _hb_glyph_info_clear_lig_props (hb_glyph_info_t *info)
//   {
//     info->lig_props() = 0;
//   }

const IS_LIG_BASE: u8 = 0x10;

#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_ligature(
    info: &mut hb_glyph_info_t,
    lig_id: u8,
    lig_num_comps: u8,
) {
    info.set_lig_props((lig_id << 5) | IS_LIG_BASE | (lig_num_comps & 0x0F));
}

#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_mark(
    info: &mut hb_glyph_info_t,
    lig_id: u8,
    lig_comp: u8,
) {
    info.set_lig_props((lig_id << 5) | (lig_comp & 0x0F));
}

#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_component(info: &mut hb_glyph_info_t, comp: u8) {
    _hb_glyph_info_set_lig_props_for_mark(info, 0, comp);
}

#[inline]
pub(crate) fn _hb_glyph_info_get_lig_id(info: &hb_glyph_info_t) -> u8 {
    info.lig_props() >> 5
}

#[inline]
pub(crate) fn _hb_glyph_info_ligated_internal(info: &hb_glyph_info_t) -> bool {
    info.lig_props() & IS_LIG_BASE != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_get_lig_comp(info: &hb_glyph_info_t) -> u8 {
    if _hb_glyph_info_ligated_internal(info) {
        0
    } else {
        info.lig_props() & 0x0F
    }
}

#[inline]
pub(crate) fn _hb_glyph_info_get_lig_num_comps(info: &hb_glyph_info_t) -> u8 {
    if info.glyph_props() & GlyphPropsFlags::LIGATURE.bits() != 0
        && _hb_glyph_info_ligated_internal(info)
    {
        info.lig_props() & 0x0F
    } else {
        1
    }
}

//   /* glyph_props: */
//   static inline void
//   _hb_glyph_info_set_glyph_props (hb_glyph_info_t *info, unsigned int props)
//   {
//     info->glyph_props() = props;
//   }

//   static inline unsigned int
//   _hb_glyph_info_get_glyph_props (const hb_glyph_info_t *info)
//   {
//     return info->glyph_props();
//   }

#[inline]
pub(crate) fn _hb_glyph_info_is_base_glyph(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::BASE_GLYPH.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_is_ligature(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::LIGATURE.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_is_mark(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::MARK.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_substituted(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::SUBSTITUTED.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_ligated(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::LIGATED.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_multiplied(info: &hb_glyph_info_t) -> bool {
    info.glyph_props() & GlyphPropsFlags::MULTIPLIED.bits() != 0
}

#[inline]
pub(crate) fn _hb_glyph_info_ligated_and_didnt_multiply(info: &hb_glyph_info_t) -> bool {
    _hb_glyph_info_ligated(info) && !_hb_glyph_info_multiplied(info)
}

#[inline]
pub(crate) fn _hb_glyph_info_clear_ligated_and_multiplied(info: &mut hb_glyph_info_t) {
    let mut n = info.glyph_props();
    n &= !(GlyphPropsFlags::LIGATED | GlyphPropsFlags::MULTIPLIED).bits();
    info.set_glyph_props(n);
}

#[inline]
pub(crate) fn _hb_glyph_info_clear_substituted(info: &mut hb_glyph_info_t) {
    let mut n = info.glyph_props();
    n &= !GlyphPropsFlags::SUBSTITUTED.bits();
    info.set_glyph_props(n);
}

pub fn _hb_clear_substitution_flags(
    _: &hb_ot_shape_plan_t,
    _: &hb_font_t,
    buffer: &mut hb_buffer_t,
) -> bool {
    let len = buffer.len;
    for info in &mut buffer.info[..len] {
        _hb_glyph_info_clear_substituted(info);
    }

    false
}
