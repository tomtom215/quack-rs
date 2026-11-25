// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`OwnedDataChunk`]: a `duckdb_data_chunk` destroyed on drop.

use libduckdb_sys::{
    duckdb_create_data_chunk, duckdb_data_chunk_reset, duckdb_destroy_data_chunk,
    duckdb_logical_type, idx_t,
};

use super::OwnedDataChunk;
use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::types::LogicalType;

impl OwnedDataChunk {
    /// Creates an empty chunk with one column per entry of `types`.
    ///
    /// The chunk holds zero rows and has room for [`capacity`][Self::capacity]
    /// of them. Fill its columns through [`writer`][DataChunk::writer] and
    /// friends, then [`set_size`][DataChunk::set_size] to the number of rows
    /// written — for example to build the chunk that
    /// [`Appender::append_chunk`][crate::appender::Appender::append_chunk]
    /// takes, without dropping to `duckdb_create_data_chunk`.
    ///
    /// `DuckDB` copies the types, so `types` may be dropped as soon as this
    /// returns.
    ///
    /// # Errors
    ///
    /// Returns an error, without calling `DuckDB`, if `types` is empty: a chunk
    /// with no columns trips a debug assertion in `DuckDB`'s
    /// `DataChunk::Initialize`. Returns an error if `DuckDB` refuses to create
    /// the chunk, which it does when a type is or contains `ANY` or `INVALID`,
    /// and when allocating the chunk throws.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::query::OwnedDataChunk;
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
    /// let chunk = OwnedDataChunk::new(&[LogicalType::new(TypeId::BigInt)])?;
    /// // SAFETY: column 0 is BIGINT and row 0 is within the chunk's capacity.
    /// unsafe {
    ///     chunk.writer(0).write_i64(0, 42);
    ///     chunk.set_size(1);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(types: &[LogicalType]) -> Result<Self, ExtensionError> {
        if types.is_empty() {
            return Err(ExtensionError::new(
                "OwnedDataChunk::new: a data chunk needs at least one column",
            ));
        }
        let mut raw: Vec<duckdb_logical_type> = types.iter().map(LogicalType::as_raw).collect();
        let count = idx_t::try_from(raw.len())
            .map_err(|_| ExtensionError::new("OwnedDataChunk::new: too many columns"))?;
        // SAFETY: `raw` holds `count` live logical types, borrowed from `types`
        // for the duration of the call. `DuckDB` copies each one into the new
        // chunk (`types.push_back(*logical_type)`) and keeps no pointer to the
        // array or to the handles.
        let chunk = unsafe { duckdb_create_data_chunk(raw.as_mut_ptr(), count) };
        if chunk.is_null() {
            return Err(ExtensionError::new(
                "OwnedDataChunk::new: DuckDB refused to create the chunk; a column type \
                 is or contains ANY or INVALID, or the allocation failed",
            ));
        }
        // SAFETY: `chunk` is non-null, freshly created, and owned by nobody else.
        Ok(unsafe { Self::from_raw(chunk) })
    }

    /// The number of rows each of the chunk's vectors has room for.
    ///
    /// For a chunk made by [`new`][Self::new], or handed out by
    /// [`QueryResult::next_chunk`][super::QueryResult::next_chunk], that is
    /// `duckdb_vector_size()`: 2048 unless `DuckDB` was built with a different
    /// `STANDARD_VECTOR_SIZE`. For a chunk from
    /// `arrow::data_chunk_from_arrow` (feature `duckdb-1-5-4`)
    /// it is the Arrow array's length, which `DuckDB` passes through as the
    /// chunk's capacity, and which may be smaller or larger than 2048.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Empties the chunk so it can be filled again: its size becomes zero and
    /// its columns are reset to fresh, writable vectors.
    ///
    /// Readers and writers taken from the chunk before the reset must not be
    /// used after it, since the vectors they point into may have been
    /// replaced.
    pub fn reset(&mut self) {
        // SAFETY: `self.chunk` is a live chunk owned by this value.
        unsafe { duckdb_data_chunk_reset(self.chunk) };
    }

    /// Takes ownership of a raw `duckdb_data_chunk`.
    ///
    /// # Safety
    ///
    /// - `chunk` must be a non-null chunk that the caller is responsible for
    ///   destroying, and must not be destroyed by anyone else.
    /// - Every vector of `chunk` must have room for at least
    ///   [`capacity`][Self::capacity] rows. A chunk from
    ///   `duckdb_create_data_chunk` or `duckdb_fetch_chunk` does; one from
    ///   `duckdb_data_chunk_from_arrow` has room for only the Arrow array's
    ///   length, and must not be passed here. Safe methods
    ///   such as `Appender::append_default_to_chunk` (feature `duckdb-1-5`)
    ///   check indices against it and rely on it.
    #[must_use]
    pub unsafe fn from_raw(chunk: libduckdb_sys::duckdb_data_chunk) -> Self {
        let capacity = usize::try_from(crate::vector::vector_size()).unwrap_or(0);
        // SAFETY: forwarded; this function's contract is the same, with
        // `capacity` the C API's vector size.
        unsafe { Self::from_raw_with_capacity(chunk, capacity) }
    }

    /// Takes ownership of a raw `duckdb_data_chunk` whose vectors have room
    /// for `capacity` rows.
    ///
    /// # Safety
    ///
    /// As [`from_raw`][Self::from_raw], with `capacity` in place of the C
    /// API's vector size: every vector of `chunk` must have room for at least
    /// `capacity` rows.
    #[must_use]
    pub(crate) const unsafe fn from_raw_with_capacity(
        chunk: libduckdb_sys::duckdb_data_chunk,
        capacity: usize,
    ) -> Self {
        Self {
            chunk,
            // SAFETY: `chunk` is valid and outlives the view, which this struct owns.
            view: unsafe { DataChunk::from_raw(chunk) },
            capacity,
        }
    }

    /// Relinquishes ownership, returning the raw handle.
    ///
    /// The caller becomes responsible for `duckdb_destroy_data_chunk`.
    #[must_use]
    pub const fn into_raw(self) -> libduckdb_sys::duckdb_data_chunk {
        let raw = self.chunk;
        std::mem::forget(self);
        raw
    }
}

impl std::fmt::Debug for OwnedDataChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `view` is a borrowing alias of `chunk`; printing it would just repeat
        // the same pointer.
        f.debug_struct("OwnedDataChunk")
            .field("chunk", &self.chunk)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for OwnedDataChunk {
    type Target = DataChunk;

    fn deref(&self) -> &DataChunk {
        &self.view
    }
}

impl Drop for OwnedDataChunk {
    fn drop(&mut self) {
        // SAFETY: `self.chunk` was owned by this value and is destroyed once.
        unsafe { duckdb_destroy_data_chunk(&raw mut self.chunk) };
    }
}
