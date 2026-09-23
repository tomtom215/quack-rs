// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Client context access (`DuckDB` 1.5.0+).
//!
//! The client context provides access to the connection's catalog, configuration
//! options, file system, and connection ID from within registered function
//! callbacks (scalar, table, aggregate, etc.).
//!
//! # Obtaining a `ClientContext`
//!
//! Use [`ClientContext::from_connection`] from within an extension entry point,
//! or obtain one from a callback via the `duckdb_*_get_client_context` family
//! of C API functions.

use std::ffi::CStr;

use libduckdb_sys::{
    duckdb_client_context, duckdb_client_context_get_catalog,
    duckdb_client_context_get_config_option, duckdb_client_context_get_connection_id,
    duckdb_config_option_scope, duckdb_connection, duckdb_connection_get_client_context,
    duckdb_destroy_client_context,
};

use crate::catalog::Catalog;
use crate::error::ExtensionError;
use crate::value::Value;

/// RAII wrapper for a `duckdb_client_context`.
///
/// Provides access to the connection's catalog, configuration, and file system.
/// Automatically destroyed when dropped.
///
/// # Lifetime
///
/// The handle does **not** keep the connection alive. `DuckDB`'s
/// `CClientContextWrapper` holds a plain reference to the `ClientContext` the
/// connection owns (`src/include/duckdb/main/capi/capi_internal.hpp`), so a
/// `ClientContext` — and a [`FileSystem`][crate::file_system::FileSystem]
/// borrowed from it — must not be used after that connection is closed.
/// Every constructor is `unsafe` and states this in its `# Safety` section.
pub struct ClientContext {
    ctx: duckdb_client_context,
}

impl ClientContext {
    /// Obtain a client context from a `duckdb_connection`.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the context cannot be obtained.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`, and must stay open for
    /// as long as the returned context (or anything borrowed from it) is used
    /// — see [Lifetime](Self#lifetime).
    pub unsafe fn from_connection(con: duckdb_connection) -> Result<Self, ExtensionError> {
        let mut ctx: duckdb_client_context = core::ptr::null_mut();
        // SAFETY: con is valid per caller's contract.
        unsafe { duckdb_connection_get_client_context(con, &raw mut ctx) };
        if ctx.is_null() {
            return Err(ExtensionError::new(
                "failed to obtain client context from connection",
            ));
        }
        Ok(Self { ctx })
    }

    /// Wrap a raw `duckdb_client_context` handle.
    ///
    /// # Safety
    ///
    /// `ctx` must be a valid, non-null `duckdb_client_context` that nothing
    /// else destroys, and the connection it belongs to must stay open for as
    /// long as the returned value is used — see [Lifetime](Self#lifetime).
    pub const unsafe fn from_raw(ctx: duckdb_client_context) -> Self {
        Self { ctx }
    }

    /// Returns the raw handle.
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_client_context {
        self.ctx
    }

    /// Retrieves a database catalog by name.
    ///
    /// Returns `None` when there is no such catalog — and in two cases that are
    /// easy to mistake for one:
    ///
    /// - **`name` is empty.** `duckdb_client_context_get_catalog` rejects that
    ///   outright (`strlen(name) == 0` is an explicit early return); it is not
    ///   a way to ask for "the default". The catalog of an in-memory database
    ///   is named `memory`; a file database's is the file's stem.
    /// - **No transaction is active.** `DuckDB` checks
    ///   `transaction.HasActiveTransaction()` and returns null otherwise, so
    ///   this works inside a function callback but not on an idle
    ///   auto-commit connection.
    ///
    /// # Safety
    ///
    /// Must be called from within an active transaction context.
    pub unsafe fn catalog(&self, name: &CStr) -> Option<Catalog> {
        // SAFETY: self.ctx is valid, caller ensures active transaction.
        let catalog = unsafe { duckdb_client_context_get_catalog(self.ctx, name.as_ptr()) };
        if catalog.is_null() {
            None
        } else {
            // SAFETY: catalog is non-null and valid.
            Some(unsafe { Catalog::from_raw(catalog) })
        }
    }

    /// Retrieves a configuration option value by name, rendered as text.
    ///
    /// Returns `None` when the option does not exist **or its value is SQL
    /// `NULL`** — the two are not distinguished. Several built-in settings are
    /// `NULL` until set (`enable_profiling`, for one), and so is an extension
    /// option after `SET my_option = NULL`. `DuckDB`'s `duckdb_get_varchar`
    /// throws on a `NULL` value from inside the C API, which used to abort the
    /// process; this checks `duckdb_is_null_value` first. A value that is not
    /// valid UTF-8 is also `None`.
    ///
    /// # Do not use this to probe for a setting that may not exist
    ///
    /// `DuckDB` 1.5.5's `duckdb_client_context_get_config_option` reads the
    /// lookup's scope before checking the lookup succeeded:
    ///
    /// ```text
    /// // src/main/capi/config_options-c.cpp
    /// switch (ctx.TryGetCurrentSetting(option_name, result).GetScope()) {
    ///
    /// // src/include/duckdb/main/setting_info.hpp
    /// SettingScope GetScope() {
    ///     D_ASSERT(scope != SettingScope::INVALID);
    /// ```
    ///
    /// A missing setting yields `SettingScope::INVALID`, so `GetScope()` trips
    /// that assertion. In a release `DuckDB` — what users run — `D_ASSERT`
    /// compiles out, the function's own `default:` branch handles `INVALID`,
    /// and this returns `None` as documented. Against a `DuckDB` built **with
    /// debug assertions**, the process aborts. Verified against 1.5.5.
    ///
    /// So this is safe for a setting you registered or know exists, and unsafe
    /// as an existence check. To ask whether a setting exists, use SQL, which
    /// has no such path:
    ///
    /// ```sql
    /// SELECT count(*) FROM duckdb_settings() WHERE name = 'my_setting';
    /// ```
    pub fn config_option(&self, name: &CStr) -> Option<String> {
        let mut scope: duckdb_config_option_scope = 0;
        // SAFETY: self.ctx is valid.
        let raw = unsafe {
            duckdb_client_context_get_config_option(self.ctx, name.as_ptr(), &raw mut scope)
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `duckdb_client_context_get_config_option` returns an owned
        // `duckdb_value`; `Value` destroys it on drop.
        let value = unsafe { Value::from_raw(raw) };
        // `Value::as_str` checks `duckdb_is_null_value` before calling
        // `duckdb_get_varchar`, which would throw (and abort) on SQL NULL.
        value.as_str().ok()
    }

    /// Returns the connection ID associated with this client context.
    #[must_use]
    pub fn connection_id(&self) -> u64 {
        // SAFETY: self.ctx is valid.
        unsafe { duckdb_client_context_get_connection_id(self.ctx) }
    }
}

impl Drop for ClientContext {
    fn drop(&mut self) {
        // SAFETY: self.ctx was obtained from a valid `DuckDB` API call.
        unsafe {
            duckdb_destroy_client_context(&raw mut self.ctx);
        }
    }
}

#[cfg(all(test, feature = "_duckdb-testing"))]
mod tests {
    use super::*;

    /// Opens a raw `duckdb_connection` for testing.
    fn open_raw_connection() -> (libduckdb_sys::duckdb_database, duckdb_connection) {
        // Ensure dispatch table is populated.
        let _db = crate::testing::InMemoryDb::open().unwrap();

        let mut db: libduckdb_sys::duckdb_database = core::ptr::null_mut();
        let mut con: duckdb_connection = core::ptr::null_mut();

        // SAFETY: dispatch table is initialized, nullptr opens in-memory.
        unsafe {
            let rc = libduckdb_sys::duckdb_open(core::ptr::null(), &raw mut db);
            assert_eq!(rc, libduckdb_sys::DuckDBSuccess, "duckdb_open failed");
            let rc = libduckdb_sys::duckdb_connect(db, &raw mut con);
            assert_eq!(rc, libduckdb_sys::DuckDBSuccess, "duckdb_connect failed");
        }
        (db, con)
    }

    /// Closes a raw connection and database.
    unsafe fn close_raw_connection(
        mut con: duckdb_connection,
        mut db: libduckdb_sys::duckdb_database,
    ) {
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut con);
            libduckdb_sys::duckdb_close(&raw mut db);
        }
    }

    #[test]
    fn from_connection_succeeds() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) };
        assert!(
            ctx.is_ok(),
            "from_connection should succeed: {:?}",
            ctx.err()
        );

        drop(ctx.unwrap());
        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn connection_id_distinguishes_connections_and_is_stable() {
        let (db, con) = open_raw_connection();
        let mut con2: duckdb_connection = core::ptr::null_mut();
        // SAFETY: `db` is open.
        let rc = unsafe { libduckdb_sys::duckdb_connect(db, &raw mut con2) };
        assert_eq!(rc, libduckdb_sys::DuckDBSuccess);

        // SAFETY: both connections are open.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();
        // SAFETY: as above.
        let ctx_again = unsafe { ClientContext::from_connection(con) }.unwrap();
        // SAFETY: as above.
        let ctx2 = unsafe { ClientContext::from_connection(con2) }.unwrap();
        assert_eq!(ctx.connection_id(), ctx_again.connection_id());
        assert_ne!(ctx.connection_id(), ctx2.connection_id());

        drop((ctx, ctx_again, ctx2));
        // SAFETY: valid handles, closed once.
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut con2);
            close_raw_connection(con, db);
        }
    }

    /// Before the fix this aborted the process: `enable_profiling` is `NULL`
    /// until set, and `duckdb_get_varchar` throws on a `NULL` value from
    /// inside the C API ("Rust cannot catch foreign exceptions").
    #[test]
    fn config_option_of_a_null_setting_is_none_not_an_abort() {
        let (db, con) = open_raw_connection();
        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();
        assert_eq!(ctx.config_option(c"enable_profiling"), None);
        // A non-NULL setting still reads.
        assert!(ctx.config_option(c"threads").is_some());
        drop(ctx);
        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn config_option_returns_some_for_known_setting() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();

        // "threads" is a well-known DuckDB config option.
        let threads = ctx.config_option(c"threads");
        assert!(threads.is_some(), "'threads' config option should exist");
        // The value should be a parseable positive integer.
        let val: usize = threads.unwrap().parse().expect("threads should be numeric");
        assert!(val > 0, "threads should be > 0");

        drop(ctx);
        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn catalog_is_found_by_name_inside_a_transaction() {
        let (db, con) = open_raw_connection();

        // Start a transaction so we have an active transaction context.
        // SAFETY: con is valid.
        unsafe {
            let sql = c"BEGIN TRANSACTION";
            libduckdb_sys::duckdb_query(con, sql.as_ptr(), core::ptr::null_mut());
        }

        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();

        // An empty name is rejected outright (`strlen(name) == 0` returns
        // early); an in-memory database's catalog is called `memory`.
        // SAFETY: within an active transaction.
        assert!(unsafe { ctx.catalog(c"") }.is_none());
        // SAFETY: within an active transaction.
        let catalog = unsafe { ctx.catalog(c"memory") }.expect("the memory catalog");
        assert_eq!(catalog.type_name(), Some("duckdb"));
        drop(catalog);

        drop(ctx);
        // Rollback the transaction.
        // SAFETY: con is valid.
        unsafe {
            let sql = c"ROLLBACK";
            libduckdb_sys::duckdb_query(con, sql.as_ptr(), core::ptr::null_mut());
        }
        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }
}

crate::debug_repr::impl_handle_debug!(ClientContext.ctx);
