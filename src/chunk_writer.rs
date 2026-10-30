// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Auto-sizing chunk writer for table function scan callbacks.
//!
//! [`ChunkWriter`] wraps a `duckdb_data_chunk` and counts the rows its scan
//! claims. On drop it calls `duckdb_data_chunk_set_size` with that count, which
//! removes the error-prone manual `set_size` after writing rows — a common
//! source of bugs in table functions.
//!
//! # Claim a row, then write it or give it back
//!
//! [`next_row`][ChunkWriter::next_row] **claims** the next row: it returns the
//! row's index and counts it, and the drop publishes every row counted. The
//! writer cannot see whether a claimed row was then written, because the
//! writes go through [`VectorWriter`]s that do not report back. A claimed row
//! that is never written is published anyway, holding whatever the output
//! vector last held: `DuckDB` reuses one output chunk for every scan call, so
//! that is a value from an earlier call (a wrong answer), and in a `LIST` or
//! `MAP` column written by hand it is an earlier call's `{offset, length}`
//! entry, pointing into child data this call did not write.
//!
//! So write every row you claim, or give it back with
//! [`release_row`][ChunkWriter::release_row] before the writer drops. The
//! natural loop over a source of unknown length claims one row more than the
//! source fills:
//!
//! ```rust,no_run
//! use quack_rs::chunk_writer::ChunkWriter;
//! use libduckdb_sys::{duckdb_function_info, duckdb_data_chunk};
//!
//! unsafe extern "C" fn my_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
//!     let mut source = [("a", 1_i64), ("b", 2)].into_iter();
//!     let mut cw = unsafe { ChunkWriter::new(output) };
//!     // One writer per column, fetched once: building a writer costs several
//!     // C API calls, so doing it for every row is several times slower.
//!     let mut names = unsafe { cw.writer(0) };
//!     let mut values = unsafe { cw.writer(1) };
//!     while let Some(row) = cw.next_row() {
//!         let Some((name, value)) = source.next() else {
//!             // The source ran out after `row` was claimed: give it back, or
//!             // it is published with stale contents.
//!             cw.release_row(row);
//!             break;
//!         };
//!         unsafe { names.write_varchar(row, name) };
//!         unsafe { values.write_i64(row, value) };
//!     }
//!     // set_size is called automatically when `cw` is dropped
//! }
//! ```
//!
//! Asking the source first (`for item in source { let Some(row) =
//! cw.next_row() else { break }; ... }`) never claims a row it cannot fill,
//! but takes the item that found the chunk full out of the source: keep it for
//! the next call if the source cannot be rewound.
//!
//! # Estimated impact
//!
//! Eliminates ~15 manual `set_size` calls and prevents off-by-one errors.

use libduckdb_sys::{duckdb_data_chunk, duckdb_data_chunk_set_size, idx_t};

use crate::vector::{StructWriter, VectorWriter};

/// `DuckDB`'s compile-time default vector size.
///
/// This is only a fallback for the (unreachable in practice) case where
/// `duckdb_vector_size()` reports `0`. **Never hardcode this value** — `DuckDB`
/// can be built with `-DSTANDARD_VECTOR_SIZE=<n>` and its `duckdb.h` documents
/// `duckdb_vector_size()` as the authoritative "amount of tuples that will fit
/// into a data chunk". Writing past the real capacity is a heap overflow.
pub const DEFAULT_STANDARD_VECTOR_SIZE: usize = 2048;

/// Queries the running `DuckDB` for its actual vector size.
///
/// Falls back to [`DEFAULT_STANDARD_VECTOR_SIZE`] only if the engine reports a
/// nonsensical `0`.
fn runtime_vector_size() -> usize {
    let size = usize::try_from(crate::vector::vector_size()).unwrap_or(0);
    if size == 0 {
        DEFAULT_STANDARD_VECTOR_SIZE
    } else {
        size
    }
}

/// A row-counting writer for a `DuckDB` output data chunk.
///
/// Counts the rows claimed with [`next_row`][Self::next_row] and not given
/// back with [`release_row`][Self::release_row], and calls
/// `duckdb_data_chunk_set_size` with that count on drop. Every claimed row
/// must be written or released: one that is neither is published with stale
/// contents (see the [module docs][self]).
///
/// The drop does not check that, because it cannot: the writes go through
/// [`VectorWriter`]s, which do not report back to this writer, so nothing here
/// knows whether the last claimed row was written.
#[derive(Debug)]
pub struct ChunkWriter {
    raw: duckdb_data_chunk,
    row_count: usize,
    capacity: usize,
}

impl ChunkWriter {
    /// Creates a new `ChunkWriter` for the given output data chunk.
    ///
    /// The capacity is read from `duckdb_vector_size()`, so it tracks the
    /// running engine even when `DuckDB` was built with a non-default
    /// `STANDARD_VECTOR_SIZE`. Do not assume 2048.
    ///
    /// # Safety
    ///
    /// - `chunk` must be a valid, writable `duckdb_data_chunk` obtained from a
    ///   `DuckDB` table function scan callback.
    /// - The `DuckDB` C API dispatch table must be initialised (it always is
    ///   inside a callback).
    #[inline]
    #[must_use]
    pub unsafe fn new(chunk: duckdb_data_chunk) -> Self {
        Self {
            raw: chunk,
            row_count: 0,
            capacity: runtime_vector_size(),
        }
    }

    /// Creates a new `ChunkWriter` with a custom capacity.
    ///
    /// # Safety
    ///
    /// Same as [`new`][Self::new]. `capacity` must not exceed the chunk's
    /// actual capacity.
    #[inline]
    #[must_use]
    pub const unsafe fn with_capacity(chunk: duckdb_data_chunk, capacity: usize) -> Self {
        Self {
            raw: chunk,
            row_count: 0,
            capacity,
        }
    }

    /// Returns `true` if the chunk has reached its capacity.
    #[inline]
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.row_count >= self.capacity
    }

    /// Claims the next row and returns its index, or `None` if the chunk is
    /// full.
    ///
    /// The row counts towards the size the drop publishes from this call on,
    /// whether or not it is then written. Write it, or give it back with
    /// [`release_row`][Self::release_row]: a claimed row that is neither is
    /// published holding whatever the reused output vector last held (see the
    /// [module docs][self]).
    #[inline]
    pub const fn next_row(&mut self) -> Option<usize> {
        if self.is_full() {
            return None;
        }
        let row = self.row_count;
        self.row_count += 1;
        Some(row)
    }

    /// Gives back `row`, the row the last [`next_row`][Self::next_row] claimed,
    /// so that the drop does not publish it.
    ///
    /// For a scan that claims a row before it knows it has data for it: the
    /// `while let Some(row) = cw.next_row()` loop whose source runs out (see
    /// the [module docs][self]). The next `next_row` returns the same index
    /// again. Rows are given back one at a time, most recent first; whatever
    /// was written to a row given back stays in the vector but is not
    /// published.
    ///
    /// The row is named, rather than implied, so that giving back the wrong
    /// one is caught instead of silently dropping a written row.
    ///
    /// # Panics
    ///
    /// If `row` is not the most recently claimed row still counted: below it,
    /// the rows claimed after it would have to move down; above it, the row
    /// was never claimed. Either is a bug in the scan. The callback macros
    /// ([`table_scan_callback!`](crate::table_scan_callback) and the others)
    /// report the panic as the query's error.
    #[inline]
    pub fn release_row(&mut self, row: usize) {
        assert!(
            row.checked_add(1) == Some(self.row_count),
            "ChunkWriter::release_row({row}): only the most recently claimed row can be \
             given back, and that is {}",
            self.row_count
                .checked_sub(1)
                .map_or_else(|| "none: no row is claimed".to_owned(), |r| r.to_string())
        );
        self.row_count = row;
    }

    /// Returns the number of rows claimed and not given back: the size the
    /// drop publishes.
    #[inline]
    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Returns the chunk's capacity (maximum rows).
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of columns in this data chunk.
    #[inline]
    #[must_use]
    pub fn column_count(&self) -> usize {
        // SAFETY: `self.raw` is a valid data chunk for this wrapper's lifetime.
        usize::try_from(unsafe { libduckdb_sys::duckdb_data_chunk_get_column_count(self.raw) })
            .unwrap_or(0)
    }

    /// Returns the raw `duckdb_vector` handle for the given column index.
    ///
    /// Use this when a column has a complex type (LIST, MAP, ARRAY) that requires
    /// operations beyond simple scalar writes — for example, calling
    /// [`ListVector::set_entry`][crate::vector::complex::ListVector::set_entry].
    ///
    /// # Safety
    ///
    /// `col_idx` must be less than the chunk's column count.
    #[must_use]
    pub unsafe fn vector(&self, col_idx: usize) -> libduckdb_sys::duckdb_vector {
        // SAFETY: `self.raw` is the valid output chunk of the scan callback in progress,
        // per the first `# Safety` clause of `ChunkWriter::new` / `with_capacity`.
        // `duckdb_data_chunk_get_vector` bounds-checks `col_idx` itself (null when out of
        // range, data_chunk-c.cpp); this function's `# Safety` clause keeps it in range, so
        // the result is the column's `Vector`, owned by the chunk.
        unsafe { libduckdb_sys::duckdb_data_chunk_get_vector(self.raw, col_idx as idx_t) }
    }

    /// Creates a [`VectorWriter`] for the given column index.
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than the chunk's column count.
    /// - The returned writer borrows nothing and caches pointers into the
    ///   chunk's buffers: it must not be used after the scan callback returns.
    ///
    /// Fetch each column's writer once per callback and keep it: building one
    /// costs several C API calls, which a per-row `cw.writer(i)` pays on every
    /// row (about 5× the cost of the write itself).
    pub unsafe fn writer(&self, col_idx: usize) -> VectorWriter {
        // SAFETY: self.raw is valid per constructor. col_idx is in bounds per caller.
        let vec =
            unsafe { libduckdb_sys::duckdb_data_chunk_get_vector(self.raw, col_idx as idx_t) };
        // SAFETY: vec is a valid writable vector from the output chunk.
        unsafe { VectorWriter::from_vector(vec) }
    }

    /// Creates a [`StructWriter`] for a STRUCT column at the given index.
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than the chunk's column count.
    /// - The column at `col_idx` must have a STRUCT type with `field_count` fields.
    /// - The returned writer borrows nothing and caches pointers into the
    ///   chunk's buffers: it must not be used after the scan callback returns.
    pub unsafe fn struct_writer(&self, col_idx: usize, field_count: usize) -> StructWriter {
        // SAFETY: `self.raw` is the valid output chunk of the scan callback in progress,
        // per the first `# Safety` clause of `ChunkWriter::new` / `with_capacity`.
        // `col_idx` is in range per this function's first `# Safety` clause, so
        // `duckdb_data_chunk_get_vector` (which returns null otherwise, data_chunk-c.cpp)
        // yields the chunk-owned, writable column vector.
        let vec =
            unsafe { libduckdb_sys::duckdb_data_chunk_get_vector(self.raw, col_idx as idx_t) };
        // SAFETY: `vec` is the non-null, writable column vector fetched above, owned by the
        // output chunk and so live for the scan callback; this function's second `# Safety`
        // clause is exactly `StructWriter::new`'s requirement that it be a STRUCT with
        // `field_count` fields.
        unsafe { StructWriter::new(vec, field_count) }
    }

    /// Returns the raw `duckdb_data_chunk` handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_data_chunk {
        self.raw
    }

    /// Manually sets the chunk size and consumes the writer without auto-setting
    /// size on drop. Use this if you need to override the auto-calculated size.
    ///
    /// # Safety
    ///
    /// - `size` must not exceed the chunk's capacity (typically 2048).
    /// - `size` must match the actual number of rows written.
    pub unsafe fn finish_with_size(self, size: usize) {
        // SAFETY: self.raw is valid per constructor.
        unsafe { duckdb_data_chunk_set_size(self.raw, size as idx_t) };
        // Prevent Drop from setting size again.
        std::mem::forget(self);
    }
}

impl Drop for ChunkWriter {
    fn drop(&mut self) {
        // SAFETY: self.raw is valid per constructor's contract.
        // Publish the rows claimed and not given back; that each was written
        // is the scan's to ensure (see the struct docs).
        unsafe { duckdb_data_chunk_set_size(self.raw, self.row_count as idx_t) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_of_chunk_writer() {
        assert_eq!(
            std::mem::size_of::<ChunkWriter>(),
            std::mem::size_of::<usize>() * 3 // pointer + row_count + capacity
        );
    }

    #[test]
    fn next_row_increments() {
        // We can't call Drop safely without a real chunk, so we use forget.
        let mut cw = ChunkWriter {
            raw: std::ptr::null_mut(),
            row_count: 0,
            capacity: 3,
        };
        assert_eq!(cw.next_row(), Some(0));
        assert_eq!(cw.next_row(), Some(1));
        assert_eq!(cw.next_row(), Some(2));
        assert_eq!(cw.next_row(), None);
        assert!(cw.is_full());
        assert_eq!(cw.row_count(), 3);
        // Forget to avoid calling Drop with a null pointer in FFI.
        std::mem::forget(cw);
    }

    /// A writer over no chunk, for the counting tests; never dropped, since
    /// the drop calls into `DuckDB`.
    const fn detached(capacity: usize) -> ChunkWriter {
        ChunkWriter {
            raw: std::ptr::null_mut(),
            row_count: 0,
            capacity,
        }
    }

    /// The source-runs-out loop: the row claimed last is given back, the
    /// count drops to the rows written, and the same index comes back next.
    #[test]
    fn release_row_gives_back_the_last_claimed_row() {
        let mut cw = detached(4);
        let mut source = [10, 20].into_iter();
        let mut written = Vec::new();
        while let Some(row) = cw.next_row() {
            let Some(v) = source.next() else {
                cw.release_row(row);
                break;
            };
            written.push((row, v));
        }
        assert_eq!(written, [(0, 10), (1, 20)]);
        assert_eq!(cw.row_count(), 2, "only the written rows are published");
        assert_eq!(
            cw.next_row(),
            Some(2),
            "the released index is claimed again"
        );
        // Most recent first, down to none, and a full writer can give back.
        cw.release_row(2);
        cw.release_row(1);
        cw.release_row(0);
        assert_eq!(cw.row_count(), 0);
        let mut full = detached(1);
        assert_eq!(full.next_row(), Some(0));
        assert!(full.is_full());
        full.release_row(0);
        assert!(!full.is_full());
        std::mem::forget(full);
        std::mem::forget(cw);
    }

    /// Giving back any row but the last claimed one is a bug in the scan, and
    /// says which row could have been given back. The panic is caught here
    /// so that the writer, whose drop calls into `DuckDB`, can be forgotten.
    #[test]
    fn release_row_refuses_any_row_but_the_last_claimed_one() {
        let release = |cw: &mut ChunkWriter, row: usize| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cw.release_row(row))).map_err(
                |payload| {
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .unwrap_or_default()
                },
            )
        };
        let mut cw = detached(4);
        let err = release(&mut cw, 0).expect_err("nothing is claimed");
        assert!(
            err.contains("release_row(0)") && err.contains("none"),
            "{err}"
        );
        assert_eq!(cw.next_row(), Some(0));
        assert_eq!(cw.next_row(), Some(1));
        for row in [0, 2, usize::MAX] {
            let err = release(&mut cw, row).expect_err("not the last claimed row");
            assert!(err.contains("and that is 1"), "{row}: {err}");
        }
        assert_eq!(cw.row_count(), 2, "a refused release changes nothing");
        std::mem::forget(cw);
    }

    #[test]
    fn default_vector_size_constant_matches_duckdb_default() {
        // DuckDB's `DEFAULT_STANDARD_VECTOR_SIZE` in
        // `duckdb/common/vector_size.hpp`. This is a fallback only — `new()`
        // asks the engine.
        assert_eq!(super::DEFAULT_STANDARD_VECTOR_SIZE, 2048);
    }

    #[test]
    fn is_full_at_zero_capacity() {
        let cw = ChunkWriter {
            raw: std::ptr::null_mut(),
            row_count: 0,
            capacity: 0,
        };
        assert!(cw.is_full());
        std::mem::forget(cw);
    }
}
