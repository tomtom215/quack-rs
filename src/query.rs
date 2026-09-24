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
//! while let Some(chunk) = result.next_chunk()? {
//!     let reader = unsafe { chunk.reader(0) };
//!     for row in 0..chunk.size() {
//!         assert_eq!(unsafe { reader.read_i32(row) }, 42);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod bind;
mod chunk;
mod connection;
mod cstr;
mod prepared;
mod result;

use libduckdb_sys::{
    duckdb_connection, duckdb_destroy_prepare, duckdb_destroy_result, duckdb_prepare,
    duckdb_prepare_error, duckdb_prepared_statement, duckdb_query, duckdb_result,
    duckdb_result_error, DuckDBSuccess,
};

use self::cstr::{c_str_to_owned, to_c_sql};
use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;

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

/// A `duckdb_result` — materialised, or streaming when it came from
/// `PreparedStatement::execute_streaming` (feature `duckdb-1-5`) — destroyed
/// on drop.
///
/// Iterate the rows with [`next_chunk`][Self::next_chunk] until it returns
/// `Ok(None)`; an `Err` means the rows stopped early.
pub struct QueryResult {
    result: duckdb_result,
    fetch: FetchState,
}

/// Whether [`QueryResult::next_chunk`] may still call `duckdb_fetch_chunk`.
#[derive(Debug)]
enum FetchState {
    Open,
    /// `duckdb_fetch_chunk` returned null with no error recorded.
    Ended,
    /// It returned null and `DuckDB` recorded this error on the result.
    Failed(String),
}

impl QueryResult {
    const fn new(result: duckdb_result) -> Self {
        Self {
            result,
            fetch: FetchState::Open,
        }
    }
}

/// Runs `sql` on `con` and returns the materialised result.
///
/// # Several statements in one string
///
/// `sql` may hold several `;`-separated statements, and `DuckDB` runs **every**
/// one of them, in order. The result returned is the first statement that
/// produces rows (a `SELECT`, …), or — when none does — the last statement's.
/// Results of later row-producing statements are discarded (`DuckDB` chains
/// them where the C API cannot reach them). The first statement that fails
/// fails the call, but the statements before it have already run and, outside
/// an explicit transaction, committed. A string with no statement at all
/// (`""`, `";"`) succeeds with an empty result.
///
/// # Errors
///
/// Returns [`ExtensionError`] carrying `DuckDB`'s own message if a statement
/// fails, or if `sql` contains an interior NUL byte.
///
/// # Safety
///
/// `con` must be a valid, open `duckdb_connection`.
pub unsafe fn query(con: duckdb_connection, sql: &str) -> Result<QueryResult, ExtensionError> {
    let c_sql = to_c_sql(sql)?;
    // SAFETY: `duckdb_result` is a `#[repr(C)]` struct of three `idx_t`
    // integers and three raw pointers; all-zero bits are a valid value of each
    // (null pointers included). DuckDB overwrites it as an out-parameter.
    let mut result: duckdb_result = unsafe { std::mem::zeroed() };
    // SAFETY: `con` is valid per the caller's contract; `c_sql` outlives the call.
    let state = unsafe { duckdb_query(con, c_sql.as_ptr(), &raw mut result) };
    if state == DuckDBSuccess {
        return Ok(QueryResult::new(result));
    }
    // SAFETY: on an ordinary failure DuckDB populated `result`, so the error
    // message is readable and the result must be destroyed. On its
    // `catch (...)` path it returns without touching `result`, which is still
    // zeroed: `duckdb_result_error` then returns null and
    // `duckdb_destroy_result` is a no-op on a zeroed result.
    let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
        .unwrap_or_else(|| no_error_message("duckdb_query"));
    // SAFETY: `result` is destroyed exactly once, here, on the error path.
    unsafe { duckdb_destroy_result(&raw mut result) };
    Err(ExtensionError::new(message))
}

/// Runs `sql` on `con` for its side effects and returns the number of rows
/// changed.
///
/// With several statements in `sql`, every one runs, but the count is that
/// of the statement whose result [`query`] returns — the first that produces
/// rows, or else the last: `"SELECT 1; INSERT …"` reports `0`.
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

/// The message for a failure `DuckDB` reported without one.
///
/// `duckdb_query`, `duckdb_execute_prepared` and
/// `duckdb_execute_prepared_streaming` return `DuckDBError` from a
/// `catch (...)` block without filling in the result, so there is no message
/// to read (`duckdb-c.cpp`, `prepared-c.cpp`).
fn no_error_message(api_func: &str) -> String {
    format!(
        "{api_func} reported failure without an error message: DuckDB caught an exception \
         that is not a std::exception (its `catch (...)` path), which records nothing"
    )
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

/// Prepares `sql` on `con`.
///
/// `sql` must hold exactly one statement; unlike [`query`], a string with
/// several is an error.
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
mod live_tests;
