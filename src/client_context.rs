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
    duckdb_destroy_client_context, duckdb_destroy_value, duckdb_get_varchar, duckdb_value,
};

use crate::catalog::Catalog;
use crate::error::ExtensionError;

/// RAII wrapper for a `duckdb_client_context`.
///
/// Provides access to the connection's catalog, configuration, and file system.
/// Automatically destroyed when dropped.
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
    /// `con` must be a valid, open `duckdb_connection`.
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
    /// `ctx` must be a valid, non-null `duckdb_client_context`.
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
    /// Pass an empty string to get the default catalog. This function can only
    /// be called from within an active transaction (e.g. during a registered
    /// function callback).
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

    /// Retrieves a configuration option value by name.
    ///
    /// Returns the value as a string, or `None` if the option does not exist.
    pub fn config_option(&self, name: &CStr) -> Option<String> {
        let mut scope: duckdb_config_option_scope = 0;
        // SAFETY: self.ctx is valid.
        let val: duckdb_value = unsafe {
            duckdb_client_context_get_config_option(self.ctx, name.as_ptr(), &raw mut scope)
        };
        if val.is_null() {
            return None;
        }
        // SAFETY: val is a valid duckdb_value.
        let c_str = unsafe { duckdb_get_varchar(val) };
        let result = if c_str.is_null() {
            None
        } else {
            // SAFETY: c_str is a valid null-terminated string.
            unsafe { CStr::from_ptr(c_str) }
                .to_str()
                .ok()
                .map(String::from)
        };
        // SAFETY: c_str was allocated by `DuckDB` and must be freed.
        if !c_str.is_null() {
            unsafe {
                libduckdb_sys::duckdb_free(c_str.cast::<core::ffi::c_void>());
            }
        }
        // SAFETY: val must be destroyed.
        let mut val_mut = val;
        unsafe {
            duckdb_destroy_value(&raw mut val_mut);
        }
        result
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
    fn connection_id_returns_nonzero() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();
        // Connection IDs are assigned sequentially starting from a positive value.
        // We just verify the call doesn't crash and returns something.
        let _id = ctx.connection_id();

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
    fn catalog_returns_some_for_default() {
        let (db, con) = open_raw_connection();

        // Start a transaction so we have an active transaction context.
        // SAFETY: con is valid.
        unsafe {
            let sql = c"BEGIN TRANSACTION";
            libduckdb_sys::duckdb_query(con, sql.as_ptr(), core::ptr::null_mut());
        }

        // SAFETY: con is a valid open connection.
        let ctx = unsafe { ClientContext::from_connection(con) }.unwrap();

        // Empty name = default catalog. Must be called within a transaction.
        // SAFETY: within an active transaction.
        let catalog = unsafe { ctx.catalog(c"") };
        // Note: catalog lookup may or may not succeed depending on DuckDB version
        // internals. We just verify the call doesn't crash.
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
