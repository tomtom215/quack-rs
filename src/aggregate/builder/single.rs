// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_aggregate_function_set_destructor, duckdb_aggregate_function_set_functions,
    duckdb_aggregate_function_set_name, duckdb_aggregate_function_set_return_type,
    duckdb_aggregate_function_set_special_handling, duckdb_connection,
    duckdb_create_aggregate_function, duckdb_destroy_aggregate_function,
    duckdb_register_aggregate_function, DuckDBSuccess,
};

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
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
#[must_use]
pub struct AggregateFunctionBuilder {
    pub(super) name: CString,
    pub(super) params: Vec<TypeId>,
    pub(super) logical_params: Vec<(usize, LogicalType)>,
    pub(super) return_type: Option<TypeId>,
    pub(super) return_logical: Option<LogicalType>,
    pub(super) state_size: Option<StateSizeFn>,
    pub(super) init: Option<StateInitFn>,
    pub(super) update: Option<UpdateFn>,
    pub(super) combine: Option<CombineFn>,
    pub(super) finalize: Option<FinalizeFn>,
    pub(super) destructor: Option<DestroyFn>,
    pub(super) null_handling: NullHandling,
}

impl AggregateFunctionBuilder {
    /// Creates a new builder for an aggregate function with the given name.
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
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
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
    /// Returns `ExtensionError` if the name is empty, too long, contains invalid
    /// characters, or does not start with a lowercase letter or underscore.
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
            state_size: None,
            init: None,
            update: None,
            combine: None,
            finalize: None,
            destructor: None,
            null_handling: NullHandling::DefaultNullHandling,
        })
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::AggregateFunctionBuilder;
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
    /// //     AggregateFunctionBuilder::new("my_func")
    /// //         .param(TypeId::Varchar)
    /// //         .param_logical(LogicalType::list(TypeId::BigInt))
    /// //         .returns(TypeId::BigInt)
    /// //         // ... callbacks ...
    /// //         ;
    /// //     Ok(())
    /// // }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::AggregateFunctionBuilder;
    /// use quack_rs::types::{LogicalType, TypeId};
    ///
    /// // fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
    /// //     AggregateFunctionBuilder::new("retention")
    /// //         .param(TypeId::Boolean)
    /// //         .param(TypeId::Boolean)
    /// //         .returns_logical(LogicalType::list(TypeId::Boolean))
    /// //         // ... callbacks ...
    /// //         ;
    /// //     Ok(())
    /// // }
    /// ```
    pub fn returns_logical(mut self, logical_type: LogicalType) -> Self {
        self.return_logical = Some(logical_type);
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

    /// Sets the NULL handling behaviour for this aggregate function.
    ///
    /// By default, `DuckDB` skips NULL rows in aggregate functions
    /// ([`DefaultNullHandling`][NullHandling::DefaultNullHandling]).
    /// Set to [`SpecialNullHandling`][NullHandling::SpecialNullHandling] to receive
    /// NULL values in your `update` callback.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
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
        // Resolve return type: prefer explicit LogicalType over TypeId.
        let ret_lt = if let Some(lt) = self.return_logical {
            lt
        } else if let Some(id) = self.return_type {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("return type not set"));
        };

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
                        libduckdb_sys::duckdb_aggregate_function_add_parameter(
                            func,
                            self.logical_params[logical_idx].1.as_raw(),
                        );
                    }
                    logical_idx += 1;
                } else if simple_idx < self.params.len() {
                    let lt = LogicalType::new(self.params[simple_idx]);
                    // SAFETY: func and lt.as_raw() are valid.
                    unsafe {
                        libduckdb_sys::duckdb_aggregate_function_add_parameter(func, lt.as_raw());
                    }
                    simple_idx += 1;
                }
            }
        }

        // Set return type
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

        // Set special NULL handling if requested
        if self.null_handling == NullHandling::SpecialNullHandling {
            // SAFETY: func is a valid aggregate function handle.
            unsafe {
                duckdb_aggregate_function_set_special_handling(func);
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
