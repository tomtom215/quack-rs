// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrapper around `duckdb_function_info` for aggregate function callbacks.

use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_aggregate_function_get_extra_info, duckdb_aggregate_function_set_error,
    duckdb_function_info,
};

/// What [`AggregateFunctionInfo::set_error`] reports when given an empty
/// message: `DuckDB` would otherwise show only an error-type prefix such as
/// `Invalid Input Error: `.
pub const EMPTY_ERROR_PLACEHOLDER: &str = "aggregate function reported an error without a message";

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
    /// An interior NUL byte in `message` is replaced with `?`
    /// (see [`message_to_c_string`][crate::callback::message_to_c_string]),
    /// and an empty message by [`EMPTY_ERROR_PLACEHOLDER`], since `DuckDB`
    /// would otherwise report only an error-type prefix.
    ///
    /// Called from `finalize`, the query fails but `DuckDB` 1.5.5 does not
    /// destroy every aggregate state it created, so whatever those states own
    /// (an [`FfiState`][crate::aggregate::FfiState] box, for one) leaks. See
    /// Known Limitations in the book.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = crate::table::cstr::error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
