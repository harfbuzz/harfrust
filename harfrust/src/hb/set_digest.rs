use read_fonts::tables::layout::CoverageTable;

type mask_t = u64;

const HB_SET_DIGEST_SHIFTS: [u32; 3] = [4, 0, 6];
const N: usize = HB_SET_DIGEST_SHIFTS.len();
const MASK_BITS: u32 = mask_t::BITS;
const MB1: u32 = MASK_BITS - 1;
const ONE: mask_t = 1;
const ALL: mask_t = mask_t::MAX;

#[derive(Clone, Debug)]
pub struct hb_set_digest_t {
    masks: [mask_t; N],
}

impl Default for hb_set_digest_t {
    fn default() -> Self {
        Self::new()
    }
}

impl hb_set_digest_t {
    pub fn new() -> Self {
        Self { masks: [0; N] }
    }

    pub fn from_coverage(coverage: &CoverageTable) -> Self {
        let mut digest = Self::new();
        digest.add_coverage(coverage);
        digest
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.masks = [0; N];
    }

    pub fn full() -> Self {
        Self { masks: [ALL; N] }
    }

    pub fn union(&mut self, other: &Self) {
        for i in 0..N {
            self.masks[i] |= other.masks[i];
        }
    }

    pub fn add(&mut self, g: u32) {
        for i in 0..N {
            let shift = HB_SET_DIGEST_SHIFTS[i];
            let bit = (g >> shift) & MB1;
            self.masks[i] |= ONE << bit;
        }
    }

    pub fn add_array(&mut self, array: impl IntoIterator<Item = u32>) {
        for g in array {
            self.add(g);
        }
    }

    pub fn add_range(&mut self, a: u32, b: u32) -> bool {
        let a = a as mask_t;
        let b = b as mask_t;

        if self.masks.iter().all(|&m| m == ALL) {
            return false;
        }

        let mut changed = false;
        for i in 0..N {
            let shift = HB_SET_DIGEST_SHIFTS[i] as mask_t;
            if (b >> shift).wrapping_sub(a >> shift) >= MB1 as mask_t {
                self.masks[i] = ALL;
            } else {
                let ma = ONE << ((a >> shift) & MB1 as mask_t);
                let mb = ONE << ((b >> shift) & MB1 as mask_t);
                self.masks[i] |= mb + mb.wrapping_sub(ma) - mask_t::from(mb < ma);
                changed = true;
            }
        }
        changed
    }

    pub fn add_coverage(&mut self, coverage: &CoverageTable) {
        match coverage {
            CoverageTable::Format1(table) => {
                for glyph in table.glyph_array() {
                    self.add(glyph.get().into());
                }
            }
            CoverageTable::Format2(table) => {
                for range in table.range_records() {
                    self.add_range(range.start_glyph_id().into(), range.end_glyph_id().into());
                }
            }
        }
    }

    pub fn may_have(&self, g: u32) -> bool {
        for i in 0..N {
            let shift = HB_SET_DIGEST_SHIFTS[i];
            let bit = (g >> shift) & MB1;
            if self.masks[i] & (ONE << bit) == 0 {
                return false;
            }
        }
        true
    }

    pub fn may_intersect(&self, other: &Self) -> bool {
        for i in 0..N {
            if self.masks[i] & other.masks[i] == 0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single() {
        let mut set = hb_set_digest_t::new();
        set.add(2);
        assert!(set.may_have(2));
    }

    #[test]
    fn test_multiple_1() {
        let mut set = hb_set_digest_t::new();
        set.add(2);
        set.add(10);
        set.add(300);
        set.add(255);
        assert!(set.may_have(2));
        assert!(set.may_have(10));
        assert!(set.may_have(255));
        assert!(set.may_have(300));
    }

    #[test]
    fn test_multiple_2() {
        let mut set = hb_set_digest_t::new();
        set.add(245);
        set.add(1060);
        set.add(300);
        set.add(599);
        assert!(set.may_have(245));
        assert!(set.may_have(1060));
        assert!(set.may_have(300));
        assert!(set.may_have(599));
    }

    #[test]
    fn test_range_1() {
        let mut set = hb_set_digest_t::new();
        set.add_range(10, 12);
        assert!(set.may_have(10));
        assert!(set.may_have(11));
        assert!(set.may_have(12));
    }

    #[test]
    fn test_range_2() {
        let mut set = hb_set_digest_t::new();
        set.add_range(20, 15);
        set.add_range(15, 20);
        for gid in 15..=20 {
            assert!(set.may_have(gid));
        }
    }

    #[test]
    fn test_range_3() {
        let mut set = hb_set_digest_t::new();
        for i in 170..=239 {
            set.add(i);
        }
        assert!(set.may_have(200));
    }

    #[test]
    fn test_complex() {
        let mut set = hb_set_digest_t::new();
        set.add_range(5670, 5675);
        set.add(3);
        set.add(8769);
        set.add(10000);
        set.add_range(3456, 3460);

        assert!(set.may_have(3));
        assert!(set.may_have(5670));
        assert!(set.may_have(5675));
        assert!(set.may_have(8769));
        assert!(set.may_have(10000));
        assert!(set.may_have(3456));
        assert!(set.may_have(3460));
    }

    #[test]
    fn test_intersect() {
        let mut a = hb_set_digest_t::new();
        let mut b = hb_set_digest_t::new();

        a.add(123);
        b.add(456);
        assert!(!a.may_intersect(&b));

        b.add(123);
        assert!(a.may_intersect(&b));
    }
    /// Gated on `std` because `#[hegel::test]`'s generated code uses the std
    /// prelude.
    #[cfg(feature = "std")]
    mod properties {
        use super::*;
        use hegel::generators::{self, Generator};

        /// Glyph ids the digest is asked about in practice are small, but it is
        /// indexed by arbitrary `u32`, and the bit arithmetic is where a digest
        /// goes wrong.
        fn glyphs(tc: &hegel::TestCase) -> Vec<u32> {
            tc.draw(
                generators::vecs(hegel::one_of!(
                    generators::integers::<u32>().max_value(0xFFFF),
                    generators::integers::<u32>(),
                ))
                .max_size(32)
                .print_as_debug(),
            )
        }

        /// Property: the digest has no false negatives.
        ///
        /// That is the whole contract of a set digest: `may_have` is allowed to
        /// say yes about a glyph that was never added, but never no about one that
        /// was. `ot_layout.rs` skips a lookup outright when `may_intersect` says
        /// no, so a false negative drops shaping work.
        #[hegel::test]
        fn everything_added_may_be_had(tc: hegel::TestCase) {
            let glyphs = glyphs(&tc);
            let mut digest = hb_set_digest_t::new();
            digest.add_array(glyphs.iter().copied());
            for glyph in &glyphs {
                assert!(digest.may_have(*glyph), "{glyph} was added");
            }
        }

        /// Property: `add_range` has no false negatives either.
        ///
        /// Ranges take a separate path that folds a whole span into each mask in
        /// one go (`mb + (mb - ma) - (mb < ma)`), rather than one bit at a time.
        #[hegel::test]
        fn everything_in_an_added_range_may_be_had(tc: hegel::TestCase) {
            let start = tc.draw(generators::integers::<u32>());
            // A span wider than a mask covers every bit anyway, so keep it narrow
            // enough that the test can enumerate it.
            let width = tc.draw(generators::integers::<u32>().max_value(300));
            let end = start.saturating_add(width);

            let mut digest = hb_set_digest_t::new();
            digest.add_range(start, end);
            for glyph in start..=end {
                assert!(digest.may_have(glyph), "{glyph} is in {start}..={end}");
            }
        }

        /// Property: a union has no false negatives about either side.
        ///
        /// `ot/lookup.rs` builds a lookup's digest by unioning its subtables',
        /// so a glyph lost here is a subtable that never gets tried.
        #[hegel::test]
        fn a_union_may_have_everything_either_side_had(tc: hegel::TestCase) {
            let left = glyphs(&tc);
            let right = glyphs(&tc);

            let mut a = hb_set_digest_t::new();
            a.add_array(left.iter().copied());
            let mut b = hb_set_digest_t::new();
            b.add_array(right.iter().copied());
            a.union(&b);

            for glyph in left.iter().chain(&right) {
                assert!(a.may_have(*glyph), "{glyph} was added to one side");
            }
        }

        /// Property: two digests that share a glyph may intersect.
        ///
        /// `may_intersect` is the guard `ot_layout.rs` uses to skip a lookup whose
        /// coverage cannot match the buffer, so saying no about a shared glyph
        /// would drop a substitution.
        #[hegel::test]
        fn digests_sharing_a_glyph_may_intersect(tc: hegel::TestCase) {
            let left = glyphs(&tc);
            let right = glyphs(&tc);
            let shared = tc.draw(generators::integers::<u32>());

            let mut a = hb_set_digest_t::new();
            a.add_array(left.iter().copied());
            a.add(shared);
            let mut b = hb_set_digest_t::new();
            b.add_array(right.iter().copied());
            b.add(shared);

            assert!(a.may_intersect(&b), "both hold {shared}");
        }
    }
}
