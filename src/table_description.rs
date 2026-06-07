// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Table description metadata (`DuckDB` 1.5.0+).
//!
//! Allows querying table structure (column count, names, and types) at runtime
//! from within an extension. Useful for replacement scans, table functions,
//! and copy functions that need to inspect existing tables.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::table_description::TableDescription;
//!
//! // From within a function callback with a valid connection:
//! // let desc = unsafe { TableDescription::create(con, "main", "my_table")? };
//! // let col_count = desc.column_count();
//! ```

use std::ffi::{CStr, CString};

use libduckdb_sys::{
    duckdb_connection, duckdb_table_description, duckdb_table_description_create,
    duckdb_table_description_destroy, duckdb_table_description_error,
    duckdb_table_description_get_column_count, duckdb_table_description_get_column_name,
    duckdb_table_description_get_column_type, idx_t,
};

use crate::error::ExtensionError;
use crate::types::LogicalType;

/// RAII wrapper for a `duckdb_table_description`.
///
/// Provides metadata about a table's columns. Automatically destroyed on drop.
pub struct TableDescription {
    desc: duckdb_table_description,
}

impl TableDescription {
    /// Creates a table description for the given schema and table.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the table does not exist or cannot be described.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn create(
        con: duckdb_connection,
        schema: &str,
        table: &str,
    ) -> Result<Self, ExtensionError> {
        let c_schema = CString::new(schema)
            .map_err(|_| ExtensionError::new("schema name contains null byte"))?;
        let c_table = CString::new(table)
            .map_err(|_| ExtensionError::new("table name contains null byte"))?;

        let mut desc: duckdb_table_description = core::ptr::null_mut();
        // SAFETY: con is valid per caller's contract.
        let rc = unsafe {
            duckdb_table_description_create(con, c_schema.as_ptr(), c_table.as_ptr(), &raw mut desc)
        };

        if rc != libduckdb_sys::DuckDBSuccess || desc.is_null() {
            // Try to get the error message.
            if !desc.is_null() {
                // SAFETY: desc is non-null and was produced by
                // duckdb_table_description_create above.
                let err_ptr = unsafe { duckdb_table_description_error(desc) };
                if !err_ptr.is_null() {
                    // SAFETY: err_ptr is a valid null-terminated string owned by
                    // the table description; it stays valid until we destroy it.
                    let msg = unsafe { CStr::from_ptr(err_ptr) }
                        .to_str()
                        .unwrap_or("unknown error");
                    let err = ExtensionError::new(format!(
                        "failed to describe table '{schema}.{table}': {msg}"
                    ));
                    // SAFETY: desc is a non-null handle we own and have not yet
                    // returned; destroying it also frees the error string.
                    unsafe { duckdb_table_description_destroy(&raw mut desc) };
                    return Err(err);
                }
                // SAFETY: desc is a non-null handle we own; destroy it before
                // returning so it does not leak.
                unsafe { duckdb_table_description_destroy(&raw mut desc) };
            }
            return Err(ExtensionError::new(format!(
                "failed to describe table '{schema}.{table}'"
            )));
        }

        Ok(Self { desc })
    }

    /// Returns the number of columns in the table.
    #[must_use]
    pub fn column_count(&self) -> idx_t {
        // SAFETY: self.desc is valid.
        unsafe { duckdb_table_description_get_column_count(self.desc) }
    }

    /// Returns the name of the column at the given index.
    ///
    /// Returns `None` if the index is out of bounds or the name is not valid UTF-8.
    #[must_use]
    pub fn column_name(&self, index: idx_t) -> Option<String> {
        // SAFETY: self.desc is valid. `DuckDB` returns a newly allocated string.
        let ptr = unsafe { duckdb_table_description_get_column_name(self.desc, index) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid null-terminated string allocated by `DuckDB`.
        let result = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .ok()
            .map(String::from);
        // Free the string allocated by `DuckDB`.
        unsafe {
            libduckdb_sys::duckdb_free(ptr.cast::<core::ffi::c_void>());
        }
        result
    }

    /// Returns the logical type of the column at the given index.
    ///
    /// Returns `None` if the index is out of bounds. The returned [`LogicalType`]
    /// is RAII-managed and will be destroyed automatically on drop.
    #[must_use]
    pub fn column_type(&self, index: idx_t) -> Option<LogicalType> {
        // SAFETY: self.desc is valid.
        let lt = unsafe { duckdb_table_description_get_column_type(self.desc, index) };
        if lt.is_null() {
            None
        } else {
            // SAFETY: lt is a freshly created handle from duckdb_table_description_get_column_type.
            Some(unsafe { LogicalType::from_raw(lt) })
        }
    }

    /// Returns the raw `duckdb_table_description` handle without consuming the
    /// wrapper. The wrapper retains ownership and destroys it on drop.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_table_description {
        self.desc
    }
}

impl Drop for TableDescription {
    fn drop(&mut self) {
        if !self.desc.is_null() {
            // SAFETY: self.desc is a non-null handle obtained from
            // duckdb_table_description_create.
            unsafe {
                duckdb_table_description_destroy(&raw mut self.desc);
            }
        }
    }
}

#[cfg(all(test, feature = "_duckdb-testing"))]
mod tests {
    use super::*;

    /// Opens a raw `duckdb_connection` for testing.
    ///
    /// Uses `InMemoryDb::open()` to ensure the dispatch table is initialized,
    /// then opens a separate raw database + connection via `libduckdb_sys`.
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
    ///
    /// # Safety
    ///
    /// `con` and `db` must be valid handles from `open_raw_connection`.
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
    fn describe_existing_table() {
        let (db, con) = open_raw_connection();

        // Create a table to describe.
        let sql = c"CREATE TABLE test_tbl (id INTEGER, name VARCHAR, score DOUBLE)";
        // SAFETY: con is valid.
        unsafe {
            let rc = libduckdb_sys::duckdb_query(con, sql.as_ptr(), core::ptr::null_mut());
            assert_eq!(rc, libduckdb_sys::DuckDBSuccess, "CREATE TABLE failed");
        }

        // SAFETY: con is valid, table exists.
        let desc = unsafe { TableDescription::create(con, "main", "test_tbl") };
        assert!(desc.is_ok(), "describe should succeed: {:?}", desc.err());
        let desc = desc.unwrap();

        assert_eq!(desc.column_count(), 3);

        assert_eq!(desc.column_name(0), Some("id".to_string()));
        assert_eq!(desc.column_name(1), Some("name".to_string()));
        assert_eq!(desc.column_name(2), Some("score".to_string()));

        // Out-of-bounds index should return None.
        assert_eq!(desc.column_name(99), None);

        // Column types should be non-null.
        let lt0 = desc.column_type(0);
        assert!(lt0.is_some(), "column_type(0) should be Some");
        // LogicalType is RAII — automatically destroyed on drop.
        drop(lt0);

        // Out-of-bounds column type should return None.
        assert!(desc.column_type(99).is_none());

        drop(desc);
        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn describe_nonexistent_table_returns_error() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is valid, table does NOT exist.
        let result = unsafe { TableDescription::create(con, "main", "no_such_table") };
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains("no_such_table"),
            "error should mention table name, got: {err_msg}"
        );

        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn describe_schema_null_byte_rejected() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is valid.
        let result = unsafe { TableDescription::create(con, "bad\0schema", "t") };
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("null byte"));

        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }

    #[test]
    fn describe_table_null_byte_rejected() {
        let (db, con) = open_raw_connection();

        // SAFETY: con is valid.
        let result = unsafe { TableDescription::create(con, "main", "bad\0table") };
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("null byte"));

        // SAFETY: valid handles.
        unsafe { close_raw_connection(con, db) };
    }
}
