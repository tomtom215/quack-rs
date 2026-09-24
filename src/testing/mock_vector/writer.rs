// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `MockVectorWriter` — an in-memory mock output vector.

use super::{MockDuckValue, MockVectorWriter};
use crate::interval::DuckInterval;

impl MockVectorWriter {
    /// Creates a writer with room for `capacity` rows, all valid and unwritten.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            rows: vec![None; capacity],
            valid: vec![true; capacity],
        }
    }

    /// Panics unless `idx` is within the capacity, as a real vector requires.
    #[track_caller]
    fn check_bounds(&self, idx: usize) {
        assert!(
            idx < self.rows.len(),
            "row {idx} is out of bounds for a mock vector of capacity {}; a real \
             DuckDB vector has a fixed capacity and writing past it corrupts memory",
            self.rows.len()
        );
    }

    /// Marks row `idx` as NULL. A later `write_*` does not undo this; see
    /// [`set_valid`][Self::set_valid].
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than the capacity.
    #[track_caller]
    pub fn set_null(&mut self, idx: usize) {
        self.check_bounds(idx);
        self.valid[idx] = false;
    }

    /// Marks row `idx` as valid again, mirroring
    /// [`VectorWriter::set_valid`][crate::vector::VectorWriter::set_valid].
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than the capacity.
    #[track_caller]
    pub fn set_valid(&mut self, idx: usize) {
        self.check_bounds(idx);
        self.valid[idx] = true;
    }

    /// Returns `true` if row `idx` has been marked NULL.
    ///
    /// A row that was never written is **not** NULL — see the type-level docs.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than the capacity.
    #[must_use]
    #[track_caller]
    pub fn is_null(&self, idx: usize) -> bool {
        self.check_bounds(idx);
        !self.valid[idx]
    }

    /// Returns `true` if a value has been written at row `idx`, whether or not
    /// the row is also marked NULL.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is not less than the capacity.
    #[must_use]
    #[track_caller]
    pub fn is_written(&self, idx: usize) -> bool {
        self.check_bounds(idx);
        self.rows[idx].is_some()
    }

    /// Returns the capacity: the number of rows this mock holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns `true` if the capacity is zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the value at row `idx` as `DuckDB` would read it.
    ///
    /// Returns `None` if the row is NULL, has never been written, or is out of
    /// bounds. Use [`is_null`][Self::is_null] and
    /// [`is_written`][Self::is_written] to tell those apart.
    #[must_use]
    pub fn get(&self, idx: usize) -> Option<&MockDuckValue> {
        if self.valid.get(idx).copied() != Some(true) {
            return None;
        }
        self.rows.get(idx).and_then(Option::as_ref)
    }

    // ── Numeric writes ──────────────────────────────────────────────────────

    /// Writes a `TINYINT` value at row `idx`.
    pub fn write_i8(&mut self, idx: usize, value: i8) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::I8(value));
    }

    /// Writes a `SMALLINT` value at row `idx`.
    pub fn write_i16(&mut self, idx: usize, value: i16) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::I16(value));
    }

    /// Writes an `INTEGER` value at row `idx`.
    pub fn write_i32(&mut self, idx: usize, value: i32) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::I32(value));
    }

    /// Writes a `BIGINT` value at row `idx`.
    pub fn write_i64(&mut self, idx: usize, value: i64) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::I64(value));
    }

    /// Writes a `UTINYINT` value at row `idx`.
    pub fn write_u8(&mut self, idx: usize, value: u8) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::U8(value));
    }

    /// Writes a `USMALLINT` value at row `idx`.
    pub fn write_u16(&mut self, idx: usize, value: u16) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::U16(value));
    }

    /// Writes a `UINTEGER` value at row `idx`.
    pub fn write_u32(&mut self, idx: usize, value: u32) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::U32(value));
    }

    /// Writes a `UBIGINT` value at row `idx`.
    pub fn write_u64(&mut self, idx: usize, value: u64) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::U64(value));
    }

    /// Writes a `FLOAT` value at row `idx`.
    pub fn write_f32(&mut self, idx: usize, value: f32) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::F32(value));
    }

    /// Writes a `DOUBLE` value at row `idx`.
    pub fn write_f64(&mut self, idx: usize, value: f64) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::F64(value));
    }

    /// Writes a `BOOLEAN` value at row `idx`.
    pub fn write_bool(&mut self, idx: usize, value: bool) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::Bool(value));
    }

    /// Writes a `HUGEINT` value at row `idx`.
    pub fn write_i128(&mut self, idx: usize, value: i128) {
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::I128(value));
    }

    /// Writes a `VARCHAR` value at row `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds or `value` is longer than
    /// [`MAX_STRING_LEN`][crate::vector::string::MAX_STRING_LEN].
    #[track_caller]
    pub fn write_varchar(&mut self, idx: usize, value: &str) {
        self.check_bounds(idx);
        if let Err(e) = crate::vector::string::check_string_len(value.len()) {
            panic!("write_varchar: {e}");
        }
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
        self.check_bounds(idx);
        self.rows[idx] = Some(MockDuckValue::Interval(value));
    }

    /// Writes a `BLOB` value at row `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds or `value` is longer than
    /// [`MAX_STRING_LEN`][crate::vector::string::MAX_STRING_LEN].
    #[track_caller]
    pub fn write_blob(&mut self, idx: usize, value: &[u8]) {
        self.check_bounds(idx);
        if let Err(e) = crate::vector::string::check_string_len(value.len()) {
            panic!("write_blob: {e}");
        }
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

    /// Writes a `UUID`'s textual 128 bits at row `idx`.
    ///
    /// Applies `DuckDB`'s top-bit flip exactly as
    /// [`VectorWriter::write_uuid`][crate::vector::VectorWriter::write_uuid]
    /// does, so a callback tested against this mock behaves the same against a
    /// real vector. [`write_i128`][Self::write_i128] writes the raw storage.
    pub fn write_uuid(&mut self, idx: usize, bits: u128) {
        self.write_i128(idx, crate::vector::uuid_to_storage(bits));
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

    /// Returns the `UUID`'s textual 128 bits at row `idx`, or `None` if NULL or
    /// wrong type.
    ///
    /// Undoes `DuckDB`'s top-bit flip exactly as
    /// [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid] does.
    /// [`try_get_i128`][Self::try_get_i128] returns the raw storage.
    #[must_use]
    pub fn try_get_uuid(&self, idx: usize) -> Option<u128> {
        self.try_get_i128(idx).map(crate::vector::uuid_from_storage)
    }
}
