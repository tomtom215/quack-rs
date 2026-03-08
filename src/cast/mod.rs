// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` custom cast functions.
//!
//! Cast functions let an extension define how to convert values from one type to
//! another, covering both explicit `CAST(x AS T)` and implicit coercions that
//! `DuckDB` may insert automatically.
//!
//! # How cast functions work
//!
//! ```text
//! SELECT my_value::MyTargetType
//!   ↓
//! DuckDB looks up registered casts from source type → target type
//!   ↓
//! Calls your cast callback with:
//!   - info:   duckdb_function_info  (use for extra data / error reporting)
//!   - count:  idx_t                 (number of rows in this chunk)
//!   - input:  duckdb_vector         (source values)
//!   - output: duckdb_vector         (destination — write results here)
//!   ↓
//! Returns true on success, false to signal a fatal error
//! ```
//!
//! # TRY CAST vs normal CAST
//!
//! When the user writes `TRY_CAST(x AS T)`, `DuckDB` passes
//! [`CastMode::Try`] to your callback (via
//! [`CastFunctionInfo::cast_mode`]).  In this mode, per-row conversion errors
//! should write `NULL` into the output vector and call
//! [`CastFunctionInfo::set_row_error`] to record what went wrong, rather than
//! aborting the whole query.
//!
//! # Example: register a VARCHAR → INTEGER cast
//!
//! ```rust,no_run
//! use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo, CastMode};
//! use quack_rs::types::TypeId;
//! use quack_rs::vector::{VectorReader, VectorWriter};
//! use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_vector, idx_t};
//!
//! unsafe extern "C" fn varchar_to_int(
//!     info: duckdb_function_info,
//!     count: idx_t,
//!     input: duckdb_vector,
//!     output: duckdb_vector,
//! ) -> bool {
//!     let cast_info = unsafe { CastFunctionInfo::new(info) };
//!     let reader = unsafe { VectorReader::from_vector(input, count as usize) };
//!     let mut writer = unsafe { VectorWriter::new(output) };
//!
//!     for row in 0..count as usize {
//!         if !unsafe { reader.is_valid(row) } {
//!             unsafe { writer.set_null(row) };
//!             continue;
//!         }
//!         let s = unsafe { reader.read_str(row) };
//!         match s.parse::<i32>() {
//!             Ok(v) => unsafe { writer.write_i32(row, v) },
//!             Err(e) => {
//!                 if cast_info.cast_mode() == CastMode::Try {
//!                     let msg = format!("cannot cast {:?} to INTEGER: {e}", s);
//!                     unsafe { cast_info.set_row_error(&msg, row as idx_t, output) };
//!                     unsafe { writer.set_null(row) };
//!                 } else {
//!                     let msg = format!("cannot cast {:?} to INTEGER: {e}", s);
//!                     unsafe { cast_info.set_error(&msg) };
//!                     return false;
//!                 }
//!             }
//!         }
//!     }
//!     true
//! }
//!
//! // fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
//! //     unsafe {
//! //         CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
//! //             .function(varchar_to_int)
//! //             .register(con)
//! //     }
//! // }
//! ```

pub mod builder;

pub use builder::{CastFn, CastFunctionBuilder, CastFunctionInfo, CastMode};
