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
//! let reader = MockVectorReader::from_i64s([Some(1), Some(5), None, Some(-3)]);
//! let mut writer = MockVectorWriter::new(4);
//! compute_double(&reader, &mut writer);
//!
//! assert_eq!(writer.try_get_i64(0), Some(2));
//! assert_eq!(writer.try_get_i64(1), Some(10));
//! assert!(writer.is_null(2));
//! assert_eq!(writer.try_get_i64(3), Some(-6));
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
        self.rows.get(idx).is_none_or(Option::is_none)
    }

    /// Returns the number of allocated rows (including NULLs).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if no rows have been allocated.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
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

    /// Writes a `VARCHAR` value at row `idx`.
    ///
    /// Alias for [`write_varchar`][MockVectorWriter::write_varchar].
    pub fn write_str(&mut self, idx: usize, value: &str) {
        self.write_varchar(idx, value);
    }

    /// Writes an `INTERVAL` value at row `idx`.
    pub fn write_interval(&mut self, idx: usize, value: DuckInterval) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::Interval(value));
    }

    /// Writes a `BLOB` value at row `idx`.
    pub fn write_blob(&mut self, idx: usize, value: &[u8]) {
        self.ensure_capacity(idx);
        self.rows[idx] = Some(MockDuckValue::Blob(value.to_vec()));
    }

    /// Writes a `DATE` value (days since epoch) at row `idx`.
    ///
    /// Semantic alias for [`write_i32`][Self::write_i32].
    pub fn write_date(&mut self, idx: usize, days_since_epoch: i32) {
        self.write_i32(idx, days_since_epoch);
    }

    /// Writes a `TIMESTAMP` value (microseconds since epoch) at row `idx`.
    ///
    /// Semantic alias for [`write_i64`][Self::write_i64].
    pub fn write_timestamp(&mut self, idx: usize, micros_since_epoch: i64) {
        self.write_i64(idx, micros_since_epoch);
    }

    /// Writes a `TIME` value (microseconds since midnight) at row `idx`.
    ///
    /// Semantic alias for [`write_i64`][Self::write_i64].
    pub fn write_time(&mut self, idx: usize, micros_since_midnight: i64) {
        self.write_i64(idx, micros_since_midnight);
    }

    /// Writes a `UUID` value (as i128) at row `idx`.
    ///
    /// Semantic alias for [`write_i128`][Self::write_i128].
    pub fn write_uuid(&mut self, idx: usize, value: i128) {
        self.write_i128(idx, value);
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

    /// Returns the `TINYINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i8(&self, idx: usize) -> Option<i8> {
        match self.get(idx) {
            Some(MockDuckValue::I8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `SMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i16(&self, idx: usize) -> Option<i16> {
        match self.get(idx) {
            Some(MockDuckValue::I16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UTINYINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u8(&self, idx: usize) -> Option<u8> {
        match self.get(idx) {
            Some(MockDuckValue::U8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `USMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u16(&self, idx: usize) -> Option<u16> {
        match self.get(idx) {
            Some(MockDuckValue::U16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UINTEGER` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u32(&self, idx: usize) -> Option<u32> {
        match self.get(idx) {
            Some(MockDuckValue::U32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UBIGINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u64(&self, idx: usize) -> Option<u64> {
        match self.get(idx) {
            Some(MockDuckValue::U64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `FLOAT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_f32(&self, idx: usize) -> Option<f32> {
        match self.get(idx) {
            Some(MockDuckValue::F32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `HUGEINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i128(&self, idx: usize) -> Option<i128> {
        match self.get(idx) {
            Some(MockDuckValue::I128(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BLOB` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_blob(&self, idx: usize) -> Option<&[u8]> {
        match self.get(idx) {
            Some(MockDuckValue::Blob(v)) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// Returns the `UUID` value (as i128) at row `idx`, or `None` if NULL or wrong type.
    ///
    /// Semantic alias for [`try_get_i128`][Self::try_get_i128].
    #[must_use]
    pub fn try_get_uuid(&self, idx: usize) -> Option<i128> {
        self.try_get_i128(idx)
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

    /// Creates a reader from a sequence of `Option<i8>` values.
    ///
    /// Convenience constructor for `TINYINT` columns.
    #[must_use]
    pub fn from_i8s(values: impl IntoIterator<Item = Option<i8>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::I8)))
    }

    /// Creates a reader from a sequence of `Option<i16>` values.
    ///
    /// Convenience constructor for `SMALLINT` columns.
    #[must_use]
    pub fn from_i16s(values: impl IntoIterator<Item = Option<i16>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::I16)))
    }

    /// Creates a reader from a sequence of `Option<u8>` values.
    ///
    /// Convenience constructor for `UTINYINT` columns.
    #[must_use]
    pub fn from_u8s(values: impl IntoIterator<Item = Option<u8>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::U8)))
    }

    /// Creates a reader from a sequence of `Option<u16>` values.
    ///
    /// Convenience constructor for `USMALLINT` columns.
    #[must_use]
    pub fn from_u16s(values: impl IntoIterator<Item = Option<u16>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::U16)))
    }

    /// Creates a reader from a sequence of `Option<u32>` values.
    ///
    /// Convenience constructor for `UINTEGER` columns.
    #[must_use]
    pub fn from_u32s(values: impl IntoIterator<Item = Option<u32>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::U32)))
    }

    /// Creates a reader from a sequence of `Option<u64>` values.
    ///
    /// Convenience constructor for `UBIGINT` columns.
    #[must_use]
    pub fn from_u64s(values: impl IntoIterator<Item = Option<u64>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::U64)))
    }

    /// Creates a reader from a sequence of `Option<f32>` values.
    ///
    /// Convenience constructor for `FLOAT` columns.
    #[must_use]
    pub fn from_f32s(values: impl IntoIterator<Item = Option<f32>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::F32)))
    }

    /// Creates a reader from a sequence of `Option<i128>` values.
    ///
    /// Convenience constructor for `HUGEINT` columns.
    #[must_use]
    pub fn from_i128s(values: impl IntoIterator<Item = Option<i128>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::I128)))
    }

    /// Creates a reader from a sequence of `Option<DuckInterval>` values.
    ///
    /// Convenience constructor for `INTERVAL` columns.
    #[must_use]
    pub fn from_intervals(values: impl IntoIterator<Item = Option<DuckInterval>>) -> Self {
        Self::new(values.into_iter().map(|v| v.map(MockDuckValue::Interval)))
    }

    /// Creates a reader from a sequence of `Option<&[u8]>` values.
    ///
    /// Convenience constructor for `BLOB` columns.
    #[must_use]
    pub fn from_blobs<'a>(values: impl IntoIterator<Item = Option<&'a [u8]>>) -> Self {
        Self::new(
            values
                .into_iter()
                .map(|v| v.map(|b| MockDuckValue::Blob(b.to_vec()))),
        )
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
    pub const fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if row `idx` is not NULL.
    ///
    /// Always returns `false` for out-of-bounds indices.
    #[must_use]
    pub fn is_valid(&self, idx: usize) -> bool {
        self.rows.get(idx).is_some_and(Option::is_some)
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

    /// Returns the `TINYINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i8(&self, idx: usize) -> Option<i8> {
        match self.get(idx) {
            Some(MockDuckValue::I8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `SMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i16(&self, idx: usize) -> Option<i16> {
        match self.get(idx) {
            Some(MockDuckValue::I16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UTINYINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u8(&self, idx: usize) -> Option<u8> {
        match self.get(idx) {
            Some(MockDuckValue::U8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `USMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u16(&self, idx: usize) -> Option<u16> {
        match self.get(idx) {
            Some(MockDuckValue::U16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UINTEGER` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u32(&self, idx: usize) -> Option<u32> {
        match self.get(idx) {
            Some(MockDuckValue::U32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UBIGINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_u64(&self, idx: usize) -> Option<u64> {
        match self.get(idx) {
            Some(MockDuckValue::U64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `FLOAT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_f32(&self, idx: usize) -> Option<f32> {
        match self.get(idx) {
            Some(MockDuckValue::F32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `HUGEINT` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_i128(&self, idx: usize) -> Option<i128> {
        match self.get(idx) {
            Some(MockDuckValue::I128(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BLOB` value at row `idx`, or `None` if NULL or wrong type.
    #[must_use]
    pub fn try_get_blob(&self, idx: usize) -> Option<&[u8]> {
        match self.get(idx) {
            Some(MockDuckValue::Blob(v)) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// Returns the `UUID` value (as i128) at row `idx`, or `None` if NULL or wrong type.
    ///
    /// Semantic alias for [`try_get_i128`][Self::try_get_i128].
    #[must_use]
    pub fn try_get_uuid(&self, idx: usize) -> Option<i128> {
        self.try_get_i128(idx)
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
        w.write_f32(7, std::f32::consts::PI);
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
    fn reader_from_i32s() {
        let r = MockVectorReader::from_i32s([Some(10), None, Some(-5)]);
        assert_eq!(r.row_count(), 3);
        assert!(r.is_valid(0));
        assert!(!r.is_valid(1));
        assert_eq!(r.try_get_i32(0), Some(10));
        assert_eq!(r.try_get_i32(1), None);
        assert_eq!(r.try_get_i32(2), Some(-5));
    }

    #[test]
    fn reader_from_f64s() {
        let r = MockVectorReader::from_f64s([Some(1.5), None, Some(-2.72)]);
        assert_eq!(r.row_count(), 3);
        assert!(r.is_valid(0));
        assert!(!r.is_valid(1));
        assert_eq!(r.try_get_f64(0), Some(1.5));
        assert_eq!(r.try_get_f64(1), None);
        assert_eq!(r.try_get_f64(2), Some(-2.72));
    }

    #[test]
    fn reader_from_bools() {
        let r = MockVectorReader::from_bools([Some(true), None, Some(false)]);
        assert_eq!(r.row_count(), 3);
        assert!(r.is_valid(0));
        assert!(!r.is_valid(1));
        assert_eq!(r.try_get_bool(0), Some(true));
        assert_eq!(r.try_get_bool(1), None);
        assert_eq!(r.try_get_bool(2), Some(false));
    }

    #[test]
    fn writer_typed_getters_i32() {
        let mut w = MockVectorWriter::new(2);
        w.write_i32(0, 42);
        w.set_null(1);
        assert_eq!(w.try_get_i32(0), Some(42));
        assert_eq!(w.try_get_i32(1), None);
        // Wrong type returns None
        assert_eq!(w.try_get_i64(0), None);
    }

    #[test]
    fn writer_typed_getters_f64() {
        let mut w = MockVectorWriter::new(2);
        w.write_f64(0, 2.72);
        w.set_null(1);
        assert_eq!(w.try_get_f64(0), Some(2.72));
        assert_eq!(w.try_get_f64(1), None);
    }

    #[test]
    fn writer_typed_getters_bool() {
        let mut w = MockVectorWriter::new(2);
        w.write_bool(0, true);
        w.write_bool(1, false);
        assert_eq!(w.try_get_bool(0), Some(true));
        assert_eq!(w.try_get_bool(1), Some(false));
    }

    #[test]
    fn writer_u16_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_u16(0, 12345);
        assert!(matches!(w.get(0), Some(MockDuckValue::U16(12345))));
    }

    #[test]
    fn writer_i128_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_i128(0, i128::MAX);
        assert!(matches!(w.get(0), Some(MockDuckValue::I128(v)) if *v == i128::MAX));
    }

    #[test]
    fn writer_interval_round_trip() {
        let interval = DuckInterval {
            months: 1,
            days: 2,
            micros: 3_000_000,
        };
        let mut w = MockVectorWriter::new(1);
        w.write_interval(0, interval);
        assert_eq!(w.try_get_interval(0), Some(interval));
    }

    #[test]
    fn reader_interval_round_trip() {
        let interval = DuckInterval {
            months: 6,
            days: 15,
            micros: 500_000,
        };
        let r = MockVectorReader::new([Some(MockDuckValue::Interval(interval))]);
        assert_eq!(r.try_get_interval(0), Some(interval));
    }

    #[test]
    fn reader_wrong_type_returns_none() {
        let r = MockVectorReader::from_i64s([Some(42)]);
        assert_eq!(r.try_get_i32(0), None);
        assert_eq!(r.try_get_f64(0), None);
        assert_eq!(r.try_get_bool(0), None);
        assert_eq!(r.try_get_str(0), None);
        assert_eq!(r.try_get_interval(0), None);
    }

    #[test]
    fn writer_is_empty() {
        let w = MockVectorWriter::new(0);
        assert!(w.is_empty());
        let w2 = MockVectorWriter::new(1);
        assert!(!w2.is_empty());
    }

    #[test]
    fn writer_try_get_i8_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_i8(0, -42);
        assert_eq!(w.try_get_i8(0), Some(-42));
    }

    #[test]
    fn writer_try_get_i16_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_i16(0, 1234);
        assert_eq!(w.try_get_i16(0), Some(1234));
    }

    #[test]
    fn writer_try_get_u8_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_u8(0, 255);
        assert_eq!(w.try_get_u8(0), Some(255));
    }

    #[test]
    fn writer_try_get_u16_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_u16(0, 60000);
        assert_eq!(w.try_get_u16(0), Some(60000));
    }

    #[test]
    fn writer_try_get_u32_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_u32(0, 123_456);
        assert_eq!(w.try_get_u32(0), Some(123_456));
    }

    #[test]
    fn writer_try_get_u64_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_u64(0, u64::MAX);
        assert_eq!(w.try_get_u64(0), Some(u64::MAX));
    }

    #[test]
    fn writer_try_get_f32_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_f32(0, 2.5);
        assert_eq!(w.try_get_f32(0), Some(2.5));
    }

    #[test]
    fn writer_try_get_i128_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_i128(0, i128::MIN);
        assert_eq!(w.try_get_i128(0), Some(i128::MIN));
    }

    #[test]
    fn reader_from_i8s() {
        let r = MockVectorReader::from_i8s([Some(1), None, Some(-1)]);
        assert_eq!(r.row_count(), 3);
        assert_eq!(r.try_get_i8(0), Some(1));
        assert!(!r.is_valid(1));
        assert_eq!(r.try_get_i8(2), Some(-1));
    }

    #[test]
    fn reader_from_i16s() {
        let r = MockVectorReader::from_i16s([Some(100), None]);
        assert_eq!(r.try_get_i16(0), Some(100));
        assert!(!r.is_valid(1));
    }

    #[test]
    fn reader_from_u8s() {
        let r = MockVectorReader::from_u8s([Some(255), None]);
        assert_eq!(r.try_get_u8(0), Some(255));
    }

    #[test]
    fn reader_from_u16s() {
        let r = MockVectorReader::from_u16s([Some(60000)]);
        assert_eq!(r.try_get_u16(0), Some(60000));
    }

    #[test]
    fn reader_from_u32s() {
        let r = MockVectorReader::from_u32s([Some(999_999)]);
        assert_eq!(r.try_get_u32(0), Some(999_999));
    }

    #[test]
    fn reader_from_u64s() {
        let r = MockVectorReader::from_u64s([Some(u64::MAX), None]);
        assert_eq!(r.try_get_u64(0), Some(u64::MAX));
        assert!(!r.is_valid(1));
    }

    #[test]
    fn reader_from_f32s() {
        let r = MockVectorReader::from_f32s([Some(1.5), None]);
        assert_eq!(r.try_get_f32(0), Some(1.5));
    }

    #[test]
    fn reader_from_i128s() {
        let r = MockVectorReader::from_i128s([Some(i128::MAX), None]);
        assert_eq!(r.try_get_i128(0), Some(i128::MAX));
    }

    #[test]
    fn reader_from_intervals() {
        let iv = DuckInterval {
            months: 1,
            days: 2,
            micros: 3,
        };
        let r = MockVectorReader::from_intervals([Some(iv), None]);
        assert_eq!(r.try_get_interval(0), Some(iv));
        assert!(!r.is_valid(1));
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

    #[test]
    fn writer_blob_round_trip() {
        let mut w = MockVectorWriter::new(2);
        w.write_blob(0, b"hello bytes");
        w.set_null(1);
        assert_eq!(w.try_get_blob(0), Some(b"hello bytes".as_slice()));
        assert_eq!(w.try_get_blob(1), None);
        // Wrong type returns None
        assert_eq!(w.try_get_i64(0), None);
    }

    #[test]
    fn writer_uuid_round_trip() {
        let mut w = MockVectorWriter::new(1);
        let uuid_val: i128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
        w.write_uuid(0, uuid_val);
        assert_eq!(w.try_get_uuid(0), Some(uuid_val));
    }

    #[test]
    fn writer_date_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_date(0, 19815); // days since epoch
        assert_eq!(w.try_get_i32(0), Some(19815));
    }

    #[test]
    fn writer_timestamp_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_timestamp(0, 1_711_756_800_000_000); // micros since epoch
        assert_eq!(w.try_get_i64(0), Some(1_711_756_800_000_000));
    }

    #[test]
    fn writer_time_round_trip() {
        let mut w = MockVectorWriter::new(1);
        w.write_time(0, 43_200_000_000); // noon in micros
        assert_eq!(w.try_get_i64(0), Some(43_200_000_000));
    }

    #[test]
    fn reader_blob_round_trip() {
        let r = MockVectorReader::from_blobs([Some(b"data".as_slice()), None]);
        assert_eq!(r.try_get_blob(0), Some(b"data".as_slice()));
        assert_eq!(r.try_get_blob(1), None);
        assert!(!r.is_valid(1));
    }

    #[test]
    fn reader_uuid_round_trip() {
        let uuid_val: i128 = 0x0ead_beef_cafe_babe_1234_5678_9abc_def0;
        let r = MockVectorReader::from_i128s([Some(uuid_val)]);
        assert_eq!(r.try_get_uuid(0), Some(uuid_val));
    }
}
