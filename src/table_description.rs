// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Table description metadata.
//!
//! Allows querying a table's structure — column names, whether a column has a
//! `DEFAULT`, and (with `duckdb-1-5`) the column count and types — at runtime
//! from within an extension. Useful for replacement scans, table functions, and
//! copy functions that need to inspect existing tables before deciding what to
//! do.
//!
//! # Feature flags
//!
//! Creating and naming columns needs no feature flag: `duckdb_table_description_*`
//! has been in the frozen stable prefix of the extension API (slots 292–297)
//! since v1.2.0. Two accessors are `DuckDB` 1.5.0 additions living in the
//! unstable region and are gated on `duckdb-1-5`:
//! [`column_count`][TableDescription::column_count] and
//! [`column_type`][TableDescription::column_type].
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::table_description::TableDescription;
//!
//! // From within a function callback with a valid connection:
//! // let desc = unsafe { TableDescription::create(con, "main", "my_table")? };
//! // let first = desc.column_name(0);
//! ```

use std::ffi::{CStr, CString};

use libduckdb_sys::{
    duckdb_column_has_default, duckdb_connection, duckdb_table_description,
    duckdb_table_description_create, duckdb_table_description_create_ext,
    duckdb_table_description_destroy, duckdb_table_description_error,
    duckdb_table_description_get_column_name, idx_t, DuckDBSuccess,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    duckdb_table_description_get_column_count, duckdb_table_description_get_column_type,
};

use crate::error::ExtensionError;
#[cfg(feature = "duckdb-1-5")]
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

        // SAFETY: `desc` is whatever DuckDB wrote; the helper takes ownership
        // from here, including destroying it on the error path.
        unsafe { Self::from_create_result(rc, desc, schema, table) }
    }

    /// Turns a `duckdb_table_description_create*` outcome into a `Result`.
    ///
    /// `duckdb.h` requires `duckdb_table_description_destroy` to be called on
    /// the result "even if the function returns `DuckDBError`", so the failure
    /// path must destroy the handle rather than simply dropping it — and it
    /// must read the error message first, because destroying frees it.
    ///
    /// # Safety
    ///
    /// `desc` must be the out-parameter of a `duckdb_table_description_create*`
    /// call that returned `rc`, and must not be used by the caller afterwards.
    unsafe fn from_create_result(
        rc: libduckdb_sys::duckdb_state,
        mut desc: duckdb_table_description,
        schema: &str,
        table: &str,
    ) -> Result<Self, ExtensionError> {
        if rc == DuckDBSuccess && !desc.is_null() {
            return Ok(Self { desc });
        }
        let mut message = format!("failed to describe table '{schema}.{table}'");
        if !desc.is_null() {
            // SAFETY: desc is non-null and was produced by a create call.
            let err_ptr = unsafe { duckdb_table_description_error(desc) };
            if !err_ptr.is_null() {
                // SAFETY: err_ptr is a NUL-terminated string owned by the
                // description; it stays valid until the destroy below.
                let detail = unsafe { CStr::from_ptr(err_ptr) }
                    .to_str()
                    .unwrap_or("unknown error");
                message.push_str(": ");
                message.push_str(detail);
            }
            // SAFETY: desc is a non-null handle we own and have not returned.
            unsafe { duckdb_table_description_destroy(&raw mut desc) };
        }
        Err(ExtensionError::new(message))
    }

    /// Creates a table description, fully qualified by optional `catalog` and
    /// `schema`.
    ///
    /// `None` means "the default", matching `duckdb_table_description_create_ext`.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if any name contains an interior NUL byte, or
    /// if the table does not exist or cannot be described.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn with_catalog(
        con: duckdb_connection,
        catalog: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Self, ExtensionError> {
        fn to_c(label: &str, value: Option<&str>) -> Result<Option<CString>, ExtensionError> {
            value
                .map(|v| {
                    CString::new(v)
                        .map_err(|_| ExtensionError::new(format!("{label} contains null byte")))
                })
                .transpose()
        }
        let c_catalog = to_c("catalog name", catalog)?;
        let c_schema = to_c("schema name", schema)?;
        let c_table = CString::new(table)
            .map_err(|_| ExtensionError::new("table name contains null byte"))?;
        let ptr = |c: &Option<CString>| c.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());

        let mut desc: duckdb_table_description = core::ptr::null_mut();
        // SAFETY: con is valid per caller's contract; each pointer is either
        // null (meaning "default") or a NUL-terminated string alive for the call.
        let rc = unsafe {
            duckdb_table_description_create_ext(
                con,
                ptr(&c_catalog),
                ptr(&c_schema),
                c_table.as_ptr(),
                &raw mut desc,
            )
        };
        // SAFETY: `desc` is whatever DuckDB wrote; the helper takes it from here,
        // including destroying it on the error path as duckdb.h requires.
        unsafe { Self::from_create_result(rc, desc, schema.unwrap_or("<default>"), table) }
    }

    /// Returns the number of columns in the table.
    #[cfg(feature = "duckdb-1-5")]
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
    #[cfg(feature = "duckdb-1-5")]
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

    /// Returns whether the column at `index` has a `DEFAULT` value.
    ///
    /// Returns `None` if the index is out of bounds. This is what makes
    /// [`Appender::append_default`][crate::appender::Appender::append_default]
    /// safe to reach for: appending a default to a column that has none is an
    /// error, and this is the only way to find out first.
    #[must_use]
    pub fn column_has_default(&self, index: idx_t) -> Option<bool> {
        let mut out = false;
        // SAFETY: self.desc is valid; DuckDB bounds-checks `index` and reports
        // failure through the return state rather than writing `out`.
        let state = unsafe { duckdb_column_has_default(self.desc, index, &raw mut out) };
        if state == DuckDBSuccess {
            Some(out)
        } else {
            None
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

crate::debug_repr::impl_handle_debug!(TableDescription.desc);
