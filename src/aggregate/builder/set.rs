// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_aggregate_function_to_set, duckdb_aggregate_function_set_destructor,
    duckdb_aggregate_function_set_extra_info, duckdb_aggregate_function_set_functions,
    duckdb_aggregate_function_set_name, duckdb_aggregate_function_set_return_type,
    duckdb_aggregate_function_set_special_handling, duckdb_connection,
    duckdb_create_aggregate_function, duckdb_create_aggregate_function_set,
    duckdb_destroy_aggregate_function, duckdb_destroy_aggregate_function_set,
    duckdb_register_aggregate_function_set, DuckDBSuccess,
};

use super::overload::{AggregateOverloadBuilder, OverloadSpec};
use crate::error::ExtensionError;
use crate::scalar::builder::signature::{merged_params, reject_duplicate_overloads};
use crate::types::logical_type::SlotCheck;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::validate::validate_function_name;

/// Builder for registering a `DuckDB` aggregate function set (multiple overloads).
///
/// Use this when one function name needs several signatures — either a variable
/// number of arguments (one overload per arity) or different parameter types.
///
/// # Known `DuckDB` limitation
///
/// Every overload is a C-API aggregate, so each reads out of bounds under
/// `agg(x) OVER ()` and `agg(x ORDER BY y)` — a `DuckDB` C API defect. See
/// [`AggregateFunctionBuilder`][crate::aggregate::AggregateFunctionBuilder#known-duckdb-limitation].
///
/// # ADR-2: Function sets for variadic signatures
///
/// `DuckDB` does not support true varargs for aggregate functions. For functions
/// that accept 2–32 boolean conditions, register 31 overloads.
///
/// # Return types are per-overload
///
/// `DuckDB` resolves an aggregate overload from its **parameter types and arity
/// only**; the return type plays no part in resolution. Each overload may
/// therefore return a different type, set with
/// [`AggregateOverloadBuilder::returns`] /
/// [`AggregateOverloadBuilder::returns_logical`]. [`returns`][Self::returns] and
/// [`returns_logical`][Self::returns_logical] on *this* builder set a **default**
/// applied to every overload that does not carry its own — convenient when all
/// arities of a variadic aggregate share one return type.
///
/// Registration fails if an overload has neither its own return type nor a
/// set-level default.
///
/// # Pitfall L6: Name must be set on each member
///
/// This builder calls `duckdb_aggregate_function_set_name` on EVERY individual
/// function before adding it to the set. If you forget this call, `DuckDB`
/// silently rejects the registration. Discovery of this bug required reading
/// `DuckDB`'s own C++ test code at
/// `test/api/capi/test_capi_aggregate_functions.cpp`.
///
/// # Examples
///
/// One return type shared by every arity — set it once on the set. Each
/// overload needs its own parameters: overloads that accept the same
/// arguments are refused at `register`.
///
/// ```rust,no_run
/// use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
/// use quack_rs::types::{LogicalType, TypeId};
/// use libduckdb_sys::{duckdb_aggregate_state, duckdb_connection, duckdb_data_chunk,
///                     duckdb_function_info, duckdb_vector, idx_t};
///
/// # unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t { 8 }
/// # unsafe extern "C" fn state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
/// # unsafe extern "C" fn update(_: duckdb_function_info, _: duckdb_data_chunk,
/// #     _: *mut duckdb_aggregate_state) {}
/// # unsafe extern "C" fn combine(_: duckdb_function_info, _: *mut duckdb_aggregate_state,
/// #     _: *mut duckdb_aggregate_state, _: idx_t) {}
/// # unsafe extern "C" fn finalize(_: duckdb_function_info, _: *mut duckdb_aggregate_state,
/// #     _: duckdb_vector, _: idx_t, _: idx_t) {}
/// # unsafe extern "C" fn destroy(_: *mut duckdb_aggregate_state, _: idx_t) {}
/// /// # Safety
/// ///
/// /// `con` must be a valid, open connection.
/// unsafe fn register_retention(
///     con: duckdb_connection,
/// ) -> Result<(), quack_rs::error::ExtensionError> {
///     // SAFETY: `con` is valid per this function's contract.
///     unsafe {
///         AggregateFunctionSetBuilder::new("retention")
///             .returns_logical(LogicalType::list(TypeId::Boolean))
///             .overloads(2..=32, |n, builder: AggregateOverloadBuilder| {
///                 (0..n)
///                     .fold(builder, |b, _| b.param(TypeId::Boolean))
///                     .state_size(state_size)
///                     .init(state_init)
///                     .update(update)
///                     .combine(combine)
///                     .finalize(finalize)
///                     .destructor(destroy)
///             })
///             .register(con)
///     }
/// }
/// ```
///
/// Different return types per overload — set them on each overload:
///
/// ```rust,no_run
/// use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
/// use quack_rs::types::TypeId;
/// # use libduckdb_sys::{duckdb_aggregate_state, duckdb_connection, duckdb_data_chunk,
/// #                     duckdb_function_info, duckdb_vector, idx_t};
/// # unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t { 8 }
/// # unsafe extern "C" fn init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
/// # unsafe extern "C" fn update(_: duckdb_function_info, _: duckdb_data_chunk,
/// #     _: *mut duckdb_aggregate_state) {}
/// # unsafe extern "C" fn combine(_: duckdb_function_info, _: *mut duckdb_aggregate_state,
/// #     _: *mut duckdb_aggregate_state, _: idx_t) {}
/// # unsafe extern "C" fn int_finalize(_: duckdb_function_info, _: *mut duckdb_aggregate_state,
/// #     _: duckdb_vector, _: idx_t, _: idx_t) {}
/// # unsafe extern "C" fn str_finalize(_: duckdb_function_info, _: *mut duckdb_aggregate_state,
/// #     _: duckdb_vector, _: idx_t, _: idx_t) {}
/// # unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// // SAFETY: `con` is a valid, open connection.
/// unsafe {
///     AggregateFunctionSetBuilder::new("my_agg")
///         .overload(
///             AggregateOverloadBuilder::new()
///                 .param(TypeId::Integer)
///                 .returns(TypeId::Integer)
///                 .state_size(state_size)
///                 .init(init)
///                 .update(update)
///                 .combine(combine)
///                 .finalize(int_finalize),
///         )
///         .overload(
///             AggregateOverloadBuilder::new()
///                 .param(TypeId::Varchar)
///                 .returns(TypeId::Varchar)
///                 .state_size(state_size)
///                 .init(init)
///                 .update(update)
///                 .combine(combine)
///                 .finalize(str_finalize),
///         )
///         .register(con)
/// }
/// # }
/// ```
#[must_use]
pub struct AggregateFunctionSetBuilder {
    pub(super) name: CString,
    pub(super) return_type: Option<TypeId>,
    pub(super) return_logical: Option<LogicalType>,
    pub(super) overloads: Vec<OverloadSpec>,
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

    /// Returns the function set name.
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }

    /// Sets the **default** return type, used by every overload that does not
    /// set its own with [`AggregateOverloadBuilder::returns`] /
    /// [`AggregateOverloadBuilder::returns_logical`].
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the **default** return type to a complex [`LogicalType`], used by
    /// every overload that does not set its own.
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

    /// Adds a single, fully-configured overload to this function set.
    ///
    /// Use this when overloads differ in more than arity — different parameter
    /// types, different return types, or different callbacks. For a family of
    /// arities that share a shape, [`overloads`][Self::overloads] is shorter.
    ///
    /// The two may be mixed; overloads are registered in the order added.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
    /// use quack_rs::types::TypeId;
    ///
    /// // AggregateFunctionSetBuilder::new("my_agg")
    /// //     .overload(
    /// //         AggregateOverloadBuilder::new()
    /// //             .param(TypeId::Integer)
    /// //             .returns(TypeId::Integer)
    /// //             .state_size(state_size)
    /// //             .init(init)
    /// //             .update(update)
    /// //             .combine(combine)
    /// //             .finalize(finalize),
    /// //     );
    /// ```
    pub fn overload(mut self, builder: AggregateOverloadBuilder) -> Self {
        self.overloads.push(builder.into_spec());
        self
    }

    /// Adds overloads for each arity in `range`, using the given builder closure.
    ///
    /// The closure receives:
    /// - `n`: the number of parameters for this overload
    /// - A fresh [`AggregateOverloadBuilder`] for configuring callbacks
    ///
    /// Each overload may set its own return type; any that does not falls back
    /// to the set-level default from [`returns`][Self::returns] /
    /// [`returns_logical`][Self::returns_logical].
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
        F: Fn(usize, AggregateOverloadBuilder) -> AggregateOverloadBuilder,
    {
        for n in range {
            self.overloads
                .push(f(n, AggregateOverloadBuilder::new()).into_spec());
        }
        self
    }

    /// The completeness checks that need no `DuckDB` call: at least one
    /// overload, each with a return type (its own or the set's) and every
    /// required callback. [`MockRegistrar`][crate::testing::MockRegistrar]
    /// runs them too.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        if self.overloads.is_empty() {
            return Err(ExtensionError::new("no overloads added to function set"));
        }
        let has_default_return = self.return_type.is_some() || self.return_logical.is_some();
        for (i, overload) in self.overloads.iter().enumerate() {
            overload.check_complete(i, has_default_return)?;
        }
        Ok(())
    }

    /// Refuses a type [`register`][Self::register] refuses before its first
    /// `DuckDB` call; `slot` checks each `TypeId` (see [`SlotCheck`]).
    pub(crate) fn check_types(&self, slot: SlotCheck) -> Result<(), ExtensionError> {
        if let Some(id) = self.return_type {
            slot(id, "aggregate function set return type")?;
        }
        for (i, overload) in self.overloads.iter().enumerate() {
            for (j, id) in overload.params.iter().enumerate() {
                slot(*id, &format!("overload {i} parameter {j}"))?;
            }
            if let Some(id) = overload.return_type {
                slot(id, &format!("overload {i} return type"))?;
            }
            // The overload's own return type, else the set-level default.
            let (id, logical) =
                if overload.return_logical.is_some() || overload.return_type.is_some() {
                    (overload.return_type, overload.return_logical.as_ref())
                } else {
                    (self.return_type, self.return_logical.as_ref())
                };
            crate::table::type_check::refuse_any_return(
                &format!("overload {i} return type"),
                id,
                logical,
            )?;
        }
        Ok(())
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
    /// - No overloads were added.
    /// - A parameter, varargs or return type was given as a bare composite
    ///   [`TypeId`] (`DECIMAL`, `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY`,
    ///   `UNION`), which carries parameters a `TypeId` cannot express. Build
    ///   it as a [`LogicalType`] and use the `*_logical` method; the error
    ///   names the slot.
    /// - An overload has neither its own return type nor a set-level default,
    ///   or is missing a required callback. The error names the overload's
    ///   index, and is reported before any `DuckDB` handle is allocated.
    /// - Two overloads accept the same call — the same argument types,
    ///   compared structurally, so `DECIMAL(18,2)` and `DECIMAL(18,3)` differ.
    ///   `DuckDB` itself would accept such a set and then fail every such call
    ///   with "Could not choose a best candidate function".
    /// - `DuckDB` reports registration failure.
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
    /// - `con` must be a valid, open `duckdb_connection`.
    /// - In each overload, the `state_size`, `init` and `destructor` callbacks,
    ///   and the ones that read the state, must describe the same state; see
    ///   [`AggregateOverloadBuilder::ffi_state`].
    #[allow(clippy::too_many_lines)]
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // Validate everything before allocating any DuckDB handle. The checks
        // that need no DuckDB call come first, so a missing callback is
        // reported by index without touching the engine.
        self.check_parts()?;
        self.check_types(LogicalType::check_slot)?;
        let signatures: Vec<_> = self
            .overloads
            .iter()
            .map(|o| merged_params(&o.params, &o.logical_params))
            .collect();
        // SAFETY: every logical parameter is a live `LogicalType` owned by this
        // builder, and the caller's contract means the C API is initialised.
        unsafe { reject_duplicate_overloads(&self.name.to_string_lossy(), &signatures)? };

        // Resolve the set-level *default* return type once, if one was given.
        // Overloads that carry their own return type never consult it.
        let default_ret_lt: Option<LogicalType> = if let Some(lt) = self.return_logical {
            Some(lt)
        } else if let Some(id) = self.return_type {
            Some(LogicalType::for_slot(
                id,
                "aggregate function set return type",
            )?)
        } else {
            None
        };

        // SAFETY: Creates a new aggregate function set handle.
        let mut set = unsafe { duckdb_create_aggregate_function_set(self.name.as_ptr()) };

        let mut register_error: Option<ExtensionError> = None;

        for (i, overload) in self.overloads.iter().enumerate() {
            // Resolve this overload's return type: its own LogicalType, then its
            // own TypeId, then the set-level default. `_ret_lt_owner` keeps a
            // `LogicalType` built from a bare `TypeId` alive until the
            // `set_return_type` call below has copied it. This `else` arm and
            // the missing-callback ones below cannot run: every overload was
            // checked before the set was created.
            let (_ret_lt_owner, ret_raw) = if let Some(ref lt) = overload.return_logical {
                (None, lt.as_raw())
            } else if let Some(id) = overload.return_type {
                let lt = LogicalType::new(id);
                let raw = lt.as_raw();
                (Some(lt), raw)
            } else if let Some(ref lt) = default_ret_lt {
                (None, lt.as_raw())
            } else {
                register_error = Some(ExtensionError::new(format!(
                    "overload {i} has no return type"
                )));
                break;
            };

            let (Some(state_size), Some(init), Some(update), Some(combine), Some(finalize)) = (
                overload.state_size,
                overload.init,
                overload.update,
                overload.combine,
                overload.finalize,
            ) else {
                register_error = Some(ExtensionError::new(format!(
                    "overload {i} is missing a required callback"
                )));
                break;
            };

            // SAFETY: Creates a new aggregate function handle for this overload.
            let mut func = unsafe { duckdb_create_aggregate_function() };

            // PITFALL L6: CRITICAL — must call this on EACH function, not just the set.
            // Without this, duckdb_register_aggregate_function_set silently returns DuckDBError.
            // Discovered by reading DuckDB's test/api/capi/test_capi_aggregate_functions.cpp.
            // SAFETY: `func` is the handle `duckdb_create_aggregate_function` just
            // returned, not yet destroyed; it is null only if allocating its info threw,
            // and `duckdb_aggregate_function_set_name` null-checks both arguments
            // (aggregate_function-c.cpp). `self.name` is a `CString`, so NUL-terminated
            // and live for the call, and DuckDB copies it into `AggregateFunction::name`.
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

            // Set this overload's return type.
            // SAFETY: func and ret_raw are valid; `_ret_lt_owner` keeps ret_raw's
            // owner alive across this call when the type was built from a TypeId.
            unsafe {
                duckdb_aggregate_function_set_return_type(func, ret_raw);
            }

            // Set callbacks
            // SAFETY: func is a valid aggregate function handle, and each
            // callback was checked to be Some above. The pointers are
            // `extern "C" fn` items with 'static lifetime, so they outlive the
            // registration.
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

            // Always register a destructor, a no-op if none was given (see
            // `callbacks::no_op_destroy`).
            let dtor = overload
                .destructor
                .unwrap_or(crate::aggregate::callbacks::no_op_destroy);
            // SAFETY: func is valid and `dtor` is a 'static `extern "C" fn`.
            unsafe {
                duckdb_aggregate_function_set_destructor(func, Some(dtor));
            }

            // Set special NULL handling if requested
            if overload.null_handling == NullHandling::SpecialNullHandling {
                // SAFETY: func is a valid aggregate function handle.
                unsafe {
                    duckdb_aggregate_function_set_special_handling(func);
                }
            }

            // Set extra info if provided
            if let Some(info) = &overload.extra_info {
                // SAFETY: func is valid; data and destroy are provided by caller.
                unsafe {
                    duckdb_aggregate_function_set_extra_info(func, info.data(), info.destroy());
                    // DuckDB owns the allocation from here: the set's copy of
                    // this function frees it, even if registration fails.
                    info.mark_transferred();
                }
            }

            // Add this function to the set
            // SAFETY: set and func are valid handles.
            unsafe {
                duckdb_add_aggregate_function_to_set(set, func);
            }

            // SAFETY: func was created above and ownership transferred to the set.
            unsafe {
                duckdb_destroy_aggregate_function(&raw mut func);
            }
        }

        if register_error.is_none() {
            // SAFETY: con is valid and set is fully configured.
            let result = unsafe { duckdb_register_aggregate_function_set(con, set) };
            if result != DuckDBSuccess {
                register_error = Some(ExtensionError::new(format!(
                    "duckdb_register_aggregate_function_set failed for '{name}': {hint}",
                    name = self.name.to_string_lossy(),
                    hint = crate::error::REGISTRATION_FAILURE_HINT
                )));
            }
        }

        // SAFETY: set was created above and must be destroyed.
        unsafe {
            duckdb_destroy_aggregate_function_set(&raw mut set);
        }

        register_error.map_or(Ok(()), Err)
    }
}

impl core::fmt::Debug for AggregateFunctionSetBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AggregateFunctionSetBuilder")
            .field("name", &self.name)
            .field("return_type", &self.return_type)
            .field("return_logical", &self.return_logical)
            .field("overloads", &self.overloads.len())
            .finish()
    }
}
