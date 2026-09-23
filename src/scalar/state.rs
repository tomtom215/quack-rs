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
//! #[derive(Clone)]
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
//!
//! # Thread safety
//!
//! `DuckDB` hands the *same* bind data to every thread executing the query
//! (`CAPIScalarFunction` in `src/main/capi/scalar_function-c.cpp` reads it
//! through the shared bound expression), so bind data must be `Send + Sync`.
//! Local state is per thread but may be freed on a different thread from the
//! one that created it, so it must be `Send`. Both bounds are enforced:
//!
//! ```rust,compile_fail
//! # use quack_rs::scalar::{ScalarBindData, ScalarBindInfo};
//! fn bind(info: &ScalarBindInfo) {
//!     // `Rc` is neither `Send` nor `Sync`: rejected.
//!     ScalarBindData::set(info, std::rc::Rc::new(1_i64));
//! }
//! ```
//!
//! ```rust,compile_fail
//! # use quack_rs::scalar::{ScalarInitInfo, ScalarLocalState};
//! fn init(info: &ScalarInitInfo) {
//!     // `Rc` is not `Send`: rejected.
//!     ScalarLocalState::set(info, std::rc::Rc::new(1_i64));
//! }
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

/// Clones a `Box<T>` behind a `duckdb_copy_callback_t`, containing any panic.
///
/// Returns null if `T::clone` panics; the copy then has no bind data, which
/// [`ScalarBindData::get`] reports as `None`.
///
/// # Safety
///
/// `ptr` must be null or point to a live `T` produced by
/// [`ScalarBindData::set`].
unsafe extern "C" fn clone_boxed<T: Clone>(ptr: *mut c_void) -> *mut c_void {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // `T::clone` is arbitrary user code and this is an `extern "C"` boundary
    // with no error channel, so the unwind is contained here.
    crate::callback::catch_ffi_panic(|| {
        // SAFETY: `ptr` points to a live `T` owned by the bind data being
        // copied; shared access only.
        let src = unsafe { &*ptr.cast::<T>() };
        Box::into_raw(Box::new(src.clone())).cast::<c_void>()
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Type-safe bind data for a `DuckDB` scalar function.
///
/// Set once in the bind callback; read in init and in every execution. `DuckDB`
/// owns the allocation and frees it through a generated, panic-safe destructor
/// when the bound function is discarded.
///
/// # Why `T: Clone + Send + Sync`
///
/// - **`Send + Sync`**: every thread executing the query reads the same value
///   concurrently (see the [module docs][self]).
/// - **`Clone`** (on [`set`][Self::set]): `DuckDB` copies the bound expression
///   whenever the optimizer duplicates it — filter pushdown through a
///   projection does — and a copy made without a copy callback has **no** bind
///   data at all. `set` registers a generated copy callback that clones `T`.
///
/// For data that is expensive or impossible to clone, store an
/// [`Arc<T>`][std::sync::Arc]: cloning it is a reference-count bump, and every
/// copy then shares one value.
pub struct ScalarBindData<T: Send + Sync + 'static> {
    _marker: PhantomData<T>,
}

impl<T: Send + Sync + 'static> ScalarBindData<T> {
    /// Stores `data` as this function's bind data, and registers the
    /// destructor and copy callback that go with it.
    ///
    /// Call at most once per bind invocation: `DuckDB` overwrites the stored
    /// pointer on a second call **without** freeing the first value, so the
    /// first value is leaked (never dropped). quack-rs cannot free it for you —
    /// the C API offers no way to read back what a bind callback stored.
    // The whole body is two FFI calls; with no live engine there is no way to
    // observe whether they happened, so the `--lib` mutation run cannot kill
    // `with ()`. Covered end to end by the typed scalar-bind tests.
    #[mutants::skip]
    pub fn set(info: &ScalarBindInfo, data: T)
    where
        T: Clone,
    {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: `raw` is a fresh `Box<T>`; `drop_boxed::<T>` is the matching
        // destructor and `clone_boxed::<T>` produces allocations the same
        // destructor frees. DuckDB owns it from here.
        unsafe {
            info.set_bind_data(raw, Some(drop_boxed::<T>));
            info.set_bind_data_copy(Some(clone_boxed::<T>));
        }
    }

    /// Borrows the bind data during execution.
    ///
    /// Returns `None` if the bind callback never stored anything, or if
    /// `DuckDB` copied the bound expression and `T::clone` panicked while
    /// copying it.
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

impl<T: Send + Sync + 'static> core::fmt::Debug for ScalarBindData<T> {
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
/// counter kept here is per-thread, not global. It must be `Send` because the
/// thread that frees it need not be the one that created it.
pub struct ScalarLocalState<T: Send + 'static> {
    _marker: PhantomData<T>,
}

impl<T: Send + 'static> ScalarLocalState<T> {
    /// Stores `state` as this thread's local state.
    ///
    /// Call at most once per init invocation: `DuckDB` overwrites the stored
    /// pointer on a second call **without** freeing the first value, so the
    /// first value is leaked (never dropped). The C API offers no way to read
    /// the state back from an init callback, so quack-rs cannot free it.
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

impl<T: Send + 'static> core::fmt::Debug for ScalarLocalState<T> {
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

    #[derive(Debug, PartialEq)]
    struct Cloned(Vec<u8>);
    impl Clone for Cloned {
        fn clone(&self) -> Self {
            assert!(self.0 != b"bomb", "bind data clone deliberately exploded");
            Self(self.0.clone())
        }
    }

    #[test]
    fn the_generated_copy_callback_produces_an_independent_box() {
        let raw = Box::into_raw(Box::new(Cloned(vec![1, 2, 3]))).cast::<c_void>();
        // SAFETY: `raw` came from `Box::into_raw(Box::new(Cloned(..)))`.
        let copy = unsafe { clone_boxed::<Cloned>(raw) };
        assert!(!copy.is_null());
        assert_ne!(copy, raw, "a copy must be a separate allocation");
        // SAFETY: both are live `Box<Cloned>` allocations.
        unsafe {
            assert_eq!(*copy.cast::<Cloned>(), Cloned(vec![1, 2, 3]));
            drop_boxed::<Cloned>(raw);
            // The copy survives the original being freed.
            assert_eq!(*copy.cast::<Cloned>(), Cloned(vec![1, 2, 3]));
            drop_boxed::<Cloned>(copy);
        }
    }

    #[test]
    fn the_generated_copy_callback_tolerates_null() {
        // SAFETY: the null case is handled explicitly.
        assert!(unsafe { clone_boxed::<Cloned>(std::ptr::null_mut()) }.is_null());
    }

    #[test]
    fn a_panicking_clone_yields_null_instead_of_unwinding() {
        let raw = Box::into_raw(Box::new(Cloned(b"bomb".to_vec()))).cast::<c_void>();
        // SAFETY: `raw` came from `Box::into_raw(Box::new(Cloned(..)))`.
        let copy = unsafe { clone_boxed::<Cloned>(raw) };
        assert!(copy.is_null(), "a failed clone is reported as no bind data");
        // SAFETY: `raw` is still live and owned here.
        unsafe { drop_boxed::<Cloned>(raw) };
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
