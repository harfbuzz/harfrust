pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use crate::hb::aat::layout_kerx_table::KerxSubtableCache;
use crate::hb::aat::layout_morx_table::MorxSubtableCache;
use crate::hb::kerning::KernSubtableCache;
use alloc::vec::Vec;
use read_fonts::{
    tables::{ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, morx::Morx, trak::Trak},
    TableProvider,
};

#[derive(Default)]
pub struct AatCache {
    pub morx: Vec<MorxSubtableCache>,
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
            let chains = morx.chains();
            for chain in morx.chains().iter() {
                let Ok(chain) = chain else {
                    continue;
                };
                for subtable in chain.subtables().iter() {
                    let Ok(subtable) = subtable else {
                        continue;
                    };
                    cache
                        .morx
                        .push(MorxSubtableCache::new(&subtable, num_glyphs));
                }
            }
        }
        if let Ok(kern) = font.kern() {
            for subtable in kern.subtables() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                cache
                    .kern
                    .push(KernSubtableCache::new(&subtable, num_glyphs));
            }
        }
        if let Ok(kerx) = font.kerx() {
            for subtable in kerx.subtables().iter() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                cache
                    .kerx
                    .push(KerxSubtableCache::new(&subtable, num_glyphs));
            }
        }
        cache
    }
}

#[derive(Clone, Default)]
pub struct AatTables<'a> {
    pub morx: Option<(Morx<'a>, &'a [MorxSubtableCache])>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<(Kern<'a>, &'a [KernSubtableCache])>,
    pub kerx: Option<(Kerx<'a>, &'a [KerxSubtableCache])>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
    pub apply_trak: bool,
}

use crate::hb::algs::HB_CODEPOINT_ENCODE3 as encode3;

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
    pub fn new(font: &impl TableProvider<'a>, cache: &'a AatCache) -> Self {
        let morx = font.morx().ok();
        let ankr = font.ankr().ok();
        let kern = font.kern().ok().map(|t| (t, cache.kern.as_slice()));
        let kerx = font.kerx().ok().map(|t| (t, cache.kerx.as_slice()));
        let trak = font.trak().ok();
        let feat = font.feat().ok();
        let morx_len = morx
            .as_ref()
            .map(|t| t.offset_data().len() as u32)
            .unwrap_or(0);
        let gsub_len = font
            .gsub()
            .map(|t| t.offset_data().len() as u32)
            .unwrap_or(0);
        let gdef_len = font
            .gdef()
            .map(|t| t.offset_data().len() as u32)
            .unwrap_or(0);
        let morx = if is_morx_blocklisted(morx_len, gsub_len, gdef_len) {
            None
        } else {
            morx.map(|t| (t, cache.morx.as_slice()))
        };
        // According to Ned, trak is applied by default for "modern fonts", as detected by presence of STAT table.
        // https://github.com/googlefonts/fontations/issues/1492
        let apply_trak = trak.is_some() && font.stat().is_ok();
        Self {
            morx,
            ankr,
            kern,
            kerx,
            trak,
            feat,
            apply_trak,
        }
    }
}
