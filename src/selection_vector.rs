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
//! # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
//! let mut sel = SelectionVector::new(5)?;
//! sel.as_mut_slice().copy_from_slice(&[3, 1, 4, 1, 5]);
//! assert_eq!(sel.len(), 5);
//! # Ok(())
//! # }
//! ```

use libduckdb_sys::{
    duckdb_create_selection_vector, duckdb_destroy_selection_vector, duckdb_selection_vector,
    duckdb_selection_vector_get_data_ptr, idx_t, sel_t,
};

use crate::error::ExtensionError;

/// Every index a [`sel_t`] can hold: `2^32`.
const SEL_T_RANGE: u64 = sel_t::MAX as u64 + 1;

/// The most indices `isize::MAX` bytes can hold, which caps any Rust slice.
const SLICE_LIMIT: u64 = (isize::MAX as usize / core::mem::size_of::<sel_t>()) as u64;

/// The largest length [`SelectionVector::new`] accepts.
///
/// This is `2^32` — the number of distinct source rows a [`sel_t`] (`u32`)
/// index can name — on 64-bit targets, and the largest `sel_t` slice that fits
/// in `isize::MAX` bytes on 32-bit ones.
///
/// `DuckDB` does no checking of its own here: `duckdb_create_selection_vector`
/// computes `size * sizeof(sel_t)` without an overflow check (so a large enough
/// `size` wraps to a tiny allocation while the caller believes it owns `size`
/// indices), and at `Allocator::MAXIMUM_ALLOC_SIZE` (`2^48` bytes) or above it
/// throws a C++ exception, which aborts an extension. Capping the length at the
/// range of `sel_t` keeps the byte count at most `2^34`, far from both. Real
/// selection vectors are sized in vector rows — `STANDARD_VECTOR_SIZE` (2048)
/// in ordinary execution — so the cap rejects only lengths no operation could
/// use.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the smaller of the two limits is at most SLICE_LIMIT, which came from a usize"
)]
pub const MAX_LEN: usize = if SEL_T_RANGE < SLICE_LIMIT {
    SEL_T_RANGE as usize
} else {
    SLICE_LIMIT as usize
};

/// RAII wrapper for a `duckdb_selection_vector`.
///
/// Owns `size` 32-bit row indices ([`sel_t`]), zero-initialised by
/// [`new`][SelectionVector::new]. Automatically destroyed on drop.
#[derive(Debug)]
pub struct SelectionVector {
    sel: duckdb_selection_vector,
    len: usize,
}

impl SelectionVector {
    /// Allocates a selection vector holding `size` indices, all zero.
    ///
    /// Fill the indices via [`as_mut_slice`][SelectionVector::as_mut_slice].
    ///
    /// `DuckDB` leaves the buffer uninitialised in release builds, so this
    /// zeroes it: every index starts as row 0, never as stale heap contents.
    ///
    /// # Errors
    ///
    /// Returns an error if `size` exceeds [`MAX_LEN`] — checked before `DuckDB`
    /// is called, because above it `DuckDB` either miscomputes the allocation
    /// size or throws — or if `DuckDB` returns a null handle or buffer.
    ///
    /// An allocation within the limit that the system cannot satisfy still
    /// makes `DuckDB` throw, which aborts the process, just as a failed Rust
    /// allocation does.
    pub fn new(size: usize) -> Result<Self, ExtensionError> {
        if size > MAX_LEN {
            return Err(ExtensionError::new(format!(
                "selection vector length {size} exceeds the maximum of {MAX_LEN}"
            )));
        }
        // `size <= MAX_LEN <= 2^32`, which always fits in a 64-bit `idx_t`.
        let raw_size = idx_t::try_from(size)
            .map_err(|_| ExtensionError::new("selection vector length does not fit in idx_t"))?;
        // SAFETY: duckdb_create_selection_vector allocates an owned handle; the
        // length check above keeps `size * sizeof(sel_t)` from overflowing and
        // below the allocator's throwing limit.
        let sel = unsafe { duckdb_create_selection_vector(raw_size) };
        if sel.is_null() {
            return Err(ExtensionError::new(
                "duckdb_create_selection_vector returned null",
            ));
        }
        let mut this = Self { sel, len: size };
        if size > 0 {
            // SAFETY: `this.sel` is a valid handle.
            let ptr = unsafe { duckdb_selection_vector_get_data_ptr(this.sel) };
            if ptr.is_null() {
                // Dropping `this` still releases the handle.
                this.len = 0;
                return Err(ExtensionError::new(
                    "duckdb_selection_vector_get_data_ptr returned null",
                ));
            }
            // SAFETY: `ptr` addresses `size` writable, aligned sel_t values that
            // DuckDB allocated but did not initialise (it only fills them under
            // `#ifdef DEBUG`); zeroing them is what makes `as_slice` sound.
            unsafe { core::ptr::write_bytes(ptr, 0, size) };
        }
        Ok(this)
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
        // SAFETY: ptr points to `self.len` aligned sel_t values, all
        // initialised by `new`; `self.len <= MAX_LEN` keeps the slice within
        // `isize::MAX` bytes.
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

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn max_len_is_the_sel_t_range_on_64_bit() {
        #[cfg(target_pointer_width = "64")]
        assert_eq!(MAX_LEN as u64, 1_u64 << 32);
        // On every target the byte count must fit a Rust slice.
        let bytes = MAX_LEN.checked_mul(core::mem::size_of::<sel_t>());
        assert!(bytes.is_some_and(|b| isize::try_from(b).is_ok()));
    }

    #[test]
    fn oversized_lengths_are_rejected_before_duckdb_is_called() {
        // No dispatch table is initialised here, so reaching DuckDB would
        // fail; an `Err` proves the check runs first. On 64-bit these include
        // `(1 << 62) + 4`, whose byte count DuckDB wraps to 16, and `1 << 47`,
        // which makes DuckDB's allocator throw.
        assert!(SelectionVector::new(MAX_LEN + 1).is_err());
        assert!(SelectionVector::new(usize::MAX).is_err());
        #[cfg(target_pointer_width = "64")]
        {
            assert!(SelectionVector::new((1 << 62) + 4).is_err());
            assert!(SelectionVector::new(1 << 47).is_err());
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

        let mut sel = SelectionVector::new(4).expect("allocate");
        assert_eq!(sel.len(), 4);
        assert!(!sel.is_empty());
        sel.as_mut_slice().copy_from_slice(&[7, 0, 3, 1]);
        assert_eq!(sel.as_slice(), &[7, 0, 3, 1]);
    }
}
