// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Bulk data appending (`DuckDB` 1.5.0+).
//!
//! [`Appender`] is an RAII wrapper around `DuckDB`'s appender — the fastest way
//! to bulk-insert rows into an existing table. This wrapper pairs the core
//! appender lifecycle (create, append a data chunk, flush, close) with the
//! 1.5.0 additions: structured [`ErrorData`] reporting
//! ([`error_data`][Appender::error_data]), reverting buffered-but-unflushed rows
//! ([`clear`][Appender::clear]), and appending a column's `DEFAULT` value into a
//! chunk ([`append_default_to_chunk`][Appender::append_default_to_chunk]).
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::appender::Appender;
//! use quack_rs::data_chunk::DataChunk;
//! use libduckdb_sys::duckdb_connection;
//!
//! # unsafe fn demo(con: duckdb_connection, chunk: &DataChunk) -> Result<(), quack_rs::error_data::ErrorData> {
//! // SAFETY: `con` is a valid, open connection.
//! let appender = unsafe { Appender::new(con, None, c"my_table") }?;
//! appender.append_chunk(chunk)?;
//! appender.flush()?;
//! # Ok(())
//! # }
//! ```

use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_append_data_chunk, duckdb_append_default_to_chunk, duckdb_appender,
    duckdb_appender_clear, duckdb_appender_close, duckdb_appender_create,
    duckdb_appender_create_ext, duckdb_appender_destroy, duckdb_appender_error_data,
    duckdb_appender_flush, duckdb_connection, duckdb_state, DuckDBSuccess,
};

use crate::data_chunk::DataChunk;
use crate::error_data::ErrorData;

/// Converts an optional `&CStr` into a (possibly null) C string pointer.
#[inline]
fn opt_ptr(s: Option<&CStr>) -> *const std::os::raw::c_char {
    s.map_or(std::ptr::null(), CStr::as_ptr)
}

/// RAII wrapper for a `duckdb_appender`.
///
/// The appender is flushed and destroyed automatically on drop. To surface any
/// error from the final flush, call [`close`][Appender::close] explicitly before
/// dropping.
pub struct Appender {
    appender: duckdb_appender,
}

impl Appender {
    /// Creates an appender for `table` in the given `schema` (or the default
    /// schema when `schema` is `None`).
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the appender cannot be created
    /// (for example because the table does not exist).
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn new(
        con: duckdb_connection,
        schema: Option<&CStr>,
        table: &CStr,
    ) -> Result<Self, ErrorData> {
        let mut raw: duckdb_appender = std::ptr::null_mut();
        // SAFETY: con is valid per caller's contract; the string pointers are
        // valid for the call; raw is a valid out-pointer.
        let state =
            unsafe { duckdb_appender_create(con, opt_ptr(schema), table.as_ptr(), &raw mut raw) };
        let appender = Self { appender: raw };
        if state == DuckDBSuccess {
            Ok(appender)
        } else {
            Err(appender.error_data())
        }
    }

    /// Creates an appender for `table`, fully qualified by optional `catalog` and
    /// `schema`.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the appender cannot be created.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn with_catalog(
        con: duckdb_connection,
        catalog: Option<&CStr>,
        schema: Option<&CStr>,
        table: &CStr,
    ) -> Result<Self, ErrorData> {
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
        let appender = Self { appender: raw };
        if state == DuckDBSuccess {
            Ok(appender)
        } else {
            Err(appender.error_data())
        }
    }

    /// Appends an entire [`DataChunk`] to the table.
    ///
    /// The chunk's column types must match the table's.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the append fails.
    pub fn append_chunk(&self, chunk: &DataChunk) -> Result<(), ErrorData> {
        // SAFETY: self.appender and chunk.as_raw() are valid.
        let state = unsafe { duckdb_append_data_chunk(self.appender, chunk.as_raw()) };
        self.check(state)
    }

    /// Writes the table column `col`'s `DEFAULT` value into row `row` of `chunk`.
    ///
    /// This is useful when building a chunk to append: columns without an
    /// explicit value can be filled with their schema default.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the default cannot be written.
    pub fn append_default_to_chunk(
        &self,
        chunk: &DataChunk,
        col: u64,
        row: u64,
    ) -> Result<(), ErrorData> {
        // SAFETY: self.appender and chunk.as_raw() are valid.
        let state =
            unsafe { duckdb_append_default_to_chunk(self.appender, chunk.as_raw(), col, row) };
        self.check(state)
    }

    /// Flushes buffered rows to the table without closing the appender.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the flush fails (e.g. a constraint
    /// violation). On failure, buffered rows can be discarded with
    /// [`clear`][Appender::clear].
    pub fn flush(&self) -> Result<(), ErrorData> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_flush(self.appender) };
        self.check(state)
    }

    /// Flushes and closes the appender. No further rows may be appended.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the final flush fails.
    pub fn close(&self) -> Result<(), ErrorData> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_close(self.appender) };
        self.check(state)
    }

    /// Discards all buffered, unflushed rows.
    ///
    /// Useful for recovering after a [`flush`][Appender::flush] error without
    /// re-appending the rows that were already committed.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the appender state is invalid.
    pub fn clear(&self) -> Result<(), ErrorData> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_clear(self.appender) };
        self.check(state)
    }

    /// Returns the structured error from the most recent failed operation.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        // SAFETY: self.appender is valid (possibly representing a failed create);
        // the call returns an owned error data handle.
        let raw = unsafe { duckdb_appender_error_data(self.appender) };
        // SAFETY: raw is an owned duckdb_error_data (possibly null).
        unsafe { ErrorData::from_raw(raw) }
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_appender {
        self.appender
    }

    /// Converts a `duckdb_state` into a `Result`, reading the appender's error
    /// data on failure.
    fn check(&self, state: duckdb_state) -> Result<(), ErrorData> {
        if state == DuckDBSuccess {
            Ok(())
        } else {
            Err(self.error_data())
        }
    }
}

impl Drop for Appender {
    fn drop(&mut self) {
        if !self.appender.is_null() {
            // SAFETY: self.appender is a valid handle that we own. Destroy flushes
            // and frees it; we intentionally ignore the state here (use `close`
            // beforehand to observe a final flush error).
            unsafe { duckdb_appender_destroy(&raw mut self.appender) };
        }
    }
}

#[cfg(all(test, feature = "_duckdb-testing"))]
mod tests {
    use super::*;

    #[test]
    fn create_for_missing_table_reports_error() {
        // Ensure the dispatch table is populated.
        let _db = crate::testing::InMemoryDb::open().unwrap();

        let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
        let mut con: duckdb_connection = std::ptr::null_mut();
        // SAFETY: dispatch table is initialized; null path opens in-memory.
        unsafe {
            assert_eq!(
                libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
                DuckDBSuccess
            );
            assert_eq!(
                libduckdb_sys::duckdb_connect(db, &raw mut con),
                DuckDBSuccess
            );
        }

        // SAFETY: con is a valid open connection.
        let result = unsafe { Appender::new(con, None, c"does_not_exist") };
        assert!(result.is_err(), "expected create to fail for missing table");
        let err = result.err().unwrap();
        assert!(err.has_error());

        // SAFETY: valid handles.
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut con);
            libduckdb_sys::duckdb_close(&raw mut db);
        }
    }
}
