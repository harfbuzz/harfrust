//! Integer-to-integer maps, mirroring HarfBuzz's map API.

use core::ffi::{c_int, c_uint, c_void};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;

use crate::common::{hr_bool_t, hr_codepoint_t, HR_CODEPOINT_INVALID};
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};
use crate::set::hr_set_t;

/// Value returned for an absent map key.
pub const HR_MAP_VALUE_INVALID: hr_codepoint_t = HR_CODEPOINT_INVALID;

pub struct hr_map_t {
    header: ObjectHeader,
    // Dense entries allow the C iterator to use an integer cursor without
    // rescanning a HashMap on every call. Mutation invalidates that cursor.
    entries: Vec<(u32, u32)>,
    indices: HashMap<u32, usize>,
}

static EMPTY_MAP: OnceLock<Empty<hr_map_t>> = OnceLock::new();

impl Object for hr_map_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_MAP
            .get_or_init(|| {
                Empty::new(hr_map_t {
                    header: ObjectHeader::immortal(),
                    entries: Vec::new(),
                    indices: HashMap::new(),
                })
            })
            .get()
    }
}

impl hr_map_t {
    fn set(&mut self, key: u32, value: u32) {
        if let Some(&index) = self.indices.get(&key) {
            self.entries[index].1 = value;
        } else {
            self.indices.insert(key, self.entries.len());
            self.entries.push((key, value));
        }
    }
}

#[no_mangle]
pub extern "C" fn hr_map_create() -> *mut hr_map_t {
    object::create(hr_map_t {
        header: ObjectHeader::new(),
        entries: Vec::new(),
        indices: HashMap::new(),
    })
}

#[no_mangle]
pub extern "C" fn hr_map_get_empty() -> *mut hr_map_t {
    hr_map_t::empty()
}

/// # Safety
/// `map` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_reference(map: *mut hr_map_t) -> *mut hr_map_t {
    // SAFETY: `map` is `NULL` or live by this function's contract.
    unsafe { object::reference(map) }
}

/// # Safety
/// `map` must be `NULL` or live, and the caller must own its reference.
#[no_mangle]
pub unsafe extern "C" fn hr_map_destroy(map: *mut hr_map_t) {
    // SAFETY: `map` is `NULL` or live, and the reference released is the
    // caller's own to give back, both by this function's contract.
    unsafe { object::destroy(map) };
}

/// # Safety
/// `map` must be `NULL` or live, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_map_set_user_data(
    map: *mut hr_map_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    // SAFETY: `map` is `NULL` or live and `key` outlives it, both by this
    // function's contract. `destroy` and `data` are the caller's, who the
    // README requires, under "Threading", to make them safe to use and to
    // release from any thread that uses the map.
    unsafe { object::set_user_data(map, key, data, destroy, replace != 0) }.into()
}

/// # Safety
/// `map` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_get_user_data(
    map: *const hr_map_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    // SAFETY: `map` is `NULL` or live by this function's contract. `key` is
    // only compared by address.
    unsafe { object::get_user_data(map.cast_mut(), key) }
}

/// Returns false for the inert empty singleton. Rust allocation failures abort.
///
/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_allocation_successful(map: *const hr_map_t) -> hr_bool_t {
    // SAFETY: `or_empty` takes `map`, which is `NULL`, or live by this
    // function's contract, and answers with a reference to that map or to
    // the empty one. `is_immutable` takes a `NULL` or live map, and is
    // handed that reference as a pointer.
    (!unsafe { object::is_immutable(object::or_empty(map)) }).into()
}

/// Copies the contents, without copying user data.
///
/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_copy(map: *const hr_map_t) -> *mut hr_map_t {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    object::create(hr_map_t {
        header: ObjectHeader::new(),
        entries: map.entries.clone(),
        indices: map.indices.clone(),
    })
}

/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_clear(map: *mut hr_map_t) {
    // SAFETY: `map` is `NULL`, or live by this function's contract, and what
    // `as_mutable` returns must not be aliased. It returns `None` for a map
    // that takes no changes, the shared empty map among them. Any other is
    // being modified by this caller, who has exclusive access to it: the
    // README requires that under "Threading". This call makes no other
    // reference to it.
    if let Some(map) = unsafe { object::as_mutable(map) } {
        map.entries.clear();
        map.indices.clear();
    }
}

/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_is_empty(map: *const hr_map_t) -> hr_bool_t {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(map) }.entries.is_empty().into()
}

/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_get_population(map: *const hr_map_t) -> c_uint {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(map) }.entries.len() as c_uint
}

/// # Safety
/// Both maps must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_is_equal(
    map: *const hr_map_t,
    other: *const hr_map_t,
) -> hr_bool_t {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    // SAFETY: `other` is `NULL`, or live by this function's contract. Both
    // references are shared, so the two may be one map.
    let other = unsafe { object::or_empty(other) };
    (map.entries.len() == other.entries.len()
        && map.entries.iter().all(|&(key, value)| {
            other
                .indices
                .get(&key)
                .is_some_and(|&index| other.entries[index].1 == value)
        }))
    .into()
}

/// Equal maps have equal hashes, regardless of insertion order. The hash
/// value is implementation dependent.
///
/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_hash(map: *const hr_map_t) -> c_uint {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(map) }
        .entries
        .iter()
        .fold(0, |hash, entry| {
            let mut item_hash = DefaultHasher::new();
            entry.hash(&mut item_hash);
            hash ^ item_hash.finish() as c_uint
        })
}

/// Stores a key/value pair, replacing any previous value. All 32-bit keys and
/// values are accepted, including `HR_MAP_VALUE_INVALID`.
///
/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_set(
    map: *mut hr_map_t,
    key: hr_codepoint_t,
    value: hr_codepoint_t,
) {
    // SAFETY: as in `hr_map_clear`: `map` is `NULL`, or live by this
    // function's contract, and a map that takes changes is the caller's
    // alone while it is modified, as the README requires under "Threading".
    if let Some(map) = unsafe { object::as_mutable(map) } {
        map.set(key, value);
    }
}

/// Returns the stored value, or `HR_MAP_VALUE_INVALID` when absent. Use
/// `hr_map_has` to distinguish an absent key from a stored invalid value.
///
/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_get(map: *const hr_map_t, key: hr_codepoint_t) -> hr_codepoint_t {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    map.indices
        .get(&key)
        .map_or(HR_MAP_VALUE_INVALID, |&index| map.entries[index].1)
}

/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_del(map: *mut hr_map_t, key: hr_codepoint_t) {
    // SAFETY: as in `hr_map_clear`: `map` is `NULL`, or live by this
    // function's contract, and a map that takes changes is the caller's
    // alone while it is modified, as the README requires under "Threading".
    if let Some(map) = unsafe { object::as_mutable(map) } {
        if let Some(index) = map.indices.remove(&key) {
            map.entries.swap_remove(index);
            if let Some(&(moved, _)) = map.entries.get(index) {
                *map.indices.get_mut(&moved).unwrap() = index;
            }
        }
    }
}

/// # Safety
/// `map` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_has(map: *const hr_map_t, key: hr_codepoint_t) -> hr_bool_t {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    unsafe { object::or_empty(map) }
        .indices
        .contains_key(&key)
        .into()
}

/// Adds the source's entries, replacing values for keys already present.
///
/// # Safety
/// Both maps must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_update(map: *mut hr_map_t, other: *const hr_map_t) {
    let other = if other.is_null() {
        hr_map_t::empty().cast_const()
    } else {
        other
    };
    if core::ptr::eq(map, other) {
        return;
    }
    // SAFETY: `other` is the caller's pointer, live by this function's
    // contract, or the empty map standing in for `NULL`.
    let other = unsafe { object::or_empty(other) };
    // SAFETY: `map` is `NULL`, or live by this function's contract, and a map
    // that takes changes is the caller's alone while it is modified, as the
    // README requires under "Threading". `other` is a different map, as
    // checked above, so the reference to it does not alias this one.
    if let Some(map) = unsafe { object::as_mutable(map) } {
        for &(key, value) in &other.entries {
            map.set(key, value);
        }
    }
}

/// Iterates entries in unspecified order. Start with `*idx = -1`;
/// exhaustion resets it to -1 and leaves key and value unchanged.
///
/// # Safety
/// `map` must be live and must not be modified during iteration.
/// `idx`, `key` and `value` must point to writable, nonoverlapping storage.
#[no_mangle]
pub unsafe extern "C" fn hr_map_next(
    map: *const hr_map_t,
    idx: *mut c_int,
    key: *mut hr_codepoint_t,
    value: *mut hr_codepoint_t,
) -> hr_bool_t {
    if idx.is_null() || key.is_null() || value.is_null() {
        return 0;
    }
    // SAFETY: the read needs `idx` aligned, readable and holding a value. It
    // is not null, as checked above, and this function's contract has it
    // point to storage for a `c_int`. It is the iteration cursor, so it
    // carries a value in, which the README has the caller set before the
    // call.
    let next = unsafe { *idx }.wrapping_add(1) as usize;
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    if let Some(&(k, v)) = map.entries.get(next) {
        // SAFETY: each write needs its pointer aligned and writable. None of
        // the three is null, as checked above, and this function's contract
        // has each point to writable storage. `key` and `value` only return
        // a value: they are assigned through the pointer, with no reference
        // made and nothing read, so they need not have been set.
        unsafe {
            *idx = next as c_int;
            *key = k;
            *value = v;
        }
        1
    } else {
        // SAFETY: `idx` is not null, as checked above, and writable by this
        // function's contract.
        unsafe {
            *idx = -1;
        }
        0
    }
}

/// Adds the map's keys to the supplied set, preserving existing members.
///
/// # Safety
/// `map` and `keys` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_keys(map: *const hr_map_t, keys: *mut hr_set_t) {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    // SAFETY: `keys` is `NULL`, or live by this function's contract, and a set
    // that takes changes is the caller's alone while it is modified, as the
    // README requires under "Threading". The reference to `map` does not
    // alias it: a map and a set are separate objects.
    if let Some(keys) = unsafe { object::as_mutable(keys) } {
        for &(key, _) in &map.entries {
            keys.add(key);
        }
    }
}

/// Adds the map's values to the supplied set, preserving existing members.
///
/// # Safety
/// `map` and `values` must be live.
#[no_mangle]
pub unsafe extern "C" fn hr_map_values(map: *const hr_map_t, values: *mut hr_set_t) {
    // SAFETY: `map` is `NULL`, or live by this function's contract.
    let map = unsafe { object::or_empty(map) };
    // SAFETY: `values` is `NULL`, or live by this function's contract, and a
    // set that takes changes is the caller's alone while it is modified, as
    // the README requires under "Threading". The reference to `map` does not
    // alias it: a map and a set are separate objects.
    if let Some(values) = unsafe { object::as_mutable(values) } {
        for &(_, value) in &map.entries {
            values.add(value);
        }
    }
}
