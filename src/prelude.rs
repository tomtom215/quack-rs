// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Convenience re-exports for the most commonly used `quack-rs` items.
//!
//! This prelude covers the types and functions needed in the `src/lib.rs`
//! of a typical `DuckDB` Rust extension. Import it with:
//!
//! ```rust,no_run
//! use quack_rs::prelude::*;
//! ```
//!
//! # What is included
//!
//! | Item | From |
//! |------|------|
//! | [`init_extension`] | `entry_point` module |
//! | `entry_point!` | `entry_point` module (macro) |
//! | [`AggregateFunctionBuilder`] | `aggregate` module |
//! | [`AggregateFunctionSetBuilder`] | `aggregate` module |
//! | [`AggregateState`] | `aggregate` module |
//! | [`FfiState`] | `aggregate` module |
//! | [`ScalarFunctionBuilder`] | `scalar` module |
//! | [`SqlMacro`] | `sql_macro` module |
//! | [`VectorReader`] | `vector` module |
//! | [`VectorWriter`] | `vector` module |
//! | [`TypeId`] | `types` module |
//! | [`LogicalType`] | `types` module |
//! | [`DuckInterval`] | `interval` module |
//! | [`interval_to_micros`] | `interval` module |
//! | [`ExtensionError`] | `error` module |
//! | [`ExtResult`] | `error` module |
//! | [`DUCKDB_API_VERSION`] | crate root |
//!
//! # What is NOT included
//!
//! The following items are intentionally excluded from the prelude because they
//! are used less frequently and benefit from explicit import paths:
//!
//! - `validate::*` — validation utilities (use explicitly to make intent clear)
//! - `scaffold::*` — project generation (use explicitly)
//! - `testing::*` — test harness (typically imported only in `#[cfg(test)]`)
//! - `interval::read_interval_at` — low-level; use [`VectorReader::read_interval`] instead
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::prelude::*;
//!
//! // Your state struct
//! #[derive(Default)]
//! struct MyState { count: i64 }
//! impl AggregateState for MyState {}
//!
//! // Registration (called from your entry point)
//! fn register(con: libduckdb_sys::duckdb_connection) -> ExtResult<()> {
//!     let _ = AggregateFunctionBuilder::try_new("my_count")?
//!         .param(TypeId::BigInt)
//!         .returns(TypeId::BigInt)
//!         .state_size(FfiState::<MyState>::size_callback)
//!         .init(FfiState::<MyState>::init_callback)
//!         // ... callbacks ...
//!         ;
//!     Ok(())
//! }
//! ```

// Entry point
pub use crate::entry_point::init_extension;

// Aggregate functions
pub use crate::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateState, FfiState,
};

// Scalar functions
pub use crate::scalar::ScalarFunctionBuilder;

// SQL macros
pub use crate::sql_macro::SqlMacro;

// Vector I/O
pub use crate::vector::{VectorReader, VectorWriter};

// Types
pub use crate::types::{LogicalType, TypeId};

// Interval
pub use crate::interval::{interval_to_micros, DuckInterval};

// Error
pub use crate::error::{ExtResult, ExtensionError};

// API version constant
pub use crate::DUCKDB_API_VERSION;

// Re-export the entry_point! macro (already exported at crate root via #[macro_export])
pub use crate::entry_point;
