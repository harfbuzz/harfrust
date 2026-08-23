pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use crate::hb::aat::layout_common::SafeToBreakAccel;
use crate::hb::aat::layout_kerx_table::KerxSubtableCache;
use crate::hb::aat::layout_morx_table::{MorxSubtableCache, MorxSubtableDescriptor};
use crate::hb::kerning::KernSubtableCache;
use crate::hb::ot::OtTables;
use crate::hb::tables::TableRanges;
use alloc::vec::Vec;
use read_fonts::{
    tables::{ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, morx::Morx, trak::Trak},
    FontRef, TableProvider,
};

#[derive(Default)]
pub struct AatCache {
    safe_to_break: SafeToBreakAccel,
    pub morx: Vec<MorxSubtableCache>,
    pub morx_descriptors: Vec<MorxSubtableDescriptor>,
    pub kern: Vec<KernSubtableCache>,
    pub kerx: Vec<KerxSubtableCache>,
}

impl AatCache {
    #[allow(unused)]
    pub fn new<'a>(font: &impl TableProvider<'a>) -> Self {
        let mut cache = Self::default();
        let num_glyphs = font
            .maxp()
            .map(|maxp| maxp.num_glyphs() as u32)
            .unwrap_or_default();
        if let Ok(morx) = font.morx() {
            let morx_base = morx.offset_data().as_bytes().as_ptr() as usize;
            for (chain_index, chain) in morx.chains().iter().enumerate() {
                let Ok(chain) = chain else {
                    continue;
                };
                for subtable in chain.subtables().iter() {
                    let Ok(subtable) = subtable else {
                        continue;
                    };
                    let entry =
                        MorxSubtableCache::new(&subtable, num_glyphs, &mut cache.safe_to_break);
                    cache.morx_descriptors.push(MorxSubtableCache::descriptor(
                        chain_index,
                        &subtable,
                        morx_base,
                    ));
                    cache.morx.push(entry);
                }
            }
        }
        if let Ok(kern) = font.kern() {
            for subtable in kern.subtables() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                let entry = KernSubtableCache::new(&subtable, num_glyphs, &mut cache.safe_to_break);
                cache.kern.push(entry);
            }
        }
        if let Ok(kerx) = font.kerx() {
            for subtable in kerx.subtables().iter() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                let entry = KerxSubtableCache::new(&subtable, num_glyphs, &mut cache.safe_to_break);
                cache.kerx.push(entry);
            }
        }
        cache
    }
}

#[derive(Clone, Default)]
pub struct AatTables<'a> {
    pub(crate) safe_to_break: Option<&'a SafeToBreakAccel>,
    pub morx: Option<(
        Morx<'a>,
        &'a [MorxSubtableCache],
        &'a [MorxSubtableDescriptor],
    )>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<(Kern<'a>, &'a [KernSubtableCache])>,
    pub kerx: Option<(Kerx<'a>, &'a [KerxSubtableCache])>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
}

use crate::algs::HB_CODEPOINT_ENCODE3 as encode3;

/// Blocklist specific broken morx tables identified by the combination of
/// morx, GSUB, and GDEF table lengths.
fn is_morx_blocklisted(morx_len: u32, gsub_len: u32, gdef_len: u32) -> bool {
    const BLOCKLIST: &[u64] = &[
        // AALMAGHRIBI.ttf — https://github.com/harfbuzz/harfbuzz/issues/4108
        encode3(19892, 2794, 340),
    ];
    let key = encode3(morx_len, gsub_len, gdef_len);
    BLOCKLIST.contains(&key)
}

impl<'a> AatTables<'a> {
    pub fn new(font: &FontRef<'a>, cache: &'a AatCache, table_ranges: &TableRanges) -> Self {
        let morx = if is_morx_blocklisted(
            table_ranges.morx.len(),
            table_ranges.gsub.len(),
            table_ranges.gdef.len(),
        ) {
            None
        } else {
            table_ranges.morx.resolve_table(font).map(|table| {
                (
                    table,
                    cache.morx.as_slice(),
                    cache.morx_descriptors.as_slice(),
                )
            })
        };
        let ankr = table_ranges.ankr.resolve_table(font);
        let kern = table_ranges
            .kern
            .resolve_table(font)
            .map(|table| (table, cache.kern.as_slice()));
        let kerx = table_ranges
            .kerx
            .resolve_table(font)
            .map(|table| (table, cache.kerx.as_slice()));
        let trak = table_ranges.trak.resolve_table(font);
        let feat = table_ranges.feat.resolve_table(font);
        Self {
            safe_to_break: Some(&cache.safe_to_break),
            morx,
            ankr,
            kern,
            kerx,
            trak,
            feat,
        }
    }

    pub fn from_tables(
        font: &impl TableProvider<'a>,
        ot_tables: &OtTables,
        cache: &'a AatCache,
    ) -> Self {
        let morx = if let Ok(morx) = font.morx() {
            let gsub_len = ot_tables
                .gsub
                .as_ref()
                .map_or(0, |table| table.table.offset_data().len() as u32);
            let gdef_len = ot_tables
                .gdef
                .table
                .as_ref()
                .map_or(0, |table| table.offset_data().len() as u32);
            if is_morx_blocklisted(morx.offset_data().len() as u32, gsub_len, gdef_len) {
                None
            } else {
                Some((
                    morx,
                    cache.morx.as_slice(),
                    cache.morx_descriptors.as_slice(),
                ))
            }
        } else {
            None
        };
        let ankr = font.ankr().ok();
        let kern = font.kern().ok().map(|table| (table, cache.kern.as_slice()));
        let kerx = font.kerx().ok().map(|table| (table, cache.kerx.as_slice()));
        let trak = font.trak().ok();
        let feat = font.feat().ok();
        Self {
            safe_to_break: Some(&cache.safe_to_break),
            morx,
            ankr,
            kern,
            kerx,
            trak,
            feat,
        }
    }
}
