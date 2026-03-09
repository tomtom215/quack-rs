// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_aggregate_function_to_set, duckdb_aggregate_function_set_destructor,
    duckdb_aggregate_function_set_functions, duckdb_aggregate_function_set_name,
    duckdb_aggregate_function_set_return_type, duckdb_connection, duckdb_create_aggregate_function,
    duckdb_create_aggregate_function_set, duckdb_destroy_aggregate_function,
    duckdb_destroy_aggregate_function_set, duckdb_register_aggregate_function_set, DuckDBSuccess,
};

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};
use crate::validate::validate_function_name;

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
#[must_use]
pub struct AggregateFunctionSetBuilder {
    pub(super) name: CString,
    pub(super) return_type: Option<TypeId>,
    pub(super) overloads: Vec<OverloadSpec>,
}

/// Specification for one overload within a function set.
pub(super) struct OverloadSpec {
    pub(super) params: Vec<TypeId>,
    pub(super) state_size: Option<StateSizeFn>,
    pub(super) init: Option<StateInitFn>,
    pub(super) update: Option<UpdateFn>,
    pub(super) combine: Option<CombineFn>,
    pub(super) finalize: Option<FinalizeFn>,
    pub(super) destructor: Option<DestroyFn>,
}

impl AggregateFunctionSetBuilder {
    /// Creates a new builder for a function set with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
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
#[must_use]
pub struct OverloadBuilder {
    pub(super) params: Vec<TypeId>,
    pub(super) state_size: Option<StateSizeFn>,
    pub(super) init: Option<StateInitFn>,
    pub(super) update: Option<UpdateFn>,
    pub(super) combine: Option<CombineFn>,
    pub(super) finalize: Option<FinalizeFn>,
    pub(super) destructor: Option<DestroyFn>,
}

impl OverloadBuilder {
    /// Creates a new `OverloadBuilder`.
    pub(super) fn new() -> Self {
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
