// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `MockVectorReader` — an in-memory mock input vector.

use super::{MockDuckValue, MockVectorReader};
use crate::interval::DuckInterval;

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
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Panics unless `idx` is a row of this reader, as a real vector requires.
    ///
    /// A real [`VectorReader`][crate::vector::VectorReader] has no bounds
    /// check: reading past the chunk's row count is undefined behaviour. A mock
    /// that quietly answered "NULL" there would let an off-by-one row loop
    /// pass its unit tests and then read out of bounds in production, so the
    /// mock refuses it — as [`MockVectorWriter`][super::MockVectorWriter] does
    /// for writes past its capacity.
    #[track_caller]
    fn check_bounds(&self, idx: usize) {
        assert!(
            idx < self.rows.len(),
            "row {idx} is out of bounds for a mock reader of {} row(s); a real \
             DuckDB vector reader has no bounds check and reading past the chunk's \
             row count is undefined behaviour",
            self.rows.len()
        );
    }

    /// Returns `true` if row `idx` is not NULL.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn is_valid(&self, idx: usize) -> bool {
        self.check_bounds(idx);
        self.rows[idx].is_some()
    }

    /// Returns the raw value at row `idx`, or `None` if it is NULL.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn get(&self, idx: usize) -> Option<&MockDuckValue> {
        self.check_bounds(idx);
        self.rows[idx].as_ref()
    }

    // ── Typed getters ───────────────────────────────────────────────────────

    /// Returns the `BIGINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_i64(&self, idx: usize) -> Option<i64> {
        match self.get(idx) {
            Some(MockDuckValue::I64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `INTEGER` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_i32(&self, idx: usize) -> Option<i32> {
        match self.get(idx) {
            Some(MockDuckValue::I32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `DOUBLE` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_f64(&self, idx: usize) -> Option<f64> {
        match self.get(idx) {
            Some(MockDuckValue::F64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BOOLEAN` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_bool(&self, idx: usize) -> Option<bool> {
        match self.get(idx) {
            Some(MockDuckValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `VARCHAR` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_str(&self, idx: usize) -> Option<&str> {
        match self.get(idx) {
            Some(MockDuckValue::Varchar(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the `INTERVAL` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_interval(&self, idx: usize) -> Option<DuckInterval> {
        match self.get(idx) {
            Some(MockDuckValue::Interval(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `TINYINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_i8(&self, idx: usize) -> Option<i8> {
        match self.get(idx) {
            Some(MockDuckValue::I8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `SMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_i16(&self, idx: usize) -> Option<i16> {
        match self.get(idx) {
            Some(MockDuckValue::I16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UTINYINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_u8(&self, idx: usize) -> Option<u8> {
        match self.get(idx) {
            Some(MockDuckValue::U8(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `USMALLINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_u16(&self, idx: usize) -> Option<u16> {
        match self.get(idx) {
            Some(MockDuckValue::U16(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UINTEGER` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_u32(&self, idx: usize) -> Option<u32> {
        match self.get(idx) {
            Some(MockDuckValue::U32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `UBIGINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_u64(&self, idx: usize) -> Option<u64> {
        match self.get(idx) {
            Some(MockDuckValue::U64(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `FLOAT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_f32(&self, idx: usize) -> Option<f32> {
        match self.get(idx) {
            Some(MockDuckValue::F32(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `HUGEINT` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_i128(&self, idx: usize) -> Option<i128> {
        match self.get(idx) {
            Some(MockDuckValue::I128(v)) => Some(*v),
            _ => None,
        }
    }

    /// Returns the `BLOB` value at row `idx`, or `None` if NULL or wrong type.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_blob(&self, idx: usize) -> Option<&[u8]> {
        match self.get(idx) {
            Some(MockDuckValue::Blob(v)) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// Returns the `UUID`'s textual 128 bits at row `idx`, or `None` if NULL or
    /// wrong type.
    ///
    /// Undoes `DuckDB`'s top-bit flip exactly as
    /// [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid] does.
    /// [`try_get_i128`][Self::try_get_i128] returns the raw storage.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than [`row_count`][Self::row_count].
    #[must_use]
    #[track_caller]
    pub fn try_get_uuid(&self, idx: usize) -> Option<u128> {
        self.try_get_i128(idx).map(crate::vector::uuid_from_storage)
    }
}
