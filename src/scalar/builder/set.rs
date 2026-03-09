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
    duckdb_scalar_function_set_name, duckdb_scalar_function_set_return_type, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};
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
    pub(super) return_type: Option<TypeId>,
    pub(super) function: Option<ScalarFn>,
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
            return_type: builder.return_type,
            function: builder.function,
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
            let Some(return_type) = overload.return_type else {
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

            // Add parameters
            for param_type in &overload.params {
                let lt = LogicalType::new(*param_type);
                unsafe {
                    duckdb_scalar_function_add_parameter(func, lt.as_raw());
                }
            }

            // Set return type
            let ret_lt = LogicalType::new(return_type);
            unsafe {
                duckdb_scalar_function_set_return_type(func, ret_lt.as_raw());
            }

            // Set callback
            unsafe {
                duckdb_scalar_function_set_function(func, Some(function));
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
    pub(super) return_type: Option<TypeId>,
    pub(super) function: Option<ScalarFn>,
}

impl ScalarOverloadBuilder {
    /// Creates a new `ScalarOverloadBuilder`.
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            return_type: None,
            function: None,
        }
    }

    /// Adds a positional parameter to this overload.
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Sets the return type for this overload.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the scalar function callback for this overload.
    pub fn function(mut self, f: ScalarFn) -> Self {
        self.function = Some(f);
        self
    }
}

impl Default for ScalarOverloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}
