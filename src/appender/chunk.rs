// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Chunk-at-a-time appends: handing the [`Appender`] a whole [`DataChunk`].

use libduckdb_sys::duckdb_append_data_chunk;
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::duckdb_append_default_to_chunk;

use super::{append_error, AppendError, Appender};
use crate::data_chunk::DataChunk;

impl Appender {
    // ── Chunk appends ───────────────────────────────────────────────────

    /// Appends an entire [`DataChunk`].
    ///
    /// The chunk's column types must match the appender's active columns; see
    /// [`column_type`][Self::column_type] to discover them.
    ///
    /// **Order:** `DuckDB` keeps row-at-a-time rows in a separate buffer until
    /// 2,048 of them accumulate, and adds a chunk to the table-bound buffer
    /// directly, so a chunk lands *ahead of* rows appended before it that are
    /// still buffered. [`flush`][Self::flush] first if insertion order
    /// matters.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, if the appender is
    /// closed or poisoned, or — without calling `DuckDB` — in the middle of a
    /// row appended by hand: when the chunk tips `DuckDB`'s buffer over its
    /// automatic-flush threshold, that flush fails on the open row *after* the
    /// chunk has been buffered, so an error would be reported for rows that
    /// were in fact kept.
    pub fn append_chunk(&self, chunk: &DataChunk) -> Result<(), AppendError> {
        self.usable()?;
        if self.column.get() != 0 {
            return Err(append_error(&format!(
                "append_chunk: a row is in progress ({} value(s) without end_row); finish it \
                 first",
                self.column.get()
            )));
        }
        // SAFETY: self.handle and chunk.as_raw() are valid.
        let state = unsafe { duckdb_append_data_chunk(self.handle, chunk.as_raw()) };
        self.check(state)?;
        let rows = u64::try_from(chunk.size()).unwrap_or(u64::MAX);
        self.buffered.set(self.buffered.get().saturating_add(rows));
        Ok(())
    }

    /// Writes the table column `col`'s `DEFAULT` value into row `row` of
    /// `chunk`.
    ///
    /// Useful when building a chunk to append: columns without an explicit
    /// value can be filled with their schema default.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the default cannot be written.
    #[cfg(feature = "duckdb-1-5")]
    pub fn append_default_to_chunk(
        &self,
        chunk: &DataChunk,
        col: u64,
        row: u64,
    ) -> Result<(), AppendError> {
        // SAFETY: self.handle and chunk.as_raw() are valid.
        let state =
            unsafe { duckdb_append_default_to_chunk(self.handle, chunk.as_raw(), col, row) };
        self.check(state)
    }
}
