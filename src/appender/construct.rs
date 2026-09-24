// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Creating an [`Appender`] and choosing the columns it appends to.

use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_appender, duckdb_appender_add_column, duckdb_appender_clear_columns,
    duckdb_appender_column_count, duckdb_appender_column_type, duckdb_appender_create,
    duckdb_appender_create_ext, duckdb_connection, idx_t, DuckDBSuccess,
};

use super::{AppendError, Appender};
use crate::types::LogicalType;

/// Converts an optional `&CStr` into a (possibly null) C string pointer.
#[inline]
fn opt_ptr(s: Option<&CStr>) -> *const std::os::raw::c_char {
    s.map_or(std::ptr::null(), CStr::as_ptr)
}

impl Appender {
    // ── Construction ────────────────────────────────────────────────────

    /// Creates an appender for `table` in the given `schema` (or the default
    /// schema when `schema` is `None`).
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the appender cannot be created — most
    /// often because the table does not exist.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`. It need not outlive
    /// the appender for memory safety (`DuckDB`'s appender holds only a weak
    /// reference to the client context and checks it before writing), but rows still buffered when it closes are lost:
    /// appends keep succeeding, and the next flush or `close` fails with
    /// "Attempting to flush data to a closed connection". Close the appender
    /// first.
    pub unsafe fn new(
        con: duckdb_connection,
        schema: Option<&CStr>,
        table: &CStr,
    ) -> Result<Self, AppendError> {
        let mut raw: duckdb_appender = std::ptr::null_mut();
        // SAFETY: con is valid per caller's contract; the string pointers are
        // valid for the call; raw is a valid out-pointer.
        let state =
            unsafe { duckdb_appender_create(con, opt_ptr(schema), table.as_ptr(), &raw mut raw) };
        // DuckDB allocates the wrapper and writes it to `raw` *before* it can
        // fail, precisely so the error is readable, so this must be constructed
        // either way — and it must be dropped on the error path, which is what
        // returning it inside `Err` via `last_error` arranges.
        let appender = Self::wrap(raw);
        if state == DuckDBSuccess {
            Ok(appender)
        } else {
            Err(appender.last_error())
        }
    }

    /// Creates an appender for `table`, fully qualified by optional `catalog`
    /// and `schema`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the appender cannot be created.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`. It need not outlive
    /// the appender for memory safety (`DuckDB`'s appender holds only a weak
    /// reference to the client context and checks it before writing), but rows still buffered when it closes are lost:
    /// appends keep succeeding, and the next flush or `close` fails with
    /// "Attempting to flush data to a closed connection". Close the appender
    /// first.
    pub unsafe fn with_catalog(
        con: duckdb_connection,
        catalog: Option<&CStr>,
        schema: Option<&CStr>,
        table: &CStr,
    ) -> Result<Self, AppendError> {
        let mut raw: duckdb_appender = std::ptr::null_mut();
        // SAFETY: con is valid per caller's contract; the string pointers are
        // valid for the call; raw is a valid out-pointer.
        let state = unsafe {
            duckdb_appender_create_ext(
                con,
                opt_ptr(catalog),
                opt_ptr(schema),
                table.as_ptr(),
                &raw mut raw,
            )
        };
        let appender = Self::wrap(raw);
        if state == DuckDBSuccess {
            Ok(appender)
        } else {
            Err(appender.last_error())
        }
    }

    // ── Schema ──────────────────────────────────────────────────────────

    /// Number of columns the appender currently expects per row.
    ///
    /// This is the *active* column list, so it reflects any
    /// [`add_column`][Self::add_column] calls rather than always matching the
    /// table's width.
    #[must_use]
    pub fn column_count(&self) -> u64 {
        // SAFETY: self.handle is valid; DuckDB returns 0 for a null or
        // uninitialised appender.
        unsafe { duckdb_appender_column_count(self.handle) }
    }

    /// Type of active column `index`, or `None` if the index is out of range.
    #[must_use]
    pub fn column_type(&self, index: u64) -> Option<LogicalType> {
        // SAFETY: self.handle is valid; DuckDB bounds-checks `index` and
        // returns null when it is out of range.
        let raw = unsafe { duckdb_appender_column_type(self.handle, index as idx_t) };
        if raw.is_null() {
            None
        } else {
            // SAFETY: raw is a freshly allocated logical type that we now own.
            Some(unsafe { LogicalType::from_raw(raw) })
        }
    }

    /// Restricts appends to a named subset of the table's columns.
    ///
    /// Columns left out are filled with their `DEFAULT` (or NULL). Calling this
    /// **flushes everything appended so far**.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the column does not exist, or if the
    /// implicit flush fails.
    pub fn add_column(&self, name: &CStr) -> Result<(), AppendError> {
        self.usable()?;
        // SAFETY: self.handle is valid and `name` is a NUL-terminated string
        // that outlives the call.
        let state = unsafe { duckdb_appender_add_column(self.handle, name.as_ptr()) };
        self.flushed(state)
    }

    /// Resets the active column list so every table column is expected again.
    ///
    /// Also flushes everything appended so far.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the implicit flush fails.
    pub fn clear_columns(&self) -> Result<(), AppendError> {
        self.usable()?;
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_clear_columns(self.handle) };
        self.flushed(state)
    }
}
