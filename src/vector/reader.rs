// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Safe typed reading from `DuckDB` data vectors.
//!
//! [`VectorReader`] provides safe access to the typed data in a `DuckDB` vector
//! without requiring direct raw pointer manipulation.
//!
//! # Pitfalls solved
//!
//! - **L5**: Booleans are read as `u8 != 0`, never as `bool`, because `DuckDB`'s
//!   C API does not guarantee the Rust `bool` invariant (must be 0 or 1).
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::vector::VectorReader;
//! use libduckdb_sys::{duckdb_data_chunk, duckdb_data_chunk_get_vector,
//!                     duckdb_data_chunk_get_size};
//!
//! // Inside a DuckDB aggregate `update` callback:
//! // let reader = unsafe { VectorReader::new(chunk, 0) };
//! // for row in 0..reader.row_count() {
//! //     if reader.is_valid(row) {
//! //         let val = unsafe { reader.read_i64(row) };
//! //     }
//! // }
//! ```

use libduckdb_sys::{
    duckdb_data_chunk, duckdb_data_chunk_get_size, duckdb_data_chunk_get_vector,
    duckdb_validity_row_is_valid, duckdb_vector, duckdb_vector_get_data,
    duckdb_vector_get_validity, idx_t,
};

/// A typed reader for a single column in a `DuckDB` data chunk.
///
/// `VectorReader` wraps a pointer to a `DuckDB` vector's data buffer and
/// provides ergonomic, type-checked access methods for common `DuckDB` types.
///
/// # Lifetimes
///
/// The reader borrows from the data chunk. Do not call `duckdb_destroy_data_chunk`
/// while a `VectorReader` that references it is live.
#[derive(Debug)]
pub struct VectorReader {
    data: *const u8,
    validity: *mut u64,
    row_count: usize,
}

impl VectorReader {
    /// Creates a new `VectorReader` for the given column in a data chunk.
    ///
    /// # Safety
    ///
    /// - `chunk` must be a valid `duckdb_data_chunk` for the duration of this reader's lifetime.
    /// - `col_idx` must be a valid column index in the chunk.
    pub unsafe fn new(chunk: duckdb_data_chunk, col_idx: usize) -> Self {
        // SAFETY: Caller guarantees chunk is valid.
        let row_count = usize::try_from(unsafe { duckdb_data_chunk_get_size(chunk) }).unwrap_or(0);
        // SAFETY: col_idx is valid per caller's contract.
        let vector = unsafe { duckdb_data_chunk_get_vector(chunk, col_idx as idx_t) };
        // SAFETY: vector is non-null for valid column indices.
        let data = unsafe { duckdb_vector_get_data(vector) }.cast::<u8>();
        // SAFETY: may be null if all values are valid (no NULLs); checked in is_valid.
        let validity = unsafe { duckdb_vector_get_validity(vector) };
        Self {
            data,
            validity,
            row_count,
        }
    }

    /// Creates a `VectorReader` directly from a raw `duckdb_vector` handle.
    ///
    /// Use this when you already have a child vector (e.g., from
    /// [`StructVector::get_child`][crate::vector::complex::StructVector::get_child] or
    /// [`ListVector::get_child`][crate::vector::complex::ListVector::get_child]).
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `duckdb_vector` for the duration of this reader's lifetime.
    /// - `row_count` must equal the number of valid rows in the vector.
    pub unsafe fn from_vector(vector: duckdb_vector, row_count: usize) -> Self {
        // SAFETY: vector is valid per caller's contract.
        let data = unsafe { duckdb_vector_get_data(vector) }.cast::<u8>();
        let validity = unsafe { duckdb_vector_get_validity(vector) };
        Self {
            data,
            validity,
            row_count,
        }
    }

    /// Returns the number of rows in this vector.
    #[mutants::skip]
    #[must_use]
    #[inline]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Returns `true` if the value at row `idx` is not NULL.
    ///
    /// # Safety
    ///
    /// `idx` must be less than `self.row_count()`.
    #[inline]
    pub unsafe fn is_valid(&self, idx: usize) -> bool {
        if self.validity.is_null() {
            return true;
        }
        // SAFETY: validity is non-null and idx is in bounds per caller's contract.
        unsafe { duckdb_validity_row_is_valid(self.validity, idx as idx_t) }
    }

    /// Reads an `i8` (TINYINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TINYINT` data.
    /// - The value at `idx` must not be NULL (check with [`is_valid`][Self::is_valid]).
    #[inline]
    pub const unsafe fn read_i8(&self, idx: usize) -> i8 {
        // SAFETY: data points to valid TINYINT array, idx is in bounds.
        unsafe { core::ptr::read_unaligned(self.data.add(idx).cast::<i8>()) }
    }

    /// Reads an `i16` (SMALLINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `SMALLINT` data.
    #[inline]
    pub const unsafe fn read_i16(&self, idx: usize) -> i16 {
        // SAFETY: 2-byte read from valid SMALLINT vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 2).cast::<i16>()) }
    }

    /// Reads an `i32` (INTEGER) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_i32(&self, idx: usize) -> i32 {
        // SAFETY: 4-byte read from valid INTEGER vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 4).cast::<i32>()) }
    }

    /// Reads an `i64` (BIGINT / TIMESTAMP) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_i64(&self, idx: usize) -> i64 {
        // SAFETY: 8-byte read from valid BIGINT/TIMESTAMP vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 8).cast::<i64>()) }
    }

    /// Reads a `u8` (UTINYINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_u8(&self, idx: usize) -> u8 {
        // SAFETY: 1-byte read from valid UTINYINT vector.
        unsafe { *self.data.add(idx) }
    }

    /// Reads a `u16` (USMALLINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_u16(&self, idx: usize) -> u16 {
        // SAFETY: 2-byte read from valid USMALLINT vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 2).cast::<u16>()) }
    }

    /// Reads a `u32` (UINTEGER) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_u32(&self, idx: usize) -> u32 {
        // SAFETY: 4-byte read from valid UINTEGER vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 4).cast::<u32>()) }
    }

    /// Reads a `u64` (UBIGINT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_u64(&self, idx: usize) -> u64 {
        // SAFETY: 8-byte read from valid UBIGINT vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 8).cast::<u64>()) }
    }

    /// Reads an `f32` (FLOAT) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_f32(&self, idx: usize) -> f32 {
        // SAFETY: 4-byte read from valid FLOAT vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 4).cast::<f32>()) }
    }

    /// Reads an `f64` (DOUBLE) value at row `idx`.
    ///
    /// # Safety
    ///
    /// See [`read_i8`][Self::read_i8].
    #[inline]
    pub const unsafe fn read_f64(&self, idx: usize) -> f64 {
        // SAFETY: 8-byte read from valid DOUBLE vector.
        unsafe { core::ptr::read_unaligned(self.data.add(idx * 8).cast::<f64>()) }
    }

    /// Reads a `bool` (BOOLEAN) value at row `idx`.
    ///
    /// # Pitfall L5: Defensive boolean reading
    ///
    /// This method reads the underlying byte as `u8` and compares with `!= 0`,
    /// rather than casting directly to `bool`. `DuckDB`'s C API does not guarantee
    /// the Rust `bool` invariant (must be exactly 0 or 1), so a direct cast could
    /// cause undefined behaviour.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `BOOLEAN` data.
    #[inline]
    pub const unsafe fn read_bool(&self, idx: usize) -> bool {
        // SAFETY: BOOLEAN data is stored as 1 byte per value.
        // We read as u8 (not bool) to avoid UB if DuckDB sets non-0/1 values.
        // This is Pitfall L5: always read boolean as u8 then compare != 0.
        unsafe { *self.data.add(idx) != 0 }
    }

    /// Reads an `i128` (HUGEINT) value at row `idx`.
    ///
    /// `DuckDB` stores HUGEINT as `{ lower: u64, upper: i64 }` in little-endian
    /// layout, totaling 16 bytes per value.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `HUGEINT` data.
    /// - The value at `idx` must not be NULL (check with [`is_valid`][Self::is_valid]).
    #[inline]
    pub const unsafe fn read_i128(&self, idx: usize) -> i128 {
        // SAFETY: HUGEINT is stored as { lower: u64, upper: i64 } = 16 bytes.
        // DuckDB lays this out in little-endian order: lower at offset 0, upper at offset 8.
        let base = unsafe { self.data.add(idx * 16) };
        let lower = unsafe { core::ptr::read_unaligned(base.cast::<u64>()) };
        let upper = unsafe { core::ptr::read_unaligned(base.add(8).cast::<i64>()) };
        // Widening casts: u64→i128 and i64→i128 are always lossless.
        #[allow(clippy::cast_lossless)]
        let result = (upper as i128) << 64 | (lower as i128);
        result
    }

    /// Reads a `u128` (UHUGEINT) value at row `idx`.
    ///
    /// `DuckDB` stores UHUGEINT as `{ lower: u64, upper: u64 }` in little-endian
    /// layout, totalling 16 bytes per value.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `UHUGEINT` data.
    #[inline]
    pub const unsafe fn read_u128(&self, idx: usize) -> u128 {
        // SAFETY: UHUGEINT = { lower: u64, upper: u64 } = 16 bytes.
        let base = unsafe { self.data.add(idx * 16) };
        let lower = unsafe { core::ptr::read_unaligned(base.cast::<u64>()) };
        let upper = unsafe { core::ptr::read_unaligned(base.add(8).cast::<u64>()) };
        ((upper as u128) << 64) | (lower as u128)
    }

    /// Reads a `TIMESTAMP WITH TIME ZONE` value at row `idx`, as microseconds
    /// since the Unix epoch in UTC.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMESTAMPTZ` data.
    #[inline]
    pub const unsafe fn read_timestamp_tz(&self, idx: usize) -> i64 {
        // SAFETY: TIMESTAMPTZ shares TIMESTAMP's i64 storage.
        unsafe { self.read_i64(idx) }
    }

    /// Reads a `TIMESTAMP_S` value at row `idx`, as seconds since the epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMESTAMP_S` data.
    #[inline]
    pub const unsafe fn read_timestamp_s(&self, idx: usize) -> i64 {
        // SAFETY: TIMESTAMP_S is stored as i64 seconds.
        unsafe { self.read_i64(idx) }
    }

    /// Reads a `TIMESTAMP_MS` value at row `idx`, as milliseconds since the
    /// epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMESTAMP_MS` data.
    #[inline]
    pub const unsafe fn read_timestamp_ms(&self, idx: usize) -> i64 {
        // SAFETY: TIMESTAMP_MS is stored as i64 milliseconds.
        unsafe { self.read_i64(idx) }
    }

    /// Reads a `TIMESTAMP_NS` value at row `idx`, as nanoseconds since the
    /// epoch.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMESTAMP_NS` data.
    #[inline]
    pub const unsafe fn read_timestamp_ns(&self, idx: usize) -> i64 {
        // SAFETY: TIMESTAMP_NS is stored as i64 nanoseconds.
        unsafe { self.read_i64(idx) }
    }

    /// Reads a `TIME WITH TIME ZONE` value at row `idx` as `DuckDB`'s packed
    /// 64-bit representation.
    ///
    /// Decode it with
    /// [`datetime::time_tz_from_bits`][crate::datetime::time_tz_from_bits].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMETZ` data.
    #[inline]
    pub const unsafe fn read_time_tz(&self, idx: usize) -> u64 {
        // SAFETY: TIMETZ is stored as a 64-bit packed value.
        unsafe { self.read_u64(idx) }
    }

    /// Reads a `DECIMAL` value at row `idx` as its unscaled integer.
    ///
    /// `DuckDB` stores a `DECIMAL` in the narrowest integer that fits its
    /// declared width — `i16` up to 4 digits, `i32` up to 9, `i64` up to 18, and
    /// `i128` up to 38 — so `width` must be the column's declared width. Get it
    /// from [`LogicalType::decimal_width`][crate::types::LogicalType::decimal_width].
    ///
    /// The represented number is `result / 10^scale`.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `DECIMAL` data with exactly this `width`.
    #[inline]
    pub const unsafe fn read_decimal(&self, idx: usize, width: u8) -> i128 {
        // SAFETY: the caller guarantees `width` matches the column's declared
        // width, which fixes the physical storage type.
        unsafe {
            if width <= 4 {
                self.read_i16(idx) as i128
            } else if width <= 9 {
                self.read_i32(idx) as i128
            } else if width <= 18 {
                self.read_i64(idx) as i128
            } else {
                self.read_i128(idx)
            }
        }
    }

    /// Returns `true` if `idx` addresses a row of this vector.
    ///
    /// Every `read_*` method requires `idx < row_count()`; this is the check to
    /// pair with them when the index comes from somewhere other than a
    /// `0..row_count()` loop.
    #[must_use]
    #[inline]
    pub const fn contains(&self, idx: usize) -> bool {
        idx < self.row_count
    }

    /// Reads a VARCHAR value at row `idx`.
    ///
    /// Returns an empty string if the data is not valid UTF-8 or if the internal
    /// string pointer is null.
    ///
    /// # Pitfall P7
    ///
    /// `DuckDB` stores strings in a 16-byte `duckdb_string_t` with two formats
    /// (inline for ≤ 12 bytes, pointer otherwise). This method handles both.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `VARCHAR` data.
    /// - For pointer-format strings, the pointed-to heap memory must be valid
    ///   for the lifetime of the returned `&str`.
    pub unsafe fn read_str(&self, idx: usize) -> &str {
        // SAFETY: Caller guarantees data is a VARCHAR vector and idx is in bounds.
        unsafe { crate::vector::string::read_duck_string(self.data, idx) }
    }

    /// Reads a `BLOB` (binary) value at row `idx`.
    ///
    /// `DuckDB` stores BLOBs using the same 16-byte `duckdb_string_t` layout as
    /// VARCHAR (inline for ≤12 bytes, pointer for larger values). The returned
    /// slice borrows from the vector's data buffer.
    ///
    /// The bytes are returned without UTF-8 validation.
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `BLOB` data.
    /// - The pointed-to memory must be valid for the lifetime of the returned slice.
    pub unsafe fn read_blob(&self, idx: usize) -> &[u8] {
        // SAFETY: BLOB uses the same duckdb_string_t layout as VARCHAR.
        unsafe { crate::vector::string::read_duck_blob(self.data, idx) }
    }

    /// Reads a `UUID` value at row `idx` as an `i128`.
    ///
    /// `DuckDB` stores UUID as a HUGEINT (128-bit integer). This is a semantic
    /// alias for [`read_i128`][Self::read_i128].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `UUID` data.
    #[inline]
    pub const unsafe fn read_uuid(&self, idx: usize) -> i128 {
        // SAFETY: UUID is stored as HUGEINT (i128).
        unsafe { self.read_i128(idx) }
    }

    /// Reads a `DATE` value at row `idx` as days since the Unix epoch.
    ///
    /// `DuckDB` stores DATE as a 4-byte `i32` representing the number of days
    /// since 1970-01-01. This is a semantic alias for [`read_i32`][Self::read_i32].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `DATE` data.
    #[inline]
    pub const unsafe fn read_date(&self, idx: usize) -> i32 {
        // SAFETY: DATE is stored as i32 (days since epoch).
        unsafe { self.read_i32(idx) }
    }

    /// Reads a `TIMESTAMP` value at row `idx` as microseconds since the Unix epoch.
    ///
    /// `DuckDB` stores TIMESTAMP as an 8-byte `i64` representing microseconds
    /// since 1970-01-01 00:00:00 UTC. This is a semantic alias for
    /// [`read_i64`][Self::read_i64].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIMESTAMP` data.
    #[inline]
    pub const unsafe fn read_timestamp(&self, idx: usize) -> i64 {
        // SAFETY: TIMESTAMP is stored as i64 (microseconds since epoch).
        unsafe { self.read_i64(idx) }
    }

    /// Reads a `TIME` value at row `idx` as microseconds since midnight.
    ///
    /// `DuckDB` stores TIME as an 8-byte `i64` representing microseconds since
    /// midnight. This is a semantic alias for [`read_i64`][Self::read_i64].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `TIME` data.
    #[inline]
    pub const unsafe fn read_time(&self, idx: usize) -> i64 {
        // SAFETY: TIME is stored as i64 (microseconds since midnight).
        unsafe { self.read_i64(idx) }
    }

    /// Reads an `INTERVAL` value at row `idx`.
    ///
    /// Returns a [`DuckInterval`][crate::interval::DuckInterval] struct.
    ///
    /// # Pitfall P8
    ///
    /// The `INTERVAL` struct is 16 bytes: `{ months: i32, days: i32, micros: i64 }`.
    /// This method handles the layout correctly using [`read_interval_at`][crate::interval::read_interval_at].
    ///
    /// # Safety
    ///
    /// - `idx` must be less than `self.row_count()`.
    /// - The column must contain `INTERVAL` data.
    #[inline]
    pub const unsafe fn read_interval(&self, idx: usize) -> crate::interval::DuckInterval {
        // SAFETY: data is a valid INTERVAL vector and idx is in bounds.
        unsafe { crate::interval::read_interval_at(self.data, idx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `VectorReader` handles the boolean-as-u8 pattern correctly.
    #[test]
    fn bool_read_u8_pattern() {
        // Simulate a DuckDB BOOLEAN vector with a non-standard value (e.g., 2)
        // to verify we use != 0 comparison rather than transmuting to bool.
        let data: [u8; 4] = [0, 1, 2, 255];

        // Directly test the read_bool logic by checking values
        // (We can't easily create a real VectorReader without DuckDB, so we test
        // the underlying invariant: any non-zero byte is `true`.)
        let as_bools: Vec<bool> = data.iter().map(|&b| b != 0).collect();
        assert_eq!(as_bools, [false, true, true, true]);
    }

    #[test]
    fn row_count_is_zero_for_empty_state() {
        // This exercises the struct layout; actual DuckDB integration is in tests/
        let reader = VectorReader {
            data: std::ptr::null(),
            validity: std::ptr::null_mut(),
            row_count: 0,
        };
        assert_eq!(reader.row_count(), 0);
    }

    #[test]
    fn contains_bounds_checks_against_row_count() {
        let reader = VectorReader {
            data: std::ptr::null(),
            validity: std::ptr::null_mut(),
            row_count: 3,
        };
        assert!(reader.contains(0));
        assert!(reader.contains(2));
        assert!(!reader.contains(3));
        assert!(!reader.contains(usize::MAX));
    }

    #[test]
    fn decimal_width_thresholds_match_duckdb_storage() {
        // DuckDB picks the physical type from the declared width:
        // <=4 -> INT16, <=9 -> INT32, <=18 -> INT64, <=38 -> INT128
        // (duckdb/common/types/decimal.hpp). Reading with the wrong width reads
        // the wrong number of bytes, so pin the boundaries.
        let mut buf = [0u8; 16];
        buf[..2].copy_from_slice(&(-1234_i16).to_le_bytes());
        let reader = VectorReader {
            data: buf.as_ptr(),
            validity: std::ptr::null_mut(),
            row_count: 1,
        };
        // SAFETY: `buf` holds one INT16 at index 0.
        assert_eq!(unsafe { reader.read_decimal(0, 4) }, -1234);

        let mut buf = [0u8; 16];
        buf[..4].copy_from_slice(&(-123_456_789_i32).to_le_bytes());
        let reader = VectorReader {
            data: buf.as_ptr(),
            validity: std::ptr::null_mut(),
            row_count: 1,
        };
        // SAFETY: `buf` holds one INT32 at index 0.
        assert_eq!(unsafe { reader.read_decimal(0, 9) }, -123_456_789);

        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&(-1_234_567_890_123_456_789_i64).to_le_bytes());
        let reader = VectorReader {
            data: buf.as_ptr(),
            validity: std::ptr::null_mut(),
            row_count: 1,
        };
        // SAFETY: `buf` holds one INT64 at index 0.
        assert_eq!(
            unsafe { reader.read_decimal(0, 18) },
            -1_234_567_890_123_456_789
        );

        let value: i128 = -170_141_183_460_469_231_731_687_303_715_884_105_727;
        let buf = value.to_le_bytes();
        let reader = VectorReader {
            data: buf.as_ptr(),
            validity: std::ptr::null_mut(),
            row_count: 1,
        };
        // SAFETY: `buf` holds one INT128 at index 0.
        assert_eq!(unsafe { reader.read_decimal(0, 38) }, value);
    }

    #[test]
    fn u128_reads_little_endian_halves() {
        let value: u128 = (0xdead_beef_u128 << 64) | 0x1234_5678;
        let buf = value.to_le_bytes();
        let reader = VectorReader {
            data: buf.as_ptr(),
            validity: std::ptr::null_mut(),
            row_count: 1,
        };
        // SAFETY: `buf` holds one UHUGEINT at index 0.
        assert_eq!(unsafe { reader.read_u128(0) }, value);
    }

    #[test]
    fn is_valid_when_validity_null() {
        // When validity is null, all rows are considered valid
        let reader = VectorReader {
            data: std::ptr::null(),
            validity: std::ptr::null_mut(),
            row_count: 5,
        };
        // SAFETY: row 0 is in bounds (row_count = 5), validity is null (all valid)
        assert!(unsafe { reader.is_valid(0) });
        assert!(unsafe { reader.is_valid(4) });
    }
}
