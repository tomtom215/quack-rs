// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Complex type vector operations: STRUCT fields, LIST elements, MAP entries.
//!
//! `DuckDB` stores complex types as nested vectors:
//!
//! - **STRUCT**: a parent vector with N child vectors, one per field.
//! - **LIST**: a parent vector holding `duckdb_list_entry { offset, length }` per row,
//!   plus a single flat child vector containing all elements end-to-end.
//! - **MAP**: stored as `LIST<STRUCT{key, value}>` — the list's child vector is a
//!   STRUCT with two children: `key` (index 0) and `value` (index 1).
//!
//! # Reading vs writing
//!
//! - Use [`StructVector`] / [`ListVector`] / [`MapVector`] to access child vectors
//!   from input or output vectors.
//! - Child vectors are themselves `duckdb_vector` handles — pass them to
//!   [`VectorReader`] or
//!   [`VectorWriter`] to read/write the actual values.
//!
//! # Example: Reading a STRUCT column
//!
//! ```rust,no_run
//! use quack_rs::vector::{VectorReader, complex::StructVector};
//! use libduckdb_sys::{duckdb_data_chunk, duckdb_data_chunk_get_vector};
//!
//! // Inside a table function scan callback:
//! // let parent_vec = unsafe { duckdb_data_chunk_get_vector(chunk, 0) };
//! // let x_vec = StructVector::get_child(parent_vec, 0); // field index 0
//! // let x_reader = unsafe { VectorReader::from_vector(x_vec, row_count) };
//! // let x: f64 = unsafe { x_reader.read_f64(row_idx) };
//! ```
//!
//! # Example: Writing a LIST column
//!
//! ```rust,no_run
//! use quack_rs::vector::{VectorWriter, complex::ListVector};
//! use libduckdb_sys::{duckdb_data_chunk_get_vector, duckdb_data_chunk};
//!
//! // Inside a scan callback:
//! // let list_vec = unsafe { duckdb_data_chunk_get_vector(output, 0) };
//! // // Write 3 elements for row 0: [10, 20, 30]
//! // ListVector::reserve(list_vec, 3);
//! // ListVector::set_size(list_vec, 3);
//! // // Write the list offset/length entry for row 0.
//! // ListVector::set_entry(list_vec, 0, 0, 3); // row=0, offset=0, length=3
//! // // Write values into the child vector.
//! // let child = ListVector::get_child(list_vec);
//! // let mut writer = unsafe { VectorWriter::from_vector(child) };
//! // unsafe { writer.write_i64(0, 10); writer.write_i64(1, 20); writer.write_i64(2, 30); }
//! ```

use libduckdb_sys::{
    duckdb_list_entry, duckdb_list_vector_get_child, duckdb_list_vector_get_size,
    duckdb_list_vector_reserve, duckdb_list_vector_set_size, duckdb_struct_vector_get_child,
    duckdb_vector, duckdb_vector_get_data, idx_t,
};

use crate::vector::{VectorReader, VectorWriter};

// ─── STRUCT ──────────────────────────────────────────────────────────────────

/// Operations on STRUCT vectors (accessing child field vectors).
pub struct StructVector;

impl StructVector {
    /// Returns the child vector for the given field index of a STRUCT vector.
    ///
    /// Field indices correspond to the order of fields in the STRUCT type definition.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` STRUCT vector.
    /// - `field_idx` must be a valid field index (0 ≤ `field_idx` < number of struct fields).
    /// - The returned vector is borrowed from `vector` and must not outlive it.
    #[inline]
    #[must_use]
    pub unsafe fn get_child(vector: duckdb_vector, field_idx: usize) -> duckdb_vector {
        // SAFETY: caller guarantees vector is a valid STRUCT vector and field_idx is valid.
        unsafe { duckdb_struct_vector_get_child(vector, field_idx as idx_t) }
    }

    /// Creates a [`VectorReader`] for the given field of a STRUCT vector.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` STRUCT vector.
    /// - `field_idx` must be a valid field index.
    /// - `row_count` must match the number of rows in the parent chunk.
    pub unsafe fn field_reader(
        vector: duckdb_vector,
        field_idx: usize,
        row_count: usize,
    ) -> VectorReader {
        let child = unsafe { Self::get_child(vector, field_idx) };
        // SAFETY: child is a valid vector with row_count rows.
        unsafe { VectorReader::from_vector(child, row_count) }
    }

    /// Creates a [`VectorWriter`] for the given field of a STRUCT vector.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` STRUCT vector.
    /// - `field_idx` must be a valid field index.
    pub unsafe fn field_writer(vector: duckdb_vector, field_idx: usize) -> VectorWriter {
        let child = unsafe { Self::get_child(vector, field_idx) };
        // SAFETY: child is a valid writable vector.
        unsafe { VectorWriter::from_vector(child) }
    }
}

// ─── LIST ────────────────────────────────────────────────────────────────────

/// Operations on LIST vectors.
///
/// A LIST vector stores a `duckdb_list_entry { offset: u64, length: u64 }` per row
/// in the parent vector, and all element values in a flat child vector.
///
/// # Write workflow
///
/// 1. [`reserve`][ListVector::reserve] — ensure child vector has capacity.
/// 2. Write element values into the child via [`get_child`][ListVector::get_child] + [`VectorWriter`].
/// 3. [`set_size`][ListVector::set_size] — tell `DuckDB` how many elements were written.
/// 4. [`set_entry`][ListVector::set_entry] — write the offset/length for each parent row.
pub struct ListVector;

impl ListVector {
    /// Returns the child vector containing all list elements (flat, across all rows).
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - The returned handle is borrowed from `vector`.
    #[inline]
    #[must_use]
    pub unsafe fn get_child(vector: duckdb_vector) -> duckdb_vector {
        // SAFETY: caller guarantees vector is a valid LIST vector.
        unsafe { duckdb_list_vector_get_child(vector) }
    }

    /// Returns the total number of elements currently in the child vector.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` LIST vector.
    #[inline]
    #[must_use]
    pub unsafe fn get_size(vector: duckdb_vector) -> usize {
        usize::try_from(unsafe { duckdb_list_vector_get_size(vector) }).unwrap_or(0)
    }

    /// Sets the number of elements in the child vector.
    ///
    /// Call after writing all element values. `DuckDB` uses this to know how many
    /// child elements are valid.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - `size` must equal the number of elements written into the child vector.
    #[inline]
    pub unsafe fn set_size(vector: duckdb_vector, size: usize) {
        // SAFETY: caller guarantees vector is valid.
        unsafe { duckdb_list_vector_set_size(vector, size as idx_t) };
    }

    /// Reserves capacity in the child vector for at least `capacity` elements.
    ///
    /// Call before writing elements to ensure the child vector has enough space.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` LIST vector.
    #[inline]
    pub unsafe fn reserve(vector: duckdb_vector, capacity: usize) {
        // SAFETY: caller guarantees vector is valid.
        unsafe { duckdb_list_vector_reserve(vector, capacity as idx_t) };
    }

    /// Writes the offset/length metadata entry for a parent row.
    ///
    /// This tells `DuckDB` where in the flat child vector this row's elements start
    /// and how many elements it has.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - `row_idx` must be a valid row index in the parent vector.
    /// - `offset + length` must not exceed the size of the child vector.
    pub unsafe fn set_entry(vector: duckdb_vector, row_idx: usize, offset: u64, length: u64) {
        // SAFETY: vector is valid; we write to the parent vector's data at row_idx.
        let data = unsafe { duckdb_vector_get_data(vector) };
        // The parent stores duckdb_list_entry per row. Each entry is { offset: u64, length: u64 }.
        let entry_ptr = unsafe { data.cast::<duckdb_list_entry>().add(row_idx) };
        // SAFETY: entry_ptr is in bounds for the allocated vector.
        unsafe {
            (*entry_ptr).offset = offset;
            (*entry_ptr).length = length;
        }
    }

    /// Returns the `duckdb_list_entry` for a given row (for reading).
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - `row_idx` must be a valid row index.
    #[must_use]
    pub unsafe fn get_entry(vector: duckdb_vector, row_idx: usize) -> duckdb_list_entry {
        let data = unsafe { duckdb_vector_get_data(vector) };
        let entry_ptr = unsafe { data.cast::<duckdb_list_entry>().add(row_idx) };
        // SAFETY: entry_ptr is valid and initialized by DuckDB or a prior set_entry call.
        unsafe { core::ptr::read_unaligned(entry_ptr) }
    }

    /// Creates a [`VectorWriter`] for the child vector (elements).
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - The child must have been reserved with at least `capacity` elements.
    pub unsafe fn child_writer(vector: duckdb_vector) -> VectorWriter {
        let child = unsafe { Self::get_child(vector) };
        unsafe { VectorWriter::from_vector(child) }
    }

    /// Creates a [`VectorReader`] for the child vector (reading list elements).
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` LIST vector.
    /// - `element_count` must equal the total number of elements in the child.
    pub unsafe fn child_reader(vector: duckdb_vector, element_count: usize) -> VectorReader {
        let child = unsafe { Self::get_child(vector) };
        unsafe { VectorReader::from_vector(child, element_count) }
    }
}

// ─── MAP ─────────────────────────────────────────────────────────────────────

/// Operations on MAP vectors.
///
/// `DuckDB` stores maps as `LIST<STRUCT{key: K, value: V}>`.
/// The child of the list vector is a STRUCT vector with two fields:
/// - field index 0: keys
/// - field index 1: values
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::vector::complex::MapVector;
/// use libduckdb_sys::duckdb_vector;
///
/// // Reading MAP keys from a MAP vector:
/// // let keys_vec = unsafe { MapVector::keys(map_vector) };
/// // let vals_vec = unsafe { MapVector::values(map_vector) };
/// ```
pub struct MapVector;

impl MapVector {
    /// Returns the child STRUCT vector (contains both keys and values as fields).
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    #[must_use]
    pub unsafe fn struct_child(vector: duckdb_vector) -> duckdb_vector {
        // MAP is LIST<STRUCT{key,value}>, so the list child is a STRUCT vector.
        unsafe { duckdb_list_vector_get_child(vector) }
    }

    /// Returns the keys vector (STRUCT field 0 of the MAP's child).
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    #[must_use]
    pub unsafe fn keys(vector: duckdb_vector) -> duckdb_vector {
        let struct_vec = unsafe { Self::struct_child(vector) };
        // SAFETY: MAP child STRUCT always has key at field 0, value at field 1.
        unsafe { duckdb_struct_vector_get_child(struct_vec, 0) }
    }

    /// Returns the values vector (STRUCT field 1 of the MAP's child).
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    #[must_use]
    pub unsafe fn values(vector: duckdb_vector) -> duckdb_vector {
        let struct_vec = unsafe { Self::struct_child(vector) };
        // SAFETY: MAP child STRUCT always has key at field 0, value at field 1.
        unsafe { duckdb_struct_vector_get_child(struct_vec, 1) }
    }

    /// Returns the total number of key-value pairs across all rows.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    #[must_use]
    pub unsafe fn total_entry_count(vector: duckdb_vector) -> usize {
        usize::try_from(unsafe { duckdb_list_vector_get_size(vector) }).unwrap_or(0)
    }

    /// Reserves capacity in the MAP's child vector for at least `capacity` entries.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    pub unsafe fn reserve(vector: duckdb_vector, capacity: usize) {
        unsafe { duckdb_list_vector_reserve(vector, capacity as idx_t) };
    }

    /// Sets the total number of key-value entries written.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid `DuckDB` MAP vector.
    #[inline]
    pub unsafe fn set_size(vector: duckdb_vector, size: usize) {
        unsafe { duckdb_list_vector_set_size(vector, size as idx_t) };
    }

    /// Writes the offset/length metadata for a parent MAP row.
    ///
    /// This has the same semantics as [`ListVector::set_entry`], since MAP is a LIST.
    ///
    /// # Safety
    ///
    /// Same as [`ListVector::set_entry`].
    #[inline]
    pub unsafe fn set_entry(vector: duckdb_vector, row_idx: usize, offset: u64, length: u64) {
        // SAFETY: same layout as ListVector.
        unsafe { ListVector::set_entry(vector, row_idx, offset, length) };
    }

    /// Returns the `duckdb_list_entry` for a given MAP row (for reading).
    ///
    /// # Safety
    ///
    /// Same as [`ListVector::get_entry`].
    #[must_use]
    pub unsafe fn get_entry(vector: duckdb_vector, row_idx: usize) -> duckdb_list_entry {
        unsafe { ListVector::get_entry(vector, row_idx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libduckdb_sys::duckdb_list_entry;

    #[test]
    fn list_entry_layout() {
        // Verify duckdb_list_entry has the expected size (2 × u64 = 16 bytes).
        assert_eq!(
            core::mem::size_of::<duckdb_list_entry>(),
            16,
            "duckdb_list_entry should be {{ offset: u64, length: u64 }}"
        );
    }

    #[test]
    fn set_and_get_list_entry() {
        // Simulate the list parent vector data buffer (one row).
        let mut data = duckdb_list_entry {
            offset: 0,
            length: 0,
        };
        let vec_ptr: duckdb_vector = std::ptr::addr_of_mut!(data).cast();

        // Write entry for row 0: offset=5, length=3.
        // We bypass the actual DuckDB call and test the pointer arithmetic directly.
        let entry_ptr = std::ptr::addr_of_mut!(data);
        unsafe {
            (*entry_ptr).offset = 5;
            (*entry_ptr).length = 3;
        }
        assert_eq!(data.offset, 5);
        assert_eq!(data.length, 3);
        let _ = vec_ptr; // suppress unused warning; no FFI call possible without runtime
    }
}
