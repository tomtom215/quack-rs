// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ArrowOptions` — the Arrow production settings of a connection or a result.

use std::marker::PhantomData;
use std::ptr;

use libduckdb_sys::{
    duckdb_arrow_options, duckdb_connection, duckdb_connection_get_arrow_options,
    duckdb_destroy_arrow_options, duckdb_result_get_arrow_options,
};

use super::ArrowOptions;
use crate::error::ExtensionError;
use crate::query::{OwnedConnection, QueryResult};

impl<'conn> ArrowOptions<'conn> {
    /// Reads the Arrow options of a connection, borrowing it for as long as
    /// the options live.
    ///
    /// # Errors
    ///
    /// `duckdb_connection_get_arrow_options` has no error channel: it writes
    /// null when the allocation throws. That is reported here as an
    /// [`ExtensionError`].
    #[mutants::skip] // FFI wrapper — needs a live DuckDB connection
    pub fn from_connection(connection: &'conn OwnedConnection) -> Result<Self, ExtensionError> {
        // SAFETY: `connection` is open, and the borrow keeps it open for all of
        // 'conn, which is as long as the returned options can be used.
        unsafe { Self::from_raw_connection(connection.as_raw()) }
    }

    /// Reads the Arrow options of a raw connection — for extension code that
    /// holds a `duckdb_connection` rather than an [`OwnedConnection`].
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] when the connection is null or `DuckDB`
    /// cannot allocate the options (it writes null in both cases).
    ///
    /// # Safety
    ///
    /// `connection` must be a live `duckdb_connection`, and it must not be
    /// disconnected for all of `'conn` (the caller picks `'conn`; choose one no
    /// longer than the connection stays open). See the type-level
    /// [lifetime](Self#lifetime) docs.
    #[mutants::skip] // FFI wrapper — needs a live DuckDB connection
    pub unsafe fn from_raw_connection(
        connection: duckdb_connection,
    ) -> Result<Self, ExtensionError> {
        let mut raw: duckdb_arrow_options = ptr::null_mut();
        // SAFETY: `connection` is live per this function's contract and `raw` is
        // a valid out-parameter.
        unsafe { duckdb_connection_get_arrow_options(connection, &raw mut raw) };
        if raw.is_null() {
            return Err(ExtensionError::new(
                "duckdb_connection_get_arrow_options returned null: the connection was null or \
                 DuckDB could not allocate its client properties",
            ));
        }
        Ok(Self {
            raw,
            _conn: PhantomData,
        })
    }

    /// Reads the Arrow options a result was produced with.
    ///
    /// Prefer this over [`from_connection`][Self::from_connection] when
    /// exporting that result's chunks: a result carries the client properties
    /// captured when it ran, which is what its chunks were built against.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] when `DuckDB` returns null, which it does
    /// for a result with no internal data.
    ///
    /// # Safety
    ///
    /// The connection that ran the query producing `result` must stay open for
    /// all of `'conn`. The captured client properties point at that
    /// connection's `ClientContext`, and a [`QueryResult`] can outlive its
    /// connection, so nothing checks this. See the type-level
    /// [lifetime](Self#lifetime) docs.
    #[mutants::skip] // FFI wrapper — needs a live DuckDB result
    pub unsafe fn from_result(result: &QueryResult) -> Result<Self, ExtensionError> {
        // `duckdb_result_get_arrow_options` takes a `duckdb_result *` but only
        // reads `internal_data`, so a copy of the POD struct is enough — the
        // same pattern every accessor in `crate::query` uses.
        let mut copy = *result.as_raw();
        // SAFETY: `copy` aliases a live result owned by `result`; DuckDB only
        // reads through the pointer.
        let raw = unsafe { duckdb_result_get_arrow_options(&raw mut copy) };
        if raw.is_null() {
            return Err(ExtensionError::new(
                "duckdb_result_get_arrow_options returned null: the result carries no data",
            ));
        }
        Ok(Self {
            raw,
            _conn: PhantomData,
        })
    }

    /// Takes ownership of a raw `duckdb_arrow_options`.
    ///
    /// # Safety
    ///
    /// `raw` must be a non-null handle the caller is responsible for
    /// destroying, and nobody else may destroy it. The connection it was read
    /// from must stay open for all of `'conn`.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_arrow_options) -> Self {
        Self {
            raw,
            _conn: PhantomData,
        }
    }

    /// The raw handle, still owned by this value.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_arrow_options {
        self.raw
    }

    /// Relinquishes ownership, returning the raw handle.
    ///
    /// The caller becomes responsible for `duckdb_destroy_arrow_options`, and
    /// for not using the handle after the connection closes.
    #[inline]
    #[must_use]
    pub const fn into_raw(self) -> duckdb_arrow_options {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }
}

impl Drop for ArrowOptions<'_> {
    #[mutants::skip] // frees a DuckDB handle; nothing observable without a runtime
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        // SAFETY: `self.raw` was owned by this value and is destroyed once;
        // DuckDB nulls it. Destruction does not touch the client context.
        unsafe { duckdb_destroy_arrow_options(&raw mut self.raw) };
    }
}

impl core::fmt::Debug for ArrowOptions<'_> {
    #[mutants::skip] // Debug rendering is not a behavioural contract
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArrowOptions")
            .field("raw", &self.raw)
            .finish()
    }
}
