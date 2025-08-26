pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use crate::hb::aat::layout_common::TypedCollectGlyphs;
use crate::hb::aat::layout_kerx_table::collect_initial_glyphs as kerx_collect_initial_glyphs;
use crate::hb::aat::layout_kerx_table::SimpleKerning;
use crate::hb::aat::layout_morx_table::collect_initial_glyphs as morx_collect_initial_glyphs;
use crate::hb::aat::layout_morx_table::{
    ContextualCtx, InsertionCtx, LigatureCtx, RearrangementCtx, LIGATURE_MAX_MATCHES,
};
use crate::hb::ot_layout_gsubgpos::MappingCache;
use crate::hb::tables::TableOffsets;
use crate::U32Set;
use alloc::vec::Vec;
use read_fonts::tables::aat::ExtendedStateTable;
use read_fonts::types::{FixedSize, GlyphId};
use read_fonts::{
    tables::{
        ankr::Ankr,
        feat::Feat,
        kern::Kern,
        kerx::{Kerx, SubtableKind as KerxSubtableKind},
        morx::{Morx, SubtableKind as MorxSubtableKind},
        trak::Trak,
    },
    FontRef, TableProvider,
};

type ClassCache = MappingCache;

fn get_class<T: bytemuck::AnyBitPattern + FixedSize>(
    machine: &ExtendedStateTable<'_, T>,
    glyph_id: GlyphId,
    cache: &ClassCache,
) -> u16 {
    if let Some(klass) = cache.get(glyph_id.to_u32()) {
        return klass as u16;
    }
    let klass = machine
        .class(glyph_id)
        .unwrap_or(read_fonts::tables::aat::class::OUT_OF_BOUNDS as u16);
    cache.set(glyph_id.to_u32(), klass as u32);
    klass
}

#[derive(Default)]
pub struct AatCache {
    pub morx: Vec<MorxSubtableCache>,
    pub kerx: Vec<KerxSubtableCache>,
}

impl AatCache {
    #[allow(unused)]
    pub fn new(font: &FontRef) -> Self {
        let mut cache = Self::default();
        let num_glyphs = font
            .maxp()
            .map(|maxp| maxp.num_glyphs() as u32)
            .unwrap_or_default();
        if let Ok(morx) = font.morx() {
            let chains = morx.chains();
            for chain in morx.chains().iter() {
                let Ok(chain) = chain else {
                    continue;
                };
                for subtable in chain.subtables().iter() {
                    let Ok(subtable) = subtable else {
                        continue;
                    };
                    let mut glyph_set = U32Set::default();
                    if let Ok(kind) = subtable.kind() {
                        match &kind {
                            MorxSubtableKind::Rearrangement(table) => {
                                let mut c = RearrangementCtx { start: 0, end: 0 };
                                morx_collect_initial_glyphs(
                                    table,
                                    &mut c,
                                    &mut glyph_set,
                                    num_glyphs,
                                );
                            }
                            MorxSubtableKind::Contextual(table) => {
                                let mut c = ContextualCtx {
                                    mark_set: false,
                                    mark: 0,
                                    table: table.clone(),
                                };
                                morx_collect_initial_glyphs(
                                    &table.state_table,
                                    &mut c,
                                    &mut glyph_set,
                                    num_glyphs,
                                );
                            }
                            MorxSubtableKind::Ligature(table) => {
                                let mut c = LigatureCtx {
                                    table: table.clone(),
                                    match_length: 0,
                                    match_positions: [0; LIGATURE_MAX_MATCHES],
                                };
                                morx_collect_initial_glyphs(
                                    &table.state_table,
                                    &mut c,
                                    &mut glyph_set,
                                    num_glyphs,
                                );
                            }
                            MorxSubtableKind::NonContextual(ref lookup) => {
                                lookup.collect_glyphs(&mut glyph_set, num_glyphs);
                            }
                            MorxSubtableKind::Insertion(table) => {
                                let mut c = InsertionCtx {
                                    mark: 0,
                                    glyphs: table.glyphs,
                                };
                                morx_collect_initial_glyphs(
                                    &table.state_table,
                                    &mut c,
                                    &mut glyph_set,
                                    num_glyphs,
                                );
                            }
                        }
                    };
                    cache.morx.push(MorxSubtableCache {
                        glyph_set,
                        class_cache: ClassCache::new(),
                    });
                }
            }
        }
        if let Ok(kerx) = font.kerx() {
            for subtable in kerx.subtables().iter() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                let mut first_set = U32Set::default();
                let mut second_set = U32Set::default();
                if let Ok(kind) = subtable.kind() {
                    match &kind {
                        KerxSubtableKind::Format0(format0) => {
                            format0.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                        }
                        KerxSubtableKind::Format1(format1) => {
                            kerx_collect_initial_glyphs(
                                &format1.state_table,
                                &mut first_set,
                                num_glyphs,
                            );
                        }
                        KerxSubtableKind::Format2(format2) => {
                            format2.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                        }
                        KerxSubtableKind::Format4(format4) => {
                            kerx_collect_initial_glyphs(
                                &format4.state_table,
                                &mut first_set,
                                num_glyphs,
                            );
                        }
                        KerxSubtableKind::Format6(format6) => {
                            format6.collect_glyphs(&mut first_set, &mut second_set, num_glyphs);
                        }
                    }
                };
                cache.kerx.push(KerxSubtableCache {
                    first_set,
                    second_set,
                    class_cache: ClassCache::new(),
                });
            }
        }
        cache
    }
}

#[derive(Clone, Default)]
pub struct AatTables<'a> {
    pub morx: Option<(Morx<'a>, &'a [MorxSubtableCache])>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<Kern<'a>>,
    pub kerx: Option<(Kerx<'a>, &'a [KerxSubtableCache])>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
}

impl<'a> AatTables<'a> {
    pub fn new(font: &FontRef<'a>, cache: &'a AatCache, table_offsets: &TableOffsets) -> Self {
        let morx = table_offsets
            .morx
            .resolve_table(font)
            .map(|table| (table, cache.morx.as_slice()));
        let ankr = table_offsets.ankr.resolve_table(font);
        let kern = table_offsets.kern.resolve_table(font);
        let kerx = table_offsets
            .kerx
            .resolve_table(font)
            .map(|table| (table, cache.kerx.as_slice()));
        let trak = table_offsets.trak.resolve_table(font);
        let feat = table_offsets.feat.resolve_table(font);
        Self {
            morx,
            ankr,
            kern,
            kerx,
            trak,
            feat,
        }
    }
}

pub struct MorxSubtableCache {
    glyph_set: U32Set,
    class_cache: ClassCache,
}

pub struct KerxSubtableCache {
    first_set: U32Set,
    second_set: U32Set,
    class_cache: ClassCache,
}
