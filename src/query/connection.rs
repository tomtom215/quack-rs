// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`OwnedConnection`] and its cross-thread [`InterruptHandle`].

use libduckdb_sys::{
    duckdb_connect, duckdb_connection, duckdb_database, duckdb_disconnect, DuckDBSuccess,
};

use super::{
    execute, interrupt, prepare, query, query_progress, InterruptHandle, OwnedConnection,
    PreparedStatement, QueryProgress, QueryResult,
};
use crate::error::ExtensionError;

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
