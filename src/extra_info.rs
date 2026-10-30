// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ownership of a function's `extra_info` allocation until `DuckDB` takes it.
//!
//! Every function builder can carry an `extra_info` pointer plus the destructor
//! `DuckDB` will call for it. `DuckDB` only takes ownership at
//! `duckdb_*_set_extra_info`, i.e. inside `register`. Between the call that
//! supplies the pointer and the call that registers the function, the
//! allocation belongs to the builder — and a builder that is dropped instead of
//! registered used to drop the pointer on the floor.
//!
//! That is easy to dismiss as a path nobody takes, until a typed constructor
//! allocates on the user's behalf:
//! [`ScalarFunctionBuilder::map1`][crate::scalar::ScalarFunctionBuilder::map1]
//! boxes a closure, and
//! [`TableFunctionBuilder::with_state`][crate::table::TableFunctionBuilder::with_state]
//! boxes two, so `build()` without `register()` leaks user data through an API
//! that never mentions a pointer. Miri's leak checker found exactly that in this
//! crate's own tests.
//!
//! [`ExtraInfo`] closes it: the destructor runs on drop unless
//! [`mark_transferred`][ExtraInfo::mark_transferred] says `DuckDB` has taken
//! over.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use libduckdb_sys::duckdb_delete_callback_t;

/// An `extra_info` pointer and its destructor, owned until `DuckDB` takes them.
pub struct ExtraInfo {
    data: *mut c_void,
    destroy: duckdb_delete_callback_t,
    /// Set through a shared reference, so a set builder can mark an overload
    /// transferred while iterating its overloads by `&`.
    ///
    /// An `AtomicBool` rather than a `Cell<bool>`: `Cell` contains an
    /// `UnsafeCell`, which is `!RefUnwindSafe`, and that propagates to every
    /// builder that owns an `ExtraInfo` — silently removing an auto trait from
    /// seven public builders that hold one directly and the two set builders
    /// that hold their overloads. `AtomicBool` gives the same shared-reference
    /// mutation with none of that (the test `builders_stay_ref_unwind_safe`
    /// checks it). `ExtraInfo` stays `!Sync` because of the raw pointer, and so
    /// does every builder that holds one.
    transferred: AtomicBool,
}

impl ExtraInfo {
    /// Takes ownership of `data`, to be freed by `destroy`.
    ///
    /// # Safety
    ///
    /// - `data` must be a pointer `destroy` can free exactly once, and no one
    ///   else may free it.
    /// - `destroy` must not panic. It is an `extern "C" fn`, so an unwind out of
    ///   it aborts the process at its own boundary — before any `catch_unwind`
    ///   on this side could see it. quack-rs's own destructors catch internally
    ///   via [`catch_ffi_panic`][crate::callback::catch_ffi_panic]; a
    ///   hand-written one must too.
    pub unsafe fn new(data: *mut c_void, destroy: duckdb_delete_callback_t) -> Self {
        Self {
            data,
            destroy,
            transferred: AtomicBool::new(false),
        }
    }

    /// The raw pointer, for handing to `duckdb_*_set_extra_info`.
    pub const fn data(&self) -> *mut c_void {
        self.data
    }

    /// The destructor, for handing to `duckdb_*_set_extra_info`.
    pub const fn destroy(&self) -> duckdb_delete_callback_t {
        self.destroy
    }

    /// Records that `DuckDB` now owns the allocation, so `Drop` leaves it alone.
    ///
    /// Call this immediately after `duckdb_*_set_extra_info` returns.
    pub fn mark_transferred(&self) {
        self.transferred.store(true, Ordering::Relaxed);
    }
}

impl Drop for ExtraInfo {
    fn drop(&mut self) {
        if self.transferred.load(Ordering::Relaxed) || self.data.is_null() {
            return;
        }
        let Some(destroy) = self.destroy else {
            return;
        };
        // A panic inside `destroy` cannot be contained from here: `destroy` is
        // an `extern "C" fn`, so Rust aborts at *its* boundary before any
        // `catch_unwind` on this side is reached. Every destructor quack-rs
        // generates catches internally (see `callback::catch_ffi_panic`); a
        // user-supplied one must do the same, which is part of `extra_info`'s
        // safety contract.
        //
        // SAFETY: `new`'s contract says `destroy` can free `data` exactly once,
        // and `transferred` proves nobody else has.
        unsafe { destroy(self.data) };
    }
}

impl core::fmt::Debug for ExtraInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtraInfo")
            .field("data", &self.data)
            .field("destroy", &self.destroy.map(|_| "<fn>"))
            .field("transferred", &self.transferred.load(Ordering::Relaxed))
            .finish()
    }
}

// ─── Typed payloads ──────────────────────────────────────────────────────────
//
// `DuckDB` hands an extra-info, bind-data or global-state pointer back as a
// bare `void *`, so every typed getter over one has to trust that the pointer
// holds the type it asks for. A wrong guess (two functions sharing a getter, a
// refactor that changes one side) reinterpreted memory. The typed setters
// (`extra_info_typed`, `set_bind_data_typed`, `set_global_state_typed`) store
// a `Tagged<T>` instead, whose first field is `T`'s `TypeId`, and the typed
// getters (`extra_info_ref`, `bind_data_ref`, `global_state_ref`) compare it
// before they read: a `T` asked for as a `U` is `None`.

/// A `T` behind its `TypeId`. `repr(C)` puts `type_id` at offset 0 whatever
/// `T` is, so the tag of any `Tagged<_>` can be read without knowing its `T`.
#[repr(C)]
struct Tagged<T> {
    type_id: core::any::TypeId,
    value: T,
}

/// Boxes `value` with its type tag and returns the pointer together with the
/// destructor that frees it: a pair that satisfies every raw setter's contract
/// (freed exactly once, by a destructor that does not unwind).
pub fn into_tagged<T: Send + Sync + 'static>(value: T) -> (*mut c_void, TaggedDestroy) {
    let tagged = Box::new(Tagged {
        type_id: core::any::TypeId::of::<T>(),
        value,
    });
    (
        Box::into_raw(tagged).cast::<c_void>(),
        drop_tagged::<T> as TaggedDestroy,
    )
}

/// The destructor [`into_tagged`] returns.
pub type TaggedDestroy = unsafe extern "C" fn(*mut c_void);

/// Frees a `Tagged<T>` from [`into_tagged`], containing a panic in `T::drop`
/// (an unwind out of an `extern "C" fn` aborts the process).
///
/// # Safety
///
/// `ptr` is null or came from `into_tagged::<T>` and is freed only here.
unsafe extern "C" fn drop_tagged<T: 'static>(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` came from `Box::into_raw(Box<Tagged<T>>)` in
    // `into_tagged::<T>` (this function's contract), and DuckDB or `ExtraInfo`
    // calls the destructor once.
    drop(crate::callback::catch_ffi_panic(|| unsafe {
        drop(Box::from_raw(ptr.cast::<Tagged<T>>()));
    }));
}

/// Borrows the `T` behind a pointer from [`into_tagged`], or `None` when the
/// pointer is null or holds a different type.
///
/// # Safety
///
/// `ptr` is null, or a live pointer that [`into_tagged`] returned for *some*
/// type — never one that a raw setter stored — and it stays live for `'a`.
pub unsafe fn tagged_ref<'a, T: 'static>(ptr: *mut c_void) -> Option<&'a T> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: every `Tagged<_>` is `repr(C)` with its `TypeId` first, and `ptr`
    // points at one (this function's contract), so its first bytes are a
    // valid, aligned `TypeId` whatever its `T` is.
    let stored = unsafe { ptr.cast::<core::any::TypeId>().read() };
    if stored != core::any::TypeId::of::<T>() {
        return None;
    }
    // SAFETY: the tag says the allocation is a `Tagged<T>`; it lives for `'a`.
    Some(unsafe { &(*ptr.cast::<Tagged<T>>()).value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Every public builder that holds an `ExtraInfo`, directly or through its
    /// overloads or inner builder, is `RefUnwindSafe`.
    #[test]
    fn builders_stay_ref_unwind_safe() {
        const fn ref_unwind_safe<T: std::panic::RefUnwindSafe>() {}
        ref_unwind_safe::<crate::aggregate::AggregateFunctionBuilder>();
        ref_unwind_safe::<crate::aggregate::AggregateOverloadBuilder>();
        ref_unwind_safe::<crate::aggregate::AggregateFunctionSetBuilder>();
        ref_unwind_safe::<crate::cast::CastFunctionBuilder>();
        ref_unwind_safe::<crate::scalar::ScalarFunctionBuilder>();
        ref_unwind_safe::<crate::scalar::ScalarOverloadBuilder>();
        ref_unwind_safe::<crate::scalar::ScalarFunctionSetBuilder>();
        ref_unwind_safe::<crate::table::TableFunctionBuilder>();
        #[cfg(feature = "duckdb-1-5")]
        ref_unwind_safe::<crate::copy_function::CopyFunctionBuilder>();
    }

    /// The payload each test allocates: a reference to *that test's* counter.
    ///
    /// A single shared `static` counter made these tests race. `cargo test` runs
    /// them in parallel, so one test's `store(0)` could land between another's
    /// drop and its assertion — which is exactly how
    /// `dropping_an_untransferred_extra_info_frees_it` failed in CI while
    /// passing everywhere else. Carrying the counter in the allocation gives
    /// each test its own and keeps them parallel.
    struct Tracked {
        freed: &'static AtomicUsize,
    }

    unsafe extern "C" fn count_free(ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: every test allocates through `boxed`.
        let tracked = unsafe { Box::from_raw(ptr.cast::<Tracked>()) };
        tracked.freed.fetch_add(1, Ordering::SeqCst);
    }

    fn boxed(freed: &'static AtomicUsize) -> *mut c_void {
        Box::into_raw(Box::new(Tracked { freed })).cast::<c_void>()
    }

    #[test]
    fn dropping_an_untransferred_extra_info_frees_it() {
        static FREED: AtomicUsize = AtomicUsize::new(0);
        // SAFETY: `count_free` frees exactly what `boxed` allocates.
        let info = unsafe { ExtraInfo::new(boxed(&FREED), Some(count_free)) };
        drop(info);
        assert_eq!(FREED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_transferred_extra_info_leaves_the_allocation_to_duckdb() {
        static FREED: AtomicUsize = AtomicUsize::new(0);
        let raw = boxed(&FREED);
        // SAFETY: as above.
        let info = unsafe { ExtraInfo::new(raw, Some(count_free)) };
        info.mark_transferred();
        drop(info);
        assert_eq!(
            FREED.load(Ordering::SeqCst),
            0,
            "DuckDB owns it now; freeing here would be a double free"
        );
        // Free it by hand so the test itself does not leak.
        // SAFETY: nothing else freed it.
        unsafe { count_free(raw) };
    }

    #[test]
    fn a_null_pointer_or_absent_destructor_is_a_no_op() {
        static FREED: AtomicUsize = AtomicUsize::new(0);
        // SAFETY: nothing is freed on either path.
        unsafe {
            drop(ExtraInfo::new(std::ptr::null_mut(), Some(count_free)));
            drop(ExtraInfo::new(std::ptr::null_mut(), None));
        }
        assert_eq!(FREED.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, PartialEq)]
    struct Config(u32);

    #[test]
    fn a_tagged_value_reads_back_as_its_own_type_only() {
        let (ptr, destroy) = into_tagged(Config(7));
        // SAFETY: `ptr` came from `into_tagged` and is live until `destroy`.
        unsafe {
            assert_eq!(tagged_ref::<Config>(ptr), Some(&Config(7)));
            assert_eq!(tagged_ref::<u32>(ptr), None, "same layout, different type");
            assert_eq!(tagged_ref::<String>(ptr), None);
            destroy(ptr);
        }
    }

    #[test]
    fn a_null_pointer_reads_as_none_and_frees_nothing() {
        // SAFETY: null is explicitly allowed by both.
        unsafe {
            assert_eq!(tagged_ref::<Config>(std::ptr::null_mut()), None);
            drop_tagged::<Config>(std::ptr::null_mut());
        }
    }

    #[test]
    fn the_destructor_drops_the_value_exactly_once() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        let (ptr, destroy) = into_tagged(Counted);
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
        // SAFETY: freed once, here.
        unsafe { destroy(ptr) };
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panicking_drop_does_not_escape_the_destructor() {
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("typed payload destructor deliberately exploded");
            }
        }
        let (ptr, destroy) = into_tagged(Bomb);
        // Reaching the next line is the assertion: an unwind out of the
        // `extern "C"` destructor would abort the test binary.
        // SAFETY: freed once, here.
        unsafe { destroy(ptr) };
    }

    /// Every builder's `extra_info_typed` keeps the value until the builder
    /// itself is dropped (unregistered), then frees it once. A body replaced
    /// by `Default::default()` (viable for `AggregateOverloadBuilder`) would
    /// drop it at the call instead.
    #[test]
    fn each_builder_holds_its_typed_extra_info_until_it_is_dropped() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        use crate::types::TypeId;
        let aggregate =
            crate::aggregate::AggregateFunctionBuilder::new("f").extra_info_typed(Counted);
        let overload = crate::aggregate::AggregateOverloadBuilder::new().extra_info_typed(Counted);
        let cast = crate::cast::CastFunctionBuilder::new(TypeId::Blob, TypeId::BigInt)
            .extra_info_typed(Counted);
        #[cfg(feature = "duckdb-1-5")]
        let copy = crate::copy_function::CopyFunctionBuilder::try_new("fmt")
            .expect("valid name")
            .extra_info_typed(Counted);
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            0,
            "each builder still holds its value"
        );
        drop(aggregate);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        drop(overload);
        assert_eq!(DROPS.load(Ordering::SeqCst), 2);
        drop(cast);
        assert_eq!(DROPS.load(Ordering::SeqCst), 3);
        #[cfg(feature = "duckdb-1-5")]
        {
            drop(copy);
            assert_eq!(DROPS.load(Ordering::SeqCst), 4);
        }
    }

    #[test]
    fn an_unregistered_typed_extra_info_is_freed_by_extra_info_drop() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        let (ptr, destroy) = into_tagged(Counted);
        // SAFETY: the pair from `into_tagged` meets `ExtraInfo::new`'s contract.
        drop(unsafe { ExtraInfo::new(ptr, Some(destroy)) });
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }
}
