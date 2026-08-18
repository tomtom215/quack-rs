// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Safe typed writing to `DuckDB` result vectors.
//!
//! [`VectorWriter`] provides safe methods for writing typed values and NULL
//! flags to a `DuckDB` output vector from within a `finalize` callback.
//!
//! # Pitfall L4: `ensure_validity_writable`
//!
//! When writing NULL values, you must call `duckdb_vector_ensure_validity_writable`
//! before `duckdb_vector_get_validity`. If you skip this call, `get_validity`
//! returns an uninitialized pointer that will cause a segfault or silent corruption.
//!
//! [`VectorWriter::set_null`] calls `ensure_validity_writable` automatically.

use libduckdb_sys::{
    duckdb_validity_set_row_invalid, duckdb_validity_set_row_valid, duckdb_vector,
    duckdb_vector_assign_string_element_len, duckdb_vector_ensure_validity_writable,
    duckdb_vector_get_data, duckdb_vector_get_validity, idx_t,
};

/// A typed writer for a `DuckDB` output vector in a `finalize` callback.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::vector::VectorWriter;
/// use libduckdb_sys::duckdb_vector;
///
/// // Inside finalize:
/// // let mut writer = unsafe { VectorWriter::new(result_vector) };
/// // for row in 0..count {
/// //     if let Some(val) = compute_result(row) {
/// //         unsafe { writer.write_i64(row, val) };
/// //     } else {
/// //         unsafe { writer.set_null(row) };
/// //     }
/// // }
/// ```
pub struct VectorWriter {
    vector: duckdb_vector,
    data: *mut u8,
    /// Lazily-resolved validity bitmap.
    ///
    /// `duckdb_vector_ensure_validity_writable` allocates the mask on first use
    /// and is a no-op afterwards, and the resulting pointer is stable for the
    /// vector's lifetime. Caching it turns "two FFI calls per NULL" into "two
    /// FFI calls per vector", which matters when a column is mostly NULL: a full
    /// 2048-row vector went from 4096 calls to 2.
    validity: *mut u64,
}

impl VectorWriter {
    /// Creates a new `VectorWriter` for the given result vector.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` output vector obtained in a `finalize`
    /// callback. The vector must not be destroyed while this writer is live.
    pub unsafe fn new(vector: duckdb_vector) -> Self {
        // SAFETY: Caller guarantees vector is valid.
        let data = unsafe { duckdb_vector_get_data(vector) }.cast::<u8>();
        Self {
            vector,
            data,
            validity: core::ptr::null_mut(),
        }
    }

    /// Creates a `VectorWriter` directly from a raw `duckdb_vector` handle.
    ///
    /// Use this when you need to write into a child vector (e.g., a STRUCT field
    /// or LIST element vector) obtained from
    /// [`StructVector::get_child`][crate::vector::complex::StructVector::get_child] or
    /// [`ListVector::get_child`][crate::vector::complex::ListVector::get_child].
    ///
    /// # Safety
    ///
    /// `vector` must be a valid, writable `duckdb_vector`. The vector must not be
    /// destroyed while this writer is live.
    pub unsafe fn from_vector(vector: duckdb_vector) -> Self {
        // SAFETY: caller guarantees vector is valid.
        let data = unsafe { duckdb_vector_get_data(vector) }.cast::<u8>();
        Self {
            vector,
            data,
            validity: core::ptr::null_mut(),
        }
    }

    /// Writes an `i8` (TINYINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TINYINT` type.
    #[inline]
    pub const unsafe fn write_i8(&mut self, idx: usize, value: i8) {
        // SAFETY: data points to a valid writable TINYINT array. idx is in bounds.
        unsafe { core::ptr::write_unaligned(self.data.add(idx).cast::<i8>(), value) };
    }

    /// Writes an `i16` (SMALLINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_i16(&mut self, idx: usize, value: i16) {
        // SAFETY: 2-byte aligned write to valid SMALLINT vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 2).cast::<i16>(), value) };
    }

    /// Writes an `i32` (INTEGER) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_i32(&mut self, idx: usize, value: i32) {
        // SAFETY: 4-byte aligned write to valid INTEGER vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 4).cast::<i32>(), value) };
    }

    /// Writes an `i64` (BIGINT / TIMESTAMP) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_i64(&mut self, idx: usize, value: i64) {
        // SAFETY: 8-byte aligned write to valid BIGINT vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 8).cast::<i64>(), value) };
    }

    /// Writes a `u8` (UTINYINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_u8(&mut self, idx: usize, value: u8) {
        // SAFETY: 1-byte write to valid UTINYINT vector.
        unsafe { *self.data.add(idx) = value };
    }

    /// Writes a `u32` (UINTEGER) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_u32(&mut self, idx: usize, value: u32) {
        // SAFETY: 4-byte aligned write to valid UINTEGER vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 4).cast::<u32>(), value) };
    }

    /// Writes a `u64` (UBIGINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_u64(&mut self, idx: usize, value: u64) {
        // SAFETY: 8-byte aligned write to valid UBIGINT vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 8).cast::<u64>(), value) };
    }

    /// Writes an `f32` (FLOAT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_f32(&mut self, idx: usize, value: f32) {
        // SAFETY: 4-byte aligned write to valid FLOAT vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 4).cast::<f32>(), value) };
    }

    /// Writes an `f64` (DOUBLE) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_f64(&mut self, idx: usize, value: f64) {
        // SAFETY: 8-byte aligned write to valid DOUBLE vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 8).cast::<f64>(), value) };
    }

    /// Writes a `bool` (BOOLEAN) value at row `idx`.
    ///
    /// Booleans are stored as a single byte: `1` for `true`, `0` for `false`.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `BOOLEAN` type.
    #[inline]
    pub unsafe fn write_bool(&mut self, idx: usize, value: bool) {
        // SAFETY: BOOLEAN stored as 1 byte.
        unsafe { *self.data.add(idx) = u8::from(value) };
    }

    /// Writes an `i128` (HUGEINT) value at row `idx`.
    ///
    /// `DuckDB` stores HUGEINT as `{ lower: u64, upper: i64 }` in little-endian
    /// layout, totaling 16 bytes per value.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `HUGEINT` type.
    #[inline]
    pub const unsafe fn write_i128(&mut self, idx: usize, value: i128) {
        // SAFETY: HUGEINT = { lower: u64, upper: i64 } = 16 bytes.
        let base = unsafe { self.data.add(idx * 16) };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lower = value as u64;
        #[allow(clippy::cast_possible_truncation)]
        let upper = (value >> 64) as i64;
        unsafe {
            core::ptr::write_unaligned(base.cast::<u64>(), lower);
            core::ptr::write_unaligned(base.add(8).cast::<i64>(), upper);
        }
    }

    /// Writes a `u16` (USMALLINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`write_i8`][Self::write_i8].
    #[inline]
    pub const unsafe fn write_u16(&mut self, idx: usize, value: u16) {
        // SAFETY: 2-byte aligned write to valid USMALLINT vector.
        unsafe { core::ptr::write_unaligned(self.data.add(idx * 2).cast::<u16>(), value) };
    }

    /// Writes a VARCHAR string value at row `idx`.
    ///
    /// This uses `duckdb_vector_assign_string_element_len` which handles both
    /// the inline (≤12 bytes) and pointer (>12 bytes) storage formats
    /// automatically. `DuckDB` manages the memory for the string data.
    ///
    /// # Note on very long strings
    ///
    /// If `value.len()` exceeds `idx_t::MAX` (2^64 − 1 on 64-bit platforms),
    /// the length is silently clamped to `idx_t::MAX`. In practice, this limit
    /// is unreachable on any current hardware (≈18 exabytes), so no explicit
    /// error path is provided.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `VARCHAR` type.
    pub unsafe fn write_varchar(&mut self, idx: usize, value: &str) {
        // SAFETY: self.vector is valid per constructor's contract.
        // duckdb_vector_assign_string_element_len copies the string data.
        unsafe {
            duckdb_vector_assign_string_element_len(
                self.vector,
                idx as idx_t,
                value.as_ptr().cast::<std::os::raw::c_char>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            );
        }
    }

    /// Writes a `DATE` value at row `idx` as days since the Unix epoch.
    ///
    /// `DuckDB` stores DATE as a 4-byte `i32`. This is a semantic alias for
    /// [`write_i32`][Self::write_i32].
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `DATE` type.
    #[inline]
    pub const unsafe fn write_date(&mut self, idx: usize, days_since_epoch: i32) {
        // SAFETY: DATE is stored as i32.
        unsafe { self.write_i32(idx, days_since_epoch) };
    }

    /// Writes a `TIMESTAMP` value at row `idx` as microseconds since the Unix epoch.
    ///
    /// `DuckDB` stores TIMESTAMP as an 8-byte `i64`. This is a semantic alias for
    /// [`write_i64`][Self::write_i64].
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMESTAMP` type.
    #[inline]
    pub const unsafe fn write_timestamp(&mut self, idx: usize, micros_since_epoch: i64) {
        // SAFETY: TIMESTAMP is stored as i64.
        unsafe { self.write_i64(idx, micros_since_epoch) };
    }

    /// Writes a `TIME` value at row `idx` as microseconds since midnight.
    ///
    /// `DuckDB` stores TIME as an 8-byte `i64`. This is a semantic alias for
    /// [`write_i64`][Self::write_i64].
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIME` type.
    #[inline]
    pub const unsafe fn write_time(&mut self, idx: usize, micros_since_midnight: i64) {
        // SAFETY: TIME is stored as i64.
        unsafe { self.write_i64(idx, micros_since_midnight) };
    }

    /// Writes an INTERVAL value at row `idx`.
    ///
    /// `DuckDB` stores INTERVAL as `{ months: i32, days: i32, micros: i64 }` in a
    /// 16-byte layout. This method writes all three components at the correct offsets.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `INTERVAL` type.
    #[inline]
    pub const unsafe fn write_interval(
        &mut self,
        idx: usize,
        value: crate::interval::DuckInterval,
    ) {
        // SAFETY: INTERVAL = { months: i32 @ 0, days: i32 @ 4, micros: i64 @ 8 } = 16 bytes.
        let base = unsafe { self.data.add(idx * 16) };
        unsafe {
            core::ptr::write_unaligned(base.cast::<i32>(), value.months);
            core::ptr::write_unaligned(base.add(4).cast::<i32>(), value.days);
            core::ptr::write_unaligned(base.add(8).cast::<i64>(), value.micros);
        }
    }

    /// Writes a `BLOB` (binary) value at row `idx`.
    ///
    /// This uses the same underlying storage as VARCHAR — `DuckDB` stores BLOBs
    /// using `duckdb_vector_assign_string_element_len`, which copies the data.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `BLOB` type.
    pub unsafe fn write_blob(&mut self, idx: usize, value: &[u8]) {
        // SAFETY: BLOB uses the same storage as VARCHAR.
        unsafe {
            duckdb_vector_assign_string_element_len(
                self.vector,
                idx as idx_t,
                value.as_ptr().cast::<std::os::raw::c_char>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            );
        }
    }

    /// Writes a `UUID` value at row `idx`.
    ///
    /// `DuckDB` stores UUID as a HUGEINT (128-bit integer). This is a semantic
    /// alias for [`write_i128`][Self::write_i128].
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `UUID` type.
    #[inline]
    pub const unsafe fn write_uuid(&mut self, idx: usize, value: i128) {
        // SAFETY: UUID is stored as HUGEINT (i128).
        unsafe { self.write_i128(idx, value) };
    }

    /// Writes a VARCHAR string value at row `idx`.
    ///
    /// This is an alias for [`write_varchar`][VectorWriter::write_varchar] provided
    /// for discoverability — extension authors often look for `write_str` first.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `VARCHAR` type.
    #[inline]
    pub unsafe fn write_str(&mut self, idx: usize, value: &str) {
        // SAFETY: Delegates to write_varchar; same contract.
        unsafe { self.write_varchar(idx, value) };
    }

    /// Writes a `u128` (UHUGEINT) value at row `idx`.
    ///
    /// `DuckDB` stores UHUGEINT as `{ lower: u64, upper: u64 }` in little-endian
    /// layout, totalling 16 bytes per value.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `UHUGEINT` type.
    #[inline]
    pub const unsafe fn write_u128(&mut self, idx: usize, value: u128) {
        // SAFETY: UHUGEINT = { lower: u64, upper: u64 } = 16 bytes.
        let base = unsafe { self.data.add(idx * 16) };
        #[allow(clippy::cast_possible_truncation)]
        let lower = value as u64;
        #[allow(clippy::cast_possible_truncation)]
        let upper = (value >> 64) as u64;
        unsafe {
            core::ptr::write_unaligned(base.cast::<u64>(), lower);
            core::ptr::write_unaligned(base.add(8).cast::<u64>(), upper);
        }
    }

    /// Writes a `TIMESTAMP WITH TIME ZONE` value at row `idx`, as microseconds
    /// since the Unix epoch in UTC.
    ///
    /// `TIMESTAMPTZ` shares `TIMESTAMP`'s 8-byte `i64` storage; only the logical
    /// type differs.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMESTAMPTZ` type.
    #[inline]
    pub const unsafe fn write_timestamp_tz(&mut self, idx: usize, micros_since_epoch: i64) {
        // SAFETY: TIMESTAMPTZ is stored as i64 microseconds.
        unsafe { self.write_i64(idx, micros_since_epoch) };
    }

    /// Writes a `TIMESTAMP_S` value at row `idx`, as seconds since the epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMESTAMP_S` type.
    #[inline]
    pub const unsafe fn write_timestamp_s(&mut self, idx: usize, seconds_since_epoch: i64) {
        // SAFETY: TIMESTAMP_S is stored as i64 seconds.
        unsafe { self.write_i64(idx, seconds_since_epoch) };
    }

    /// Writes a `TIMESTAMP_MS` value at row `idx`, as milliseconds since the
    /// epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMESTAMP_MS` type.
    #[inline]
    pub const unsafe fn write_timestamp_ms(&mut self, idx: usize, millis_since_epoch: i64) {
        // SAFETY: TIMESTAMP_MS is stored as i64 milliseconds.
        unsafe { self.write_i64(idx, millis_since_epoch) };
    }

    /// Writes a `TIMESTAMP_NS` value at row `idx`, as nanoseconds since the
    /// epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMESTAMP_NS` type.
    #[inline]
    pub const unsafe fn write_timestamp_ns(&mut self, idx: usize, nanos_since_epoch: i64) {
        // SAFETY: TIMESTAMP_NS is stored as i64 nanoseconds.
        unsafe { self.write_i64(idx, nanos_since_epoch) };
    }

    /// Writes a `TIME WITH TIME ZONE` value at row `idx`.
    ///
    /// `bits` is `DuckDB`'s packed representation; build one with
    /// [`datetime::time_tz_bits`][crate::datetime::time_tz_bits] rather than
    /// assembling it by hand.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `TIMETZ` type.
    #[inline]
    pub const unsafe fn write_time_tz(&mut self, idx: usize, bits: u64) {
        // SAFETY: TIMETZ is stored as a 64-bit packed value.
        unsafe { self.write_u64(idx, bits) };
    }

    /// Writes a `DECIMAL` value at row `idx` from its unscaled representation.
    ///
    /// `DuckDB` stores a `DECIMAL` in the narrowest integer that fits its
    /// declared width — `i16` up to 4 digits, `i32` up to 9, `i64` up to 18, and
    /// `i128` beyond — so the width must match the column's type. Get it from
    /// [`LogicalType::decimal_width`][crate::types::LogicalType::decimal_width].
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    /// - The vector must have `DECIMAL` type with exactly this `width`.
    #[inline]
    pub const unsafe fn write_decimal(&mut self, idx: usize, width: u8, unscaled: i128) {
        // SAFETY: the caller guarantees `width` matches the column's declared
        // width, which fixes the physical storage type.
        unsafe {
            #[allow(clippy::cast_possible_truncation)]
            if width <= 4 {
                self.write_i16(idx, unscaled as i16);
            } else if width <= 9 {
                self.write_i32(idx, unscaled as i32);
            } else if width <= 18 {
                self.write_i64(idx, unscaled as i64);
            } else {
                self.write_i128(idx, unscaled);
            }
        }
    }

    /// Marks row `idx` as NULL in the output vector.
    ///
    /// # Pitfall L4: `ensure_validity_writable`
    ///
    /// This method calls `duckdb_vector_ensure_validity_writable` before
    /// `duckdb_vector_get_validity`, which is required before writing any NULL
    /// flags. Forgetting this call returns an uninitialized pointer.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    pub unsafe fn set_null(&mut self, idx: usize) {
        // SAFETY: self.vector is valid per constructor's contract.
        let validity = unsafe { self.writable_validity() };
        // SAFETY: validity is now initialized and idx is in bounds per caller's contract.
        unsafe {
            duckdb_validity_set_row_invalid(validity, idx as idx_t);
        }
    }

    /// Marks every row in `range` as NULL.
    ///
    /// Equivalent to calling [`set_null`][Self::set_null] for each index, but
    /// resolves the validity bitmap once.
    ///
    /// # Safety
    ///
    /// Every index in `range` must be within the vector's capacity.
    pub unsafe fn set_null_range(&mut self, range: core::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        // SAFETY: self.vector is valid per constructor's contract.
        let validity = unsafe { self.writable_validity() };
        for idx in range {
            // SAFETY: idx is in bounds per caller's contract.
            unsafe { duckdb_validity_set_row_invalid(validity, idx as idx_t) };
        }
    }

    /// Resolves (once) and returns the writable validity bitmap pointer.
    ///
    /// # Pitfall L4: `ensure_validity_writable`
    ///
    /// `duckdb_vector_get_validity` returns an unusable pointer until
    /// `duckdb_vector_ensure_validity_writable` has allocated the mask. This
    /// does both, then caches the result — `EnsureWritable` is a no-op after the
    /// first call and the pointer is stable for the vector's lifetime.
    ///
    /// # Safety
    ///
    /// `self.vector` must still be a valid, flat, writable vector.
    unsafe fn writable_validity(&mut self) -> *mut u64 {
        if self.validity.is_null() {
            // SAFETY: self.vector is valid per constructor's contract.
            unsafe { duckdb_vector_ensure_validity_writable(self.vector) };
            // SAFETY: the mask was just allocated, so the pointer is usable.
            self.validity = unsafe { duckdb_vector_get_validity(self.vector) };
        }
        self.validity
    }

    /// Marks row `idx` as valid (non-NULL) in the output vector.
    ///
    /// Use this to undo a previous [`set_null`][Self::set_null] call for a row,
    /// or to explicitly mark a row as valid after writing its value.
    ///
    /// Like [`set_null`][Self::set_null], this calls `ensure_validity_writable`
    /// before modifying the validity bitmap.
    ///
    /// # Safety
    ///
    /// - `idx` must be within the vector's capacity.
    pub unsafe fn set_valid(&mut self, idx: usize) {
        // SAFETY: self.vector is valid per constructor's contract.
        let validity = unsafe { self.writable_validity() };
        // SAFETY: validity is now initialized and idx is in bounds per caller's contract.
        unsafe {
            duckdb_validity_set_row_valid(validity, idx as idx_t);
        }
    }

    /// Returns the underlying raw vector handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_vector {
        self.vector
    }
}

#[cfg(test)]
mod tests {
    // Functional tests for VectorWriter require a live DuckDB instance and are
    // located in tests/integration_test.rs. Unit tests here verify the struct
    // layout and any pure-Rust logic.

    #[test]
    fn size_of_vector_writer() {
        use super::VectorWriter;
        use std::mem::size_of;
        // vector + data + cached validity pointer
        assert_eq!(size_of::<VectorWriter>(), 3 * size_of::<usize>());
    }
}
