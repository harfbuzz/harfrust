use super::ot_layout::TableIndex;
use super::{common::TagExt, set_digest::hb_set_digest_t};
use crate::hb::hb_tag_t;
use alloc::vec::Vec;
use lookup::{LookupCache, LookupInfo, SubtableCache};
use read_fonts::tables::layout::FeatureTableSubstitutionRecord;
use read_fonts::{
    tables::{
        gdef::Gdef,
        gpos::{AnchorTable, DeviceOrVariationIndex, Gpos},
        gsub::{ClassDef, FeatureList, FeatureVariations, Gsub, ScriptList},
        layout::{Feature, LangSys, Script},
        varc::{Condition, CoverageTable},
        variations::{DeltaSetIndex, ItemVariationStore},
    },
    types::{BigEndian, F2Dot14, GlyphId, Offset32},
    FontData, FontRef, ReadError, ResolveOffset, TableProvider,
};

pub mod contextual;
pub mod gpos;
pub mod gsub;
pub mod lookup;

pub struct OtCache {
    pub gsub: LookupCache,
    pub gpos: LookupCache,
    pub gdef_mark_set_digests: Vec<hb_set_digest_t>,
}

impl OtCache {
    pub fn new(font: &FontRef) -> Self {
        let gsub = font
            .gsub()
            .map(|t| {
                let mut cache = LookupCache::new();
                cache.create_all(&t);
                cache
            })
            .unwrap_or_default();
        let gpos = font
            .gpos()
            .map(|t| {
                let mut cache = LookupCache::new();
                cache.create_all(&t);
                cache
            })
            .unwrap_or_default();
        let mut gdef_mark_set_digests = Vec::new();
        if let Ok(gdef) = font.gdef() {
            if let Some(Ok(mark_sets)) = gdef.mark_glyph_sets_def() {
                gdef_mark_set_digests.extend(mark_sets.coverages().iter().map(|set| {
                    set.ok()
                        .map(|coverage| hb_set_digest_t::from_coverage(&coverage))
                        .unwrap_or_default()
                }));
            }
        }
        Self {
            gsub,
            gpos,
            gdef_mark_set_digests,
        }
    }
}

#[derive(Clone)]
pub struct GsubTable<'a> {
    pub table: Gsub<'a>,
    pub lookups: &'a LookupCache,
}

impl crate::hb::ot_layout::LayoutTable for GsubTable<'_> {
    const INDEX: TableIndex = TableIndex::GSUB;
    const IN_PLACE: bool = false;

    fn get_lookup(&self, index: u16) -> Option<&LookupInfo> {
        let lookup = self.lookups.get(index)?;
        (lookup.subtables_count > 0).then_some(lookup)
    }
}

#[derive(Clone)]
pub struct GposTable<'a> {
    pub table: Gpos<'a>,
    pub lookups: &'a LookupCache,
}

impl crate::hb::ot_layout::LayoutTable for GposTable<'_> {
    const INDEX: TableIndex = TableIndex::GPOS;
    const IN_PLACE: bool = true;

    fn get_lookup(&self, index: u16) -> Option<&LookupInfo> {
        let lookup = self.lookups.get(index)?;
        (lookup.subtables_count > 0).then_some(lookup)
    }
}

#[derive(Clone, Default)]
pub struct GdefTable<'a> {
    table: Option<Gdef<'a>>,
    classes: Option<ClassDef<'a>>,
    mark_classes: Option<ClassDef<'a>>,
    mark_sets: Option<(FontData<'a>, &'a [BigEndian<Offset32>])>,
}

impl<'a> GdefTable<'a> {
    fn new(font: &FontRef<'a>) -> Self {
        if let Ok(gdef) = font.gdef() {
            let classes = gdef.glyph_class_def().transpose().ok().flatten();
            let mark_classes = gdef.mark_attach_class_def().transpose().ok().flatten();
            let mark_sets = gdef
                .mark_glyph_sets_def()
                .transpose()
                .ok()
                .flatten()
                .map(|sets| (sets.offset_data(), sets.coverage_offsets()));
            Self {
                table: Some(gdef),
                classes,
                mark_classes,
                mark_sets,
            }
        } else {
            Self::default()
        }
    }
}

#[derive(Clone)]
pub struct OtTables<'a> {
    pub gsub: Option<GsubTable<'a>>,
    pub gpos: Option<GposTable<'a>>,
    pub gdef: GdefTable<'a>,
    pub gdef_mark_set_digests: &'a [hb_set_digest_t],
    pub coords: &'a [F2Dot14],
    pub var_store: Option<ItemVariationStore<'a>>,
}

impl<'a> OtTables<'a> {
    pub fn new(font: &FontRef<'a>, cache: &'a OtCache, coords: &'a [F2Dot14]) -> Self {
        let gsub = font.gsub().ok().map(|table| GsubTable {
            table,
            lookups: &cache.gsub,
        });
        let gpos = font.gpos().ok().map(|table| GposTable {
            table,
            lookups: &cache.gpos,
        });
        let coords = if coords.iter().any(|coord| *coord != F2Dot14::ZERO) {
            coords
        } else {
            &[]
        };
        let gdef = GdefTable::new(font);
        let var_store = if !coords.is_empty() {
            gdef.table
                .as_ref()
                .and_then(|gdef| gdef.item_var_store().transpose().ok().flatten())
        } else {
            None
        };
        Self {
            gsub,
            gpos,
            gdef,
            gdef_mark_set_digests: &cache.gdef_mark_set_digests,
            var_store,
            coords,
        }
    }

    pub fn has_glyph_classes(&self) -> bool {
        self.gdef.classes.is_some()
    }

    pub fn glyph_class(&self, glyph_id: u32) -> u16 {
        self.gdef
            .classes
            .as_ref()
            .map_or(0, |class_def| class_def.get((glyph_id as u16).into()))
    }

    pub fn glyph_mark_attachment_class(&self, glyph_id: u32) -> u16 {
        self.gdef
            .mark_classes
            .as_ref()
            .map_or(0, |class_def| class_def.get((glyph_id as u16).into()))
    }

    pub fn is_mark_glyph(&self, glyph_id: u32, set_index: u16) -> bool {
        if self
            .gdef_mark_set_digests
            .get(set_index as usize)
            .is_some_and(|digest| digest.may_have_glyph(glyph_id.into()))
        {
            self.gdef
                .mark_sets
                .as_ref()
                .and_then(|(data, offsets)| Some((data, offsets.get(set_index as usize)?.get())))
                .and_then(|(data, offset)| offset.resolve::<CoverageTable>(*data).ok())
                .is_some_and(|coverage| coverage.get(glyph_id).is_some())
        } else {
            false
        }
    }

    pub fn table_data_and_lookups(
        &self,
        table_index: TableIndex,
    ) -> Option<(&'a [u8], &'a LookupCache)> {
        if table_index == TableIndex::GSUB {
            let table = self.gsub.as_ref()?;
            Some((table.table.offset_data().as_bytes(), table.lookups))
        } else {
            let table = self.gpos.as_ref()?;
            Some((table.table.offset_data().as_bytes(), table.lookups))
        }
    }

    pub fn subtable_cache(
        &self,
        table_index: TableIndex,
        lookup: LookupInfo,
    ) -> Option<SubtableCache<'a>> {
        let (table_data, lookups) = self.table_data_and_lookups(table_index)?;
        Some(SubtableCache::new(table_data, lookups, lookup))
    }

    pub fn subtable_cache_for_index(
        &self,
        table_index: TableIndex,
        lookup_index: u16,
    ) -> Option<SubtableCache<'a>> {
        let (table_data, lookups) = self.table_data_and_lookups(table_index)?;
        let lookup = lookups.get(lookup_index)?;
        Some(SubtableCache::new(table_data, lookups, lookup.clone()))
    }

    pub(super) fn resolve_anchor(&self, anchor: &AnchorTable) -> (i32, i32) {
        let mut x = anchor.x_coordinate() as i32;
        let mut y = anchor.y_coordinate() as i32;
        if let Some(vs) = self.var_store.as_ref() {
            let delta = |val: Option<Result<DeviceOrVariationIndex<'_>, ReadError>>| match val {
                Some(Ok(DeviceOrVariationIndex::VariationIndex(varix))) => vs
                    .compute_delta(
                        DeltaSetIndex {
                            outer: varix.delta_set_outer_index(),
                            inner: varix.delta_set_inner_index(),
                        },
                        self.coords,
                    )
                    .unwrap_or_default(),
                _ => 0,
            };
            x += delta(anchor.x_device());
            y += delta(anchor.y_device());
        }
        (x, y)
    }
}

pub enum LayoutTable<'a> {
    Gsub(Gsub<'a>),
    Gpos(Gpos<'a>),
}

impl<'a> LayoutTable<'a> {
    fn script_list(&self) -> Option<ScriptList<'a>> {
        match self {
            Self::Gsub(gsub) => gsub.script_list().ok(),
            Self::Gpos(gpos) => gpos.script_list().ok(),
        }
    }

    fn feature_list(&self) -> Option<FeatureList<'a>> {
        match self {
            Self::Gsub(gsub) => gsub.feature_list().ok(),
            Self::Gpos(gpos) => gpos.feature_list().ok(),
        }
    }

    fn feature_variations(&self) -> Option<FeatureVariations<'a>> {
        match self {
            Self::Gsub(gsub) => gsub.feature_variations(),
            Self::Gpos(gpos) => gpos.feature_variations(),
        }
        .transpose()
        .ok()
        .flatten()
    }

    fn script(&self, index: u16) -> Option<Script<'a>> {
        self.script_list()?
            .get(index)
            .ok()
            .map(|script| script.element)
    }

    fn langsys_index(&self, script_index: u16, tag: hb_tag_t) -> Option<u16> {
        let script = self.script(script_index)?;
        script.lang_sys_index_for_tag(tag)
    }

    fn langsys(&self, script_index: u16, langsys_index: Option<u16>) -> Option<LangSys<'a>> {
        let script = self.script(script_index)?;
        if let Some(index) = langsys_index {
            let record = script.lang_sys_records().get(index as usize)?;
            record.lang_sys(script.offset_data()).ok()
        } else {
            script.default_lang_sys().transpose().ok().flatten()
        }
    }

    pub(crate) fn feature(&self, index: u16) -> Option<Feature<'a>> {
        self.feature_list()?
            .get(index)
            .ok()
            .map(|feature| feature.element)
    }

    fn feature_tag(&self, index: u16) -> Option<hb_tag_t> {
        self.feature_list()?
            .get(index)
            .ok()
            .map(|feature| feature.tag)
    }

    pub(crate) fn feature_variation_index(&self, coords: &[F2Dot14]) -> Option<u32> {
        let feature_variations = self.feature_variations()?;
        for (index, rec) in feature_variations
            .feature_variation_records()
            .iter()
            .enumerate()
        {
            // If the ConditionSet offset is 0, this is treated as the
            // universal condition: all contexts are matched.
            if rec.condition_set_offset().is_null() {
                return Some(index as u32);
            }
            let Some(Ok(condition_set)) = rec.condition_set(feature_variations.offset_data())
            else {
                continue;
            };
            // Otherwise, all conditions must be satisfied.
            if condition_set
                .conditions()
                .iter()
                // .. except we ignore errors
                .filter_map(Result::ok)
                .all(|cond| match cond {
                    Condition::Format1AxisRange(format1) => {
                        let coord = coords
                            .get(format1.axis_index() as usize)
                            .copied()
                            .unwrap_or_default();
                        coord >= format1.filter_range_min_value()
                            && coord <= format1.filter_range_max_value()
                    }
                    _ => false,
                })
            {
                return Some(index as u32);
            }
        }
        None
    }

    pub(crate) fn feature_substitution(
        &self,
        variation_index: u32,
        feature_index: u16,
    ) -> Option<Feature<'a>> {
        let feature_variations = self.feature_variations()?;
        let record = feature_variations
            .feature_variation_records()
            .get(variation_index as usize)?;
        let subst_table = record
            .feature_table_substitution(feature_variations.offset_data())?
            .ok()?;
        let subst_records = subst_table.substitutions();
        match subst_records.binary_search_by_key(
            &feature_index,
            FeatureTableSubstitutionRecord::feature_index,
        ) {
            Ok(ix) => Some(
                subst_records
                    .get(ix)?
                    .alternate_feature(subst_table.offset_data())
                    .ok()?,
            ),
            _ => None,
        }
    }

    pub(crate) fn feature_index(&self, tag: hb_tag_t) -> Option<u16> {
        let list = self.feature_list()?;
        for (index, feature) in list.feature_records().iter().enumerate() {
            if feature.feature_tag() == tag {
                return Some(index as u16);
            }
        }
        None
    }

    pub(crate) fn lookup_count(&self) -> u16 {
        match self {
            Self::Gsub(gsub) => gsub
                .lookup_list()
                .map(|list| list.lookup_count())
                .unwrap_or_default(),
            Self::Gpos(gpos) => gpos
                .lookup_list()
                .map(|list| list.lookup_count())
                .unwrap_or_default(),
        }
    }

    // hb_ot_layout_table_select_script
    /// Returns true + index and tag of the first found script tag in the given GSUB or GPOS table
    /// or false + index and tag if falling back to a default script.
    pub(crate) fn select_script(&self, script_tags: &[hb_tag_t]) -> Option<(bool, u16, hb_tag_t)> {
        let selected = self.script_list()?.select(script_tags)?;
        Some((!selected.is_fallback, selected.index, selected.tag))
    }

    // hb_ot_layout_script_select_language
    /// Returns the index of the first found language tag in the given GSUB or GPOS table,
    /// underneath the specified script index.
    pub(crate) fn select_script_language(
        &self,
        script_index: u16,
        lang_tags: &[hb_tag_t],
    ) -> Option<u16> {
        for &tag in lang_tags {
            if let Some(index) = self.langsys_index(script_index, tag) {
                return Some(index);
            }
        }

        // try finding 'dflt'
        if let Some(index) = self.langsys_index(script_index, hb_tag_t::default_language()) {
            return Some(index);
        }

        None
    }

    // hb_ot_layout_language_get_required_feature
    /// Returns the index and tag of a required feature in the given GSUB or GPOS table,
    /// underneath the specified script and language.
    pub(crate) fn get_required_language_feature(
        &self,
        script_index: u16,
        lang_index: Option<u16>,
    ) -> Option<(u16, hb_tag_t)> {
        let sys = self.langsys(script_index, lang_index)?;
        let idx = sys.required_feature_index();
        if idx == 0xFFFF {
            return None;
        }
        let tag = self.feature_tag(idx)?;
        Some((idx, tag))
    }

    // hb_ot_layout_language_find_feature
    /// Returns the index of a given feature tag in the given GSUB or GPOS table,
    /// underneath the specified script and language.
    pub(crate) fn find_language_feature(
        &self,
        script_index: u16,
        lang_index: Option<u16>,
        feature_tag: hb_tag_t,
    ) -> Option<u16> {
        self.langsys(script_index, lang_index)?
            .feature_index_for_tag(&self.feature_list()?, feature_tag)
    }
}

fn coverage_index(coverage: Result<CoverageTable, ReadError>, gid: GlyphId) -> Option<u16> {
    coverage.ok().and_then(|coverage| coverage.get(gid))
}

fn covered(coverage: Result<CoverageTable, ReadError>, gid: GlyphId) -> bool {
    coverage_index(coverage, gid).is_some()
}

fn glyph_class(class_def: Result<ClassDef, ReadError>, gid: GlyphId) -> u16 {
    let Ok(gid16) = gid.try_into() else {
        return 0;
    };
    class_def
        .map(|class_def| class_def.get(gid16))
        .unwrap_or_default()
}
