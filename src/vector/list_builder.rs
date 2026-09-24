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
/// and the C API wrapper does not catch it — the exception unwinds into Rust,
/// which aborts the process ("Rust cannot catch foreign exceptions").
/// [`ListBuilder`] refuses to make the call instead.
///
/// Staying below it is necessary, not sufficient: a reservation under the
/// ceiling that the allocator cannot satisfy throws through the same uncaught
/// path. Use [`ListBuilder::with_element_limit`] to bound lengths that come
/// from untrusted input.
///
/// Typed `u64` to match `duckdb::DConstants::MAX_VECTOR_SIZE`, which is
/// `1ULL << 37ULL` — an `idx_t`, not a pointer-sized value. It does not fit in a
/// 32-bit `usize`, so typing it as one made the crate fail to compile for
/// `wasm32`, which is a target `DuckDB`'s own extension CI builds.
pub const MAX_LIST_CHILD_CAPACITY: u64 = 1 << 37;

/// [`MAX_LIST_CHILD_CAPACITY`] clamped to this target's `usize`, for the
/// capacity arithmetic.
///
/// On a 64-bit target this is the ceiling itself. On a 32-bit target the
/// ceiling is larger than any allocation `usize` can describe, so `usize::MAX`
/// is the real limit and `DuckDB`'s is unreachable.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the branch above proves the value fits"
)]
pub(crate) const MAX_CHILD_CAPACITY_USIZE: usize = if MAX_LIST_CHILD_CAPACITY > usize::MAX as u64 {
    usize::MAX
} else {
    MAX_LIST_CHILD_CAPACITY as usize
};

/// Incremental builder for a `LIST` (or `MAP`) output vector.
///
/// See the [module docs][crate::vector::list_builder] for why the manual
/// sequence is easy to get wrong.
#[derive(Debug)]
pub struct ListBuilder {
    vector: duckdb_vector,
    /// Whether `written` has been initialised from the child's current size.
    /// Deferred to the first write so that [`new`][Self::new] stays `const`
    /// and makes no `DuckDB` call.
    started: bool,
    /// Elements in the child so far, including any already there when the
    /// builder started — the offset of the next row.
    written: usize,
    /// Capacity most recently requested, so `push_row` only reserves when it
    /// must (each reserve that grows is a reallocation plus a copy).
    reserved: usize,
    /// Most child elements this builder will reserve: [`MAX_LIST_CHILD_CAPACITY`]
    /// unless lowered with [`with_element_limit`][Self::with_element_limit].
    limit: usize,
    /// Set when a requested capacity exceeded `limit`; the builder then writes
    /// every remaining row as NULL rather than letting `DuckDB` throw.
    overflowed: bool,
}

impl ListBuilder {
    /// Starts building into `vector`.
    ///
    /// Rows are **appended** after any elements the vector's child already
    /// holds, so several builders — or several calls of a callback — can fill
    /// one vector in turn. `DuckDB` relies on this: it can call an
    /// aggregate's `finalize` many times on the same result vector with an
    /// increasing `offset`, one row per call (`agg(x ORDER BY y)` finalizes
    /// through `SortedAggregateFunction` that way), and a builder that started
    /// at child offset 0 each time would overwrite the earlier rows' elements.
    /// A fresh output vector has an empty child, so for a scalar function this
    /// makes no difference. The element limit counts the elements already
    /// there.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid, writable `LIST` or `MAP` output vector.
    #[must_use]
    pub const unsafe fn new(vector: duckdb_vector) -> Self {
        Self {
            vector,
            started: false,
            written: 0,
            reserved: 0,
            limit: MAX_CHILD_CAPACITY_USIZE,
            overflowed: false,
        }
    }

    /// Lowers the number of child elements this builder will reserve, in total
    /// across all rows. Values above [`MAX_LIST_CHILD_CAPACITY`] are clamped to
    /// it.
    ///
    /// # Why an extension needs this
    ///
    /// [`MAX_LIST_CHILD_CAPACITY`] is `DuckDB`'s own ceiling, 2^37 elements —
    /// far more memory than most machines have. A request below that ceiling
    /// that the allocator cannot satisfy makes `DuckDB` throw from
    /// `duckdb_list_vector_reserve`, which has no `try`/`catch` (`DuckDB`
    /// 1.5.5, `src/main/capi/data_chunk-c.cpp`), so the exception unwinds into
    /// Rust and the process aborts. Nothing in this crate can catch it. When
    /// row lengths come from untrusted input, set a limit that fits in memory;
    /// a row that would exceed it is written as NULL and
    /// [`overflowed`][Self::overflowed] reports it.
    #[must_use]
    pub const fn with_element_limit(mut self, limit: usize) -> Self {
        self.limit = if limit < MAX_CHILD_CAPACITY_USIZE {
            limit
        } else {
            MAX_CHILD_CAPACITY_USIZE
        };
        self
    }

    /// Elements in the child vector so far: those this builder wrote, plus any
    /// the vector already held when the builder started (see
    /// [`new`][Self::new]).
    #[must_use]
    #[inline]
    pub const fn element_count(&self) -> usize {
        self.written
    }

    /// Returns `true` if a row would have taken the child vector past the
    /// element limit ([`MAX_LIST_CHILD_CAPACITY`], or the one set with
    /// [`with_element_limit`][Self::with_element_limit]).
    ///
    /// From that row on, the builder writes nothing into the child: that row
    /// and every later [`push_row`][Self::push_row] /
    /// [`push_map_row`][Self::push_map_row] row is set to NULL, with an empty
    /// list entry. Rows written before it stay valid. Check this before
    /// [`finish`][Self::finish] if the row lengths come from untrusted input,
    /// and report an error if a NULL is not an acceptable answer.
    #[must_use]
    #[inline]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Grows the child vector to hold at least `capacity` elements in total.
    ///
    /// Returns `false` — writing nothing — if `capacity` exceeds the element
    /// limit, or an earlier request already did.
    ///
    /// # Safety
    ///
    /// `self.vector` must still be a valid LIST/MAP vector.
    unsafe fn ensure_capacity(&mut self, capacity: usize) -> bool {
        if self.overflowed {
            return false;
        }
        // `self.limit <= MAX_CHILD_CAPACITY_USIZE`, which is DuckDB's ceiling
        // clamped to `usize` (on a 32-bit target no `usize` can reach it).
        if capacity > self.limit {
            self.overflowed = true;
            return false;
        }
        if capacity > self.reserved {
            // Grow geometrically: each reserve that grows reallocates and copies
            // the whole child, so doing it once per row is quadratic.
            let target = capacity.next_power_of_two().min(self.limit);
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
    /// If the row would exceed the element limit — or an earlier row already
    /// did — the closure is not called and the row is written as NULL; see
    /// [`overflowed`][Self::overflowed].
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
        // SAFETY: `self.vector` is valid per the constructor's contract.
        unsafe { self.start() };
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            // SAFETY: `row_idx` is in bounds per the caller's contract.
            unsafe { self.refuse_row(row_idx) };
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
    /// Past the element limit the row is written as NULL, exactly as for
    /// [`push_row`][Self::push_row].
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
        // SAFETY: `self.vector` is valid per the constructor's contract.
        unsafe { self.start() };
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            // SAFETY: `row_idx` is in bounds per the caller's contract.
            unsafe { self.refuse_row(row_idx) };
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

    /// Picks up the child's current size on first use, so that rows are
    /// appended after elements an earlier builder or callback wrote.
    ///
    /// # Safety
    ///
    /// `self.vector` must be a valid `LIST` or `MAP` vector.
    unsafe fn start(&mut self) {
        if !self.started {
            // SAFETY: forwarded from this function's own contract.
            let existing = unsafe { ListVector::get_size(self.vector) };
            self.written = existing;
            self.reserved = existing;
            self.started = true;
        }
    }

    /// Writes `row_idx` as a NULL with an empty entry.
    ///
    /// The entry matters as much as the NULL: `DuckDB` reuses output vectors
    /// across chunks, so a row whose entry is never written keeps the
    /// `{offset, length}` of an earlier chunk, pointing into child data this
    /// chunk never wrote.
    ///
    /// # Safety
    ///
    /// `row_idx` must be within the parent vector's capacity.
    unsafe fn refuse_row(&mut self, row_idx: usize) {
        // SAFETY: `self.vector` is valid per the constructor's contract and
        // `row_idx` is in bounds per this function's.
        unsafe {
            ListVector::set_entry(self.vector, row_idx, self.written as u64, 0);
            VectorWriter::from_vector(self.vector).set_null(row_idx);
        }
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
    pub unsafe fn finish(mut self) {
        // SAFETY: `self.vector` is valid per the constructor's contract. A
        // builder that wrote nothing keeps the child's existing size.
        unsafe { self.start() };
        // SAFETY: `self.vector` is valid per the constructor's contract, and
        // `self.written` counts exactly the elements the caller wrote.
        unsafe { ListVector::set_size(self.vector, self.written) };
    }
}

#[cfg(test)]
mod tests {
    use super::{ListBuilder, MAX_CHILD_CAPACITY_USIZE, MAX_LIST_CHILD_CAPACITY};

    #[test]
    fn the_element_limit_defaults_to_and_is_clamped_by_duckdbs_ceiling() {
        // SAFETY: `new` and `with_element_limit` only store the pointer; no
        // DuckDB call is made, so a null vector is never dereferenced.
        let fresh = unsafe { ListBuilder::new(std::ptr::null_mut()) };
        assert_eq!(fresh.limit, MAX_CHILD_CAPACITY_USIZE);
        assert!(!fresh.overflowed());
        let lowered = fresh.with_element_limit(100);
        assert_eq!(lowered.limit, 100);
        // SAFETY: as above.
        let raised =
            unsafe { ListBuilder::new(std::ptr::null_mut()) }.with_element_limit(usize::MAX);
        assert_eq!(raised.limit, MAX_CHILD_CAPACITY_USIZE);
    }

    /// A request past the element limit is refused before `DuckDB` is asked
    /// for anything, and the refusal sticks: `overflowed` reports it and every
    /// later request — however small — is refused too, so no row after the
    /// first overflowing one is written.
    #[test]
    fn a_request_past_the_element_limit_is_refused_and_reported() {
        // SAFETY: `ensure_capacity` makes no DuckDB call for a request of 0
        // elements or one past the limit, so the null vector is never used.
        let mut builder = unsafe { ListBuilder::new(std::ptr::null_mut()) }.with_element_limit(10);
        // SAFETY: as above.
        assert!(unsafe { builder.ensure_capacity(0) });
        assert!(!builder.overflowed());
        // SAFETY: as above.
        assert!(!unsafe { builder.ensure_capacity(11) });
        assert!(builder.overflowed());
        // SAFETY: as above; the builder has overflowed, so nothing is reserved.
        assert!(!unsafe { builder.ensure_capacity(0) });
        assert!(builder.overflowed());
        assert_eq!(builder.element_count(), 0);
    }

    #[test]
    fn max_capacity_matches_duckdbs_constant() {
        // duckdb::DConstants::MAX_VECTOR_SIZE = 1ULL << 37ULL. Above this,
        // ListVector::Reserve throws a C++ exception that the C API does not
        // catch, so it must never be reached from Rust.
        assert_eq!(MAX_LIST_CHILD_CAPACITY, 137_438_953_472_u64);
        // The ceiling is an `idx_t`, so it must not be pointer-sized: typing it
        // `usize` made `1 << 37` a const-eval overflow on 32-bit targets and
        // broke the wasm32 build outright.
        assert_eq!(
            MAX_CHILD_CAPACITY_USIZE,
            usize::try_from(MAX_LIST_CHILD_CAPACITY).unwrap_or(usize::MAX)
        );
    }
}
