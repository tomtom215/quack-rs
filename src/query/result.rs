// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Reading a [`QueryResult`]: its columns, its chunks and what kind of outcome it is.

use libduckdb_sys::{
    duckdb_column_count, duckdb_column_name, duckdb_column_type, duckdb_destroy_result,
    duckdb_fetch_chunk, duckdb_result, duckdb_result_error, duckdb_rows_changed, idx_t,
};

use super::cstr::c_str_to_owned;
use super::{FetchState, OwnedDataChunk, QueryResult, ResultKind};
use crate::error::ExtensionError;
use crate::types::TypeId;

impl QueryResult {
    /// Number of columns in the result.
    #[must_use]
    pub fn column_count(&self) -> usize {
        let mut result = self.result;
        // SAFETY: `result` is a valid materialised result; the C API takes a
        // mutable pointer but does not mutate observable state here.
        usize::try_from(unsafe { duckdb_column_count(&raw mut result) }).unwrap_or(0)
    }

    /// Name of column `index`, or `None` if the index is out of range or the
    /// name is not UTF-8.
    #[must_use]
    pub fn column_name(&self, index: usize) -> Option<String> {
        if index >= self.column_count() {
            return None;
        }
        let mut result = self.result;
        // SAFETY: `index` was bounds-checked against `column_count`.
        let ptr = unsafe { duckdb_column_name(&raw mut result, index as idx_t) };
        // SAFETY: DuckDB returns a NUL-terminated string owned by the result.
        unsafe { c_str_to_owned(ptr) }
    }

    /// [`TypeId`] of column `index`, or `None` if the index is out of range or
    /// the type is one this build does not recognise.
    #[must_use]
    pub fn column_type(&self, index: usize) -> Option<TypeId> {
        if index >= self.column_count() {
            return None;
        }
        let mut result = self.result;
        // SAFETY: `index` was bounds-checked against `column_count`.
        let raw = unsafe { duckdb_column_type(&raw mut result, index as idx_t) };
        TypeId::try_from_duckdb_type(raw)
    }

    /// Rows changed by an `INSERT` / `UPDATE` / `DELETE`. Zero for other
    /// statements.
    #[must_use]
    pub fn rows_changed(&self) -> u64 {
        let mut result = self.result;
        // SAFETY: `result` is a valid materialised result.
        unsafe { duckdb_rows_changed(&raw mut result) }
    }

    /// Fetches the next chunk of rows: `Ok(None)` once every row has been
    /// read, `Err` if the rows stopped early.
    ///
    /// Chunks hold at most `duckdb_vector_size()` rows; call this repeatedly:
    ///
    /// ```rust,no_run
    /// # fn demo(mut result: quack_rs::query::QueryResult) -> Result<(), quack_rs::error::ExtensionError> {
    /// while let Some(chunk) = result.next_chunk()? {
    ///     // ... read chunk.size() rows ...
    /// #   let _ = chunk;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// `duckdb_fetch_chunk` returns null both at the end of the rows and when
    /// fetching fails, recording the failure on the result. On a **streaming**
    /// result that happens when execution fails part-way (a runtime error in
    /// the query, an interrupt) or when another statement has run on the same
    /// connection, which invalidates the stream. This returns that error — with
    /// `DuckDB`'s message — instead of reporting a clean end after a partial
    /// result. Once it has returned `Ok(None)` or an error it keeps returning
    /// the same thing without calling `DuckDB` again.
    pub fn next_chunk(&mut self) -> Result<Option<OwnedDataChunk>, ExtensionError> {
        match &self.fetch {
            FetchState::Open => {}
            FetchState::Ended => return Ok(None),
            FetchState::Failed(message) => return Err(ExtensionError::new(message.clone())),
        }
        // SAFETY: `duckdb_fetch_chunk` takes the result by value (it reads the
        // internal pointer) and returns a chunk the caller owns, or null when
        // there are no more rows or fetching failed.
        let chunk = unsafe { duckdb_fetch_chunk(self.result) };
        if !chunk.is_null() {
            // SAFETY: `chunk` is non-null and owned by us from here on.
            return Ok(Some(unsafe { OwnedDataChunk::from_raw(chunk) }));
        }
        // `duckdb_fetch_chunk` catches a failed `Fetch` and records it with
        // `QueryResult::SetError`; a clean end records nothing.
        // SAFETY: `self.result` is a live result owned by this value.
        let error = unsafe { c_str_to_owned(duckdb_result_error(&raw mut self.result)) };
        self.fetch = error.map_or(FetchState::Ended, |message| {
            FetchState::Failed(format!(
                "the result ended early: fetching the next chunk failed: {message}"
            ))
        });
        self.next_chunk()
    }

    /// Returns the full [`LogicalType`][crate::types::LogicalType] of column
    /// `index`.
    ///
    /// [`column_type`][Self::column_type] collapses a column to its top-level
    /// [`TypeId`], which is all you need for a scalar but
    /// loses everything about a `STRUCT`'s fields, a `LIST`'s element type, a
    /// `DECIMAL`'s width and scale, or an `ENUM`'s dictionary. This keeps them.
    ///
    /// Returns `None` when `index` is out of range.
    #[must_use]
    pub fn column_logical_type(&self, index: usize) -> Option<crate::types::LogicalType> {
        if index >= self.column_count() {
            return None;
        }
        let mut result = self.result;
        // SAFETY: `result` is a valid materialised result and `index` is in range.
        let raw =
            unsafe { libduckdb_sys::duckdb_column_logical_type(&raw mut result, index as idx_t) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `duckdb_column_logical_type` returns a handle the caller owns
        // and must destroy; `LogicalType` does that on drop.
        Some(unsafe { crate::types::LogicalType::from_raw(raw) })
    }

    /// What kind of outcome the statement produced.
    #[must_use]
    pub fn result_kind(&self) -> ResultKind {
        // SAFETY: `duckdb_result_return_type` takes the result by value and
        // only reads it.
        let raw = unsafe { libduckdb_sys::duckdb_result_return_type(self.result) };
        match raw {
            libduckdb_sys::duckdb_result_type_DUCKDB_RESULT_TYPE_QUERY_RESULT => ResultKind::Rows,
            libduckdb_sys::duckdb_result_type_DUCKDB_RESULT_TYPE_CHANGED_ROWS => {
                ResultKind::ChangedRows
            }
            libduckdb_sys::duckdb_result_type_DUCKDB_RESULT_TYPE_NOTHING => ResultKind::Nothing,
            _ => ResultKind::Invalid,
        }
    }

    /// Whether this result is streaming rather than fully materialised.
    ///
    /// Only [`PreparedStatement::execute_streaming`][super::PreparedStatement::execute_streaming] can produce a streaming
    /// result, and even then `DuckDB` may decide to materialise — which is why
    /// this is a question rather than a guarantee.
    ///
    /// Requires `duckdb-1-5`: `duckdb_result_is_streaming` sits in the unstable
    /// region of the C API struct.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        // SAFETY: `duckdb_result_is_streaming` takes the result by value and
        // only reads it.
        unsafe { libduckdb_sys::duckdb_result_is_streaming(self.result) }
    }

    /// The Arrow production settings this result was created with
    /// (`duckdb_result_get_arrow_options`).
    ///
    /// Hand these to
    /// [`arrow::to_arrow_schema`][crate::arrow::to_arrow_schema] and
    /// [`arrow::data_chunk_to_arrow`][crate::arrow::data_chunk_to_arrow] when
    /// exporting this result's chunks: they carry the client properties captured
    /// when the query ran, which is what the chunks were built against.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] when `DuckDB` returns null, which it does
    /// for a result with no internal data.
    ///
    /// # Safety
    ///
    /// The connection that ran this query must stay open for all of `'conn`:
    /// the options point at its `ClientContext`, and a `QueryResult` does not
    /// borrow its connection. See
    /// [`ArrowOptions::from_result`][crate::arrow::ArrowOptions::from_result].
    /// [`ArrowOptions::from_connection`][crate::arrow::ArrowOptions::from_connection]
    /// is the safe alternative.
    #[cfg(feature = "duckdb-1-5-4")]
    pub unsafe fn arrow_options<'conn>(
        &self,
    ) -> Result<crate::arrow::ArrowOptions<'conn>, ExtensionError> {
        // SAFETY: forwarded verbatim from this function's contract.
        unsafe { crate::arrow::ArrowOptions::from_result(self) }
    }

    /// Returns the raw `duckdb_result`.
    ///
    /// Use this for C API calls this crate does not wrap. Do not destroy it —
    /// this value still owns it.
    #[must_use]
    pub const fn as_raw(&self) -> &duckdb_result {
        &self.result
    }
}

impl std::fmt::Debug for QueryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately does not call into DuckDB: `Debug` is often reached from
        // panic/assertion paths where an extra FFI call is unhelpful.
        f.debug_struct("QueryResult").finish_non_exhaustive()
    }
}

impl Drop for QueryResult {
    fn drop(&mut self) {
        // SAFETY: `self.result` was populated by DuckDB and is destroyed once.
        unsafe { duckdb_destroy_result(&raw mut self.result) };
    }
}
