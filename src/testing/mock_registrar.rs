// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`MockRegistrar`] — a [`Registrar`] implementation for testing.
//!
//! `MockRegistrar` records which functions were registered without calling any
//! `DuckDB` C API. Use it to unit-test your registration logic — verifying that
//! the right functions are registered with the right names — without a live
//! `DuckDB` instance.
//!
//! It refuses, with the same error, a builder the real registration refuses
//! before its first `DuckDB` call: a missing return type or callback, a
//! function set with no overloads, a copy function that implements neither
//! direction, a config option without a type or default, a composite or
//! literal [`TypeId`] in any slot, an `ANY` return type. Checks that need
//! `DuckDB` are not run: a name or signature collision, a type the running
//! `DuckDB` lacks (`TIME_NS` before 1.5.0), a default that does not convert
//! to its config option's type.
//!
//! # Limitation: builders with `LogicalType` fields
//!
//! Builders that contain [`LogicalType`] values (e.g.,
//! created with `.returns_logical(...)` or `.param_logical(...)`) cannot be used
//! with `MockRegistrar` in `loadable-extension` test mode. `LogicalType`'s `Drop`
//! implementation calls `duckdb_destroy_logical_type`, which panics when the
//! `DuckDB` dispatch table is uninitialized.
//!
//! Stick to [`TypeId`]-based parameter and return types
//! when building functions for use with `MockRegistrar`.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::connection::Registrar;
//! use quack_rs::testing::MockRegistrar;
//! use quack_rs::scalar::ScalarFunctionBuilder;
//! use quack_rs::aggregate::AggregateFunctionBuilder;
//! use quack_rs::types::TypeId;
//! use quack_rs::error::ExtensionError;
//! use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};
//!
//! unsafe extern "C" fn word_count(
//!     _info: duckdb_function_info,
//!     _input: duckdb_data_chunk,
//!     _output: duckdb_vector,
//! ) {
//!     // ...
//! }
//!
//! fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
//!     let scalar = ScalarFunctionBuilder::new("word_count")
//!         .param(TypeId::Varchar)
//!         .returns(TypeId::BigInt)
//!         .function(word_count);
//!     unsafe { reg.register_scalar(scalar) }
//! }
//!
//! let mock = MockRegistrar::new();
//! register_all(&mock).unwrap();
//! assert!(mock.has_scalar("word_count"));
//! assert_eq!(mock.total_registrations(), 1);
//! ```

use std::cell::RefCell;

use crate::aggregate::{AggregateFunctionBuilder, AggregateFunctionSetBuilder};
use crate::cast::CastFunctionBuilder;
use crate::connection::Registrar;
use crate::error::ExtensionError;
use crate::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder};
use crate::sql_macro::SqlMacro;
use crate::table::TableFunctionBuilder;
use crate::types::{LogicalType, TypeId};

/// A record of a single cast function registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastRecord {
    /// The source type being cast from (if set via simple `TypeId`).
    pub source: Option<TypeId>,
    /// The target type being cast to (if set via simple `TypeId`).
    pub target: Option<TypeId>,
}

/// An in-memory mock implementation of [`Registrar`] for unit testing.
///
/// All `register_*` methods succeed silently (returning `Ok(())`) and record
/// the function name (or types for casts). No `DuckDB` C API is called.
///
/// # Thread safety
///
/// `MockRegistrar` uses `RefCell` for interior mutability and is **not** `Sync`.
/// Call it from a single thread within your tests.
#[derive(Debug, Default)]
pub struct MockRegistrar {
    scalar_names: RefCell<Vec<String>>,
    scalar_set_names: RefCell<Vec<String>>,
    aggregate_names: RefCell<Vec<String>>,
    aggregate_set_names: RefCell<Vec<String>>,
    table_names: RefCell<Vec<String>>,
    sql_macro_names: RefCell<Vec<String>>,
    casts: RefCell<Vec<CastRecord>>,
    #[cfg(feature = "duckdb-1-5")]
    copy_function_names: RefCell<Vec<String>>,
    #[cfg(feature = "duckdb-1-5")]
    config_option_names: RefCell<Vec<String>>,
}

impl MockRegistrar {
    /// Creates a new, empty `MockRegistrar`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ── Inspection ──────────────────────────────────────────────────────────

    /// Returns the names of all scalar functions registered so far.
    #[must_use]
    pub fn scalar_names(&self) -> Vec<String> {
        self.scalar_names.borrow().clone()
    }

    /// Returns the names of all scalar function sets registered so far.
    #[must_use]
    pub fn scalar_set_names(&self) -> Vec<String> {
        self.scalar_set_names.borrow().clone()
    }

    /// Returns the names of all aggregate functions registered so far.
    #[must_use]
    pub fn aggregate_names(&self) -> Vec<String> {
        self.aggregate_names.borrow().clone()
    }

    /// Returns the names of all aggregate function sets registered so far.
    #[must_use]
    pub fn aggregate_set_names(&self) -> Vec<String> {
        self.aggregate_set_names.borrow().clone()
    }

    /// Returns the names of all table functions registered so far.
    #[must_use]
    pub fn table_names(&self) -> Vec<String> {
        self.table_names.borrow().clone()
    }

    /// Returns the names of all SQL macros registered so far.
    #[must_use]
    pub fn sql_macro_names(&self) -> Vec<String> {
        self.sql_macro_names.borrow().clone()
    }

    /// Returns all cast registrations recorded so far.
    #[must_use]
    pub fn casts(&self) -> Vec<CastRecord> {
        self.casts.borrow().clone()
    }

    /// Returns the names of all copy functions registered so far.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn copy_function_names(&self) -> Vec<String> {
        self.copy_function_names.borrow().clone()
    }

    /// Returns `true` if a copy function with the given name was registered.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn has_copy_function(&self, name: &str) -> bool {
        self.copy_function_names.borrow().iter().any(|n| n == name)
    }

    /// Returns the names of all config options registered so far.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn config_option_names(&self) -> Vec<String> {
        self.config_option_names.borrow().clone()
    }

    /// Returns `true` if a config option with the given name was registered.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn has_config_option(&self, name: &str) -> bool {
        self.config_option_names.borrow().iter().any(|n| n == name)
    }

    /// Returns the total number of registrations across all types.
    #[must_use]
    pub fn total_registrations(&self) -> usize {
        let base = self.scalar_names.borrow().len()
            + self.scalar_set_names.borrow().len()
            + self.aggregate_names.borrow().len()
            + self.aggregate_set_names.borrow().len()
            + self.table_names.borrow().len()
            + self.sql_macro_names.borrow().len()
            + self.casts.borrow().len();
        #[cfg(feature = "duckdb-1-5")]
        {
            base + self.copy_function_names.borrow().len() + self.config_option_names.borrow().len()
        }
        #[cfg(not(feature = "duckdb-1-5"))]
        {
            base
        }
    }

    // ── Convenience predicates ──────────────────────────────────────────────

    /// Returns `true` if a scalar function with the given name was registered.
    #[must_use]
    pub fn has_scalar(&self, name: &str) -> bool {
        self.scalar_names.borrow().iter().any(|n| n == name)
    }

    /// Returns `true` if a scalar function set with the given name was registered.
    #[must_use]
    pub fn has_scalar_set(&self, name: &str) -> bool {
        self.scalar_set_names.borrow().iter().any(|n| n == name)
    }

    /// Returns `true` if an aggregate function with the given name was registered.
    #[must_use]
    pub fn has_aggregate(&self, name: &str) -> bool {
        self.aggregate_names.borrow().iter().any(|n| n == name)
    }

    /// Returns `true` if an aggregate function set with the given name was registered.
    #[must_use]
    pub fn has_aggregate_set(&self, name: &str) -> bool {
        self.aggregate_set_names.borrow().iter().any(|n| n == name)
    }

    /// Returns `true` if a table function with the given name was registered.
    #[must_use]
    pub fn has_table(&self, name: &str) -> bool {
        self.table_names.borrow().iter().any(|n| n == name)
    }

    /// Returns `true` if a SQL macro with the given name was registered.
    #[must_use]
    pub fn has_sql_macro(&self, name: &str) -> bool {
        self.sql_macro_names.borrow().iter().any(|n| n == name)
    }
}

impl Registrar for MockRegistrar {
    /// Records a scalar function registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context — no `DuckDB`
    /// connection is required.
    unsafe fn register_scalar(&self, builder: ScalarFunctionBuilder) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.scalar_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    /// Records a scalar function set registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_scalar_set(
        &self,
        builder: ScalarFunctionSetBuilder,
    ) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.scalar_set_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    /// Records an aggregate function registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_aggregate(
        &self,
        builder: AggregateFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.aggregate_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    /// Records an aggregate function set registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_aggregate_set(
        &self,
        builder: AggregateFunctionSetBuilder,
    ) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.aggregate_set_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    /// Records a table function registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_table(&self, builder: TableFunctionBuilder) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.table_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    /// Records a SQL macro registration. Never calls `DuckDB` C API.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_sql_macro(&self, sql_macro: SqlMacro) -> Result<(), ExtensionError> {
        self.sql_macro_names
            .borrow_mut()
            .push(sql_macro.name().to_owned());
        Ok(())
    }

    /// Records a cast function registration. Never calls `DuckDB` C API.
    ///
    /// The source and target types are captured from the builder.
    ///
    /// # Safety
    ///
    /// This implementation is safe to call in any context.
    unsafe fn register_cast(&self, builder: CastFunctionBuilder) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.casts.borrow_mut().push(CastRecord {
            source: builder.source(),
            target: builder.target(),
        });
        Ok(())
    }

    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_copy_function(
        &self,
        builder: crate::copy_function::CopyFunctionBuilder,
    ) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        self.copy_function_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }

    #[cfg(feature = "duckdb-1-5")]
    unsafe fn register_config_option(
        &self,
        builder: crate::config_option::ConfigOptionBuilder,
    ) -> Result<(), ExtensionError> {
        builder.check_parts()?;
        builder.check_types(LogicalType::check_slot_offline)?;
        self.config_option_names
            .borrow_mut()
            .push(builder.name().to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeId;
    use libduckdb_sys::{
        duckdb_aggregate_state, duckdb_bind_info, duckdb_data_chunk, duckdb_function_info,
        duckdb_init_info, duckdb_vector, idx_t,
    };

    // Callbacks for complete builders. Never invoked: the mock only records.
    unsafe extern "C" fn scalar_fn(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: duckdb_vector,
    ) {
    }
    unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t {
        0
    }
    unsafe extern "C" fn state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
    unsafe extern "C" fn update(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: *mut duckdb_aggregate_state,
    ) {
    }
    unsafe extern "C" fn combine(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: *mut duckdb_aggregate_state,
        _: idx_t,
    ) {
    }
    unsafe extern "C" fn finalize(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: duckdb_vector,
        _: idx_t,
        _: idx_t,
    ) {
    }
    unsafe extern "C" fn table_bind(_: duckdb_bind_info) {}
    unsafe extern "C" fn table_init(_: duckdb_init_info) {}
    unsafe extern "C" fn table_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}
    unsafe extern "C" fn cast_fn(
        _: duckdb_function_info,
        _: idx_t,
        _: duckdb_vector,
        _: duckdb_vector,
    ) -> bool {
        true
    }

    fn scalar(name: &str) -> ScalarFunctionBuilder {
        ScalarFunctionBuilder::new(name)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(scalar_fn)
    }

    fn aggregate(name: &str) -> AggregateFunctionBuilder {
        AggregateFunctionBuilder::new(name)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(state_size)
            .init(state_init)
            .update(update)
            .combine(combine)
            .finalize(finalize)
    }

    fn table(name: &str) -> TableFunctionBuilder {
        TableFunctionBuilder::new(name)
            .bind(table_bind)
            .init(table_init)
            .scan(table_scan)
    }

    fn cast() -> CastFunctionBuilder {
        CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer).function(cast_fn)
    }

    #[test]
    fn mock_registrar_records_scalar() {
        let mock = MockRegistrar::new();
        unsafe { mock.register_scalar(scalar("my_fn")).unwrap() };
        assert!(mock.has_scalar("my_fn"));
        assert_eq!(mock.scalar_names(), vec!["my_fn"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_aggregate() {
        let mock = MockRegistrar::new();
        unsafe { mock.register_aggregate(aggregate("my_agg")).unwrap() };
        assert!(mock.has_aggregate("my_agg"));
        assert_eq!(mock.aggregate_names(), vec!["my_agg"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_scalar_set() {
        let mock = MockRegistrar::new();
        let builder = crate::scalar::ScalarFunctionSetBuilder::new("my_set").overload(
            crate::scalar::ScalarOverloadBuilder::new()
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .function(scalar_fn),
        );
        unsafe { mock.register_scalar_set(builder).unwrap() };
        assert!(mock.has_scalar_set("my_set"));
        assert!(!mock.has_scalar_set("other"));
        assert_eq!(mock.scalar_set_names(), vec!["my_set"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_aggregate_set() {
        let mock = MockRegistrar::new();
        let builder = AggregateFunctionSetBuilder::new("my_agg_set").overload(
            crate::aggregate::AggregateOverloadBuilder::new()
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .state_size(state_size)
                .init(state_init)
                .update(update)
                .combine(combine)
                .finalize(finalize),
        );
        unsafe { mock.register_aggregate_set(builder).unwrap() };
        assert!(mock.has_aggregate_set("my_agg_set"));
        assert_eq!(mock.aggregate_set_names(), vec!["my_agg_set"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_table() {
        let mock = MockRegistrar::new();
        unsafe { mock.register_table(table("my_table")).unwrap() };
        assert!(mock.has_table("my_table"));
        assert_eq!(mock.table_names(), vec!["my_table"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_sql_macro() {
        let mock = MockRegistrar::new();
        let macro_ = SqlMacro::scalar("my_macro", &["x"], "x + 1").unwrap();
        unsafe { mock.register_sql_macro(macro_).unwrap() };
        assert!(mock.has_sql_macro("my_macro"));
        assert_eq!(mock.sql_macro_names(), vec!["my_macro"]);
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_records_cast() {
        let mock = MockRegistrar::new();
        unsafe { mock.register_cast(cast()).unwrap() };
        let casts = mock.casts();
        assert_eq!(casts.len(), 1);
        assert_eq!(casts[0].source, Some(TypeId::Varchar));
        assert_eq!(casts[0].target, Some(TypeId::Integer));
        assert_eq!(mock.total_registrations(), 1);
    }

    #[test]
    fn mock_registrar_multiple_registrations() {
        let mock = MockRegistrar::new();

        let s1 = scalar("fn_one");
        let s2 = scalar("fn_two");

        unsafe {
            mock.register_scalar(s1).unwrap();
            mock.register_scalar(s2).unwrap();
        }

        assert_eq!(mock.total_registrations(), 2);
        assert!(mock.has_scalar("fn_one"));
        assert!(mock.has_scalar("fn_two"));
        assert!(!mock.has_scalar("fn_three"));
    }

    #[test]
    #[cfg(feature = "duckdb-1-5")]
    fn mock_registrar_records_copy_function() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_copy_function("my_format"));
        assert_eq!(mock.copy_function_names(), [] as [String; 0]);

        let builder = crate::copy_function::CopyFunctionBuilder::try_new("my_format")
            .unwrap()
            .bind(copy_bind)
            .sink(copy_sink)
            .finalize(copy_finalize);
        unsafe { mock.register_copy_function(builder).unwrap() };

        assert!(mock.has_copy_function("my_format"));
        assert!(!mock.has_copy_function("other_format"));
        assert_eq!(mock.copy_function_names(), vec!["my_format"]);

        // Config options go through the same trait, so an extension's whole
        // registration closure is mockable rather than only part of it.
        assert!(!mock.has_config_option("my_option"));
        assert_eq!(mock.config_option_names(), [] as [String; 0]);

        let option = crate::config_option::ConfigOptionBuilder::try_new("my_option")
            .expect("name")
            .option_type(TypeId::Varchar)
            .default_value("x")
            .expect("default");
        // SAFETY: the mock ignores the connection entirely.
        unsafe { mock.register_config_option(option) }.expect("register");
        assert!(mock.has_config_option("my_option"));
        // A name that was never registered must not match: without this, a
        // `has_config_option` that always returns `true` passes every other
        // assertion here. (Surviving mutant reported by the incremental
        // mutation gate; mirrors the `has_copy_function` coverage above.)
        assert!(!mock.has_config_option("other_option"));
        assert_eq!(mock.config_option_names(), vec!["my_option"]);
        assert_eq!(mock.copy_function_names().len(), 1);
        assert_eq!(mock.total_registrations(), 2);
    }

    /// Registers one scalar **and** one copy function so that
    /// `total_registrations` must add (not subtract) the copy-function count.
    /// With the `+ with -` mutation, `base(1) - copy_len(1) = 0 ≠ 2`.
    #[test]
    #[cfg(feature = "duckdb-1-5")]
    fn mock_registrar_total_registrations_scalar_plus_copy_function() {
        let mock = MockRegistrar::new();

        let scalar = scalar("my_scalar");
        let copy_fn = crate::copy_function::CopyFunctionBuilder::try_new("my_format")
            .unwrap()
            .bind(copy_bind)
            .sink(copy_sink)
            .finalize(copy_finalize);

        unsafe {
            mock.register_scalar(scalar).unwrap();
            mock.register_copy_function(copy_fn).unwrap();
        }

        assert_eq!(mock.total_registrations(), 2);
        assert!(mock.has_scalar("my_scalar"));
        assert!(mock.has_copy_function("my_format"));
    }

    #[test]
    fn mock_registrar_has_aggregate_false_when_empty() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_aggregate("x"));
    }

    #[test]
    fn mock_registrar_has_aggregate_set_false_when_empty() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_aggregate_set("x"));
    }

    #[test]
    fn mock_registrar_has_table_false_when_empty() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_table("x"));
    }

    #[test]
    fn mock_registrar_has_sql_macro_false_when_empty() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_sql_macro("x"));
    }

    #[test]
    fn mock_registrar_has_scalar_false_when_empty() {
        let mock = MockRegistrar::new();
        assert!(!mock.has_scalar("x"));
    }

    #[test]
    fn mock_registrar_empty_total_registrations() {
        let mock = MockRegistrar::new();
        assert_eq!(mock.total_registrations(), 0);
    }

    #[test]
    fn mock_registrar_total_registrations_counts_all_types() {
        let mock = MockRegistrar::new();

        let scalar = scalar("sc");
        let agg = aggregate("ag");
        let table = table("tb");
        let macro_ = SqlMacro::scalar("mc", &["x"], "x + 1").unwrap();
        let cast = cast();

        unsafe {
            mock.register_scalar(scalar).unwrap();
            mock.register_aggregate(agg).unwrap();
            mock.register_table(table).unwrap();
            mock.register_sql_macro(macro_).unwrap();
            mock.register_cast(cast).unwrap();
        }

        assert_eq!(mock.total_registrations(), 5);
    }

    #[test]
    fn mock_registrar_used_with_generic_registrar() {
        // Demonstrates using MockRegistrar where &impl Registrar is expected.
        fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
            unsafe { reg.register_scalar(scalar("compute")) }
        }

        let mock = MockRegistrar::new();
        register_all(&mock).unwrap();
        assert!(mock.has_scalar("compute"));
    }

    #[cfg(feature = "duckdb-1-5")]
    unsafe extern "C" fn copy_bind(_: libduckdb_sys::duckdb_copy_function_bind_info) {}
    #[cfg(feature = "duckdb-1-5")]
    unsafe extern "C" fn copy_sink(
        _: libduckdb_sys::duckdb_copy_function_sink_info,
        _: duckdb_data_chunk,
    ) {
    }
    #[cfg(feature = "duckdb-1-5")]
    unsafe extern "C" fn copy_finalize(_: libduckdb_sys::duckdb_copy_function_finalize_info) {}

    /// The mock used to record any builder, so a test could pass with a
    /// registration the real connection refuses at `LOAD` — the module's own
    /// doc example registered a scalar with no function callback. It now
    /// runs the same checks the real registration runs before its first
    /// `DuckDB` call, with the same messages, and records nothing on failure.
    #[test]
    fn the_mock_refuses_what_registration_refuses() {
        let mock = MockRegistrar::new();
        let refuse = |result: Result<(), ExtensionError>, want: &str| {
            let err = result.expect_err(want);
            assert!(err.as_str().contains(want), "{want}: {err}");
        };
        // SAFETY (every call): the mock ignores the connection entirely.
        unsafe {
            refuse(
                mock.register_scalar(ScalarFunctionBuilder::new("f").returns(TypeId::BigInt)),
                "function callback not set",
            );
            refuse(
                mock.register_scalar(ScalarFunctionBuilder::new("f").function(scalar_fn)),
                "return type not set",
            );
            refuse(
                mock.register_scalar_set(crate::scalar::ScalarFunctionSetBuilder::new("s")),
                "no overloads",
            );
            refuse(
                mock.register_scalar_set(
                    crate::scalar::ScalarFunctionSetBuilder::new("s").overload(
                        crate::scalar::ScalarOverloadBuilder::new().returns(TypeId::BigInt),
                    ),
                ),
                "overload 0 has no function callback",
            );
            refuse(
                mock.register_aggregate(
                    AggregateFunctionBuilder::new("a")
                        .returns(TypeId::BigInt)
                        .state_size(state_size)
                        .init(state_init)
                        .update(update)
                        .combine(combine),
                ),
                "finalize callback not set",
            );
            refuse(
                mock.register_aggregate_set(AggregateFunctionSetBuilder::new("as")),
                "no overloads",
            );
            refuse(
                mock.register_table(
                    TableFunctionBuilder::new("t")
                        .bind(table_bind)
                        .init(table_init),
                ),
                "scan callback not set",
            );
            refuse(
                mock.register_cast(CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)),
                "cast function callback not set",
            );
        }
        assert_eq!(mock.total_registrations(), 0, "nothing refused is recorded");
    }

    /// The mock ran only the missing-part checks, so it recorded builders
    /// whose types the real registration refuses before its first `DuckDB`
    /// call: a composite or literal `TypeId` in any slot, an `ANY` return.
    #[test]
    fn the_mock_refuses_the_types_registration_refuses() {
        let mock = MockRegistrar::new();
        let refuse = |result: Result<(), ExtensionError>, want: &str| {
            let err = result.expect_err(want);
            assert!(err.as_str().contains(want), "{want}: {err}");
        };
        // SAFETY (every call): the mock ignores the connection entirely.
        unsafe {
            refuse(
                mock.register_scalar(scalar("f").param(TypeId::Struct)),
                "scalar function parameter 1: ",
            );
            refuse(
                mock.register_scalar(scalar("f").varargs(TypeId::IntegerLiteral)),
                "scalar function varargs: ",
            );
            refuse(
                mock.register_scalar(scalar("f").returns(TypeId::Any)),
                "scalar function return type must not be or contain ANY",
            );
            refuse(
                mock.register_scalar_set(
                    crate::scalar::ScalarFunctionSetBuilder::new("s").overload(
                        crate::scalar::ScalarOverloadBuilder::new()
                            .param(TypeId::List)
                            .returns(TypeId::BigInt)
                            .function(scalar_fn),
                    ),
                ),
                "overload 0 parameter 0: ",
            );
            refuse(
                mock.register_aggregate(aggregate("a").returns(TypeId::StringLiteral)),
                "aggregate function return type: ",
            );
            refuse(
                mock.register_aggregate_set(
                    AggregateFunctionSetBuilder::new("as")
                        .returns(TypeId::Any)
                        .overload(
                            crate::aggregate::AggregateOverloadBuilder::new()
                                .param(TypeId::BigInt)
                                .state_size(state_size)
                                .init(state_init)
                                .update(update)
                                .combine(combine)
                                .finalize(finalize),
                        ),
                ),
                "overload 0 return type must not be or contain ANY",
            );
            refuse(
                mock.register_table(table("t").param(TypeId::Map)),
                "table function parameter 0: ",
            );
            refuse(
                mock.register_table(table("t").named_param("n", TypeId::Union)),
                "table function named parameter n: ",
            );
            refuse(
                mock.register_cast(
                    CastFunctionBuilder::new(TypeId::Struct, TypeId::Integer).function(cast_fn),
                ),
                "cast function source type: ",
            );
        }
        assert_eq!(mock.total_registrations(), 0, "nothing refused is recorded");
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn the_mock_refuses_incomplete_copy_functions_and_config_options() {
        let mock = MockRegistrar::new();
        let copy = crate::copy_function::CopyFunctionBuilder::try_new("fmt").unwrap();
        // SAFETY: the mock ignores the connection entirely.
        let err = unsafe { mock.register_copy_function(copy) }.expect_err("nothing set");
        assert!(err.as_str().contains("implements nothing"), "{err}");
        let option = crate::config_option::ConfigOptionBuilder::try_new("opt")
            .expect("name")
            .option_type(TypeId::Varchar);
        // SAFETY: as above.
        let err = unsafe { mock.register_config_option(option) }.expect_err("no default");
        assert!(err.as_str().contains("has no default value"), "{err}");
        let option = crate::config_option::ConfigOptionBuilder::try_new("opt")
            .expect("name")
            .option_type(TypeId::Struct)
            .default_value("x")
            .expect("default");
        // SAFETY: as above.
        let err = unsafe { mock.register_config_option(option) }.expect_err("composite type");
        assert!(err.as_str().starts_with("config option type: "), "{err}");
        assert_eq!(mock.total_registrations(), 0);
    }
}
