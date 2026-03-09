// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrappers around `DuckDB` callback info handles.
//!
//! These types provide safe, chainable methods for the most common operations
//! performed inside bind, init, and scan callbacks.

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_bind_add_result_column, duckdb_bind_info, duckdb_bind_set_cardinality,
    duckdb_bind_set_error, duckdb_function_info, duckdb_function_set_error, duckdb_init_info,
    duckdb_init_set_error, idx_t,
};

use crate::types::{LogicalType, TypeId};

/// Helper wrapper around `duckdb_bind_info` for use inside bind callbacks.
///
/// Provides ergonomic methods for the most common bind operations.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::table::BindInfo;
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::duckdb_bind_info;
///
/// unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
///     unsafe {
///         BindInfo::new(info)
///             .add_result_column("id",   TypeId::BigInt)
///             .add_result_column("name", TypeId::Varchar)
///             .set_cardinality(100, true);
///     }
/// }
/// ```
pub struct BindInfo {
    info: duckdb_bind_info,
}

impl BindInfo {
    /// Wraps a raw `duckdb_bind_info`.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_bind_info` provided by `DuckDB` in a bind callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_bind_info) -> Self {
        Self { info }
    }

    /// Declares an output column with the given name and type.
    ///
    /// Call this once per output column in the order they will appear in the result.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn add_result_column(&self, name: &str, type_id: TypeId) -> &Self {
        let c_name = CString::new(name).expect("column name must not contain null bytes");
        let lt = LogicalType::new(type_id);
        // SAFETY: self.info is valid per constructor's contract.
        unsafe {
            duckdb_bind_add_result_column(self.info, c_name.as_ptr(), lt.as_raw());
        }
        self
    }

    /// Adds an output column with a pre-built `LogicalType`.
    ///
    /// Use this when the column type is a complex type (LIST, STRUCT, MAP) built
    /// via `LogicalType::list`, `LogicalType::struct_type`, or `LogicalType::map`.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn add_result_column_with_type(&self, name: &str, logical_type: &LogicalType) -> &Self {
        let c_name = CString::new(name).expect("column name must not contain null bytes");
        // SAFETY: self.info is valid; logical_type.as_raw() is valid.
        unsafe {
            duckdb_bind_add_result_column(self.info, c_name.as_ptr(), logical_type.as_raw());
        }
        self
    }

    /// Sets a cardinality hint for the query optimizer.
    ///
    /// `is_exact` — if `true`, `DuckDB` treats this as the exact row count;
    /// if `false`, it is treated as an estimate.
    pub fn set_cardinality(&self, rows: u64, is_exact: bool) -> &Self {
        // SAFETY: self.info is valid.
        unsafe {
            duckdb_bind_set_cardinality(self.info, rows as idx_t, is_exact);
        }
        self
    }

    /// Reports an error from the bind callback.
    ///
    /// After calling this, `DuckDB` will abort query parsing and report the error.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    pub fn set_error(&self, message: &str) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid.
        unsafe {
            duckdb_bind_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the number of positional parameters passed to this function call.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        // SAFETY: self.info is valid.
        usize::try_from(unsafe { libduckdb_sys::duckdb_bind_get_parameter_count(self.info) })
            .unwrap_or(0)
    }

    /// Returns the raw `duckdb_bind_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_bind_info {
        self.info
    }
}

/// Helper wrapper around `duckdb_init_info` for use inside init callbacks.
///
/// Provides ergonomic methods for the most common init operations.
pub struct InitInfo {
    info: duckdb_init_info,
}

impl InitInfo {
    /// Wraps a raw `duckdb_init_info`.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_init_info` provided by `DuckDB`.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_init_info) -> Self {
        Self { info }
    }

    /// Returns the number of projected (requested) columns.
    ///
    /// Only valid when projection pushdown is enabled for the table function.
    #[must_use]
    pub fn projected_column_count(&self) -> usize {
        // SAFETY: self.info is valid.
        usize::try_from(unsafe { libduckdb_sys::duckdb_init_get_column_count(self.info) })
            .unwrap_or(0)
    }

    /// Returns the output column index at the given projection position.
    ///
    /// Only valid when projection pushdown is enabled.
    #[must_use]
    pub fn projected_column_index(&self, projection_idx: usize) -> usize {
        // SAFETY: self.info is valid.
        usize::try_from(unsafe {
            libduckdb_sys::duckdb_init_get_column_index(self.info, projection_idx as idx_t)
        })
        .unwrap_or(0)
    }

    /// Sets the maximum number of threads for parallel scanning.
    ///
    /// Only effective when `local_init` is also set on the table function.
    pub fn set_max_threads(&self, n: u64) {
        // SAFETY: self.info is valid.
        unsafe { libduckdb_sys::duckdb_init_set_max_threads(self.info, n as idx_t) };
    }

    /// Reports an error from the init callback.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    pub fn set_error(&self, message: &str) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid.
        unsafe { duckdb_init_set_error(self.info, c_msg.as_ptr()) };
    }

    /// Returns the raw `duckdb_init_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_init_info {
        self.info
    }
}

/// Helper wrapper around `duckdb_function_info` for use inside scan callbacks.
pub struct FunctionInfo {
    info: duckdb_function_info,
}

impl FunctionInfo {
    /// Wraps a raw `duckdb_function_info`.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_function_info` provided by `DuckDB` in a scan callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_function_info) -> Self {
        Self { info }
    }

    /// Reports an error from the scan callback.
    ///
    /// `DuckDB` will abort the query and propagate this as a SQL error.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    pub fn set_error(&self, message: &str) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid.
        unsafe { duckdb_function_set_error(self.info, c_msg.as_ptr()) };
    }

    /// Returns the raw `duckdb_function_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_function_info {
        self.info
    }
}
