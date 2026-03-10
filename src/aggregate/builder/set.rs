// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_aggregate_function_to_set, duckdb_aggregate_function_set_destructor,
    duckdb_aggregate_function_set_functions, duckdb_aggregate_function_set_name,
    duckdb_aggregate_function_set_return_type, duckdb_aggregate_function_set_special_handling,
    duckdb_connection, duckdb_create_aggregate_function, duckdb_create_aggregate_function_set,
    duckdb_destroy_aggregate_function, duckdb_destroy_aggregate_function_set,
    duckdb_register_aggregate_function_set, DuckDBSuccess,
};

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
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
/// use quack_rs::types::{LogicalType, TypeId};
/// use libduckdb_sys::duckdb_connection;
///
/// // fn register_retention(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     AggregateFunctionSetBuilder::new("retention")
/// //         .returns_logical(LogicalType::list(TypeId::Boolean))
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
    pub(super) return_logical: Option<LogicalType>,
    pub(super) overloads: Vec<OverloadSpec>,
}

/// Specification for one overload within a function set.
pub(super) struct OverloadSpec {
    pub(super) params: Vec<TypeId>,
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) state_size: Option<StateSizeFn>,
    pub(super) init: Option<StateInitFn>,
    pub(super) update: Option<UpdateFn>,
    pub(super) combine: Option<CombineFn>,
    pub(super) finalize: Option<FinalizeFn>,
    pub(super) destructor: Option<DestroyFn>,
    pub(super) null_handling: NullHandling,
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
            return_logical: None,
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
            return_logical: None,
            overloads: Vec::new(),
        })
    }

    /// Sets the return type for all overloads in this function set.
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the return type to a complex [`LogicalType`] for all overloads.
    ///
    /// Use this for parameterized return types that [`TypeId`] cannot express,
    /// such as `LIST(BOOLEAN)`, `LIST(TIMESTAMP)`, `MAP(VARCHAR, INTEGER)`, etc.
    ///
    /// If both `returns` and `returns_logical` are called, the logical type takes
    /// precedence.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::AggregateFunctionSetBuilder;
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // AggregateFunctionSetBuilder::new("retention")
    /// //     .returns_logical(LogicalType::list(TypeId::Boolean))
    /// //     .overloads(2..=32, |n, builder| {
    /// //         (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
    /// //             .state_size(my_state_size)
    /// //             .init(my_init)
    /// //             .update(my_update)
    /// //             .combine(my_combine)
    /// //             .finalize(my_finalize)
    /// //     });
    /// ```
    pub fn returns_logical(mut self, logical_type: LogicalType) -> Self {
        self.return_logical = Some(logical_type);
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
                logical_params: builder.logical_params,
                state_size: builder.state_size,
                init: builder.init,
                update: builder.update,
                combine: builder.combine,
                finalize: builder.finalize,
                destructor: builder.destructor,
                null_handling: builder.null_handling,
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
    #[allow(clippy::too_many_lines)]
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // Resolve return type: prefer explicit LogicalType over TypeId.
        let ret_lt = if let Some(lt) = self.return_logical {
            lt
        } else if let Some(id) = self.return_type {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("return type not set for function set"));
        };

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

            // Add parameters: merge simple TypeId params and complex LogicalType params
            // in the order they were added (tracked by position).
            {
                let mut simple_idx = 0;
                let mut logical_idx = 0;
                let total = overload.params.len() + overload.logical_params.len();
                for pos in 0..total {
                    if logical_idx < overload.logical_params.len()
                        && overload.logical_params[logical_idx].0 == pos
                    {
                        // SAFETY: func and logical type handle are valid.
                        unsafe {
                            libduckdb_sys::duckdb_aggregate_function_add_parameter(
                                func,
                                overload.logical_params[logical_idx].1.as_raw(),
                            );
                        }
                        logical_idx += 1;
                    } else if simple_idx < overload.params.len() {
                        let lt = LogicalType::new(overload.params[simple_idx]);
                        // SAFETY: func and lt.as_raw() are valid handles.
                        unsafe {
                            libduckdb_sys::duckdb_aggregate_function_add_parameter(
                                func,
                                lt.as_raw(),
                            );
                        }
                        simple_idx += 1;
                    }
                }
            }

            // Set return type (shared across all overloads)
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

            // Set special NULL handling if requested
            if overload.null_handling == NullHandling::SpecialNullHandling {
                // SAFETY: func is a valid aggregate function handle.
                unsafe {
                    duckdb_aggregate_function_set_special_handling(func);
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
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) state_size: Option<StateSizeFn>,
    pub(super) init: Option<StateInitFn>,
    pub(super) update: Option<UpdateFn>,
    pub(super) combine: Option<CombineFn>,
    pub(super) finalize: Option<FinalizeFn>,
    pub(super) destructor: Option<DestroyFn>,
    pub(super) null_handling: NullHandling,
}

impl OverloadBuilder {
    /// Creates a new `OverloadBuilder`.
    pub(super) fn new() -> Self {
        Self {
            params: Vec::new(),
            logical_params: Vec::new(),
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
            null_handling: NullHandling::DefaultNullHandling,
        }
    }

    /// Adds a positional parameter to this overload.
    ///
    /// For complex types like `LIST(BIGINT)`, use
    /// [`param_logical`][Self::param_logical].
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Adds a positional parameter with a complex [`LogicalType`].
    ///
    /// Use this for parameterized types that [`TypeId`] cannot express, such as
    /// `LIST(BIGINT)`, `MAP(VARCHAR, INTEGER)`, or `STRUCT(...)`.
    pub fn param_logical(mut self, logical_type: LogicalType) -> Self {
        let position = self.params.len() + self.logical_params.len();
        self.logical_params.push((position, logical_type));
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

    /// Sets the NULL handling behaviour for this overload.
    ///
    /// By default, `DuckDB` skips NULL rows in aggregate functions
    /// ([`DefaultNullHandling`][NullHandling::DefaultNullHandling]).
    /// Set to [`SpecialNullHandling`][NullHandling::SpecialNullHandling] to receive
    /// NULL values in your `update` callback.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
        self
    }
}
