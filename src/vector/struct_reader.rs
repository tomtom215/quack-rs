// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Batched, typed reader for STRUCT input vectors.
//!
//! [`StructReader`] pre-creates [`VectorReader`]s for every field at construction,
//! then exposes typed `read_*` methods that take `(row, field_idx)`.
//! This is the read-side counterpart to [`StructWriter`][super::StructWriter].
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::vector::StructReader;
//! use libduckdb_sys::duckdb_vector;
//!
//! // Inside a scan callback, given a STRUCT input vector with 3 fields:
//! // let sr = unsafe { StructReader::new(struct_vec, 3, row_count) };
//! // for row in 0..row_count {
//! //     let name = unsafe { sr.read_str(row, 0) };
//! //     let age = unsafe { sr.read_i32(row, 1) };
//! //     let active = unsafe { sr.read_bool(row, 2) };
//! // }
//! ```

use libduckdb_sys::duckdb_vector;

use crate::interval::DuckInterval;
use crate::vector::complex::StructVector;
use crate::vector::VectorReader;

/// A batched reader for STRUCT input vectors.
///
/// Pre-creates a [`VectorReader`] for every field at construction, allowing
/// direct typed reads without repeated `duckdb_struct_vector_get_child` calls.
pub struct StructReader {
    vector: duckdb_vector,
    fields: Vec<VectorReader>,
}

impl StructReader {
    /// Creates a new `StructReader` for a STRUCT vector with `field_count` fields.
    ///
    /// # Safety
    ///
    /// - `vector` must be a valid `DuckDB` STRUCT vector.
    /// - `field_count` must match the number of fields in the STRUCT type.
    /// - `row_count` must match the number of rows in the parent chunk.
    /// - The vector must remain valid for the lifetime of this reader.
    pub unsafe fn new(vector: duckdb_vector, field_count: usize, row_count: usize) -> Self {
        let mut fields = Vec::with_capacity(field_count);
        for idx in 0..field_count {
            // SAFETY: caller guarantees vector is valid STRUCT with field_count fields.
            fields.push(unsafe { StructVector::field_reader(vector, idx, row_count) });
        }
        Self { vector, fields }
    }

    /// Returns the number of fields in this struct reader.
    #[mutants::skip]
    #[must_use]
    #[inline]
    pub const fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Returns a reference to the [`VectorReader`] for the given field.
    ///
    /// # Panics
    ///
    /// Panics if `field_idx >= field_count`.
    #[must_use]
    #[inline]
    pub fn field(&self, field_idx: usize) -> &VectorReader {
        &self.fields[field_idx]
    }

    /// Returns the raw `duckdb_vector` handle for the given field.
    ///
    /// Use this when a struct field has a complex type (LIST, MAP, ARRAY) that
    /// requires operations beyond simple scalar reads — for example, calling
    /// [`ListVector::get_entry`][crate::vector::complex::ListVector::get_entry] or
    /// [`ListVector::child_reader`][crate::vector::complex::ListVector::child_reader].
    ///
    /// # Safety
    ///
    /// - `field_idx` must be a valid field index (0 ≤ `field_idx` < `field_count`).
    /// - The returned vector is borrowed from the parent STRUCT vector and must
    ///   not outlive it.
    #[must_use]
    #[inline]
    pub unsafe fn child_vector(&self, field_idx: usize) -> duckdb_vector {
        unsafe { StructVector::get_child(self.vector, field_idx) }
    }

    /// Returns `true` if the value at `row` in field `field_idx` is not NULL.
    ///
    /// # Safety
    ///
    /// `row` must be less than the row count.
    ///
    /// # Panics
    ///
    /// Panics if `field_idx >= field_count`.
    #[inline]
    pub unsafe fn is_valid(&self, row: usize, field_idx: usize) -> bool {
        unsafe { self.fields[field_idx].is_valid(row) }
    }

    /// Reads a `bool` (BOOLEAN) value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// - `row` must be less than the row count.
    /// - The field at `field_idx` must have `BOOLEAN` type.
    ///
    /// # Panics
    ///
    /// Panics if `field_idx >= field_count`.
    #[inline]
    pub unsafe fn read_bool(&self, row: usize, field_idx: usize) -> bool {
        unsafe { self.fields[field_idx].read_bool(row) }
    }

    /// Reads a VARCHAR value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// - `row` must be less than the row count.
    /// - The field at `field_idx` must have `VARCHAR` type.
    ///
    /// # Panics
    ///
    /// Panics if `field_idx >= field_count`.
    #[inline]
    pub unsafe fn read_str(&self, row: usize, field_idx: usize) -> &str {
        unsafe { self.fields[field_idx].read_str(row) }
    }

    /// Reads an `i8` (TINYINT) value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i8(&self, row: usize, field_idx: usize) -> i8 {
        unsafe { self.fields[field_idx].read_i8(row) }
    }

    /// Reads an `i16` (SMALLINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i16(&self, row: usize, field_idx: usize) -> i16 {
        unsafe { self.fields[field_idx].read_i16(row) }
    }

    /// Reads an `i32` (INTEGER) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i32(&self, row: usize, field_idx: usize) -> i32 {
        unsafe { self.fields[field_idx].read_i32(row) }
    }

    /// Reads an `i64` (BIGINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i64(&self, row: usize, field_idx: usize) -> i64 {
        unsafe { self.fields[field_idx].read_i64(row) }
    }

    /// Reads an `i128` (HUGEINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i128(&self, row: usize, field_idx: usize) -> i128 {
        unsafe { self.fields[field_idx].read_i128(row) }
    }

    /// Reads a `u8` (UTINYINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u8(&self, row: usize, field_idx: usize) -> u8 {
        unsafe { self.fields[field_idx].read_u8(row) }
    }

    /// Reads a `u16` (USMALLINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u16(&self, row: usize, field_idx: usize) -> u16 {
        unsafe { self.fields[field_idx].read_u16(row) }
    }

    /// Reads a `u32` (UINTEGER) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u32(&self, row: usize, field_idx: usize) -> u32 {
        unsafe { self.fields[field_idx].read_u32(row) }
    }

    /// Reads a `u64` (UBIGINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u64(&self, row: usize, field_idx: usize) -> u64 {
        unsafe { self.fields[field_idx].read_u64(row) }
    }

    /// Reads an `f32` (FLOAT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_f32(&self, row: usize, field_idx: usize) -> f32 {
        unsafe { self.fields[field_idx].read_f32(row) }
    }

    /// Reads an `f64` (DOUBLE) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_f64(&self, row: usize, field_idx: usize) -> f64 {
        unsafe { self.fields[field_idx].read_f64(row) }
    }

    /// Reads an INTERVAL value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_interval(&self, row: usize, field_idx: usize) -> DuckInterval {
        unsafe { self.fields[field_idx].read_interval(row) }
    }

    /// Reads a DATE value (days since epoch).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_date(&self, row: usize, field_idx: usize) -> i32 {
        unsafe { self.fields[field_idx].read_date(row) }
    }

    /// Reads a TIMESTAMP value (microseconds since epoch).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_timestamp(&self, row: usize, field_idx: usize) -> i64 {
        unsafe { self.fields[field_idx].read_timestamp(row) }
    }

    /// Reads a TIME value (microseconds since midnight).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_time(&self, row: usize, field_idx: usize) -> i64 {
        unsafe { self.fields[field_idx].read_time(row) }
    }

    /// Reads a `BLOB` (binary) value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_blob(&self, row: usize, field_idx: usize) -> &[u8] {
        unsafe { self.fields[field_idx].read_blob(row) }
    }

    /// Reads a `UUID` value (as i128) from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_uuid(&self, row: usize, field_idx: usize) -> i128 {
        unsafe { self.fields[field_idx].read_uuid(row) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_reader_field_count() {
        let sr = StructReader {
            vector: std::ptr::null_mut(),
            fields: Vec::new(),
        };
        assert_eq!(sr.field_count(), 0);
    }

    #[test]
    fn size_of_struct_reader() {
        assert_eq!(
            std::mem::size_of::<StructReader>(),
            4 * std::mem::size_of::<usize>() // vector ptr + Vec (ptr + len + cap)
        );
    }
}
