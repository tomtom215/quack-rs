// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` scalar functions.
//!
//! Scalar functions (also called "table functions" in some contexts) take a data
//! chunk of input rows and produce one output value per row. This is the most
//! common function type in `DuckDB` extensions.

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_connection, duckdb_create_scalar_function, duckdb_data_chunk,
    duckdb_destroy_scalar_function, duckdb_function_info, duckdb_register_scalar_function,
    duckdb_scalar_function_add_parameter, duckdb_scalar_function_set_function,
    duckdb_scalar_function_set_name, duckdb_scalar_function_set_return_type, duckdb_vector,
    DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};
use crate::validate::validate_function_name;

/// The scalar function callback signature.
///
/// This function is called once per data chunk. It receives:
/// - `info`: Function metadata (use for extra data or error reporting)
/// - `input`: The input data chunk containing all parameter columns
/// - `output`: The output vector to write results into
pub type ScalarFn = unsafe extern "C" fn(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
);

/// Builder for registering a single `DuckDB` scalar function.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::scalar::ScalarFunctionBuilder;
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_data_chunk,
///                     duckdb_vector};
///
/// unsafe extern "C" fn double_it(
///     _info: duckdb_function_info,
///     _input: duckdb_data_chunk,
///     _output: duckdb_vector,
/// ) {
///     // Read from input, write doubled values to output
/// }
///
/// // fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     unsafe {
/// //         ScalarFunctionBuilder::new("double_it")
/// //             .param(TypeId::BigInt)
/// //             .returns(TypeId::BigInt)
/// //             .function(double_it)
/// //             .register(con)
/// //     }
/// // }
/// ```
pub struct ScalarFunctionBuilder {
    name: CString,
    params: Vec<TypeId>,
    return_type: Option<TypeId>,
    function: Option<ScalarFn>,
}

impl ScalarFunctionBuilder {
    /// Creates a new builder for a scalar function with the given name.
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
            function: None,
        }
    }

    /// Creates a new builder with function name validation.
    ///
    /// Unlike [`new`][Self::new], this method validates the function name against
    /// `DuckDB` naming conventions and returns an error instead of panicking.
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
            params: Vec::new(),
            return_type: None,
            function: None,
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

    /// Sets the scalar function callback.
    pub fn function(mut self, f: ScalarFn) -> Self {
        self.function = Some(f);
        self
    }

    /// Registers the scalar function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The return type was not set.
    /// - The function callback was not set.
    /// - `DuckDB` reports a registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let return_type = self
            .return_type
            .ok_or_else(|| ExtensionError::new("return type not set"))?;
        let function = self
            .function
            .ok_or_else(|| ExtensionError::new("function callback not set"))?;

        // SAFETY: duckdb_create_scalar_function allocates a new function handle.
        let func = unsafe { duckdb_create_scalar_function() };

        // SAFETY: func is a valid newly created function handle.
        unsafe {
            duckdb_scalar_function_set_name(func, self.name.as_ptr());
        }

        // Add parameters
        for param_type in &self.params {
            let lt = LogicalType::new(*param_type);
            // SAFETY: func and lt.as_raw() are valid.
            unsafe {
                duckdb_scalar_function_add_parameter(func, lt.as_raw());
            }
        }

        // Set return type
        let ret_lt = LogicalType::new(return_type);
        // SAFETY: func and ret_lt.as_raw() are valid.
        unsafe {
            duckdb_scalar_function_set_return_type(func, ret_lt.as_raw());
        }

        // Set callback
        // SAFETY: function is a valid extern "C" fn pointer.
        unsafe {
            duckdb_scalar_function_set_function(func, Some(function));
        }

        // Register
        // SAFETY: con is a valid open connection, func is fully configured.
        let result = unsafe { duckdb_register_scalar_function(con, func) };

        // SAFETY: func was created above and must be destroyed after use.
        unsafe {
            duckdb_destroy_scalar_function(&mut { func });
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_scalar_function failed for '{}'",
                self.name.to_string_lossy()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_stores_name() {
        let b = ScalarFunctionBuilder::new("my_scalar");
        assert_eq!(b.name.to_str().unwrap(), "my_scalar");
    }

    #[test]
    fn builder_stores_params() {
        let b = ScalarFunctionBuilder::new("f")
            .param(TypeId::BigInt)
            .param(TypeId::Varchar);
        assert_eq!(b.params.len(), 2);
        assert_eq!(b.params[0], TypeId::BigInt);
        assert_eq!(b.params[1], TypeId::Varchar);
    }

    #[test]
    fn builder_stores_return_type() {
        let b = ScalarFunctionBuilder::new("f").returns(TypeId::Double);
        assert_eq!(b.return_type, Some(TypeId::Double));
    }

    #[test]
    fn builder_missing_return_type() {
        let b = ScalarFunctionBuilder::new("f");
        assert!(b.return_type.is_none());
    }

    #[test]
    fn builder_missing_function() {
        let b = ScalarFunctionBuilder::new("f");
        assert!(b.function.is_none());
    }

    #[test]
    fn builder_stores_function() {
        unsafe extern "C" fn my_func(
            _: duckdb_function_info,
            _: duckdb_data_chunk,
            _: duckdb_vector,
        ) {
        }

        let b = ScalarFunctionBuilder::new("f").function(my_func);
        assert!(b.function.is_some());
    }

    #[test]
    fn try_new_valid_name() {
        let b = ScalarFunctionBuilder::try_new("word_count");
        assert!(b.is_ok());
    }

    #[test]
    fn try_new_empty_rejected() {
        assert!(ScalarFunctionBuilder::try_new("").is_err());
    }

    #[test]
    fn try_new_uppercase_rejected() {
        assert!(ScalarFunctionBuilder::try_new("MyFunc").is_err());
    }

    #[test]
    fn try_new_hyphen_rejected() {
        assert!(ScalarFunctionBuilder::try_new("my-func").is_err());
    }
}
