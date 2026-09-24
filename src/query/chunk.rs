// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`OwnedDataChunk`]: a `duckdb_data_chunk` destroyed on drop.

use libduckdb_sys::duckdb_destroy_data_chunk;

use super::OwnedDataChunk;
use crate::data_chunk::DataChunk;

impl OwnedDataChunk {
    /// Takes ownership of a raw `duckdb_data_chunk`.
    ///
    /// # Safety
    ///
    /// `chunk` must be a non-null chunk that the caller is responsible for
    /// destroying, and must not be destroyed by anyone else.
    #[must_use]
    pub const unsafe fn from_raw(chunk: libduckdb_sys::duckdb_data_chunk) -> Self {
        Self {
            chunk,
            // SAFETY: `chunk` is valid and outlives the view, which this struct owns.
            view: unsafe { DataChunk::from_raw(chunk) },
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
