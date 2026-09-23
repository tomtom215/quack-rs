// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Callback info wrappers for copy function callbacks.
//!
//! Each copy function phase (bind, global init, sink, finalize) receives an
//! opaque info handle from `DuckDB`. These wrappers provide safe, ergonomic
//! access to the underlying C API functions.

use std::ffi::CStr;
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

use crate::table::cstr::str_to_cstring;
use crate::types::LogicalType;

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

    /// Returns the logical type of the column at `index`, or `None` if
    /// `index` is not less than [`column_count`][Self::column_count].
    ///
    /// `duckdb_copy_function_bind_get_column_type` returns a null handle for
    /// an out-of-range index; wrapping that in a [`LogicalType`] would hand
    /// `DuckDB` a null pointer on the first use. This mirrors
    /// [`BindInfo::result_column_type`][crate::table::BindInfo::result_column_type].
    ///
    /// # Safety
    ///
    /// Must be called during the bind callback, while `self.info` is live.
    #[must_use]
    pub unsafe fn column_type(&self, index: u64) -> Option<LogicalType> {
        if index >= self.column_count() {
            return None;
        }
        // SAFETY: self.info is valid and `index` is in range.
        let raw = unsafe { duckdb_copy_function_bind_get_column_type(self.info, index) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: DuckDB returns a freshly allocated handle the caller owns.
        Some(unsafe { LogicalType::from_raw(raw) })
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

    /// The options given to the `COPY … TO` statement, as a `STRUCT` value.
    ///
    /// `COPY t TO 'f' (FORMAT my_format, COMPRESSION 'zstd', LEVEL 3)` arrives
    /// here as a `STRUCT` whose fields are the option names. Read them with
    /// [`Value::struct_field_names`][crate::value::Value::struct_field_names]
    /// and [`Value::struct_child`][crate::value::Value::struct_child].
    ///
    /// How `DuckDB` 1.5.5 builds it (`MakeValueFromCopyOptions`,
    /// `src/main/capi/copy_function-c.cpp`):
    ///
    /// - **Field names are upper-cased**: `compression` arrives as
    ///   `COMPRESSION`, whatever case the user typed. `FORMAT` itself is not
    ///   among them.
    /// - **No options at all** gives a SQL `NULL`, not an empty `STRUCT`:
    ///   the returned [`Value`][crate::value::Value] is non-null as a handle but
    ///   [`is_sql_null`][crate::value::Value::is_sql_null], and has no fields.
    /// - **An option with no value** (`(FORMAT f, HEADER)`) is a `NULL`
    ///   field.
    /// - **An option with several values** (`LST (1, 2)`) is a `LIST` when
    ///   they share a type and an unnamed `STRUCT` otherwise; one value is
    ///   that value.
    /// - An explicit `NULL` value (`(FORMAT f, X NULL)`) never gets here:
    ///   the binder rejects it ("NULL is not supported as a valid option").
    ///
    /// Returns `None` only when `DuckDB` hands back a null handle.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::copy_function::CopyBindInfo;
    ///
    /// # fn demo(bind: &CopyBindInfo) -> Option<String> {
    /// let options = bind.options()?;
    /// if options.is_sql_null() {
    ///     return None; // COPY ... (FORMAT my_format) with no other options
    /// }
    /// let names = options.struct_field_names();
    /// // Option names arrive upper-cased.
    /// let idx = names.iter().position(|n| n == "COMPRESSION")?;
    /// options.struct_child(idx)?.as_str().ok()
    /// # }
    /// ```
    #[must_use]
    pub fn options(&self) -> Option<crate::value::Value> {
        // SAFETY: `self.info` is valid per the constructor's contract. DuckDB
        // returns a freshly allocated `duckdb_value` the caller owns.
        let raw = unsafe { libduckdb_sys::duckdb_copy_function_bind_get_options(self.info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is owned by us from here; `Value` destroys it on drop.
        Some(unsafe { crate::value::Value::from_raw(raw) })
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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

    /// Returns the destination path for the copy operation.
    ///
    /// The returned `String` is a copy. `DuckDB` keeps ownership of its own
    /// buffer — `duckdb_copy_function_global_init_get_file_path` returns
    /// `info_ref.file_path.c_str()`, the interior pointer of a live C++
    /// `std::string`, so it must **not** be freed. Doing so corrupts the heap.
    ///
    /// # Safety
    ///
    /// `self.info` must be a valid handle from an active callback invocation.
    #[must_use]
    pub unsafe fn get_file_path(&self) -> String {
        // SAFETY: self.info is valid per constructor contract.
        let c_str = unsafe { duckdb_copy_function_global_init_get_file_path(self.info) };
        if c_str.is_null() {
            return String::new();
        }
        // SAFETY: c_str is a valid NUL-terminated string that DuckDB owns and
        // keeps alive for the duration of this callback. It is copied here and
        // deliberately not freed: unlike the `char *` returns elsewhere in the
        // C API (which `strdup` or `duckdb_malloc`), this one is `const char *`
        // and borrowed.
        unsafe { CStr::from_ptr(c_str) }
            .to_str()
            .unwrap_or("")
            .to_owned()
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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

crate::debug_repr::impl_handle_debug!(
    CopyBindInfo.info,
    CopyGlobalInitInfo.info,
    CopySinkInfo.info,
    CopyFinalizeInfo.info
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Each wrapper hands back exactly the handle it was built from.
    #[test]
    fn info_wrappers_round_trip_their_handles() {
        let raw = std::ptr::NonNull::<u8>::dangling().as_ptr();
        // SAFETY: the handles are only stored and read back, never passed to DuckDB.
        unsafe {
            assert_eq!(CopyBindInfo::new(raw.cast()).as_raw().cast(), raw);
            assert_eq!(CopyGlobalInitInfo::new(raw.cast()).as_raw().cast(), raw);
            assert_eq!(CopySinkInfo::new(raw.cast()).as_raw().cast(), raw);
            assert_eq!(CopyFinalizeInfo::new(raw.cast()).as_raw().cast(), raw);
        }
    }

    /// A null bind info reports no columns, so `column_type` refuses every
    /// index before reaching `duckdb_copy_function_bind_get_column_type`.
    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn column_type_is_none_out_of_range() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        // SAFETY: DuckDB null-checks the bind info in both calls.
        let info = unsafe { CopyBindInfo::new(std::ptr::null_mut()) };
        assert_eq!(info.column_count(), 0);
        // SAFETY: as above.
        assert!(unsafe { info.column_type(0) }.is_none());
    }
}
