// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrappers around `DuckDB` callback info handles.
//!
//! These types provide safe, chainable methods for the most common operations
//! performed inside bind, init, and scan callbacks.

use std::ffi::CString;

use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_bind_add_result_column, duckdb_bind_get_extra_info, duckdb_bind_get_named_parameter,
    duckdb_bind_get_parameter, duckdb_bind_info, duckdb_bind_set_cardinality,
    duckdb_bind_set_error, duckdb_function_get_extra_info, duckdb_function_info,
    duckdb_function_set_error, duckdb_init_get_extra_info, duckdb_init_info, duckdb_init_set_error,
    duckdb_value, idx_t,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{duckdb_client_context, duckdb_table_function_get_client_context};

use crate::types::{LogicalType, TypeId};
use crate::value::Value;

/// Converts a `&str` to `CString` without panicking.
///
/// If the string contains an interior null byte, it is truncated at that point.
/// This is preferred over `.expect()` in FFI callback contexts where panics are UB.
#[mutants::skip] // private FFI helper — tested in replacement_scan::tests
fn str_to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let pos = s.bytes().position(|b| b == 0).unwrap_or(s.len());
        // SAFETY: pos is at the first null byte, so s[..pos] has no nulls.
        CString::new(&s.as_bytes()[..pos]).unwrap_or_default()
    })
}

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
    /// If `name` contains an interior null byte it is truncated at that point.
    pub fn add_result_column(&self, name: &str, type_id: TypeId) -> &Self {
        let c_name = str_to_cstring(name);
        let lt = LogicalType::new(type_id);
        // SAFETY: self.info is valid per constructor's contract.
        unsafe {
            duckdb_bind_add_result_column(self.info, c_name.as_ptr(), lt.as_raw());
        }
        self
    }

    /// Number of result columns `DuckDB` already knows this table function must
    /// produce.
    ///
    /// Zero for an ordinary table function, which is expected to declare its own
    /// columns with
    /// [`add_result_column`][Self::add_result_column]. Non-zero when the
    /// function is driving a `COPY … FROM`: the target table's schema is fixed
    /// before the bind callback runs, and `duckdb.h` is explicit that such a
    /// function "should not define its own result columns using
    /// `duckdb_bind_add_result_column`" and should read the expected schema from
    /// here instead.
    ///
    /// Requires `duckdb-1-5`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::table::BindInfo;
    ///
    /// # fn demo(bind: &BindInfo) {
    /// // A COPY ... FROM reader adapts to the table it is loading into.
    /// for i in 0..bind.result_column_count() {
    ///     let name = bind.result_column_name(i).unwrap_or_default();
    ///     // SAFETY: `i` is in range.
    ///     let ty = unsafe { bind.result_column_type(i) };
    ///     let _ = (name, ty);
    /// }
    /// # }
    /// ```
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // FFI accessor — needs a live DuckDB bind callback
    #[must_use]
    pub fn result_column_count(&self) -> usize {
        // SAFETY: `self.info` is valid per the constructor's contract.
        usize::try_from(unsafe {
            libduckdb_sys::duckdb_table_function_bind_get_result_column_count(self.info)
        })
        .unwrap_or(0)
    }

    /// Name of result column `index`, or `None` if it is out of range.
    ///
    /// The C API returns a **borrowed** pointer into `DuckDB`'s own storage —
    /// `names[col_idx].c_str()` — which `duckdb.h` says "must not be destroyed"
    /// and which is only valid "for the duration of the bind callback or until
    /// the next call to `duckdb_bind_add_result_column`". This copies it, so the
    /// returned `String` is not subject to either restriction (pitfall P11).
    ///
    /// Requires `duckdb-1-5`.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // FFI accessor — needs a live DuckDB bind callback
    #[must_use]
    pub fn result_column_name(&self, index: usize) -> Option<String> {
        if index >= self.result_column_count() {
            return None;
        }
        // SAFETY: `self.info` is valid and `index` is in range.
        let ptr = unsafe {
            libduckdb_sys::duckdb_table_function_bind_get_result_column_name(
                self.info,
                index as idx_t,
            )
        };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `ptr` is a NUL-terminated string DuckDB owns; it is copied
        // here and never freed by this crate.
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_str()
            .ok()
            .map(str::to_owned)
    }

    /// Type of result column `index`, or `None` if it is out of range.
    ///
    /// Unlike [`result_column_name`][Self::result_column_name] this handle is
    /// **owned** — `duckdb_table_function_bind_get_result_column_type` returns a
    /// freshly allocated `LogicalType` — so it is wrapped in [`LogicalType`],
    /// which destroys it on drop.
    ///
    /// Requires `duckdb-1-5`.
    ///
    /// # Safety
    ///
    /// Must be called during the bind callback, while `self.info` is live.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // FFI accessor — needs a live DuckDB bind callback
    #[must_use]
    pub unsafe fn result_column_type(&self, index: usize) -> Option<LogicalType> {
        if index >= self.result_column_count() {
            return None;
        }
        // SAFETY: `self.info` is valid and `index` is in range.
        let raw = unsafe {
            libduckdb_sys::duckdb_table_function_bind_get_result_column_type(
                self.info,
                index as idx_t,
            )
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: DuckDB returns a handle the caller owns and must destroy.
        Some(unsafe { LogicalType::from_raw(raw) })
    }

    /// Adds an output column with a pre-built `LogicalType`.
    ///
    /// Use this when the column type is a complex type (LIST, STRUCT, MAP) built
    /// via `LogicalType::list`, `LogicalType::struct_type`, or `LogicalType::map`.
    ///
    /// If `name` contains an interior null byte it is truncated at that point.
    pub fn add_result_column_with_type(&self, name: &str, logical_type: &LogicalType) -> &Self {
        let c_name = str_to_cstring(name);
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
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid.
        unsafe {
            duckdb_bind_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the number of positional parameters passed to this function call.
    #[mutants::skip]
    #[must_use]
    pub fn parameter_count(&self) -> usize {
        // SAFETY: self.info is valid.
        usize::try_from(unsafe { libduckdb_sys::duckdb_bind_get_parameter_count(self.info) })
            .unwrap_or(0)
    }

    /// Returns the parameter value at the given positional index.
    ///
    /// # Safety
    ///
    /// - `index` must be less than [`parameter_count`][BindInfo::parameter_count].
    /// - The caller is responsible for destroying the returned `duckdb_value`.
    pub unsafe fn get_parameter(&self, index: u64) -> duckdb_value {
        unsafe { duckdb_bind_get_parameter(self.info, index) }
    }

    /// Returns the parameter value for the given named parameter.
    ///
    /// # Safety
    ///
    /// - `name` must correspond to a named parameter declared for this function.
    /// - The caller is responsible for destroying the returned `duckdb_value`.
    ///
    /// If `name` contains an interior null byte it is truncated at that point.
    pub unsafe fn get_named_parameter(&self, name: &str) -> duckdb_value {
        let c_name = str_to_cstring(name);
        unsafe { duckdb_bind_get_named_parameter(self.info, c_name.as_ptr()) }
    }

    /// Returns the positional parameter at `index` as an owned [`Value`].
    ///
    /// The returned `Value` is RAII-managed — it will call `duckdb_destroy_value`
    /// on drop, so the caller does not need to manually free it.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`parameter_count`][BindInfo::parameter_count].
    pub unsafe fn get_parameter_value(&self, index: u64) -> Value {
        // SAFETY: index is valid per caller's contract.
        let raw = unsafe { duckdb_bind_get_parameter(self.info, index) };
        // SAFETY: raw is a fresh duckdb_value owned by us.
        unsafe { Value::from_raw(raw) }
    }

    /// Returns the named parameter as an owned [`Value`].
    ///
    /// The returned `Value` is RAII-managed — it will call `duckdb_destroy_value`
    /// on drop.
    ///
    /// # Safety
    ///
    /// `name` must correspond to a named parameter declared for this function.
    ///
    /// If `name` contains an interior null byte it is truncated at that point.
    pub unsafe fn get_named_parameter_value(&self, name: &str) -> Value {
        let c_name = str_to_cstring(name);
        // SAFETY: name is valid per caller's contract.
        let raw = unsafe { duckdb_bind_get_named_parameter(self.info, c_name.as_ptr()) };
        // SAFETY: raw is a fresh duckdb_value owned by us.
        unsafe { Value::from_raw(raw) }
    }

    /// Returns the extra info pointer set on the table function.
    ///
    /// # Safety
    ///
    /// The caller must ensure the returned pointer (if non-null) is used
    /// according to its original type.
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        unsafe { duckdb_bind_get_extra_info(self.info) }
    }

    /// Returns the client context for this callback.
    ///
    /// The returned [`ClientContext`][crate::client_context::ClientContext] provides
    /// access to the connection's catalog, configuration, and connection ID.
    ///
    /// # Safety
    ///
    /// The inner handle must be valid (requires `DuckDB` runtime).
    #[cfg(feature = "duckdb-1-5")]
    pub unsafe fn get_client_context(&self) -> crate::client_context::ClientContext {
        let mut ctx: duckdb_client_context = core::ptr::null_mut();
        unsafe { duckdb_table_function_get_client_context(self.info, &raw mut ctx) };
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
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
    #[mutants::skip]
    #[must_use]
    pub fn projected_column_count(&self) -> usize {
        // SAFETY: self.info is valid.
        usize::try_from(unsafe { libduckdb_sys::duckdb_init_get_column_count(self.info) })
            .unwrap_or(0)
    }

    /// Returns the output column index at the given projection position.
    ///
    /// Only valid when projection pushdown is enabled.
    #[mutants::skip]
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
    #[mutants::skip]
    pub fn set_max_threads(&self, n: u64) {
        // SAFETY: self.info is valid.
        unsafe { libduckdb_sys::duckdb_init_set_max_threads(self.info, n as idx_t) };
    }

    /// Reports an error from the init callback.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid.
        unsafe { duckdb_init_set_error(self.info, c_msg.as_ptr()) };
    }

    /// Returns the extra info pointer set on the table function.
    ///
    /// # Safety
    ///
    /// The caller must ensure the returned pointer (if non-null) is used
    /// according to its original type.
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        unsafe { duckdb_init_get_extra_info(self.info) }
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
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid.
        unsafe { duckdb_function_set_error(self.info, c_msg.as_ptr()) };
    }

    /// Returns the extra info pointer set on the table function.
    ///
    /// # Safety
    ///
    /// The caller must ensure the returned pointer (if non-null) is used
    /// according to its original type.
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        unsafe { duckdb_function_get_extra_info(self.info) }
    }

    /// Returns the raw `duckdb_function_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_function_info {
        self.info
    }
}

crate::debug_repr::impl_handle_debug!(BindInfo.info, InitInfo.info, FunctionInfo.info);
