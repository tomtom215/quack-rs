// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrapper around `duckdb_function_info` for aggregate function callbacks.

use std::ffi::CString;
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_aggregate_function_get_extra_info, duckdb_aggregate_function_set_error,
    duckdb_function_info,
};

/// Converts a `&str` to `CString` without panicking.
#[mutants::skip] // private FFI helper — tested in replacement_scan::tests
fn str_to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let pos = s.bytes().position(|b| b == 0).unwrap_or(s.len());
        CString::new(&s.as_bytes()[..pos]).unwrap_or_default()
    })
}

/// Ergonomic wrapper around the `duckdb_function_info` handle provided to
/// aggregate function callbacks (update, combine, finalize, etc.).
///
/// Provides access to extra info and error reporting.
pub struct AggregateFunctionInfo {
    info: duckdb_function_info,
}

impl AggregateFunctionInfo {
    /// Wraps a raw `duckdb_function_info` provided by `DuckDB` inside an
    /// aggregate function callback.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_function_info` passed by `DuckDB` to an
    /// aggregate function callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_function_info) -> Self {
        Self { info }
    }

    /// Retrieves the extra-info pointer previously set via
    /// [`AggregateFunctionBuilder::extra_info`][crate::aggregate::AggregateFunctionBuilder::extra_info].
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the aggregate function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_aggregate_function_get_extra_info(self.info) }
    }

    /// Reports an error from an aggregate function callback, causing `DuckDB`
    /// to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_aggregate_function_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the raw `duckdb_function_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_function_info {
        self.info
    }
}

crate::debug_repr::impl_handle_debug!(AggregateFunctionInfo.info);
