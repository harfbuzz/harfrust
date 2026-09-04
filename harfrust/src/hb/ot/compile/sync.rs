//! The two synchronisation primitives the compiled form needs, in whichever
//! flavour this build has.
//!
//! An [`Interner`](super::set::Interner) is shared between every lookup of a
//! font and filled as plans reach new ones, and a
//! [`Program`](super::lookup::Program) fills each of its slots the first time
//! something asks for it. Both are behind a shared reference, so both need
//! interior mutability that is sound across threads: a mutex and a once-cell.
//!
//! `std` has both. Without it they come from `spin`, and rather than emulate
//! `std`'s signatures the wrappers here present the narrower API the callers
//! actually want -- notably a `lock` that cannot fail, since a poisoned lock
//! hands back the data anyway and there is nothing else to do with it. That
//! removes an `unwrap_or_else` and an `.ok()?` from the callers as well as
//! giving the two backends one shape.

use core::ops::DerefMut;

/// Mutual exclusion, with a `lock` that always succeeds.
#[derive(Debug, Default)]
pub struct Mutex<T>(Inner<T>);

impl<T> Mutex<T> {
    pub fn new(value: T) -> Self {
        Self(Inner::new(value))
    }

    /// Lock, blocking until it is free.
    ///
    /// Infallible by construction. `std` reports a poisoned lock -- a thread
    /// panicked while holding it -- and the only sensible response here is to
    /// take the data anyway: every caller is filling a cache, so the worst a
    /// half-written predecessor can leave behind is a table that has to be
    /// rebuilt.
    pub fn lock(&self) -> impl DerefMut<Target = T> + '_ {
        #[cfg(feature = "std")]
        return self.0.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(not(feature = "std"))]
        return self.0.lock();
    }
}

/// A cell written at most once, readable through a shared reference.
#[derive(Debug)]
pub struct OnceLock<T>(Once<T>);

impl<T> OnceLock<T> {
    pub fn new() -> Self {
        Self(Once::new())
    }

    /// The value, if it has been written.
    pub fn get(&self) -> Option<&T> {
        self.0.get()
    }

    /// The value, initialising it if this is the first ask.
    ///
    /// Two threads racing here may both run `f`; one of the results is kept and
    /// the other dropped. That is the same contract `std` offers and it is fine
    /// for what this holds -- compiling a lookup twice wastes work and changes
    /// no answer.
    pub fn get_or_init<F: FnOnce() -> T>(&self, f: F) -> &T {
        #[cfg(feature = "std")]
        return self.0.get_or_init(f);
        #[cfg(not(feature = "std"))]
        return self.0.call_once(f);
    }

    /// Write the value if the cell is empty, reporting whether it took.
    ///
    /// `std` hands the rejected value back on failure; nothing here wants it,
    /// so the narrower answer is a bool -- which `spin`, whose only writer is
    /// `call_once`, can also give.
    pub fn set(&self, value: T) -> bool {
        #[cfg(feature = "std")]
        return self.0.set(value).is_ok();
        #[cfg(not(feature = "std"))]
        {
            let mut took = false;
            self.0.call_once(|| {
                took = true;
                value
            });
            took
        }
    }
}

impl<T> Default for OnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "std")]
type Inner<T> = std::sync::Mutex<T>;
#[cfg(feature = "std")]
type Once<T> = std::sync::OnceLock<T>;

#[cfg(not(feature = "std"))]
type Inner<T> = spin::Mutex<T>;
#[cfg(not(feature = "std"))]
type Once<T> = spin::Once<T>;

/// Three words, written once and read on every buffer after that.
///
/// Not a `OnceLock<[u64; 3]>`, which is the obvious way to say this: that is
/// 32 bytes where this is 24, and it lives in a slot the shaping loop walks
/// once per lookup per buffer. The eight bytes cost about five percent on
/// short Latin lines, which is the whole margin the thing it gates earns.
///
/// The word at index 0 doubles as the written flag. A reader that sees it
/// non-zero has, through the release/acquire pair, also seen the other two --
/// without that, a reader could catch a half-written digest, find a zero in
/// it, and conclude a lookup cannot match when it can. A digest that folds to
/// all zeros reads as unwritten and is rebuilt, which is wasteful rather than
/// wrong, and cannot happen for a lookup that covers any glyph at all.
#[cfg(target_has_atomic = "64")]
#[derive(Default, Debug)]
pub struct DigestCell([core::sync::atomic::AtomicU64; 3]);

#[cfg(target_has_atomic = "64")]
impl DigestCell {
    #[inline]
    pub fn get(&self) -> Option<[u64; 3]> {
        use core::sync::atomic::Ordering;
        let first = self.0[0].load(Ordering::Acquire);
        if first == 0 {
            return None;
        }
        Some([
            first,
            self.0[1].load(Ordering::Relaxed),
            self.0[2].load(Ordering::Relaxed),
        ])
    }

    #[inline]
    pub fn set(&self, words: &[u64; 3]) {
        use core::sync::atomic::Ordering;
        self.0[1].store(words[1], Ordering::Relaxed);
        self.0[2].store(words[2], Ordering::Relaxed);
        self.0[0].store(words[0], Ordering::Release);
    }
}

/// The same, for a target with no 64-bit atomics -- `thumbv7em-none-eabihf`,
/// which this crate builds for. Costs eight bytes more per lookup; a target
/// that cannot do a 64-bit load is not the one that margin was measured on.
#[cfg(not(target_has_atomic = "64"))]
#[derive(Default, Debug)]
pub struct DigestCell(OnceLock<[u64; 3]>);

#[cfg(not(target_has_atomic = "64"))]
impl DigestCell {
    #[inline]
    pub fn get(&self) -> Option<[u64; 3]> {
        self.0.get().copied()
    }

    #[inline]
    pub fn set(&self, words: &[u64; 3]) {
        self.0.set(*words);
    }
}
