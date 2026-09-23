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
//!
//! fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
//!     let _ = con;
//!     Ok(())
//! }
//!
//! // The entry-point macros come with the glob import.
//! entry_point!(my_extension_init_c_api, |con| register(con));
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
//! | [`AbiPolicy`] | `abi` module |
//! | [`Connection`] | `connection` module |
//! | [`Registrar`] | `connection` module |
//! | [`CastFn`] | `cast` module |
//! | [`CastFunctionBuilder`] | `cast` module |
//! | [`CastFunctionInfo`] | `cast` module |
//! | [`CastMode`] | `cast` module |
//! | [`AggregateFunctionBuilder`] | `aggregate` module |
//! | [`AggregateFunctionInfo`] | `aggregate` module |
//! | [`AggregateFunctionSetBuilder`] | `aggregate` module |
//! | [`AggregateOverloadBuilder`] | `aggregate` module |
//! | [`AggregateState`] | `aggregate` module |
//! | [`FfiState`] | `aggregate` module |
//! | [`ScalarFunctionBuilder`] | `scalar` module |
//! | [`ScalarFunctionInfo`] | `scalar` module |
//! | [`ScalarFunctionSetBuilder`] | `scalar` module |
//! | [`ScalarOverloadBuilder`] | `scalar` module |
//! | [`TypedScalarFunctionBuilder`] | `scalar` module |
//! | [`TableFunctionBuilder`] | `table` module |
//! | [`TypedTableFunctionBuilder`] | `table` module |
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
//! | [`ListBuilder`] | `vector` module |
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
//! | [`Date`] / [`Time`] / [`TimeTz`] / [`Timestamp`] | `datetime` module |
//! | [`OwnedConnection`] / [`OwnedDataChunk`] / [`PreparedStatement`] / [`QueryResult`] | `query` module |
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
//! | [`Appender`] | `appender` module |
//!
//! ## `DuckDB` 1.5.0+ items (require the `duckdb-1-5` feature)
//!
//! | Item | From |
//! |------|------|
//! | `ScalarBindInfo` / `ScalarInitInfo` | `scalar` module |
//! | `ScalarBindData` / `ScalarLocalState` | `scalar` module |
//! | `CopyFunctionBuilder` and its callback types: `CopyBindFn` / `CopyBindInfo`, `CopyGlobalInitFn` / `CopyGlobalInitInfo`, `CopySinkFn` / `CopySinkInfo`, `CopyFinalizeFn` / `CopyFinalizeInfo` | `copy_function` module |
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

/// C extension API layout policy for the entry point.
pub use crate::abi::AbiPolicy;

// Connection facade and Registrar trait
pub use crate::connection::{Connection, Registrar};

// Cast functions
pub use crate::cast::{CastFn, CastFunctionBuilder, CastFunctionInfo, CastMode};

// Aggregate functions
pub use crate::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionInfo, AggregateFunctionSetBuilder,
    AggregateOverloadBuilder, AggregateState, FfiState,
};

// Scalar functions
#[cfg(feature = "duckdb-1-5")]
pub use crate::scalar::{ScalarBindData, ScalarBindInfo, ScalarInitInfo, ScalarLocalState};
pub use crate::scalar::{
    ScalarFunctionBuilder, ScalarFunctionInfo, ScalarFunctionSetBuilder, ScalarOverloadBuilder,
    TypedScalarFunctionBuilder,
};

// Copy functions
#[cfg(feature = "duckdb-1-5")]
pub use crate::copy_function::{
    CopyBindFn, CopyBindInfo, CopyFinalizeFn, CopyFinalizeInfo, CopyFunctionBuilder,
    CopyGlobalInitFn, CopyGlobalInitInfo, CopySinkFn, CopySinkInfo,
};

// The appender itself is in the stable C API and needs no feature (only a
// few of its methods do), so it is exported unconditionally.
pub use crate::appender::Appender;

// DuckDB 1.5.0+ API surfaces (require the `duckdb-1-5` feature).
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
pub use crate::vector::{
    ListBuilder, StructReader, StructWriter, ValidityBitmap, VectorReader, VectorWriter,
};

// Types
pub use crate::types::{LogicalType, NullHandling, TypeId};

// Interval
pub use crate::interval::{interval_to_micros, DuckInterval};

/// Calendar conversions for `DATE` / `TIME` / `TIMESTAMP`.
pub use crate::datetime::{Date, Time, TimeTz, Timestamp};

/// Running SQL from inside an extension.
pub use crate::query::{OwnedConnection, OwnedDataChunk, PreparedStatement, QueryResult};

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

// `#[macro_export]` places the entry-point macros at the crate root only; a glob
// import of this module does not reach them unless they are re-exported here.
// (`crate::entry_point` also names the module; this re-exports both, which is
// harmless: the macro and module namespaces are separate.)
pub use crate::{entry_point, entry_point_v2};

#[cfg(test)]
mod tests {
    /// Every name this module re-exports appears in one of the tables in the
    /// module documentation, so the tables cannot drift from the code again.
    #[test]
    fn the_module_docs_list_every_re_export() {
        let source = include_str!("prelude.rs");
        let (docs, code) = source
            .split_once("// Entry point")
            .expect("the re-exports start at the entry-point section");
        let code = code.split("#[cfg(test)]").next().unwrap_or(code);
        let mut missing = Vec::new();
        for item in code.split("pub use ").skip(1) {
            let item = item.split(';').next().unwrap_or("");
            let names = item.rsplit("::").next().unwrap_or(item);
            for name in names
                .trim_matches(|c: char| c == '{' || c == '}' || c.is_whitespace())
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
            {
                let listed = docs.contains(&format!("[`{name}`]"))
                    || docs.contains(&format!("`{name}`"))
                    || docs.contains(&format!("`{name}!`"));
                if !listed {
                    missing.push(name.to_owned());
                }
            }
        }
        assert!(
            missing.is_empty(),
            "not listed in the prelude docs: {missing:?}"
        );
    }
}
