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
//! with "DuckDB API not initialized".
//!
//! These mock types provide the same write/read interface but store data in a
//! plain `Vec`, with no `DuckDB` dependency at all.
//!
//! # Recommended pattern
//!
//! Extract your callback logic into a pure-Rust function, then call it from both
//! the FFI callback (with the real writer) and your tests (with the mock):
//!
//! ```rust
//! use quack_rs::testing::{MockVectorWriter, MockVectorReader, MockDuckValue};
//!
//! /// Pure business logic — testable without DuckDB.
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
//! #[test]
//! fn test_double_logic() {
//!     let reader = MockVectorReader::from_i64s([Some(1), Some(5), None, Some(-3)]);
//!     let mut writer = MockVectorWriter::new(4);
//!     compute_double(&reader, &mut writer);
//!
//!     assert_eq!(writer.try_get_i64(0), Some(2));
//!     assert_eq!(writer.try_get_i64(1), Some(10));
//!     assert!(writer.is_null(2));
//!     assert_eq!(writer.try_get_i64(3), Some(-6));
//! }
//! ```

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
#[derive(Debug, Default)]
pub struct MockVectorWriter {
    rows: Vec<Option<MockDuckValue>>,
}

impl MockVectorWriter {
    /// Creates a new writer pre-allocated for `capacity` rows (all NULL).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            rows: vec![None; capacity],
        }
    }

    /// Ensures the internal buffer is large enough to hold row `idx`.
    fn ensure_capacity(&mut self, idx: usize) {
        if idx >= self.rows.len() {
            self.rows.resize(idx + 1, None);
        }
    }

    /// Marks row `idx` as NULL.
    pub fn set_null(&mut self, idx: usize) {
        self.ensure_capacity(idx);
        self.rows[idx] = None;
    }

    /// Returns `true` if row `idx` is NULL or has not been written.
    #[must_use]
    pub fn is_null(&self, idx: usize) -> bool {
        self.rows.get(idx).map_or(true, |v| v.is_none())
    }

    /// Returns the number of allocated rows (including NULLs).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if no rows have been allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the raw `Option<MockDuckValue>` for row `idx`.
    ///
    /// Returns `None` if the row is NULL or has never been written.
    #[must_use]
    pub fn get(&self, idx: usize) -> Option<&MockDuckValue> {
        self.rows.get(idx).and_then(|v| v.as_ref())
    }

    // ── Numeric writes ──────────────────────────────────────────────────────

    /// Writes a `TINYINT` value at row `idx`.
    pub fn write_i8(&mut self, idx: usize, value: i8) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::I8(value));
    }

    /// Writes a `SMALLINT` value at row `idx`.
    pub fn write_i16(&mut self, idx: usize, value: i16) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::I16(value));
    }

    /// Writes an `INTEGER` value at row `idx`.
    pub fn write_i32(&mut self, idx: usize, value: i32) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::I32(value));
    }

    /// Writes a `BIGINT` value at row `idx`.
    pub fn write_i64(&mut self, idx: usize, value: i64) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::I64(value));
    }

    /// Writes a `UTINYINT` value at row `idx`.
    pub fn write_u8(&mut self, idx: usize, value: u8) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::U8(value));
    }

    /// Writes a `USMALLINT` value at row `idx`.
    pub fn write_u16(&mut self, idx: usize, value: u16) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::U16(value));
    }

    /// Writes a `UINTEGER` value at row `idx`.
    pub fn write_u32(&mut self, idx: usize, value: u32) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::U32(value));
    }

    /// Writes a `UBIGINT` value at row `idx`.
    pub fn write_u64(&mut self, idx: usize, value: u64) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::U64(value));
    }

    /// Writes a `FLOAT` value at row `idx`.
    pub fn write_f32(&mut self, idx: usize, value: f32) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::F32(value));
    }

    /// Writes a `DOUBLE` value at row `idx`.
    pub fn write_f64(&mut self, idx: usize, value: f64) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::F64(value));
    }

    /// Writes a `BOOLEAN` value at row `idx`.
    pub fn write_bool(&mut self, idx: usize, value: bool) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::Bool(value));
    }

    /// Writes a `HUGEINT` value at row `idx`.
    pub fn write_i128(&mut self, idx: usize, value: i128) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::I128(value));
    }

    /// Writes a `VARCHAR` value at row `idx`.
    pub fn write_varchar(&mut self, idx: usize, value: &str) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::Varchar(value.to_owned()));
    }

    /// Writes an `INTERVAL` value at row `idx`.
    pub fn write_interval(&mut self, idx: usize, value: DuckInterval) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::Interval(value));
    }

    // ── Typed getters ───────────────────────────────────────────────────────

    /// Returns the `BIGINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i64(&self, idx: usize) -> Option<i64> {
        match self.get(idx) {
            Some(MockDuckValue::I64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `INTEGER` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i32(&self, idx: usize) -> Option<i32> {
        match self.get(idx) {
            Some(MockDuckValue::I32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `DOUBLE` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_f64(&self, idx: usize) -> Option<f64> {
        match self.get(idx) {
            Some(MockDuckValue::F64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BOOLEAN` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_bool(&self, idx: usize) -> Option<bool> {
        match self.get(idx) {
            Some(MockDuckValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `VARCHAR` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_str(&self, idx: usize) -> Option<&str> {
        match self.get(idx) {
            Some(MockDuckValue::Varchar(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the `INTERVAL` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_interval(&self, idx: usize) -> Option<DuckInterval> {
        match self.get(idx) {
            Some(MockDuckValue::Interval(v)) => Some(*v),
            _ => None,
        }
    }
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

impl MockVectorReader {
    /// Creates a reader from an arbitrary sequence of `Option<MockDuckValue>`.
    ///
    /// `None` entries represent NULL rows.
    #[must_use]
    pub fn new(rows: impl IntoIterator<Item = Option<MockDuckValue>>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }

    /// Creates a reader from a sequence of `Option<i64>` values.
    ///
    /// Convenience constructor for `BIGINT` columns.
    #[must_use]
    pub fn from_i64s(values: impl IntoIterator<Item = Option<i64>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::I64)))
    }

    /// Creates a reader from a sequence of `Option<i32>` values.
    ///
    /// Convenience constructor for `INTEGER` columns.
    #[must_use]
    pub fn from_i32s(values: impl IntoIterator<Item = Option<i32>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::I32)))
    }

    /// Creates a reader from a sequence of `Option<f64>` values.
    ///
    /// Convenience constructor for `DOUBLE` columns.
    #[must_use]
    pub fn from_f64s(values: impl IntoIterator<Item = Option<f64>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::F64)))
    }

    /// Creates a reader from a sequence of `Option<bool>` values.
    ///
    /// Convenience constructor for `BOOLEAN` columns.
    #[must_use]
    pub fn from_bools(values: impl IntoIterator<Item = Option<bool>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::Bool)))
    }

    /// Creates a reader from a sequence of `Option<&str>` values.
    ///
    /// Convenience constructor for `VARCHAR` columns.
    #[must_use]
    pub fn from_strs<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        Self::new(
            values
                .into_iter()
                .map(|v| v.map(|s| MockDuckValue::Varchar(s.to_owned()))),
        )
    }

    /// Returns the number of rows in this reader.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if row `idx` is not NULL.
    ///
    /// Always returns `false` for out-of-bounds indices.
    #[must_use]
    pub fn is_valid(&self, idx: usize) -> bool {
        self.rows.get(idx).map_or(false, |v| v.is_some())
    }

    /// Returns the raw value at row `idx`, or `None` if NULL or out of bounds.
    #[must_use]
    pub fn get(&self, idx: usize) -> Option<&MockDuckValue> {
        self.rows.get(idx).and_then(|v| v.as_ref())
    }

    // ── Typed getters ───────────────────────────────────────────────────────

    /// Returns the `BIGINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i64(&self, idx: usize) -> Option<i64> {
        match self.get(idx) {
            Some(MockDuckValue::I64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `INTEGER` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i32(&self, idx: usize) -> Option<i32> {
        match self.get(idx) {
            Some(MockDuckValue::I32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `DOUBLE` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_f64(&self, idx: usize) -> Option<f64> {
        match self.get(idx) {
            Some(MockDuckValue::F64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BOOLEAN` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_bool(&self, idx: usize) -> Option<bool> {
        match self.get(idx) {
            Some(MockDuckValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `VARCHAR` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_str(&self, idx: usize) -> Option<&str> {
        match self.get(idx) {
            Some(MockDuckValue::Varchar(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the `INTERVAL` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_interval(&self, idx: usize) -> Option<DuckInterval> {
        match self.get(idx) {
            Some(MockDuckValue::Interval(v)) => Some(*v),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_write_and_read_i64() {
        let mut w = MockVectorWriter::new(3);
        w.write_i64(0, 42);
        w.write_i64(1, -100);
        w.set_null(2);
        assert_eq!(w.try_get_i64(0), Some(42));
        assert_eq!(w.try_get_i64(1), Some(-100));
        assert!(w.is_null(2));
    }

    #[test]
    fn writer_grows_beyond_initial_capacity() {
        let mut w = MockVectorWriter::new(1);
        w.write_i64(5, 99); // grows from 1 to 6
        assert_eq!(w.len(), 6);
        assert_eq!(w.try_get_i64(5), Some(99));
        assert!(w.is_null(0)); // never written
    }

    #[test]
    fn writer_set_null_clears_previous_value() {
        let mut w = MockVectorWriter::new(1);
        w.write_i64(0, 42);
        assert!(!w.is_null(0));
        w.set_null(0);
        assert!(w.is_null(0));
    }

    #[test]
    fn writer_varchar() {
        let mut w = MockVectorWriter::new(2);
        w.write_varchar(0, "hello");
        w.set_null(1);
        assert_eq!(w.try_get_str(0), Some("hello"));
        assert!(w.is_null(1));
    }

    #[test]
    fn writer_all_types_round_trip() {
        let mut w = MockVectorWriter::new(10);
        w.write_i8(0, 127);
        w.write_i16(1, 1000);
        w.write_i32(2, 100_000);
        w.write_i64(3, 1_000_000_000);
        w.write_u8(4, 255);
        w.write_u32(5, 999);
        w.write_u64(6, u64::MAX);
        w.write_f32(7, 3.14_f32);
        w.write_f64(8, std::f64::consts::PI);
        w.write_bool(9, true);

        assert!(matches!(w.get(0), Some(MockDuckValue::I8(127))));
        assert!(matches!(w.get(1), Some(MockDuckValue::I16(1000))));
        assert!(matches!(w.get(2), Some(MockDuckValue::I32(100_000))));
        assert_eq!(w.try_get_i64(3), Some(1_000_000_000));
        assert!(matches!(w.get(4), Some(MockDuckValue::U8(255))));
        assert_eq!(w.try_get_bool(9), Some(true));
    }

    #[test]
    fn reader_from_i64s() {
        let r = MockVectorReader::from_i64s([Some(1), None, Some(3)]);
        assert_eq!(r.row_count(), 3);
        assert!(r.is_valid(0));
        assert!(!r.is_valid(1));
        assert!(r.is_valid(2));
        assert_eq!(r.try_get_i64(0), Some(1));
        assert_eq!(r.try_get_i64(1), None);
        assert_eq!(r.try_get_i64(2), Some(3));
    }

    #[test]
    fn reader_from_strs() {
        let r = MockVectorReader::from_strs([Some("hello"), None, Some("world")]);
        assert_eq!(r.try_get_str(0), Some("hello"));
        assert_eq!(r.try_get_str(1), None);
        assert_eq!(r.try_get_str(2), Some("world"));
    }

    #[test]
    fn reader_out_of_bounds_is_invalid() {
        let r = MockVectorReader::from_i64s([Some(1)]);
        assert!(!r.is_valid(99));
        assert_eq!(r.try_get_i64(99), None);
    }

    #[test]
    fn mock_double_pattern() {
        // Demonstrates extracting callback logic into a testable pure-Rust function.
        fn double_values(reader: &MockVectorReader, writer: &mut MockVectorWriter) {
            for i in 0..reader.row_count() {
                if reader.is_valid(i) {
                    let v = reader.try_get_i64(i).unwrap_or(0);
                    writer.write_i64(i, v * 2);
                } else {
                    writer.set_null(i);
                }
            }
        }

        let reader = MockVectorReader::from_i64s([Some(1), Some(5), None, Some(-3)]);
        let mut writer = MockVectorWriter::new(4);
        double_values(&reader, &mut writer);

        assert_eq!(writer.try_get_i64(0), Some(2));
        assert_eq!(writer.try_get_i64(1), Some(10));
        assert!(writer.is_null(2));
        assert_eq!(writer.try_get_i64(3), Some(-6));
    }
}
