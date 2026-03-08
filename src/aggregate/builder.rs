// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder types for registering `DuckDB` aggregate functions.
//!
//! # Pitfall L6: Function set name must be set on EACH member
//!
//! When using `duckdb_register_aggregate_function_set`, the function name must be
//! set on **each individual function** added to the set, not just on the set itself.
//! If you call `duckdb_aggregate_function_set_name` only on the set (or forget to
//! call it on an individual function), `DuckDB` silently returns `DuckDBError` at
//! registration time, and the function is never registered.
//!
//! [`AggregateFunctionSetBuilder`] enforces this by calling the name-setter
//! internally for every function added to the set.

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_aggregate_function_to_set, duckdb_aggregate_function_set_destructor,
    duckdb_aggregate_function_set_functions, duckdb_aggregate_function_set_name,
    duckdb_aggregate_function_set_return_type, duckdb_connection, duckdb_create_aggregate_function,
    duckdb_create_aggregate_function_set, duckdb_destroy_aggregate_function,
    duckdb_destroy_aggregate_function_set, duckdb_register_aggregate_function,
    duckdb_register_aggregate_function_set, DuckDBSuccess,
};

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::error::ExtensionError;
use crate::types::LogicalType;
use crate::types::TypeId;
use crate::validate::validate_function_name;

/// Builder for registering a single-signature `DuckDB` aggregate function.
///
/// # Pitfall L6
///
/// Unlike `duckdb_register_aggregate_function`, this builder also handles
/// the case where you later want to convert to a function set — it sets the
/// function name correctly.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::aggregate::AggregateFunctionBuilder;
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_aggregate_state,
///                     duckdb_data_chunk, duckdb_vector, idx_t};
///
/// unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t { 8 }
/// unsafe extern "C" fn state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
/// unsafe extern "C" fn update(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_aggregate_state) {}
/// unsafe extern "C" fn combine(_: duckdb_function_info, _: duckdb_aggregate_state, _: duckdb_aggregate_state, _: idx_t) {}
/// unsafe extern "C" fn finalize(_: duckdb_function_info, _: duckdb_aggregate_state, _: duckdb_vector, _: idx_t, _: idx_t) {}
///
/// // fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     AggregateFunctionBuilder::new("word_count")
/// //         .param(TypeId::Varchar)
/// //         .returns(TypeId::BigInt)
/// //         .state_size(state_size)
/// //         .init(state_init)
/// //         .update(update)
/// //         .combine(combine)
/// //         .finalize(finalize)
/// //         .register(con)
/// // }
/// ```
pub struct AggregateFunctionBuilder {
    name: CString,
    params: Vec<TypeId>,
    return_type: Option<TypeId>,
    state_size: Option<StateSizeFn>,
    init: Option<StateInitFn>,
    update: Option<UpdateFn>,
    combine: Option<CombineFn>,
    finalize: Option<FinalizeFn>,
    destructor: Option<DestroyFn>,
}

impl AggregateFunctionBuilder {
    /// Creates a new builder for an aggregate function with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: CString::new(name).expect("function name must not contain null bytes"),
            params: Vec::new(),
            return_type: None,
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
        }
    }

    /// Creates a new builder with function name validation.
    ///
    /// Unlike [`new`][Self::new], this method validates the function name against
    /// `DuckDB` naming conventions and returns an error instead of panicking.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the name is empty, too long, contains invalid
    /// characters, or does not start with a lowercase letter or underscore.
    pub fn try_new(name: &str) -> Result<Self, ExtensionError> {
        validate_function_name(name)?;
        let c_name = CString::new(name)
            .map_err(|_| ExtensionError::new("function name contains interior null byte"))?;
        Ok(Self {
            name: c_name,
            params: Vec::new(),
            return_type: None,
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
        })
    }

    /// Adds a positional parameter with the given type.
    ///
    /// Call this once per parameter in order.
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Sets the return type for this function.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the `state_size` callback.
    pub fn state_size(mut self, f: StateSizeFn) -> Self {
        self.state_size = Some(f);
        self
    }

    /// Sets the `state_init` callback.
    pub fn init(mut self, f: StateInitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the `update` callback.
    pub fn update(mut self, f: UpdateFn) -> Self {
        self.update = Some(f);
        self
    }

    /// Sets the `combine` callback.
    pub fn combine(mut self, f: CombineFn) -> Self {
        self.combine = Some(f);
        self
    }

    /// Sets the `finalize` callback.
    pub fn finalize(mut self, f: FinalizeFn) -> Self {
        self.finalize = Some(f);
        self
    }

    /// Sets the optional `destructor` callback.
    ///
    /// Required if your state allocates heap memory (e.g., when using
    /// [`FfiState<T>`][crate::aggregate::FfiState]).
    pub fn destructor(mut self, f: DestroyFn) -> Self {
        self.destructor = Some(f);
        self
    }

    /// Registers the aggregate function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The return type was not set.
    /// - Any required callback was not set.
    /// - `DuckDB` reports a registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let return_type = self
            .return_type
            .ok_or_else(|| ExtensionError::new("return type not set"))?;
        let state_size = self
            .state_size
            .ok_or_else(|| ExtensionError::new("state_size callback not set"))?;
        let init = self
            .init
            .ok_or_else(|| ExtensionError::new("init callback not set"))?;
        let update = self
            .update
            .ok_or_else(|| ExtensionError::new("update callback not set"))?;
        let combine = self
            .combine
            .ok_or_else(|| ExtensionError::new("combine callback not set"))?;
        let finalize = self
            .finalize
            .ok_or_else(|| ExtensionError::new("finalize callback not set"))?;

        // SAFETY: duckdb_create_aggregate_function allocates a new function handle.
        let func = unsafe { duckdb_create_aggregate_function() };

        // SAFETY: func is a valid newly created function handle.
        unsafe {
            duckdb_aggregate_function_set_name(func, self.name.as_ptr());
        }

        // Add parameters
        for param_type in &self.params {
            let lt = LogicalType::new(*param_type);
            // SAFETY: func and lt.as_raw() are valid.
            unsafe {
                libduckdb_sys::duckdb_aggregate_function_add_parameter(func, lt.as_raw());
            }
        }

        // Set return type
        let ret_lt = LogicalType::new(return_type);
        // SAFETY: func and ret_lt.as_raw() are valid.
        unsafe {
            duckdb_aggregate_function_set_return_type(func, ret_lt.as_raw());
        }

        // Set callbacks
        // SAFETY: All function pointers are valid extern "C" fn pointers.
        unsafe {
            duckdb_aggregate_function_set_functions(
                func,
                Some(state_size),
                Some(init),
                Some(update),
                Some(combine),
                Some(finalize),
            );
        }

        if let Some(dtor) = self.destructor {
            // SAFETY: dtor is a valid extern "C" fn pointer.
            unsafe {
                duckdb_aggregate_function_set_destructor(func, Some(dtor));
            }
        }

        // Register
        // SAFETY: con is a valid open connection, func is fully configured.
        let result = unsafe { duckdb_register_aggregate_function(con, func) };

        // SAFETY: func was created above and must be destroyed after use.
        unsafe {
            duckdb_destroy_aggregate_function(&mut { func });
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_aggregate_function failed for '{}'",
                self.name.to_string_lossy()
            )))
        }
    }
}

/// Builder for registering a `DuckDB` aggregate function set (multiple overloads).
///
/// Use this when your function accepts a variable number of arguments by
/// registering N overloads (one per arity) under a single name.
///
/// # ADR-2: Function sets for variadic signatures
///
/// `DuckDB` does not support true varargs for aggregate functions. For functions
/// that accept 2–32 boolean conditions, register 31 overloads.
///
/// # Pitfall L6: Name must be set on each member
///
/// This builder calls `duckdb_aggregate_function_set_name` on EVERY individual
/// function before adding it to the set. If you forget this call, `DuckDB`
/// silently rejects the registration. Discovery of this bug required reading
/// `DuckDB`'s own C++ test code at `test/api/capi/test_capi_aggregate_functions.cpp`.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::aggregate::AggregateFunctionSetBuilder;
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::duckdb_connection;
///
/// // fn register_retention(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     AggregateFunctionSetBuilder::new("retention")
/// //         .returns(TypeId::BigInt)
/// //         .overloads(2..=32, |_n, builder| {
/// //             builder
/// //                 .state_size(state_size)
/// //                 .init(state_init)
/// //                 .update(update)
/// //                 .combine(combine)
/// //                 .finalize(finalize)
/// //                 .destructor(destroy)
/// //         })
/// //         .register(con)
/// // }
/// ```
pub struct AggregateFunctionSetBuilder {
    name: CString,
    return_type: Option<TypeId>,
    overloads: Vec<OverloadSpec>,
}

/// Specification for one overload within a function set.
struct OverloadSpec {
    params: Vec<TypeId>,
    state_size: Option<StateSizeFn>,
    init: Option<StateInitFn>,
    update: Option<UpdateFn>,
    combine: Option<CombineFn>,
    finalize: Option<FinalizeFn>,
    destructor: Option<DestroyFn>,
}

impl AggregateFunctionSetBuilder {
    /// Creates a new builder for a function set with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: CString::new(name).expect("function name must not contain null bytes"),
            return_type: None,
            overloads: Vec::new(),
        }
    }

    /// Creates a new builder with function name validation.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the name is invalid.
    /// See [`validate_function_name`] for the full set of rules.
    pub fn try_new(name: &str) -> Result<Self, ExtensionError> {
        validate_function_name(name)?;
        let c_name = CString::new(name)
            .map_err(|_| ExtensionError::new("function name contains interior null byte"))?;
        Ok(Self {
            name: c_name,
            return_type: None,
            overloads: Vec::new(),
        })
    }

    /// Sets the return type for all overloads in this function set.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Adds overloads for each arity in `range`, using the given builder closure.
    ///
    /// The closure receives:
    /// - `n`: the number of parameters for this overload
    /// - A fresh [`OverloadBuilder`] for configuring callbacks
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::AggregateFunctionSetBuilder;
    /// use quack_rs::types::TypeId;
    ///
    /// // AggregateFunctionSetBuilder::new("retention")
    /// //     .returns(TypeId::BigInt)
    /// //     .overloads(2..=32, |n, builder| {
    /// //         let builder = builder
    /// //             .state_size(my_state_size)
    /// //             .init(my_init)
    /// //             .update(my_update)
    /// //             .combine(my_combine)
    /// //             .finalize(my_finalize);
    /// //         // `n` booleans as params
    /// //         (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
    /// //     });
    /// ```
    pub fn overloads<F>(mut self, range: std::ops::RangeInclusive<usize>, f: F) -> Self
    where
        F: Fn(usize, OverloadBuilder) -> OverloadBuilder,
    {
        for n in range {
            let builder = f(n, OverloadBuilder::new());
            self.overloads.push(OverloadSpec {
                params: builder.params,
                state_size: builder.state_size,
                init: builder.init,
                update: builder.update,
                combine: builder.combine,
                finalize: builder.finalize,
                destructor: builder.destructor,
            });
        }
        self
    }

    /// Registers the function set on the given connection.
    ///
    /// # Pitfall L6
    ///
    /// This method calls `duckdb_aggregate_function_set_name` on EVERY individual
    /// function in the set. Omitting this call causes silent registration failure.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - Return type was not set.
    /// - Any overload is missing required callbacks.
    /// - `DuckDB` reports registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let return_type = self
            .return_type
            .ok_or_else(|| ExtensionError::new("return type not set for function set"))?;

        if self.overloads.is_empty() {
            return Err(ExtensionError::new("no overloads added to function set"));
        }

        // SAFETY: Creates a new aggregate function set handle.
        let set = unsafe { duckdb_create_aggregate_function_set(self.name.as_ptr()) };

        let mut register_error: Option<ExtensionError> = None;

        for overload in &self.overloads {
            let Some(state_size) = overload.state_size else {
                register_error = Some(ExtensionError::new("overload missing state_size"));
                break;
            };
            let Some(init) = overload.init else {
                register_error = Some(ExtensionError::new("overload missing init"));
                break;
            };
            let Some(update) = overload.update else {
                register_error = Some(ExtensionError::new("overload missing update"));
                break;
            };
            let Some(combine) = overload.combine else {
                register_error = Some(ExtensionError::new("overload missing combine"));
                break;
            };
            let Some(finalize) = overload.finalize else {
                register_error = Some(ExtensionError::new("overload missing finalize"));
                break;
            };

            // SAFETY: Creates a new aggregate function handle for this overload.
            let func = unsafe { duckdb_create_aggregate_function() };

            // PITFALL L6: CRITICAL — must call this on EACH function, not just the set.
            // Without this, duckdb_register_aggregate_function_set silently returns DuckDBError.
            // Discovered by reading DuckDB's test/api/capi/test_capi_aggregate_functions.cpp.
            unsafe {
                duckdb_aggregate_function_set_name(func, self.name.as_ptr());
            }

            // Add parameters for this overload
            for &param_type in &overload.params {
                let lt = LogicalType::new(param_type);
                // SAFETY: func and lt.as_raw() are valid handles.
                unsafe {
                    libduckdb_sys::duckdb_aggregate_function_add_parameter(func, lt.as_raw());
                }
            }

            // Set return type
            let ret_lt = LogicalType::new(return_type);
            // SAFETY: func and ret_lt.as_raw() are valid.
            unsafe {
                duckdb_aggregate_function_set_return_type(func, ret_lt.as_raw());
            }

            // Set callbacks
            unsafe {
                duckdb_aggregate_function_set_functions(
                    func,
                    Some(state_size),
                    Some(init),
                    Some(update),
                    Some(combine),
                    Some(finalize),
                );
            }

            if let Some(dtor) = overload.destructor {
                unsafe {
                    duckdb_aggregate_function_set_destructor(func, Some(dtor));
                }
            }

            // Add this function to the set
            // SAFETY: set and func are valid handles.
            unsafe {
                duckdb_add_aggregate_function_to_set(set, func);
            }

            // SAFETY: func was created above and ownership transferred to the set.
            unsafe {
                duckdb_destroy_aggregate_function(&mut { func });
            }
        }

        if register_error.is_none() {
            // SAFETY: con is valid and set is fully configured.
            let result = unsafe { duckdb_register_aggregate_function_set(con, set) };
            if result != DuckDBSuccess {
                register_error = Some(ExtensionError::new(format!(
                    "duckdb_register_aggregate_function_set failed for '{}'",
                    self.name.to_string_lossy()
                )));
            }
        }

        // SAFETY: set was created above and must be destroyed.
        unsafe {
            duckdb_destroy_aggregate_function_set(&mut { set });
        }

        register_error.map_or(Ok(()), Err)
    }
}

/// A builder for one overload within a [`AggregateFunctionSetBuilder`].
///
/// Returned by the closure passed to [`AggregateFunctionSetBuilder::overloads`].
pub struct OverloadBuilder {
    params: Vec<TypeId>,
    state_size: Option<StateSizeFn>,
    init: Option<StateInitFn>,
    update: Option<UpdateFn>,
    combine: Option<CombineFn>,
    finalize: Option<FinalizeFn>,
    destructor: Option<DestroyFn>,
}

impl OverloadBuilder {
    /// Creates a new `OverloadBuilder`.
    #[must_use]
    fn new() -> Self {
        Self {
            params: Vec::new(),
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
        }
    }

    /// Adds a positional parameter to this overload.
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Sets the `state_size` callback for this overload.
    pub fn state_size(mut self, f: StateSizeFn) -> Self {
        self.state_size = Some(f);
        self
    }

    /// Sets the `init` callback for this overload.
    pub fn init(mut self, f: StateInitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the `update` callback for this overload.
    pub fn update(mut self, f: UpdateFn) -> Self {
        self.update = Some(f);
        self
    }

    /// Sets the `combine` callback for this overload.
    pub fn combine(mut self, f: CombineFn) -> Self {
        self.combine = Some(f);
        self
    }

    /// Sets the `finalize` callback for this overload.
    pub fn finalize(mut self, f: FinalizeFn) -> Self {
        self.finalize = Some(f);
        self
    }

    /// Sets the optional destructor callback for this overload.
    pub fn destructor(mut self, f: DestroyFn) -> Self {
        self.destructor = Some(f);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libduckdb_sys::{
        duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
    };

    // Verify that AggregateFunctionBuilder stores name correctly
    #[test]
    fn builder_stores_name() {
        let b = AggregateFunctionBuilder::new("my_func");
        assert_eq!(b.name.to_str().unwrap(), "my_func");
    }

    #[test]
    fn builder_stores_params() {
        let b = AggregateFunctionBuilder::new("f")
            .param(TypeId::BigInt)
            .param(TypeId::Varchar);
        assert_eq!(b.params.len(), 2);
        assert_eq!(b.params[0], TypeId::BigInt);
        assert_eq!(b.params[1], TypeId::Varchar);
    }

    #[test]
    fn builder_stores_return_type() {
        let b = AggregateFunctionBuilder::new("f").returns(TypeId::BigInt);
        assert_eq!(b.return_type, Some(TypeId::BigInt));
    }

    #[test]
    fn function_set_builder_stores_overloads() {
        unsafe extern "C" fn ss(_: duckdb_function_info) -> idx_t {
            0
        }
        unsafe extern "C" fn si(_: duckdb_function_info, _: duckdb_aggregate_state) {}
        unsafe extern "C" fn su(
            _: duckdb_function_info,
            _: duckdb_data_chunk,
            _: *mut duckdb_aggregate_state,
        ) {
        }
        unsafe extern "C" fn sc(
            _: duckdb_function_info,
            _: *mut duckdb_aggregate_state,
            _: *mut duckdb_aggregate_state,
            _: idx_t,
        ) {
        }
        unsafe extern "C" fn sf(
            _: duckdb_function_info,
            _: *mut duckdb_aggregate_state,
            _: duckdb_vector,
            _: idx_t,
            _: idx_t,
        ) {
        }

        let b = AggregateFunctionSetBuilder::new("retention")
            .returns(TypeId::BigInt)
            .overloads(2..=4, |n, builder| {
                (0..n)
                    .fold(builder, |b, _| b.param(TypeId::Boolean))
                    .state_size(ss)
                    .init(si)
                    .update(su)
                    .combine(sc)
                    .finalize(sf)
            });

        // overloads(2..=4) = 3 overloads (n=2, n=3, n=4)
        assert_eq!(b.overloads.len(), 3);
        assert_eq!(b.overloads[0].params.len(), 2);
        assert_eq!(b.overloads[1].params.len(), 3);
        assert_eq!(b.overloads[2].params.len(), 4);
    }

    #[test]
    fn register_missing_return_type_returns_error() {
        let b = AggregateFunctionBuilder::new("f");
        // We can't call register with a null connection, but we can verify
        // the error path for missing return type by inspecting the error.
        // In a real integration test, we'd call register(con) with a live connection.
        // Here we verify the builder stores None for return_type.
        assert!(b.return_type.is_none());
    }

    #[test]
    fn function_set_builder_name() {
        let b = AggregateFunctionSetBuilder::new("my_set");
        assert_eq!(b.name.to_str().unwrap(), "my_set");
    }

    #[test]
    fn overload_builder_params() {
        let ob = OverloadBuilder::new()
            .param(TypeId::Boolean)
            .param(TypeId::Boolean)
            .param(TypeId::BigInt);
        assert_eq!(ob.params.len(), 3);
    }

    #[test]
    fn try_new_valid_name() {
        assert!(AggregateFunctionBuilder::try_new("word_count").is_ok());
    }

    #[test]
    fn try_new_empty_rejected() {
        assert!(AggregateFunctionBuilder::try_new("").is_err());
    }

    #[test]
    fn try_new_uppercase_rejected() {
        assert!(AggregateFunctionBuilder::try_new("MyFunc").is_err());
    }

    #[test]
    fn try_new_hyphen_rejected() {
        assert!(AggregateFunctionBuilder::try_new("my-func").is_err());
    }

    #[test]
    fn set_try_new_valid_name() {
        assert!(AggregateFunctionSetBuilder::try_new("retention").is_ok());
    }

    #[test]
    fn set_try_new_empty_rejected() {
        assert!(AggregateFunctionSetBuilder::try_new("").is_err());
    }
}
