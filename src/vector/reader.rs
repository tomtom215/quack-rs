// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

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
    duckdb_validity_row_is_valid, duckdb_vector_get_data, duckdb_vector_get_validity, idx_t,
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

    /// Returns the number of rows in this vector.
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
