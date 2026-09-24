// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! [`AggregateFunctionBuilder::register`].

use libduckdb_sys::{
    duckdb_aggregate_function_set_destructor, duckdb_aggregate_function_set_extra_info,
    duckdb_aggregate_function_set_functions, duckdb_aggregate_function_set_name,
    duckdb_aggregate_function_set_return_type, duckdb_aggregate_function_set_special_handling,
    duckdb_connection, duckdb_create_aggregate_function, duckdb_destroy_aggregate_function,
    duckdb_register_aggregate_function, DuckDBSuccess,
};

use super::AggregateFunctionBuilder;
use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling};

impl AggregateFunctionBuilder {
    /// Registers the aggregate function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The return type was not set.
    /// - Any required callback was not set.
    /// - A parameter, varargs or return type was given as a bare composite
    ///   [`TypeId`][crate::types::TypeId] (`DECIMAL`, `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY`,
    ///   `UNION`), which carries parameters a `TypeId` cannot express. Build
    ///   it as a [`LogicalType`][crate::types::LogicalType] and use the `*_logical` method; the error
    ///   names the slot.
    /// - `DuckDB` reports a registration failure.
    ///
    /// # Name collisions
    ///
    /// An aggregate can neither extend nor replace an existing catalog entry:
    /// registration fails if the name is already taken by any scalar function,
    /// aggregate function or macro, built-in or not — including an earlier
    /// registration of this same aggregate. (`DuckDB` registers with
    /// `ALTER_ON_CONFLICT`, and turning the create into an alter is not
    /// implemented for aggregates: `CreateInfo::GetAlterInfo` throws.)
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    #[allow(clippy::too_many_lines)]
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // See `ScalarFunctionBuilder::register` -- validate before allocating.
        self.check_parts()?;
        for (i, id) in self.params.iter().enumerate() {
            LogicalType::check_slot(*id, &format!("aggregate function parameter {i}"))?;
        }
        if let Some(id) = self.return_type {
            LogicalType::check_slot(id, "aggregate function return type")?;
        }
        crate::table::type_check::refuse_any_return(
            "aggregate function return type",
            self.return_type,
            self.return_logical.as_ref(),
        )?;
        // Resolve return type: prefer explicit LogicalType over TypeId.
        let ret_lt = if let Some(lt) = self.return_logical {
            lt
        } else if let Some(id) = self.return_type {
            LogicalType::for_slot(id, "aggregate function return type")?
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
        let mut func = unsafe { duckdb_create_aggregate_function() };

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

        // Always register a destructor, a no-op if none was given: without one
        // DuckDB streams running-frame windows through a path that gives C API
        // aggregates wrong answers (see `callbacks::no_op_destroy`).
        let dtor = self
            .destructor
            .unwrap_or(crate::aggregate::callbacks::no_op_destroy);
        // SAFETY: dtor is a valid extern "C" fn pointer.
        unsafe {
            duckdb_aggregate_function_set_destructor(func, Some(dtor));
        }

        // Set special NULL handling if requested
        if self.null_handling == NullHandling::SpecialNullHandling {
            // SAFETY: func is a valid aggregate function handle.
            unsafe {
                duckdb_aggregate_function_set_special_handling(func);
            }
        }

        // Set extra info if provided
        if let Some(info) = self.extra_info {
            // SAFETY: func is valid; data and destroy are provided by caller.
            unsafe {
                duckdb_aggregate_function_set_extra_info(func, info.data(), info.destroy());
                // DuckDB owns the allocation from here.
                info.mark_transferred();
            }
        }

        // Register
        // SAFETY: con is a valid open connection, func is fully configured.
        let result = unsafe { duckdb_register_aggregate_function(con, func) };

        // SAFETY: func was created above and must be destroyed after use.
        unsafe {
            duckdb_destroy_aggregate_function(&raw mut func);
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_aggregate_function failed for '{name}': {hint}",
                name = self.name.to_string_lossy(),
                hint = crate::error::REGISTRATION_FAILURE_HINT
            )))
        }
    }
}
