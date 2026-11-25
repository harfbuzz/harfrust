//! Reference counting, user data and immutability, mirroring HarfBuzz's
//! object model (`hb-object.hh`).
//!
//! Every public object in this API carries an [`ObjectHeader`] as its first
//! field. Objects are heap allocated and handed to C as raw pointers, with the
//! usual `hr_*_reference` / `hr_*_destroy` pair managing their lifetime.
//!
//! Each type also has an immortal "empty" singleton, returned by
//! `hr_*_get_empty()` and used in place of `NULL` whenever a constructor
//! fails. Immortal objects carry a reference count of [`IMMORTAL`]; referencing
//! and destroying them are both no-ops, and they reject user data, exactly as
//! in HarfBuzz.
//!
//! # Live pointers
//!
//! The functions here ask that an object pointer be "`NULL` or live", and
//! the exported ones built on them ask that it be live. A pointer to a `T`
//! is *live* when both of these hold:
//!
//! - It is the pointer this API made for an object, or a copy of it. Each
//!   one is what `Box::into_raw` gave for a `Box<T>`, in [`create`] or in
//!   [`Empty::new`], so it is non-null and aligned and points to a `T`.
//! - The object has not been freed. An immortal object never is. Any other
//!   is freed by the [`destroy`] that releases its last reference, once the
//!   object's destroy callbacks have run.
//!
//! A function that asks for a live pointer needs it to stay live until the
//! function returns, and for as long as a reference the function returns is
//! used.
//!
//! A pointer made from a shared reference to such an object will do in place
//! of a live one, for as long as that reference could itself be used,
//! wherever the object is only read, which includes everything done through
//! its header. It will not do for [`as_mutable`] or [`destroy`].
//!
//! # `NULL` for an object
//!
//! An exported function asks that each object it is passed be live, as
//! HarfBuzz does, but for the few arguments its documentation says may be
//! `NULL`. It is sound when handed `NULL` for one all the same. [`or_empty`]
//! and [`as_mutable`] take `NULL`, which makes it the type's empty object to
//! the functions that use them, and a function that reads an object without
//! them checks for `NULL` first and does nothing with it. The subset input
//! and plan, which have no empty object, are always read that way.
//!
//! So a caller is promised nothing about `NULL`, and nothing is assumed of
//! one that passes it. That is why the comments in an exported function
//! split on it, as in "`font` is `NULL`, or live by this function's
//! contract": a `NULL` pointer needs no contract, and one that is not `NULL`
//! is live because the contract says so. A helper documented as asking what
//! an exported function asks is written the same way, since it is handed
//! that function's arguments as they came. And where a comment says that an
//! exported function *takes* `NULL`, it speaks of what the function is sound
//! for, not of what its documentation asks.
//!
//! # Owning a reference
//!
//! [`create`] hands its caller the first reference to an object, and
//! [`reference()`] hands out another. Whoever is handed a reference owns it
//! until they pass it on or release it, once, with [`destroy`]. But for the
//! exception below, an object is not freed while a reference to it is owned,
//! so its pointer stays live for at least that long. Using an object takes
//! owning a reference to it, or being lent the object by someone who keeps
//! theirs meanwhile, as a function is lent its arguments.
//!
//! The exception is the destroy callbacks of an object whose last reference
//! has been released. The object is still live while they run, and they may
//! use it. They may take references to it too, but one taken then keeps
//! nothing alive. Whatever a destroy callback does with the object, and
//! whatever it hands the object to, must be done with it before the callback
//! returns. The README asks this of callers under "Object lifetime".
//!
//! # Exclusive access
//!
//! Any number of shared references to a live object may be in use at once,
//! and the functions here make one whenever they need the header, which is
//! atomics and a lock. What is not behind such interior mutability is
//! written only by code that has the object to itself. Whoever makes an
//! exclusive reference to an object, or writes it through its pointer,
//! answers for no other reference to it being in use meanwhile, and so for
//! nothing else using the object at all. Setters go through [`as_mutable`],
//! whose contract says so, and which turns away the objects nobody has to
//! themselves: an immortal one, which every thread shares, and one that is
//! releasing its last reference, which its destroy callbacks may be reading.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{PoisonError, RwLock};

/// Reference count of an immortal (inert) object.
const IMMORTAL: i64 = -1;

/// Reference count of an object whose last reference has been released, for
/// as long as its destroy callbacks run.
///
/// Those callbacks may still use the object, and one that takes a reference
/// of its own and gives it back would otherwise walk the count from zero to
/// one and back, releasing the object a second time. Parked this far below
/// zero, no number of references a callback could take brings the count back
/// to one, or to [`IMMORTAL`]. HarfBuzz parks its count the same way.
const DYING: i64 = i64::MIN / 2;

/// Callback invoked to release a `user_data` value.
pub type hr_destroy_func_t = Option<unsafe extern "C" fn(user_data: *mut c_void)>;

/// Key used to attach and retrieve user data on an object.
///
/// Only the address of the key matters; its contents are never read.
#[repr(C)]
pub struct hr_user_data_key_t {
    /// Unused; present so the struct has a well-defined size in C.
    pub unused: core::ffi::c_char,
}

/// One `user_data` entry, owning its value.
struct UserDataItem {
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    // Safety invariant: `destroy`, if any, is safe to call once with `data`,
    // on any thread.
    destroy: hr_destroy_func_t,
}

// SAFETY: the pointers are opaque to us; we only ever compare `key` by address
// and hand `data` back to the caller, and to the destroy callback it supplied.
// Callers are required to make their user data safe to use and to release
// from any thread that uses the object, which the README documents under
// "Threading".
unsafe impl Send for UserDataItem {}
// SAFETY: as for `Send`. Sharing an item shares its two pointers and nothing
// they point to: `key` is still only compared, and `data` only handed back,
// on whichever thread asks.
unsafe impl Sync for UserDataItem {}

impl Drop for UserDataItem {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: by the invariant on `destroy`, one call with `data` is
            // sound on whichever thread drops the item, and an item is
            // dropped once.
            unsafe { destroy(self.data) };
        }
    }
}

/// Header shared by every reference counted object in this API.
pub struct ObjectHeader {
    // Safety invariant: `IMMORTAL` for good on an immortal object. On any
    // other, the number of references to it that are owned and not yet
    // released, for as long as there is one. From the moment the last of
    // them is released it is zero and then `DYING`, plus however many the
    // object's destroy callbacks have taken and not given back, and is never
    // one again.
    ref_count: AtomicI64,
    immutable: AtomicBool,
    /// Read far more often than written: user data is typically attached once
    /// and then looked up, so readers should not queue behind one another.
    user_data: RwLock<Vec<UserDataItem>>,
}

impl ObjectHeader {
    /// Creates a header for a live object with a reference count of one.
    pub fn new() -> Self {
        Self {
            ref_count: AtomicI64::new(1),
            immutable: AtomicBool::new(false),
            user_data: RwLock::new(Vec::new()),
        }
    }

    /// Creates a header for an immortal object.
    pub fn immortal() -> Self {
        Self {
            ref_count: AtomicI64::new(IMMORTAL),
            immutable: AtomicBool::new(true),
            user_data: RwLock::new(Vec::new()),
        }
    }

    fn is_immortal(&self) -> bool {
        self.ref_count.load(Ordering::Relaxed) == IMMORTAL
    }

    /// Runs the destroy callback of every user data item and forgets them.
    ///
    /// The callbacks are the caller's code and may come back into the API,
    /// so they run once the lock is released, as they do in [`set_user_data`].
    fn release_user_data(&self) {
        let items = {
            let mut items = self
                .user_data
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            core::mem::take(&mut *items)
        };
        drop(items);
    }
}

impl Default for ObjectHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// Implemented by every reference counted object in this API.
///
/// The functions in this module rely on what `header` and `empty` are
/// documented to return, and on what `finalize` is documented to leave
/// behind. Nothing makes an implementation keep to that but its author, and
/// every implementation is in this crate.
pub trait Object: Sized + 'static {
    /// Returns this object's header.
    ///
    /// # Safety-usable invariant
    ///
    /// Every call on one object returns the same header, which the object
    /// holds for as long as it exists.
    fn header(&self) -> &ObjectHeader;

    /// Returns the immortal empty singleton for this type.
    ///
    /// # Safety-usable invariant
    ///
    /// Every call returns the same pointer: that of an [`Empty`] holding an
    /// object whose header is [`ObjectHeader::immortal`], which makes the
    /// pointer [live](self#live-pointers) for the rest of the process.
    fn empty() -> *mut Self;

    /// Releases everything the object holds whose release runs the caller's
    /// code, before the object is dropped.
    ///
    /// The default is for an object that holds nothing of the kind.
    ///
    /// # Safety
    ///
    /// `ptr` must be [live](self#live-pointers), and the object's last
    /// reference must have been released, so that nothing owns one and the
    /// object takes no changes. Nothing else may be using the object when
    /// this is called: no reference to it may be in use, and no other thread
    /// may access it.
    ///
    /// An implementation may write the object through `ptr`. The destroy
    /// callbacks it runs are the caller's code, and may still query the
    /// object through its original pointer. Whenever one runs, the
    /// implementation must hold no reference to the object, and each field
    /// must hold a value that every reader of the object allows for.
    ///
    /// # Safety-usable invariant
    ///
    /// Once this has returned, dropping the object runs none of the caller's
    /// code: whatever would have run some has been released here, and an
    /// object that takes no changes is given nothing more to release.
    unsafe fn finalize(_ptr: *mut Self) {}
}

/// Allocates a new object with a reference count of one.
///
/// # Safety-usable invariant
///
/// The pointer returned is what `Box::into_raw` gave for a `Box<T>` holding
/// `value`. It is [live](self#live-pointers), and the caller owns the
/// reference that a header from [`ObjectHeader::new`] counts.
pub fn create<T: Object>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

/// A leaked, immortal object, held so that `hr_*_get_empty` hands back the
/// same pointer every time.
///
/// This keeps the pointer itself rather than its address, so that the
/// provenance needed to dereference it survives.
pub struct Empty<T>(
    // Safety invariant: what `Box::into_raw` gave for a `Box<T>`, which this
    // never frees.
    *mut T,
);

// SAFETY: the pointee is leaked and lives as long as the process, and every
// thread is handed the same pointer to it. Everything reachable through a
// shared reference to an immortal object is safe to use from several threads
// at once, including what is filled in on first use, which sits behind a
// `OnceLock`. An exclusive reference to one must never be formed, and
// `as_mutable` refuses immortal objects so that no setter forms one.
unsafe impl<T> Send for Empty<T> {}
// SAFETY: as for `Send`: all that sharing an `Empty` shares is that same
// pointer.
unsafe impl<T> Sync for Empty<T> {}

impl<T> Empty<T> {
    /// Leaks `value`, to be handed out for the life of the process.
    ///
    /// `value` is to carry a header from [`ObjectHeader::immortal`]. With
    /// any other, [`destroy`] would come to free what this hands out as
    /// immortal.
    pub fn new(value: T) -> Self {
        Self(Box::into_raw(Box::new(value)))
    }

    /// Returns the singleton: the same pointer every time.
    pub fn get(&self) -> *mut T {
        self.0
    }
}

/// Increments the reference count and returns the object.
///
/// Returns `NULL` for a `NULL` argument, and is a no-op for immortal objects.
/// An object releasing its last reference is counted like any other, but no
/// reference taken then keeps it alive.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers).
///
/// The caller is handed `ptr` back, and [owns](self#owning-a-reference) the
/// reference taken.
pub unsafe fn reference<T: Object>(ptr: *mut T) -> *mut T {
    // SAFETY: `as_ref` needs `ptr` to be null, or aligned and pointing to a
    // `T` that stays allocated, and unwritten outside its interior-mutable
    // parts, while the reference is used. That is no longer than this call,
    // for which this function's contract has `ptr` `NULL` or live, and a
    // live pointer is aligned and points to a `T`. A live object is written
    // only by whoever has it to itself, who answers for `obj` not being in
    // use (see "Exclusive access" in the module docs).
    let Some(obj) = (unsafe { ptr.as_ref() }) else {
        return core::ptr::null_mut();
    };
    let header = obj.header();
    if header.is_immortal() {
        return ptr;
    }
    // The caller owns the reference this counts, which keeps the invariant
    // on `ref_count`.
    header.ref_count.fetch_add(1, Ordering::Relaxed);
    ptr
}

/// Decrements the reference count, freeing the object when it reaches zero.
///
/// Ignores `NULL` and immortal objects.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers). The caller must
/// [own](self#owning-a-reference) the reference being released, and no
/// longer owns it once this is called.
pub unsafe fn destroy<T: Object>(ptr: *mut T) {
    // SAFETY: as in `reference`, `as_ref` needs `ptr` to be null, or a `T`
    // that stays allocated and unwritten while `obj` and `header` are used.
    // This function's contract has `ptr` `NULL` or live, and the reference
    // the caller owns keeps the object allocated up to the decrement below.
    // The two are used past it only when it released the last reference,
    // and then nothing frees the object but this call, after their last
    // use. Nothing has the object to itself in the meantime: before the
    // decrement that is as in `reference`, and after it the object is
    // releasing its last reference, which `accepts_changes` reports and
    // `as_mutable` refuses.
    let Some(obj) = (unsafe { ptr.as_ref() }) else {
        return;
    };
    let header = obj.header();
    if header.is_immortal() {
        return;
    }
    // The caller gives up the reference it owned, which keeps the invariant
    // on `ref_count`.
    if header.ref_count.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    // Nothing else holds a reference, so nothing else reads the count from
    // here on but the callbacks below, on this thread.
    header.ref_count.store(DYING, Ordering::Relaxed);
    // Last reference. As in HarfBuzz, the destroy callback of every attached
    // user data item runs first, while the object still holds everything it
    // owns: a callback on a font may look at the font's face. `finalize`
    // then lets go of whatever else the caller gave the object, and only
    // after that is ownership taken back and the object dropped.
    header.release_user_data();
    // SAFETY: `finalize` needs `ptr` live, the object's last reference
    // released, and nothing else using the object.
    // - `ptr` was live on entry by this function's contract, and nothing
    //   frees the object but this call, further down.
    // - The count read one, and the caller owned a reference, so by the
    //   invariant on `ref_count` that was the last. The count is parked at
    //   `DYING` now, which is what makes the object take no changes.
    // - With no reference owned, only the object's destroy callbacks may be
    //   using it, and each is done with it by the time it returns (see
    //   "Owning a reference" in the module docs). Those run so far have
    //   returned, and `obj` and `header` are not used again.
    unsafe { T::finalize(ptr) };
    // SAFETY: `Box::from_raw` needs a pointer that `Box::into_raw` gave for a
    // `Box<T>` and that has not been made into a `Box` since, and the object
    // to be the `Box`'s alone from here on.
    // - `ptr` is live, so it is what `create` or `Empty::new` got from
    //   `Box::into_raw` for a `Box<T>`.
    // - Only this line frees an object, and an object gets here once: the
    //   count read one a single time and is parked at `DYING`.
    // - As for `finalize` above, nothing else is using the object, and the
    //   callbacks `finalize` ran have returned too. Dropping the `T` runs
    //   none of the caller's code, which could still reach the object
    //   through `ptr`: that is what `finalize` is documented to leave
    //   behind.
    drop(unsafe { Box::from_raw(ptr) });
}

/// Attaches user data to an object, returning `false` if it could not be set.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers). `key` must outlive
/// the object.
///
/// `destroy_func`, if given, must be safe to call once with `data`, on
/// whichever thread releases the data and at whichever point that is: when
/// the entry is replaced or removed, or when the object's last reference is
/// released. If this returns `false` the data is not taken, and
/// `destroy_func` is not called.
pub unsafe fn set_user_data<T: Object>(
    ptr: *mut T,
    key: *const hr_user_data_key_t,
    data: *mut c_void,
    destroy_func: hr_destroy_func_t,
    replace: bool,
) -> bool {
    // SAFETY: as in `reference`: this function's contract has `ptr` `NULL`
    // or live until it returns, and `obj` is not used past that.
    let Some(obj) = (unsafe { ptr.as_ref() }) else {
        return false;
    };
    let header = obj.header();
    // Inert objects and objects releasing their final reference never take
    // user data. A destroy callback may query the detached metadata, but
    // must not attach callbacks that would later run inside Box::drop.
    if header.ref_count.load(Ordering::Relaxed) <= 0 || key.is_null() {
        return false;
    }
    // Whatever this displaces is dropped once the lock is released, because
    // dropping runs the caller's destructor, and HarfBuzz permits that to
    // come back in here. Releasing it under the lock would deadlock.
    let mut displaced = None;
    let stored = {
        let Ok(mut items) = header.user_data.write() else {
            return false;
        };
        // Neither data nor a destructor means the entry goes away, rather
        // than staying behind to answer with NULL: HarfBuzz removes the key,
        // and a key left in place could never be set again without `replace`.
        let clearing = data.is_null() && destroy_func.is_none();
        // Either item built below upholds the invariant on its `destroy`:
        // this function's contract has `destroy_func` safe to call once with
        // `data` wherever the entry comes to be dropped, and nothing but
        // that drop calls it.
        match items.iter().position(|item| item.key == key) {
            Some(_) if !replace => false,
            Some(at) if clearing => {
                displaced = Some(items.remove(at));
                true
            }
            Some(at) => {
                displaced = Some(core::mem::replace(
                    &mut items[at],
                    UserDataItem {
                        key,
                        data,
                        destroy: destroy_func,
                    },
                ));
                true
            }
            // Clearing a key that is not there has nothing to do, but is not
            // a failure.
            None if clearing && replace => true,
            None => {
                items.push(UserDataItem {
                    key,
                    data,
                    destroy: destroy_func,
                });
                true
            }
        }
    };
    drop(displaced);
    stored
}

/// Retrieves user data previously attached with [`set_user_data`].
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers).
pub unsafe fn get_user_data<T: Object>(ptr: *mut T, key: *const hr_user_data_key_t) -> *mut c_void {
    // SAFETY: as in `reference`: this function's contract has `ptr` `NULL`
    // or live until it returns, and `obj` is not used past that.
    let Some(obj) = (unsafe { ptr.as_ref() }) else {
        return core::ptr::null_mut();
    };
    let Ok(items) = obj.header().user_data.read() else {
        return core::ptr::null_mut();
    };
    items
        .iter()
        .find(|item| item.key == key)
        .map_or(core::ptr::null_mut(), |item| item.data)
}

/// Marks an object immutable. Immutable objects reject all further mutation.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers).
pub unsafe fn make_immutable<T: Object>(ptr: *mut T) {
    // SAFETY: as in `reference`: this function's contract has `ptr` `NULL`
    // or live until it returns, and `obj` is not used past that.
    if let Some(obj) = unsafe { ptr.as_ref() } {
        obj.header().immutable.store(true, Ordering::Release);
    }
}

/// Returns whether an object has been marked immutable.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers).
pub unsafe fn is_immutable<T: Object>(ptr: *const T) -> bool {
    // SAFETY: as in `reference`: this function's contract has `ptr` `NULL`
    // or live until it returns, and the reference is not used past that.
    unsafe { ptr.as_ref() }.is_some_and(|obj| obj.header().immutable.load(Ordering::Acquire))
}

/// Returns a shared reference to an object, falling back to its immortal empty
/// singleton when `ptr` is `NULL`.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers), for as long as the
/// returned reference is used.
pub unsafe fn or_empty<'a, T: Object>(ptr: *const T) -> &'a T {
    // SAFETY: as in `reference`, but for longer: this function's contract
    // has `ptr` `NULL` or live for as long as the reference returned is
    // used.
    match unsafe { ptr.as_ref() } {
        Some(obj) => obj,
        // SAFETY: making the reference needs a non-null, aligned pointer to
        // a `T` that stays allocated, and unwritten outside its
        // interior-mutable parts, for `'a`. `Object::empty` documents its
        // pointer as live for the rest of the process, which is as long as
        // any `'a`, and a live pointer is non-null and aligned and points to
        // a `T`. Nothing has an immortal object to itself to write it:
        // `as_mutable` refuses one.
        None => unsafe { &*T::empty().cast_const() },
    }
}

/// Returns whether an object takes changes: it is not `NULL`, has not been
/// marked immutable, and is not releasing its final reference.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers).
pub unsafe fn accepts_changes<T: Object>(ptr: *const T) -> bool {
    // SAFETY: as in `reference`: this function's contract has `ptr` `NULL`
    // or live until it returns, and the reference is not used past that.
    unsafe { ptr.as_ref() }.is_some_and(|obj| {
        let header = obj.header();
        !header.immutable.load(Ordering::Acquire) && header.ref_count.load(Ordering::Relaxed) > 0
    })
}

/// Returns a mutable reference to an object, or `None` when `ptr` is `NULL` or
/// the object has been marked immutable or is releasing its final reference.
///
/// Setters use this so that mutating an immutable object is silently ignored,
/// as it is in HarfBuzz.
///
/// # Safety
///
/// `ptr` must be `NULL` or [live](self#live-pointers), for as long as the
/// returned reference is used, and the caller must not alias that reference.
/// If the object takes changes, no other reference to it may be in use when
/// this is called, and until the returned reference is last used nothing
/// else may access the object: not through `ptr` or another reference, and
/// not from a callback or another thread.
pub unsafe fn as_mutable<'a, T: Object>(ptr: *mut T) -> Option<&'a mut T> {
    // Reject shared immutable objects and finalization reentry before taking
    // an exclusive reference. Destroy callbacks may still query the object.
    //
    // SAFETY: `accepts_changes` needs `ptr` to be `NULL` or live, which this
    // function's contract has it.
    if !unsafe { accepts_changes(ptr.cast_const()) } {
        return None;
    }
    // SAFETY: `as_mut` needs `ptr` to be null, or aligned and pointing to a
    // `T` that stays allocated, and that nothing else reads or writes, while
    // the reference is used.
    // - `ptr` is not null, or `accepts_changes` would have said no, so this
    //   function's contract has it live for that long. A live pointer is
    //   aligned and points to a `T`, and being the pointer this API made for
    //   the object it may be written through.
    // - The object took changes a moment ago, so this function's contract
    //   has no other reference to it in use now, and nothing else accessing
    //   it for as long as the one returned is used. That rules out whatever
    //   would freeze it, or release its last reference, since the check.
    unsafe { ptr.as_mut() }
}
