use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, Ordering};

/// Trait for atomics used in cache storage
pub trait AtomicStorage: Sized {
    const BITS: usize;
    fn get(&self) -> u32;
    fn set(&self, val: u32);
    fn default() -> Self;
}

impl AtomicStorage for AtomicU8 {
    const BITS: usize = 8;

    fn get(&self) -> u32 {
        self.load(Ordering::Relaxed) as u32
    }

    fn set(&self, val: u32) {
        self.store(val as u8, Ordering::Relaxed);
    }

    fn default() -> Self {
        Self::new(u8::MAX)
    }
}

impl AtomicStorage for AtomicU16 {
    const BITS: usize = 16;

    fn get(&self) -> u32 {
        self.load(Ordering::Relaxed) as u32
    }

    fn set(&self, val: u32) {
        self.store(val as u16, Ordering::Relaxed);
    }

    fn default() -> Self {
        Self::new(u16::MAX)
    }
}

impl AtomicStorage for AtomicU32 {
    const BITS: usize = 32;

    fn get(&self) -> u32 {
        self.load(Ordering::Relaxed)
    }

    fn set(&self, val: u32) {
        self.store(val, Ordering::Relaxed);
    }

    fn default() -> Self {
        Self::new(u32::MAX)
    }
}

/// Selects correct type from STORAGE_BITS
pub trait SelectAtomic<const BITS: usize> {
    type Type: AtomicStorage;
}
impl SelectAtomic<8> for () {
    type Type = AtomicU8;
}
impl SelectAtomic<16> for () {
    type Type = AtomicU16;
}
impl SelectAtomic<32> for () {
    type Type = AtomicU32;
}

/// Public wrapper
pub type hb_cache_t<
    const KEY_BITS: usize,
    const VALUE_BITS: usize,
    const CACHE_SIZE: usize,
    const STORAGE_BITS: usize,
> = hb_cache_core_t<KEY_BITS, VALUE_BITS, CACHE_SIZE, <() as SelectAtomic<STORAGE_BITS>>::Type>;

/// Core cache
#[derive(Debug)]
pub struct hb_cache_core_t<
    const KEY_BITS: usize,
    const VALUE_BITS: usize,
    const CACHE_SIZE: usize,
    T: AtomicStorage,
> {
    values: [T; CACHE_SIZE],
}

impl<const KEY_BITS: usize, const VALUE_BITS: usize, const CACHE_SIZE: usize, T: AtomicStorage>
    hb_cache_core_t<KEY_BITS, VALUE_BITS, CACHE_SIZE, T>
{
    pub const MAX_VALUE: u32 = (1 << VALUE_BITS) - 1;
    const CACHE_BITS: usize = CACHE_SIZE.ilog2() as usize;

    pub fn new() -> Self {
        debug_assert!(
            CACHE_SIZE.is_power_of_two(),
            "CACHE_SIZE must be a power of two"
        );

        debug_assert!(
            KEY_BITS >= Self::CACHE_BITS,
            "KEY_BITS must be >= log2(CACHE_SIZE)"
        );
        debug_assert!(
            KEY_BITS + VALUE_BITS <= Self::CACHE_BITS + T::BITS,
            "KEY_BITS + VALUE_BITS must fit in CACHE_BITS + T::BITS"
        );

        Self {
            values: core::array::from_fn(|_| T::default()),
        }
    }

    #[inline]
    pub fn get(&self, key: u32) -> Option<u32> {
        let index = (key as usize) & (CACHE_SIZE - 1);
        let stored = self.values[index].get();
        let tag = stored >> VALUE_BITS;
        let expected_tag = key >> Self::CACHE_BITS;

        if stored == T::default().get() || tag != expected_tag {
            return None;
        }

        Some(stored & ((1 << VALUE_BITS) - 1))
    }

    #[inline]
    pub fn set(&self, key: u32, value: u32) {
        if (key >> KEY_BITS) != 0 || (value >> VALUE_BITS) != 0 {
            return;
        }
        self.set_unchecked(key, value);
    }

    #[inline]
    fn set_unchecked(&self, key: u32, value: u32) {
        let index = (key as usize) & (CACHE_SIZE - 1);
        let packed = ((key >> Self::CACHE_BITS) << VALUE_BITS) | value;
        self.values[index].set(packed);
    }
}

/// Gated on `std` because `#[hegel::test]`'s generated code uses the std
/// prelude.
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use hegel::generators;
    use hegel::TestCase;
    use std::collections::HashMap;

    /// The instantiation `Charmap` uses: 21-bit keys (a Unicode scalar value),
    /// 19-bit values (a glyph id), 256 slots of `u32`.
    type Cache = hb_cache_t<21, 19, 256, 32>;

    const KEY_BITS: u32 = 21;
    const VALUE_BITS: u32 = 19;

    /// Keys are drawn from a narrow band as well as the whole 21-bit space so
    /// that slot collisions and repeats actually happen: the slot is the key's
    /// low 8 bits, so keys 256 apart share one.
    fn draw_key(tc: &TestCase) -> u32 {
        tc.draw(hegel::one_of!(
            generators::integers::<u32>().max_value(600),
            generators::integers::<u32>().max_value((1 << KEY_BITS) - 1),
            generators::sampled_from(vec![0, 255, 256, 511, (1 << KEY_BITS) - 1]),
        ))
    }

    fn draw_value(tc: &TestCase) -> u32 {
        tc.draw(hegel::one_of!(
            generators::integers::<u32>().max_value(600),
            generators::integers::<u32>().max_value((1 << VALUE_BITS) - 1),
        ))
    }

    struct CacheModel {
        cache: Cache,
        /// The last value successfully written for each key.
        written: HashMap<u32, u32>,
    }

    impl CacheModel {
        /// A hit has to carry the last value written for that key; a miss is
        /// always allowed, since a later key sharing the slot evicts an
        /// earlier one.
        fn check(&self, key: u32) {
            let cached = self.cache.get(key);
            match (cached, self.written.get(&key)) {
                (None, _) => {}
                (Some(cached), Some(written)) => assert_eq!(
                    cached, *written,
                    "{key} was last written as {written}, read back as {cached}"
                ),
                (Some(cached), None) => {
                    panic!("{key} was never written, read back as {cached}")
                }
            }
        }
    }

    #[hegel::state_machine]
    impl CacheModel {
        #[rule]
        fn set(&mut self, tc: TestCase) {
            let key = draw_key(&tc);
            let value = draw_value(&tc);
            self.cache.set(key, value);
            self.written.insert(key, value);
            self.check(key);
        }

        /// `set` drops keys and values that do not fit their bit widths.
        #[rule]
        fn set_out_of_range(&mut self, tc: TestCase) {
            let key = draw_key(&tc);
            let (key, value) = if tc.draw(generators::booleans()) {
                (
                    key | (1 << KEY_BITS),
                    tc.draw(generators::integers::<u32>().max_value((1 << VALUE_BITS) - 1)),
                )
            } else {
                (key, draw_value(&tc) | (1 << VALUE_BITS))
            };

            let before = self.cache.get(key);
            self.cache.set(key, value);
            assert_eq!(self.cache.get(key), before, "{key} => {value} was refused");
        }

        #[rule]
        fn get(&mut self, tc: TestCase) {
            let key = draw_key(&tc);
            self.check(key);
        }

        #[invariant]
        fn every_hit_is_the_last_value_written(&self, _: TestCase) {
            for key in self.written.keys() {
                self.check(*key);
            }
        }
    }

    /// Property: reading the cache never produces a value other than the last
    /// one written for that key.
    ///
    /// The slot is the key's low `log2(CACHE_SIZE)` bits and the rest of the
    /// key is stored as a tag beside the value, so a key that shares a slot
    /// with another has to miss rather than return the other's value. The one
    /// hit the implementation gives up is the largest key with the largest
    /// value, whose packed form equals the empty-slot sentinel; a miss is
    /// always allowed, so the property covers that too.
    #[hegel::test]
    fn a_cache_hit_carries_the_last_value_written(tc: TestCase) {
        hegel::stateful::run(
            CacheModel {
                cache: Cache::new(),
                written: HashMap::new(),
            },
            tc,
        );
    }
}
