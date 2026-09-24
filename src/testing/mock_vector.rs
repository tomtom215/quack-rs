// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! In-memory mock types for `DuckDB` vectors.
//!
//! [`MockVectorWriter`] and [`MockVectorReader`] let you test callback logic —
//! the code that reads input rows and writes output values — without a live
//! `DuckDB` instance.
//!
//! # Why these exist
//!
//! `DuckDB` loadable extensions use `libduckdb-sys` with
//! `features = ["loadable-extension"]`, which routes every C API call through a
//! lazy dispatch table. That table is only initialized when `DuckDB` calls
//! `duckdb_rs_extension_api_init` at extension load time. In `cargo test`, no
//! `DuckDB` process loads the extension, so the dispatch table is never
//! initialized and any call to `VectorReader::new` or `VectorWriter::new` panics
//! with `DuckDB API not initialized`.
//!
//! These mock types provide the same write/read interface but store data in a
//! plain `Vec`, with no `DuckDB` dependency at all.
//!
//! # What they are for
//!
//! Keep the per-row computation in plain Rust functions and test those
//! directly. The mocks are for the row loop around it: they share the method
//! names of [`VectorReader`][crate::vector::VectorReader] and
//! [`VectorWriter`][crate::vector::VectorWriter] but are **separate types**, so
//! a function written against them cannot be handed the real reader and writer.
//! To run a real callback against real vectors, use
//! `InMemoryDb` (`bundled-test` / `bundled-test-prebuilt` features).
//!
//! [`MockVectorWriter`] reproduces a real output vector's NULL and capacity
//! behaviour rather than being more forgiving than it; see its type docs.
//!
//! ```rust
//! use quack_rs::testing::{MockVectorWriter, MockVectorReader, MockDuckValue};
//!
//! /// A row loop prototyped against the mocks.
//! fn compute_double(reader: &MockVectorReader, writer: &mut MockVectorWriter) {
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
//! let reader = MockVectorReader::from_i64s([Some(1), Some(5), None, Some(-3)]);
//! let mut writer = MockVectorWriter::new(4);
//! compute_double(&reader, &mut writer);
//!
//! assert_eq!(writer.try_get_i64(0), Some(2));
//! assert_eq!(writer.try_get_i64(1), Some(10));
//! assert!(writer.is_null(2));
//! assert_eq!(writer.try_get_i64(3), Some(-6));
//! ```

mod reader;
#[cfg(test)]
mod tests;
mod writer;

use crate::interval::DuckInterval;

/// A `DuckDB`-compatible value variant for testing.
///
/// Used by both [`MockVectorWriter`] and [`MockVectorReader`] to represent the
/// typed values in a column without requiring a live `DuckDB` runtime.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MockDuckValue {
    /// `TINYINT` / `INT8`
    I8(i8),
    /// `SMALLINT` / `INT16`
    I16(i16),
    /// `INTEGER` / `INT32`
    I32(i32),
    /// `BIGINT` / `INT64`
    I64(i64),
    /// `UTINYINT` / `UINT8`
    U8(u8),
    /// `USMALLINT` / `UINT16`
    U16(u16),
    /// `UINTEGER` / `UINT32`
    U32(u32),
    /// `UBIGINT` / `UINT64`
    U64(u64),
    /// `FLOAT`
    F32(f32),
    /// `DOUBLE`
    F64(f64),
    /// `BOOLEAN`
    Bool(bool),
    /// `HUGEINT`
    I128(i128),
    /// `VARCHAR`
    Varchar(String),
    /// `BLOB`
    Blob(Vec<u8>),
    /// `INTERVAL`
    Interval(DuckInterval),
}

/// An in-memory mock output vector for testing finalize and scan callbacks.
///
/// Write typed values and NULL flags using the same method names as
/// [`VectorWriter`][crate::vector::VectorWriter]. Inspect the results with
/// [`try_get_i64`][Self::try_get_i64], [`is_null`][Self::is_null], etc.
///
/// # Example
///
/// ```rust
/// use quack_rs::testing::{MockVectorWriter, MockDuckValue};
///
/// let mut w = MockVectorWriter::new(3);
/// w.write_i64(0, 42);
/// w.write_i64(1, -7);
/// w.set_null(2);
///
/// assert_eq!(w.try_get_i64(0), Some(42));
/// assert_eq!(w.try_get_i64(1), Some(-7));
/// assert!(w.is_null(2));
/// ```
///
/// # It behaves like a real output vector, including where that hurts
///
/// A mock that is more forgiving than `DuckDB` makes a test pass for code that
/// is wrong in production. So, as with a real vector (verified against
/// `DuckDB` 1.5.5):
///
/// - **Validity is separate from data.** [`set_null`][Self::set_null] clears a
///   validity bit and a later `write_*` does **not** set it again — a real
///   vector keeps returning NULL for that row. Use
///   [`set_valid`][Self::set_valid] to undo a NULL.
/// - **A row that is never written is valid, not NULL.** `DuckDB` hands out
///   output vectors whose rows are all valid and whose data is whatever the
///   buffer last held, so a callback that forgets `set_null` returns garbage.
///   Here such a row reports [`is_null`][Self::is_null] `== false` and
///   [`is_written`][Self::is_written] `== false`, so a test can catch it.
/// - **Capacity is fixed.** Writing at or past the capacity given to
///   [`new`][Self::new] panics; on a real vector it is out-of-bounds memory
///   access.
/// - **Strings over [`MAX_STRING_LEN`][crate::vector::string::MAX_STRING_LEN]
///   panic**, as [`VectorWriter::write_varchar`][crate::vector::VectorWriter::write_varchar]
///   does.
#[derive(Debug, Default)]
pub struct MockVectorWriter {
    /// Data per row; `None` means never written.
    rows: Vec<Option<MockDuckValue>>,
    /// Validity per row; `true` (valid) until `set_null`.
    valid: Vec<bool>,
}

/// An in-memory mock input vector for testing update and scan callbacks.
///
/// Construct from typed slices using the convenience constructors, then call
/// `row_count()`, `is_valid()`, and `try_get_*()` in your callback logic,
/// matching the method names used in real `DuckDB` callbacks.
///
/// # Example
///
/// ```rust
/// use quack_rs::testing::{MockVectorReader, MockDuckValue};
///
/// let reader = MockVectorReader::from_i64s([Some(10), None, Some(30)]);
/// assert_eq!(reader.row_count(), 3);
/// assert!(reader.is_valid(0));
/// assert!(!reader.is_valid(1));
/// assert_eq!(reader.try_get_i64(0), Some(10));
/// assert_eq!(reader.try_get_i64(1), None); // NULL row
/// assert_eq!(reader.try_get_i64(2), Some(30));
/// ```
#[derive(Debug, Clone)]
pub struct MockVectorReader {
    rows: Vec<Option<MockDuckValue>>,
}
