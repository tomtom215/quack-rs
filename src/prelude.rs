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
//! | [`init_extension_v2`] | `entry_point` module |
//! | `entry_point!` | `entry_point` module (macro) |
//! | `entry_point_v2!` | `entry_point` module (macro) |
//! | [`Connection`] | `connection` module |
//! | [`Registrar`] | `connection` module |
//! | [`CastFn`] | `cast` module |
//! | [`CastFunctionBuilder`] | `cast` module |
//! | [`CastFunctionInfo`] | `cast` module |
//! | [`CastMode`] | `cast` module |
//! | [`AggregateFunctionBuilder`] | `aggregate` module |
//! | [`AggregateFunctionInfo`] | `aggregate` module |
//! | [`AggregateFunctionSetBuilder`] | `aggregate` module |
//! | [`AggregateState`] | `aggregate` module |
//! | [`FfiState`] | `aggregate` module |
//! | [`ScalarFunctionBuilder`] | `scalar` module |
//! | [`ScalarFunctionInfo`] | `scalar` module |
//! | [`ScalarFunctionSetBuilder`] | `scalar` module |
//! | [`ScalarOverloadBuilder`] | `scalar` module |
//! | [`TableFunctionBuilder`] | `table` module |
//! | [`BindInfo`] | `table` module |
//! | [`InitInfo`] | `table` module |
//! | [`FunctionInfo`] | `table` module |
//! | [`FfiBindData`] | `table` module |
//! | [`FfiInitData`] | `table` module |
//! | [`FfiLocalInitData`] | `table` module |
//! | [`ReplacementScanBuilder`] | `replacement_scan` module |
//! | [`ReplacementScanInfo`] | `replacement_scan` module |
//! | [`SqlMacro`] | `sql_macro` module |
//! | [`ChunkWriter`] | `chunk_writer` module |
//! | [`DataChunk`] | `data_chunk` module |
//! | [`Value`] | `value` module |
//! | [`VectorReader`] | `vector` module |
//! | [`VectorWriter`] | `vector` module |
//! | [`ValidityBitmap`] | `vector::validity` module |
//! | [`ArrayVector`] | `vector::complex` module |
//! | [`StructReader`] | `vector::struct_reader` module |
//! | [`StructWriter`] | `vector::struct_writer` module |
//! | [`StructVector`] | `vector::complex` module |
//! | [`ListVector`] | `vector::complex` module |
//! | [`MapVector`] | `vector::complex` module |
//! | [`TypeId`] | `types` module |
//! | [`LogicalType`] | `types` module |
//! | [`NullHandling`] | `types` module |
//! | [`DuckInterval`] | `interval` module |
//! | [`interval_to_micros`] | `interval` module |
//! | [`ExtensionError`] | `error` module |
//! | [`ExtResult`] | `error` module |
//! | [`SecretEntry`] | `secrets` module |
//! | [`SecretsManager`] | `secrets` module |
//! | [`TlsConfigProvider`] | `tls` module |
//! | [`TlsVersion`] | `tls` module |
//! | [`audit_tls_provider`] | `tls` module |
//! | [`ExtensionWarning`] | `warning` module |
//! | [`WarningCollector`] | `warning` module |
//! | [`WarningSeverity`] | `warning` module |
//! | [`DUCKDB_API_VERSION`] | crate root |
//!
//! ## `DuckDB` 1.5.0+ items (require the `duckdb-1-5` feature)
//!
//! | Item | From |
//! |------|------|
//! | `Appender` | `appender` module |
//! | `ErrorData` / `DuckDbErrorType` | `error_data` module |
//! | `Expression` | `expression` module |
//! | `FileSystem` / `FileHandle` / `FileOpenOptions` / `FileFlag` | `file_system` module |
//! | `SelectionVector` | `selection_vector` module |
//! | `InstanceCache` | `instance_cache` module |
//!
//! # What is NOT included
//!
//! The following items are intentionally excluded from the prelude because they
//! are used less frequently and benefit from explicit import paths:
//!
//! - [`crate::config::DbConfig`] — RAII wrapper for opening secondary `DuckDB` databases;
//!   import explicitly via `use quack_rs::config::DbConfig` when needed
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
pub use crate::entry_point::{init_extension, init_extension_v2};

// Connection facade and Registrar trait
pub use crate::connection::{Connection, Registrar};

// Cast functions
pub use crate::cast::{CastFn, CastFunctionBuilder, CastFunctionInfo, CastMode};

// Aggregate functions
pub use crate::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionInfo, AggregateFunctionSetBuilder, AggregateState,
    FfiState,
};

// Scalar functions
#[cfg(feature = "duckdb-1-5")]
pub use crate::scalar::{ScalarBindInfo, ScalarInitInfo};
pub use crate::scalar::{
    ScalarFunctionBuilder, ScalarFunctionInfo, ScalarFunctionSetBuilder, ScalarOverloadBuilder,
};

// Copy functions
#[cfg(feature = "duckdb-1-5")]
pub use crate::copy_function::{
    CopyBindFn, CopyBindInfo, CopyFinalizeFn, CopyFinalizeInfo, CopyFunctionBuilder,
    CopyGlobalInitFn, CopyGlobalInitInfo, CopySinkFn, CopySinkInfo,
};

// DuckDB 1.5.0+ API surfaces (require the `duckdb-1-5` feature).
#[cfg(feature = "duckdb-1-5")]
pub use crate::appender::Appender;
#[cfg(feature = "duckdb-1-5")]
pub use crate::error_data::{DuckDbErrorType, ErrorData};
#[cfg(feature = "duckdb-1-5")]
pub use crate::expression::Expression;
#[cfg(feature = "duckdb-1-5")]
pub use crate::file_system::{FileFlag, FileHandle, FileOpenOptions, FileSystem};
#[cfg(feature = "duckdb-1-5")]
pub use crate::instance_cache::InstanceCache;
#[cfg(feature = "duckdb-1-5")]
pub use crate::selection_vector::SelectionVector;

// Table functions
pub use crate::table::{
    BindInfo, FfiBindData, FfiInitData, FfiLocalInitData, FunctionInfo, InitInfo,
    TableFunctionBuilder, TypedTableFunctionBuilder,
};

// Replacement scans
pub use crate::replacement_scan::{ReplacementScanBuilder, ReplacementScanInfo};

// SQL macros
pub use crate::sql_macro::SqlMacro;

// Chunk writer
pub use crate::chunk_writer::ChunkWriter;

// Data chunks
pub use crate::data_chunk::DataChunk;

// Value
pub use crate::value::Value;

// Vector I/O
pub use crate::vector::complex::{ArrayVector, ListVector, MapVector, StructVector};
pub use crate::vector::{StructReader, StructWriter, ValidityBitmap, VectorReader, VectorWriter};

// Types
pub use crate::types::{LogicalType, NullHandling, TypeId};

// Interval
pub use crate::interval::{interval_to_micros, DuckInterval};

// Error
pub use crate::error::{ExtResult, ExtensionError};

// Secrets manager
pub use crate::secrets::{SecretEntry, SecretsManager};

// TLS config provider
pub use crate::tls::{audit_tls_provider, TlsConfigProvider, TlsVersion};

// Warnings
pub use crate::warning::{ExtensionWarning, WarningCollector, WarningSeverity};

// API version constant
pub use crate::DUCKDB_API_VERSION;

// The entry_point! macro is already available at the crate root via #[macro_export],
// so `use quack_rs::prelude::*` brings it into scope automatically.
