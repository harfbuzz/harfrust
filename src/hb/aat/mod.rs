pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use crate::hb::ot_layout_gsubgpos::MappingCache;
use crate::hb::tables::TableOffsets;
use alloc::vec::Vec;
use read_fonts::{
    tables::{ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, morx::Morx, trak::Trak},
    FontRef, TableProvider,
};

type ClassCache = MappingCache;

#[derive(Default)]
pub struct AatCache {
    pub morx: Vec<MorxSubtableCache>,
    pub kerx: Vec<KerxSubtableCache>,
}

impl AatCache {
    #[allow(unused)]
    pub fn new(font: &FontRef) -> Self {
        let mut cache = Self::default();
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

                    cache.morx.push(MorxSubtableCache {
                        class_cache: ClassCache::new(),
                    });
                }
            }
        }
        if let Ok(kerx) = font.kerx() {
            // TODO: fill cache.kerx
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
    class_cache: ClassCache,
}

pub struct KerxSubtableCache {
    // TODO: and here?
}
