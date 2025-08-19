pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use crate::hb::tables::TableOffsets;
use alloc::vec::Vec;
use read_fonts::{
    tables::{ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, morx::Morx, trak::Trak},
    FontRef, TableProvider,
};

#[derive(Default)]
pub struct AatCache {
    pub morx: Vec<AatSubtableCache>,
    pub kerx: Vec<AatSubtableCache>,
}

impl AatCache {
    #[allow(unused)]
    pub fn new(font: &FontRef) -> Self {
        let mut cache = Self::default();
        if let Ok(morx) = font.morx() {
            // TODO: fill cache.morx
        }
        if let Ok(kerx) = font.kerx() {
            // TODO: fill cache kerx
        }
        cache
    }
}

#[derive(Clone, Default)]
pub struct AatTables<'a> {
    pub morx: Option<TableWithCache<'a, Morx<'a>>>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<Kern<'a>>,
    pub kerx: Option<TableWithCache<'a, Kerx<'a>>>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
}

impl<'a> AatTables<'a> {
    pub fn new(font: &FontRef<'a>, cache: &'a AatCache, table_offsets: &TableOffsets) -> Self {
        let morx = table_offsets
            .morx
            .resolve_table(font)
            .map(|table| TableWithCache {
                table,
                subtables: &cache.morx,
            });
        let ankr = table_offsets.ankr.resolve_table(font);
        let kern = table_offsets.kern.resolve_table(font);
        let kerx = table_offsets
            .kerx
            .resolve_table(font)
            .map(|table| TableWithCache {
                table,
                subtables: &cache.kerx,
            });
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

#[derive(Clone)]
pub struct TableWithCache<'a, T> {
    pub table: T,
    pub subtables: &'a [AatSubtableCache],
}

pub struct AatSubtableCache {
    // TODO: maybe a bitset or something here?
}
