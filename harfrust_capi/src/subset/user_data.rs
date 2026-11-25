//! Owned object metadata, with destruction outside the lock for reentrancy.
use crate::{hr_destroy_func_t, hr_user_data_key_t};
use core::ffi::c_void;
use std::sync::{Mutex, PoisonError};

struct Item {
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    // Safety invariant: `destroy`, if any, is safe to call once with `data`,
    // on any thread.
    destroy: hr_destroy_func_t,
}
// SAFETY: pointers are opaque; the caller promises thread-safe destruction.
unsafe impl Send for Item {}
impl Drop for Item {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: `destroy` is the caller's function, with whatever it
            // requires of its argument. By the invariant on `destroy`, one
            // call with `data` is sound on whichever thread drops the item,
            // and an item is dropped once.
            unsafe { destroy(self.data) };
        }
    }
}

pub struct UserData {
    // None once final destruction starts. Reentrant setters then reject new
    // metadata instead of adding items to a vector being destroyed.
    items: Mutex<Option<Vec<Item>>>,
}
impl Default for UserData {
    fn default() -> Self {
        Self {
            items: Mutex::new(Some(Vec::new())),
        }
    }
}
impl Drop for UserData {
    fn drop(&mut self) {
        self.release();
    }
}
impl UserData {
    // Called before the owner releases its sets, maps, or font bytes, so
    // callbacks may still query those resources. Detach the items before
    // invoking caller code, just as replacement does below.
    pub fn release(&self) {
        let items = self
            .items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(items);
    }

    // Every `Item` is built here, from `data` and `destroy` as given.
    // Nothing here checks them against the invariant on `Item::destroy`: it
    // is the caller's to supply. Both callers are exported setters whose
    // contracts require the data and destructor to be safe to release from
    // any thread.
    pub fn set(
        &self,
        key: *const hr_user_data_key_t,
        data: *mut c_void,
        destroy: hr_destroy_func_t,
        replace: bool,
    ) -> bool {
        if key.is_null() {
            return false;
        }
        let displaced;
        {
            let Ok(mut items) = self.items.lock() else {
                return false;
            };
            let Some(items) = items.as_mut() else {
                return false;
            };
            let clearing = data.is_null() && destroy.is_none();
            match items.iter().position(|item| item.key == key) {
                Some(_) if !replace => return false,
                Some(index) if clearing => {
                    displaced = Some(items.remove(index));
                }
                Some(index) => {
                    displaced = Some(core::mem::replace(
                        &mut items[index],
                        Item { key, data, destroy },
                    ));
                }
                None if clearing && replace => return true,
                None => {
                    items.push(Item { key, data, destroy });
                    displaced = None;
                }
            }
        }
        drop(displaced);
        true
    }
    pub fn get(&self, key: *const hr_user_data_key_t) -> *mut c_void {
        let Ok(items) = self.items.lock() else {
            return core::ptr::null_mut();
        };
        let Some(items) = items.as_ref() else {
            return core::ptr::null_mut();
        };
        items
            .iter()
            .find(|item| item.key == key)
            .map_or(core::ptr::null_mut(), |item| item.data)
    }
}
