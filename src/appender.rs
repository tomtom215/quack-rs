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
//! **A chunk at a time.** Build a [`DataChunk`][crate::data_chunk::DataChunk] and hand it over with
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

mod chunk;
mod construct;
mod lifecycle;
mod rows;
mod scalars;

use std::cell::Cell;
use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_appender, duckdb_appender_destroy, duckdb_appender_error, duckdb_state, DuckDBSuccess,
};

#[cfg(feature = "duckdb-1-5")]
use crate::error_data::ErrorData;

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
