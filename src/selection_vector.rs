// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Selection vectors (`DuckDB` 1.5.0+).
//!
//! A [`SelectionVector`] is a list of row indices used to logically reorder or
//! filter a data vector without copying its payload — the building block behind
//! `DuckDB`'s zero-copy filtering. Extensions that implement custom filtering or
//! reordering in vectorized callbacks can allocate one, fill in the indices, and
//! hand it to the relevant `DuckDB` vector operations.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::selection_vector::SelectionVector;
//!
//! // Select rows 3, 1, 4, 1, 5 (in that order) from a source vector.
//! let mut sel = SelectionVector::new(5);
//! sel.as_mut_slice().copy_from_slice(&[3, 1, 4, 1, 5]);
//! assert_eq!(sel.len(), 5);
//! ```

use libduckdb_sys::{
    duckdb_create_selection_vector, duckdb_destroy_selection_vector, duckdb_selection_vector,
    duckdb_selection_vector_get_data_ptr, idx_t, sel_t,
};

/// RAII wrapper for a `duckdb_selection_vector`.
///
/// Owns `size` 32-bit row indices ([`sel_t`]). Automatically destroyed on drop.
pub struct SelectionVector {
    sel: duckdb_selection_vector,
    len: usize,
}

impl SelectionVector {
    /// Allocates a selection vector holding `size` indices.
    ///
    /// The indices are uninitialised; fill them via
    /// [`as_mut_slice`][SelectionVector::as_mut_slice].
    #[must_use]
    pub fn new(size: usize) -> Self {
        let raw_size = idx_t::try_from(size).unwrap_or(idx_t::MAX);
        // SAFETY: duckdb_create_selection_vector allocates an owned handle.
        let sel = unsafe { duckdb_create_selection_vector(raw_size) };
        let len = if sel.is_null() { 0 } else { size };
        Self { sel, len }
    }

    /// Returns the number of indices in this selection vector.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the selection vector holds no indices.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the indices as a read-only slice.
    #[must_use]
    pub fn as_slice(&self) -> &[sel_t] {
        if self.sel.is_null() || self.len == 0 {
            return &[];
        }
        // SAFETY: self.sel is valid; the data pointer addresses `self.len` sel_t
        // elements that live as long as the selection vector.
        let ptr = unsafe { duckdb_selection_vector_get_data_ptr(self.sel) };
        if ptr.is_null() {
            return &[];
        }
        // SAFETY: ptr points to `self.len` valid, aligned sel_t values.
        unsafe { std::slice::from_raw_parts(ptr, self.len) }
    }

    /// Returns the indices as a mutable slice for filling in.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [sel_t] {
        if self.sel.is_null() || self.len == 0 {
            return &mut [];
        }
        // SAFETY: self.sel is valid; the data pointer addresses `self.len` sel_t
        // elements that live as long as the selection vector.
        let ptr = unsafe { duckdb_selection_vector_get_data_ptr(self.sel) };
        if ptr.is_null() {
            return &mut [];
        }
        // SAFETY: ptr points to `self.len` valid, aligned sel_t values and we hold
        // a unique borrow of `self`.
        unsafe { std::slice::from_raw_parts_mut(ptr, self.len) }
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_selection_vector {
        self.sel
    }
}

impl Drop for SelectionVector {
    fn drop(&mut self) {
        if !self.sel.is_null() {
            // SAFETY: self.sel is a valid handle that we own. This destroy variant
            // takes the handle by value.
            unsafe { duckdb_destroy_selection_vector(self.sel) };
        }
    }
}

#[cfg(all(test, feature = "_duckdb-testing"))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_indices() {
        // Ensure the dispatch table is populated.
        let _db = crate::testing::InMemoryDb::open().unwrap();

        let mut sel = SelectionVector::new(4);
        assert_eq!(sel.len(), 4);
        assert!(!sel.is_empty());
        sel.as_mut_slice().copy_from_slice(&[7, 0, 3, 1]);
        assert_eq!(sel.as_slice(), &[7, 0, 3, 1]);
    }
}
