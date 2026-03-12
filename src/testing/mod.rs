// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Test utilities for `DuckDB` extension development.
//!
//! This module provides several complementary tools for testing extension code
//! without spinning up a live `DuckDB` process:
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`AggregateTestHarness`] | Test aggregate state update/combine/finalize logic |
//! | [`MockVectorWriter`] | Write values to an in-memory buffer (replaces real output vector) |
//! | [`MockVectorReader`] | Read values from an in-memory buffer (replaces real input vector) |
//! | [`MockRegistrar`] | Verify which functions are registered, without a `DuckDB` connection |
//! | [`InMemoryDb`] | Open a real bundled `DuckDB` for SQL-level tests (`bundled-test` feature) |
//!
//! # Architectural limitation: `loadable-extension` dispatch
//!
//! `DuckDB` loadable extensions use `libduckdb-sys` with
//! `features = ["loadable-extension"]`. This routes every `DuckDB` C API call
//! through a lazy dispatch table (a global `AtomicPtr` per function). The table
//! is normally populated when `DuckDB` calls `duckdb_rs_extension_api_init` at
//! extension-load time.
//!
//! **In `cargo test`, no `DuckDB` process loads the extension.** However,
//! [`InMemoryDb`] works around this automatically: `InMemoryDb::open()` initialises
//! the dispatch table from the bundled `DuckDB` symbols before opening a connection
//! (see Pitfall P9 in `LESSONS.md` for the full explanation).
//!
//! The dispatch table is **not** initialised for quack-rs's own FFI wrappers.
//! Any direct `libduckdb-sys` call that is *not* mediated by [`InMemoryDb`] will
//! still panic with:
//!
//! ```text
//! DuckDB API not initialized
//! ```
//!
//! This affects:
//!
//! - `VectorReader::new` and `VectorWriter::new` — both call `duckdb_vector_get_data`
//! - `Connection::register_*` — calls registration C API functions
//! - `LogicalType::new` — calls `duckdb_create_logical_type`; `LogicalType::drop` calls
//!   `duckdb_destroy_logical_type`
//! - Any other code that touches `libduckdb-sys` symbols directly
//!
//! # What CAN be tested with `cargo test`
//!
//! - **Aggregate state logic** — use [`AggregateTestHarness`]
//! - **Callback output logic** — extract into pure Rust, test with [`MockVectorWriter`] / [`MockVectorReader`]
//! - **Registration structure** — use [`MockRegistrar`] (builders with only [`TypeId`][crate::types::TypeId] parameters)
//! - **SQL macro SQL generation** — [`SqlMacro::to_sql()`][crate::sql_macro::SqlMacro::to_sql] is pure Rust
//! - **Interval conversions** — [`interval_to_micros`][crate::interval::interval_to_micros] is pure Rust
//! - **Validation / scaffold** — [`validate`][crate::validate] and [`scaffold`][crate::scaffold] are pure Rust
//! - **SQL-level results** — use `InMemoryDb` (requires `bundled-test` Cargo feature)
//!
//! # What requires E2E tests (`SQLLogicTest`)
//!
//! - FFI wiring correctness (entry point, callback signatures)
//! - Function registration success (Pitfall P6: registration can fail silently)
//! - NULL handling through real `DuckDB` NULL propagation
//! - Multi-group aggregation results
//! - Extension loading without crash
//!
//! See the [testing guide](https://quack-rs.com/testing) and `LESSONS.md` Pitfall P3 for details.
//!
//! # Why you need both unit tests AND E2E tests
//!
//! **Unit tests (this module)** verify that your `MyState::update` and
//! `MyState::combine` methods produce correct results. They run fast and
//! catch logical bugs.
//!
//! **E2E tests (`DuckDB` CLI / ``SQLLogicTest``)** verify that the FFI wiring is
//! correct — that `state_size`, `state_init`, `state_destroy`, and the
//! callback signatures match what `DuckDB` expects.
//!
//! In duckdb-behavioral, 435 unit tests passed while the extension was completely
//! broken due to three bugs that only E2E tests can catch:
//! - SEGFAULT on load (wrong entry point)
//! - 6 of 7 functions failing silently (function set name not set on each member)
//! - Window funnel returning wrong results (combine not propagating config)
//!
//! **Unit tests alone are insufficient. Always run E2E tests.**
//!
//! # Example: testing aggregate logic
//!
//! ```rust
//! use quack_rs::testing::AggregateTestHarness;
//! use quack_rs::aggregate::AggregateState;
//!
//! #[derive(Default, Debug, PartialEq)]
//! struct SumState { total: i64 }
//! impl AggregateState for SumState {}
//!
//! impl SumState {
//!     fn update(&mut self, value: i64) {
//!         self.total += value;
//!     }
//! }
//!
//! let mut harness = AggregateTestHarness::<SumState>::new();
//! harness.update(|s| s.update(10));
//! harness.update(|s| s.update(20));
//! harness.update(|s| s.update(5));
//!
//! let state = harness.finalize();
//! assert_eq!(state.total, 35);
//! ```
//!
//! # Example: testing callback output logic with mocks
//!
//! ```rust
//! use quack_rs::testing::{MockVectorReader, MockVectorWriter};
//!
//! // Extract pure logic from the FFI callback into a testable function.
//! fn double_values(reader: &MockVectorReader, writer: &mut MockVectorWriter) {
//!     for i in 0..reader.row_count() {
//!         if reader.is_valid(i) {
//!             let v = reader.try_get_i64(i).unwrap_or(0);
//!             writer.write_i64(i, v * 2);
//!         } else {
//!             writer.set_null(i);
//!         }
//!     }
//! }
//!
//! let reader = MockVectorReader::from_i64s([Some(1), None, Some(5)]);
//! let mut writer = MockVectorWriter::new(3);
//! double_values(&reader, &mut writer);
//!
//! assert_eq!(writer.try_get_i64(0), Some(2));
//! assert!(writer.is_null(1));
//! assert_eq!(writer.try_get_i64(2), Some(10));
//! ```
//!
//! # Example: testing registration with `MockRegistrar`
//!
//! ```rust
//! use quack_rs::connection::Registrar;
//! use quack_rs::testing::MockRegistrar;
//! use quack_rs::scalar::ScalarFunctionBuilder;
//! use quack_rs::types::TypeId;
//! use quack_rs::error::ExtensionError;
//!
//! fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
//!     let f = ScalarFunctionBuilder::new("my_fn")
//!         .param(TypeId::BigInt)
//!         .returns(TypeId::BigInt);
//!     unsafe { reg.register_scalar(f) }
//! }
//!
//! let mock = MockRegistrar::new();
//! register_all(&mock).unwrap();
//! assert!(mock.has_scalar("my_fn"));
//! ```

pub mod harness;
pub mod mock_registrar;
pub mod mock_vector;

#[cfg(feature = "bundled-test")]
pub mod in_memory_db;

pub use harness::AggregateTestHarness;
pub use mock_registrar::{CastRecord, MockRegistrar};
pub use mock_vector::{MockDuckValue, MockVectorReader, MockVectorWriter};

#[cfg(feature = "bundled-test")]
pub use in_memory_db::InMemoryDb;
