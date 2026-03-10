// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_scalar_function_to_set, duckdb_connection, duckdb_create_scalar_function,
    duckdb_create_scalar_function_set, duckdb_destroy_scalar_function,
    duckdb_destroy_scalar_function_set, duckdb_register_scalar_function_set,
    duckdb_scalar_function_add_parameter, duckdb_scalar_function_set_function,
    duckdb_scalar_function_set_name, duckdb_scalar_function_set_return_type,
    duckdb_scalar_function_set_special_handling, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::validate::validate_function_name;

use super::single::ScalarFn;

/// Builder for registering a `DuckDB` scalar function set (multiple overloads).
///
/// Use this when your scalar function accepts different parameter types or arities
/// by registering N overloads under a single name.
///
/// # Pitfall L6 (applies to scalar sets too)
///
/// This builder calls `duckdb_scalar_function_set_name` on EVERY individual
/// function before adding it to the set, matching the pattern established by
/// [`AggregateFunctionSetBuilder`][crate::aggregate::AggregateFunctionSetBuilder].
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::scalar::{ScalarFunctionSetBuilder, ScalarOverloadBuilder};
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_data_chunk,
///                     duckdb_vector};
///
/// unsafe extern "C" fn add_ints(
///     _: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector,
/// ) {}
/// unsafe extern "C" fn add_doubles(
///     _: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector,
/// ) {}
///
/// // fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     unsafe {
/// //         ScalarFunctionSetBuilder::new("my_add")
/// //             .overload(
/// //                 ScalarOverloadBuilder::new()
/// //                     .param(TypeId::Integer).param(TypeId::Integer)
/// //                     .returns(TypeId::Integer)
/// //                     .function(add_ints)
/// //             )
/// //             .overload(
/// //                 ScalarOverloadBuilder::new()
/// //                     .param(TypeId::Double).param(TypeId::Double)
/// //                     .returns(TypeId::Double)
/// //                     .function(add_doubles)
/// //             )
/// //             .register(con)
/// //     }
/// // }
/// ```
#[must_use]
pub struct ScalarFunctionSetBuilder {
    pub(super) name: CString,
    pub(super) overloads: Vec<ScalarOverloadSpec>,
}

/// Specification for one overload within a scalar function set.
pub(super) struct ScalarOverloadSpec {
    pub(super) params: Vec<TypeId>,
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) return_type: Option<TypeId>,
    pub(super) return_logical: Option<LogicalType>,
    pub(super) function: Option<ScalarFn>,
    pub(super) null_handling: NullHandling,
}

impl ScalarFunctionSetBuilder {
    /// Creates a new builder for a scalar function set with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn new(name: &str) -> Self {
        Self {
            name: CString::new(name).expect("function name must not contain null bytes"),
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
            overloads: Vec::new(),
        })
    }

    /// Adds a single overload to this function set.
    pub fn overload(mut self, builder: ScalarOverloadBuilder) -> Self {
        self.overloads.push(ScalarOverloadSpec {
            params: builder.params,
            logical_params: builder.logical_params,
            return_type: builder.return_type,
            return_logical: builder.return_logical,
            function: builder.function,
            null_handling: builder.null_handling,
        });
        self
    }

    /// Registers the scalar function set on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - No overloads were added.
    /// - Any overload is missing a return type or function callback.
    /// - `DuckDB` reports registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        if self.overloads.is_empty() {
            return Err(ExtensionError::new(
                "no overloads added to scalar function set",
            ));
        }

        // SAFETY: Creates a new scalar function set handle.
        let set = unsafe { duckdb_create_scalar_function_set(self.name.as_ptr()) };

        let mut register_error: Option<ExtensionError> = None;

        for overload in &self.overloads {
            // Resolve return type: prefer explicit LogicalType over TypeId.
            // `_ret_lt_owner` keeps the LogicalType alive when created from TypeId.
            let (_ret_lt_owner, ret_raw) = if let Some(ref lt) = overload.return_logical {
                (None, lt.as_raw())
            } else if let Some(id) = overload.return_type {
                let lt = LogicalType::new(id);
                let raw = lt.as_raw();
                (Some(lt), raw)
            } else {
                register_error = Some(ExtensionError::new("overload missing return type"));
                break;
            };

            let Some(function) = overload.function else {
                register_error = Some(ExtensionError::new("overload missing function callback"));
                break;
            };

            // SAFETY: Creates a new scalar function handle for this overload.
            let func = unsafe { duckdb_create_scalar_function() };

            // PITFALL L6: Must call this on EACH function, not just the set.
            unsafe {
                duckdb_scalar_function_set_name(func, self.name.as_ptr());
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
                        unsafe {
                            duckdb_scalar_function_add_parameter(
                                func,
                                overload.logical_params[logical_idx].1.as_raw(),
                            );
                        }
                        logical_idx += 1;
                    } else if simple_idx < overload.params.len() {
                        let lt = LogicalType::new(overload.params[simple_idx]);
                        unsafe {
                            duckdb_scalar_function_add_parameter(func, lt.as_raw());
                        }
                        simple_idx += 1;
                    }
                }
            }

            // Set return type
            unsafe {
                duckdb_scalar_function_set_return_type(func, ret_raw);
            }

            // Set callback
            unsafe {
                duckdb_scalar_function_set_function(func, Some(function));
            }

            // Set special NULL handling if requested
            if overload.null_handling == NullHandling::SpecialNullHandling {
                // SAFETY: func is a valid scalar function handle.
                unsafe {
                    duckdb_scalar_function_set_special_handling(func);
                }
            }

            // Add to set
            unsafe {
                duckdb_add_scalar_function_to_set(set, func);
            }

            // Destroy individual function (ownership transferred to set)
            unsafe {
                duckdb_destroy_scalar_function(&mut { func });
            }
        }

        if register_error.is_none() {
            let result = unsafe { duckdb_register_scalar_function_set(con, set) };
            if result != DuckDBSuccess {
                register_error = Some(ExtensionError::new(format!(
                    "duckdb_register_scalar_function_set failed for '{}'",
                    self.name.to_string_lossy()
                )));
            }
        }

        // SAFETY: set was created above and must be destroyed.
        unsafe {
            duckdb_destroy_scalar_function_set(&mut { set });
        }

        register_error.map_or(Ok(()), Err)
    }
}

/// A builder for one overload within a [`ScalarFunctionSetBuilder`].
#[must_use]
pub struct ScalarOverloadBuilder {
    pub(super) params: Vec<TypeId>,
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) return_type: Option<TypeId>,
    pub(super) return_logical: Option<LogicalType>,
    pub(super) function: Option<ScalarFn>,
    pub(super) null_handling: NullHandling,
}

impl ScalarOverloadBuilder {
    /// Creates a new `ScalarOverloadBuilder`.
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            logical_params: Vec::new(),
            return_type: None,
            return_logical: None,
            function: None,
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

    /// Sets the return type for this overload.
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the return type to a complex [`LogicalType`] for this overload.
    ///
    /// Use this for parameterized return types that [`TypeId`] cannot express,
    /// such as `LIST(BOOLEAN)`, `LIST(TIMESTAMP)`, `MAP(VARCHAR, INTEGER)`, etc.
    ///
    /// If both `returns` and `returns_logical` are called, the logical type takes
    /// precedence.
    pub fn returns_logical(mut self, logical_type: LogicalType) -> Self {
        self.return_logical = Some(logical_type);
        self
    }

    /// Sets the scalar function callback for this overload.
    pub fn function(mut self, f: ScalarFn) -> Self {
        self.function = Some(f);
        self
    }

    /// Sets the NULL handling behaviour for this overload.
    ///
    /// By default, `DuckDB` returns NULL if any argument is NULL
    /// ([`DefaultNullHandling`][NullHandling::DefaultNullHandling]).
    /// Set to [`SpecialNullHandling`][NullHandling::SpecialNullHandling] to receive
    /// NULL values in your callback and handle them yourself.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
        self
    }
}

impl Default for ScalarOverloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}
