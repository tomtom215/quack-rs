// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Bulk data appending.
//!
//! [`Appender`] is an RAII wrapper around `DuckDB`'s appender — the fastest way
//! to bulk-insert rows into an existing table, and considerably faster than
//! issuing `INSERT` statements.
//!
//! # Two ways to append
//!
//! **Row at a time.** Call one `append_*` per column, then
//! [`end_row`][Appender::end_row] — or let [`row`][Appender::row] call it for
//! you, which is the difference between a forgotten `end_row` being a compile
//! -time non-issue and a silently short table:
//!
//! ```rust,no_run
//! use quack_rs::appender::Appender;
//! # use libduckdb_sys::duckdb_connection;
//! # unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::appender::AppendError> {
//! // SAFETY: `con` is a valid, open connection.
//! let appender = unsafe { Appender::new(con, None, c"measurements") }?;
//! for (sensor, reading) in [("a", 1.5_f64), ("b", 2.5)] {
//!     appender.row(|row| {
//!         row.append_str(sensor)?;
//!         row.append_f64(reading)
//!     })?;
//! }
//! appender.close()?;
//! # Ok(())
//! # }
//! ```
//!
//! **A chunk at a time.** Build a [`DataChunk`] and hand it over with
//! [`append_chunk`][Appender::append_chunk]. Fewer FFI crossings, and the
//! natural fit when the data already lives in vectors.
//!
//! # Errors and the appender's lifecycle
//!
//! Appended rows are buffered. A constraint violation therefore surfaces at
//! [`flush`][Appender::flush] or [`close`][Appender::close], not at the
//! `append_*` call that caused it, and it **invalidates every buffered row**.
//!
//! Dropping an `Appender` closes it, and a failure there has nowhere to go —
//! `DuckDB`'s own header is explicit that after destruction "it is no longer
//! possible to obtain the specific error message". Call
//! [`close`][Appender::close] explicitly whenever the outcome matters.
//!
//! ## A row that fails half-way
//!
//! `DuckDB` counts the values of the current row and has no way to take one
//! back: `end_row` wants every column, `flush` wants none, and its `Close`
//! flushes only in those two states — in any other it returns *success* and
//! writes nothing. A [`row`][Appender::row] whose closure fails after its first
//! value therefore leaves a half-written row that cannot be completed or
//! dropped, and with it **every row buffered since the last flush is lost**
//! (`DuckDB` also flushes on its own each time 204,800 rows accumulate; rows
//! before that are safe).
//!
//! quack-rs tracks the row itself so that this is never silent:
//!
//! - the appender becomes *poisoned*: every later append, `row`, `end_row`,
//!   `flush` and `close` returns an error that says how many buffered rows
//!   were not written;
//! - `close` with a row started but not ended (by `row` or by hand) is an
//!   error rather than a silent no-op;
//! - with `duckdb-1-5`, `clear` discards the buffered rows and the half row
//!   and makes the appender usable again. Without it, a poisoned appender
//!   stays poisoned.
//!
//! A value that fails as the *first* of its row loses nothing (`DuckDB` has
//! not counted it), and one that fails later in a row appended by hand can be
//! retried — only an abandoned row poisons. If losing buffered rows is not
//! acceptable, [`flush`][Appender::flush] at the points you can afford to
//! lose work back to.
//!
//! After a successful [`close`][Appender::close] the appender refuses further
//! work; `DuckDB` itself would accept appends and write them at the next
//! flush.
//!
//! ## Schema changes while rows are buffered
//!
//! Buffered rows are written by column *position* when they are flushed. If
//! another connection drops a column and adds one in between, a buffered
//! value lands in the new column (cast to its type) with no error. Flush
//! before a concurrent `ALTER TABLE` if that matters.
//!
//! # Feature flags
//!
//! The appender is available **without** any feature flag: `DuckDB` has kept
//! `duckdb_appender_*` in the frozen stable prefix of the extension API
//! (slots 281–291 and 330–356) since v1.2.0, so using it does not push an
//! extension onto the version-pinned unstable ABI. Three methods are the
//! exception and are gated on `duckdb-1-5`: `error_data`, `clear` and
//! `append_default_to_chunk`. They are not linked here because the links would
//! not resolve when the feature is off, which is exactly when a reader most
//! wants to know they exist.
//!
//! That gate also picks the error type — see [`AppendError`].

use std::cell::Cell;
use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_append_blob, duckdb_append_data_chunk, duckdb_append_date, duckdb_append_default,
    duckdb_append_hugeint, duckdb_append_interval, duckdb_append_null, duckdb_append_time,
    duckdb_append_timestamp, duckdb_append_uhugeint, duckdb_append_value,
    duckdb_append_varchar_length, duckdb_appender, duckdb_appender_add_column,
    duckdb_appender_clear_columns, duckdb_appender_close, duckdb_appender_column_count,
    duckdb_appender_column_type, duckdb_appender_create, duckdb_appender_create_ext,
    duckdb_appender_destroy, duckdb_appender_end_row, duckdb_appender_error, duckdb_appender_flush,
    duckdb_connection, duckdb_date, duckdb_hugeint, duckdb_interval, duckdb_state, duckdb_time,
    duckdb_timestamp, duckdb_uhugeint, idx_t, DuckDBSuccess,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{duckdb_append_default_to_chunk, duckdb_appender_clear};

use crate::data_chunk::DataChunk;
#[cfg(feature = "duckdb-1-5")]
use crate::error_data::ErrorData;
use crate::interval::DuckInterval;
use crate::types::LogicalType;
use crate::value::Value;

/// The error type every fallible [`Appender`] operation reports.
///
/// `DuckDB` exposes the appender's error two ways, and only one of them is in
/// the stable prefix:
///
/// | Feature | Type | C API |
/// |---------|------|-------|
/// | `duckdb-1-5` on | [`ErrorData`] — message **and** machine-readable category | `duckdb_appender_error_data` (unstable slot 408) |
/// | `duckdb-1-5` off | [`ExtensionError`][crate::error::ExtensionError] — message only | `duckdb_appender_error` (stable slot 285) |
///
/// Enabling `duckdb-1-5` therefore upgrades the error type in place; it does
/// not change any method's shape.
#[cfg(feature = "duckdb-1-5")]
pub type AppendError = ErrorData;

/// The error type every fallible [`Appender`] operation reports.
///
/// See the `duckdb-1-5` variant of this alias for the full explanation: without
/// that feature the appender reports errors through the stable
/// `duckdb_appender_error`, which carries a message but no category.
#[cfg(not(feature = "duckdb-1-5"))]
pub type AppendError = crate::error::ExtensionError;

/// `duckdb_append_varchar_length` narrows its length argument to `uint32_t`
/// with `UnsafeNumericCast`, which is a plain `static_cast` in the release
/// builds `DuckDB` ships. A longer string would be silently truncated to its
/// low 32 bits, so it is refused here instead.
const MAX_VARCHAR_LEN: usize = u32::MAX as usize;

/// Converts an optional `&CStr` into a (possibly null) C string pointer.
#[inline]
fn opt_ptr(s: Option<&CStr>) -> *const std::os::raw::c_char {
    s.map_or(std::ptr::null(), CStr::as_ptr)
}

/// RAII wrapper for a `duckdb_appender`.
///
/// The appender is closed and destroyed automatically on drop. To surface any
/// error from the final flush, call [`close`][Appender::close] explicitly
/// beforehand.
///
/// See the [module docs][crate::appender] for the two append styles and the
/// buffering rules that decide when an error appears.
pub struct Appender {
    handle: duckdb_appender,
    /// Values appended to the current row: a mirror of `DuckDB`'s
    /// `BaseAppender::column`, which advances only on a successful append.
    column: Cell<u64>,
    /// Rows ended or appended as chunks since the last flush this wrapper
    /// performed — what a poisoned appender loses.
    buffered: Cell<u64>,
    lifecycle: Cell<Lifecycle>,
}

/// Where an [`Appender`] is in its life, as far as quack-rs can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    Open,
    /// A `row` was abandoned after some of its values went in; see the module
    /// docs.
    Poisoned,
    /// `close` succeeded.
    Closed,
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
    /// `con` must be a valid, open `duckdb_connection`.
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
    /// `con` must be a valid, open `duckdb_connection`.
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

    // ── Row-at-a-time appends ───────────────────────────────────────────

    /// Appends one row, calling [`end_row`][Self::end_row] afterwards.
    ///
    /// The closure appends one value per active column. `end_row` runs only if
    /// the closure succeeded.
    ///
    /// If the closure (or `end_row`) fails after the row's first value went
    /// in, `DuckDB` is left holding a half-written row it can neither finish
    /// nor drop, and every row buffered since the last flush is lost. The
    /// appender is then *poisoned*: see the [module docs][crate::appender].
    ///
    /// # Errors
    ///
    /// Returns whatever the closure returned, or the [`AppendError`] from
    /// `end_row` — most often "call to `EndRow` before all columns have been
    /// appended to". Also an error, before the closure runs, when the appender
    /// is closed or poisoned, or when a row appended by hand is still open.
    pub fn row<F>(&self, append: F) -> Result<(), AppendError>
    where
        F: FnOnce(&Self) -> Result<(), AppendError>,
    {
        self.usable()?;
        if self.column.get() != 0 {
            return Err(append_error(&format!(
                "row: a row appended by hand is still open ({} value(s) without end_row); \
                 finish it before starting another",
                self.column.get()
            )));
        }
        let result = append(self).and_then(|()| self.end_row());
        if result.is_err() && self.column.get() != 0 {
            self.lifecycle.set(Lifecycle::Poisoned);
        }
        result
    }

    /// Finishes the current row.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if fewer values were appended than the
    /// appender has active columns (append the rest and call it again), or if
    /// the appender is closed or poisoned.
    pub fn end_row(&self) -> Result<(), AppendError> {
        self.usable()?;
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_appender_end_row(self.handle) };
        self.check(state)?;
        self.column.set(0);
        self.buffered.set(self.buffered.get().saturating_add(1));
        Ok(())
    }

    /// Appends SQL `NULL` to the current row, whatever the column's type.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_null(&self) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_null(self.handle) })
    }

    /// Appends the column's `DEFAULT` value to the current row.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the column has no default, or the append
    /// fails.
    pub fn append_default(&self) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_default(self.handle) })
    }

    /// Appends a `VARCHAR`.
    ///
    /// Uses `duckdb_append_varchar_length`, so **interior NUL bytes are
    /// preserved** — unlike the NUL-terminated `duckdb_append_varchar`, which
    /// would stop at the first one.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails, or if `value` is longer
    /// than `u32::MAX` bytes — a length `DuckDB` narrows to 32 bits without
    /// checking in its release builds.
    pub fn append_str(&self, value: &str) -> Result<(), AppendError> {
        self.append_bytes_as(value.as_bytes(), true)
    }

    /// Appends a `BLOB`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_bytes(&self, value: &[u8]) -> Result<(), AppendError> {
        self.append_bytes_as(value, false)
    }

    fn append_bytes_as(&self, value: &[u8], varchar: bool) -> Result<(), AppendError> {
        self.usable()?;
        if varchar {
            if value.len() > MAX_VARCHAR_LEN {
                return Err(append_error(&format!(
                    "VARCHAR of {} bytes exceeds DuckDB's {MAX_VARCHAR_LEN}-byte appender limit",
                    value.len()
                )));
            }
            // SAFETY: self.handle is valid; the pointer/length pair describes
            // `value`, which outlives the call.
            let state = unsafe {
                duckdb_append_varchar_length(
                    self.handle,
                    value.as_ptr().cast::<std::os::raw::c_char>(),
                    value.len() as idx_t,
                )
            };
            return self.record_append(state);
        }
        // SAFETY: as above; DuckDB copies the bytes into a BLOB value.
        let state = unsafe {
            duckdb_append_blob(
                self.handle,
                value.as_ptr().cast::<std::os::raw::c_void>(),
                value.len() as idx_t,
            )
        };
        self.record_append(state)
    }

    /// Appends a `DATE` as days since 1970-01-01.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_date(&self, days: i32) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_date(self.handle, duckdb_date { days }) })
    }

    /// Appends a `TIME` as microseconds since midnight.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_time(&self, micros: i64) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_time(self.handle, duckdb_time { micros }) })
    }

    /// Appends a `TIMESTAMP` as microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_timestamp(&self, micros: i64) -> Result<(), AppendError> {
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe {
            duckdb_append_timestamp(self.handle, duckdb_timestamp { micros })
        })
    }

    /// Appends an `INTERVAL`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_interval(&self, value: DuckInterval) -> Result<(), AppendError> {
        let raw = duckdb_interval {
            months: value.months,
            days: value.days,
            micros: value.micros,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_interval(self.handle, raw) })
    }

    /// Appends an arbitrary [`Value`], letting `DuckDB` cast it to the column's
    /// type.
    ///
    /// This is the escape hatch for types with no dedicated `append_*`:
    /// `LIST`, `STRUCT`, `MAP`, `UUID`, `DECIMAL`, `ENUM`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if `value` holds a null handle — which
    /// `duckdb_append_value` would dereference — or if the append fails.
    pub fn append_value(&self, value: &Value) -> Result<(), AppendError> {
        self.usable()?;
        if value.as_raw().is_null() {
            // duckdb_append_value dereferences its argument with no null check.
            return Err(append_error("cannot append a null duckdb_value handle"));
        }
        // SAFETY: self.handle is valid and value.as_raw() is non-null.
        self.record_append(unsafe { duckdb_append_value(self.handle, value.as_raw()) })
    }

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

    // ── Errors ──────────────────────────────────────────────────────────

    /// Returns the structured error from the most recent failed operation.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        // SAFETY: self.handle may be null (a failed create); DuckDB handles
        // that and returns an owned, empty error data handle.
        let raw = unsafe { libduckdb_sys::duckdb_appender_error_data(self.handle) };
        // SAFETY: raw is an owned duckdb_error_data (possibly null).
        unsafe { ErrorData::from_raw(raw) }
    }

    /// Returns the message from the most recent failed operation, if any.
    ///
    /// Always available; with `duckdb-1-5` prefer
    /// `error_data` (`duckdb-1-5`), which also carries the error category.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        if self.handle.is_null() {
            return None;
        }
        // SAFETY: self.handle is non-null; DuckDB returns null when there is
        // no error, and otherwise a string it owns until the appender is
        // destroyed — so it is copied out here rather than borrowed.
        let ptr = unsafe { duckdb_appender_error(self.handle) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid NUL-terminated string owned by the appender.
        Some(
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_appender {
        self.handle
    }

    /// Reads whichever error channel this build has.
    #[cfg(feature = "duckdb-1-5")]
    fn last_error(&self) -> AppendError {
        self.error_data()
    }

    /// Reads whichever error channel this build has.
    #[cfg(not(feature = "duckdb-1-5"))]
    fn last_error(&self) -> AppendError {
        self.error_message().map_or_else(
            || append_error("appender operation failed"),
            crate::error::ExtensionError::new,
        )
    }

    /// Converts a `duckdb_state` into a `Result`, reading the appender's error
    /// on failure.
    fn check(&self, state: duckdb_state) -> Result<(), AppendError> {
        if state == DuckDBSuccess {
            Ok(())
        } else {
            Err(self.last_error())
        }
    }

    // ── Row and lifecycle tracking ──────────────────────────────────────

    /// Wraps a handle from `duckdb_appender_create*` in a fresh, open state.
    const fn wrap(handle: duckdb_appender) -> Self {
        Self {
            handle,
            column: Cell::new(0),
            buffered: Cell::new(0),
            lifecycle: Cell::new(Lifecycle::Open),
        }
    }

    /// `Ok` while the appender is open; the reason otherwise.
    fn usable(&self) -> Result<(), AppendError> {
        match self.lifecycle.get() {
            Lifecycle::Open => Ok(()),
            Lifecycle::Closed => Err(append_error(
                "the appender is closed; create a new one to append more rows",
            )),
            Lifecycle::Poisoned => Err(self.lost_rows_error("append")),
        }
    }

    /// Runs one value append if the appender is open, and counts it.
    fn append_one(&self, append: impl FnOnce() -> duckdb_state) -> Result<(), AppendError> {
        self.usable()?;
        self.record_append(append())
    }

    /// Counts a value append that `DuckDB` accepted. A rejected value leaves
    /// `DuckDB`'s column counter where it was (every append path throws before
    /// advancing it), so the same column can be tried again.
    fn record_append(&self, state: duckdb_state) -> Result<(), AppendError> {
        self.check(state)?;
        self.column.set(self.column.get().saturating_add(1));
        Ok(())
    }

    /// Records the outcome of an operation that flushes on success.
    fn flushed(&self, state: duckdb_state) -> Result<(), AppendError> {
        self.check(state)?;
        self.buffered.set(0);
        Ok(())
    }

    /// The error for rows `DuckDB` will not write: a poisoned appender, or
    /// `close` with an unfinished row.
    fn lost_rows_error(&self, operation: &str) -> AppendError {
        let rows = self.buffered.get();
        let values = self.column.get();
        if self.lifecycle.get() != Lifecycle::Poisoned {
            return append_error(&format!(
                "{operation}: the current row is unfinished ({values} value(s) without \
                 end_row), and DuckDB writes nothing while it is: up to {rows} row(s) buffered \
                 since the last flush are lost if the appender is dropped now; end the row and \
                 close again"
            ));
        }
        let recovery = if cfg!(feature = "duckdb-1-5") {
            "; `clear` discards them and makes the appender usable again"
        } else {
            ""
        };
        append_error(&format!(
            "{operation}: the appender is poisoned: up to {rows} row(s) buffered since the last \
             flush were not written, because a row failed after {values} of its values were \
             appended and DuckDB can neither finish nor drop a half-written row{recovery}"
        ))
    }
}

/// Builds an [`AppendError`] for a failure quack-rs detected itself, before
/// `DuckDB` was ever called.
#[cfg(feature = "duckdb-1-5")]
fn append_error(message: &str) -> AppendError {
    ErrorData::new(crate::error_data::DuckDbErrorType::InvalidInput, message)
}

/// Builds an [`AppendError`] for a failure quack-rs detected itself, before
/// `DuckDB` was ever called.
#[cfg(not(feature = "duckdb-1-5"))]
fn append_error(message: &str) -> AppendError {
    crate::error::ExtensionError::new(message)
}

/// Generates the fixed-width numeric `append_*` methods, which differ only in
/// the C function they call.
macro_rules! append_scalar {
    ($($(#[$attr:meta])* $name:ident($ty:ty) => $c_fn:ident),* $(,)?) => {
        impl Appender {
            $(
                $(#[$attr])*
                ///
                /// # Errors
                ///
                /// Returns an [`AppendError`] if the append fails.
                pub fn $name(&self, value: $ty) -> Result<(), AppendError> {
                    // SAFETY: self.handle is valid.
                    self.append_one(|| unsafe { libduckdb_sys::$c_fn(self.handle, value) })
                }
            )*
        }
    };
}

append_scalar! {
    /// Appends a `BOOLEAN`.
    append_bool(bool) => duckdb_append_bool,
    /// Appends a `TINYINT`.
    append_i8(i8) => duckdb_append_int8,
    /// Appends a `SMALLINT`.
    append_i16(i16) => duckdb_append_int16,
    /// Appends an `INTEGER`.
    append_i32(i32) => duckdb_append_int32,
    /// Appends a `BIGINT`.
    append_i64(i64) => duckdb_append_int64,
    /// Appends a `UTINYINT`.
    append_u8(u8) => duckdb_append_uint8,
    /// Appends a `USMALLINT`.
    append_u16(u16) => duckdb_append_uint16,
    /// Appends a `UINTEGER`.
    append_u32(u32) => duckdb_append_uint32,
    /// Appends a `UBIGINT`.
    append_u64(u64) => duckdb_append_uint64,
    /// Appends a `FLOAT`.
    append_f32(f32) => duckdb_append_float,
    /// Appends a `DOUBLE`.
    append_f64(f64) => duckdb_append_double,
}

impl Appender {
    /// Appends a `HUGEINT`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_i128(&self, value: i128) -> Result<(), AppendError> {
        let raw = duckdb_hugeint {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            lower: value as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (value >> 64) as i64,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_hugeint(self.handle, raw) })
    }

    /// Appends a `UHUGEINT`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_u128(&self, value: u128) -> Result<(), AppendError> {
        let raw = duckdb_uhugeint {
            #[allow(clippy::cast_possible_truncation)]
            lower: value as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (value >> 64) as u64,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_uhugeint(self.handle, raw) })
    }
}

impl Drop for Appender {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: self.handle is a valid handle that we own. Destroy
            // closes (and so flushes) it first; the state is intentionally
            // ignored here — `close` beforehand is how a final flush error is
            // observed, because destruction also frees the error message.
            unsafe { duckdb_appender_destroy(&raw mut self.handle) };
        }
    }
}

crate::debug_repr::impl_handle_debug!(Appender.handle);
