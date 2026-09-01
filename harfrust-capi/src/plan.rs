//! A per-face cache of shape plans.
//!
//! HarfBuzz's `hb_shape` builds its plan through `hb_shape_plan_create_cached`,
//! so repeated calls over the same face, segment properties and features reuse
//! one plan. HarfRust builds a fresh plan per call and offers no cache of its
//! own, so this supplies one, keeping the cost of `hr_shape` in line with what
//! HarfBuzz callers expect.
//!
//! # Why nothing ever needs flushing
//!
//! The cache hangs off the face, and everything the planner reads from a font
//! is fixed for the life of that face: the presence of `GSUB`, `morx` and
//! `mort`, and the script chosen from `GSUB`. The one input that varies with a
//! font instance is the pair of `GSUB` and `GPOS` feature variation indices,
//! and those are part of the key below.
//!
//! So changing a font's variation settings simply misses and builds a new
//! plan, rather than leaving a stale one behind, and the other mutable font
//! properties (scale, point size and callbacks) do not enter into a plan at
//! all. Two fonts over one face share the cache safely for the same reason.

use std::sync::{Arc, Mutex};

use harfrust::font::FontInstance;
use harfrust::{Direction, Feature, Language, Script, ShapePlan};

/// How many plans one face keeps.
///
/// HarfBuzz's list is unbounded; this caps it so a caller that varies feature
/// ranges cannot grow the cache without limit. Plans are a few kilobytes each,
/// and a face rarely sees more than a handful of distinct combinations.
const CAPACITY: usize = 32;

/// Everything a plan depends on.
///
/// `ShapePlanKey` cannot be built from a [`FontInstance`], so the cache keeps
/// its own key rather than asking HarfRust whether a plan matches. Features are
/// compared exactly, which can miss a reusable plan that HarfBuzz would have
/// matched, but never returns a wrong one.
#[derive(PartialEq)]
struct PlanKey {
    direction: Direction,
    script: Option<Script>,
    language: Option<Language>,
    /// The `GSUB` and `GPOS` feature variation indices the instance selects,
    /// so two variable font instances do not share a plan.
    feature_variations: [Option<u32>; 2],
    features: Vec<Feature>,
}

impl PlanKey {
    fn new(
        instance: &FontInstance,
        direction: Direction,
        script: Option<Script>,
        language: Option<Language>,
        features: &[Feature],
    ) -> Self {
        let variations = instance.feature_variations();
        Self {
            direction,
            script,
            language,
            feature_variations: [variations.gsub(), variations.gpos()],
            features: features.to_vec(),
        }
    }
}

/// A face's cache of shape plans, newest first.
#[derive(Default)]
pub(crate) struct PlanCache {
    entries: Mutex<Vec<(PlanKey, Arc<ShapePlan>)>>,
}

impl PlanCache {
    /// Returns a plan for the given properties, building one if the cache does
    /// not already hold a match.
    pub(crate) fn get(
        &self,
        instance: &FontInstance,
        direction: Direction,
        script: Option<Script>,
        language: Option<Language>,
        features: &[Feature],
    ) -> Arc<ShapePlan> {
        let key = PlanKey::new(instance, direction, script, language.clone(), features);

        // A poisoned lock only means some other caller panicked mid-shape; fall
        // back to an uncached plan rather than propagating it.
        let Ok(entries) = self.entries.lock() else {
            return Arc::new(build(instance, direction, script, language, features));
        };

        // A linear search, as HarfBuzz does over its per-face plan list.
        if let Some((_, plan)) = entries.iter().find(|(found, _)| *found == key) {
            return Arc::clone(plan);
        }

        // Build without holding the lock, so one slow plan does not stall
        // shaping on other threads.
        drop(entries);
        let plan = Arc::new(build(instance, direction, script, language, features));

        if let Ok(mut entries) = self.entries.lock() {
            // Another thread may have inserted an equal key meanwhile; an extra
            // copy is harmless, so just prepend, as HarfBuzz does.
            entries.insert(0, (key, Arc::clone(&plan)));
            entries.truncate(CAPACITY);
        }
        plan
    }
}

fn build(
    instance: &FontInstance,
    direction: Direction,
    script: Option<Script>,
    language: Option<Language>,
    features: &[Feature],
) -> ShapePlan {
    ShapePlan::new(instance, direction, script, language.as_ref(), features)
}
