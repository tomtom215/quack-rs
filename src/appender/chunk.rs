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
#[cfg(feature = "duckdb-1-5")]
use crate::query::OwnedDataChunk;

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
    /// `col` and `row` are checked against the chunk before `DuckDB` sees them.
    /// `DuckDB` 2.0 does not check the row itself for every column type: in a
    /// release build, a `VARCHAR` column's row is written without a bounds
    /// check, so row 2048 of a 2048-row chunk wrote past the vector and
    /// reported success, and row 2<sup>40</sup> crashed the process (QRS-008,
    /// `AUDIT.md` section 16). The chunk is an [`OwnedDataChunk`] because its
    /// capacity is known; a [`DataChunk`] borrowed
    /// from a callback does not report one.
    ///
    ///
    /// A chunk imported from Arrow is refused until it is
    /// [reset](OwnedDataChunk::reset): `DuckDB` imports its fixed-width
    /// columns without copying, so the default would be written into the Arrow
    /// producer's buffer. Measured on `DuckDB` 1.5.6 before the check existed:
    /// the producer's value was overwritten, and with the buffer in a read-only
    /// page the process died with `SIGSEGV` (`AUDIT.md` section 17, F-05).
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if `col` is not a column of `chunk`, if `row`
    /// is not below [`chunk.capacity()`](OwnedDataChunk::capacity), if the
    /// chunk is [externally backed](OwnedDataChunk::is_externally_backed), or
    /// if `DuckDB` cannot write the default. Nothing is written on error.
    #[cfg(feature = "duckdb-1-5")]
    pub fn append_default_to_chunk(
        &self,
        chunk: &OwnedDataChunk,
        col: u64,
        row: u64,
    ) -> Result<(), AppendError> {
        let columns = chunk.column_count();
        if usize::try_from(col).map_or(true, |c| c >= columns) {
            return Err(append_error(&format!(
                "append_default_to_chunk: column {col} is out of range for a chunk of \
                 {columns} column(s)"
            )));
        }
        if chunk.is_externally_backed() {
            return Err(append_error(
                "append_default_to_chunk: the chunk was imported from Arrow and its columns \
                 may be the producer's buffers, which DuckDB imports without copying; \
                 writing a default would write into memory the producer owns (and may share \
                 or have mapped read-only). Copy the rows into a chunk from \
                 `OwnedDataChunk::new`, or call `OwnedDataChunk::reset` first",
            ));
        }
        let capacity = chunk.capacity();
        if usize::try_from(row).map_or(true, |r| r >= capacity) {
            return Err(append_error(&format!(
                "append_default_to_chunk: row {row} is out of range for a chunk with room \
                 for {capacity} row(s)"
            )));
        }
        // SAFETY: self.handle and chunk.as_raw() are valid, and `col` and `row`
        // are inside the chunk: `col` below its column count, `row` below the
        // capacity `OwnedDataChunk` guarantees (its `from_raw` contract).
        let state =
            unsafe { duckdb_append_default_to_chunk(self.handle, chunk.as_raw(), col, row) };
        self.check(state)
    }
}
