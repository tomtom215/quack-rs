// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic wrapper around `duckdb_function_info` for scalar function callbacks.

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

/// What the scalar `set_error` methods report when given an empty message:
/// `DuckDB` would otherwise show only an error-type prefix such as
/// `Invalid Input Error: `.
pub const EMPTY_ERROR_PLACEHOLDER: &str = "scalar function reported an error without a message";

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
    /// An interior NUL byte in `message` is replaced with `?`
    /// (see [`message_to_c_string`][crate::callback::message_to_c_string]),
    /// and an empty message by [`EMPTY_ERROR_PLACEHOLDER`], since `DuckDB`
    /// would otherwise report only an error-type prefix.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = crate::table::cstr::error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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

/// The argument `DuckDB` passes to a scalar function **bind** callback: a
/// `duckdb_bind_info`, in a type of its own.
///
/// A table function's bind callback receives a `duckdb_bind_info` too, but
/// `DuckDB` casts it to a different, larger struct, so a callback written for
/// one kind of function corrupts memory when registered on the other (a
/// table bind's `duckdb_bind_set_error` writes past the scalar bind info).
/// With its own argument type, [`ScalarBindFn`][crate::scalar::ScalarBindFn]
/// no longer accepts a table callback, and a scalar callback no longer fits a
/// table function's `bind`. `#[repr(transparent)]`, so the C ABI is the
/// pointer's.
///
/// ```rust,compile_fail
/// use quack_rs::scalar::ScalarFunctionBuilder;
/// quack_rs::table_bind_callback!(table_bind, |_info| {});
/// // Does not compile: a table bind callback takes a `duckdb_bind_info`.
/// let _ = ScalarFunctionBuilder::new("f").bind(table_bind);
/// ```
#[cfg(feature = "duckdb-1-5")]
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct RawScalarBindInfo(pub duckdb_bind_info);

/// The argument `DuckDB` passes to a scalar function **init** callback; see
/// [`RawScalarBindInfo`] for why it has a type of its own.
///
/// ```rust,compile_fail
/// use quack_rs::scalar::ScalarFunctionBuilder;
/// quack_rs::table_init_callback!(table_init, |_info| {});
/// // Does not compile: a table init callback takes a `duckdb_init_info`.
/// let _ = ScalarFunctionBuilder::new("f").init(table_init);
/// ```
#[cfg(feature = "duckdb-1-5")]
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct RawScalarInitInfo(pub duckdb_init_info);

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
    /// Wraps the info `DuckDB` passes to a scalar function bind callback.
    ///
    /// **Breaking** in 0.18.0: takes a [`RawScalarBindInfo`], the argument type
    /// of [`ScalarBindFn`][crate::scalar::ScalarBindFn], not a bare
    /// `duckdb_bind_info`.
    ///
    /// # Safety
    ///
    /// `info` must be the argument `DuckDB` passed to a scalar function bind
    /// callback that is still running.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: RawScalarBindInfo) -> Self {
        Self { info: info.0 }
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
    /// - `index` must be less than [`argument_count`][Self::argument_count].
    ///   The returned `duckdb_expression` handle is owned by the caller and
    ///   must be used according to `DuckDB` expression API rules.
    /// - On `DuckDB` before v1.5.5 the argument must be one `DuckDB` can copy.
    ///   Those releases copy the expression outside any `try`, and copying a
    ///   scalar subquery (`f((SELECT 1))`) throws a C++ exception through
    ///   this call, which aborts the process. [`argument`][Self::argument]
    ///   checks the engine version for you.
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
    /// The expression is the argument as the query wrote it, before `DuckDB`
    /// casts it to the declared parameter type: for a `BIGINT` parameter,
    /// `f(40 + 2)` reports an `INTEGER` return type, and folds to an `INTEGER`
    /// value.
    ///
    /// # Asking for an argument can fail the query
    ///
    /// `DuckDB` copies the argument's expression to hand it out, and some
    /// expressions cannot be copied — a scalar subquery, as in
    /// `f((SELECT 'x'))`, for one. `duckdb_scalar_function_bind_get_argument`
    /// then marks the bind as failed itself (`scalar_function-c.cpp`) and
    /// returns null, so the query fails whatever the callback does next, and
    /// the C API has no way to ask first. This returns `None` in that case and
    /// replaces `DuckDB`'s message — a raw JSON exception such as
    /// `{"exception_type":"Serialization",…}` — with one that says what
    /// happened. A bind callback that can do without an argument should not
    /// ask for it.
    ///
    /// That holds from `DuckDB` v1.5.5. Releases v1.5.0 to v1.5.4 copy the
    /// expression outside any `try`, so the same subquery throws a C++
    /// exception through the C API, which aborts a Rust process. On those
    /// releases (and on development builds that predate v1.5.5's fix) this
    /// therefore asks for no argument at all: it fails the bind with a message
    /// saying why and returns `None`.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`argument_count`][Self::argument_count].
    #[must_use]
    pub unsafe fn argument(&self, index: u64) -> Option<Expression> {
        // SAFETY: a bind callback runs with the dispatch table initialised.
        let engine = unsafe { crate::abi::engine_version() };
        if !engine
            .as_deref()
            .is_some_and(get_argument_catches_copy_failures)
        {
            self.set_error(&format!(
                "argument {index} of this scalar function cannot be inspected at bind time on \
                 DuckDB {}: before v1.5.5, DuckDB aborts the process when the argument is a \
                 scalar subquery, and there is no way to tell first. Upgrade to DuckDB v1.5.5 \
                 or later, or do not inspect arguments in the bind callback",
                engine.as_deref().unwrap_or("(unknown version)")
            ));
            return None;
        }
        // SAFETY: self.info is valid per constructor contract; caller guarantees index.
        // The engine catches a failed copy (checked above), so nothing throws.
        let raw = unsafe { duckdb_scalar_function_bind_get_argument(self.info, index) };
        if raw.is_null() {
            // DuckDB returns null without an error only for an index out of
            // range; for any other index a null means the copy threw and the
            // bind is already failed. Its error is a plain assignment, so this
            // message replaces the JSON one.
            if index < self.argument_count() {
                self.set_error(&format!(
                    "argument {index} of this scalar function cannot be inspected at bind \
                     time: DuckDB could not copy its expression (a scalar subquery cannot be \
                     copied), and it fails the query when a bind callback asks for such an \
                     argument"
                ));
            }
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
    /// Prefer [`ScalarBindData::set`][crate::scalar::ScalarBindData::set],
    /// which generates the destructor *and* the copy callback.
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
    /// # Calling it twice
    ///
    /// A second call overwrites the pointer and destructor without running the
    /// first destructor: the first value is leaked.
    ///
    /// # Identical calls share bind data
    ///
    /// `DuckDB` merges two calls with the same arguments without comparing
    /// their bind data, so the value stored here must depend only on the
    /// arguments, their types and `extra_info` — or the function must be
    /// volatile. See [`ScalarBindData`][crate::scalar::ScalarBindData].
    ///
    /// # Safety
    ///
    /// `data` must point to valid memory. `destroy` will be called by `DuckDB`
    /// to free the data when the query finishes. The typical pattern is to box
    /// your data: `Box::into_raw(Box::new(my_data)).cast()`. `DuckDB` reads the
    /// data from every executing thread at once, so it must be safe to share
    /// across threads (`Sync`) and to free from any of them (`Send`).
    ///
    /// A copy callback registered earlier in this bind — by
    /// [`set_bind_data_copy`][Self::set_bind_data_copy], or by
    /// [`ScalarBindData::set`][crate::scalar::ScalarBindData::set] — stays in
    /// effect: this call replaces only the pointer and the destructor
    /// (`duckdb_scalar_function_set_bind_data`), and `DuckDB` applies the old
    /// copy callback to the new `data` whenever it copies the expression. So
    /// `data` must be of the type that callback expects, or a matching copy
    /// callback must be registered after this call.
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
    /// An interior NUL byte in `message` is replaced with `?`
    /// (see [`message_to_c_string`][crate::callback::message_to_c_string]),
    /// and an empty message by [`EMPTY_ERROR_PLACEHOLDER`], since `DuckDB`
    /// would otherwise report only an error-type prefix.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = crate::table::cstr::error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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
    /// Wraps the info `DuckDB` passes to a scalar function init callback.
    ///
    /// **Breaking** in 0.18.0: takes a [`RawScalarInitInfo`], the argument type
    /// of [`ScalarInitFn`][crate::scalar::ScalarInitFn], not a bare
    /// `duckdb_init_info`.
    ///
    /// # Safety
    ///
    /// `info` must be the argument `DuckDB` passed to a scalar function init
    /// callback that is still running.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: RawScalarInitInfo) -> Self {
        Self { info: info.0 }
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
    /// A second call overwrites the pointer and destructor without running the
    /// first destructor: the first value is leaked.
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
    /// An interior NUL byte in `message` is replaced with `?`
    /// (see [`message_to_c_string`][crate::callback::message_to_c_string]),
    /// and an empty message by [`EMPTY_ERROR_PLACEHOLDER`], since `DuckDB`
    /// would otherwise report only an error-type prefix.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = crate::table::cstr::error_cstring(message, EMPTY_ERROR_PLACEHOLDER);
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
    /// The inner handle must be valid (requires `DuckDB` runtime), and the
    /// returned context must not be used after the connection running this
    /// query is closed — see
    /// [`ClientContext`](crate::client_context::ClientContext#lifetime).
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

/// Whether `duckdb_scalar_function_bind_get_argument` in the engine reporting
/// `engine` catches a failed copy of the argument. It does from v1.5.5: the
/// function body has no `try` at tags v1.5.0 to v1.5.4 and one at v1.5.5
/// (`src/main/capi/scalar_function-c.cpp`). A pre-release or development build
/// (`v1.5.5-dev42`) precedes its release, so it counts only if its base is
/// later than v1.5.5; an engine version that does not parse does not count.
#[cfg(feature = "duckdb-1-5")]
fn get_argument_catches_copy_failures(engine: &str) -> bool {
    const FIXED: (u64, u64, u64) = (1, 5, 5);
    let (base, pre_release) = engine
        .split_once('-')
        .map_or((engine, false), |(b, _)| (b, true));
    crate::abi::parse_version(base).is_some_and(|version| {
        if pre_release {
            version > FIXED
        } else {
            version >= FIXED
        }
    })
}

#[cfg(test)]
mod tests {

    /// `duckdb_scalar_function_bind_get_argument` has no `try` at `DuckDB` tags
    /// v1.5.0 to v1.5.4 and one at v1.5.5, so only v1.5.5 and later, or a
    /// development build of a later release, may be asked for an argument.
    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn only_engines_that_catch_a_failed_copy_are_asked_for_an_argument() {
        use super::get_argument_catches_copy_failures as catches;
        for engine in [
            "v1.5.5",
            "v1.5.6",
            "v1.6.0",
            "v2.0.0",
            "v1.5.6-dev42",
            "1.5.5",
        ] {
            assert!(catches(engine), "{engine}");
        }
        for engine in [
            "v1.5.0",
            "v1.5.4",
            "v1.4.4",
            "v1.5.5-dev42",
            "v1.5.5-rc1",
            "0d3cd0e22e",
            "",
        ] {
            assert!(!catches(engine), "{engine}");
        }
    }
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
        let info = unsafe { ScalarBindInfo::new(RawScalarBindInfo(raw)) };
        assert_eq!(info.as_raw(), raw);
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn scalar_init_info_as_raw_roundtrip() {
        let raw = std::ptr::null_mut();
        let info = unsafe { ScalarInitInfo::new(RawScalarInitInfo(raw)) };
        assert_eq!(info.as_raw(), raw);
    }
}
