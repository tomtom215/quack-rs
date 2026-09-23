// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrappers around `DuckDB` callback info handles.
//!
//! These types provide safe, chainable methods for the most common operations
//! performed inside bind, init, and scan callbacks.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use libduckdb_sys::{
    duckdb_bind_add_result_column, duckdb_bind_get_extra_info, duckdb_bind_get_named_parameter,
    duckdb_bind_get_parameter, duckdb_bind_info, duckdb_bind_set_cardinality,
    duckdb_bind_set_error, duckdb_function_get_extra_info, duckdb_function_info,
    duckdb_function_set_error, duckdb_init_get_extra_info, duckdb_init_info, duckdb_init_set_error,
    duckdb_value, idx_t,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{duckdb_client_context, duckdb_table_function_get_client_context};

use crate::table::cstr::{error_cstring, str_to_cstring};
use crate::types::{LogicalType, TypeId};
use crate::value::Value;

/// The message the table function wrappers report in place of an empty one.
///
/// [`BindInfo::set_error`], [`InitInfo::set_error`] and
/// [`FunctionInfo::set_error`] substitute it for `""` (or a message that is
/// empty after truncation at an interior NUL): `DuckDB` would otherwise report
/// `Binder Error: ` followed by nothing, and the user would learn only that
/// something failed.
pub const EMPTY_ERROR_PLACEHOLDER: &str = "table function reported an error without a message";

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
    /// Whether [`set_error`][Self::set_error] has been called on this
    /// wrapper. The typed bind trampoline reads it so its own "no columns
    /// declared" error does not overwrite a more specific one (such as a
    /// rejected column type). An `AtomicBool` rather than a `Cell` so the
    /// wrapper stays `RefUnwindSafe`.
    error_reported: AtomicBool,
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
        Self {
            info,
            error_reported: AtomicBool::new(false),
        }
    }

    /// Whether [`set_error`][Self::set_error] was called through this wrapper.
    ///
    /// An error set by calling `duckdb_bind_set_error` on
    /// [`as_raw`][Self::as_raw] directly is not seen.
    // Only the typed trampoline's `duckdb-1-5` column check reads it.
    #[cfg_attr(not(feature = "duckdb-1-5"), allow(dead_code))]
    pub(crate) fn error_reported(&self) -> bool {
        self.error_reported.load(Ordering::Relaxed)
    }

    /// Declares an output column with the given name and type.
    ///
    /// Call this once per output column in the order they will appear in the result.
    ///
    /// If `name` contains an interior null byte it is truncated at that point.
    ///
    /// # Rejected types
    ///
    /// A type that cannot be a result column is reported as a **bind error**
    /// (via [`set_error`][Self::set_error]) naming the column, and the column
    /// is not added:
    ///
    /// - `ANY` (and, through
    ///   [`add_result_column_with_type`][Self::add_result_column_with_type],
    ///   any type *containing* `ANY` or `INVALID`, such as `LIST(ANY)`).
    ///   `duckdb_bind_add_result_column` would silently drop such a column,
    ///   shifting the index of every column declared after it.
    /// - A composite id (`List`, `Struct`, `Decimal`, ...) that needs its own
    ///   [`LogicalType`] constructor; use `add_result_column_with_type`.
    ///   (This used to panic.)
    ///
    /// The query then fails at bind time with that message, even if the bind
    /// callback goes on to succeed.
    pub fn add_result_column(&self, name: &str, type_id: TypeId) -> &Self {
        match LogicalType::try_new(type_id) {
            Ok(lt) => self.add_result_column_with_type(name, &lt),
            Err(e) => {
                self.set_error(&format!("add_result_column('{name}'): {e}"));
                self
            }
        }
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
    ///
    /// A type containing `ANY` or `INVALID` is reported as a bind error rather
    /// than added; see "Rejected types" on
    /// [`add_result_column`][Self::add_result_column].
    pub fn add_result_column_with_type(&self, name: &str, logical_type: &LogicalType) -> &Self {
        // SAFETY: `logical_type` owns a live handle.
        if unsafe { crate::table::type_check::contains_any_or_invalid(logical_type.as_raw()) } {
            self.set_error(&format!(
                "add_result_column('{name}'): a result column's type must not be or contain \
                 ANY or INVALID; DuckDB would silently drop the column and shift every later one"
            ));
            return self;
        }
        let c_name = str_to_cstring(name);
        // SAFETY: self.info is valid; logical_type.as_raw() is valid.
        unsafe {
            duckdb_bind_add_result_column(self.info, c_name.as_ptr(), logical_type.as_raw());
        }
        self
    }

    /// Sets a cardinality hint for the query optimizer.
    ///
    /// # What `is_exact` actually does
    ///
    /// `duckdb.h` describes `is_exact` as "whether or not the cardinality is
    /// exact", but `DuckDB` 1.5.5 implements it the other way round
    /// (`src/main/capi/table_function-c.cpp`, `duckdb_bind_set_cardinality`):
    ///
    /// | `is_exact` | `NodeStatistics` recorded |
    /// |---|---|
    /// | `true`  | `NodeStatistics(rows)` — an **estimate only**, no upper bound |
    /// | `false` | `NodeStatistics(rows, rows)` — the estimate **and** `max_cardinality = rows` |
    ///
    /// The estimate drives plan costing either way (`EXPLAIN` shows `~rows`
    /// for both). The upper bound additionally feeds the optimizer's
    /// statistics propagation for joins and set operations
    /// (`src/optimizer/statistics/operator/propagate_join.cpp`), which treats
    /// it as a limit, so pass `false` only when `rows` really is a bound the
    /// scan never exceeds, and `true` for a guess.
    /// quack-rs passes the flag through unchanged, so code stays correct if
    /// `DuckDB` later swaps the branches to match its header.
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
    /// An empty message is replaced by [`EMPTY_ERROR_PLACEHOLDER`].
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        self.error_reported.store(true, Ordering::Relaxed);
        let c_msg = error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
    /// # A parameter the caller did not supply
    ///
    /// Named parameters are optional in SQL. When the query does not pass
    /// `name := ...`, `DuckDB` returns a null handle, and so does this: the
    /// returned `Value` has [`is_null`][Value::is_null] `== true`. Test for
    /// that, or read it with a defaulting accessor such as
    /// [`as_i64_or`][Value::as_i64_or], before using a plain getter.
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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

    /// Returns the declared column index at the given projection position, or
    /// `None` when `projection_idx` is not less than
    /// [`projected_column_count`][Self::projected_column_count].
    ///
    /// Only meaningful when projection pushdown is enabled.
    ///
    /// `duckdb_init_get_column_index` answers an out-of-range position with
    /// `0` — indistinguishable from "the first declared column" — so the range
    /// check happens here.
    #[mutants::skip]
    #[must_use]
    pub fn projected_column_index(&self, projection_idx: usize) -> Option<usize> {
        if projection_idx >= self.projected_column_count() {
            return None;
        }
        // SAFETY: self.info is valid and `projection_idx` is in range.
        usize::try_from(unsafe {
            libduckdb_sys::duckdb_init_get_column_index(self.info, projection_idx as idx_t)
        })
        .ok()
    }

    /// Sets the maximum number of threads for parallel scanning.
    ///
    /// The default is 1. Above 1, `DuckDB` may call the scan callback from up
    /// to `n` threads **at the same time**, whether or not `local_init` is set
    /// (`local_init` only gives each thread its own state; it is not what
    /// enables parallelism). All of those calls share the one global init
    /// data and the one bind data, so after raising this, the scan must not
    /// use [`FfiInitData::get_mut`][crate::table::FfiInitData::get_mut]; keep
    /// shared mutable state behind a `Mutex` or atomics instead.
    #[mutants::skip]
    pub fn set_max_threads(&self, n: u64) {
        // SAFETY: self.info is valid.
        unsafe { libduckdb_sys::duckdb_init_set_max_threads(self.info, n as idx_t) };
    }

    /// Reports an error from the init callback.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    /// An empty message is replaced by [`EMPTY_ERROR_PLACEHOLDER`].
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
    /// An empty message is replaced by [`EMPTY_ERROR_PLACEHOLDER`].
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
