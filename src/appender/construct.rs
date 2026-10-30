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

    /// Creates an appender whose rows feed `sql` instead of a table
    /// (`DuckDB` 1.5.0+): each flush runs `sql` with the buffered rows
    /// available as a table — an `INSERT … SELECT`, `UPDATE … FROM`,
    /// `DELETE … USING`, `INSERT OR REPLACE` or `MERGE INTO` over the batch.
    ///
    /// `types` are the appended columns' types. The rows are visible to
    /// `sql` as the table `table` (default `appended_data`) with the column
    /// names `columns` (default `col1`, `col2`, …, 1-based). Rows are
    /// appended exactly as for a table appender. `DuckDB` does not look at
    /// `sql` until a flush runs it (`QueryAppender`'s constructor only stores
    /// it), so a query that does not parse or bind, like a constraint it
    /// violates, fails at [`flush`][Self::flush] or [`close`][Self::close],
    /// and the rows buffered for it are lost as for any failed flush.
    ///
    /// Requires `duckdb-1-5`: `duckdb_appender_create_query` sits in the
    /// unstable region of the C API struct.
    ///
    /// # Errors
    ///
    /// Before `DuckDB` is called: `types` is empty, `columns` does not have
    /// one name per type (`duckdb_appender_create_query` would read past
    /// the end of a short list), or `sql`, `table` or a column name contains
    /// an interior NUL byte. Otherwise `DuckDB`'s error, if it refuses to
    /// create the appender.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`; see
    /// [`new`][Self::new] for how long it must outlive the appender.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::appender::Appender;
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// # unsafe fn demo(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::appender::AppendError> {
    /// let types = [LogicalType::new(TypeId::Integer), LogicalType::new(TypeId::Varchar)];
    /// // SAFETY: `con` is open.
    /// let upsert = unsafe {
    ///     Appender::for_query(
    ///         con,
    ///         "INSERT OR REPLACE INTO users SELECT id, name FROM batch",
    ///         &types,
    ///         Some("batch"),
    ///         Some(&["id", "name"]),
    ///     )
    /// }?;
    /// upsert.row(|row| {
    ///     row.append_i32(7)?;
    ///     row.append_str("ada")
    /// })?;
    /// upsert.close()?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "duckdb-1-5")]
    pub unsafe fn for_query(
        con: duckdb_connection,
        sql: &str,
        types: &[LogicalType],
        table: Option<&str>,
        columns: Option<&[&str]>,
    ) -> Result<Self, AppendError> {
        use std::ffi::CString;
        let nul = |what: &str| {
            super::append_error(&format!(
                "Appender::for_query: {what} contains an interior NUL byte"
            ))
        };
        if types.is_empty() {
            return Err(super::append_error(
                "Appender::for_query: at least one column type is needed",
            ));
        }
        if let Some(names) = columns.filter(|names| names.len() != types.len()) {
            return Err(super::append_error(&format!(
                "Appender::for_query: {} column name(s) for {} column type(s); \
                 duckdb_appender_create_query reads one name per type",
                names.len(),
                types.len()
            )));
        }
        let c_sql = CString::new(sql).map_err(|_| nul("the SQL"))?;
        let c_table = table
            .map(CString::new)
            .transpose()
            .map_err(|_| nul("the table name"))?;
        let c_columns = columns
            .map(|names| {
                names
                    .iter()
                    .map(|n| CString::new(*n))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|_| nul("a column name"))?;
        let mut name_ptrs: Option<Vec<*const std::os::raw::c_char>> = c_columns
            .as_ref()
            .map(|names| names.iter().map(|n| n.as_ptr()).collect());
        let mut raw_types: Vec<libduckdb_sys::duckdb_logical_type> =
            types.iter().map(LogicalType::as_raw).collect();
        let mut raw: duckdb_appender = std::ptr::null_mut();
        // SAFETY: `con` is valid per the caller's contract. Every pointer
        // outlives the call; `raw_types` and `name_ptrs` hold exactly
        // `types.len()` live entries each (checked above), which is the
        // count passed. DuckDB copies all of them, and catches every
        // exception from creating the appender.
        let state = unsafe {
            libduckdb_sys::duckdb_appender_create_query(
                con,
                c_sql.as_ptr(),
                types.len() as idx_t,
                raw_types.as_mut_ptr(),
                c_table.as_deref().map_or(std::ptr::null(), CStr::as_ptr),
                name_ptrs
                    .as_mut()
                    .map_or(std::ptr::null_mut(), Vec::as_mut_ptr),
                &raw mut raw,
            )
        };
        // As in `new`: on a failure after allocation DuckDB still hands out
        // the wrapper, so that its error is readable and it is destroyed.
        let appender = Self::wrap(raw);
        if state == DuckDBSuccess {
            Ok(appender)
        } else if appender.handle.is_null() {
            Err(super::append_error(
                "duckdb_appender_create_query refused its arguments",
            ))
        } else {
            Err(appender.last_error())
        }
    }

    /// Takes ownership of a raw appender, which is then closed and destroyed
    /// (`duckdb_appender_destroy`) on drop: the way back into this crate's
    /// RAII for an appender a raw call created.
    ///
    /// # Safety
    ///
    /// `handle` must come from a `duckdb_appender_create*` call, must not be
    /// destroyed by anyone else, and must be at a row boundary (no values
    /// appended since its last `duckdb_appender_end_row`): the row and
    /// poisoning checks described in the [module docs][crate::appender]
    /// start counting from here. See [`new`][Self::new] for how long its
    /// connection must stay open.
    #[must_use]
    pub const unsafe fn from_raw(handle: duckdb_appender) -> Self {
        Self::wrap(handle)
    }

    /// Gives up ownership and returns the raw appender without flushing or
    /// closing it. The caller must destroy it (`duckdb_appender_destroy`,
    /// which flushes) or hand it back to [`from_raw`][Self::from_raw]; this
    /// wrapper's record of a half-written row or a poisoned state is lost.
    #[must_use]
    pub const fn into_raw(self) -> duckdb_appender {
        let raw = self.handle;
        std::mem::forget(self);
        raw
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
