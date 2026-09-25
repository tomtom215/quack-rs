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
//! through [`FfiBindData`][crate::table::FfiBindData] /
//! [`FfiInitData`][crate::table::FfiInitData]. For extensions that merely need
//! "take some parameters at bind, stream rows until exhausted", that ceremony
//! is largely accidental complexity. [`TypedTableFunctionBuilder`] collapses
//! it to closures.
//!
//! # Two entry points
//!
//! | Constructor | Bind produces | Each execution starts from |
//! |---|---|---|
//! | [`TableFunctionBuilder::with_state`] | a template `S: Clone` | a clone of the template |
//! | [`TableFunctionBuilder::with_bind_init`] | immutable bind data `B` | `init(&B)` |
//!
//! Use `with_state` when the scan state is cheap to clone. Use
//! `with_bind_init` when it is not (it holds a file handle, a large buffer, a
//! connection), or when the parameters and the cursor are naturally separate.
//!
//! ```rust,no_run
//! use quack_rs::prelude::*;
//!
//! #[derive(Clone)]
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
//! - The `bind` closure runs **once per bind** — once per query plan, not once
//!   per execution. It receives a [`BindInfo`], declares the output schema,
//!   and reads parameters.
//! - `DuckDB` keeps that one bind result for the lifetime of the plan and runs
//!   `init` against it **every time the plan executes**: each `EXECUTE` of a
//!   prepared statement, each iteration of a recursive CTE that references the
//!   function, and so on. The builder therefore never *moves* state out of the
//!   bind result; each execution gets a fresh `S` (a clone of the template, or
//!   the result of your `init` closure).
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
//! [`InitInfo::set_max_threads`][crate::table::InitInfo::set_max_threads] with
//! `1`. Extensions that want true multi-worker parallelism should continue to
//! use the raw [`TableFunctionBuilder`] API and split state across
//! `local_init`.
//!
//! # No projection pushdown
//!
//! The typed builder does not offer `projection_pushdown`,
//! [`build`][TypedTableFunctionBuilder::build] returns an error if it was
//! switched on on the raw builder before `with_state` / `with_bind_init`, and
//! registering the builder `build` returns fails if it was switched on
//! afterwards. So does replacing its `bind`, `init`, `local_init`, `scan` or
//! `extra_info`: the typed callbacks read one another's data.
//! With pushdown on,
//! `DuckDB` hands the scan a chunk holding only the *projected* columns, in
//! projection order, so `chunk.writer(0)` is no longer "the first declared
//! column" — and the scan closure has no way to learn the mapping. A scan
//! written against the declared schema would then write column `a`'s values
//! into column `b`. Use the raw [`TableFunctionBuilder`] with
//! [`InitInfo::projected_column_index`][crate::table::InitInfo::projected_column_index]
//! when you need pushdown.
//!
//! ```rust,compile_fail
//! use quack_rs::prelude::*;
//!
//! // Does not compile: the typed builder has no `projection_pushdown`.
//! let _ = TableFunctionBuilder::new("two_cols")
//!     .with_state(|_bind| Ok(0_u8))
//!     .projection_pushdown(true);
//! ```

use std::sync::{Arc, Mutex, PoisonError};

use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::table::builder::TableFunctionBuilder;
use crate::table::info::BindInfo;
use crate::types::{LogicalType, TypeId};

mod trampolines;

/// Produces a fresh scan state for one execution of a bound plan.
///
/// Stored as the table function's bind data. `DuckDB` may call `init` for the
/// same bind data from any worker thread, hence `Send + Sync`.
type StateFactory<S> = dyn Fn() -> Result<S, ExtensionError> + Send + Sync + 'static;

/// Boxed bind closure signature stored inside [`TypedCallbacks`]: declares the
/// schema, then returns the factory every later `init` draws its state from.
type BindClosure<S> =
    dyn Fn(&BindInfo) -> Result<Box<StateFactory<S>>, ExtensionError> + Send + Sync + 'static;

/// Boxed scan closure signature stored inside [`TypedCallbacks`].
type ScanClosure<S> =
    dyn Fn(&mut S, &DataChunk) -> Result<(), ExtensionError> + Send + Sync + 'static;

/// Heap-allocated bundle of user closures, stored as the table function's
/// `extra_info` so it survives across FFI callbacks.
struct TypedCallbacks<S: Send + 'static> {
    bind: Box<BindClosure<S>>,
    scan: Box<ScanClosure<S>>,
}

/// Closure-based builder for table functions with a typed, mutable scan state.
///
/// Obtain one via [`TableFunctionBuilder::with_state`] or
/// [`TableFunctionBuilder::with_bind_init`]. Set a scan closure with
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
    /// Switches this builder into closure-based "typed state" mode, with the
    /// scan state cloned from a template for every execution.
    ///
    /// The supplied `bind` closure runs once per bind (see the
    /// [module docs][crate::table::typed] for what that means for prepared
    /// statements). It must:
    ///
    /// - Declare the output schema via
    ///   [`BindInfo::add_result_column`][crate::table::BindInfo::add_result_column]
    ///   — at least one column. With `duckdb-1-5`, a bind that declares none
    ///   is reported as an ordinary bind error instead of the `INTERNAL Error`
    ///   `DuckDB` raises for it. Used as a `COPY … FROM` reader, it must
    ///   instead declare none and read the target table's columns
    ///   (`BindInfo::result_column_count`);
    ///   with `duckdb-1-5` a column declared there fails the bind.
    /// - Read parameters (positional or named) from the [`BindInfo`].
    /// - Return the *template* scan state `S` on success, or an
    ///   [`ExtensionError`] on failure. Errors are propagated to `DuckDB` via
    ///   `duckdb_bind_set_error`.
    ///
    /// Every execution of the bound plan starts from `template.clone()`, so a
    /// prepared statement executed twice scans the same rows twice. If `S` is
    /// expensive or impossible to clone, use
    /// [`with_bind_init`][Self::with_bind_init] instead.
    ///
    /// Continue building the function by calling [`scan`][TypedTableFunctionBuilder::scan].
    ///
    /// See the [module-level docs][crate::table::typed] for an end-to-end example.
    pub fn with_state<S, F>(self, bind: F) -> TypedTableFunctionBuilder<S>
    where
        S: Clone + Send + 'static,
        F: Fn(&BindInfo) -> Result<S, ExtensionError> + Send + Sync + 'static,
    {
        let bind: Box<BindClosure<S>> = Box::new(move |info| {
            // `S` is `Send` but not necessarily `Sync`, and `init` may run on
            // any thread, so the template sits behind a `Mutex`. `clone` takes
            // `&S`, so a panic inside it cannot leave the template half
            // modified: recovering from poison is sound.
            let template = Mutex::new(bind(info)?);
            let factory: Box<StateFactory<S>> = Box::new(move || {
                Ok(template
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone())
            });
            Ok(factory)
        });
        TypedTableFunctionBuilder {
            inner: self,
            bind: Some(bind),
            scan: None,
        }
    }

    /// Switches this builder into closure-based "typed state" mode, with
    /// immutable bind data `B` and a fresh scan state `S` built from it for
    /// every execution.
    ///
    /// - `bind` runs once per bind: declare the output schema with
    ///   [`BindInfo::add_result_column`][crate::table::BindInfo::add_result_column],
    ///   read parameters, and return the bind data `B`.
    /// - `init` runs once per *execution* of the bound plan — every `EXECUTE`
    ///   of a prepared statement, every re-scan inside a recursive CTE — and
    ///   builds the scan state `S` from `&B`. This is where to open files or
    ///   reset cursors.
    ///
    /// `B` must be `Send + Sync` because `DuckDB` may run `init` for the same
    /// bind data on any worker thread. `S` need only be `Send`: scans are
    /// serialised (see the [module docs][crate::table::typed]).
    ///
    /// Errors from either closure are reported to `DuckDB` (`bind` as a binder
    /// error, `init` as an invalid-input error).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::prelude::*;
    ///
    /// struct Params { n: i64 }
    /// struct Cursor { next: i64, end: i64 }
    ///
    /// fn register(reg: &impl Registrar) -> ExtResult<()> {
    ///     let builder = TableFunctionBuilder::new("count_up")
    ///         .param(TypeId::BigInt)
    ///         .with_bind_init(
    ///             |bind| {
    ///                 bind.add_result_column("n", TypeId::BigInt);
    ///                 let n = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
    ///                 Ok(Params { n })
    ///             },
    ///             |params: &Params| Ok(Cursor { next: 1, end: params.n }),
    ///         )
    ///         .scan(|cursor, chunk| {
    ///             if cursor.next > cursor.end {
    ///                 unsafe { chunk.set_size(0) };
    ///                 return Ok(());
    ///             }
    ///             unsafe {
    ///                 chunk.writer(0).write_i64(0, cursor.next);
    ///                 chunk.set_size(1);
    ///             }
    ///             cursor.next += 1;
    ///             Ok(())
    ///         })
    ///         .build()?;
    ///     unsafe { reg.register_table(builder) }
    /// }
    /// ```
    pub fn with_bind_init<B, S, FB, FI>(self, bind: FB, init: FI) -> TypedTableFunctionBuilder<S>
    where
        B: Send + Sync + 'static,
        S: Send + 'static,
        FB: Fn(&BindInfo) -> Result<B, ExtensionError> + Send + Sync + 'static,
        FI: Fn(&B) -> Result<S, ExtensionError> + Send + Sync + 'static,
    {
        let init = Arc::new(init);
        let bind: Box<BindClosure<S>> = Box::new(move |info| {
            let bound = bind(info)?;
            let init = Arc::clone(&init);
            let factory: Box<StateFactory<S>> = Box::new(move || init(&bound));
            Ok(factory)
        });
        TypedTableFunctionBuilder {
            inner: self,
            bind: Some(bind),
            scan: None,
        }
    }
}

impl<S: Send + 'static> TypedTableFunctionBuilder<S> {
    /// Sets the scan closure.
    ///
    /// The closure receives a mutable reference to the scan state for the
    /// current execution, plus a [`DataChunk`] for the output chunk. It must
    /// fill the chunk with zero or more output rows and set the chunk size via
    /// [`DataChunk::set_size`] or a [`ChunkWriter`][crate::chunk_writer::ChunkWriter].
    /// Returning with chunk size zero signals end-of-stream to `DuckDB`.
    ///
    /// Column `i` of the chunk is the `i`-th column declared in `bind`: the
    /// typed builder never enables projection pushdown (and
    /// [`build`][Self::build] refuses a raw builder that had it enabled).
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

    /// Finalises the typed builder into a raw [`TableFunctionBuilder`] ready
    /// for registration.
    ///
    /// The returned builder has its `bind`, `init`, and `scan` callbacks wired
    /// to closure trampolines, and stores the user closures in `extra_info`
    /// for the lifetime of the registered function.
    ///
    /// The closures are owned by the returned builder's `extra_info`: they are
    /// handed to `DuckDB` on successful registration, and freed when the
    /// builder is dropped otherwise — an unregistered builder does not leak.
    ///
    /// # Errors
    ///
    /// Returns an error if [`scan`][Self::scan] was never called (the bind
    /// closure is always set at construction time), or if
    /// [`projection_pushdown`][TableFunctionBuilder::projection_pushdown] was
    /// enabled on the raw builder before it was turned into a typed one. The
    /// typed scan closure writes columns in declaration order and cannot learn
    /// the projection, so with pushdown on `SELECT b` would receive column
    /// `a`'s values. Switching it off silently would change configuration the
    /// caller asked for, so it is an error instead.
    pub fn build(self) -> Result<TableFunctionBuilder, ExtensionError> {
        if self.inner.projection_pushdown_enabled() {
            return Err(ExtensionError::new(format!(
                "typed table function '{}': projection_pushdown was enabled on the builder \
                 before with_state/with_bind_init. The typed scan closure writes columns in \
                 declaration order and cannot see the projection, so pushdown would put one \
                 column's values under another's name. Remove projection_pushdown(true), or \
                 use the raw TableFunctionBuilder with InitInfo::projected_column_index.",
                self.inner.name()
            )));
        }
        let bind = self
            .bind
            .ok_or_else(|| ExtensionError::new("typed table function: bind closure not set"))?;
        let scan = self
            .scan
            .ok_or_else(|| ExtensionError::new("typed table function: scan closure not set"))?;
        Ok(trampolines::wire(
            self.inner,
            TypedCallbacks::<S> { bind, scan },
        ))
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
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
    fn with_bind_init_produces_typed_builder() {
        struct NotClone;
        let typed = TableFunctionBuilder::new("demo")
            .with_bind_init(|_bind| Ok(3_u64), |_n: &u64| Ok(NotClone));
        assert_eq!(typed.name(), "demo");
        assert!(typed.bind.is_some());
        assert!(typed.scan.is_none());
    }

    unsafe extern "C" fn noop_bind(_: libduckdb_sys::duckdb_bind_info) {}
    unsafe extern "C" fn noop_init(_: libduckdb_sys::duckdb_init_info) {}
    unsafe extern "C" fn noop_scan(
        _: libduckdb_sys::duckdb_function_info,
        _: libduckdb_sys::duckdb_data_chunk,
    ) {
    }

    #[test]
    fn projection_pushdown_after_build_is_refused_at_registration() {
        use crate::connection::Registrar;
        use crate::testing::MockRegistrar;
        let built = || {
            TableFunctionBuilder::new("demo")
                .with_state::<DummyState, _>(|_bind| Ok(DummyState { _rows: 10 }))
                .scan(|_state, _chunk| Ok(()))
                .build()
                .expect("build")
        };
        let registrar = MockRegistrar::new();
        // SAFETY: `MockRegistrar` never calls `DuckDB`.
        let err = unsafe { registrar.register_table(built().projection_pushdown(true)) }
            .expect_err("pushdown after build");
        assert!(err.as_str().contains("after build()"), "{err}");
        // SAFETY: as above.
        unsafe { registrar.register_table(built().projection_pushdown(false)) }
            .expect("pushdown off");
        // A raw builder keeps pushdown.
        let raw = TableFunctionBuilder::new("raw")
            .bind(noop_bind)
            .init(noop_init)
            .scan(noop_scan)
            .projection_pushdown(true);
        // SAFETY: as above.
        unsafe { registrar.register_table(raw) }.expect("raw pushdown");
    }

    #[test]
    fn replacing_a_typed_functions_callbacks_after_build_is_refused() {
        use crate::connection::Registrar;
        use crate::testing::MockRegistrar;
        let built = || {
            TableFunctionBuilder::new("demo")
                .with_state::<DummyState, _>(|_bind| Ok(DummyState { _rows: 10 }))
                .scan(|_state, _chunk| Ok(()))
                .build()
                .expect("build")
        };
        let registrar = MockRegistrar::new();
        for (what, builder) in [
            ("bind", built().bind(noop_bind)),
            ("init", built().init(noop_init)),
            ("local_init", built().local_init(noop_init)),
            ("scan", built().scan(noop_scan)),
        ] {
            // SAFETY: `MockRegistrar` never calls `DuckDB`.
            let err = unsafe { registrar.register_table(builder) }.expect_err(what);
            assert!(err.as_str().contains(what), "{what}: {err}");
        }
        // SAFETY: as above.
        unsafe { registrar.register_table(built()) }.expect("untouched");
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
            .named_param("path", TypeId::Varchar);
        // The inner builder's fields are private to `builder`; its `Debug`
        // output is the observable record of what the passthroughs stored.
        let inner = format!("{:?}", typed.inner);
        assert!(inner.contains("params: [Varchar]"), "{inner}");
        assert!(inner.contains("named_params: 1"), "{inner}");
    }

    #[test]
    fn build_refuses_projection_pushdown_enabled_beforehand() {
        let typed = TableFunctionBuilder::new("demo")
            .projection_pushdown(true)
            .with_state::<DummyState, _>(|_| Ok(DummyState { _rows: 0 }))
            .scan(|_state, _chunk| Ok(()));
        match typed.build() {
            Err(e) => assert!(e.as_str().contains("projection_pushdown"), "{e}"),
            Ok(_) => panic!("projection pushdown must be refused"),
        }
    }

    /// The factories are what make repeated `init` calls against one bind
    /// result work; exercise them without a live `DuckDB` by calling the
    /// factory the bind closure would have stored.
    #[test]
    fn with_state_factory_hands_out_independent_clones() {
        // SAFETY: a null bind info is never dereferenced by this closure.
        let info = unsafe { BindInfo::new(std::ptr::null_mut()) };
        let typed = TableFunctionBuilder::new("demo").with_state(|_| Ok(vec![1_u8, 2]));
        let factory = (typed.bind.expect("bind set"))(&info).expect("bind ok");
        let mut first = factory().expect("first");
        first.push(3);
        assert_eq!(first, vec![1, 2, 3]);
        assert_eq!(factory().expect("second"), vec![1, 2]);
    }

    #[test]
    fn with_bind_init_runs_init_once_per_factory_call() {
        static INITS: AtomicUsize = AtomicUsize::new(0);
        // SAFETY: a null bind info is never dereferenced by this closure.
        let info = unsafe { BindInfo::new(std::ptr::null_mut()) };
        let typed = TableFunctionBuilder::new("demo").with_bind_init(
            |_| Ok(5_i64),
            |n: &i64| {
                INITS.fetch_add(1, Ordering::SeqCst);
                Ok(*n * 2)
            },
        );
        let factory = (typed.bind.expect("bind set"))(&info).expect("bind ok");
        assert_eq!(factory().expect("first"), 10);
        assert_eq!(factory().expect("second"), 10);
        assert_eq!(INITS.load(Ordering::SeqCst), 2);
    }
}
