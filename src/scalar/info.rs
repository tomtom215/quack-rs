// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrapper around `duckdb_function_info` for scalar function callbacks.

use std::ffi::CString;
use std::os::raw::c_void;

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    duckdb_bind_info, duckdb_client_context, duckdb_copy_callback_t, duckdb_delete_callback_t,
    duckdb_expression, duckdb_init_info, duckdb_scalar_function_bind_get_argument,
    duckdb_scalar_function_bind_get_argument_count, duckdb_scalar_function_bind_get_extra_info,
    duckdb_scalar_function_bind_set_error, duckdb_scalar_function_get_bind_data,
    duckdb_scalar_function_get_client_context, duckdb_scalar_function_get_state,
    duckdb_scalar_function_init_get_bind_data, duckdb_scalar_function_init_get_client_context,
    duckdb_scalar_function_init_get_extra_info, duckdb_scalar_function_init_set_error,
    duckdb_scalar_function_init_set_state, duckdb_scalar_function_set_bind_data,
    duckdb_scalar_function_set_bind_data_copy,
};
use libduckdb_sys::{
    duckdb_function_info, duckdb_scalar_function_get_extra_info, duckdb_scalar_function_set_error,
};

#[cfg(feature = "duckdb-1-5")]
use crate::expression::Expression;

/// Converts a `&str` to `CString` without panicking.
#[mutants::skip] // private FFI helper — tested in replacement_scan::tests
fn str_to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let pos = s.bytes().position(|b| b == 0).unwrap_or(s.len());
        CString::new(&s.as_bytes()[..pos]).unwrap_or_default()
    })
}

/// Ergonomic wrapper around the `duckdb_function_info` handle provided to a
/// scalar function callback.
///
/// Provides access to extra info and error reporting.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::scalar::ScalarFunctionInfo;
/// use libduckdb_sys::{duckdb_function_info, duckdb_data_chunk, duckdb_vector};
///
/// unsafe extern "C" fn my_func(
///     info: duckdb_function_info,
///     _input: duckdb_data_chunk,
///     _output: duckdb_vector,
/// ) {
///     let info = unsafe { ScalarFunctionInfo::new(info) };
///     let _extra = unsafe { info.get_extra_info() };
///     // ... use extra info ...
/// }
/// ```
pub struct ScalarFunctionInfo {
    info: duckdb_function_info,
}

impl ScalarFunctionInfo {
    /// Wraps a raw `duckdb_function_info` provided by `DuckDB` inside a scalar
    /// function callback.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_function_info` passed by `DuckDB` to a
    /// scalar function callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_function_info) -> Self {
        Self { info }
    }

    /// Retrieves the extra-info pointer previously set via
    /// [`ScalarFunctionBuilder::extra_info`][crate::scalar::ScalarFunctionBuilder::extra_info].
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the scalar function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_get_extra_info(self.info) }
    }

    /// Reports an error from the scalar function callback, causing `DuckDB`
    /// to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_scalar_function_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the raw `duckdb_function_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_function_info {
        self.info
    }

    /// Retrieves the bind data pointer previously set via
    /// [`ScalarBindInfo::set_bind_data`] during the bind callback.
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as `DuckDB` has not yet called
    /// the destructor registered with [`ScalarBindInfo::set_bind_data`].
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub unsafe fn get_bind_data(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_get_bind_data(self.info) }
    }

    /// Retrieves the per-thread state pointer previously set via
    /// [`ScalarInitInfo::set_state`] during the init callback.
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as `DuckDB` has not yet called
    /// the destructor registered with [`ScalarInitInfo::set_state`].
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub unsafe fn get_state(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_get_state(self.info) }
    }
}

/// Ergonomic wrapper around the `duckdb_bind_info` handle provided to a
/// scalar function bind callback (`DuckDB` 1.5.0+).
///
/// Provides access to function arguments, extra info, bind data storage,
/// and error reporting.
#[cfg(feature = "duckdb-1-5")]
pub struct ScalarBindInfo {
    info: duckdb_bind_info,
}

#[cfg(feature = "duckdb-1-5")]
impl ScalarBindInfo {
    /// Wraps a raw `duckdb_bind_info` provided by `DuckDB` inside a scalar
    /// function bind callback.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_bind_info` passed by `DuckDB` to a
    /// scalar function bind callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_bind_info) -> Self {
        Self { info }
    }

    /// Returns the number of arguments passed to the scalar function.
    #[mutants::skip]
    #[must_use]
    pub fn argument_count(&self) -> u64 {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_bind_get_argument_count(self.info) }
    }

    /// Returns the argument expression at `index`.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`argument_count`][Self::argument_count].
    /// The returned `duckdb_expression` handle is owned by the caller and
    /// must be used according to `DuckDB` expression API rules.
    #[must_use]
    pub unsafe fn get_argument(&self, index: u64) -> duckdb_expression {
        // SAFETY: self.info is valid per constructor contract; caller guarantees index.
        unsafe { duckdb_scalar_function_bind_get_argument(self.info, index) }
    }

    /// Returns the argument at `index` as an RAII [`Expression`], or `None` if
    /// `DuckDB` returns a null handle.
    ///
    /// This is the ergonomic counterpart to [`get_argument`][Self::get_argument]:
    /// the returned [`Expression`] is destroyed automatically on drop and exposes
    /// safe accessors for the argument's return type and constant folding.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`argument_count`][Self::argument_count].
    #[must_use]
    pub unsafe fn argument(&self, index: u64) -> Option<Expression> {
        // SAFETY: self.info is valid per constructor contract; caller guarantees index.
        let raw = unsafe { duckdb_scalar_function_bind_get_argument(self.info, index) };
        if raw.is_null() {
            None
        } else {
            // SAFETY: raw is a non-null, owned duckdb_expression handle.
            Some(unsafe { Expression::from_raw(raw) })
        }
    }

    /// Retrieves the extra-info pointer previously set via
    /// [`ScalarFunctionBuilder::extra_info`][crate::scalar::ScalarFunctionBuilder::extra_info].
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the scalar function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_bind_get_extra_info(self.info) }
    }

    /// Stores per-query bind data that can later be retrieved during execution
    /// via [`ScalarFunctionInfo::get_bind_data`].
    ///
    /// # Pair this with [`set_bind_data_copy`][Self::set_bind_data_copy]
    ///
    /// `DuckDB` copies a bound expression whenever it duplicates a plan — and
    /// when no copy callback is registered, the copy's bind data is **null**,
    /// not a duplicate of yours. From `DuckDB`'s
    /// `src/main/capi/scalar_function-c.cpp`:
    ///
    /// ```cpp
    /// unique_ptr<FunctionData> Copy() const override {
    ///     auto copy = make_uniq<CScalarFunctionBindData>(info);
    ///     if (copy_callback) {
    ///         copy->bind_data = copy_callback(bind_data);
    ///         // ...
    ///     }
    ///     return std::move(copy);   // bind_data stays null without a callback
    /// }
    /// ```
    ///
    /// The execute callback then sees a null pointer from
    /// [`ScalarFunctionInfo::get_bind_data`] on the copied expression. That is
    /// a **silent wrong answer**, not a crash, so always register a copy
    /// callback alongside any bind data you rely on at execution time.
    ///
    /// Written up as Pitfall L10 in `LESSONS.md`.
    ///
    /// # Safety
    ///
    /// `data` must point to valid memory. `destroy` will be called by `DuckDB`
    /// to free the data when the query finishes. The typical pattern is to box
    /// your data: `Box::into_raw(Box::new(my_data)).cast()`.
    pub unsafe fn set_bind_data(&self, data: *mut c_void, destroy: duckdb_delete_callback_t) {
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_scalar_function_set_bind_data(self.info, data, destroy);
        }
    }

    /// Registers the callback `DuckDB` uses to duplicate the bind data set by
    /// [`set_bind_data`][Self::set_bind_data] when it copies a bound
    /// expression.
    ///
    /// Without this, a copied expression carries **null** bind data — see
    /// [`set_bind_data`][Self::set_bind_data] for the upstream code that does
    /// it. Registering a copy callback is therefore not an optimisation; it is
    /// what makes bind data survive.
    ///
    /// `copy` receives the pointer passed to `set_bind_data` and must return a
    /// pointer to an **independently owned** duplicate. `DuckDB` frees the
    /// duplicate with the same `destroy` callback given to `set_bind_data`, so
    /// the two must agree on ownership — returning the same pointer twice is a
    /// double free.
    ///
    /// # Order
    ///
    /// `DuckDB` stores the copy callback on the same bind-data slot, so call
    /// this **after** [`set_bind_data`][Self::set_bind_data] within one bind
    /// callback.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::os::raw::c_void;
    ///
    /// struct MyBindData {
    ///     pattern: String,
    /// }
    ///
    /// unsafe extern "C" fn destroy(data: *mut c_void) {
    ///     if !data.is_null() {
    ///         drop(unsafe { Box::from_raw(data.cast::<MyBindData>()) });
    ///     }
    /// }
    ///
    /// unsafe extern "C" fn copy(data: *mut c_void) -> *mut c_void {
    ///     if data.is_null() {
    ///         return std::ptr::null_mut();
    ///     }
    ///     // Clone into a fresh allocation; `destroy` frees each one once.
    ///     let src = unsafe { &*data.cast::<MyBindData>() };
    ///     let dup = Box::new(MyBindData {
    ///         pattern: src.pattern.clone(),
    ///     });
    ///     Box::into_raw(dup).cast()
    /// }
    ///
    /// // Inside a bind callback holding a `ScalarBindInfo`:
    /// // let boxed = Box::new(MyBindData { pattern: "%x%".to_owned() });
    /// // unsafe {
    /// //     bind_info.set_bind_data(Box::into_raw(boxed).cast(), Some(destroy));
    /// //     bind_info.set_bind_data_copy(Some(copy));
    /// // }
    /// ```
    ///
    /// # Safety
    ///
    /// `copy` must return a pointer that is valid to free with the `destroy`
    /// callback registered by [`set_bind_data`][Self::set_bind_data], and must
    /// not alias the pointer it was given. It must not unwind: it runs across
    /// an FFI boundary, so wrap any panicking work in
    /// [`catch_ffi_panic`][crate::callback::catch_ffi_panic] and return null on
    /// failure.
    pub unsafe fn set_bind_data_copy(&self, copy: duckdb_copy_callback_t) {
        // SAFETY: self.info is valid per constructor contract; `copy` is either
        // None or a function pointer whose contract the caller upholds.
        unsafe {
            duckdb_scalar_function_set_bind_data_copy(self.info, copy);
        }
    }

    /// Reports an error from the scalar function bind callback, causing
    /// `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_scalar_function_bind_set_error(self.info, c_msg.as_ptr());
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
        let mut ctx: duckdb_client_context = core::ptr::null_mut();
        // SAFETY: self.info is a valid bind-info handle per this fn's contract;
        // the call writes the client-context handle into `ctx`.
        unsafe { duckdb_scalar_function_get_client_context(self.info, &raw mut ctx) };
        // SAFETY: `ctx` was just populated by DuckDB with a client-context handle.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the raw `duckdb_bind_info` handle.
    #[mutants::skip]
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_bind_info {
        self.info
    }
}

/// Ergonomic wrapper around the `duckdb_init_info` handle provided to a
/// scalar function init callback (`DuckDB` 1.5.0+).
///
/// Provides access to extra info, bind data, per-thread state storage,
/// and error reporting.
#[cfg(feature = "duckdb-1-5")]
pub struct ScalarInitInfo {
    info: duckdb_init_info,
}

#[cfg(feature = "duckdb-1-5")]
impl ScalarInitInfo {
    /// Wraps a raw `duckdb_init_info` provided by `DuckDB` inside a scalar
    /// function init callback.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_init_info` passed by `DuckDB` to a
    /// scalar function init callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_init_info) -> Self {
        Self { info }
    }

    /// Retrieves the extra-info pointer previously set via
    /// [`ScalarFunctionBuilder::extra_info`][crate::scalar::ScalarFunctionBuilder::extra_info].
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the scalar function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_init_get_extra_info(self.info) }
    }

    /// Retrieves the bind data pointer previously set via
    /// [`ScalarBindInfo::set_bind_data`] during the bind callback.
    ///
    /// Returns a raw `*mut c_void`. Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as `DuckDB` has not yet called
    /// the destructor registered with [`ScalarBindInfo::set_bind_data`].
    #[must_use]
    pub unsafe fn get_bind_data(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_scalar_function_init_get_bind_data(self.info) }
    }

    /// Stores per-thread state that can later be retrieved during execution
    /// via [`ScalarFunctionInfo::get_state`].
    ///
    /// # Safety
    ///
    /// `state` must point to valid memory. `destroy` will be called by `DuckDB`
    /// to free the state when the thread finishes. The typical pattern is to box
    /// your data: `Box::into_raw(Box::new(my_state)).cast()`.
    pub unsafe fn set_state(&self, state: *mut c_void, destroy: duckdb_delete_callback_t) {
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_scalar_function_init_set_state(self.info, state, destroy);
        }
    }

    /// Reports an error from the scalar function init callback, causing
    /// `DuckDB` to abort the current query.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_scalar_function_init_set_error(self.info, c_msg.as_ptr());
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
        let mut ctx: duckdb_client_context = core::ptr::null_mut();
        // SAFETY: self.info is a valid init-info handle per this fn's contract;
        // the call writes the client-context handle into `ctx`.
        unsafe { duckdb_scalar_function_init_get_client_context(self.info, &raw mut ctx) };
        // SAFETY: `ctx` was just populated by DuckDB with a client-context handle.
        unsafe { crate::client_context::ClientContext::from_raw(ctx) }
    }

    /// Returns the raw `duckdb_init_info` handle.
    #[mutants::skip]
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_init_info {
        self.info
    }
}

crate::debug_repr::impl_handle_debug!(ScalarFunctionInfo.info);

#[cfg(feature = "duckdb-1-5")]
crate::debug_repr::impl_handle_debug!(ScalarBindInfo.info, ScalarInitInfo.info);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_function_info_as_raw_roundtrip() {
        let raw = std::ptr::null_mut();
        let info = unsafe { ScalarFunctionInfo::new(raw) };
        assert_eq!(info.as_raw(), raw);
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn scalar_bind_info_as_raw_roundtrip() {
        let raw = std::ptr::null_mut();
        let info = unsafe { ScalarBindInfo::new(raw) };
        assert_eq!(info.as_raw(), raw);
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn scalar_init_info_as_raw_roundtrip() {
        let raw = std::ptr::null_mut();
        let info = unsafe { ScalarInitInfo::new(raw) };
        assert_eq!(info.as_raw(), raw);
    }
}
