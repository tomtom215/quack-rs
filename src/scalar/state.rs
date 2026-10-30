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
//! unsafe extern "C" fn my_bind(info: quack_rs::scalar::RawScalarBindInfo) {
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

/// What a [`ScalarBindData`] allocation holds.
enum Slot<T> {
    /// The value the bind callback stored, or a clone of it.
    Value(T),
    /// `T::clone` panicked while `DuckDB` copied the bound expression: the
    /// panic message, reported when the copy's bind data is read.
    CopyFailed(String),
}

/// Clones a `Box<Slot<T>>` behind a `duckdb_copy_callback_t`, containing any
/// panic.
///
/// If `T::clone` panics, the copy holds the panic message instead of a value
/// ([`Slot::CopyFailed`]), which [`ScalarBindData::get`] reports as the
/// call's error. `DuckDB`'s copy callback has no error channel, and a null
/// copy is read as "no bind data", a silent wrong answer.
///
/// # Safety
///
/// `ptr` must be null or point to a live `Slot<T>` produced by
/// [`ScalarBindData::set`] or by this function.
unsafe extern "C" fn clone_slot<T: Clone>(ptr: *mut c_void) -> *mut c_void {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    // `T::clone` is arbitrary user code and this is an `extern "C"` boundary
    // with no error channel, so the unwind is contained here.
    let copy = crate::callback::catch_ffi_panic(|| {
        // SAFETY: `ptr` points to a live `Slot<T>` owned by the bind data
        // being copied; shared access only.
        match unsafe { &*ptr.cast::<Slot<T>>() } {
            Slot::Value(value) => Slot::Value(value.clone()),
            Slot::CopyFailed(message) => Slot::CopyFailed(message.clone()),
        }
    })
    .unwrap_or_else(Slot::CopyFailed);
    Box::into_raw(Box::new(copy)).cast::<c_void>()
}

/// The bind data behind `raw`: the value, or `None` after reporting a failed
/// copy through `report`.
///
/// # Safety
///
/// `raw` must be null or a live `Slot<T>` that outlives `'a`.
unsafe fn read_slot<'a, T>(raw: *mut c_void, report: impl FnOnce(&str)) -> Option<&'a T> {
    // SAFETY: forwarded from this function's own contract.
    match unsafe { raw.cast::<Slot<T>>().as_ref() }? {
        Slot::Value(value) => Some(value),
        Slot::CopyFailed(message) => {
            report(&copy_failure(message));
            None
        }
    }
}

/// The error a failed bind-data copy is reported as.
fn copy_failure(message: &str) -> String {
    format!(
        "quack-rs: DuckDB copied this function call (the optimizer does, e.g. to push a filter \
         down) and the bind data's Clone panicked, so the copy has no bind data: {message}"
    )
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
///
/// # Bind data must be a function of the call's arguments
///
/// `DuckDB` decides whether two calls are the same expression without looking
/// at their bind data: `CScalarFunctionBindData::Equals`
/// (`src/main/capi/scalar_function-c.cpp`, `DuckDB` 1.5.5) compares only the
/// function's `extra_info` and callback pointer. Two calls with the same
/// arguments are therefore merged by common-subexpression elimination and
/// `GROUP BY` matching even if their bind callbacks stored different values,
/// and one call's bind data answers for both. Checked against 1.5.5: with a
/// bind callback that stores a counter, `SELECT f(i), f(i)` runs the bind
/// callback twice and returns the first call's value in both columns.
///
/// This is harmless when the bind data depends only on what `DuckDB` does
/// compare — the arguments (constant arguments included: `f(i, 2)` and
/// `f(i, 3)` are different expressions and keep their own bind data), the
/// argument types, and `extra_info`. It is wrong when the bind callback reads
/// something else that can differ between two identical calls in one query:
/// a counter, a clock, a random source. Mark such a function
/// [`volatile`][crate::scalar::ScalarFunctionBuilder::volatile]: volatile
/// calls are never merged (checked: the counter example then returns two
/// different values).
pub struct ScalarBindData<T: Send + Sync + 'static> {
    _marker: PhantomData<T>,
}

impl<T: Send + Sync + 'static> ScalarBindData<T> {
    /// Stores `data` as this function's bind data, and registers the
    /// destructor and copy callback that go with it.
    ///
    /// A second call through the same `info` replaces the first value, which
    /// is dropped then: `DuckDB` overwrites the stored pointer without running
    /// its destructor, so `info` remembers what this stored and frees it. The
    /// C API offers no way to read back what a bind callback stored, so a
    /// value stored through another [`ScalarBindInfo`] wrapper of the same
    /// bind call, or replaced by the raw
    /// [`set_bind_data`][ScalarBindInfo::set_bind_data], is leaked (never
    /// dropped).
    // The body is FFI calls; with no live engine there is no way to observe
    // whether they happened, so the `--lib` mutation run cannot kill `with
    // ()`. Covered end to end by the typed scalar-bind tests.
    #[cfg_attr(test, mutants::skip)]
    pub fn set(info: &ScalarBindInfo, data: T)
    where
        T: Clone,
    {
        let raw = Box::into_raw(Box::new(Slot::Value(data))).cast::<c_void>();
        let replaced = info.take_typed_bind_data();
        // SAFETY: `raw` is a fresh `Box<Slot<T>>`; `drop_boxed::<Slot<T>>` is
        // the matching destructor and `clone_slot::<T>` produces allocations
        // the same destructor frees. DuckDB owns it from here.
        unsafe {
            info.set_bind_data(raw, Some(drop_boxed::<Slot<T>>));
            info.set_bind_data_copy(Some(clone_slot::<T>));
        }
        info.replace_typed_bind_data((raw, drop_boxed::<Slot<T>>));
        if let Some((previous, destroy)) = replaced {
            // SAFETY: `previous` is the value an earlier `set` through `info`
            // stored with `destroy` as its destructor; the call above
            // overwrote DuckDB's only pointer to it (a plain assignment in
            // `duckdb_scalar_function_set_bind_data`), and no copy of it can
            // exist yet: copies are made after the bind callback returns.
            unsafe { destroy(previous) };
        }
    }

    /// Borrows the bind data during execution.
    ///
    /// Returns `None` if the bind callback never stored anything, or if
    /// `DuckDB` copied the bound expression and `T::clone` panicked while
    /// copying it. In the second case it also reports the panic message as
    /// this call's error ([`ScalarFunctionInfo::set_error`]), so the query
    /// fails with it instead of answering as if no bind data had been set; a
    /// later `set_error` in the callback replaces that message (use
    /// [`try_get`][Self::try_get] to handle it yourself).
    ///
    /// # Safety
    ///
    /// - The bind callback must have stored a `T` via [`set`][Self::set] — and
    ///   nothing else. Reading a different type reinterprets memory.
    /// - The returned reference must not outlive the callback invocation that
    ///   `info` belongs to. The lifetime `'a` is the caller's to choose and
    ///   nothing ties it to `info`; `DuckDB` frees the bind data when the query
    ///   finishes, and a reference kept past that reads freed memory.
    #[must_use]
    pub unsafe fn get<'a>(info: &ScalarFunctionInfo) -> Option<&'a T> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_bind_data() };
        // SAFETY: `raw` is null or was produced by `set` (or copied from it),
        // and DuckDB keeps it alive for the whole execution.
        unsafe { read_slot(raw, |message| info.set_error(message)) }
    }

    /// Borrows the bind data during execution, or returns the error a failed
    /// copy of it left: `Ok(None)` if the bind callback never stored
    /// anything, `Err` with the message [`get`][Self::get] reports if
    /// `DuckDB` copied the bound expression and `T::clone` panicked.
    ///
    /// # Errors
    ///
    /// The panic message of the failed `T::clone`, with context.
    ///
    /// # Safety
    ///
    /// See [`get`][Self::get].
    pub unsafe fn try_get<'a>(info: &ScalarFunctionInfo) -> Result<Option<&'a T>, String> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_bind_data() };
        let mut failure = None;
        // SAFETY: as in `get`.
        let value = unsafe { read_slot(raw, |message| failure = Some(message.to_owned())) };
        failure.map_or(Ok(value), Err)
    }

    /// Borrows the bind data during the init callback.
    ///
    /// Like [`get`][Self::get], reports a failed copy as the init callback's
    /// error ([`ScalarInitInfo::set_error`]) and returns `None`.
    ///
    /// # Safety
    ///
    /// See [`get`][Self::get].
    #[must_use]
    pub unsafe fn get_from_init<'a>(info: &ScalarInitInfo) -> Option<&'a T> {
        // SAFETY: forwarded from this function's own contract.
        let raw = unsafe { info.get_bind_data() };
        // SAFETY: as in `get`.
        unsafe { read_slot(raw, |message| info.set_error(message)) }
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
    #[cfg_attr(test, mutants::skip)]
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
    /// - The returned reference must not outlive the callback invocation that
    ///   `info` belongs to. The lifetime `'a` is the caller's to choose and
    ///   nothing ties it to `info`; `DuckDB` frees the local state when the
    ///   thread's execution ends.
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

    fn slot(value: Cloned) -> *mut c_void {
        Box::into_raw(Box::new(Slot::Value(value))).cast()
    }

    /// The value behind a slot, or the failure it reports.
    fn read(raw: *mut c_void) -> Result<Option<Cloned>, String> {
        let mut failure = None;
        // SAFETY: every `raw` here is null or a live `Slot<Cloned>`.
        let value = unsafe { read_slot::<Cloned>(raw, |m| failure = Some(m.to_owned())) };
        failure.map_or_else(|| Ok(value.cloned()), Err)
    }

    #[test]
    fn the_generated_copy_callback_produces_an_independent_box() {
        let raw = slot(Cloned(vec![1, 2, 3]));
        // SAFETY: `raw` is a live `Box<Slot<Cloned>>`.
        let copy = unsafe { clone_slot::<Cloned>(raw) };
        assert!(!copy.is_null());
        assert_ne!(copy, raw, "a copy must be a separate allocation");
        assert_eq!(read(copy), Ok(Some(Cloned(vec![1, 2, 3]))));
        // SAFETY: both are live `Box<Slot<Cloned>>` allocations.
        unsafe { drop_boxed::<Slot<Cloned>>(raw) };
        // The copy survives the original being freed.
        assert_eq!(read(copy), Ok(Some(Cloned(vec![1, 2, 3]))));
        // SAFETY: as above.
        unsafe { drop_boxed::<Slot<Cloned>>(copy) };
    }

    #[test]
    fn the_generated_copy_callback_tolerates_null() {
        // SAFETY: the null case is handled explicitly.
        assert!(unsafe { clone_slot::<Cloned>(std::ptr::null_mut()) }.is_null());
        assert_eq!(read(std::ptr::null_mut()), Ok(None), "no bind data");
    }

    /// FFI-6: a panicking `Clone` used to make the copy null, which reads as
    /// "no bind data" (the query answered wrongly, the panic message was
    /// lost). The copy now carries the message, reading it reports it, and a
    /// copy of the copy keeps it.
    #[test]
    fn a_panicking_clone_is_reported_when_the_copy_is_read() {
        let raw = slot(Cloned(b"bomb".to_vec()));
        // SAFETY: `raw` is a live `Box<Slot<Cloned>>`.
        let copy = unsafe { clone_slot::<Cloned>(raw) };
        assert!(!copy.is_null(), "a failed clone is not 'no bind data'");
        let err = read(copy).expect_err("the failure is reported");
        assert!(
            err.contains("bind data clone deliberately exploded") && err.contains("Clone panicked"),
            "{err}"
        );
        // SAFETY: `copy` is a live `Box<Slot<Cloned>>`.
        let again = unsafe { clone_slot::<Cloned>(copy) };
        assert_eq!(read(again), Err(err));
        // SAFETY: each is live and freed once.
        unsafe {
            drop_boxed::<Slot<Cloned>>(raw);
            drop_boxed::<Slot<Cloned>>(copy);
            drop_boxed::<Slot<Cloned>>(again);
        }
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
