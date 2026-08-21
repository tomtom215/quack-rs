// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Typed bind data and per-thread local state for scalar functions
//! (`DuckDB` 1.5.0+).
//!
//! [`ScalarBindInfo::set_bind_data`][crate::scalar::ScalarBindInfo::set_bind_data]
//! and
//! [`ScalarInitInfo::set_state`][crate::scalar::ScalarInitInfo::set_state] take a
//! raw pointer and a `duckdb_delete_callback_t`, which means every extension
//! that wants bind data writes its own:
//!
//! ```rust,ignore
//! unsafe extern "C" fn drop_bind(ptr: *mut c_void) {
//!     if !ptr.is_null() {
//!         drop(unsafe { Box::from_raw(ptr.cast::<BindData>()) });  // aborts if
//!     }                                                           // Drop panics
//! }
//! ```
//!
//! That is the hazard [`FfiState`][crate::aggregate::FfiState] and
//! [`FfiBindData`][crate::table::FfiBindData] exist to remove for aggregates and
//! table functions: an unwind out of an `extern "C" fn` is a process abort on
//! Rust 1.81+, and `BindData`'s `Drop` is arbitrary user code. [`ScalarBindData`]
//! and [`ScalarLocalState`] are the same thing for scalar functions — the
//! destructor is generated, panic-safe, and impossible to forget.
//!
//! # Lifecycle
//!
//! ```text
//! bind        → ScalarBindData::<T>::set(&bind_info, value)
//! init        → ScalarBindData::<T>::get_from_init(&init_info)   (read)
//!               ScalarLocalState::<S>::set(&init_info, state)    (per thread)
//! execute     → ScalarBindData::<T>::get(&function_info)         (read)
//!               ScalarLocalState::<S>::get_mut(&function_info)   (read/write)
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::scalar::state::ScalarBindData;
//! use quack_rs::scalar::{ScalarBindInfo, ScalarFunctionInfo};
//!
//! struct Factor(i64);
//!
//! # #[allow(unused)]
//! unsafe extern "C" fn my_bind(info: libduckdb_sys::duckdb_bind_info) {
//!     // SAFETY: DuckDB passes a valid bind info.
//!     let bind = unsafe { ScalarBindInfo::new(info) };
//!     ScalarBindData::set(&bind, Factor(10));
//! }
//!
//! quack_rs::scalar_callback!(my_func, |info, input, output| {
//!     // SAFETY: DuckDB passes a valid function info.
//!     let fninfo = unsafe { ScalarFunctionInfo::new(info) };
//!     // SAFETY: `my_bind` stored a `Factor`, and nothing else did.
//!     let factor = unsafe { ScalarBindData::<Factor>::get(&fninfo) };
//!     let _ = factor.map(|f| f.0);
//! });
//! ```

use std::marker::PhantomData;
use std::os::raw::c_void;

use crate::scalar::info::{ScalarBindInfo, ScalarFunctionInfo, ScalarInitInfo};

/// Frees a `Box<T>` behind a `duckdb_delete_callback_t`, containing any panic.
///
/// # Safety
///
/// `ptr` must have come from `Box::into_raw` on a `Box<T>`, and must not be
/// freed by anyone else.
unsafe extern "C" fn drop_boxed<T>(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` came from `Box::into_raw(Box::<T>::new(..))`. `T::drop` is
    // arbitrary user code and this is an `extern "C"` boundary with no error
    // channel, so the unwind is contained here rather than aborting.
    drop(crate::callback::catch_ffi_panic(|| unsafe {
        drop(Box::from_raw(ptr.cast::<T>()));
    }));
}

/// Type-safe bind data for a `DuckDB` scalar function.
///
/// Set once in the bind callback; read in init and in every execution. `DuckDB`
/// owns the allocation and frees it through a generated, panic-safe destructor
/// when the bound function is discarded.
pub struct ScalarBindData<T: 'static> {
    _marker: PhantomData<T>,
}

impl<T: 'static> ScalarBindData<T> {
    /// Stores `data` as this function's bind data.
    ///
    /// Call at most once per bind invocation. `DuckDB` replaces any previous
    /// value, dropping it through the destructor registered with it.
    // The whole body is one `duckdb_scalar_function_set_bind_data` call; with no
    // live engine there is no way to observe whether it happened, so the `--lib`
    // mutation run cannot kill `with ()`. Covered end to end by the typed
    // scalar-bind tests.
    #[mutants::skip]
    pub fn set(info: &ScalarBindInfo, data: T) {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: `raw` is a fresh `Box<T>` and `drop_boxed::<T>` is the
        // matching destructor; DuckDB owns it from here.
        unsafe { info.set_bind_data(raw, Some(drop_boxed::<T>)) };
    }

    /// Borrows the bind data during execution.
    ///
    /// Returns `None` if the bind callback never stored anything.
    ///
    /// # Safety
    ///
    /// The bind callback must have stored a `T` via [`set`][Self::set] — and
    /// nothing else. Reading a different type reinterprets memory.
    #[must_use]
    pub unsafe fn get<'a>(info: &ScalarFunctionInfo) -> Option<&'a T> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_bind_data() };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` was produced by `set`, and DuckDB keeps it alive for
        // the whole execution.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Borrows the bind data during the init callback.
    ///
    /// # Safety
    ///
    /// See [`get`][Self::get].
    #[must_use]
    pub unsafe fn get_from_init<'a>(info: &ScalarInitInfo) -> Option<&'a T> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_bind_data() };
        if raw.is_null() {
            return None;
        }
        // SAFETY: as in `get`.
        Some(unsafe { &*raw.cast::<T>() })
    }
}

impl<T: 'static> core::fmt::Debug for ScalarBindData<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ScalarBindData<")?;
        f.write_str(core::any::type_name::<T>())?;
        f.write_str(">")
    }
}

/// Type-safe per-thread local state for a `DuckDB` scalar function.
///
/// `DuckDB` calls the init callback once per execution thread, so each thread
/// gets its own `T` and [`get_mut`][Self::get_mut] hands out `&mut T` without
/// synchronisation. That is why `T` need only be `Send`, not `Sync` — and why a
/// counter kept here is per-thread, not global.
pub struct ScalarLocalState<T: 'static> {
    _marker: PhantomData<T>,
}

impl<T: 'static> ScalarLocalState<T> {
    /// Stores `state` as this thread's local state.
    ///
    /// Call at most once per init invocation.
    // As `ScalarBindData::set`: one FFI call, no observable effect without a
    // live engine.
    #[mutants::skip]
    pub fn set(info: &ScalarInitInfo, state: T) {
        let raw = Box::into_raw(Box::new(state)).cast::<c_void>();
        // SAFETY: `raw` is a fresh `Box<T>` and `drop_boxed::<T>` is the
        // matching destructor; DuckDB owns it from here.
        unsafe { info.set_state(raw, Some(drop_boxed::<T>)) };
    }

    /// Mutably borrows this thread's local state during execution.
    ///
    /// Returns `None` if the init callback never stored anything.
    ///
    /// # Safety
    ///
    /// - The init callback must have stored a `T` via [`set`][Self::set].
    /// - Only one borrow may be live at a time within a callback invocation.
    #[must_use]
    pub unsafe fn get_mut<'a>(info: &ScalarFunctionInfo) -> Option<&'a mut T> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_state() };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` was produced by `set` on this thread, and DuckDB keeps
        // it alive for the whole execution. The caller promises exclusivity.
        Some(unsafe { &mut *raw.cast::<T>() })
    }
}

impl<T: 'static> core::fmt::Debug for ScalarLocalState<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ScalarLocalState<")?;
        f.write_str(core::any::type_name::<T>())?;
        f.write_str(">")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Tracked;
    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn the_generated_destructor_frees_exactly_once() {
        DROPS.store(0, Ordering::SeqCst);
        let raw = Box::into_raw(Box::new(Tracked)).cast::<c_void>();
        // SAFETY: `raw` came from `Box::into_raw(Box::new(Tracked))`.
        unsafe { drop_boxed::<Tracked>(raw) };
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_generated_destructor_tolerates_null() {
        // SAFETY: the null case is handled explicitly.
        unsafe { drop_boxed::<Tracked>(std::ptr::null_mut()) };
    }

    struct DropBomb;
    impl Drop for DropBomb {
        fn drop(&mut self) {
            panic!("bind data destructor deliberately exploded");
        }
    }

    #[test]
    fn a_panicking_drop_does_not_escape_the_destructor() {
        let raw = Box::into_raw(Box::new(DropBomb)).cast::<c_void>();
        // Reaching the line after this is the assertion: without the
        // `catch_ffi_panic` inside `drop_boxed`, the unwind would hit the
        // `extern "C"` boundary and abort the test binary.
        // SAFETY: `raw` came from `Box::into_raw(Box::new(DropBomb))`.
        unsafe { drop_boxed::<DropBomb>(raw) };
    }

    #[test]
    fn debug_names_the_payload_type() {
        let s = format!(
            "{:?}",
            ScalarBindData::<u32> {
                _marker: PhantomData
            }
        );
        assert!(s.contains("u32"), "{s}");
        let s = format!(
            "{:?}",
            ScalarLocalState::<u64> {
                _marker: PhantomData
            }
        );
        assert!(s.contains("u64"), "{s}");
    }
}
