// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Safe helpers for reading from and writing to `DuckDB` data vectors.
//!
//! `DuckDB` represents columnar data as "vectors" — arrays of typed values
//! with an associated validity bitmap for NULL tracking. This module provides
//! safe wrappers that eliminate the raw pointer arithmetic and undocumented
//! struct layouts that trip up extension authors.
//!
//! # Pitfalls solved by this module
//!
//! - **L4**: `ensure_validity_writable` — [`VectorWriter`] calls this automatically
//!   before any NULL-setting operation.
//! - **L5**: Boolean reading — [`VectorReader`] always reads bytes as `u8 != 0`,
//!   never as `bool`, preventing undefined behaviour.
//! - **P7**: `duckdb_string_t` format — [`string`] handles both the inline (≤12 bytes)
//!   and pointer (>12 bytes) cases.

pub mod complex;
pub mod reader;
pub mod string;
pub mod struct_reader;
pub mod struct_writer;
pub mod validity;
pub mod writer;

pub use reader::VectorReader;
pub use string::{read_duck_blob, read_duck_string, DuckStringView};
pub use struct_reader::StructReader;
pub use struct_writer::StructWriter;
pub use validity::ValidityBitmap;
pub use writer::VectorWriter;

/// Returns the default vector size used by `DuckDB` (typically 2048).
#[mutants::skip]
pub fn vector_size() -> u64 {
    unsafe { libduckdb_sys::duckdb_vector_size() as u64 }
}

/// Returns the logical type of a vector.
///
/// # Safety
/// `vector` must be a valid `duckdb_vector`.
pub unsafe fn vector_get_column_type(
    vector: libduckdb_sys::duckdb_vector,
) -> crate::types::LogicalType {
    let raw = unsafe { libduckdb_sys::duckdb_vector_get_column_type(vector) };
    unsafe { crate::types::LogicalType::from_raw(raw) }
}
