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

pub mod reader;
pub mod string;
pub mod validity;
pub mod writer;

pub use reader::VectorReader;
pub use string::{read_duck_string, DuckStringView};
pub use validity::ValidityBitmap;
pub use writer::VectorWriter;
