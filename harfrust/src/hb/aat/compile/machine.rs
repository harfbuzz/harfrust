//! A `morx` state machine, decoded.
//!
//! The font stores one as two arrays and a header: a state array of entry
//! indices, `n_classes` wide and one row per state, and an entry table those
//! indices name. Both are big-endian, neither is bounds-checked, and reaching
//! an entry from a state and a class means an indexed read into the first, a
//! multiply, a slice of the second, and a parse -- per glyph, per subtable.
//!
//! None of that depends on the text. This holds the same two arrays in the
//! machine's own byte order, with the entries already parsed, so a step is two
//! indexed loads.

use alloc::boxed::Box;
use alloc::vec::Vec;
use read_fonts::tables::aat::{NoPayload, StateEntry, StateTableParts};
use read_fonts::tables::morx::{self, ContextualEntryData, InsertionEntryData};
use read_fonts::types::BigEndian;
use read_fonts::types::FixedSize;
use read_fonts::{FontData, FontRead};

/// The state array and entry table of one machine, decoded.
///
/// Generic over the payload for the same reason the font's is: a rearrangement
/// entry carries nothing, a ligature entry an action index, a contextual entry
/// two lookup indices. Only the actions read it, and they read it rarely,
/// which is why the payload is left in whatever form the font stated it -- the
/// fields a *step* reads, the next state and the flags, are the ones decoded.
pub struct States<T> {
    n_classes: u32,
    /// Row-major, `n_classes` per state, native endian.
    rows: Box<[u16]>,
    entries: Box<[StateEntry<T>]>,
}

impl<T> States<T>
where
    T: bytemuck::AnyBitPattern + FixedSize,
{
    /// Decode the machine whose header is at the start of `data`.
    ///
    /// `None` if the header does not describe a machine this can hold, in
    /// which case the caller keeps driving from the font.
    pub fn new(data: &[u8]) -> Option<Self> {
        let parts = StateTableParts::read(FontData::new(data)).ok()?;
        let n_classes = parts.n_classes;
        if n_classes == 0 {
            return None;
        }
        let (array_at, entries_at) = (
            parts.state_array_offset as usize,
            parts.entry_table_offset as usize,
        );
        // The state array runs from its own offset to the entry table's, and
        // the entry table from there to the end of the subtable. The font
        // states neither length, which is why the reader that does this per
        // step has a bounds check on every access.
        let array = data.get(array_at..entries_at)?;
        let rows: Vec<u16> = array
            .chunks_exact(2)
            .map(|w| u16::from_be_bytes([w[0], w[1]]))
            .collect();

        // How many entries there are is not stated anywhere, and the bytes
        // after the entry table are not entries: a ligature machine follows
        // its entries with actions, components and ligature glyphs, an
        // insertion machine with the glyphs it inserts. Reading to the end of
        // the subtable decodes all of that as state entries -- 304KiB of them
        // on Lucida Grande, against a real entry table a fraction the size.
        //
        // The state array names every entry that can be reached, so the
        // highest index in it is the last one worth having.
        let reachable = rows.iter().copied().max().map_or(0, |top| top as usize + 1);
        let stride = StateEntry::<T>::RAW_BYTE_LEN.max(1);
        let table = data.get(entries_at..)?;
        let entries: Vec<StateEntry<T>> = table
            .chunks_exact(stride)
            .take(reachable)
            .map_while(|record| StateEntry::<T>::read(FontData::new(record)).ok())
            .collect();
        if entries.is_empty() {
            return None;
        }

        Some(Self {
            n_classes,
            rows: rows.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
        })
    }

    /// The entry for a state and a class.
    ///
    /// Two indexed loads. The class is clamped exactly as the font's reader
    /// clamps it -- a class beyond the machine's width is out of bounds, which
    /// is a class every machine defines -- and everything else that reader
    /// checks is checked here once, when this was built.
    #[inline]
    pub fn entry(&self, state: u16, class: u16) -> Option<&StateEntry<T>> {
        let class = if u32::from(class) >= self.n_classes {
            u32::from(read_fonts::tables::aat::class::OUT_OF_BOUNDS)
        } else {
            u32::from(class)
        };
        let row = u32::from(state)
            .wrapping_mul(self.n_classes)
            .wrapping_add(class);
        let index = *self.rows.get(row as usize)?;
        self.entries.get(index as usize)
    }

    #[cfg(all(test, feature = "std"))]
    pub fn heap_bytes(&self) -> usize {
        self.rows.len() * 2 + self.entries.len() * size_of::<StateEntry<T>>()
    }
}

/// The decoded machine of one `morx` subtable, in whichever shape its kind
/// calls for.
///
/// One variant per payload type, because that is what the payload is: a
/// rearrangement entry carries nothing, a ligature entry an action index, and
/// the other two a pair of indices. `None` is not a failure to be reported --
/// it means the caller drives from the font instead, which it can always do.
#[derive(Default)]
pub enum MorxStates {
    #[default]
    None,
    Rearrangement(States<NoPayload>),
    Contextual(States<ContextualEntryData>),
    Ligature(States<BigEndian<u16>>),
    Insertion(States<InsertionEntryData>),
}

impl MorxStates {
    /// Decode whichever machine this subtable holds.
    pub fn new(kind: &morx::SubtableKind<'_>, data: &[u8]) -> Self {
        match kind {
            morx::SubtableKind::Rearrangement(_) => {
                States::new(data).map_or(Self::None, Self::Rearrangement)
            }
            morx::SubtableKind::Contextual(_) => {
                States::new(data).map_or(Self::None, Self::Contextual)
            }
            morx::SubtableKind::Ligature(_) => States::new(data).map_or(Self::None, Self::Ligature),
            morx::SubtableKind::Insertion(_) => {
                States::new(data).map_or(Self::None, Self::Insertion)
            }
            morx::SubtableKind::NonContextual(_) => Self::None,
        }
    }

    pub fn rearrangement(&self) -> Option<&States<NoPayload>> {
        match self {
            Self::Rearrangement(s) => Some(s),
            _ => None,
        }
    }

    pub fn contextual(&self) -> Option<&States<ContextualEntryData>> {
        match self {
            Self::Contextual(s) => Some(s),
            _ => None,
        }
    }

    pub fn ligature(&self) -> Option<&States<BigEndian<u16>>> {
        match self {
            Self::Ligature(s) => Some(s),
            _ => None,
        }
    }

    pub fn insertion(&self) -> Option<&States<InsertionEntryData>> {
        match self {
            Self::Insertion(s) => Some(s),
            _ => None,
        }
    }

    #[cfg(all(test, feature = "std"))]
    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Rearrangement(s) => s.heap_bytes(),
            Self::Contextual(s) => s.heap_bytes(),
            Self::Ligature(s) => s.heap_bytes(),
            Self::Insertion(s) => s.heap_bytes(),
        }
    }
}
