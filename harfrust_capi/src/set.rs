//! Sets of codepoints and glyph IDs used by layout queries.

use core::ffi::c_void;
use core::ops::RangeInclusive;
use read_fonts::collections::IntSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;

use crate::common::{hr_bool_t, hr_codepoint_t, HR_CODEPOINT_INVALID};
use crate::object::{self, hr_destroy_func_t, hr_user_data_key_t, Empty, Object, ObjectHeader};

/// Unset set value, also used to start iteration.
pub const HR_SET_VALUE_INVALID: hr_codepoint_t = HR_CODEPOINT_INVALID;

pub struct hr_set_t {
    header: ObjectHeader,
    pub(crate) values: IntSet<u32>,
}

static EMPTY_SET: OnceLock<Empty<hr_set_t>> = OnceLock::new();

impl Object for hr_set_t {
    fn header(&self) -> &ObjectHeader {
        &self.header
    }

    fn empty() -> *mut Self {
        EMPTY_SET
            .get_or_init(|| {
                Empty::new(hr_set_t {
                    header: ObjectHeader::immortal(),
                    values: IntSet::default(),
                })
            })
            .get()
    }
}

impl hr_set_t {
    // IntSet's domain includes the invalid value. Like HarfBuzz, we never
    // insert it into the underlying bitset, and omit it from enumeration
    // and population even though an inverted set answers true to has().
    fn ranges(&self) -> impl Iterator<Item = RangeInclusive<u32>> + '_ {
        self.values.iter_ranges().filter_map(|range| {
            let first = *range.start();
            let last = (*range.end()).min(HR_SET_VALUE_INVALID - 1);
            (first <= last).then_some(first..=last)
        })
    }

    fn next(&self, start: u32) -> u32 {
        if start == HR_SET_VALUE_INVALID {
            return HR_SET_VALUE_INVALID;
        }
        if !self.values.is_inverted() {
            // The bitset seeks directly to the page containing start.
            return self
                .values
                .range(start..)
                .next()
                .unwrap_or(HR_SET_VALUE_INVALID);
        }
        if self.values.contains(start) {
            return start;
        }
        // Skip excluded ranges without enumerating each excluded codepoint.
        self.ranges()
            .find(|r| *r.end() >= start)
            .map_or(HR_SET_VALUE_INVALID, |r| start.max(*r.start()))
    }

    pub(crate) fn add(&mut self, value: u32) {
        if value != HR_CODEPOINT_INVALID {
            self.values.insert(value);
        }
    }

    pub(crate) fn include_all_glyphs(&mut self, count: u32) {
        if count > 0 {
            self.values.insert_range(0..=count - 1);
        }
    }
}

#[no_mangle]
pub extern "C" fn hr_set_create() -> *mut hr_set_t {
    object::create(hr_set_t {
        header: ObjectHeader::new(),
        values: IntSet::default(),
    })
}

#[no_mangle]
pub extern "C" fn hr_set_get_empty() -> *mut hr_set_t {
    hr_set_t::empty()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_reference(set: *mut hr_set_t) -> *mut hr_set_t {
    unsafe { object::reference(set) }
}

/// # Safety
/// `set` must be `NULL` or live, and the caller must own its reference.
#[no_mangle]
pub unsafe extern "C" fn hr_set_destroy(set: *mut hr_set_t) {
    unsafe { object::destroy(set) };
}

/// # Safety
/// `set` must be `NULL` or live, and `key` must outlive it.
#[no_mangle]
pub unsafe extern "C" fn hr_set_set_user_data(
    set: *mut hr_set_t,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy: hr_destroy_func_t,
    replace: hr_bool_t,
) -> hr_bool_t {
    unsafe { object::set_user_data(set, key, data, destroy, replace != 0) }.into()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_get_user_data(
    set: *const hr_set_t,
    key: *const hr_user_data_key_t,
) -> *mut c_void {
    unsafe { object::get_user_data(set.cast_mut(), key) }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_clear(set: *mut hr_set_t) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        set.values.clear();
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_add(set: *mut hr_set_t, value: hr_codepoint_t) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        set.add(value);
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_has(set: *const hr_set_t, value: hr_codepoint_t) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    set.values.contains(value).into()
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_is_empty(set: *const hr_set_t) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    (unsafe { hr_set_get_population(set) } == 0).into()
}

/// Returns false for the inert empty singleton. Rust allocation failures abort.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_allocation_successful(set: *const hr_set_t) -> hr_bool_t {
    (!unsafe { object::is_immutable(object::or_empty(set)) }).into()
}

/// Copies the contents, without copying user data.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_copy(set: *const hr_set_t) -> *mut hr_set_t {
    object::create(hr_set_t {
        header: ObjectHeader::new(),
        values: unsafe { object::or_empty(set) }.values.clone(),
    })
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_invert(set: *mut hr_set_t) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        set.values.invert();
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_is_inverted(set: *const hr_set_t) -> hr_bool_t {
    unsafe { object::or_empty(set) }.values.is_inverted().into()
}

/// Adds the inclusive range. Reversed ranges are ignored. An invalid `last`
/// is ignored for ordinary sets and extends to the end for inverted sets.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_add_range(
    set: *mut hr_set_t,
    first: hr_codepoint_t,
    last: hr_codepoint_t,
) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        if first <= last
            && first != HR_SET_VALUE_INVALID
            && (last != HR_SET_VALUE_INVALID || set.values.is_inverted())
        {
            set.values
                .insert_range(first..=last.min(HR_SET_VALUE_INVALID - 1));
        }
    }
}

/// Adds codepoints supplied in increasing order.
///
/// # Safety
/// `set` must be `NULL` or live. `sorted_codepoints` must point to
/// `num_codepoints` readable elements, or be `NULL` when the count is zero.
#[no_mangle]
pub unsafe extern "C" fn hr_set_add_sorted_array(
    set: *mut hr_set_t,
    sorted_codepoints: *const hr_codepoint_t,
    num_codepoints: core::ffi::c_uint,
) {
    if num_codepoints == 0 || sorted_codepoints.is_null() {
        return;
    }
    if let Some(set) = unsafe { object::as_mutable(set) } {
        let values =
            unsafe { core::slice::from_raw_parts(sorted_codepoints, num_codepoints as usize) };
        set.values.extend(
            values
                .iter()
                .copied()
                .filter(|&v| v != HR_SET_VALUE_INVALID),
        );
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_del(set: *mut hr_set_t, value: hr_codepoint_t) {
    if value != HR_SET_VALUE_INVALID {
        if let Some(set) = unsafe { object::as_mutable(set) } {
            set.values.remove(value);
        }
    }
}

/// Deletes the inclusive range. An invalid `last` extends to the end for
/// ordinary sets and is ignored for inverted sets.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_del_range(
    set: *mut hr_set_t,
    first: hr_codepoint_t,
    last: hr_codepoint_t,
) {
    if let Some(set) = unsafe { object::as_mutable(set) } {
        if first <= last
            && first != HR_SET_VALUE_INVALID
            && (last != HR_SET_VALUE_INVALID || !set.values.is_inverted())
        {
            set.values
                .remove_range(first..=last.min(HR_SET_VALUE_INVALID - 1));
        }
    }
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_is_equal(
    set: *const hr_set_t,
    other: *const hr_set_t,
) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    let other = unsafe { object::or_empty(other) };
    set.ranges().eq(other.ranges()).into()
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_intersects(
    set: *const hr_set_t,
    other: *const hr_set_t,
) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    let other = unsafe { object::or_empty(other) };
    let mut a = set.ranges();
    let mut b = other.ranges();
    let (mut x, mut y) = (a.next(), b.next());
    while let (Some(ar), Some(br)) = (&x, &y) {
        if ar.end() < br.start() {
            x = a.next();
        } else if br.end() < ar.start() {
            y = b.next();
        } else {
            return 1;
        }
    }
    0
}

/// Equal sets have equal hashes. The hash value is implementation dependent.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_hash(set: *const hr_set_t) -> core::ffi::c_uint {
    let mut hash = DefaultHasher::new();
    for range in unsafe { object::or_empty(set) }.ranges() {
        range.hash(&mut hash);
    }
    hash.finish() as core::ffi::c_uint
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_is_subset(
    set: *const hr_set_t,
    larger_set: *const hr_set_t,
) -> hr_bool_t {
    let set = unsafe { object::or_empty(set) };
    let larger = unsafe { object::or_empty(larger_set) };
    let mut larger_ranges = larger.ranges();
    let mut covering = larger_ranges.next();
    for range in set.ranges() {
        while covering.as_ref().is_some_and(|r| r.end() < range.start()) {
            covering = larger_ranges.next();
        }
        if !covering
            .as_ref()
            .is_some_and(|r| r.start() <= range.start() && r.end() >= range.end())
        {
            return 0;
        }
    }
    1
}

// Check pointer identity before making references: the C API permits self
// assignment and self operations, but Rust cannot alias &mut and &.
unsafe fn combine(
    set: *mut hr_set_t,
    other: *const hr_set_t,
    op: fn(&mut IntSet<u32>, &IntSet<u32>),
    subtract_self: bool,
) {
    let other = if other.is_null() {
        hr_set_t::empty().cast_const()
    } else {
        other
    };
    if core::ptr::eq(set, other) {
        if subtract_self {
            unsafe { hr_set_clear(set) };
        }
        return;
    }
    let other = unsafe { object::or_empty(other) };
    if let Some(set) = unsafe { object::as_mutable(set) } {
        op(&mut set.values, &other.values);
    }
}

/// Replaces the contents, retaining the destination's user data.
///
/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_set(set: *mut hr_set_t, other: *const hr_set_t) {
    unsafe { combine(set, other, IntSet::clone_from, false) };
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_union(set: *mut hr_set_t, other: *const hr_set_t) {
    unsafe { combine(set, other, IntSet::union, false) };
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_intersect(set: *mut hr_set_t, other: *const hr_set_t) {
    unsafe { combine(set, other, IntSet::intersect, false) };
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_subtract(set: *mut hr_set_t, other: *const hr_set_t) {
    unsafe { combine(set, other, IntSet::subtract, true) };
}

/// # Safety
/// Both sets must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_symmetric_difference(set: *mut hr_set_t, other: *const hr_set_t) {
    unsafe {
        combine(
            set,
            other,
            |a, b| {
                let mut rhs = b.clone();
                rhs.subtract(a);
                a.subtract(b);
                a.union(&rhs);
            },
            true,
        );
    }
}

/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_get_population(set: *const hr_set_t) -> core::ffi::c_uint {
    let values = &unsafe { object::or_empty(set) }.values;
    (values.len() - u64::from(values.is_inverted())) as core::ffi::c_uint
}

/// Returns the smallest value, or `HR_SET_VALUE_INVALID` if empty.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_get_min(set: *const hr_set_t) -> hr_codepoint_t {
    unsafe { object::or_empty(set) }.next(0)
}

/// Returns the largest value, or `HR_SET_VALUE_INVALID` if empty.
///
/// # Safety
/// `set` must be `NULL` or live.
#[no_mangle]
pub unsafe extern "C" fn hr_set_get_max(set: *const hr_set_t) -> hr_codepoint_t {
    let set = unsafe { object::or_empty(set) };
    if !set.values.is_inverted() {
        return set.values.last().unwrap_or(HR_SET_VALUE_INVALID);
    }
    if set.values.contains(HR_SET_VALUE_INVALID - 1) {
        return HR_SET_VALUE_INVALID - 1;
    }
    set.ranges()
        .last()
        .map_or(HR_SET_VALUE_INVALID, |r| *r.end())
}

/// Finds the next value. Start with `HR_SET_VALUE_INVALID`; exhaustion writes
/// that value again.
///
/// # Safety
/// `set` must be `NULL` or live. `codepoint` must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_set_next(
    set: *const hr_set_t,
    codepoint: *mut hr_codepoint_t,
) -> hr_bool_t {
    let Some(codepoint) = (unsafe { codepoint.as_mut() }) else {
        return 0;
    };
    let start = codepoint.wrapping_add(1);
    *codepoint = unsafe { object::or_empty(set) }.next(start);
    (*codepoint != HR_SET_VALUE_INVALID).into()
}

/// Finds the preceding value. Start with `HR_SET_VALUE_INVALID`; exhaustion
/// writes that value again.
///
/// # Safety
/// `set` must be `NULL` or live. `codepoint` must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_set_previous(
    set: *const hr_set_t,
    codepoint: *mut hr_codepoint_t,
) -> hr_bool_t {
    let Some(codepoint) = (unsafe { codepoint.as_mut() }) else {
        return 0;
    };
    let end = *codepoint;
    *codepoint = unsafe { object::or_empty(set) }
        .ranges()
        .take_while(|r| *r.start() < end)
        .last()
        .map_or(HR_SET_VALUE_INVALID, |r| (end - 1).min(*r.end()));
    (*codepoint != HR_SET_VALUE_INVALID).into()
}

/// Finds the next contiguous range after `*last`. Start with
/// `HR_SET_VALUE_INVALID`; exhaustion writes it to both outputs.
///
/// # Safety
/// `set` must be `NULL` or live. `first` and `last` must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_set_next_range(
    set: *const hr_set_t,
    first: *mut hr_codepoint_t,
    last: *mut hr_codepoint_t,
) -> hr_bool_t {
    if first.is_null() || last.is_null() {
        return 0;
    }
    let start = unsafe { *last }.wrapping_add(1);
    let range = unsafe { object::or_empty(set) }
        .ranges()
        .find(|r| *r.end() >= start);
    let (a, b) = range.map_or((HR_SET_VALUE_INVALID, HR_SET_VALUE_INVALID), |r| {
        (start.max(*r.start()), *r.end())
    });
    unsafe {
        *first = a;
        *last = b;
    }
    (a != HR_SET_VALUE_INVALID).into()
}

/// Finds the preceding contiguous range before `*first`. Start with
/// `HR_SET_VALUE_INVALID`; exhaustion writes it to both outputs.
///
/// # Safety
/// `set` must be `NULL` or live. `first` and `last` must be writable.
#[no_mangle]
pub unsafe extern "C" fn hr_set_previous_range(
    set: *const hr_set_t,
    first: *mut hr_codepoint_t,
    last: *mut hr_codepoint_t,
) -> hr_bool_t {
    if first.is_null() || last.is_null() {
        return 0;
    }
    let end = unsafe { *first };
    let range = unsafe { object::or_empty(set) }
        .ranges()
        .take_while(|r| *r.start() < end)
        .last();
    let (a, b) = range.map_or((HR_SET_VALUE_INVALID, HR_SET_VALUE_INVALID), |r| {
        (*r.start(), (end - 1).min(*r.end()))
    });
    unsafe {
        *first = a;
        *last = b;
    }
    (a != HR_SET_VALUE_INVALID).into()
}

/// Writes at most `size` values after `codepoint`. Start with
/// `HR_SET_VALUE_INVALID`. Returns the number written.
///
/// # Safety
/// `set` must be `NULL` or live. `out` must point to `size` writable elements,
/// or be `NULL` when `size` is zero.
#[no_mangle]
pub unsafe extern "C" fn hr_set_next_many(
    set: *const hr_set_t,
    codepoint: hr_codepoint_t,
    out: *mut hr_codepoint_t,
    size: core::ffi::c_uint,
) -> core::ffi::c_uint {
    if size == 0 || out.is_null() {
        return 0;
    }
    let start = codepoint.wrapping_add(1);
    let values = unsafe { object::or_empty(set) }
        .ranges()
        .filter(|r| *r.end() >= start)
        .flat_map(|r| start.max(*r.start())..=*r.end());
    let out = unsafe { core::slice::from_raw_parts_mut(out, size as usize) };
    let mut count = 0;
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value;
        count += 1;
    }
    count
}
