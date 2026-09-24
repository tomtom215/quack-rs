// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Inspecting and executing a [`PreparedStatement`]; `bind.rs` binds its parameters.

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_bind_parameter_index, duckdb_clear_bindings, duckdb_destroy_prepare,
    duckdb_destroy_result, duckdb_execute_prepared, duckdb_nparams, duckdb_parameter_name,
    duckdb_prepared_statement, duckdb_result, duckdb_result_error, idx_t, DuckDBSuccess,
};

use super::cstr::c_str_to_owned;
use super::{no_error_message, PreparedStatement, QueryResult};
use crate::error::ExtensionError;

impl PreparedStatement {
    /// Number of `?` / `$name` parameters in the statement.
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        usize::try_from(unsafe { duckdb_nparams(self.statement) }).unwrap_or(0)
    }

    /// Name of the parameter at 1-based `index`, or `None` when `index` is out
    /// of range.
    ///
    /// Every parameter has a name: a named one (`$foo`) returns it with its
    /// case preserved (while [`parameter_index`][Self::parameter_index]
    /// matches case-insensitively), and a positional one (`?`) returns its
    /// 1-based position as text — `"1"`, `"2"`, ….
    #[must_use]
    pub fn parameter_name(&self, index: usize) -> Option<String> {
        if index == 0 || index > self.parameter_count() {
            return None;
        }
        // SAFETY: `index` was bounds-checked; DuckDB owns the returned string.
        let ptr = unsafe { duckdb_parameter_name(self.statement, index as idx_t) };
        // SAFETY: `ptr` is null or NUL-terminated.
        let name = unsafe { c_str_to_owned(ptr) };
        // DuckDB allocates the name; free it once copied.
        if !ptr.is_null() {
            // SAFETY: `ptr` came from DuckDB's allocator.
            unsafe { libduckdb_sys::duckdb_free(ptr.cast_mut().cast()) };
        }
        name
    }

    /// 1-based index of the named parameter, or `None` if there is no such
    /// parameter.
    #[must_use]
    pub fn parameter_index(&self, name: &str) -> Option<usize> {
        let c_name = CString::new(name).ok()?;
        let mut index: idx_t = 0;
        // SAFETY: `self.statement` is valid; `c_name` outlives the call.
        let state =
            unsafe { duckdb_bind_parameter_index(self.statement, &raw mut index, c_name.as_ptr()) };
        (state == DuckDBSuccess).then(|| usize::try_from(index).unwrap_or(0))
    }

    /// Executes the statement, asking `DuckDB` to stream the result instead of
    /// materialising it.
    ///
    /// A materialised result holds every row in memory before the first one is
    /// readable; a streaming result produces chunks on demand, which is what an
    /// extension scanning a large table wants.
    /// [`QueryResult::next_chunk`] drives both — `duckdb_fetch_chunk` is the
    /// documented way to read either. The difference at the call site is
    /// memory, and *when errors arrive*: a runtime error part-way through the
    /// query surfaces from `next_chunk`, after the rows before it.
    ///
    /// `DuckDB` may still materialise (see
    /// [`QueryResult::is_streaming`]), and a streaming result must be consumed
    /// before another statement runs on the same connection: running one
    /// invalidates the stream, and the next `next_chunk` returns an error.
    ///
    /// Requires `duckdb-1-5`: `duckdb_execute_prepared_streaming` sits in the
    /// unstable region of the C API struct.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] carrying `DuckDB`'s message if execution
    /// fails.
    #[cfg(feature = "duckdb-1-5")]
    pub fn execute_streaming(&self) -> Result<QueryResult, ExtensionError> {
        // SAFETY: `duckdb_result` is a `#[repr(C)]` struct of three `idx_t`
        // integers and three raw pointers; all-zero bits are a valid value of each
        // (null pointers included). DuckDB overwrites it as an out-parameter.
        let mut result: duckdb_result = unsafe { std::mem::zeroed() };
        // SAFETY: `self.statement` is valid for this value's lifetime;
        // `result` is a fresh out-parameter DuckDB fills in.
        let state = unsafe {
            libduckdb_sys::duckdb_execute_prepared_streaming(self.statement, &raw mut result)
        };
        if state == DuckDBSuccess {
            return Ok(QueryResult::new(result));
        }
        // SAFETY: on an ordinary failure DuckDB populates the error slot and
        // the result must be destroyed; on its `catch (...)` path `result` is
        // left zeroed, which both calls handle.
        let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
            .unwrap_or_else(|| no_error_message("duckdb_execute_prepared_streaming"));
        // SAFETY: destroyed exactly once, here, on the error path.
        unsafe { duckdb_destroy_result(&raw mut result) };
        Err(ExtensionError::new(message))
    }

    /// Clears every binding, so the statement can be reused with fresh values.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` rejects the request.
    pub fn clear_bindings(&self) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        if unsafe { duckdb_clear_bindings(self.statement) } == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new("duckdb_clear_bindings failed"))
        }
    }

    /// Executes the statement with its current bindings.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] carrying `DuckDB`'s message if execution
    /// fails.
    pub fn execute(&self) -> Result<QueryResult, ExtensionError> {
        // SAFETY: `duckdb_result` is a plain C struct with no invalid bit patterns;
        // DuckDB overwrites it entirely before it is read.
        let mut result: duckdb_result = unsafe { std::mem::zeroed() };
        // SAFETY: `self.statement` is valid for this value's lifetime.
        let state = unsafe { duckdb_execute_prepared(self.statement, &raw mut result) };
        if state == DuckDBSuccess {
            return Ok(QueryResult::new(result));
        }
        // SAFETY: on an ordinary failure DuckDB populated `result`; on its
        // `catch (...)` path it left the zeroed `result` untouched, and
        // `duckdb_result_error` returns null for that.
        let message = unsafe { c_str_to_owned(duckdb_result_error(&raw mut result)) }
            .unwrap_or_else(|| no_error_message("duckdb_execute_prepared"));
        // SAFETY: destroyed exactly once, here, on the error path.
        unsafe { duckdb_destroy_result(&raw mut result) };
        Err(ExtensionError::new(message))
    }

    /// Returns the raw `duckdb_prepared_statement`.
    ///
    /// Do not destroy it — this value still owns it.
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_prepared_statement {
        self.statement
    }
}

impl std::fmt::Debug for PreparedStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedStatement")
            .field("statement", &self.statement)
            .finish()
    }
}

impl Drop for PreparedStatement {
    fn drop(&mut self) {
        // SAFETY: `self.statement` was owned by this value and is destroyed once.
        unsafe { duckdb_destroy_prepare(&raw mut self.statement) };
    }
}
