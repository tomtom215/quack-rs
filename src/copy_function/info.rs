// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Callback info wrappers for copy function callbacks.
//!
//! Each copy function phase (bind, global init, sink, finalize) receives an
//! opaque info handle from `DuckDB`. These wrappers provide safe, ergonomic
//! access to the underlying C API functions.

use std::ffi::{CStr, CString};
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_copy_function_bind_get_client_context, duckdb_copy_function_bind_get_column_count,
    duckdb_copy_function_bind_get_column_type, duckdb_copy_function_bind_get_extra_info,
    duckdb_copy_function_bind_info, duckdb_copy_function_bind_set_bind_data,
    duckdb_copy_function_bind_set_error, duckdb_copy_function_finalize_get_bind_data,
    duckdb_copy_function_finalize_get_client_context, duckdb_copy_function_finalize_get_extra_info,
    duckdb_copy_function_finalize_get_global_state, duckdb_copy_function_finalize_info,
    duckdb_copy_function_finalize_set_error, duckdb_copy_function_global_init_get_bind_data,
    duckdb_copy_function_global_init_get_client_context,
    duckdb_copy_function_global_init_get_extra_info,
    duckdb_copy_function_global_init_get_file_path, duckdb_copy_function_global_init_info,
    duckdb_copy_function_global_init_set_error, duckdb_copy_function_global_init_set_global_state,
    duckdb_copy_function_sink_get_bind_data, duckdb_copy_function_sink_get_client_context,
    duckdb_copy_function_sink_get_extra_info, duckdb_copy_function_sink_get_global_state,
    duckdb_copy_function_sink_info, duckdb_copy_function_sink_set_error, duckdb_delete_callback_t,
};

use crate::types::LogicalType;

/// Converts a `&str` to `CString` without panicking.
#[mutants::skip] // private FFI helper — tested in replacement_scan::tests
fn str_to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let pos = s.bytes().position(|b| b == 0).unwrap_or(s.len());
        CString::new(&s.as_bytes()[..pos]).unwrap_or_default()
    })
}

// ── CopyBindInfo ─────────────────────────────────────────────────────────────

/// Wrapper around the `duckdb_copy_function_bind_info` handle provided to a
/// copy function bind callback.
pub struct CopyBindInfo {
    info: duckdb_copy_function_bind_info,
}

impl CopyBindInfo {
    /// Wraps a raw `duckdb_copy_function_bind_info` handle.
    ///
    /// # Safety
    ///
    /// `info` must be a valid handle passed by `DuckDB` to a copy function bind
    /// callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_copy_function_bind_info) -> Self {
        Self { info }
    }

    /// Returns the number of columns in the output.
    #[mutants::skip]
    #[must_use]
    pub fn column_count(&self) -> u64 {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_bind_get_column_count(self.info) }
    }

    /// Returns the logical type of the column at `index`.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`column_count`][Self::column_count].
    #[must_use]
    pub unsafe fn column_type(&self, index: u64) -> LogicalType {
        // SAFETY: self.info is valid; caller guarantees index is in range.
        let raw = unsafe { duckdb_copy_function_bind_get_column_type(self.info, index) };
        // SAFETY: DuckDB returns a valid logical type handle.
        unsafe { LogicalType::from_raw(raw) }
    }

    /// Retrieves the extra-info pointer previously set on the copy function.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the copy function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_bind_get_extra_info(self.info) }
    }

    /// Sets the bind data pointer and its destructor.
    ///
    /// # Safety
    ///
    /// `data` must remain valid until `DuckDB` calls `destroy`, or for the
    /// lifetime of the query if `destroy` is `None`.
    pub unsafe fn set_bind_data(&self, data: *mut c_void, destroy: duckdb_delete_callback_t) {
        // SAFETY: self.info is valid; data validity is the caller's responsibility.
        unsafe {
            duckdb_copy_function_bind_set_bind_data(self.info, data, destroy);
        }
    }

    /// Reports a fatal error, causing `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_copy_function_bind_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the client context for this callback.
    ///
    /// The returned [`ClientContext`][crate::client_context::ClientContext] provides
    /// access to the connection's catalog, configuration, and connection ID.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    pub unsafe fn get_client_context(&self) -> crate::client_context::ClientContext {
        // SAFETY: self.info is a valid copy-bind-info handle per this fn's contract.
        let ctx = unsafe { duckdb_copy_function_bind_get_client_context(self.info) };
        // SAFETY: `ctx` is a client-context handle returned by DuckDB.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the underlying raw handle.
    #[mutants::skip]
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_copy_function_bind_info {
        self.info
    }
}

// ── CopyGlobalInitInfo ───────────────────────────────────────────────────────

/// Wrapper around the `duckdb_copy_function_global_init_info` handle provided
/// to a copy function global init callback.
pub struct CopyGlobalInitInfo {
    info: duckdb_copy_function_global_init_info,
}

impl CopyGlobalInitInfo {
    /// Wraps a raw `duckdb_copy_function_global_init_info` handle.
    ///
    /// # Safety
    ///
    /// `info` must be a valid handle passed by `DuckDB` to a copy function
    /// global init callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_copy_function_global_init_info) -> Self {
        Self { info }
    }

    /// Retrieves the bind data pointer set during the bind phase.
    ///
    /// # Safety
    ///
    /// The returned pointer must be cast back to the original type.
    #[must_use]
    pub unsafe fn get_bind_data(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_global_init_get_bind_data(self.info) }
    }

    /// Retrieves the extra-info pointer previously set on the copy function.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the copy function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_global_init_get_extra_info(self.info) }
    }

    /// Returns the file path for the copy operation.
    ///
    /// # Safety
    ///
    /// `self.info` must be a valid handle from an active callback invocation.
    #[must_use]
    pub unsafe fn get_file_path(&self) -> String {
        // SAFETY: self.info is valid per constructor contract.
        let c_str = unsafe { duckdb_copy_function_global_init_get_file_path(self.info) };
        let result = if c_str.is_null() {
            String::new()
        } else {
            // SAFETY: c_str is a valid null-terminated string from DuckDB.
            unsafe { CStr::from_ptr(c_str) }
                .to_str()
                .unwrap_or("")
                .to_owned()
        };
        // SAFETY: c_str was allocated by DuckDB and must be freed.
        if !c_str.is_null() {
            unsafe {
                libduckdb_sys::duckdb_free(c_str as *mut c_void);
            }
        }
        result
    }

    /// Sets the global state pointer and its destructor.
    ///
    /// # Safety
    ///
    /// `state` must remain valid until `DuckDB` calls `destroy`, or for the
    /// lifetime of the query if `destroy` is `None`.
    pub unsafe fn set_global_state(&self, state: *mut c_void, destroy: duckdb_delete_callback_t) {
        // SAFETY: self.info is valid; state validity is the caller's responsibility.
        unsafe {
            duckdb_copy_function_global_init_set_global_state(self.info, state, destroy);
        }
    }

    /// Reports a fatal error, causing `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_copy_function_global_init_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the client context for this callback.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    pub unsafe fn get_client_context(&self) -> crate::client_context::ClientContext {
        // SAFETY: self.info is a valid copy-global-init-info handle per this fn's contract.
        let ctx = unsafe { duckdb_copy_function_global_init_get_client_context(self.info) };
        // SAFETY: `ctx` is a client-context handle returned by DuckDB.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the underlying raw handle.
    #[mutants::skip]
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_copy_function_global_init_info {
        self.info
    }
}

// ── CopySinkInfo ─────────────────────────────────────────────────────────────

/// Wrapper around the `duckdb_copy_function_sink_info` handle provided to a
/// copy function sink callback.
pub struct CopySinkInfo {
    info: duckdb_copy_function_sink_info,
}

impl CopySinkInfo {
    /// Wraps a raw `duckdb_copy_function_sink_info` handle.
    ///
    /// # Safety
    ///
    /// `info` must be a valid handle passed by `DuckDB` to a copy function
    /// sink callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_copy_function_sink_info) -> Self {
        Self { info }
    }

    /// Retrieves the bind data pointer set during the bind phase.
    ///
    /// # Safety
    ///
    /// The returned pointer must be cast back to the original type.
    #[must_use]
    pub unsafe fn get_bind_data(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_sink_get_bind_data(self.info) }
    }

    /// Retrieves the extra-info pointer previously set on the copy function.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the copy function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_sink_get_extra_info(self.info) }
    }

    /// Retrieves the global state pointer set during the global init phase.
    ///
    /// # Safety
    ///
    /// The returned pointer must be cast back to the original type.
    #[must_use]
    pub unsafe fn get_global_state(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_sink_get_global_state(self.info) }
    }

    /// Reports a fatal error, causing `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_copy_function_sink_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the client context for this callback.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    pub unsafe fn get_client_context(&self) -> crate::client_context::ClientContext {
        // SAFETY: self.info is a valid copy-sink-info handle per this fn's contract.
        let ctx = unsafe { duckdb_copy_function_sink_get_client_context(self.info) };
        // SAFETY: `ctx` is a client-context handle returned by DuckDB.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the underlying raw handle.
    #[mutants::skip]
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_copy_function_sink_info {
        self.info
    }
}

// ── CopyFinalizeInfo ─────────────────────────────────────────────────────────

/// Wrapper around the `duckdb_copy_function_finalize_info` handle provided to
/// a copy function finalize callback.
pub struct CopyFinalizeInfo {
    info: duckdb_copy_function_finalize_info,
}

impl CopyFinalizeInfo {
    /// Wraps a raw `duckdb_copy_function_finalize_info` handle.
    ///
    /// # Safety
    ///
    /// `info` must be a valid handle passed by `DuckDB` to a copy function
    /// finalize callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_copy_function_finalize_info) -> Self {
        Self { info }
    }

    /// Retrieves the bind data pointer set during the bind phase.
    ///
    /// # Safety
    ///
    /// The returned pointer must be cast back to the original type.
    #[must_use]
    pub unsafe fn get_bind_data(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_finalize_get_bind_data(self.info) }
    }

    /// Retrieves the extra-info pointer previously set on the copy function.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the copy function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_finalize_get_extra_info(self.info) }
    }

    /// Retrieves the global state pointer set during the global init phase.
    ///
    /// # Safety
    ///
    /// The returned pointer must be cast back to the original type.
    #[must_use]
    pub unsafe fn get_global_state(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_copy_function_finalize_get_global_state(self.info) }
    }

    /// Reports a fatal error, causing `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_copy_function_finalize_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the client context for this callback.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    pub unsafe fn get_client_context(&self) -> crate::client_context::ClientContext {
        // SAFETY: self.info is a valid copy-finalize-info handle per this fn's contract.
        let ctx = unsafe { duckdb_copy_function_finalize_get_client_context(self.info) };
        // SAFETY: `ctx` is a client-context handle returned by DuckDB.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the underlying raw handle.
    #[mutants::skip]
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_copy_function_finalize_info {
        self.info
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_bind_info_wraps_null() {
        let _info = unsafe { CopyBindInfo::new(std::ptr::null_mut()) };
    }

    #[test]
    fn copy_global_init_info_wraps_null() {
        let _info = unsafe { CopyGlobalInitInfo::new(std::ptr::null_mut()) };
    }

    #[test]
    fn copy_sink_info_wraps_null() {
        let _info = unsafe { CopySinkInfo::new(std::ptr::null_mut()) };
    }

    #[test]
    fn copy_finalize_info_wraps_null() {
        let _info = unsafe { CopyFinalizeInfo::new(std::ptr::null_mut()) };
    }
}
