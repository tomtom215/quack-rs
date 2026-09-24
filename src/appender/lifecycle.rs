// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Flushing, closing and (with `duckdb-1-5`) clearing an [`Appender`].

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::duckdb_appender_clear;
use libduckdb_sys::{duckdb_appender_close, duckdb_appender_flush};

use super::{AppendError, Appender, Lifecycle};

impl Appender {
    // ── Lifecycle ───────────────────────────────────────────────────────

    /// Flushes buffered rows to the table without closing the appender.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the flush fails — a constraint violation,
    /// typically. On failure every buffered row is invalidated; with
    /// `duckdb-1-5` they can be discarded with `clear`. Also an error when the
    /// appender is closed or poisoned, or a row is in progress.
    pub fn flush(&self) -> Result<(), AppendError> {
        self.usable()?;
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_flush(self.handle) };
        self.flushed(state)
    }

    /// Flushes and closes the appender. Every later append, `row`, `end_row`,
    /// `flush` and `append_chunk` returns an error; closing again is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the final flush fails (the appender then
    /// stays open, so the buffered rows can be discarded with `clear` under
    /// `duckdb-1-5`), and — instead of `DuckDB`'s silent success — when a row
    /// is unfinished or the appender is poisoned: `DuckDB`'s `Close` writes
    /// nothing in that state, so the error says how many buffered rows were
    /// not written. An unfinished row appended by hand can still be completed
    /// and the appender closed again.
    pub fn close(&self) -> Result<(), AppendError> {
        match self.lifecycle.get() {
            Lifecycle::Closed => return Ok(()),
            Lifecycle::Poisoned => return Err(self.lost_rows_error("close")),
            Lifecycle::Open if self.column.get() != 0 => {
                return Err(self.lost_rows_error("close"));
            }
            Lifecycle::Open => {}
        }
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_close(self.handle) };
        self.flushed(state)?;
        self.lifecycle.set(Lifecycle::Closed);
        Ok(())
    }

    /// Discards all buffered, unflushed rows, and any half-written row.
    ///
    /// Useful for recovering after a [`flush`][Self::flush] error without
    /// re-appending the rows that were already committed, and the only way
    /// to make a poisoned appender (see the [module docs][crate::appender])
    /// usable again. It does not reopen a closed one.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the appender state is invalid.
    #[cfg(feature = "duckdb-1-5")]
    pub fn clear(&self) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_clear(self.handle) };
        self.check(state)?;
        // `BaseAppender::Clear` resets the chunk, the buffered collection and
        // the column counter.
        self.column.set(0);
        self.buffered.set(0);
        if self.lifecycle.get() == Lifecycle::Poisoned {
            self.lifecycle.set(Lifecycle::Open);
        }
        Ok(())
    }
}
