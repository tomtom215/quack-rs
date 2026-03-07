// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! RAII wrapper for `duckdb_logical_type`.
//!
//! # Pitfall L7: `LogicalType` memory leak
//!
//! Every `duckdb_create_logical_type` call allocates memory that must be freed
//! with `duckdb_destroy_logical_type`. Forgetting to call the destructor leaks
//! memory. [`LogicalType`] implements `Drop` to prevent this.

use crate::types::TypeId;
use libduckdb_sys::{duckdb_create_logical_type, duckdb_destroy_logical_type, duckdb_logical_type};

/// An RAII wrapper around a `duckdb_logical_type` handle.
///
/// Created from a [`TypeId`], this type ensures `duckdb_destroy_logical_type`
/// is called when it is dropped. This prevents the memory leak described in
/// [Pitfall L7](https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md).
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::types::{LogicalType, TypeId};
///
/// // Requires DuckDB runtime to be initialized (i.e., loaded as an extension).
/// let lt = LogicalType::new(TypeId::BigInt);
/// // `lt` is automatically destroyed when it goes out of scope
/// ```
pub struct LogicalType {
    inner: duckdb_logical_type,
}

impl LogicalType {
    /// Creates a new `LogicalType` for the given `TypeId`.
    ///
    /// Calls `duckdb_create_logical_type` internally.
    ///
    /// # Panics
    ///
    /// Panics if `duckdb_create_logical_type` returns a null pointer (should never
    /// happen for supported types, but is checked defensively).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // Requires DuckDB runtime (called from within a loaded extension).
    /// let lt = LogicalType::new(TypeId::Timestamp);
    /// assert!(!lt.as_raw().is_null());
    /// ```
    #[must_use]
    pub fn new(type_id: TypeId) -> Self {
        // SAFETY: `duckdb_create_logical_type` is safe to call with any valid DUCKDB_TYPE.
        // It returns a heap-allocated handle that must be freed with duckdb_destroy_logical_type.
        let inner = unsafe { duckdb_create_logical_type(type_id.to_duckdb_type()) };
        assert!(!inner.is_null(), "duckdb_create_logical_type returned null");
        Self { inner }
    }

    /// Returns the underlying raw `duckdb_logical_type` handle.
    ///
    /// # Safety note
    ///
    /// Do not call `duckdb_destroy_logical_type` on the returned handle; that is
    /// handled by this type's `Drop` implementation.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_logical_type {
        self.inner
    }

    /// Consumes this `LogicalType` and returns the raw handle without destroying it.
    ///
    /// The caller is responsible for calling `duckdb_destroy_logical_type` on the
    /// returned handle.
    #[must_use]
    pub const fn into_raw(self) -> duckdb_logical_type {
        let raw = self.inner;
        // Prevent Drop from running by wrapping in ManuallyDrop
        std::mem::forget(self);
        raw
    }
}

impl Drop for LogicalType {
    fn drop(&mut self) {
        // SAFETY: `self.inner` was created by `duckdb_create_logical_type` and has not
        // been transferred elsewhere. It is safe to destroy exactly once here.
        unsafe {
            duckdb_destroy_logical_type(&raw mut self.inner);
        }
    }
}

// LogicalType is not Clone or Copy because the underlying handle is not reference-counted.
// If you need to pass it to multiple places, use `as_raw()` to borrow the handle temporarily.

#[cfg(test)]
mod tests {
    // Note: LogicalType tests that call DuckDB API (duckdb_create_logical_type)
    // require a running DuckDB runtime and are covered in tests/integration_test.rs.
    // The `loadable-extension` feature uses lazy-initialized function pointers
    // that cannot be called without a prior call to duckdb_rs_extension_api_init.

    #[test]
    fn size_of_logical_type_struct() {
        use super::LogicalType;
        // LogicalType must be pointer-sized (it contains a single pointer).
        assert_eq!(
            std::mem::size_of::<LogicalType>(),
            std::mem::size_of::<*mut ()>()
        );
    }
}
