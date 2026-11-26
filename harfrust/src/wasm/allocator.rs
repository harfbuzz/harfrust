use wasmi::{Caller, Memory, Store};

use super::{ptr_add, read_u32};
use crate::wasm::Host;
use alloc::vec::Vec;

/// Pages the host takes at a time when it needs room for the module. A block
/// larger than this gets the pages it needs.
const ALLOC_PAGES_MIN: u32 = 4;

/// A block of the guest's memory that the host handed out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Block {
    offset: u32,
    len: u32,
    free: bool,
}

/// The host's allocator for the pages it takes in the guest's memory.
///
/// The blocks are recorded here, in offsets: the memory can move when it
/// grows, and an offset survives that where a pointer would not. The memory
/// cannot shrink, so what matters is the room the host has live rather than
/// what it has ever asked for.
#[derive(Default)]
pub(crate) struct Heap {
    /// The blocks handed out or freed, in address order.
    blocks: Vec<Block>,
    /// The next address to hand out, and the end of the pages taken.
    next: u32,
    end: u32,
}

impl Heap {
    /// Takes a block of `size` bytes within the pages taken so far.
    fn alloc_within(&mut self, size: u32) -> Option<u32> {
        if let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.free && block.len >= size)
        {
            return Some(self.take_free(index, size));
        }
        let stop = self.next.checked_add(size)?;
        if stop > self.end {
            return None;
        }
        let offset = self.next;
        self.next = stop;
        self.place(offset, size);
        Some(offset)
    }

    /// Takes `pages` pages from `start`, placing a block of `size` bytes at
    /// the beginning and leaving the rest for later requests.
    fn grow(&mut self, start: u32, pages: u32, size: u32) -> Option<u32> {
        let end = start.checked_add(pages.checked_mul(65536)?)?;
        let stop = start.checked_add(size)?;
        if stop > end {
            return None;
        }
        self.next = stop;
        self.end = end;
        self.place(start, size);
        Some(start)
    }

    /// Returns the block at `offset` to the heap.
    ///
    /// An offset the host never handed out is ignored, as is one that is
    /// already free: a guest may put its own pointer in one of these
    /// structures, or return the same block twice.
    pub fn free(&mut self, offset: u32) {
        let Ok(index) = self
            .blocks
            .binary_search_by_key(&offset, |block| block.offset)
        else {
            return;
        };
        if self.blocks[index].free {
            return;
        }
        self.blocks[index].free = true;
        self.join(index);
        if index > 0 {
            self.join(index - 1);
        }
    }

    /// Takes the start of the free block at `index`, leaving whatever is left
    /// of it free.
    fn take_free(&mut self, index: usize, size: u32) -> u32 {
        let block = self.blocks[index];
        self.blocks[index] = Block {
            offset: block.offset,
            len: size,
            free: false,
        };
        if let Some(rest) = block.len.checked_sub(size).filter(|rest| *rest > 0) {
            self.blocks.insert(
                index + 1,
                Block {
                    offset: block.offset + size,
                    len: rest,
                    free: true,
                },
            );
        }
        block.offset
    }

    /// Joins the block at `index` with the one after it when both are free and
    /// touch.
    fn join(&mut self, index: usize) {
        let Some(next) = self.blocks.get(index + 1) else {
            return;
        };
        let block = self.blocks[index];
        if !block.free || !next.free || block.offset.saturating_add(block.len) != next.offset {
            return;
        }
        self.blocks[index].len = block.len.saturating_add(next.len);
        self.blocks.remove(index + 1);
    }

    /// Records a block of `len` bytes at `offset`.
    fn place(&mut self, offset: u32, len: u32) {
        let at = self.blocks.partition_point(|block| block.offset < offset);
        self.blocks.insert(
            at,
            Block {
                offset,
                len,
                free: false,
            },
        );
    }
}

/// Allocates from the guest memory for the host.
///
/// The host takes pages at the end of the memory for its own use, so what it
/// hands out cannot collide with the guest's own heap. A block the guest has
/// returned is handed out again before any more pages are taken.
fn alloc_in<'a, 'b: 'a, 'c: 'a>(
    mut ctx: impl wasmi::AsContextMut<Data = Host<'a, 'b, 'c>>,
    memory: Memory,
    size: u32,
    align: u32,
) -> Option<u32> {
    let size = align_up(size.max(1), align)?;
    if let Some(offset) = ctx.as_context_mut().data_mut().heap.alloc_within(size) {
        return Some(offset);
    }

    // The memory's size is a whole number of pages, which is aligned enough
    // for any block.
    let start = u32::try_from(memory.data_size(&ctx)).ok()?;
    let needed = size.div_ceil(65536);
    let wanted = needed.max(ALLOC_PAGES_MIN);
    // A module with little room left may only grow by what the block needs.
    let pages = match memory.grow(&mut ctx, u64::from(wanted)) {
        Ok(_) => wanted,
        Err(_) if wanted > needed => {
            memory.grow(&mut ctx, u64::from(needed)).ok()?;
            needed
        }
        Err(_) => return None,
    };
    ctx.as_context_mut()
        .data_mut()
        .heap
        .grow(start, pages, size)
}

/// Returns to the heap the block a guest structure's pointer field names.
///
/// Only a pointer the host handed out is in the heap; one the guest put there
/// itself is left alone.
pub(super) fn free_field(
    caller: &mut Caller<'_, Host<'_, '_, '_>>,
    memory: Memory,
    base: u32,
    offset: u32,
) {
    let Some(addr) = ptr_add(base, offset) else {
        return;
    };
    let Some(ptr) = read_u32(&*caller, memory, addr) else {
        return;
    };
    caller.data_mut().heap.free(ptr);
}

/// Allocates from the guest memory used by a host function.
pub(super) fn host_alloc(
    caller: &mut Caller<'_, Host<'_, '_, '_>>,
    size: u32,
    align: u32,
) -> Option<u32> {
    let memory = caller.data().memory?;
    alloc_in(caller, memory, size, align)
}

/// Allocates from the guest memory outside a host function.
pub(super) fn store_alloc(
    store: &mut Store<Host<'_, '_, '_>>,
    memory: Memory,
    size: u32,
    align: u32,
) -> Option<u32> {
    alloc_in(store, memory, size, align)
}

fn align_up(value: u32, align: u32) -> Option<u32> {
    let align = align.max(8);
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

#[cfg(test)]
mod tests {
    use super::{Block, Heap, ALLOC_PAGES_MIN};

    /// A heap with four pages taken, as the first allocation leaves it.
    fn heap() -> Heap {
        let mut heap = Heap::default();
        assert_eq!(heap.grow(65536, ALLOC_PAGES_MIN, 32), Some(65536));
        heap
    }

    #[test]
    fn blocks_are_taken_from_the_pages_grown() {
        let mut heap = heap();
        assert_eq!(heap.alloc_within(64), Some(65568));
        assert_eq!(heap.alloc_within(64), Some(65632));
        // Four pages, less the 32 bytes the growth placed first.
        assert_eq!(heap.alloc_within(4 * 65536), None);
        assert_eq!(heap.alloc_within(4 * 65536 - 32 - 128), Some(65696));
    }

    #[test]
    fn a_freed_block_is_used_again() {
        let mut heap = heap();
        let first = heap.alloc_within(64).unwrap();
        let second = heap.alloc_within(64).unwrap();
        heap.free(second);
        // Freeing the same block twice is ignored, as are offsets the host
        // never handed out.
        heap.free(second);
        heap.free(0);
        heap.free(first - 8);
        assert_eq!(heap.alloc_within(64), Some(second));
        // The block is in use again, so the next request goes after it.
        assert_eq!(heap.alloc_within(64), Some(second + 64));
    }

    #[test]
    fn neighbours_join_when_freed() {
        let mut heap = heap();
        let first = heap.alloc_within(64).unwrap();
        let second = heap.alloc_within(64).unwrap();
        let third = heap.alloc_within(64).unwrap();
        heap.free(second);
        heap.free(first);
        heap.free(third);
        // One block of 192 bytes now, which no single block held before.
        assert_eq!(heap.alloc_within(192), Some(first));
        assert_eq!(
            heap.blocks[1],
            Block {
                offset: first,
                len: 192,
                free: false,
            }
        );
    }

    #[test]
    fn a_free_block_is_split_and_the_rest_kept() {
        let mut heap = heap();
        let first = heap.alloc_within(128).unwrap();
        heap.free(first);
        assert_eq!(heap.alloc_within(64), Some(first));
        assert_eq!(heap.alloc_within(64), Some(first + 64));
        assert_eq!(
            heap.blocks[1],
            Block {
                offset: first,
                len: 64,
                free: false,
            }
        );
    }

    #[test]
    fn a_block_is_not_taken_from_the_middle_of_another() {
        let mut heap = heap();
        let first = heap.alloc_within(64).unwrap();
        let second = heap.alloc_within(64).unwrap();
        // The second block is not free, so a request larger than the first
        // block's remains goes after both rather than into either.
        heap.free(first);
        assert_eq!(heap.alloc_within(128), Some(second + 64));
    }
}
