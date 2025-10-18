use core::sync::atomic::{AtomicU16, AtomicU32, Ordering};

/// Trait for atomics used in cache storage
pub trait AtomicStorage: Sized {
    const BITS: usize;
    fn get(&self) -> u32;
    fn set(&self, val: u32);
    fn default() -> Self;
}

/// Implement AtomicStorage directly for AtomicU16
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

/// Implement AtomicStorage directly for AtomicU32
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

    pub fn new() -> Self {
        debug_assert!(
            CACHE_SIZE.is_power_of_two(),
            "CACHE_SIZE must be a power of two"
        );

        let cache_bits = CACHE_SIZE.ilog2() as usize;

        debug_assert!(
            KEY_BITS >= cache_bits,
            "KEY_BITS must be >= log2(CACHE_SIZE)"
        );
        debug_assert!(
            KEY_BITS + VALUE_BITS <= cache_bits + T::BITS,
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
        let expected_tag = key >> (CACHE_SIZE as u32).ilog2();

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
    pub fn set_unchecked(&self, key: u32, value: u32) {
        let index = (key as usize) & (CACHE_SIZE - 1);
        let packed = ((key >> (CACHE_SIZE as u32).ilog2()) << VALUE_BITS) | value;
        self.values[index].set(packed);
    }
}
