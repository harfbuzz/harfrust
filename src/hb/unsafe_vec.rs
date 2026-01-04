use std::fmt;
use std::ops::{
    Deref, DerefMut, Index, IndexMut, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};

#[repr(transparent)]
pub struct UnsafeVec<T> {
    pub inner: Vec<T>,
}

impl<T> UnsafeVec<T> {
    #[inline]
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn push(&mut self, value: T) {
        self.inner.push(value);
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.inner.as_mut_ptr()
    }

    #[inline]
    pub fn into_vec(self) -> Vec<T> {
        self.inner
    }

    #[inline]
    pub fn from_vec(v: Vec<T>) -> Self {
        Self { inner: v }
    }
}

impl<T> Default for UnsafeVec<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for UnsafeVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// Expose full Vec<T> API (resize/truncate/extend/etc.)
impl<T> Deref for UnsafeVec<T> {
    type Target = Vec<T>;
    #[inline]
    fn deref(&self) -> &Vec<T> {
        &self.inner
    }
}
impl<T> DerefMut for UnsafeVec<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.inner
    }
}

/// Unchecked single-element indexing (UB if out of bounds)
impl<T> Index<usize> for UnsafeVec<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        unsafe { self.inner.get_unchecked(i) }
    }
}
impl<T> IndexMut<usize> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut T {
        unsafe { self.inner.get_unchecked_mut(i) }
    }
}

/// Unchecked range indexing (UB if invalid range)
impl<T> Index<Range<usize>> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, r: Range<usize>) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.inner.as_ptr().add(r.start), r.end - r.start) }
    }
}
impl<T> IndexMut<Range<usize>> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, r: Range<usize>) -> &mut [T] {
        unsafe {
            std::slice::from_raw_parts_mut(self.inner.as_mut_ptr().add(r.start), r.end - r.start)
        }
    }
}

impl<T> Index<RangeFrom<usize>> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, r: RangeFrom<usize>) -> &[T] {
        let len = self.inner.len();
        unsafe { std::slice::from_raw_parts(self.inner.as_ptr().add(r.start), len - r.start) }
    }
}
impl<T> IndexMut<RangeFrom<usize>> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, r: RangeFrom<usize>) -> &mut [T] {
        let len = self.inner.len();
        unsafe {
            std::slice::from_raw_parts_mut(self.inner.as_mut_ptr().add(r.start), len - r.start)
        }
    }
}

impl<T> Index<RangeTo<usize>> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, r: RangeTo<usize>) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.inner.as_ptr(), r.end) }
    }
}
impl<T> IndexMut<RangeTo<usize>> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, r: RangeTo<usize>) -> &mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.inner.as_mut_ptr(), r.end) }
    }
}

impl<T> Index<RangeFull> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, _: RangeFull) -> &[T] {
        &self.inner
    }
}
impl<T> IndexMut<RangeFull> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, _: RangeFull) -> &mut [T] {
        &mut self.inner
    }
}

impl<T> Index<RangeInclusive<usize>> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, r: RangeInclusive<usize>) -> &[T] {
        let start = *r.start();
        let end_incl = *r.end();
        unsafe { std::slice::from_raw_parts(self.inner.as_ptr().add(start), end_incl - start + 1) }
    }
}
impl<T> IndexMut<RangeInclusive<usize>> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, r: RangeInclusive<usize>) -> &mut [T] {
        let start = *r.start();
        let end_incl = *r.end();
        unsafe {
            std::slice::from_raw_parts_mut(self.inner.as_mut_ptr().add(start), end_incl - start + 1)
        }
    }
}

impl<T> Index<RangeToInclusive<usize>> for UnsafeVec<T> {
    type Output = [T];
    #[inline]
    fn index(&self, r: RangeToInclusive<usize>) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.inner.as_ptr(), r.end + 1) }
    }
}
impl<T> IndexMut<RangeToInclusive<usize>> for UnsafeVec<T> {
    #[inline]
    fn index_mut(&mut self, r: RangeToInclusive<usize>) -> &mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.inner.as_mut_ptr(), r.end + 1) }
    }
}

/// Iteration support: `for x in &v`, `for x in &mut v`, `for x in v`
impl<'a, T> IntoIterator for &'a UnsafeVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}
impl<'a, T> IntoIterator for &'a mut UnsafeVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}
impl<T> IntoIterator for UnsafeVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}
