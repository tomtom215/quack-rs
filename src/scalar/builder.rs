// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` scalar functions.
//!
//! Scalar functions take a data chunk of input rows and produce one output value
//! per row. This is the most common function type in `DuckDB` extensions.

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_scalar_function_to_set, duckdb_connection, duckdb_create_scalar_function,
    duckdb_create_scalar_function_set, duckdb_data_chunk, duckdb_destroy_scalar_function,
    duckdb_destroy_scalar_function_set, duckdb_function_info, duckdb_register_scalar_function,
    duckdb_register_scalar_function_set, duckdb_scalar_function_add_parameter,
    duckdb_scalar_function_set_function, duckdb_scalar_function_set_name,
    duckdb_scalar_function_set_return_type, duckdb_scalar_function_set_special_handling,
    duckdb_vector, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
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
#[must_use]
pub struct ScalarFunctionBuilder {
    name: CString,
    params: Vec<TypeId>,
    return_type: Option<TypeId>,
    function: Option<ScalarFn>,
    null_handling: NullHandling,
}

impl ScalarFunctionBuilder {
    /// Creates a new builder for a scalar function with the given name.
    ///
    /// # Panics
    ///
    /// Panics if `name` contains an interior null byte.
    pub fn new(name: &str) -> Self {
        Self {
            name: CString::new(name).expect("function name must not contain null bytes"),
            params: Vec::new(),
            return_type: None,
            function: None,
            null_handling: NullHandling::DefaultNullHandling,
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
            null_handling: NullHandling::DefaultNullHandling,
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

    /// Sets the NULL handling behaviour for this function.
    ///
    /// By default, `DuckDB` returns NULL if any argument is NULL
    /// ([`DefaultNullHandling`][NullHandling::DefaultNullHandling]).
    /// Set to [`SpecialNullHandling`][NullHandling::SpecialNullHandling] to receive
    /// NULL values in your callback and handle them yourself.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
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

        // Set special NULL handling if requested
        if self.null_handling == NullHandling::SpecialNullHandling {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_special_handling(func);
            }
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
    name: CString,
    overloads: Vec<ScalarOverloadSpec>,
}

/// Specification for one overload within a scalar function set.
struct ScalarOverloadSpec {
    params: Vec<TypeId>,
    return_type: Option<TypeId>,
    function: Option<ScalarFn>,
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
    params: Vec<TypeId>,
    return_type: Option<TypeId>,
    function: Option<ScalarFn>,
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

    // --- ScalarFunctionSetBuilder tests ---

    #[test]
    fn set_builder_stores_name() {
        let b = ScalarFunctionSetBuilder::new("my_set");
        assert_eq!(b.name.to_str().unwrap(), "my_set");
    }

    #[test]
    fn set_builder_stores_overloads() {
        unsafe extern "C" fn f1(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
        unsafe extern "C" fn f2(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

        let b = ScalarFunctionSetBuilder::new("my_add")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Integer)
                    .param(TypeId::Integer)
                    .returns(TypeId::Integer)
                    .function(f1),
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Double)
                    .function(f2),
            );

        assert_eq!(b.overloads.len(), 2);
        assert_eq!(b.overloads[0].params.len(), 2);
        assert_eq!(b.overloads[1].params.len(), 2);
    }

    #[test]
    fn set_try_new_valid_name() {
        assert!(ScalarFunctionSetBuilder::try_new("my_add").is_ok());
    }

    #[test]
    fn set_try_new_empty_rejected() {
        assert!(ScalarFunctionSetBuilder::try_new("").is_err());
    }

    #[test]
    fn overload_builder_default() {
        let ob = ScalarOverloadBuilder::default();
        assert!(ob.params.is_empty());
        assert!(ob.return_type.is_none());
        assert!(ob.function.is_none());
    }

    #[test]
    fn overload_builder_stores_fields() {
        unsafe extern "C" fn f(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

        let ob = ScalarOverloadBuilder::new()
            .param(TypeId::BigInt)
            .returns(TypeId::Varchar)
            .function(f);
        assert_eq!(ob.params.len(), 1);
        assert_eq!(ob.return_type, Some(TypeId::Varchar));
        assert!(ob.function.is_some());
    }
}
