//! A flat bitmap over the 16-bit glyph space, specialized for the AAT
//! apply path.
//!
//! The buffer glyph set is rebuilt for every line and probed for every
//! subtable, so membership operations must be branch-light and search-
//! free: one bit per glyph id, indexed directly. AAT tables address
//! glyphs as 16-bit ids, so the deleted glyph (0xFFFF) is tracked as a
//! flag and larger ids — which no AAT table can reference — are simply
//! not members.

use alloc::vec::Vec;
use core::ops::RangeInclusive;

const MAX_GLYPH: u32 = 0xFFFE;
const DELETED: u32 = 0xFFFF;

/// Glyphs per `touched` block: 8 words of 64 bits.
const BLOCK_BITS: u32 = 9;

#[derive(Default)]
pub struct GlyphSet {
    /// One bit per glyph id, grown on demand to the highest inserted id.
    words: Vec<u64>,
    /// One bit per 512-glyph block of `words`, tracking which blocks may
    /// contain set bits; `clear` zeroes only those blocks, so clearing
    /// costs in proportion to what a line actually touched.
    touched: u128,
    has_deleted: bool,
}

impl GlyphSet {
    #[inline(always)]
    pub fn insert(&mut self, g: u32) {
        if g > MAX_GLYPH {
            self.has_deleted |= g == DELETED;
            return;
        }
        let w = (g >> 6) as usize;
        let bit = 1u64 << (g & 63);
        if let Some(word) = self.words.get_mut(w) {
            *word |= bit;
        } else {
            self.words.resize(w + 1, 0);
            self.words[w] |= bit;
        }
        self.touched |= 1 << (g >> BLOCK_BITS);
    }

    #[inline(always)]
    pub fn contains(&self, g: u32) -> bool {
        if g > MAX_GLYPH {
            return g == DELETED && self.has_deleted;
        }
        match self.words.get((g >> 6) as usize) {
            Some(word) => word & (1 << (g & 63)) != 0,
            None => false,
        }
    }

    pub fn insert_range(&mut self, range: RangeInclusive<u32>) {
        let start = *range.start();
        let end = *range.end();
        if start > end {
            return;
        }
        if end >= DELETED && start <= DELETED {
            self.has_deleted = true;
        }
        let end = end.min(MAX_GLYPH);
        if start > end {
            return;
        }
        let (ws, we) = ((start >> 6) as usize, (end >> 6) as usize);
        if self.words.len() <= we {
            self.words.resize(we + 1, 0);
        }
        let start_mask = !0u64 << (start & 63);
        let end_mask = !0u64 >> (63 - (end & 63));
        if ws == we {
            self.words[ws] |= start_mask & end_mask;
        } else {
            self.words[ws] |= start_mask;
            for word in &mut self.words[ws + 1..we] {
                *word = !0;
            }
            self.words[we] |= end_mask;
        }
        for major in (start >> BLOCK_BITS)..=(end >> BLOCK_BITS) {
            self.touched |= 1 << major;
        }
    }

    pub fn clear(&mut self) {
        let len = self.words.len();
        let mut t = self.touched;
        while t != 0 {
            let base = (t.trailing_zeros() as usize) << (BLOCK_BITS - 6);
            t &= t - 1;
            let end = (base + (1 << (BLOCK_BITS - 6))).min(len);
            if base < len {
                self.words[base..end].fill(0);
            }
        }
        self.touched = 0;
        self.has_deleted = false;
    }

    #[inline]
    pub fn intersects_set(&self, other: &GlyphSet) -> bool {
        if self.has_deleted && other.has_deleted {
            return true;
        }
        let n = self.words.len().min(other.words.len());
        self.words[..n]
            .iter()
            .zip(&other.words[..n])
            .any(|(a, b)| a & b != 0)
    }
}
