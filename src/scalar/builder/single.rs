// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    duckdb_bind_info, duckdb_init_info, duckdb_scalar_function_set_bind,
    duckdb_scalar_function_set_init, duckdb_scalar_function_set_varargs,
    duckdb_scalar_function_set_volatile,
};
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_connection, duckdb_create_scalar_function, duckdb_data_chunk, duckdb_delete_callback_t,
    duckdb_destroy_scalar_function, duckdb_function_info, duckdb_register_scalar_function,
    duckdb_scalar_function_add_parameter, duckdb_scalar_function_set_extra_info,
    duckdb_scalar_function_set_function, duckdb_scalar_function_set_name,
    duckdb_scalar_function_set_return_type, duckdb_scalar_function_set_special_handling,
    duckdb_vector, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::validate::validate_function_name;

/// The scalar function bind callback signature (`DuckDB` 1.5.0+).
///
/// Called once during query planning. Use this to inspect arguments and
/// allocate per-query state via `duckdb_scalar_function_bind_set_bind_data`.
#[cfg(feature = "duckdb-1-5")]
pub type ScalarBindFn = unsafe extern "C" fn(info: duckdb_bind_info);

/// The scalar function init callback signature (`DuckDB` 1.5.0+).
///
/// Called once per thread before execution begins. Use this to allocate
/// per-thread local state via `duckdb_scalar_function_init_set_state`.
#[cfg(feature = "duckdb-1-5")]
pub type ScalarInitFn = unsafe extern "C" fn(info: duckdb_init_info);

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
    pub(super) name: CString,
    pub(super) params: Vec<TypeId>,
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) return_type: Option<TypeId>,
    pub(super) return_logical: Option<LogicalType>,
    pub(super) function: Option<ScalarFn>,
    pub(super) null_handling: NullHandling,
    pub(super) extra_info: Option<(*mut c_void, duckdb_delete_callback_t)>,
    #[cfg(feature = "duckdb-1-5")]
    pub(super) varargs: Option<LogicalType>,
    #[cfg(feature = "duckdb-1-5")]
    pub(super) volatile: bool,
    #[cfg(feature = "duckdb-1-5")]
    pub(super) bind: Option<ScalarBindFn>,
    #[cfg(feature = "duckdb-1-5")]
    pub(super) init: Option<ScalarInitFn>,
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
            logical_params: Vec::new(),
            return_type: None,
            return_logical: None,
            function: None,
            null_handling: NullHandling::DefaultNullHandling,
            extra_info: None,
            #[cfg(feature = "duckdb-1-5")]
            varargs: None,
            #[cfg(feature = "duckdb-1-5")]
            volatile: false,
            #[cfg(feature = "duckdb-1-5")]
            bind: None,
            #[cfg(feature = "duckdb-1-5")]
            init: None,
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
            logical_params: Vec::new(),
            return_type: None,
            return_logical: None,
            function: None,
            null_handling: NullHandling::DefaultNullHandling,
            extra_info: None,
            #[cfg(feature = "duckdb-1-5")]
            varargs: None,
            #[cfg(feature = "duckdb-1-5")]
            volatile: false,
            #[cfg(feature = "duckdb-1-5")]
            bind: None,
            #[cfg(feature = "duckdb-1-5")]
            init: None,
        })
    }

    /// Returns the function name.
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }

    /// Adds a positional parameter with the given type.
    ///
    /// Call this once per parameter in order. For complex types like
    /// `LIST(BIGINT)` or `MAP(VARCHAR, INTEGER)`, use [`param_logical`][Self::param_logical].
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Adds a positional parameter with a complex [`LogicalType`].
    ///
    /// Use this for parameterized types that [`TypeId`] cannot express, such as
    /// `LIST(BIGINT)`, `MAP(VARCHAR, INTEGER)`, or `STRUCT(...)`.
    ///
    /// The parameter position is determined by the total number of `param` and
    /// `param_logical` calls made so far.
    #[mutants::skip] // position arithmetic tested via E2E
    pub fn param_logical(mut self, logical_type: LogicalType) -> Self {
        let position = self.params.len() + self.logical_params.len();
        self.logical_params.push((position, logical_type));
        self
    }

    /// Sets the return type for this function.
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the return type to a complex [`LogicalType`].
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

    /// Sets the scalar function callback.
    pub fn function(mut self, f: ScalarFn) -> Self {
        self.function = Some(f);
        self
    }

    /// Marks this function as accepting variadic arguments of the given type.
    ///
    /// After the fixed positional parameters, `DuckDB` will accept any number of
    /// additional arguments that match the given type. Requires `duckdb-1-5`.
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    #[cfg(feature = "duckdb-1-5")]
    pub fn varargs(mut self, type_id: TypeId) -> Self {
        self.varargs = Some(LogicalType::new(type_id));
        self
    }

    /// Marks this function as accepting variadic arguments with a complex type.
    ///
    /// Identical to [`varargs`][Self::varargs] but accepts a [`LogicalType`]
    /// for parameterized types. Requires `duckdb-1-5`.
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    #[cfg(feature = "duckdb-1-5")]
    pub fn varargs_logical(mut self, logical_type: LogicalType) -> Self {
        self.varargs = Some(logical_type);
        self
    }

    /// Marks this function as volatile.
    ///
    /// Volatile functions are re-evaluated for every row, even when called with
    /// the same arguments (e.g. `random()`). Non-volatile functions may be
    /// optimized by `DuckDB` to only execute once for constant arguments.
    /// Requires `duckdb-1-5`.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    pub const fn volatile(mut self) -> Self {
        self.volatile = true;
        self
    }

    /// Sets a bind callback for this scalar function (`DuckDB` 1.5.0+).
    ///
    /// The bind callback is invoked once during query planning. It can inspect
    /// the function arguments and store per-query data via
    /// `duckdb_scalar_function_bind_set_bind_data`. This data can later be
    /// retrieved during execution via `duckdb_scalar_function_get_bind_data`.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    pub fn bind(mut self, f: ScalarBindFn) -> Self {
        self.bind = Some(f);
        self
    }

    /// Sets an init callback for this scalar function (`DuckDB` 1.5.0+).
    ///
    /// The init callback is invoked once per thread before execution begins.
    /// Use it to allocate per-thread local state via
    /// `duckdb_scalar_function_init_set_state`. The state pointer can later be
    /// retrieved during execution via `duckdb_scalar_function_get_state`.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    pub fn init(mut self, f: ScalarInitFn) -> Self {
        self.init = Some(f);
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

    /// Attaches arbitrary data to this scalar function.
    ///
    /// The data pointer is available inside the callback via
    /// `duckdb_function_get_extra_info`. The `destroy` callback is called by
    /// `DuckDB` when the function is dropped to free the data.
    ///
    /// # Safety
    ///
    /// `data` must point to valid memory that outlives the function registration,
    /// or will be freed by `destroy`. The typical pattern
    /// is to box your data: `Box::into_raw(Box::new(my_data)).cast()`.
    pub unsafe fn extra_info(
        mut self,
        data: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        self.extra_info = Some((data, destroy));
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
        // Reject composite TypeIds before any DuckDB handle exists, so the
        // diagnostic names the offending slot instead of surfacing as an opaque
        // `duckdb_register_scalar_function failed`. See `TypeId::is_composite`.
        for (i, id) in self.params.iter().enumerate() {
            LogicalType::check_slot(*id, &format!("scalar function parameter {i}"))?;
        }
        if let Some(id) = self.return_type {
            LogicalType::check_slot(id, "scalar function return type")?;
        }
        // Resolve return type: prefer explicit LogicalType over TypeId.
        let ret_lt = if let Some(lt) = self.return_logical {
            lt
        } else if let Some(id) = self.return_type {
            LogicalType::for_slot(id, "scalar function return type")?
        } else {
            return Err(ExtensionError::new("return type not set"));
        };

        let function = self
            .function
            .ok_or_else(|| ExtensionError::new("function callback not set"))?;

        // SAFETY: duckdb_create_scalar_function allocates a new function handle.
        let mut func = unsafe { duckdb_create_scalar_function() };

        // SAFETY: func is a valid newly created function handle.
        unsafe {
            duckdb_scalar_function_set_name(func, self.name.as_ptr());
        }

        // Add parameters: merge simple TypeId params and complex LogicalType params
        // in the order they were added (tracked by position).
        {
            let mut simple_idx = 0;
            let mut logical_idx = 0;
            let total = self.params.len() + self.logical_params.len();
            for pos in 0..total {
                if logical_idx < self.logical_params.len()
                    && self.logical_params[logical_idx].0 == pos
                {
                    // SAFETY: func and logical type handle are valid.
                    unsafe {
                        duckdb_scalar_function_add_parameter(
                            func,
                            self.logical_params[logical_idx].1.as_raw(),
                        );
                    }
                    logical_idx += 1;
                } else if simple_idx < self.params.len() {
                    let lt = LogicalType::new(self.params[simple_idx]);
                    // SAFETY: func and lt.as_raw() are valid.
                    unsafe {
                        duckdb_scalar_function_add_parameter(func, lt.as_raw());
                    }
                    simple_idx += 1;
                }
            }
        }

        // Set return type
        // SAFETY: func and ret_lt.as_raw() are valid.
        unsafe {
            duckdb_scalar_function_set_return_type(func, ret_lt.as_raw());
        }

        // Set callback
        // SAFETY: function is a valid extern "C" fn pointer.
        unsafe {
            duckdb_scalar_function_set_function(func, Some(function));
        }

        // Set extra info if provided
        if let Some((data, destroy)) = self.extra_info {
            // SAFETY: func is valid; data and destroy are provided by caller.
            unsafe {
                duckdb_scalar_function_set_extra_info(func, data, destroy);
            }
        }

        // Set bind callback if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if let Some(bind_fn) = self.bind {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_bind(func, Some(bind_fn));
            }
        }

        // Set init callback if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if let Some(init_fn) = self.init {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_init(func, Some(init_fn));
            }
        }

        // Set varargs type if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if let Some(ref varargs_type) = self.varargs {
            // SAFETY: func and varargs_type.as_raw() are valid.
            unsafe {
                duckdb_scalar_function_set_varargs(func, varargs_type.as_raw());
            }
        }

        // Set volatile flag if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if self.volatile {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_volatile(func);
            }
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
            duckdb_destroy_scalar_function(&raw mut func);
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

impl core::fmt::Debug for ScalarFunctionBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        let mut s = f.debug_struct("ScalarFunctionBuilder");
        s.field("name", &self.name)
            .field("params", &self.params)
            .field("logical_params", &self.logical_params.len())
            .field("return_type", &self.return_type)
            .field("return_logical", &self.return_logical)
            .field("function", &Callback::of(&self.function))
            .field("null_handling", &self.null_handling)
            .field("extra_info", &Callback::of(&self.extra_info));
        #[cfg(feature = "duckdb-1-5")]
        s.field("varargs", &self.varargs)
            .field("volatile", &self.volatile)
            .field("bind", &Callback::of(&self.bind))
            .field("init", &Callback::of(&self.init));
        s.finish()
    }
}
