pub mod layout;
pub mod layout_common;
pub mod layout_kerx_table;
pub mod layout_morx_table;
pub mod layout_trak_table;
pub mod map;

use read_fonts::{
    tables::{ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, morx::Morx, trak::Trak},
    FontRef, TableProvider,
};

#[derive(Clone, Default)]
pub struct AatTables<'a> {
    pub morx: Option<Morx<'a>>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<Kern<'a>>,
    pub kerx: Option<Kerx<'a>>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
}

impl<'a> AatTables<'a> {
    pub fn new(font: &FontRef<'a>) -> Self {
        Self {
            morx: font.morx().ok(),
            ankr: font.ankr().ok(),
            kern: font.kern().ok(),
            kerx: font.kerx().ok(),
            trak: font.trak().ok(),
            feat: font.feat().ok(),
        }
    }
}
