// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrapper around `DuckDB` data chunks.
//!
//! [`DataChunk`] provides safe, convenient access to the vectors and metadata
//! of a `duckdb_data_chunk`, eliminating the raw FFI calls that extension
//! authors currently need to write in every scan callback.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::data_chunk::DataChunk;
//! use quack_rs::vector::{VectorWriter, VectorReader};
//! use libduckdb_sys::{duckdb_function_info, duckdb_data_chunk};
//!
//! unsafe extern "C" fn my_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
//!     let chunk = unsafe { DataChunk::from_raw(output) };
//!     let mut writer = unsafe { chunk.writer(0) };
//!     unsafe { writer.write_i64(0, 42) };
//!     unsafe { chunk.set_size(1) };
//! }
//! ```

use libduckdb_sys::{
    duckdb_data_chunk, duckdb_data_chunk_get_column_count, duckdb_data_chunk_get_size,
    duckdb_data_chunk_get_vector, duckdb_data_chunk_set_size, duckdb_vector, idx_t,
};

use crate::chunk_writer::ChunkWriter;
use crate::vector::complex::StructVector;
use crate::vector::{StructReader, StructWriter, VectorReader, VectorWriter};

/// A non-owning wrapper around a `duckdb_data_chunk`.
///
/// This wrapper does **not** destroy the chunk on drop — `DuckDB` owns the
/// chunk and manages its lifetime. `DataChunk` simply provides ergonomic
/// methods for accessing vectors and metadata within callback functions.
pub struct DataChunk {
    raw: duckdb_data_chunk,
}

impl DataChunk {
    /// Wraps a raw `duckdb_data_chunk` handle.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid `duckdb_data_chunk` obtained from a `DuckDB`
    /// callback (e.g., a scan callback's `output` parameter or an aggregate
    /// `update` callback's `input` chunk). The chunk must remain valid for
    /// the lifetime of this wrapper.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_data_chunk) -> Self {
        Self { raw }
    }

    /// Returns the number of rows in this data chunk.
    #[inline]
    #[must_use]
    pub fn size(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { duckdb_data_chunk_get_size(self.raw) }).unwrap_or(0)
    }

    /// Sets the number of rows in this data chunk.
    ///
    /// Call this in scan callbacks after writing output rows. Set to `0` to
    /// signal end of stream.
    ///
    /// # Safety
    ///
    /// `size` must not exceed the chunk's capacity (typically 2048).
    #[inline]
    pub unsafe fn set_size(&self, size: usize) {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_data_chunk_set_size(self.raw, size as idx_t) };
    }

    /// Returns the number of columns in this data chunk.
    #[inline]
    #[must_use]
    pub fn column_count(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { duckdb_data_chunk_get_column_count(self.raw) }).unwrap_or(0)
    }

    /// Returns the raw `duckdb_vector` handle for the given column index.
    ///
    /// # Safety
    ///
    /// `col_idx` must be less than [`column_count`][DataChunk::column_count].
    #[inline]
    #[must_use]
    pub unsafe fn vector(&self, col_idx: usize) -> duckdb_vector {
        // SAFETY: self.raw is valid and col_idx is in bounds per caller's contract.
        unsafe { duckdb_data_chunk_get_vector(self.raw, col_idx as idx_t) }
    }

    /// Creates a [`VectorWriter`] for the given column index.
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than [`column_count`][DataChunk::column_count].
    /// - The chunk must be a writable output chunk (not a read-only input chunk).
    pub unsafe fn writer(&self, col_idx: usize) -> VectorWriter {
        let vec = unsafe { self.vector(col_idx) };
        // SAFETY: vec is a valid writable vector from the output chunk.
        unsafe { VectorWriter::from_vector(vec) }
    }

    /// Creates a [`VectorReader`] for the given column index.
    ///
    /// The reader's row count is set to this chunk's current [`size`][DataChunk::size].
    ///
    /// # Safety
    ///
    /// `col_idx` must be less than [`column_count`][DataChunk::column_count].
    pub unsafe fn reader(&self, col_idx: usize) -> VectorReader {
        // SAFETY: self.raw is valid; col_idx is in bounds per caller's contract.
        unsafe { VectorReader::new(self.raw, col_idx) }
    }

    /// Creates a [`StructReader`] for a STRUCT column at the given index.
    ///
    /// This is a convenience method that combines [`vector`][Self::vector] with
    /// [`StructReader::new`].
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than [`column_count`][Self::column_count].
    /// - The column at `col_idx` must have a STRUCT type with `field_count` fields.
    pub unsafe fn struct_reader(&self, col_idx: usize, field_count: usize) -> StructReader {
        let vec = unsafe { self.vector(col_idx) };
        // SAFETY: vec is a valid STRUCT vector per caller's contract.
        unsafe { StructReader::new(vec, field_count, self.size()) }
    }

    /// Creates a [`VectorReader`] for a field of a STRUCT column.
    ///
    /// Convenience for accessing a specific field in a STRUCT input column.
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than [`column_count`][Self::column_count].
    /// - The column at `col_idx` must have a STRUCT type.
    /// - `field_idx` must be a valid field index within the STRUCT.
    pub unsafe fn struct_field_reader(&self, col_idx: usize, field_idx: usize) -> VectorReader {
        let vec = unsafe { self.vector(col_idx) };
        // SAFETY: vec is a valid STRUCT vector per caller's contract.
        unsafe { StructVector::field_reader(vec, field_idx, self.size()) }
    }

    /// Creates a [`StructWriter`] for a STRUCT column at the given index.
    ///
    /// This is a convenience method that combines [`vector`][Self::vector] with
    /// [`StructWriter::new`].
    ///
    /// # Safety
    ///
    /// - `col_idx` must be less than [`column_count`][Self::column_count].
    /// - The column at `col_idx` must have a STRUCT type with `field_count` fields.
    /// - The chunk must be a writable output chunk.
    pub unsafe fn struct_writer(&self, col_idx: usize, field_count: usize) -> StructWriter {
        let vec = unsafe { self.vector(col_idx) };
        // SAFETY: vec is a valid STRUCT vector per caller's contract.
        unsafe { StructWriter::new(vec, field_count) }
    }

    /// Creates a [`ChunkWriter`] for this output data chunk.
    ///
    /// The [`ChunkWriter`] tracks rows via [`next_row()`][ChunkWriter::next_row]
    /// and automatically calls `set_size` on drop.
    ///
    /// # Safety
    ///
    /// This chunk must be a valid, writable output chunk from a table function
    /// scan callback.
    pub unsafe fn into_chunk_writer(self) -> ChunkWriter {
        // SAFETY: self.raw is valid per constructor's contract.
        unsafe { ChunkWriter::new(self.raw) }
    }

    /// Returns the raw `duckdb_data_chunk` handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_data_chunk {
        self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_of_data_chunk() {
        assert_eq!(
            std::mem::size_of::<DataChunk>(),
            std::mem::size_of::<usize>()
        );
    }
}
