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

use std::cell::Cell;
use std::os::raw::c_void;

use libduckdb_sys::duckdb_delete_callback_t;

/// An `extra_info` pointer and its destructor, owned until `DuckDB` takes them.
pub struct ExtraInfo {
    data: *mut c_void,
    destroy: duckdb_delete_callback_t,
    /// A `Cell` so a set builder can mark an overload transferred while
    /// iterating its overloads by shared reference.
    transferred: Cell<bool>,
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
            transferred: Cell::new(false),
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
        self.transferred.set(true);
    }
}

impl Drop for ExtraInfo {
    fn drop(&mut self) {
        if self.transferred.get() || self.data.is_null() {
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
            .field("transferred", &self.transferred.get())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FREED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn count_free(ptr: *mut c_void) {
        FREED.fetch_add(1, Ordering::SeqCst);
        if !ptr.is_null() {
            // SAFETY: every test below allocates with `Box::into_raw(Box::new(7u32))`.
            drop(unsafe { Box::from_raw(ptr.cast::<u32>()) });
        }
    }

    fn boxed() -> *mut c_void {
        Box::into_raw(Box::new(7u32)).cast::<c_void>()
    }

    #[test]
    fn dropping_an_untransferred_extra_info_frees_it() {
        FREED.store(0, Ordering::SeqCst);
        // SAFETY: `count_free` frees exactly what `boxed` allocates.
        let info = unsafe { ExtraInfo::new(boxed(), Some(count_free)) };
        drop(info);
        assert_eq!(FREED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_transferred_extra_info_leaves_the_allocation_to_duckdb() {
        FREED.store(0, Ordering::SeqCst);
        let raw = boxed();
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
        FREED.store(0, Ordering::SeqCst);
        // SAFETY: nothing is freed on either path.
        unsafe {
            drop(ExtraInfo::new(std::ptr::null_mut(), Some(count_free)));
            drop(ExtraInfo::new(std::ptr::null_mut(), None));
        }
        assert_eq!(FREED.load(Ordering::SeqCst), 0);
    }
}
