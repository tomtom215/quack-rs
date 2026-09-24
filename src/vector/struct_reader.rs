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
#[derive(Debug)]
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
    pub fn field_count(&self) -> usize {
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
        // SAFETY: `StructVector::get_child` needs a live STRUCT vector and a field
        // index below its field count. `new`'s contract makes `self.vector` a valid
        // STRUCT vector with `field_count` fields for this reader's lifetime, and
        // this function's `# Safety` clause requires `field_idx < field_count`.
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
        // SAFETY: `VectorReader::is_valid` needs `row` below the reader's row count.
        // `self.fields[field_idx]` (bounds-checked) was built by `new` over STRUCT
        // child `field_idx` with the parent's `row_count`, and this function's
        // `# Safety` clause requires `row` below that count.
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
        // SAFETY: `VectorReader::read_bool` needs `row` below its row count and a
        // BOOLEAN column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a BOOLEAN field.
        unsafe { self.fields[field_idx].read_bool(row) }
    }

    /// Reads a VARCHAR value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// - `row` must be less than the row count.
    /// - The field at `field_idx` must have `VARCHAR` type.
    /// - The field must not be NULL at `row` (check [`is_valid`][Self::is_valid]
    ///   first): a NULL row may hold a stale pointer; see
    ///   [`VectorReader::read_str`].
    ///
    /// # Panics
    ///
    /// Panics if `field_idx >= field_count`.
    #[inline]
    pub unsafe fn read_str(&self, row: usize, field_idx: usize) -> &str {
        // SAFETY: `VectorReader::read_str` needs `row` below its row count and a
        // VARCHAR column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a VARCHAR field.
        // The result borrows `&self`, and the string heap it may point into belongs
        // to the child vector, which `new`'s contract keeps valid for the reader's
        // lifetime.
        unsafe { self.fields[field_idx].read_str(row) }
    }

    /// Reads an `i8` (TINYINT) value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i8(&self, row: usize, field_idx: usize) -> i8 {
        // SAFETY: `VectorReader::read_i8` needs `row` below its row count and a
        // TINYINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a TINYINT field.
        unsafe { self.fields[field_idx].read_i8(row) }
    }

    /// Reads an `i16` (SMALLINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i16(&self, row: usize, field_idx: usize) -> i16 {
        // SAFETY: `VectorReader::read_i16` needs `row` below its row count and a
        // SMALLINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a SMALLINT field.
        unsafe { self.fields[field_idx].read_i16(row) }
    }

    /// Reads an `i32` (INTEGER) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i32(&self, row: usize, field_idx: usize) -> i32 {
        // SAFETY: `VectorReader::read_i32` needs `row` below its row count and a
        // INTEGER column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a INTEGER field.
        unsafe { self.fields[field_idx].read_i32(row) }
    }

    /// Reads an `i64` (BIGINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i64(&self, row: usize, field_idx: usize) -> i64 {
        // SAFETY: `VectorReader::read_i64` needs `row` below its row count and a
        // BIGINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a BIGINT field.
        unsafe { self.fields[field_idx].read_i64(row) }
    }

    /// Reads an `i128` (HUGEINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_i128(&self, row: usize, field_idx: usize) -> i128 {
        // SAFETY: `VectorReader::read_i128` needs `row` below its row count and a
        // HUGEINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a HUGEINT field.
        unsafe { self.fields[field_idx].read_i128(row) }
    }

    /// Reads a `u8` (UTINYINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u8(&self, row: usize, field_idx: usize) -> u8 {
        // SAFETY: `VectorReader::read_u8` needs `row` below its row count and a
        // UTINYINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a UTINYINT field.
        unsafe { self.fields[field_idx].read_u8(row) }
    }

    /// Reads a `u16` (USMALLINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u16(&self, row: usize, field_idx: usize) -> u16 {
        // SAFETY: `VectorReader::read_u16` needs `row` below its row count and a
        // USMALLINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a USMALLINT field.
        unsafe { self.fields[field_idx].read_u16(row) }
    }

    /// Reads a `u32` (UINTEGER) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u32(&self, row: usize, field_idx: usize) -> u32 {
        // SAFETY: `VectorReader::read_u32` needs `row` below its row count and a
        // UINTEGER column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a UINTEGER field.
        unsafe { self.fields[field_idx].read_u32(row) }
    }

    /// Reads a `u64` (UBIGINT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_u64(&self, row: usize, field_idx: usize) -> u64 {
        // SAFETY: `VectorReader::read_u64` needs `row` below its row count and a
        // UBIGINT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a UBIGINT field.
        unsafe { self.fields[field_idx].read_u64(row) }
    }

    /// Reads an `f32` (FLOAT) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_f32(&self, row: usize, field_idx: usize) -> f32 {
        // SAFETY: `VectorReader::read_f32` needs `row` below its row count and a
        // FLOAT column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a FLOAT field.
        unsafe { self.fields[field_idx].read_f32(row) }
    }

    /// Reads an `f64` (DOUBLE) value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_f64(&self, row: usize, field_idx: usize) -> f64 {
        // SAFETY: `VectorReader::read_f64` needs `row` below its row count and a
        // DOUBLE column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a DOUBLE field.
        unsafe { self.fields[field_idx].read_f64(row) }
    }

    /// Reads an INTERVAL value.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_interval(&self, row: usize, field_idx: usize) -> DuckInterval {
        // SAFETY: `VectorReader::read_interval` needs `row` below its row count and a
        // INTERVAL column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a INTERVAL field.
        unsafe { self.fields[field_idx].read_interval(row) }
    }

    /// Reads a DATE value (days since epoch).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_date(&self, row: usize, field_idx: usize) -> i32 {
        // SAFETY: `VectorReader::read_date` needs `row` below its row count and a
        // DATE column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a DATE field.
        unsafe { self.fields[field_idx].read_date(row) }
    }

    /// Reads a TIMESTAMP value (microseconds since epoch).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_timestamp(&self, row: usize, field_idx: usize) -> i64 {
        // SAFETY: `VectorReader::read_timestamp` needs `row` below its row count and a
        // TIMESTAMP column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a TIMESTAMP field.
        unsafe { self.fields[field_idx].read_timestamp(row) }
    }

    /// Reads a TIME value (microseconds since midnight).
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_time(&self, row: usize, field_idx: usize) -> i64 {
        // SAFETY: `VectorReader::read_time` needs `row` below its row count and a
        // TIME column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a TIME field.
        unsafe { self.fields[field_idx].read_time(row) }
    }

    /// Reads a `BLOB` (binary) value from field `field_idx` at row `row`.
    ///
    /// # Safety
    ///
    /// As for [`read_bool`][Self::read_bool], and the field must not be NULL at
    /// `row` (check [`is_valid`][Self::is_valid] first): a NULL row may hold a
    /// stale pointer; see [`VectorReader::read_blob`].
    #[inline]
    pub unsafe fn read_blob(&self, row: usize, field_idx: usize) -> &[u8] {
        // SAFETY: `VectorReader::read_blob` needs `row` below its row count and a
        // BLOB column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a BLOB field.
        // The result borrows `&self`, and the string heap it may point into belongs
        // to the child vector, which `new`'s contract keeps valid for the reader's
        // lifetime.
        unsafe { self.fields[field_idx].read_blob(row) }
    }

    /// Reads a `UUID`'s textual 128 bits from field `field_idx` at row `row`.
    ///
    /// See [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid]
    /// for why this is not the raw `HUGEINT` storage.
    ///
    /// # Safety
    ///
    /// See [`read_bool`][Self::read_bool].
    #[inline]
    pub unsafe fn read_uuid(&self, row: usize, field_idx: usize) -> u128 {
        // SAFETY: `VectorReader::read_uuid` needs `row` below its row count and a
        // UUID column. `self.fields[field_idx]` (bounds-checked) was built by `new`
        // over STRUCT child `field_idx` with the parent's `row_count`, and the
        // `# Safety` contract (`read_bool`'s, for the field's own type) gives
        // `row` below that count and a UUID field.
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
