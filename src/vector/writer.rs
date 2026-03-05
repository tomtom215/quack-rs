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
    duckdb_validity_set_row_invalid, duckdb_vector, duckdb_vector_ensure_validity_writable,
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
        Self { vector, data }
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
        // PITFALL L4: must call ensure_validity_writable before get_validity for NULL output.
        unsafe {
            duckdb_vector_ensure_validity_writable(self.vector);
        }
        // SAFETY: ensure_validity_writable allocates the bitmap; it is now safe to read.
        let validity = unsafe { duckdb_vector_get_validity(self.vector) };
        // SAFETY: validity is now initialized and idx is in bounds per caller's contract.
        unsafe {
            duckdb_validity_set_row_invalid(validity, idx as idx_t);
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
        // VectorWriter contains a pointer + a pointer = 2 * pointer size
        assert_eq!(size_of::<VectorWriter>(), 2 * size_of::<usize>());
    }
}
