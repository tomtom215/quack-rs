//! Validity bitmap helpers for `DuckDB` NULL tracking.
//!
//! `DuckDB` tracks NULL values using a packed validity bitmap. Each row's validity
//! is represented by a single bit in a `u64` array. This module provides a safe
//! wrapper that encapsulates the bit manipulation.
//!
//! # Pitfall L4: `ensure_validity_writable`
//!
//! When writing NULL values, `duckdb_vector_get_validity` returns an uninitialized
//! pointer if `duckdb_vector_ensure_validity_writable` has not been called first.
//! [`ValidityBitmap::ensure_writable`] wraps this requirement so it cannot be
//! forgotten.

use libduckdb_sys::{
    duckdb_validity_row_is_valid, duckdb_validity_set_row_invalid, duckdb_validity_set_row_valid,
    duckdb_vector, duckdb_vector_ensure_validity_writable, duckdb_vector_get_validity, idx_t,
};

/// A wrapper around a `DuckDB` validity bitmap for reading or writing NULL flags.
///
/// Obtain a writable instance via [`ValidityBitmap::ensure_writable`]. For
/// read-only access, use [`ValidityBitmap::get_read_only`].
///
/// # Safety invariants
///
/// The underlying `duckdb_vector` must remain valid for the lifetime `'v` of
/// this wrapper. Do not call `duckdb_destroy_data_chunk` while a `ValidityBitmap`
/// that references the vector is live.
pub struct ValidityBitmap<'v> {
    validity: *mut u64,
    _phantom: std::marker::PhantomData<&'v mut duckdb_vector>,
}

impl ValidityBitmap<'_> {
    /// Ensures the validity bitmap is writable and returns a [`ValidityBitmap`].
    ///
    /// This calls `duckdb_vector_ensure_validity_writable` before
    /// `duckdb_vector_get_validity`, which is required before writing NULL flags.
    ///
    /// # Pitfall L4
    ///
    /// Calling `duckdb_vector_get_validity` without first calling
    /// `duckdb_vector_ensure_validity_writable` returns an uninitialized pointer
    /// when the vector has never had a NULL written into it.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` vector handle obtained from a data chunk
    /// within the current callback invocation. The vector must not be destroyed
    /// while the returned `ValidityBitmap` is live.
    pub unsafe fn ensure_writable(vector: duckdb_vector) -> Self {
        // SAFETY: `vector` is a valid DuckDB vector handle. This call marks the
        // vector's validity buffer as writable and allocates it if it doesn't exist.
        unsafe { duckdb_vector_ensure_validity_writable(vector) };
        // SAFETY: The bitmap was just ensured to be writable and allocated.
        let validity = unsafe { duckdb_vector_get_validity(vector) };
        Self {
            validity,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Gets the validity bitmap for read-only NULL checks.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` vector handle. The returned pointer may
    /// be null if the vector has no validity bitmap (all rows are valid).
    pub unsafe fn get_read_only(vector: duckdb_vector) -> Self {
        // SAFETY: Caller guarantees `vector` is valid.
        let validity = unsafe { duckdb_vector_get_validity(vector) };
        Self {
            validity,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns `true` if the row at `idx` is valid (non-NULL).
    ///
    /// If the validity bitmap pointer is null (all rows are valid), always returns
    /// `true`.
    ///
    /// # Safety
    ///
    /// `idx` must be less than the number of rows in the vector.
    #[inline]
    pub unsafe fn row_is_valid(&self, idx: idx_t) -> bool {
        if self.validity.is_null() {
            return true;
        }
        // SAFETY: `validity` is non-null and `idx` is in bounds per the caller's contract.
        unsafe { duckdb_validity_row_is_valid(self.validity, idx) }
    }

    /// Marks the row at `idx` as NULL (invalid).
    ///
    /// # Safety
    ///
    /// - This bitmap must have been created via [`ensure_writable`][ValidityBitmap::ensure_writable].
    /// - `idx` must be less than the number of rows in the vector.
    #[inline]
    pub unsafe fn set_row_invalid(&mut self, idx: idx_t) {
        debug_assert!(
            !self.validity.is_null(),
            "set_row_invalid called on a non-writable bitmap"
        );
        // SAFETY: Bitmap is writable (ensured in constructor) and idx is in bounds.
        unsafe { duckdb_validity_set_row_invalid(self.validity, idx) };
    }

    /// Marks the row at `idx` as valid (non-NULL).
    ///
    /// # Safety
    ///
    /// - This bitmap must have been created via [`ensure_writable`][ValidityBitmap::ensure_writable].
    /// - `idx` must be less than the number of rows in the vector.
    #[inline]
    pub unsafe fn set_row_valid(&mut self, idx: idx_t) {
        debug_assert!(
            !self.validity.is_null(),
            "set_row_valid called on a non-writable bitmap"
        );
        // SAFETY: Bitmap is writable and idx is in bounds.
        unsafe { duckdb_validity_set_row_valid(self.validity, idx) };
    }

    /// Returns the raw validity bitmap pointer.
    ///
    /// This is an escape hatch for code that needs direct bitmap access.
    /// Prefer the safe methods on this type when possible.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> *mut u64 {
        self.validity
    }
}

#[cfg(test)]
mod tests {
    // Note: validity bitmap tests that require a real DuckDB vector are in
    // integration tests (tests/integration_test.rs). Unit tests here cover
    // the struct size and null-bitmap logic.

    #[test]
    fn read_only_with_null_ptr() {
        // When the validity pointer is null, all rows are considered valid.
        use super::ValidityBitmap;
        use libduckdb_sys::duckdb_vector;
        // We cannot easily construct a real vector here without a full DuckDB connection,
        // but we can test the null-pointer code path using get_read_only on a null vector.
        // This is a known-unsafe call but acceptable in a controlled test.
        // Instead, test the invariant: if validity ptr is null, row_is_valid returns true.
        let bm = ValidityBitmap {
            validity: std::ptr::null_mut(),
            _phantom: std::marker::PhantomData::<&mut duckdb_vector>,
        };
        // SAFETY: idx 0 is vacuously valid when bitmap is null.
        assert!(unsafe { bm.row_is_valid(0) });
    }
}
