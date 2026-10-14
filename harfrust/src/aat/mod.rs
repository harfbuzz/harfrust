pub mod common;
pub mod kern;
pub mod kerx;
pub mod layout;
pub mod map;
pub mod morx;
pub mod trak;

use crate::aat::common::SafeToBreakAccel;
use crate::aat::kern::KernSubtableCache;
use crate::aat::kerx::KerxSubtableCache;
use crate::aat::morx::{MorphSubtableCache, MorphSubtableDescriptor};
use crate::once::Once;
use crate::ot::OtCache;
use alloc::boxed::Box;
use alloc::vec::Vec;
use read_fonts::{
    tables::{
        ankr::Ankr, feat::Feat, kern::Kern, kerx::Kerx, ltag::Ltag, mort::Mort, morx::Morx,
        trak::Trak,
    },
    TableProvider,
};

/// Font-wide AAT table disposition and accelerators initialized on first use.
pub struct AatCache {
    // Keep uninitialized caches small, including in fonts without AAT tables.
    morx: Once<Box<MorphTableCache>>,
    mort: Once<Box<MorphTableCache>>,
    kern: Once<Box<KerningTableCache<KernSubtableCache>>>,
    kerx: Once<Box<KerningTableCache<KerxSubtableCache>>>,
    num_glyphs: u32,
    has_morx_from_tables: bool,
    has_mort_from_tables: bool,
    has_ankr: bool,
    has_kern: bool,
    has_kerx: bool,
    pub(crate) has_trak: bool,
    has_feat: bool,
    has_ltag: bool,
}

impl AatCache {
    pub fn new<'a>(font: &impl TableProvider<'a>, ot_cache: &OtCache) -> Self {
        let num_glyphs = font
            .maxp()
            .map(|maxp| maxp.num_glyphs() as u32)
            .unwrap_or_default();
        let morx = font.morx().ok();
        let mort = font.mort().ok();
        let kern = font.kern().ok();
        let kerx = font.kerx().ok();
        let morx_len = morx
            .as_ref()
            .map_or(0, |table| table.offset_data().len() as u32);
        let active_gdef_len = if ot_cache.has_gdef {
            ot_cache.gdef_len
        } else {
            0
        };
        Self {
            morx: Once::new(),
            mort: Once::new(),
            kern: Once::new(),
            kerx: Once::new(),
            num_glyphs,
            has_morx_from_tables: morx.is_some()
                && !is_morx_blocklisted(morx_len, ot_cache.gsub_len, active_gdef_len),
            has_mort_from_tables: mort.is_some(),
            has_ankr: font.ankr().is_ok(),
            has_kern: kern.is_some(),
            has_kerx: kerx.is_some(),
            has_trak: font.trak().is_ok(),
            has_feat: font.feat().is_ok(),
            has_ltag: font.ltag().is_ok(),
        }
    }
}

#[derive(Default)]
pub(crate) struct MorphTableCache {
    safe_to_break: SafeToBreakAccel,
    subtables: Vec<MorphSubtableCache>,
    descriptors: Vec<MorphSubtableDescriptor>,
}

impl MorphTableCache {
    fn new_morx(morx: &Morx<'_>, num_glyphs: u32) -> Self {
        let mut cache = Self::default();
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
                    MorphSubtableCache::new_morx(&subtable, num_glyphs, &mut cache.safe_to_break);
                cache.descriptors.push(MorphSubtableCache::descriptor_morx(
                    chain_index,
                    &subtable,
                    morx_base,
                ));
                cache.subtables.push(entry);
            }
        }
        cache
    }

    fn new_mort(mort: &Mort<'_>, num_glyphs: u32) -> Self {
        let mut cache = Self::default();
        let mort_base = mort.offset_data().as_bytes().as_ptr() as usize;
        for (chain_index, chain) in mort.chains().iter().enumerate() {
            let Ok(chain) = chain else {
                continue;
            };
            for subtable in chain.subtables().iter() {
                let Ok(subtable) = subtable else {
                    continue;
                };
                let entry =
                    MorphSubtableCache::new_mort(&subtable, num_glyphs, &mut cache.safe_to_break);
                cache.descriptors.push(MorphSubtableCache::descriptor_mort(
                    chain_index,
                    &subtable,
                    mort_base,
                ));
                cache.subtables.push(entry);
            }
        }
        cache
    }
}

pub(crate) struct KerningTableCache<T> {
    safe_to_break: SafeToBreakAccel,
    subtables: Vec<T>,
}

impl KerningTableCache<KernSubtableCache> {
    fn new(kern: &Kern<'_>, num_glyphs: u32) -> Self {
        let mut cache = Self {
            safe_to_break: SafeToBreakAccel::default(),
            subtables: Vec::new(),
        };
        for subtable in kern.subtables() {
            let Ok(subtable) = subtable else {
                continue;
            };
            let entry = KernSubtableCache::new(&subtable, num_glyphs, &mut cache.safe_to_break);
            cache.subtables.push(entry);
        }
        cache
    }
}

impl KerningTableCache<KerxSubtableCache> {
    fn new(kerx: &Kerx<'_>, num_glyphs: u32) -> Self {
        let mut cache = Self {
            safe_to_break: SafeToBreakAccel::default(),
            subtables: Vec::new(),
        };
        for subtable in kerx.subtables().iter() {
            let Ok(subtable) = subtable else {
                continue;
            };
            let entry = KerxSubtableCache::new(&subtable, num_glyphs, &mut cache.safe_to_break);
            cache.subtables.push(entry);
        }
        cache
    }
}

#[derive(Clone, Default)]
pub struct AatData<'a> {
    cache: Option<&'a AatCache>,
    pub morx: Option<Morx<'a>>,
    pub mort: Option<Mort<'a>>,
    pub ankr: Option<Ankr<'a>>,
    pub kern: Option<Kern<'a>>,
    pub kerx: Option<Kerx<'a>>,
    pub trak: Option<Trak<'a>>,
    pub feat: Option<Feat<'a>>,
    pub ltag: Option<Ltag<'a>>,
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

pub(crate) static EMPTY_AAT_DATA: AatData<'static> = AatData {
    cache: None,
    morx: None,
    mort: None,
    ankr: None,
    kern: None,
    kerx: None,
    trak: None,
    feat: None,
    ltag: None,
};

impl<'a> AatData<'a> {
    pub fn from_tables(font: &impl TableProvider<'a>, cache: &'a AatCache) -> Self {
        let morx = cache
            .has_morx_from_tables
            .then(|| font.morx().ok())
            .flatten();
        let mort = cache
            .has_mort_from_tables
            .then(|| font.mort().ok())
            .flatten();
        let ankr = cache.has_ankr.then(|| font.ankr().ok()).flatten();
        let kern = cache.has_kern.then(|| font.kern().ok()).flatten();
        let kerx = cache.has_kerx.then(|| font.kerx().ok()).flatten();
        let trak = cache.has_trak.then(|| font.trak().ok()).flatten();
        let feat = cache.has_feat.then(|| font.feat().ok()).flatten();
        let ltag = cache.has_ltag.then(|| font.ltag().ok()).flatten();
        Self {
            cache: Some(cache),
            morx,
            mort,
            ankr,
            kern,
            kerx,
            trak,
            feat,
            ltag,
        }
    }

    // Access accelerators only when applying the corresponding table, so
    // planning can select GPOS or a preferred AAT table without building them.
    pub(crate) fn morx_cache(&self) -> Option<&'a MorphTableCache> {
        let table = self.morx.as_ref()?;
        let cache = self.cache?;
        Some(
            cache
                .morx
                .get_or_init(|| Box::new(MorphTableCache::new_morx(table, cache.num_glyphs))),
        )
    }

    pub(crate) fn mort_cache(&self) -> Option<&'a MorphTableCache> {
        let table = self.mort.as_ref()?;
        let cache = self.cache?;
        Some(
            cache
                .mort
                .get_or_init(|| Box::new(MorphTableCache::new_mort(table, cache.num_glyphs))),
        )
    }

    pub(crate) fn kern_cache(&self) -> Option<&'a KerningTableCache<KernSubtableCache>> {
        let table = self.kern.as_ref()?;
        let cache = self.cache?;
        Some(cache.kern.get_or_init(|| {
            Box::new(KerningTableCache::<KernSubtableCache>::new(
                table,
                cache.num_glyphs,
            ))
        }))
    }

    pub(crate) fn kerx_cache(&self) -> Option<&'a KerningTableCache<KerxSubtableCache>> {
        let table = self.kerx.as_ref()?;
        let cache = self.cache?;
        Some(cache.kerx.get_or_init(|| {
            Box::new(KerningTableCache::<KerxSubtableCache>::new(
                table,
                cache.num_glyphs,
            ))
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::{
        font::Font, shape, Buffer, SerializeFlags, ShapeOptions, ShapePlan, ShaperFont, Tag,
    };
    use alloc::sync::Arc;
    use read_fonts::{
        model::{Blob, TableFunction},
        FontRef,
    };

    fn initialized(cache: &AatCache) -> [bool; 4] {
        [
            cache.morx.get().is_some(),
            cache.mort.get().is_some(),
            cache.kern.get().is_some(),
            cache.kerx.get().is_some(),
        ]
    }

    fn shape_text(shaper: &ShaperFont<'_, '_>, text: &str, plan: &ShapePlan) -> Buffer {
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        shape(shaper, &mut buffer, ShapeOptions::new().plan(Some(plan))).unwrap();
        buffer
    }

    fn plan_for_text(font: &Font, text: &str) -> ShapePlan {
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        ShapePlan::new(font, buffer.direction(), buffer.script(), None, &[])
    }

    #[test]
    fn gpos_kerning_leaves_legacy_kern_cache_uninitialized() {
        let font = Font::new(
            include_bytes!("../../tests/fonts/rb_custom/PT_Sans-Caption-Web-Regular.ttf").to_vec(),
            0,
        )
        .unwrap();
        let shaper = ShaperFont::new(&font);
        let aat = shaper.layout().aat;
        let cache = aat.cache.unwrap();
        assert!(aat.kern.is_some());
        assert_eq!(initialized(cache), [false; 4]);

        let plan = plan_for_text(&font, "AV");
        assert!(plan.apply_gpos);
        assert!(!plan.apply_kern);
        assert_eq!(initialized(cache), [false; 4]);
        let _ = shape_text(&shaper, "AV", &plan);
        assert_eq!(initialized(cache), [false; 4]);
    }

    #[test]
    fn aat_caches_initialize_on_first_shape_and_are_shared() {
        let cases: &[(&[u8], &str, usize)] = &[
            (
                include_bytes!("../../tests/fonts/in-house/MORXTwentyeight.ttf"),
                "AxEyDyy",
                0,
            ),
            (
                include_bytes!("../../tests/fonts/text-rendering-tests/TestAATMort.ttf"),
                "ABCEFGX",
                1,
            ),
            (
                include_bytes!("../../tests/fonts/text-rendering-tests/TestKERNOne.otf"),
                "uTuT",
                2,
            ),
        ];
        for &(bytes, text, cache_index) in cases {
            let font = Font::new(bytes.to_vec(), 0).unwrap();
            let shaper = ShaperFont::new(&font);
            let aat = shaper.layout().aat;
            let cache = aat.cache.unwrap();
            assert_eq!(initialized(cache), [false; 4]);
            let plan = plan_for_text(&font, text);
            assert_eq!(initialized(cache), [false; 4]);

            let first = shape_text(&shaper, text, &plan);
            let mut expected = [false; 4];
            expected[cache_index] = true;
            assert_eq!(initialized(cache), expected);

            let instance = font.instance_builder().build();
            let second_shaper = ShaperFont::new(&instance);
            assert!(core::ptr::eq(
                cache,
                second_shaper.layout().aat.cache.unwrap()
            ));
            let second = shape_text(&second_shaper, text, &plan);
            assert_eq!(
                first.serialize(Some(&shaper), SerializeFlags::GLYPH_FLAGS),
                second.serialize(Some(&second_shaper), SerializeFlags::GLYPH_FLAGS)
            );
            assert_eq!(initialized(cache), expected);
        }
    }

    #[test]
    fn kerx_cache_initializes_without_legacy_kern_cache() {
        let bytes = include_bytes!("../../tests/fonts/text-rendering-tests/TestKERNOne.otf");
        let source = FontRef::new(bytes).unwrap();
        let source_font = Font::new(bytes.to_vec(), 0).unwrap();
        let charmap = source_font.charmap();
        let left = charmap.map_unicode('u' as u32).unwrap().to_u32() as u16;
        let right = charmap.map_unicode('T' as u32).unwrap().to_u32() as u16;
        // A format-0 kerx subtable with one horizontal pair adjustment of -100.
        #[rustfmt::skip]
        let mut kerx = vec![
            0, 2, 0, 0, // version 2, padding
            0, 0, 0, 1, // nTables
            0, 0, 0, 34, // subtable length
            0, 0, 0, 0, // coverage: horizontal, format 0
            0, 0, 0, 0, // tupleCount
            0, 0, 0, 1, // nPairs
            0, 0, 0, 6, // searchRange
            0, 0, 0, 0, // entrySelector
            0, 0, 0, 0, // rangeShift
        ];
        kerx.extend_from_slice(&left.to_be_bytes());
        kerx.extend_from_slice(&right.to_be_bytes());
        kerx.extend_from_slice(&(-100i16).to_be_bytes());
        let tables = TableFunction::new(Arc::new(move |tag| {
            if tag == Tag::new(b"kerx") {
                Some(Blob::from(kerx.clone()))
            } else {
                source
                    .data_for_tag(tag)
                    .map(|data| Blob::from(data.as_bytes().to_vec()))
            }
        }));
        let font = Font::new(tables, 0).unwrap();
        let shaper = ShaperFont::new(&font);
        let aat = shaper.layout().aat;
        let cache = aat.cache.unwrap();
        assert!(aat.kern.is_some());
        assert!(aat.kerx.is_some());
        let plan = plan_for_text(&font, "uT");
        assert!(plan.apply_kerx);
        assert!(!plan.apply_kern);
        assert_eq!(initialized(cache), [false; 4]);

        let buffer = shape_text(&shaper, "uT", &plan);
        assert_eq!(initialized(cache), [false, false, false, true]);
        let positions = buffer.glyph_positions();
        assert_eq!(
            positions[0].x_advance,
            shaper.default_glyph_h_advance(left.into()) - 50
        );
        assert_eq!(
            positions[1].x_advance,
            shaper.default_glyph_h_advance(right.into()) - 50
        );
        assert_eq!(positions[1].x_offset, -50);
    }

    #[test]
    fn concurrent_first_shapes_share_morx_cache() {
        let font = Font::new(
            include_bytes!("../../tests/fonts/in-house/MORXTwentyeight.ttf").to_vec(),
            0,
        )
        .unwrap();
        let shaper = ShaperFont::new(&font);
        let cache = shaper.layout().aat.cache.unwrap();
        let plan = plan_for_text(&font, "AxEyDyy");
        let barrier = std::sync::Barrier::new(4);
        assert_eq!(initialized(cache), [false; 4]);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let shaper = ShaperFont::new(&font);
                        barrier.wait();
                        let buffer = shape_text(&shaper, "AxEyDyy", &plan);
                        (
                            buffer.serialize(Some(&shaper), SerializeFlags::GLYPH_FLAGS),
                            core::ptr::from_ref(shaper.layout().aat.morx_cache().unwrap()) as usize,
                        )
                    })
                })
                .collect();
            let results: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            for result in &results[1..] {
                assert_eq!(results[0].0, result.0);
                assert_eq!(results[0].1, result.1);
            }
        });
        assert_eq!(initialized(cache), [true, false, false, false]);
    }
}
