use crate::hb::tables::{legacy_symbol_font_page, SelectedCmapSubtable, TableRanges};

use super::cache::hb_cache_t;
use read_fonts::{
    tables::cmap::{Cmap, Cmap14, CmapSubtable, MapVariant},
    types::GlyphId,
    FontRef, TableProvider,
};

pub type cache_t = hb_cache_t<21, 19, 256, 32>;

#[derive(Clone)]
pub struct Charmap<'a> {
    subtable: Option<(SelectedCmapSubtable, CmapSubtable<'a>)>,
    vs_subtable: Option<Cmap14<'a>>,
}

impl<'a> Charmap<'a> {
    pub fn new(font: &FontRef<'a>, table_ranges: &TableRanges) -> Self {
        if let Some(cmap) = table_ranges.cmap.resolve_table::<Cmap>(font) {
            let data = cmap.offset_data();
            let records = cmap.encoding_records();
            let subtable = table_ranges
                .cmap_subtable
                .and_then(|s| Some((s, records.get(s.index as usize)?.subtable(data).ok()?)));
            let vs_subtable = table_ranges
                .cmap_vs_subtable
                .and_then(|index| records.get(index as usize))
                .and_then(|rec| rec.subtable(data).ok())
                .and_then(|subtable| match subtable {
                    CmapSubtable::Format14(table) => Some(table),
                    _ => None,
                });
            Self {
                subtable,
                vs_subtable,
            }
        } else {
            Self {
                subtable: None,
                vs_subtable: None,
            }
        }
    }

    pub fn from_tables(font: &impl TableProvider<'a>) -> Self {
        if let Ok(cmap) = font.cmap() {
            let subtable = if let Some((index, record, subtable)) = cmap.best_subtable() {
                Some((
                    SelectedCmapSubtable {
                        index,
                        is_mac_roman: record.is_mac_roman(),
                        is_symbol: record.is_symbol(),
                        symbol_font_page: legacy_symbol_font_page(font.os2().ok().as_ref()),
                    },
                    subtable,
                ))
            } else {
                None
            };
            Self {
                subtable,
                vs_subtable: cmap.uvs_subtable().map(|(_, subtable)| subtable),
            }
        } else {
            Self {
                subtable: None,
                vs_subtable: None,
            }
        }
    }

    pub fn map(&self, mut c: u32) -> Option<GlyphId> {
        let subtable = self.subtable.as_ref()?;
        if subtable.0.is_mac_roman && c > 0x7F {
            c = unicode_to_macroman(c);
        }
        let result = subtable.1.map_codepoint(c);
        if result.is_none() && subtable.0.is_symbol {
            let mapped = match subtable.0.symbol_font_page {
                0xB200 => arabic_pua_map(c, true),
                0xB300 => arabic_pua_map(c, false),
                0 if c <= 0x00FF => 0xF000 + c,
                _ => 0,
            };
            if mapped != 0 {
                return subtable.1.map_codepoint(mapped);
            }
        }
        result
    }

    pub fn map_variant(&self, c: u32, vs: u32) -> Option<GlyphId> {
        let subtable = self.vs_subtable.as_ref()?;
        match subtable.map_variant(c, vs)? {
            MapVariant::UseDefault => self.map(c),
            MapVariant::Variant(gid) => Some(gid),
        }
    }
}

fn arabic_pua_map(c: u32, simplified: bool) -> u32 {
    let Ok(c) = usize::try_from(c) else {
        return 0;
    };
    let mapped = if simplified {
        super::ot_shaper_arabic_pua::_hb_arabic_pua_simp_map(c)
    } else {
        super::ot_shaper_arabic_pua::_hb_arabic_pua_trad_map(c)
    };
    u32::from(mapped)
}

#[rustfmt::skip]
static UNICODE_TO_MACROMAN: &[u16] = &[
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1,
    0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8,
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3,
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC,
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF,
    0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211,
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8,
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153,
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA,
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1,
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC,
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

fn unicode_to_macroman(c: u32) -> u32 {
    let u = c as u16;
    let Some(index) = UNICODE_TO_MACROMAN.iter().position(|m| *m == u) else {
        return 0;
    };
    (0x80 + index) as u32
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn maps_legacy_arabic_symbol_fonts() {
        let simplified = FontRef::new(include_bytes!(
            "../../tests/fonts/in-house/SimpArabicTest.ttf"
        ))
        .unwrap();
        let simplified_ranges = TableRanges::new(&simplified);
        assert_eq!(
            simplified_ranges.cmap_subtable.unwrap().symbol_font_page,
            0xB200
        );
        assert_eq!(
            Charmap::new(&simplified, &simplified_ranges).map(0x0627),
            Some(GlyphId::new(45))
        );

        let traditional = FontRef::new(include_bytes!(
            "../../tests/fonts/in-house/TradArabicTest.ttf"
        ))
        .unwrap();
        let traditional_ranges = TableRanges::new(&traditional);
        assert_eq!(
            traditional_ranges.cmap_subtable.unwrap().symbol_font_page,
            0xB300
        );
        assert_eq!(
            Charmap::new(&traditional, &traditional_ranges).map(0x0627),
            Some(GlyphId::new(65))
        );
    }

    /// KNOWN FAILURE: a cmap entry that resolves to glyph 0 is reported as a
    /// hit rather than a miss.
    ///
    /// Every one of HarfBuzz's `CmapSubtable*::get_glyph` implementations ends
    /// with `if (unlikely (!gid)) return false`, so "the cmap maps this
    /// codepoint to .notdef" and "the cmap does not map this codepoint" are
    /// the same answer there. HarfRust takes whatever `read-fonts` returns,
    /// and `Cmap0::map_codepoint` hands back `Some(GlyphId::new(0))` for an
    /// entry of 0. Callers that ask "does the font have this glyph" then get
    /// the wrong answer: normalisation composes sequences the font cannot
    /// draw, `hide_default_ignorables` keeps glyphs HarfBuzz drops, and the
    /// Arabic fallback synthesises lookups over glyph 0.
    ///
    /// `unicode_to_macroman` above reaches the same place a second way: it
    /// returns 0 for a codepoint outside Mac Roman, and that 0 is then looked
    /// up as a codepoint.
    ///
    /// `TestCMAPMacTurkish.ttf` shows it through the public API. It carries
    /// only a Mac Roman cmap, so shaping five Arabic letters gives one glyph
    /// in HarfRust and five in HarfBuzz: every letter maps to glyph 0, and the
    /// Arabic fallback ligates the run.
    #[test]
    #[ignore = "cmap hits on glyph 0 are not treated as misses"]
    fn a_cmap_entry_of_glyph_zero_is_a_miss() {
        let fonts: [(&str, &[u8]); 2] = [
            (
                "TestCMAPMacTurkish.ttf",
                include_bytes!("../../tests/fonts/text-rendering-tests/TestCMAPMacTurkish.ttf"),
            ),
            (
                "cmap0_font1.otf",
                include_bytes!("../../tests/fonts/aots/cmap0_font1.otf"),
            ),
        ];
        let charmaps: Vec<(&str, Charmap)> = fonts
            .iter()
            .map(|(name, bytes)| {
                let font = FontRef::new(bytes).unwrap();
                let ranges = TableRanges::new(&font);
                (*name, Charmap::new(&font, &ranges))
            })
            .collect();

        hegel::Hegel::new(|tc| {
            let index = tc.draw(hegel::generators::integers::<usize>().max_value(1));
            let codepoint = tc.draw(hegel::generators::integers::<u32>().max_value(0x10_FFFF));
            let (name, charmap) = &charmaps[index];
            assert_ne!(
                charmap.map(codepoint),
                Some(GlyphId::new(0)),
                "{name} maps U+{codepoint:04X} to .notdef"
            );
        })
        .run();
    }
}
