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
//! # Feature flags
//!
//! The appender is available **without** any feature flag: `DuckDB` has kept
//! `duckdb_appender_*` in the frozen stable prefix of the extension API
//! (slots 281–291 and 330–356) since v1.2.0, so using it does not push an
//! extension onto the version-pinned unstable ABI. Three methods are the
//! exception and are gated on `duckdb-1-5`:
//! `error_data`, `clear` and `append_default_to_chunk`. (Not linked: with the
//! feature off those items do not exist, and an intra-doc link to a missing
//! item is a rustdoc error — which is how the default-feature `cargo doc`
//! build broke in 0.16.0.)
//!
//! That gate also picks the error type — see [`AppendError`].

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
    appender: duckdb_appender,
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
        let appender = Self { appender: raw };
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
        let appender = Self { appender: raw };
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
        // SAFETY: self.appender is valid; DuckDB returns 0 for a null or
        // uninitialised appender.
        unsafe { duckdb_appender_column_count(self.appender) }
    }

    /// Type of active column `index`, or `None` if the index is out of range.
    #[must_use]
    pub fn column_type(&self, index: u64) -> Option<LogicalType> {
        // SAFETY: self.appender is valid; DuckDB bounds-checks `index` and
        // returns null when it is out of range.
        let raw = unsafe { duckdb_appender_column_type(self.appender, index as idx_t) };
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
        // SAFETY: self.appender is valid and `name` is a NUL-terminated string
        // that outlives the call.
        let state = unsafe { duckdb_appender_add_column(self.appender, name.as_ptr()) };
        self.check(state)
    }

    /// Resets the active column list so every table column is expected again.
    ///
    /// Also flushes everything appended so far.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the implicit flush fails.
    pub fn clear_columns(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_clear_columns(self.appender) };
        self.check(state)
    }

    // ── Row-at-a-time appends ───────────────────────────────────────────

    /// Appends one row, calling [`end_row`][Self::end_row] afterwards.
    ///
    /// The closure appends one value per active column. `end_row` runs only if
    /// the closure succeeded, so a failed append does not leave a half-written
    /// row behind.
    ///
    /// # Errors
    ///
    /// Returns whatever the closure returned, or the [`AppendError`] from
    /// `end_row` — most often "call to `EndRow` before all columns have been
    /// appended to".
    pub fn row<F>(&self, append: F) -> Result<(), AppendError>
    where
        F: FnOnce(&Self) -> Result<(), AppendError>,
    {
        append(self)?;
        self.end_row()
    }

    /// Finishes the current row.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if fewer values were appended than the
    /// appender has active columns.
    pub fn end_row(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_end_row(self.appender) };
        self.check(state)
    }

    /// Appends SQL `NULL` to the current row, whatever the column's type.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_null(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_null(self.appender) })
    }

    /// Appends the column's `DEFAULT` value to the current row.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the column has no default, or the append
    /// fails.
    pub fn append_default(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_default(self.appender) })
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
        if varchar {
            if value.len() > MAX_VARCHAR_LEN {
                return Err(append_error(&format!(
                    "VARCHAR of {} bytes exceeds DuckDB's {MAX_VARCHAR_LEN}-byte appender limit",
                    value.len()
                )));
            }
            // SAFETY: self.appender is valid; the pointer/length pair describes
            // `value`, which outlives the call.
            let state = unsafe {
                duckdb_append_varchar_length(
                    self.appender,
                    value.as_ptr().cast::<std::os::raw::c_char>(),
                    value.len() as idx_t,
                )
            };
            return self.check(state);
        }
        // SAFETY: as above; DuckDB copies the bytes into a BLOB value.
        let state = unsafe {
            duckdb_append_blob(
                self.appender,
                value.as_ptr().cast::<std::os::raw::c_void>(),
                value.len() as idx_t,
            )
        };
        self.check(state)
    }

    /// Appends a `DATE` as days since 1970-01-01.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_date(&self, days: i32) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_date(self.appender, duckdb_date { days }) })
    }

    /// Appends a `TIME` as microseconds since midnight.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_time(&self, micros: i64) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_time(self.appender, duckdb_time { micros }) })
    }

    /// Appends a `TIMESTAMP` as microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_timestamp(&self, micros: i64) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_timestamp(self.appender, duckdb_timestamp { micros }) })
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
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_interval(self.appender, raw) })
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
        if value.as_raw().is_null() {
            // duckdb_append_value dereferences its argument with no null check.
            return Err(append_error("cannot append a null duckdb_value handle"));
        }
        // SAFETY: self.appender is valid and value.as_raw() is non-null.
        self.check(unsafe { duckdb_append_value(self.appender, value.as_raw()) })
    }

    // ── Chunk appends ───────────────────────────────────────────────────

    /// Appends an entire [`DataChunk`].
    ///
    /// The chunk's column types must match the appender's active columns; see
    /// [`column_type`][Self::column_type] to discover them.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_chunk(&self, chunk: &DataChunk) -> Result<(), AppendError> {
        // SAFETY: self.appender and chunk.as_raw() are valid.
        let state = unsafe { duckdb_append_data_chunk(self.appender, chunk.as_raw()) };
        self.check(state)
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
        // SAFETY: self.appender and chunk.as_raw() are valid.
        let state =
            unsafe { duckdb_append_default_to_chunk(self.appender, chunk.as_raw(), col, row) };
        self.check(state)
    }

    // ── Lifecycle ───────────────────────────────────────────────────────

    /// Flushes buffered rows to the table without closing the appender.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the flush fails — a constraint violation,
    /// typically. On failure every buffered row is invalidated; with
    /// `duckdb-1-5` they can be discarded with `clear`.
    pub fn flush(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_flush(self.appender) };
        self.check(state)
    }

    /// Flushes and closes the appender. No further rows may be appended.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the final flush fails.
    pub fn close(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_close(self.appender) };
        self.check(state)
    }

    /// Discards all buffered, unflushed rows.
    ///
    /// Useful for recovering after a [`flush`][Self::flush] error without
    /// re-appending the rows that were already committed.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the appender state is invalid.
    #[cfg(feature = "duckdb-1-5")]
    pub fn clear(&self) -> Result<(), AppendError> {
        // SAFETY: self.appender is valid.
        let state = unsafe { duckdb_appender_clear(self.appender) };
        self.check(state)
    }

    // ── Errors ──────────────────────────────────────────────────────────

    /// Returns the structured error from the most recent failed operation.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        // SAFETY: self.appender may be null (a failed create); DuckDB handles
        // that and returns an owned, empty error data handle.
        let raw = unsafe { libduckdb_sys::duckdb_appender_error_data(self.appender) };
        // SAFETY: raw is an owned duckdb_error_data (possibly null).
        unsafe { ErrorData::from_raw(raw) }
    }

    /// Returns the message from the most recent failed operation, if any.
    ///
    /// Always available; with `duckdb-1-5` prefer `error_data`, which also
    /// carries the error category.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        if self.appender.is_null() {
            return None;
        }
        // SAFETY: self.appender is non-null; DuckDB returns null when there is
        // no error, and otherwise a string it owns until the appender is
        // destroyed — so it is copied out here rather than borrowed.
        let ptr = unsafe { duckdb_appender_error(self.appender) };
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
        self.appender
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
                    // SAFETY: self.appender is valid.
                    self.check(unsafe { libduckdb_sys::$c_fn(self.appender, value) })
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
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_hugeint(self.appender, raw) })
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
        // SAFETY: self.appender is valid.
        self.check(unsafe { duckdb_append_uhugeint(self.appender, raw) })
    }
}

impl Drop for Appender {
    fn drop(&mut self) {
        if !self.appender.is_null() {
            // SAFETY: self.appender is a valid handle that we own. Destroy
            // closes (and so flushes) it first; the state is intentionally
            // ignored here — `close` beforehand is how a final flush error is
            // observed, because destruction also frees the error message.
            unsafe { duckdb_appender_destroy(&raw mut self.appender) };
        }
    }
}

crate::debug_repr::impl_handle_debug!(Appender.appender);
