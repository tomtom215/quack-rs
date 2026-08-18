// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Safe construction of `LIST` and `MAP` output vectors.
//!
//! # The hazard this exists to remove
//!
//! A `LIST` vector stores a `{ offset, length }` entry per row, plus one flat
//! child vector holding every row's elements end to end. Writing one means:
//!
//! 1. `duckdb_list_vector_reserve` — make room in the child,
//! 2. write the elements into the child,
//! 3. `duckdb_list_vector_set_size` — declare how many are valid,
//! 4. write each parent row's `{ offset, length }` entry.
//!
//! Step 1 is the trap. `duckdb_list_vector_reserve` takes a **total** capacity,
//! not an increment, and when it grows it calls `Vector::Resize`, which
//! **reallocates the child's data buffer**. Any [`VectorWriter`] made from the
//! child before that call now holds a dangling pointer — and the natural way to
//! write a list row by row is to keep one writer and reserve as you go:
//!
//! ```rust,ignore
//! // WRONG: `writer` dangles after the second reserve grows the child.
//! let mut writer = unsafe { ListVector::child_writer(vec) };
//! for (row, items) in rows.iter().enumerate() {
//!     unsafe { ListVector::reserve(vec, total + items.len()) };  // may realloc
//!     for (i, item) in items.iter().enumerate() {
//!         unsafe { writer.write_i64(total + i, *item) };          // use-after-free
//!     }
//!     total += items.len();
//! }
//! ```
//!
//! [`ListBuilder`] re-fetches the child writer after every reserve, tracks the
//! running offset, and writes the parent entries for you, so the sequence cannot
//! be got wrong.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::vector::ListBuilder;
//!
//! # fn demo(list_vector: libduckdb_sys::duckdb_vector) {
//! let rows = [vec![1_i64, 2, 3], vec![], vec![42]];
//! // SAFETY: `list_vector` is a LIST output vector with BIGINT elements.
//! let mut builder = unsafe { ListBuilder::new(list_vector) };
//! for (row, items) in rows.iter().enumerate() {
//!     // SAFETY: the child vector holds BIGINT, and `row` is in bounds.
//!     unsafe {
//!         builder.push_row(row, items.len(), |writer, base| {
//!             for (i, item) in items.iter().enumerate() {
//!                 writer.write_i64(base + i, *item);
//!             }
//!         });
//!     }
//! }
//! // SAFETY: every element promised by `push_row` was written.
//! unsafe { builder.finish() };
//! # }
//! ```
//!
//! # `MAP`
//!
//! A `MAP` is a `LIST<STRUCT{key, value}>`, so the same builder drives it —
//! [`push_map_row`][ListBuilder::push_map_row] hands the closure a writer for
//! the key child and one for the value child.

use libduckdb_sys::duckdb_vector;

use crate::vector::complex::{ListVector, MapVector, StructVector};
use crate::vector::VectorWriter;

/// `DuckDB`'s hard ceiling on a child vector's capacity.
///
/// `duckdb_list_vector_reserve` throws a C++ `OutOfRangeException` above this,
/// and the C API wrapper does not catch it — the exception would unwind into
/// Rust, which is undefined behaviour. [`ListBuilder`] refuses to make the call
/// instead. Matches `duckdb::DConstants::MAX_VECTOR_SIZE`.
pub const MAX_LIST_CHILD_CAPACITY: usize = 1 << 37;

/// Incremental builder for a `LIST` (or `MAP`) output vector.
///
/// See the [module docs][crate::vector::list_builder] for why the manual
/// sequence is easy to get wrong.
pub struct ListBuilder {
    vector: duckdb_vector,
    /// Total elements written into the child so far — the offset of the next row.
    written: usize,
    /// Capacity most recently requested, so `push_row` only reserves when it
    /// must (each reserve that grows is a reallocation plus a copy).
    reserved: usize,
    /// Set when a requested capacity exceeded [`MAX_LIST_CHILD_CAPACITY`]; the
    /// builder then stops writing rather than letting `DuckDB` throw.
    overflowed: bool,
}

impl ListBuilder {
    /// Starts building into `vector`.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid, writable `LIST` or `MAP` output vector.
    #[must_use]
    pub const unsafe fn new(vector: duckdb_vector) -> Self {
        Self {
            vector,
            written: 0,
            reserved: 0,
            overflowed: false,
        }
    }

    /// Total elements written into the child vector so far.
    #[must_use]
    #[inline]
    pub const fn element_count(&self) -> usize {
        self.written
    }

    /// Returns `true` if a requested capacity exceeded
    /// [`MAX_LIST_CHILD_CAPACITY`], after which the builder writes nothing more.
    ///
    /// Check this before [`finish`][Self::finish] if the row lengths come from
    /// untrusted input; the rows already written stay valid.
    #[must_use]
    #[inline]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Grows the child vector to hold at least `capacity` elements in total.
    ///
    /// Returns `false` — writing nothing — if `capacity` exceeds
    /// [`MAX_LIST_CHILD_CAPACITY`].
    ///
    /// # Safety
    ///
    /// `self.vector` must still be a valid LIST/MAP vector.
    unsafe fn ensure_capacity(&mut self, capacity: usize) -> bool {
        if self.overflowed {
            return false;
        }
        if capacity > MAX_LIST_CHILD_CAPACITY {
            self.overflowed = true;
            return false;
        }
        if capacity > self.reserved {
            // Grow geometrically: each reserve that grows reallocates and copies
            // the whole child, so doing it once per row is quadratic.
            let target = capacity.next_power_of_two().min(MAX_LIST_CHILD_CAPACITY);
            // SAFETY: `self.vector` is valid per this function's contract, and
            // `target` is within DuckDB's limit.
            unsafe { ListVector::reserve(self.vector, target) };
            self.reserved = target;
        }
        true
    }

    /// Appends `len` elements for parent row `row_idx`.
    ///
    /// The closure receives a [`VectorWriter`] for the child vector — freshly
    /// obtained *after* the reserve, so it is never stale — and `base`, the
    /// index the row's first element occupies. Write indices `base..base + len`.
    ///
    /// Does nothing once [`overflowed`][Self::overflowed] is set.
    ///
    /// # Safety
    ///
    /// - `row_idx` must be within the parent vector's capacity.
    /// - The closure must only write child indices in `base..base + len`, using
    ///   accessors matching the child's type.
    pub unsafe fn push_row<F>(&mut self, row_idx: usize, len: usize, write: F)
    where
        F: FnOnce(&mut VectorWriter, usize),
    {
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            return;
        }
        if len > 0 {
            // SAFETY: the child was just reserved, so this writer is fresh.
            let mut writer = unsafe { ListVector::child_writer(self.vector) };
            write(&mut writer, base);
        }
        // SAFETY: `row_idx` is in bounds per the caller's contract.
        unsafe { ListVector::set_entry(self.vector, row_idx, base as u64, len as u64) };
        self.written = base + len;
    }

    /// Appends `len` key/value pairs for parent row `row_idx` of a `MAP`.
    ///
    /// The closure receives a writer for the key child, a writer for the value
    /// child, and the base index. Write indices `base..base + len` in both.
    ///
    /// # Safety
    ///
    /// - `self.vector` must be a `MAP` vector.
    /// - `row_idx` must be within the parent vector's capacity.
    /// - The closure must only write child indices in `base..base + len`.
    pub unsafe fn push_map_row<F>(&mut self, row_idx: usize, len: usize, write: F)
    where
        F: FnOnce(&mut VectorWriter, &mut VectorWriter, usize),
    {
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            return;
        }
        if len > 0 {
            // SAFETY: the child STRUCT was just reserved, so both writers are
            // fresh. MAP children are always key at field 0, value at field 1.
            let (mut keys, mut values) = unsafe {
                let struct_child = MapVector::struct_child(self.vector);
                (
                    VectorWriter::from_vector(StructVector::get_child(struct_child, 0)),
                    VectorWriter::from_vector(StructVector::get_child(struct_child, 1)),
                )
            };
            write(&mut keys, &mut values, base);
        }
        // SAFETY: `row_idx` is in bounds per the caller's contract.
        unsafe { ListVector::set_entry(self.vector, row_idx, base as u64, len as u64) };
        self.written = base + len;
    }

    /// Declares how many child elements are valid, completing the vector.
    ///
    /// Forgetting this leaves `DuckDB` reading a child size of zero, so the
    /// column comes back as empty lists with no error.
    ///
    /// # Safety
    ///
    /// Every element promised by a [`push_row`][Self::push_row] /
    /// [`push_map_row`][Self::push_map_row] call must actually have been
    /// written.
    pub unsafe fn finish(self) {
        // SAFETY: `self.vector` is valid per the constructor's contract, and
        // `self.written` counts exactly the elements the caller wrote.
        unsafe { ListVector::set_size(self.vector, self.written) };
    }
}

#[cfg(test)]
mod tests {
    use super::MAX_LIST_CHILD_CAPACITY;

    #[test]
    fn max_capacity_matches_duckdbs_constant() {
        // duckdb::DConstants::MAX_VECTOR_SIZE = 1ULL << 37ULL. Above this,
        // ListVector::Reserve throws a C++ exception that the C API does not
        // catch, so it must never be reached from Rust.
        assert_eq!(MAX_LIST_CHILD_CAPACITY, 137_438_953_472);
    }
}
