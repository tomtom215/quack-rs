// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`Connection`] — version-agnostic extension registration facade.
//!
//! [`Connection`] wraps the `duckdb_connection` and `duckdb_database` handles
//! provided to your extension during initialization. It implements the
//! [`Registrar`] trait, which provides a uniform API for registering all
//! extension components that works identically across `DuckDB` 1.4.x and 1.5.x.
//!
//! # Obtaining a `Connection`
//!
//! Use [`init_extension_v2`][crate::entry_point::init_extension_v2] or the
//! [`entry_point_v2!`][crate::entry_point_v2] macro. These pass a `&Connection`
//! to your registration callback instead of the raw `duckdb_connection`.
//!
//! ```rust,no_run
//! use quack_rs::connection::{Connection, Registrar};
//! use quack_rs::error::ExtensionError;
//! use quack_rs::scalar::ScalarFunctionBuilder;
//!
//! unsafe fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
//!     let builder = ScalarFunctionBuilder::map1("my_fn", |x: i64| x + 1)?;
//!     unsafe { reg.register_typed_scalar(builder) }
//! }
//!
//! quack_rs::entry_point_v2!(my_extension_init_c_api, |con| {
//!     unsafe { register_all(con) }
//! });
//! ```
//!
//! # `DuckDB` version compatibility
//!
//! [`Connection`] and [`Registrar`] provide a stable API across `DuckDB` 1.4.x,
//! 1.5.x and 2.0.0. The underlying C API version string (`"v1.2.0"`) loads in every
//! one of them: E2E tests confirm it for `DuckDB` 1.4.4, 1.5.0 and 2.0.0, and 1.5.6
//! and 2.0.0, which declare `v1.5.6`, accept any earlier C API version (their
//! `VersioningUtils::IsSupportedCAPIVersion`).
//!
//! When a future `DuckDB` release changes the C API version or adds new
//! registration surface, additional methods will be added to [`Connection`]
//! behind a version-specific feature flag (e.g. `duckdb-1-5`).

use core::ffi::c_void;

use libduckdb_sys::{duckdb_connection, duckdb_database, duckdb_delete_callback_t};

use crate::aggregate::{AggregateFunctionBuilder, AggregateFunctionSetBuilder};
use crate::cast::CastFunctionBuilder;
#[cfg(feature = "duckdb-1-5")]
use crate::copy_function::CopyFunctionBuilder;
use crate::error::ExtensionError;
use crate::replacement_scan::{ReplacementScanBuilder, ReplacementScanFn};
use crate::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, TypedScalarFunctionBuilder};
use crate::sql_macro::SqlMacro;
use crate::table::TableFunctionBuilder;

/// Version-agnostic trait for registering `DuckDB` extension components.
///
/// Implemented by [`Connection`]. Writing registration code against this trait
/// means the same code compiles and runs on `DuckDB` 1.4.x and 1.5.x without
/// modification.
///
/// # Why every method is `unsafe`
///
/// Each method requires that the implementor's underlying `duckdb_connection`
/// is valid for the call, and that the raw callbacks installed on the builder
/// fit the calls `DuckDB` will make to them: each callback's own `# Safety`
/// preconditions must hold where it is installed. For a raw aggregate that
/// means `state_size`, `init`, `destructor` and the data callbacks all
/// describe the same state ([`AggregateFunctionBuilder::ffi_state`][crate::aggregate::AggregateFunctionBuilder::ffi_state]
/// installs `FfiState<T>`'s three together); scalar and table bind/init
/// callbacks are kept apart by their argument types. Builders whose callbacks
/// quack-rs installs itself — a closure-built [`TypedScalarFunctionBuilder`],
/// a typed table function — add no obligation.
///
/// For the [`Connection`] handed to your registration closure, the connection
/// requirement always holds: quack-rs builds it from the handles `DuckDB`
/// passed the entry point and only lends it to the closure, so it cannot
/// outlive them.
///
/// They stay `unsafe` because `Registrar` is a *safe* trait: any type can
/// implement it around any handle, so a safe method could not rely on the
/// handle being valid. Making them safe would mean `unsafe trait Registrar`, a
/// breaking change for every implementor (including test doubles like
/// [`MockRegistrar`][crate::testing::MockRegistrar]) that buys nothing for the
/// one implementation the entry point hands out.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::connection::Registrar;
/// use quack_rs::error::ExtensionError;
/// use quack_rs::scalar::ScalarFunctionBuilder;
///
/// /// Register all functions for this extension.
/// ///
/// /// # Safety
/// ///
/// /// `reg` must provide a valid `DuckDB` connection for the duration of this call.
/// unsafe fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
///     let builder = ScalarFunctionBuilder::map1("my_fn", |x: i64| x + 1)?;
///     unsafe { reg.register_typed_scalar(builder) }
/// }
/// ```
pub trait Registrar {
    /// Register a scalar function.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_scalar(&self, builder: ScalarFunctionBuilder) -> Result<(), ExtensionError>;

    /// Register a scalar function built from a Rust closure
    /// ([`ScalarFunctionBuilder::map1`] and friends).
    ///
    /// The default implementation hands the underlying builder to
    /// [`register_scalar`][Self::register_scalar], so existing implementations
    /// (including [`MockRegistrar`][crate::testing::MockRegistrar]) need no
    /// change. An implementation must register that builder as it receives it:
    /// the closure's trampoline refuses to run, with a SQL error, if the
    /// signature was edited on the way.
    ///
    /// # Errors
    ///
    /// Returns whatever [`register_scalar`][Self::register_scalar] returns.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_typed_scalar(
        &self,
        builder: TypedScalarFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: forwarded from this method's own contract.
        unsafe { self.register_scalar(builder.into_inner()) }
    }

    /// Register a scalar function set (multiple overloads under one name).
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_scalar_set(
        &self,
        builder: ScalarFunctionSetBuilder,
    ) -> Result<(), ExtensionError>;

    /// Register an aggregate function.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call,
    /// and the builder's raw callbacks must fit the calls `DuckDB` makes: its
    /// `state_size`, `init`, `destructor` and data callbacks must all describe
    /// the same state (see [`Registrar`]'s "Why every method is `unsafe`").
    unsafe fn register_aggregate(
        &self,
        builder: AggregateFunctionBuilder,
    ) -> Result<(), ExtensionError>;

    /// Register an aggregate function set (multiple overloads under one name).
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call,
    /// and within each overload the raw callbacks must all describe the same
    /// state (see [`Registrar`]'s "Why every method is `unsafe`").
    unsafe fn register_aggregate_set(
        &self,
        builder: AggregateFunctionSetBuilder,
    ) -> Result<(), ExtensionError>;

    /// Register a table function.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_table(&self, builder: TableFunctionBuilder) -> Result<(), ExtensionError>;

    /// Register a `SQL` macro (scalar or table-returning).
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_sql_macro(&self, sql_macro: SqlMacro) -> Result<(), ExtensionError>;

    /// Register a custom type cast function.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    unsafe fn register_cast(&self, builder: CastFunctionBuilder) -> Result<(), ExtensionError>;

    /// Register a custom `COPY TO` function (`DuckDB` 1.5.0+).
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_copy_function(
        &self,
        builder: CopyFunctionBuilder,
    ) -> Result<(), ExtensionError>;

    /// Registers an extension-defined configuration option.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `DuckDB` rejects the option.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid.
    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_config_option(
        &self,
        builder: crate::config_option::ConfigOptionBuilder,
    ) -> Result<(), ExtensionError>;
}

/// Wraps the `duckdb_connection` and `duckdb_database` provided to your
/// extension at load time.
///
/// `Connection` implements [`Registrar`], offering a single, uniform API for
/// registering all extension components. It also exposes
/// [`register_replacement_scan`][Self::register_replacement_scan] and
/// [`register_replacement_scan_with_data`][Self::register_replacement_scan_with_data],
/// which require the `duckdb_database` handle and therefore cannot be part of
/// the `Registrar` trait.
///
/// # Obtaining a `Connection`
///
/// Use [`init_extension_v2`][crate::entry_point::init_extension_v2] (or the
/// [`entry_point_v2!`][crate::entry_point_v2] macro). Both pass a `&Connection`
/// to your registration callback.
///
/// # Version compatibility
///
/// `Connection` provides a uniform API across `DuckDB` 1.4.x and 1.5.x.
/// When future `DuckDB` releases add new C API surface, additional methods will
/// be gated on the corresponding feature flag.
///
/// # What the collision checks cost
///
/// Before the first scalar and the first table function it registers, a
/// `Connection` lists the catalog once each (see the field docs below), so
/// that a registration that would silently replace a built-in, or be dropped,
/// is refused instead. Each listing is a scan of `duckdb_functions()`, which
/// builds a row for **every function and macro in every attached database**
/// before any `WHERE` applies: the table function has no filter or projection
/// pushdown (`src/function/table/system/duckdb_functions.cpp`), so narrowing
/// the query to the names being registered, or to the name column, does not
/// make it cheaper. Measured with the `DuckDB` CLI on a shared 4-vCPU machine:
/// one scan takes about 20 ms on 1.5.6 and 13 ms on 2.0.0 in an empty
/// database, and about 110 ms on both with 20,000 user macros, so a `LOAD`
/// into a database with that many macros spends about 0.2 s more on its two
/// scans. Registering on the raw connection (`register(con)` on a builder)
/// scans once per call instead of once per load.
#[derive(Debug)]
pub struct Connection {
    con: duckdb_connection,
    db: duckdb_database,
    /// The catalog's scalar signatures, listed on the first scalar
    /// registration and kept up to date with this connection's own, so the
    /// collision check costs one catalog scan per extension load rather than
    /// one per function (see [`ScalarFunctionBuilder::register`]).
    ///
    /// SQL run directly on the raw handle during registration — a
    /// `CREATE MACRO`, another extension's `LOAD` — is not seen by it. A macro
    /// name cannot take a scalar signature (`DuckDB` refuses the scalar
    /// instead), so what can go unseen is a scalar function registered by
    /// other code in between, whose collision the check would then miss.
    scalars: core::cell::RefCell<Option<crate::scalar::builder::collision::ExistingScalars>>,
    /// The catalog's table function and table macro names, listed on the first
    /// table function registration and kept up to date with this connection's
    /// own, so the collision check costs one catalog scan per extension load
    /// rather than one per function (see
    /// [`TableFunctionBuilder::register`]).
    ///
    /// It is a one-time copy, so a table function registered on the raw handle
    /// afterwards — another extension's `LOAD`, or a direct call to
    /// [`TableFunctionBuilder::register`] — is not seen by it. A later
    /// `register_table` under that name then passes the check and `DuckDB`
    /// deals with the duplicate itself — dropping it with a success report on
    /// v1.5.x, failing the call with no reason on v1.4.x — which is the failure
    /// the check exists to prevent. The scalar snapshot above has the same gap.
    ///
    /// [`TableFunctionBuilder::register`]: crate::table::TableFunctionBuilder::register
    tables: core::cell::RefCell<Option<crate::table::collision::ExistingTableFunctions>>,
}

impl Connection {
    /// Create a `Connection` from raw `DuckDB` handles.
    ///
    /// [`init_extension_v2`][crate::entry_point::init_extension_v2] makes the
    /// one an extension registers through. This constructor lets other code —
    /// a test of an extension's registration function against a real
    /// database, say — drive the same [`Registrar`] implementation.
    ///
    /// # Safety
    ///
    /// Both `con` and `db` must be valid, non-null handles for the duration of
    /// the `Connection`'s lifetime, and `con` must be a connection to `db`.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(con: duckdb_connection, db: duckdb_database) -> Self {
        Self {
            con,
            db,
            scalars: core::cell::RefCell::new(None),
            tables: core::cell::RefCell::new(None),
        }
    }

    /// Return the raw `duckdb_connection` handle.
    ///
    /// Use this to call C API functions that `quack-rs` does not yet wrap.
    #[inline]
    pub const fn as_raw_connection(&self) -> duckdb_connection {
        self.con
    }

    /// Return the raw `duckdb_database` handle.
    ///
    /// Use this to call C API functions that require the database handle, such
    /// as replacement scan registration or (with `duckdb-1-5`) config option
    /// registration.
    ///
    /// # The handle dies when `LOAD` returns
    ///
    /// `DuckDB` hands the entry point a database handle that belongs to the
    /// load: it is freed when `LOAD` finishes. Use it inside the registration
    /// closure only, and **never store it** (in a `static`, in `extra_info`, in
    /// a function's state). A connection opened from a stored handle later is a
    /// use after free: valgrind reports it, and it fails or segfaults at random
    /// depending on what reuses the memory. To run SQL after loading, open an
    /// [`OwnedConnection`][crate::query::OwnedConnection] during registration
    /// with [`open_connection`][Self::open_connection] and keep that (it holds
    /// its own reference to the database instance), mindful of what keeping
    /// one costs (see there).
    #[inline]
    pub const fn as_raw_database(&self) -> duckdb_database {
        self.db
    }

    /// Runs `sql` on this connection and returns the materialised result.
    ///
    /// Only valid inside your registration closure: the entry point disconnects
    /// this connection once the closure returns. For SQL that must run later,
    /// open an [`OwnedConnection`][crate::query::OwnedConnection] with
    /// [`open_connection`][Self::open_connection] and keep it.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] carrying `DuckDB`'s own message if the
    /// statement fails.
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    pub unsafe fn query(&self, sql: &str) -> Result<crate::query::QueryResult, ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { crate::query::query(self.con, sql) }
    }

    /// Runs `sql` for its side effects, returning the number of rows changed.
    ///
    /// # Errors
    ///
    /// See [`query`][Self::query].
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    pub unsafe fn execute(&self, sql: &str) -> Result<u64, ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { crate::query::execute(self.con, sql) }
    }

    /// Prepares `sql` on this connection.
    ///
    /// Use this rather than interpolating values into SQL text.
    ///
    /// # Errors
    ///
    /// See [`query`][Self::query].
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    pub unsafe fn prepare(
        &self,
        sql: &str,
    ) -> Result<crate::query::PreparedStatement, ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { crate::query::prepare(self.con, sql) }
    }

    /// The tables `sql` reads or writes, without running it; see
    /// [`query::table_names`][crate::query::table_names] (`DuckDB` 1.5.0+).
    ///
    /// # Errors
    ///
    /// See [`query::table_names`][crate::query::table_names].
    ///
    /// # Safety
    ///
    /// The underlying connection must be valid for the duration of this call.
    // One FFI call; covered by tests/ffi_roundtrip/query_metadata.rs.
    #[cfg_attr(test, mutants::skip)]
    #[cfg(feature = "duckdb-1-5")]
    pub unsafe fn table_names(
        &self,
        sql: &str,
        qualified: bool,
    ) -> Result<Vec<String>, ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { crate::query::table_names(self.con, sql, qualified) }
    }

    /// Opens a second, independently owned connection to the same database.
    ///
    /// Unlike the borrowed registration connection, the returned
    /// [`OwnedConnection`][crate::query::OwnedConnection] stays valid after
    /// extension loading finishes — a `duckdb_connection` holds its own
    /// reference to the database instance. Keep one when a callback or a
    /// background thread needs to run SQL.
    ///
    /// # What keeping a connection costs
    ///
    /// The retained connection keeps the **database instance** alive, so
    /// closing the host's own connection does not shut the database down: a
    /// file database is not checkpointed (its `.wal` stays) and its lock is
    /// held until the process exits, which makes a second process fail with
    /// "Could not set lock on file". If the connection is owned by state the
    /// extension registers (a closure captured by a function, say), the
    /// instance also references that function, and the cycle means it is never
    /// freed. A process-wide `static` holding the connection is first-wins:
    /// with several database instances in one process (a Python program that
    /// calls `duckdb.connect()` repeatedly), every instance's functions would
    /// run on the first instance's connection. Prefer opening a connection per
    /// instance, held by that instance's own registered state, and decide
    /// whether the extension really needs to run SQL from a callback at all.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `DuckDB` refuses to open the connection.
    ///
    /// # Safety
    ///
    /// The underlying `duckdb_database` must be valid for the duration of this
    /// call.
    pub unsafe fn open_connection(&self) -> Result<crate::query::OwnedConnection, ExtensionError> {
        // SAFETY: self.db is valid per Connection invariant.
        unsafe { crate::query::OwnedConnection::open(self.db) }
    }

    /// Register a replacement scan backed by a raw function pointer and extra
    /// data.
    ///
    /// For the ergonomic owned-data variant, see
    /// [`register_replacement_scan_with_data`][Self::register_replacement_scan_with_data].
    ///
    /// # Safety
    ///
    /// - The underlying `duckdb_database` must be valid.
    /// - `extra_data` must remain valid until `delete_callback` is called (or
    ///   until the database is closed if `delete_callback` is `None`).
    pub unsafe fn register_replacement_scan(
        &self,
        callback: ReplacementScanFn,
        extra_data: *mut c_void,
        delete_callback: duckdb_delete_callback_t,
    ) {
        // SAFETY: self.db is valid per Connection invariant.
        unsafe {
            ReplacementScanBuilder::register(self.db, callback, extra_data, delete_callback);
        }
    }

    /// Register a replacement scan with owned extra data.
    ///
    /// Boxes `data` and registers a drop destructor automatically. This is the
    /// safe, ergonomic alternative to
    /// [`register_replacement_scan`][Self::register_replacement_scan].
    ///
    /// `T` must be `Send + Sync`; see
    /// [`ReplacementScanBuilder::register_with_data`].
    ///
    /// # Safety
    ///
    /// The underlying `duckdb_database` must be valid.
    pub unsafe fn register_replacement_scan_with_data<T: Send + Sync + 'static>(
        &self,
        callback: ReplacementScanFn,
        data: T,
    ) {
        // SAFETY: self.db is valid per Connection invariant.
        unsafe {
            ReplacementScanBuilder::register_with_data(self.db, callback, data);
        }
    }
}

impl Registrar for Connection {
    unsafe fn register_scalar(&self, builder: ScalarFunctionBuilder) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant; caller upholds builder contract.
        unsafe { builder.register_with(self.con, Some(&self.scalars)) }
    }

    unsafe fn register_typed_scalar(
        &self,
        builder: TypedScalarFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe {
            builder
                .into_inner()
                .register_with(self.con, Some(&self.scalars))
        }
    }

    unsafe fn register_scalar_set(
        &self,
        builder: ScalarFunctionSetBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register_with(self.con, Some(&self.scalars)) }
    }

    unsafe fn register_aggregate(
        &self,
        builder: AggregateFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register(self.con) }
    }

    unsafe fn register_aggregate_set(
        &self,
        builder: AggregateFunctionSetBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register(self.con) }
    }

    unsafe fn register_table(&self, builder: TableFunctionBuilder) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register_with(self.con, Some(&self.tables)) }
    }

    unsafe fn register_sql_macro(&self, sql_macro: SqlMacro) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { sql_macro.register(self.con) }
    }

    unsafe fn register_cast(&self, builder: CastFunctionBuilder) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register(self.con) }
    }

    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_copy_function(
        &self,
        builder: CopyFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register(self.con) }
    }

    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_config_option(
        &self,
        builder: crate::config_option::ConfigOptionBuilder,
    ) -> Result<(), ExtensionError> {
        // SAFETY: self.con is valid per Connection invariant.
        unsafe { builder.register(self.con) }
    }
}
