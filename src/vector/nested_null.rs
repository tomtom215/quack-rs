// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The child validity masks a NULL in a nested vector must also clear.
//!
//! `DuckDB`'s own `FlatVector::SetNull` (`src/common/types/vector.cpp`) does
//! not stop at the vector it is given: for a vector whose physical type is
//! `STRUCT` (`STRUCT`, `UNION`, `VARIANT`) it recurses into every child entry
//! at the same row, and for an `ARRAY` of size `n` it recurses into child rows
//! `idx * n .. idx * n + n`. `LIST` and `MAP` children are left alone.
//! Operators such as `struct_extract` rely on this: they read the child
//! vector without looking at the parent's validity, so a NULL struct row whose
//! fields are still valid reads back its stale field values.
//!
//! The C API's `duckdb_validity_set_row_invalid` only touches the one mask it
//! is given, so [`VectorWriter`][super::VectorWriter] uses [`resolve`] to find
//! every descendant mask once and then clears the right rows in each.

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_array_vector_get_child, duckdb_destroy_logical_type,
    duckdb_get_type_id, duckdb_struct_type_child_count, duckdb_struct_vector_get_child,
    duckdb_vector, duckdb_vector_ensure_validity_writable, duckdb_vector_get_column_type,
    duckdb_vector_get_validity, idx_t, DUCKDB_TYPE_DUCKDB_TYPE_ARRAY,
};

/// A descendant validity mask that a NULL in the top-level vector also clears.
///
/// Row `r` of the top-level vector covers rows
/// `r * rows_per_row .. (r + 1) * rows_per_row` of this mask: `1` below
/// `STRUCT` edges, multiplied by the array size at each `ARRAY` edge.
#[derive(Debug, Clone, Copy)]
pub struct NullTarget {
    validity: *mut u64,
    rows_per_row: u64,
}

impl NullTarget {
    /// Clears the rows of this mask that `rows` of the top-level vector cover.
    ///
    /// # Safety
    ///
    /// `rows` must be within the top-level vector's capacity, so the covered
    /// rows are within this descendant's capacity.
    pub unsafe fn clear(&self, rows: core::ops::Range<usize>) {
        let (Some(start), Some(end)) = (
            (rows.start as u64).checked_mul(self.rows_per_row),
            (rows.end as u64).checked_mul(self.rows_per_row),
        ) else {
            return;
        };
        for row in start..end {
            // SAFETY: the mask covers the descendant's capacity, which holds
            // `rows_per_row` rows for every top-level row, and `rows` is in
            // bounds per the caller's contract.
            unsafe { libduckdb_sys::duckdb_validity_set_row_invalid(self.validity, row as idx_t) };
        }
    }

    /// Marks valid the rows of this mask that `rows` of the top-level vector
    /// cover: the inverse of [`clear`][Self::clear].
    ///
    /// # Safety
    ///
    /// As for [`clear`][Self::clear].
    pub unsafe fn restore(&self, rows: core::ops::Range<usize>) {
        let (Some(start), Some(end)) = (
            (rows.start as u64).checked_mul(self.rows_per_row),
            (rows.end as u64).checked_mul(self.rows_per_row),
        ) else {
            return;
        };
        for row in start..end {
            // SAFETY: as in `clear`.
            unsafe { libduckdb_sys::duckdb_validity_set_row_valid(self.validity, row as idx_t) };
        }
    }
}

/// Collects every descendant mask that `FlatVector::SetNull` would clear.
///
/// Each mask is made writable first, so the returned pointers are non-null
/// and stable until a `reserve` on a `LIST` or `MAP` whose child holds
/// `vector` (directly, or through STRUCT fields and ARRAY elements) moves
/// them; the owning writer's contract rules that out. A `LIST`/`MAP` child
/// below `vector` is never collected, so reserving one of those cannot move
/// any of them.
///
/// # Safety
///
/// `vector` must be a valid, flat, writable vector.
pub unsafe fn resolve(vector: duckdb_vector) -> Vec<NullTarget> {
    let mut targets = Vec::new();
    // SAFETY: forwarded from this function's contract.
    unsafe { collect(vector, 1, &mut targets) };
    targets
}

/// Appends the masks below `vector`, each of whose top-level rows covers
/// `rows_per_row` of `vector`'s rows.
///
/// # Safety
///
/// `vector` must be a valid, flat, writable vector.
unsafe fn collect(vector: duckdb_vector, rows_per_row: u64, out: &mut Vec<NullTarget>) {
    // SAFETY: `vector` is valid; the call returns an owned copy of its type
    // (or null), destroyed below.
    let mut ty = unsafe { duckdb_vector_get_column_type(vector) };
    if ty.is_null() {
        return;
    }
    // SAFETY: `ty` is a live logical type. Both calls return 0 for a type of
    // the wrong kind rather than throwing: `duckdb_struct_type_child_count`
    // checks for physical type STRUCT, which is exactly the condition
    // `FlatVector::SetNull` recurses on.
    let (struct_children, is_array, array_size) = unsafe {
        (
            duckdb_struct_type_child_count(ty),
            duckdb_get_type_id(ty) == DUCKDB_TYPE_DUCKDB_TYPE_ARRAY,
            duckdb_array_type_array_size(ty),
        )
    };
    // SAFETY: `ty` was created above and is destroyed exactly once.
    unsafe { duckdb_destroy_logical_type(&raw mut ty) };

    if struct_children > 0 {
        for idx in 0..struct_children {
            // SAFETY: `idx` is below the STRUCT's child count.
            let child = unsafe { duckdb_struct_vector_get_child(vector, idx) };
            // SAFETY: a STRUCT child of a flat, writable vector is one too.
            unsafe { push_and_descend(child, rows_per_row, out) };
        }
    } else if is_array {
        let Some(child_rows) = rows_per_row.checked_mul(array_size) else {
            return;
        };
        // SAFETY: `vector` is an ARRAY vector.
        let child = unsafe { duckdb_array_vector_get_child(vector) };
        // SAFETY: an ARRAY child of a flat, writable vector is one too.
        unsafe { push_and_descend(child, child_rows, out) };
    }
}

/// Records `child`'s mask and then everything below it.
///
/// # Safety
///
/// `child` must be null or a valid, flat, writable vector.
unsafe fn push_and_descend(child: duckdb_vector, rows_per_row: u64, out: &mut Vec<NullTarget>) {
    if child.is_null() {
        return;
    }
    // SAFETY: `child` is valid; this allocates its mask if it has none, after
    // which `get_validity` returns it.
    let validity = unsafe {
        duckdb_vector_ensure_validity_writable(child);
        duckdb_vector_get_validity(child)
    };
    if !validity.is_null() {
        out.push(NullTarget {
            validity,
            rows_per_row,
        });
    }
    // SAFETY: forwarded from this function's contract.
    unsafe { collect(child, rows_per_row, out) };
}
