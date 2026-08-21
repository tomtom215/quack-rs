// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Running SQL from inside an extension.
//!
//! Extensions routinely need to talk SQL to the database that is loading them:
//! checking whether a table exists before registering a replacement scan,
//! creating a helper view or macro, reading a setting, or looking up a secret
//! through `duckdb_secrets()`. The C API exposes all of that
//! (`duckdb_query`, `duckdb_prepare`, `duckdb_bind_*`, `duckdb_fetch_chunk`),
//! but every handle involved has a matching `destroy` that must run exactly once
//! — including on the error paths, which is where hand-written FFI usually leaks.
//!
//! This module wraps those handles in RAII types:
//!
//! | Type | Owns | Released by |
//! |------|------|-------------|
//! | [`QueryResult`] | `duckdb_result` | `duckdb_destroy_result` |
//! | [`OwnedDataChunk`] | `duckdb_data_chunk` | `duckdb_destroy_data_chunk` |
//! | [`PreparedStatement`] | `duckdb_prepared_statement` | `duckdb_destroy_prepare` |
//! | [`OwnedConnection`] | `duckdb_connection` | `duckdb_disconnect` |
//!
//! Everything here is in the **stable** prefix of the C extension API, so it
//! works on every `DuckDB` from v1.2.0 onwards and needs no feature flag. See
//! [`crate::abi`].
//!
//! # When you can run a query
//!
//! During extension load, inside your registration closure. The
//! `duckdb_connection` `DuckDB` hands you there is a real connection, and
//! [`Connection::query`][crate::connection::Connection::query] uses it directly.
//!
//! Inside a scalar/table/aggregate **callback** you do not have a connection —
//! the C API gives you a `duckdb_client_context`, and there is no
//! `duckdb_client_context_get_connection`. If a callback needs to run SQL, open
//! an [`OwnedConnection`] during registration and keep it: a connection created
//! from the load-time `duckdb_database` holds a `shared_ptr` to the database
//! instance, so it stays valid after loading finishes.
//!
//! Do not reuse the *borrowed* registration connection after your closure
//! returns — the entry point disconnects it.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::error::ExtensionError;
//! use quack_rs::query;
//!
//! # unsafe fn demo(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
//! // SAFETY: `con` is the connection DuckDB passed to the entry point.
//! let mut result = unsafe { query::query(con, "SELECT 42 AS answer") }?;
//! while let Some(chunk) = result.next_chunk() {
//!     let reader = unsafe { chunk.reader(0) };
//!     for row in 0..chunk.size() {
//!         assert_eq!(unsafe { reader.read_i32(row) }, 42);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod cstr;

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_bind_blob, duckdb_bind_boolean, duckdb_bind_double, duckdb_bind_int64, duckdb_bind_null,
    duckdb_bind_parameter_index, duckdb_bind_varchar_length, duckdb_clear_bindings,
    duckdb_column_count, duckdb_column_name, duckdb_column_type, duckdb_connect, duckdb_connection,
    duckdb_database, duckdb_destroy_data_chunk, duckdb_destroy_prepare, duckdb_destroy_result,
    duckdb_disconnect, duckdb_execute_prepared, duckdb_fetch_chunk, duckdb_nparams,
    duckdb_parameter_name, duckdb_prepare, duckdb_prepare_error, duckdb_prepared_statement,
    duckdb_query, duckdb_result, duckdb_result_error, duckdb_rows_changed, idx_t, DuckDBSuccess,
};

use self::cstr::{c_str_to_owned, to_c_sql};
use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::types::TypeId;

// ─── Data chunk ──────────────────────────────────────────────────────────────

/// A `duckdb_data_chunk` owned by this crate.
///
/// [`QueryResult::next_chunk`] hands out chunks that the caller owns and must
/// destroy. This type does that on drop and derefs to the borrowing
/// [`DataChunk`] wrapper, so all the usual readers apply.
pub struct OwnedDataChunk {
    chunk: libduckdb_sys::duckdb_data_chunk,
    view: DataChunk,
}

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

// ─── Query result ────────────────────────────────────────────────────────────

/// What kind of outcome a statement produced.
///
/// Mirrors `duckdb_result_type`. A `SELECT` yields [`QueryResult`][Self::Rows];
/// `INSERT` / `UPDATE` / `DELETE` yield [`ChangedRows`][Self::ChangedRows], for
/// which [`QueryResult::rows_changed`] is meaningful; `CREATE` / `SET` and
/// friends yield [`Nothing`][Self::Nothing].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResultKind {
    /// The statement produced rows.
    Rows,
    /// The statement changed rows; see [`QueryResult::rows_changed`].
    ChangedRows,
    /// The statement produced neither rows nor row counts.
    Nothing,
    /// `DuckDB` reported `DUCKDB_RESULT_TYPE_INVALID`, or a value this build
    /// does not know.
    Invalid,
}

/// A materialised `duckdb_result`, destroyed on drop.
///
/// Iterate the rows with [`next_chunk`][Self::next_chunk] until it returns
/// `None`.
pub struct QueryResult {
    result: duckdb_result,
}

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

    /// Fetches the next chunk of rows, or `None` once the result is exhausted.
    ///
    /// Chunks hold at most `duckdb_vector_size()` rows; call this repeatedly.
    #[must_use]
    pub fn next_chunk(&mut self) -> Option<OwnedDataChunk> {
        // SAFETY: `duckdb_fetch_chunk` takes the result by value (it reads the
        // internal pointer) and returns a chunk the caller owns, or null when
        // there are no more rows.
        let chunk = unsafe { duckdb_fetch_chunk(self.result) };
        if chunk.is_null() {
            return None;
        }
        // SAFETY: `chunk` is non-null and owned by us from here on.
        Some(unsafe { OwnedDataChunk::from_raw(chunk) })
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
    /// Only [`PreparedStatement::execute_streaming`] can produce a streaming
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
    #[cfg(feature = "duckdb-1-5-4")]
    pub fn arrow_options(&self) -> Result<crate::arrow::ArrowOptions, ExtensionError> {
        crate::arrow::ArrowOptions::from_result(self)
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

/// Runs `sql` on `con` and returns the materialised result.
///
/// # Errors
///
/// Returns [`ExtensionError`] carrying `DuckDB`'s own message if the statement
/// fails, or if `sql` contains an interior NUL byte.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
pub unsafe fn query(con: duckdb_connection, sql: &str) -> Result<QueryResult, ExtensionError> {
    let c_sql = to_c_sql(sql)?;
    let mut result: duckdb_result = unsafe { std::mem::zeroed() };
    // SAFETY: `con` is valid per the caller's contract; `c_sql` outlives the call.
    let state = unsafe { duckdb_query(con, c_sql.as_ptr(), &raw mut result) };
    if state == DuckDBSuccess {
        return Ok(QueryResult { result });
    }
    // SAFETY: even on failure DuckDB populated `result`, so the error message is
    // readable and the result must still be destroyed.
    let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
        .unwrap_or_else(|| String::from("query failed without an error message"));
    // SAFETY: `result` is destroyed exactly once, here, on the error path.
    unsafe { duckdb_destroy_result(&raw mut result) };
    Err(ExtensionError::new(message))
}

/// Runs `sql` on `con` for its side effects and returns the number of rows
/// changed.
///
/// # Errors
///
/// See [`query`].
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
pub unsafe fn execute(con: duckdb_connection, sql: &str) -> Result<u64, ExtensionError> {
    // SAFETY: forwarded from this function's own contract.
    let result = unsafe { query(con, sql) }?;
    Ok(result.rows_changed())
}

// ─── Prepared statements ─────────────────────────────────────────────────────

/// A `duckdb_prepared_statement`, destroyed on drop.
///
/// Prepared statements are how an extension runs SQL with values it did not
/// author. Interpolating a table name or a user-supplied string into SQL text is
/// an injection bug; binding it is not.
///
/// Parameters are 1-indexed, matching the C API.
pub struct PreparedStatement {
    statement: duckdb_prepared_statement,
}

impl PreparedStatement {
    /// Number of `?` / `$name` parameters in the statement.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        usize::try_from(unsafe { duckdb_nparams(self.statement) }).unwrap_or(0)
    }

    /// Name of the parameter at 1-based `index`, if it has one.
    #[must_use]
    pub fn parameter_name(&self, index: usize) -> Option<String> {
        if index == 0 || index > self.parameter_count() {
            return None;
        }
        // SAFETY: `index` was bounds-checked; DuckDB owns the returned string.
        let ptr = unsafe { duckdb_parameter_name(self.statement, index as idx_t) };
        // SAFETY: `ptr` is null or NUL-terminated.
        let name = unsafe { c_str_to_owned(ptr) };
        // DuckDB allocates the name; free it once copied.
        if !ptr.is_null() {
            // SAFETY: `ptr` came from DuckDB's allocator.
            unsafe { libduckdb_sys::duckdb_free(ptr.cast_mut().cast()) };
        }
        name
    }

    /// 1-based index of the named parameter, or `None` if there is no such
    /// parameter.
    #[must_use]
    pub fn parameter_index(&self, name: &str) -> Option<usize> {
        let c_name = CString::new(name).ok()?;
        let mut index: idx_t = 0;
        // SAFETY: `self.statement` is valid; `c_name` outlives the call.
        let state =
            unsafe { duckdb_bind_parameter_index(self.statement, &raw mut index, c_name.as_ptr()) };
        (state == DuckDBSuccess).then(|| usize::try_from(index).unwrap_or(0))
    }

    /// Binds a `BIGINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` rejects the binding (bad index or type).
    pub fn bind_i64(&self, index: usize, value: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_int64(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `DOUBLE` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_f64(&self, index: usize, value: f64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_double(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `BOOLEAN` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_bool(&self, index: usize, value: bool) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_boolean(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `VARCHAR` at 1-based `index`.
    ///
    /// The length is passed explicitly, so embedded NUL bytes are preserved and
    /// no `CString` conversion can fail.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_str(&self, index: usize, value: &str) -> Result<(), ExtensionError> {
        // SAFETY: `value` is valid for the duration of the call; the length is
        // passed explicitly so the pointer need not be NUL-terminated.
        let state = unsafe {
            duckdb_bind_varchar_length(
                self.statement,
                index as idx_t,
                value.as_ptr().cast::<std::os::raw::c_char>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            )
        };
        self.check(state, index)
    }

    /// Binds a `BLOB` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_blob(&self, index: usize, value: &[u8]) -> Result<(), ExtensionError> {
        // SAFETY: `value` is valid for the duration of the call.
        let state = unsafe {
            duckdb_bind_blob(
                self.statement,
                index as idx_t,
                value.as_ptr().cast::<std::os::raw::c_void>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            )
        };
        self.check(state, index)
    }

    /// Binds SQL `NULL` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_null(&self, index: usize) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_null(self.statement, index as idx_t) },
            index,
        )
    }

    /// Binds a `TINYINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i8(&self, index: usize, value: i8) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int8(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `SMALLINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i16(&self, index: usize, value: i16) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int16(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds an `INTEGER` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i32(&self, index: usize, value: i32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int32(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UTINYINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u8(&self, index: usize, value: u8) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint8(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `USMALLINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u16(&self, index: usize, value: u16) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint16(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UINTEGER` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u32(&self, index: usize, value: u32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint32(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UBIGINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u64(&self, index: usize, value: u64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint64(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `FLOAT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_f32(&self, index: usize, value: f32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_float(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `HUGEINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i128(&self, index: usize, value: i128) -> Result<(), ExtensionError> {
        let raw = crate::value::hugeint_from_i128(value);
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_hugeint(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `UHUGEINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u128(&self, index: usize, value: u128) -> Result<(), ExtensionError> {
        let raw = crate::value::uhugeint_from_u128(value);
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uhugeint(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `DECIMAL(width, scale)` at 1-based `index`.
    ///
    /// `unscaled` is the value multiplied by `10^scale`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_decimal(
        &self,
        index: usize,
        width: u8,
        scale: u8,
        unscaled: i128,
    ) -> Result<(), ExtensionError> {
        let raw = libduckdb_sys::duckdb_decimal {
            width,
            scale,
            value: crate::value::hugeint_from_i128(unscaled),
        };
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_decimal(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `DATE` at 1-based `index`, as days since 1970-01-01.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_date(&self, index: usize, days: i32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_date(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_date { days },
                )
            },
            index,
        )
    }

    /// Binds a `TIME` at 1-based `index`, as microseconds since midnight.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_time(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_time(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_time { micros },
                )
            },
            index,
        )
    }

    /// Binds a `TIMESTAMP` at 1-based `index`, as microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_timestamp(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_timestamp(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_timestamp { micros },
                )
            },
            index,
        )
    }

    /// Binds a `TIMESTAMP WITH TIME ZONE` at 1-based `index`, as microseconds
    /// since the epoch.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_timestamp_tz(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_timestamp_tz(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_timestamp { micros },
                )
            },
            index,
        )
    }

    /// Binds an `INTERVAL` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_interval(
        &self,
        index: usize,
        value: crate::interval::DuckInterval,
    ) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_interval(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_interval {
                        months: value.months,
                        days: value.days,
                        micros: value.micros,
                    },
                )
            },
            index,
        )
    }

    /// Binds an arbitrary [`Value`][crate::value::Value] at 1-based `index`.
    ///
    /// This is the escape hatch for everything the typed `bind_*` methods do
    /// not cover — `STRUCT`, `LIST`, `MAP`, `ARRAY`, `UNION`, `ENUM`, `UUID`,
    /// `BIT`, and any type a future `DuckDB` adds. `DuckDB` copies the value, so
    /// `value` may be dropped immediately afterwards.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(stmt: &quack_rs::query::PreparedStatement)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// let ty = LogicalType::list(TypeId::BigInt);
    /// let list = Value::list_value(&ty, &[Value::bigint(1), Value::bigint(2)])?;
    /// stmt.bind_value(1, &list)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn bind_value(
        &self,
        index: usize,
        value: &crate::value::Value,
    ) -> Result<(), ExtensionError> {
        if value.is_null() {
            return Err(ExtensionError::new(format!(
                "bind_value: the Value at parameter {index} has a null handle"
            )));
        }
        // SAFETY: `self.statement` is valid for this value's lifetime, and
        // `value` outlives the call (DuckDB copies it).
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_value(self.statement, index as idx_t, value.as_raw())
            },
            index,
        )
    }

    /// Executes the statement, asking `DuckDB` to stream the result instead of
    /// materialising it.
    ///
    /// A materialised result holds every row in memory before the first one is
    /// readable; a streaming result produces chunks on demand, which is what an
    /// extension scanning a large table wants.
    /// [`QueryResult::next_chunk`] drives both — `duckdb_fetch_chunk` is the
    /// documented way to read either — so the only difference at the call site
    /// is memory.
    ///
    /// `DuckDB` may still materialise (see
    /// [`QueryResult::is_streaming`]), and a streaming result must be consumed
    /// before another statement runs on the same connection.
    ///
    /// Requires `duckdb-1-5`: `duckdb_execute_prepared_streaming` sits in the
    /// unstable region of the C API struct.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] carrying `DuckDB`'s message if execution
    /// fails.
    #[cfg(feature = "duckdb-1-5")]
    pub fn execute_streaming(&self) -> Result<QueryResult, ExtensionError> {
        let mut result: duckdb_result = unsafe { std::mem::zeroed() };
        // SAFETY: `self.statement` is valid for this value's lifetime;
        // `result` is a fresh out-parameter DuckDB fills in.
        let state = unsafe {
            libduckdb_sys::duckdb_execute_prepared_streaming(self.statement, &raw mut result)
        };
        if state == DuckDBSuccess {
            return Ok(QueryResult { result });
        }
        // SAFETY: on failure DuckDB still populates the error slot; the result
        // must be destroyed either way.
        let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
            .unwrap_or_else(|| String::from("streaming execution failed without a message"));
        // SAFETY: destroyed exactly once, here, on the error path.
        unsafe { duckdb_destroy_result(&raw mut result) };
        Err(ExtensionError::new(message))
    }

    /// Clears every binding, so the statement can be reused with fresh values.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` rejects the request.
    pub fn clear_bindings(&self) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        if unsafe { duckdb_clear_bindings(self.statement) } == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new("duckdb_clear_bindings failed"))
        }
    }

    /// Executes the statement with its current bindings.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] carrying `DuckDB`'s message if execution
    /// fails.
    pub fn execute(&self) -> Result<QueryResult, ExtensionError> {
        // SAFETY: `duckdb_result` is a plain C struct with no invalid bit patterns;
        // DuckDB overwrites it entirely before it is read.
        let mut result: duckdb_result = unsafe { std::mem::zeroed() };
        // SAFETY: `self.statement` is valid for this value's lifetime.
        let state = unsafe { duckdb_execute_prepared(self.statement, &raw mut result) };
        if state == DuckDBSuccess {
            return Ok(QueryResult { result });
        }
        // SAFETY: DuckDB populated `result` even on failure.
        let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
            .unwrap_or_else(|| String::from("prepared statement failed without an error message"));
        // SAFETY: destroyed exactly once, here, on the error path.
        unsafe { duckdb_destroy_result(&raw mut result) };
        Err(ExtensionError::new(message))
    }

    /// Returns the raw `duckdb_prepared_statement`.
    ///
    /// Do not destroy it — this value still owns it.
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_prepared_statement {
        self.statement
    }

    fn check(
        &self,
        state: libduckdb_sys::duckdb_state,
        index: usize,
    ) -> Result<(), ExtensionError> {
        if state == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "failed to bind parameter {index} (statement has {} parameter(s))",
                self.parameter_count()
            )))
        }
    }
}

impl std::fmt::Debug for PreparedStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedStatement")
            .field("statement", &self.statement)
            .finish()
    }
}

impl Drop for PreparedStatement {
    fn drop(&mut self) {
        // SAFETY: `self.statement` was owned by this value and is destroyed once.
        unsafe { duckdb_destroy_prepare(&raw mut self.statement) };
    }
}

/// Prepares `sql` on `con`.
///
/// # Errors
///
/// Returns [`ExtensionError`] carrying `DuckDB`'s parse/bind error, or if `sql`
/// contains an interior NUL byte.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
pub unsafe fn prepare(
    con: duckdb_connection,
    sql: &str,
) -> Result<PreparedStatement, ExtensionError> {
    let c_sql = to_c_sql(sql)?;
    let mut statement: duckdb_prepared_statement = std::ptr::null_mut();
    // SAFETY: `con` is valid per the caller's contract; `c_sql` outlives the call.
    let state = unsafe { duckdb_prepare(con, c_sql.as_ptr(), &raw mut statement) };
    if state == DuckDBSuccess {
        return Ok(PreparedStatement { statement });
    }
    // SAFETY: on failure DuckDB still allocates the statement so the error is
    // readable; it must be destroyed either way.
    let message = unsafe { c_str_to_owned(duckdb_prepare_error(statement)) }
        .unwrap_or_else(|| String::from("prepare failed without an error message"));
    // SAFETY: destroyed exactly once, here, on the error path.
    unsafe { duckdb_destroy_prepare(&raw mut statement) };
    Err(ExtensionError::new(message))
}

// ─── Cancellation and progress ───────────────────────────────────────────────

/// A snapshot of a running query's progress.
///
/// `percentage` is `-1.0` when `DuckDB` cannot report progress — the progress
/// bar is disabled (`SET enable_progress_bar = true` turns it on), no query is
/// running, or the connection handle was null.
///
/// The three fields are read from separate atomics, so they are individually
/// current but not a consistent snapshot of one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueryProgress {
    /// Percent complete in `0.0..=100.0`, or `-1.0` when unavailable.
    pub percentage: f64,
    /// Rows processed so far.
    pub rows_processed: u64,
    /// Rows the plan expects to process in total.
    pub total_rows_to_process: u64,
}

/// Requests cancellation of whatever `con` is currently running.
///
/// The call itself is non-blocking: it sets `ClientContext::interrupted`, an
/// `atomic<bool>` the executor polls, so the running query fails with an
/// interrupt error at its next check rather than immediately. Calling it while
/// no query is running arms nothing — `DuckDB` clears the flag when a query
/// starts.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection` **for the duration of this
/// call**. It is sound to call this from a different thread than the one
/// running the query — that is the point of the API — but it is not sound to
/// race it against `duckdb_disconnect`. Prefer
/// [`OwnedConnection::interrupt_handle`], whose lifetime enforces that.
pub unsafe fn interrupt(con: duckdb_connection) {
    // SAFETY: `con` is valid per the caller's contract; DuckDB null-checks it
    // itself, and the flag it sets is atomic.
    unsafe { libduckdb_sys::duckdb_interrupt(con) };
}

/// Reads the progress of whatever `con` is currently running.
///
/// # Safety
///
/// Same contract as [`interrupt`].
#[must_use]
pub unsafe fn query_progress(con: duckdb_connection) -> QueryProgress {
    // SAFETY: `con` is valid per the caller's contract; every field DuckDB
    // reads is an atomic.
    let raw = unsafe { libduckdb_sys::duckdb_query_progress(con) };
    QueryProgress {
        percentage: raw.percentage,
        rows_processed: raw.rows_processed,
        total_rows_to_process: raw.total_rows_to_process,
    }
}

/// A cancellation handle for an [`OwnedConnection`], usable from another thread.
///
/// Both operations reach `DuckDB` through atomics — `ClientContext::interrupted`
/// is an `atomic<bool>`, and every field of `QueryProgress` is an atomic — so
/// this is `Send + Sync`. The borrow of the connection is what keeps it sound:
/// the handle cannot outlive the `duckdb_disconnect` in
/// [`OwnedConnection`]'s `Drop`.
///
/// # Example
///
/// ```rust,no_run
/// # use quack_rs::query::OwnedConnection;
/// # fn demo(con: &OwnedConnection) -> Result<(), quack_rs::error::ExtensionError> {
/// let watchdog = con.interrupt_handle();
/// std::thread::scope(|scope| {
///     scope.spawn(|| {
///         std::thread::sleep(std::time::Duration::from_secs(30));
///         watchdog.cancel();
///     });
///     con.query("SELECT count(*) FROM huge_table")
/// })?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct InterruptHandle<'a> {
    con: duckdb_connection,
    _borrow: core::marker::PhantomData<&'a OwnedConnection>,
}

// SAFETY: the only operations are `duckdb_interrupt` and
// `duckdb_query_progress`. The first sets `ClientContext::interrupted`, an
// `atomic<bool>`; the second copy-constructs a `QueryProgress` whose three
// fields are `atomic<double>` / `atomic<uint64_t>`. Neither touches
// non-atomic connection state, and the `'a` borrow rules out a concurrent
// disconnect.
unsafe impl Send for InterruptHandle<'_> {}
// SAFETY: as above — both operations take `&self` and are atomic.
unsafe impl Sync for InterruptHandle<'_> {}

impl InterruptHandle<'_> {
    /// Requests cancellation of the query currently running on the connection.
    pub fn cancel(&self) {
        // SAFETY: the `'a` borrow keeps the connection open for this call.
        unsafe { interrupt(self.con) };
    }

    /// Reads the progress of the query currently running on the connection.
    #[must_use]
    pub fn progress(&self) -> QueryProgress {
        // SAFETY: the `'a` borrow keeps the connection open for this call.
        unsafe { query_progress(self.con) }
    }
}

// ─── Owned connection ────────────────────────────────────────────────────────

/// A `duckdb_connection` this crate opened and will disconnect on drop.
///
/// Open one during registration when the extension needs to run SQL later —
/// from a background thread, or from a callback, where the C API offers no way
/// back to a connection. A connection created from the load-time
/// `duckdb_database` holds a `shared_ptr` to the database instance, so it
/// outlives extension loading.
///
/// This is *not* the connection `DuckDB` passes to your entry point; that one is
/// borrowed and is disconnected when registration returns.
pub struct OwnedConnection {
    con: duckdb_connection,
}

impl OwnedConnection {
    /// Opens a new connection to `db`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `DuckDB` refuses to open the connection.
    ///
    /// # Safety
    ///
    /// `db` must be a valid `duckdb_database`, such as the one obtained from
    /// [`Connection::as_raw_database`][crate::connection::Connection::as_raw_database].
    pub unsafe fn open(db: duckdb_database) -> Result<Self, ExtensionError> {
        let mut con: duckdb_connection = std::ptr::null_mut();
        // SAFETY: `db` is valid per the caller's contract.
        if unsafe { duckdb_connect(db, &raw mut con) } != DuckDBSuccess {
            return Err(ExtensionError::new("duckdb_connect failed"));
        }
        Ok(Self { con })
    }

    /// Runs `sql` on this connection.
    ///
    /// # Errors
    ///
    /// See [`query`].
    pub fn query(&self, sql: &str) -> Result<QueryResult, ExtensionError> {
        // SAFETY: `self.con` is open for this value's lifetime.
        unsafe { query(self.con, sql) }
    }

    /// Runs `sql` for its side effects, returning the number of rows changed.
    ///
    /// # Errors
    ///
    /// See [`query`].
    pub fn execute(&self, sql: &str) -> Result<u64, ExtensionError> {
        // SAFETY: `self.con` is open for this value's lifetime.
        unsafe { execute(self.con, sql) }
    }

    /// Prepares `sql` on this connection.
    ///
    /// # Errors
    ///
    /// See [`prepare`].
    pub fn prepare(&self, sql: &str) -> Result<PreparedStatement, ExtensionError> {
        // SAFETY: `self.con` is open for this value's lifetime.
        unsafe { prepare(self.con, sql) }
    }

    /// Returns a `Send + Sync` handle for cancelling a query running on this
    /// connection, or reading its progress, from another thread.
    ///
    /// The handle borrows the connection, so it cannot outlive the
    /// `duckdb_disconnect` in [`Drop`].
    #[must_use]
    pub const fn interrupt_handle(&self) -> InterruptHandle<'_> {
        InterruptHandle {
            con: self.con,
            _borrow: core::marker::PhantomData,
        }
    }

    /// Requests cancellation of whatever this connection is currently running.
    ///
    /// See [`interrupt`] for what "cancellation" means here. To cancel from
    /// another thread, use [`interrupt_handle`][Self::interrupt_handle].
    pub fn interrupt(&self) {
        // SAFETY: `self.con` is open for this value's lifetime.
        unsafe { interrupt(self.con) };
    }

    /// Reads the progress of whatever this connection is currently running.
    #[must_use]
    pub fn progress(&self) -> QueryProgress {
        // SAFETY: `self.con` is open for this value's lifetime.
        unsafe { query_progress(self.con) }
    }

    /// Returns the raw handle. Do not disconnect it — this value still owns it.
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_connection {
        self.con
    }
}

impl std::fmt::Debug for OwnedConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedConnection")
            .field("con", &self.con)
            .finish()
    }
}

impl Drop for OwnedConnection {
    fn drop(&mut self) {
        // SAFETY: `self.con` was opened by this value and is disconnected once.
        unsafe { duckdb_disconnect(&raw mut self.con) };
    }
}

// SAFETY: a duckdb_connection is a `duckdb::Connection *`, which owns its own
// ClientContext and may be moved between threads. It is *not* `Sync`: DuckDB
// does not permit concurrent use of one connection, so `OwnedConnection`
// deliberately does not implement `Sync`.
unsafe impl Send for OwnedConnection {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_connection_is_send() {
        // DuckDB allows moving a connection between threads. It is deliberately
        // not `Sync`: DuckDB does not permit concurrent use of one connection,
        // and this module deliberately carries no `unsafe impl Sync` to grant
        // it — `&OwnedConnection` therefore cannot cross a thread boundary, so
        // two threads cannot reach the same connection through this type.
        const fn assert_send<T: Send>() {}
        assert_send::<OwnedConnection>();
    }
}

/// Tests that need a live `DuckDB`.
#[cfg(all(test, feature = "_duckdb-testing"))]
mod live_tests {
    use super::*;
    use crate::testing::InMemoryDb;

    /// Opens a raw connection against a fresh in-memory database.
    ///
    /// Returns the database handle too: it must outlive the connection.
    unsafe fn open_raw() -> (libduckdb_sys::duckdb_database, duckdb_connection) {
        let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
        let mut con: duckdb_connection = std::ptr::null_mut();
        // SAFETY: standard open/connect sequence against an in-memory database.
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
        (db, con)
    }

    /// Closes what `open_raw` produced.
    ///
    /// # Safety
    ///
    /// `con` and `db` must come from `open_raw` and not have been closed.
    unsafe fn close_raw(mut db: libduckdb_sys::duckdb_database, mut con: duckdb_connection) {
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut con);
            libduckdb_sys::duckdb_close(&raw mut db);
        }
    }

    #[test]
    fn query_reads_scalar_results() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let mut result =
            unsafe { query(con, "SELECT 42 AS answer, 'hi' AS greeting") }.expect("query succeeds");
        assert_eq!(result.column_count(), 2);
        assert_eq!(result.column_name(0).as_deref(), Some("answer"));
        assert_eq!(result.column_name(1).as_deref(), Some("greeting"));
        assert_eq!(result.column_type(0), Some(TypeId::Integer));
        assert_eq!(result.column_type(1), Some(TypeId::Varchar));
        assert_eq!(result.column_name(2), None);
        assert_eq!(result.column_type(2), None);

        let chunk = result.next_chunk().expect("one chunk");
        assert_eq!(chunk.size(), 1);
        // SAFETY: column 0 is INTEGER, column 1 is VARCHAR, row 0 exists.
        unsafe {
            assert_eq!(chunk.reader(0).read_i32(0), 42);
            assert_eq!(chunk.reader(1).read_str(0), "hi");
        }
        drop(chunk);
        assert!(result.next_chunk().is_none());

        drop(result);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn query_surfaces_duckdb_errors() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let err = unsafe { query(con, "SELECT * FROM no_such_table") }
            .expect_err("missing table must fail");
        assert!(err.as_str().contains("no_such_table"), "{err}");

        // The connection is still usable after a failed query.
        let ok = unsafe { query(con, "SELECT 1") };
        assert!(ok.is_ok());

        unsafe { close_raw(db, con) };
    }

    #[test]
    fn execute_reports_rows_changed() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        unsafe { execute(con, "CREATE TABLE t(i INTEGER)") }.expect("create");
        let changed =
            unsafe { execute(con, "INSERT INTO t VALUES (1), (2), (3)") }.expect("insert");
        assert_eq!(changed, 3);

        unsafe { close_raw(db, con) };
    }

    #[test]
    fn multi_chunk_results_are_fully_drained() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        // More rows than one vector holds, so DuckDB returns several chunks.
        let rows = crate::vector::vector_size() * 3 + 7;
        let sql = format!("SELECT i FROM range({rows}) t(i)");
        let mut result = unsafe { query(con, &sql) }.expect("range query");

        let mut seen: u64 = 0;
        let mut chunks = 0;
        while let Some(chunk) = result.next_chunk() {
            chunks += 1;
            for row in 0..chunk.size() {
                // SAFETY: `range()` yields BIGINT, and `row` is in bounds.
                assert_eq!(
                    unsafe { chunk.reader(0).read_i64(row) },
                    i64::try_from(seen).expect("row counter fits in i64")
                );
                seen += 1;
            }
        }
        assert_eq!(seen, rows);
        assert!(chunks > 1, "expected several chunks, got {chunks}");

        drop(result);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn prepared_statements_bind_and_execute() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let stmt = unsafe { prepare(con, "SELECT ? + ?") }.expect("prepare");
        assert_eq!(stmt.parameter_count(), 2);
        stmt.bind_i64(1, 20).expect("bind 1");
        stmt.bind_i64(2, 22).expect("bind 2");
        let mut result = stmt.execute().expect("execute");
        let chunk = result.next_chunk().expect("one chunk");
        // SAFETY: the expression yields BIGINT and row 0 exists.
        assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 42);

        drop(chunk);
        drop(result);
        drop(stmt);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn prepared_statements_are_reusable_after_clearing() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let stmt = unsafe { prepare(con, "SELECT ?::BIGINT * 2") }.expect("prepare");
        for input in [1_i64, 7, 100] {
            stmt.clear_bindings().expect("clear");
            stmt.bind_i64(1, input).expect("bind");
            let mut result = stmt.execute().expect("execute");
            let chunk = result.next_chunk().expect("one chunk");
            // SAFETY: the expression yields BIGINT and row 0 exists.
            assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, input * 2);
        }

        drop(stmt);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn binding_a_string_avoids_sql_injection() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        unsafe { execute(con, "CREATE TABLE t(s VARCHAR)") }.expect("create");
        let stmt = unsafe { prepare(con, "INSERT INTO t VALUES (?)") }.expect("prepare");
        // Text that would be catastrophic if interpolated into SQL.
        stmt.bind_str(1, "'); DROP TABLE t; --").expect("bind");
        stmt.execute().expect("insert");
        drop(stmt);

        let mut result = unsafe { query(con, "SELECT s FROM t") }.expect("select");
        let chunk = result.next_chunk().expect("one chunk");
        assert_eq!(chunk.size(), 1);
        // SAFETY: column 0 is VARCHAR and row 0 exists.
        assert_eq!(
            unsafe { chunk.reader(0).read_str(0) },
            "'); DROP TABLE t; --"
        );

        drop(chunk);
        drop(result);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn named_parameters_resolve_by_name() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let stmt = unsafe { prepare(con, "SELECT $needle::BIGINT") }.expect("prepare");
        let index = stmt.parameter_index("needle").expect("named parameter");
        assert_eq!(stmt.parameter_name(index).as_deref(), Some("needle"));
        assert_eq!(stmt.parameter_index("nope"), None);
        stmt.bind_i64(index, 5).expect("bind");
        let mut result = stmt.execute().expect("execute");
        let chunk = result.next_chunk().expect("one chunk");
        // SAFETY: the expression yields BIGINT and row 0 exists.
        assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 5);

        drop(chunk);
        drop(result);
        drop(stmt);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn prepare_surfaces_parse_errors() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        let err = unsafe { prepare(con, "SELECT FROM WHERE") }.expect_err("syntax error");
        assert!(!err.as_str().is_empty());

        unsafe { close_raw(db, con) };
    }

    #[test]
    fn null_and_blob_bindings_round_trip() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let (db, con) = unsafe { open_raw() };

        unsafe { execute(con, "CREATE TABLE t(b BLOB, n INTEGER)") }.expect("create");
        let stmt = unsafe { prepare(con, "INSERT INTO t VALUES (?, ?)") }.expect("prepare");
        stmt.bind_blob(1, &[0x00, 0xFF, 0x80]).expect("bind blob");
        stmt.bind_null(2).expect("bind null");
        stmt.execute().expect("insert");
        drop(stmt);

        let mut result = unsafe { query(con, "SELECT b, n FROM t") }.expect("select");
        let chunk = result.next_chunk().expect("one chunk");
        // SAFETY: column 0 is BLOB, column 1 is INTEGER, row 0 exists.
        unsafe {
            assert_eq!(chunk.reader(0).read_blob(0), &[0x00, 0xFF, 0x80]);
            assert!(!chunk.reader(1).is_valid(0));
        }

        drop(chunk);
        drop(result);
        unsafe { close_raw(db, con) };
    }

    #[test]
    fn owned_connection_outlives_the_database_handle() {
        let _guard = InMemoryDb::open().expect("dispatch table");
        let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
        // SAFETY: standard open against an in-memory database.
        unsafe {
            assert_eq!(
                libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
                DuckDBSuccess
            );
        }
        // SAFETY: `db` was just opened.
        let con = unsafe { OwnedConnection::open(db) }.expect("connect");

        // Close the database handle, mirroring what happens when extension
        // loading finishes and DuckDB drops its DatabaseWrapper. The connection
        // holds its own reference, so it stays usable.
        // SAFETY: `db` is closed exactly once; `con` keeps the instance alive.
        unsafe { libduckdb_sys::duckdb_close(&raw mut db) };

        con.execute("CREATE TABLE t(i INTEGER)").expect("create");
        con.execute("INSERT INTO t VALUES (1), (2)")
            .expect("insert");
        let mut result = con.query("SELECT count(*) FROM t").expect("count");
        let chunk = result.next_chunk().expect("one chunk");
        // SAFETY: count(*) yields BIGINT and row 0 exists.
        assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 2);
    }
}
