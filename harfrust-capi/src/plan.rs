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

use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use harfrust::font::FontInstance;
use harfrust::{Direction, Feature, Language, Script, ShapePlan};

/// How many plans one face keeps.
///
/// HarfBuzz's list is unbounded; this caps it so a caller that varies feature
/// ranges cannot grow the cache without limit. Plans are a few kilobytes each,
/// and a face rarely sees more than a handful of distinct combinations. The
/// cap also bounds the depth of the recursive drop of the list below.
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
    /// Compares against a caller's values without building a key from them.
    fn matches(
        &self,
        direction: Direction,
        script: Option<Script>,
        language: Option<&Language>,
        features: &[Feature],
        feature_variations: [Option<u32>; 2],
    ) -> bool {
        self.direction == direction
            && self.script == script
            && self.language.as_ref() == language
            && self.feature_variations == feature_variations
            && self.features == features
    }
}

/// A face's cache of shape plans.
///
/// A singly linked list whose links are each written once, mirroring the
/// atomic list HarfBuzz keeps on its own faces. No node is ever unlinked or
/// changed after it is published, which is what lets a reader walk the list
/// with plain acquire loads: a hit takes no lock, and hands back a borrow
/// rather than a counted reference, because the nodes live as long as the
/// cache and the cache outlives every shaping call made through the face.
#[derive(Default)]
pub(crate) struct PlanCache {
    head: OnceLock<Box<Node>>,
    len: AtomicUsize,
}

/// One cached plan and the link to the next.
struct Node {
    key: PlanKey,
    /// Counted, because [`hr_shape_plan_create_cached`] hands the same plan
    /// out as an object the caller owns. Shaping only ever borrows it.
    plan: Arc<ShapePlan>,
    next: OnceLock<Box<Node>>,
}

/// A plan held by the cache, or one built for a caller it had no room for.
pub(crate) enum CachedPlan<'a> {
    Cached(&'a Arc<ShapePlan>),
    Uncached(Arc<ShapePlan>),
}

impl CachedPlan<'_> {
    /// Takes a counted reference, for a caller that keeps the plan.
    pub(crate) fn to_arc(&self) -> Arc<ShapePlan> {
        match self {
            Self::Cached(plan) => Arc::clone(plan),
            Self::Uncached(plan) => Arc::clone(plan),
        }
    }
}

impl Deref for CachedPlan<'_> {
    type Target = ShapePlan;

    fn deref(&self) -> &ShapePlan {
        match self {
            Self::Cached(plan) => plan,
            Self::Uncached(plan) => plan,
        }
    }
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
    ) -> CachedPlan<'_> {
        let variations = instance.feature_variations();
        let feature_variations = [variations.gsub(), variations.gpos()];

        // A linear walk, as HarfBuzz does over its per-face plan list. The key
        // is compared against the caller's own values, so a hit -- the common
        // case once a face is warm -- builds nothing at all.
        let mut link = &self.head;
        while let Some(node) = link.get() {
            if node.key.matches(
                direction,
                script,
                language.as_ref(),
                features,
                feature_variations,
            ) {
                return CachedPlan::Cached(&node.plan);
            }
            link = &node.next;
        }

        let plan = Arc::new(build(instance, direction, script, language.as_ref(), features));
        if self.len.load(Ordering::Relaxed) >= CAPACITY {
            return CachedPlan::Uncached(plan);
        }
        let key = PlanKey {
            direction,
            script,
            language,
            feature_variations,
            features: features.to_vec(),
        };
        CachedPlan::Cached(self.insert(key, plan))
    }

    /// Publishes a node at the end of the list, or hands back an equal one that
    /// another thread published while this one was building.
    fn insert(&self, key: PlanKey, plan: Arc<ShapePlan>) -> &Arc<ShapePlan> {
        let mut node = Box::new(Node {
            key,
            plan,
            next: OnceLock::new(),
        });
        let mut link = &self.head;
        loop {
            match link.set(node) {
                Ok(()) => {
                    self.len.fetch_add(1, Ordering::Relaxed);
                    let published = link.get().expect("just set");
                    return &published.plan;
                }
                Err(unplaced) => {
                    node = unplaced;
                    let occupant = link.get().expect("set failed, so it is occupied");
                    if occupant.key == node.key {
                        return &occupant.plan;
                    }
                    link = &occupant.next;
                }
            }
        }
    }
}

fn build(
    instance: &FontInstance,
    direction: Direction,
    script: Option<Script>,
    language: Option<&Language>,
    features: &[Feature],
) -> ShapePlan {
    ShapePlan::new(instance, direction, script, language, features)
}
