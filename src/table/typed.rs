// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Closure-based table functions with typed scan state.
//!
//! This module provides [`TypedTableFunctionBuilder`], a higher-level builder
//! layered on top of [`TableFunctionBuilder`]. It lets extension authors write
//! table functions using safe Rust closures instead of `unsafe extern "C" fn`
//! trampolines for `bind`, `init`, and `scan`.
//!
//! # Motivation
//!
//! The raw [`TableFunctionBuilder`] API requires authors to write three
//! hand-rolled `unsafe extern "C" fn` callbacks and manually shuttle state
//! through [`FfiBindData`] / [`FfiInitData`]. For extensions that merely need
//! "take some parameters at bind, stream rows until exhausted", that ceremony
//! is largely accidental complexity. [`TypedTableFunctionBuilder`] collapses
//! that to two closures:
//!
//! ```rust,no_run
//! use quack_rs::prelude::*;
//!
//! struct State { remaining: u64 }
//!
//! fn register(reg: &impl Registrar) -> ExtResult<()> {
//!     let builder = TableFunctionBuilder::new("count_down")
//!         .param(TypeId::BigInt)
//!         .with_state::<State, _>(|bind| {
//!             bind.add_result_column("n", TypeId::BigInt);
//!             let raw = unsafe { bind.get_parameter_value(0) };
//!             let n = raw.as_i64_or(0).max(0) as u64;
//!             Ok(State { remaining: n })
//!         })
//!         .scan(|state, chunk| {
//!             if state.remaining == 0 {
//!                 unsafe { chunk.set_size(0) };
//!                 return Ok(());
//!             }
//!             let mut writer = unsafe { chunk.writer(0) };
//!             unsafe { writer.write_i64(0, state.remaining as i64) };
//!             state.remaining -= 1;
//!             unsafe { chunk.set_size(1) };
//!             Ok(())
//!         })
//!         .build()?;
//!     unsafe { reg.register_table(builder) }
//! }
//! ```
//!
//! # Design
//!
//! - The `bind` closure runs exactly once per query. It receives a
//!   [`BindInfo`], declares the output schema, reads parameters, and returns
//!   the initial state `S`.
//! - The `scan` closure runs repeatedly until it sets the output chunk size
//!   to zero. It receives `&mut S` and a [`DataChunk`] for output.
//! - Panics in user closures are caught via `std::panic::catch_unwind`; the
//!   error is reported through `DuckDB` and the chunk size is forced to zero
//!   to safely terminate the scan.
//!
//! # Threading
//!
//! Because `S` is only required to be `Send + 'static` (not `Sync`), the typed
//! builder forces scans to execute on a single worker via
//! [`InitInfo::set_max_threads`] with `1`. Extensions that want true multi-worker
//! parallelism should continue to use the raw [`TableFunctionBuilder`] API and
//! split state across `local_init`.

use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use libduckdb_sys::{
    duckdb_bind_info, duckdb_data_chunk, duckdb_data_chunk_set_size, duckdb_function_info,
    duckdb_init_info,
};

use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::table::bind_data::FfiBindData;
use crate::table::builder::TableFunctionBuilder;
use crate::table::info::{BindInfo, FunctionInfo, InitInfo};
use crate::table::init_data::FfiInitData;
use crate::types::{LogicalType, TypeId};

/// Boxed bind closure signature stored inside [`TypedCallbacks`].
type BindClosure<S> = dyn Fn(&BindInfo) -> Result<S, ExtensionError> + Send + Sync + 'static;

/// Boxed scan closure signature stored inside [`TypedCallbacks`].
type ScanClosure<S> =
    dyn Fn(&mut S, &DataChunk) -> Result<(), ExtensionError> + Send + Sync + 'static;

/// Heap-allocated bundle of user closures, stored as the table function's
/// `extra_info` so it survives across FFI callbacks.
struct TypedCallbacks<S: Send + 'static> {
    bind: Box<BindClosure<S>>,
    scan: Box<ScanClosure<S>>,
}

impl<S: Send + 'static> TypedCallbacks<S> {
    /// `extra_info` destructor passed to `DuckDB`.
    ///
    /// # Safety
    ///
    /// `ptr` must have been produced by [`Box::into_raw`] on a
    /// `Box<TypedCallbacks<S>>` created by [`TypedTableFunctionBuilder::build`].
    /// `DuckDB` calls this exactly once when the table function is dropped.
    unsafe extern "C" fn destroy_extra(ptr: *mut c_void) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: ptr was produced by Box::into_raw in `build`.
        unsafe {
            drop(Box::from_raw(ptr.cast::<Self>()));
        }
    }
}

/// Closure-based builder for table functions with a typed, mutable scan state.
///
/// Obtain one via [`TableFunctionBuilder::with_state`]. Set a scan closure with
/// [`scan`][Self::scan] and finish with [`build`][Self::build] to recover a
/// fully-configured [`TableFunctionBuilder`] that can be passed to any
/// [`Registrar`][crate::connection::Registrar].
///
/// # Example
///
/// See the [module-level docs][crate::table::typed] for a complete example.
#[must_use]
pub struct TypedTableFunctionBuilder<S: Send + 'static> {
    inner: TableFunctionBuilder,
    bind: Option<Box<BindClosure<S>>>,
    scan: Option<Box<ScanClosure<S>>>,
}

impl TableFunctionBuilder {
    /// Switches this builder into closure-based "typed state" mode.
    ///
    /// The supplied `bind` closure runs once per query invocation. It must:
    ///
    /// - Declare the output schema via
    ///   [`BindInfo::add_result_column`][crate::table::BindInfo::add_result_column].
    /// - Read parameters (positional or named) from the [`BindInfo`].
    /// - Return the initial scan state `S` on success, or an
    ///   [`ExtensionError`] on failure. Errors are propagated to `DuckDB` via
    ///   `duckdb_bind_set_error`.
    ///
    /// Continue building the function by calling [`scan`][TypedTableFunctionBuilder::scan].
    ///
    /// See the [module-level docs][crate::table::typed] for an end-to-end example.
    pub fn with_state<S, F>(self, bind: F) -> TypedTableFunctionBuilder<S>
    where
        S: Send + 'static,
        F: Fn(&BindInfo) -> Result<S, ExtensionError> + Send + Sync + 'static,
    {
        TypedTableFunctionBuilder {
            inner: self,
            bind: Some(Box::new(bind)),
            scan: None,
        }
    }
}

impl<S: Send + 'static> TypedTableFunctionBuilder<S> {
    /// Sets the scan closure.
    ///
    /// The closure receives a mutable reference to the scan state produced by
    /// the `bind` closure, plus a [`DataChunk`] for the output chunk. It must
    /// fill the chunk with zero or more output rows and set the chunk size via
    /// [`DataChunk::set_size`] or a [`ChunkWriter`][crate::chunk_writer::ChunkWriter].
    /// Returning with chunk size zero signals end-of-stream to `DuckDB`.
    ///
    /// Errors are reported through `duckdb_function_set_error` and terminate
    /// the scan.
    pub fn scan<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut S, &DataChunk) -> Result<(), ExtensionError> + Send + Sync + 'static,
    {
        self.scan = Some(Box::new(f));
        self
    }

    /// Returns the underlying function name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Adds a positional parameter. Delegates to [`TableFunctionBuilder::param`].
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.inner = self.inner.param(type_id);
        self
    }

    /// Adds a positional parameter with a complex [`LogicalType`].
    /// Delegates to [`TableFunctionBuilder::param_logical`].
    pub fn param_logical(mut self, logical_type: LogicalType) -> Self {
        self.inner = self.inner.param_logical(logical_type);
        self
    }

    /// Adds a named parameter. Delegates to [`TableFunctionBuilder::named_param`].
    pub fn named_param(mut self, name: &str, type_id: TypeId) -> Self {
        self.inner = self.inner.named_param(name, type_id);
        self
    }

    /// Adds a named parameter with a complex [`LogicalType`].
    /// Delegates to [`TableFunctionBuilder::named_param_logical`].
    pub fn named_param_logical(mut self, name: &str, logical_type: LogicalType) -> Self {
        self.inner = self.inner.named_param_logical(name, logical_type);
        self
    }

    /// Enables or disables projection pushdown.
    /// Delegates to [`TableFunctionBuilder::projection_pushdown`].
    pub fn projection_pushdown(mut self, enable: bool) -> Self {
        self.inner = self.inner.projection_pushdown(enable);
        self
    }

    /// Finalises the typed builder into a raw [`TableFunctionBuilder`] ready
    /// for registration.
    ///
    /// The returned builder has its `bind`, `init`, and `scan` callbacks wired
    /// to closure trampolines, and stores the user closures in `extra_info`
    /// for the lifetime of the registered function.
    ///
    /// # Errors
    ///
    /// Returns an error if [`scan`][Self::scan] was never called. The bind
    /// closure is always set at construction time via
    /// [`TableFunctionBuilder::with_state`].
    ///
    /// # Leak note
    ///
    /// On success, the returned builder owns a heap allocation that `DuckDB`
    /// will free via the registered `extra_info` destructor when registration
    /// succeeds. If the returned builder is dropped without being registered,
    /// the allocation leaks (one-shot leak per unused builder; not UB).
    pub fn build(self) -> Result<TableFunctionBuilder, ExtensionError> {
        let bind = self
            .bind
            .ok_or_else(|| ExtensionError::new("typed table function: bind closure not set"))?;
        let scan = self
            .scan
            .ok_or_else(|| ExtensionError::new("typed table function: scan closure not set"))?;

        let cbs = Box::new(TypedCallbacks::<S> { bind, scan });
        let raw = Box::into_raw(cbs).cast::<c_void>();

        // SAFETY: `raw` is a freshly-allocated, non-null pointer to a
        // `TypedCallbacks<S>`. `destroy_extra` drops the same type.
        let builder = unsafe {
            self.inner
                .bind(typed_bind_trampoline::<S>)
                .init(typed_init_trampoline::<S>)
                .scan(typed_scan_trampoline::<S>)
                .extra_info(raw, TypedCallbacks::<S>::destroy_extra)
        };
        Ok(builder)
    }
}

/// Extracts a human-readable message from a `catch_unwind` panic payload.
///
/// The payload text is included: `DuckDB`'s `set_error` takes an ordinary
/// `&str`, so there is no reason to discard the one piece of information that
/// tells the user *which* assertion or `unwrap` failed.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&'static str>().map_or_else(
        || {
            payload.downcast_ref::<String>().map_or_else(
                || {
                    String::from(
                        "quack-rs: typed table function closure panicked (unknown payload)",
                    )
                },
                |s| format!("quack-rs: typed table function closure panicked: {s}"),
            )
        },
        |s| format!("quack-rs: typed table function closure panicked: {s}"),
    )
}

/// Bind trampoline monomorphised per state type `S`.
///
/// # Safety
///
/// Invoked by `DuckDB` during query parsing. `info` is a valid `duckdb_bind_info`.
unsafe extern "C" fn typed_bind_trampoline<S: Send + 'static>(info: duckdb_bind_info) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: DuckDB guarantees `info` is valid for the duration of the bind callback.
        let bind_info = unsafe { BindInfo::new(info) };
        // SAFETY: extra_info was set by `build()` to a `Box<TypedCallbacks<S>>`.
        let raw = unsafe { bind_info.get_extra_info() };
        if raw.is_null() {
            bind_info.set_error("quack-rs: typed table function missing extra_info");
            return;
        }
        // SAFETY: `raw` originated from `Box::into_raw(Box::new(TypedCallbacks::<S>))`
        // in `build()`. It remains valid until DuckDB invokes `destroy_extra`.
        let cbs = unsafe { &*raw.cast::<TypedCallbacks<S>>() };

        match (cbs.bind)(&bind_info) {
            Ok(state) => {
                // SAFETY: `info` is valid; this is the bind callback's single
                // opportunity to set bind data. We wrap the state in a
                // Mutex<Option<_>> so the init trampoline can take it out.
                unsafe {
                    FfiBindData::<Mutex<Option<S>>>::set(info, Mutex::new(Some(state)));
                }
            }
            Err(e) => bind_info.set_error(e.as_str()),
        }
    }));

    if let Err(payload) = outcome {
        // SAFETY: `info` is valid.
        let bind_info = unsafe { BindInfo::new(info) };
        bind_info.set_error(&panic_message(&*payload));
    }
}

/// Init trampoline monomorphised per state type `S`.
///
/// Moves the state produced during `bind` into the init-data slot so the
/// scan trampoline can read `&mut S`.
///
/// # Safety
///
/// Invoked by `DuckDB` once per query. `info` is a valid `duckdb_init_info`.
unsafe extern "C" fn typed_init_trampoline<S: Send + 'static>(info: duckdb_init_info) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `info` is valid for the duration of this callback.
        let init_info = unsafe { InitInfo::new(info) };

        // SAFETY: bind_data was set by `typed_bind_trampoline` as a
        // `Mutex<Option<S>>`. It remains alive until query teardown.
        let bind_state = unsafe { FfiBindData::<Mutex<Option<S>>>::get_from_init(info) };

        let Some(cell) = bind_state else {
            init_info.set_error("quack-rs: typed table function missing bind state");
            return;
        };

        let taken = if let Ok(mut guard) = cell.lock() {
            guard.take()
        } else {
            init_info.set_error("quack-rs: typed table function bind-state mutex poisoned");
            return;
        };

        let Some(state) = taken else {
            init_info.set_error("quack-rs: typed table function bind state already consumed");
            return;
        };

        // SAFETY: `info` is valid; `FfiInitData::set` boxes the state and
        // registers a drop-on-destroy callback with DuckDB.
        unsafe {
            FfiInitData::<S>::set(info, state);
        }

        // `S` is only `Send`, not `Sync`, so we cannot safely share it across
        // parallel scan workers. Force serial scans.
        init_info.set_max_threads(1);
    }));

    if let Err(payload) = outcome {
        // SAFETY: `info` is valid.
        let init_info = unsafe { InitInfo::new(info) };
        init_info.set_error(&panic_message(&*payload));
    }
}

/// Scan trampoline monomorphised per state type `S`.
///
/// # Safety
///
/// Invoked by `DuckDB` repeatedly until the chunk size is set to zero.
unsafe extern "C" fn typed_scan_trampoline<S: Send + 'static>(
    info: duckdb_function_info,
    output: duckdb_data_chunk,
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `info` is valid per DuckDB contract.
        let fninfo = unsafe { FunctionInfo::new(info) };
        // SAFETY: extra_info was set by `build()`.
        let raw = unsafe { fninfo.get_extra_info() };
        if raw.is_null() {
            fninfo.set_error("quack-rs: typed table function missing extra_info");
            // SAFETY: `output` is a valid data chunk.
            unsafe { duckdb_data_chunk_set_size(output, 0) };
            return;
        }
        // SAFETY: same provenance as the bind trampoline.
        let cbs = unsafe { &*raw.cast::<TypedCallbacks<S>>() };

        // SAFETY: init_data was set by `typed_init_trampoline`; the scan
        // runs serialised (set_max_threads(1)), so no aliasing &mut exists.
        let state = unsafe { FfiInitData::<S>::get_mut(info) };
        let Some(state) = state else {
            fninfo.set_error("quack-rs: typed table function missing scan state");
            // SAFETY: `output` is valid.
            unsafe { duckdb_data_chunk_set_size(output, 0) };
            return;
        };

        // SAFETY: `output` is a valid data chunk provided by DuckDB.
        let chunk = unsafe { DataChunk::from_raw(output) };
        if let Err(e) = (cbs.scan)(state, &chunk) {
            fninfo.set_error(e.as_str());
            // SAFETY: `output` is valid.
            unsafe { duckdb_data_chunk_set_size(output, 0) };
        }
    }));

    if let Err(payload) = outcome {
        // SAFETY: `info` is valid.
        let fninfo = unsafe { FunctionInfo::new(info) };
        fninfo.set_error(&panic_message(&*payload));
        // SAFETY: `output` is valid.
        unsafe { duckdb_data_chunk_set_size(output, 0) };
    }
}

impl<S: Send + 'static> core::fmt::Debug for TypedTableFunctionBuilder<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        f.debug_struct("TypedTableFunctionBuilder")
            .field("state", &core::any::type_name::<S>())
            .field("inner", &self.inner)
            .field("bind", &Callback::of(&self.bind))
            .field("scan", &Callback::of(&self.scan))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyState {
        _rows: u64,
    }

    #[test]
    fn with_state_produces_typed_builder() {
        let typed = TableFunctionBuilder::new("demo")
            .with_state::<DummyState, _>(|_bind| Ok(DummyState { _rows: 10 }));
        assert_eq!(typed.name(), "demo");
        assert!(typed.bind.is_some());
        assert!(typed.scan.is_none());
    }

    #[test]
    fn build_without_scan_errors() {
        let typed = TableFunctionBuilder::new("demo")
            .with_state::<DummyState, _>(|_bind| Ok(DummyState { _rows: 10 }));
        match typed.build() {
            Err(e) => assert!(e.as_str().contains("scan closure not set")),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn build_with_bind_and_scan_succeeds() {
        let typed = TableFunctionBuilder::new("demo")
            .param(TypeId::BigInt)
            .with_state::<DummyState, _>(|_bind| Ok(DummyState { _rows: 10 }))
            .scan(|_state, _chunk| Ok(()));
        let builder = typed.build().expect("build should succeed");
        assert_eq!(builder.name(), "demo");
    }

    #[test]
    fn passthroughs_mutate_inner_builder() {
        let typed = TableFunctionBuilder::new("demo")
            .with_state::<DummyState, _>(|_| Ok(DummyState { _rows: 0 }))
            .param(TypeId::Varchar)
            .named_param("path", TypeId::Varchar)
            .projection_pushdown(true);
        assert_eq!(typed.name(), "demo");
    }

    #[test]
    fn destroy_extra_null_is_noop() {
        // Must not panic.
        unsafe {
            TypedCallbacks::<DummyState>::destroy_extra(std::ptr::null_mut());
        }
    }

    #[test]
    fn destroy_extra_drops_box() {
        let cbs: Box<TypedCallbacks<DummyState>> = Box::new(TypedCallbacks {
            bind: Box::new(|_| Ok(DummyState { _rows: 0 })),
            scan: Box::new(|_, _| Ok(())),
        });
        let raw = Box::into_raw(cbs).cast::<c_void>();
        unsafe { TypedCallbacks::<DummyState>::destroy_extra(raw) };
    }

    #[test]
    fn panic_message_classifies_known_payloads() {
        let s: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert!(panic_message(&*s).contains("panicked"));
        let s: Box<dyn std::any::Any + Send> = Box::new(String::from("boom"));
        assert!(panic_message(&*s).contains("panicked"));
        // Unknown payload falls through to the "unknown payload" branch.
        let s: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert!(panic_message(&*s).contains("unknown payload"));
    }
}
