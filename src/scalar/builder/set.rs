// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;

use libduckdb_sys::{
    duckdb_add_scalar_function_to_set, duckdb_connection, duckdb_create_scalar_function,
    duckdb_create_scalar_function_set, duckdb_destroy_scalar_function,
    duckdb_destroy_scalar_function_set, duckdb_register_scalar_function_set,
    duckdb_scalar_function_add_parameter, duckdb_scalar_function_set_extra_info,
    duckdb_scalar_function_set_function, duckdb_scalar_function_set_name,
    duckdb_scalar_function_set_return_type, duckdb_scalar_function_set_special_handling,
    duckdb_scalar_function_set_volatile, DuckDBSuccess,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{duckdb_scalar_function_set_bind, duckdb_scalar_function_set_init};

use crate::error::ExtensionError;
use crate::types::logical_type::SlotCheck;
use crate::types::{LogicalType, NullHandling};
use crate::validate::validate_function_name;

use super::overload::{ScalarOverloadBuilder, ScalarOverloadSpec};
use super::signature::{merged_params, reject_duplicate_overloads};

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

    /// Returns the function set name.
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("")
    }

    /// Adds a single overload to this function set.
    pub fn overload(mut self, builder: ScalarOverloadBuilder) -> Self {
        self.overloads.push(builder.into_spec());
        self
    }

    /// Registers the scalar function set on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - No overloads were added.
    /// - A parameter, varargs or return type was given as a bare composite
    ///   [`TypeId`][crate::types::TypeId] (`DECIMAL`, `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY`,
    ///   `UNION`), which carries parameters a `TypeId` cannot express. Build
    ///   it as a [`LogicalType`][crate::types::LogicalType] and use the `*_logical` method; the error
    ///   names the slot.
    /// - Any overload is missing a return type or function callback. The error
    ///   names the overload's index, and is reported before any `DuckDB`
    ///   handle is allocated.
    /// - Two overloads accept the same call: the same argument types
    ///   (compared structurally, so `DECIMAL(18,2)` and `DECIMAL(18,3)`
    ///   differ), or, with varargs, the same types at some argument count
    ///   (`f(BIGINT)` and `f(BIGINT, BIGINT...)` both take `f(1)`). `DuckDB`
    ///   itself would accept such a set and then fail every such call with
    ///   "Could not choose a best candidate function".
    /// - An overload accepts the same call as a scalar function that already
    ///   exists (see "Name collisions").
    /// - `DuckDB` reports registration failure.
    ///
    /// # Name collisions
    ///
    /// `DuckDB` registers scalar functions with `ALTER_ON_CONFLICT`
    /// (`duckdb_register_scalar_function_set` in `scalar_function-c.cpp`), so a
    /// **scalar** function that already has this name — built-in or not — never
    /// makes `DuckDB` refuse the registration. A new signature is added as an
    /// overload. One with the same parameter types as an existing overload
    /// either **silently replaces it** for every connection to the database
    /// (same return type: `FunctionSet::MergeFunctionSet` with
    /// `override = true`) or makes every call ambiguous ("Could not choose a
    /// best candidate function", different return type). Registering
    /// `abs(BIGINT)` would change the built-in `abs` for `BIGINT`.
    ///
    /// quack-rs therefore refuses an overload, naming its index, before
    /// registering, when `duckdb_functions()` already lists a scalar with this
    /// name that accepts the same call — the rule and its coverage are those of
    /// [`ScalarFunctionBuilder::register`][super::ScalarFunctionBuilder::register],
    /// including its cost: one catalog scan per call, or one per extension load
    /// through the entry point's [`Connection`](crate::connection::Connection).
    /// `DuckDB` itself refuses a name that belongs to an aggregate function or
    /// a macro (such as the built-in `list_sum`).
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // SAFETY: forwarded from this function's own contract.
        unsafe { self.register_with(con, None) }
    }

    /// The completeness checks that need no `DuckDB` call: at least one
    /// overload, each with a return type and a function callback.
    /// [`MockRegistrar`][crate::testing::MockRegistrar] runs them too.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        if self.overloads.is_empty() {
            return Err(ExtensionError::new(
                "no overloads added to scalar function set",
            ));
        }
        for (i, overload) in self.overloads.iter().enumerate() {
            overload.check_complete(i)?;
        }
        Ok(())
    }

    /// Refuses a type [`register`][Self::register] refuses before its first
    /// `DuckDB` call; `slot` checks each `TypeId` (see [`SlotCheck`]).
    pub(crate) fn check_types(&self, slot: SlotCheck) -> Result<(), ExtensionError> {
        for (i, overload) in self.overloads.iter().enumerate() {
            crate::table::type_check::refuse_any_return(
                &format!("overload {i} return type"),
                overload.return_type,
                overload.return_logical.as_ref(),
            )?;
        }
        for (i, overload) in self.overloads.iter().enumerate() {
            for (j, id) in overload.params.iter().enumerate() {
                slot(*id, &format!("overload {i} parameter {j}"))?;
            }
            if let Some(id) = overload.return_type {
                slot(id, &format!("overload {i} return type"))?;
            }
            if let Some(ref varargs) = overload.varargs {
                varargs.check(slot, &format!("overload {i} varargs"))?;
            }
        }
        Ok(())
    }

    /// [`register`][Self::register], checking signatures against `snapshot`
    /// (listed on first use) instead of listing the catalog for this call.
    ///
    /// # Safety
    ///
    /// As [`register`][Self::register].
    #[allow(clippy::too_many_lines)]
    pub(crate) unsafe fn register_with(
        self,
        con: duckdb_connection,
        snapshot: Option<&std::cell::RefCell<Option<super::collision::ExistingScalars>>>,
    ) -> Result<(), ExtensionError> {
        // Validate everything before allocating any DuckDB handle. The checks
        // that need no DuckDB call come first, so a missing callback is
        // reported by index without touching the engine.
        self.check_parts()?;
        self.check_types(LogicalType::check_slot)?;
        let signatures: Vec<_> = self
            .overloads
            .iter()
            .map(|o| {
                let mut params = merged_params(&o.params, &o.logical_params);
                if let Some(ref varargs) = o.varargs {
                    params.push(varargs.param_ref());
                }
                params
            })
            .collect();
        // SAFETY: every logical parameter is a live `LogicalType` owned by this
        // builder, and the caller's contract means the C API is initialised.
        unsafe { reject_duplicate_overloads(&self.name.to_string_lossy(), &signatures)? };
        let name = self.name.to_string_lossy().into_owned();
        let rendered: Vec<_> = signatures
            .iter()
            .enumerate()
            .map(|(i, signature)| {
                // SAFETY: the logical parameters are live, as above.
                (format!("overload {i}"), unsafe {
                    super::collision::render_signature(signature)
                })
            })
            .collect();
        // SAFETY: `con` is valid per this function's contract.
        unsafe { super::collision::refuse_taken_signatures(con, snapshot, &name, &rendered)? };

        // SAFETY: Creates a new scalar function set handle.
        let mut set = unsafe { duckdb_create_scalar_function_set(self.name.as_ptr()) };

        let mut register_error: Option<ExtensionError> = None;

        for overload in &self.overloads {
            // Resolve return type: prefer explicit LogicalType over TypeId.
            // `_ret_lt_owner` keeps the LogicalType alive when created from TypeId.
            // The two `else` arms below cannot run: every overload was checked
            // for a return type and a callback before the set was created.
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
            let mut func = unsafe { duckdb_create_scalar_function() };

            // PITFALL L6: Must call this on EACH function, not just the set.
            // SAFETY: `func` is the handle `duckdb_create_scalar_function` returned above
            // (a fresh `new ScalarFunction`, scalar_function-c.cpp), destroyed only at the
            // end of this iteration. `self.name` is a `CString`, so NUL-terminated and live
            // for the call; `duckdb_scalar_function_set_name` copies it into
            // `ScalarFunction::name`.
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
                        // SAFETY: `func` is the handle `duckdb_create_scalar_function`
                        // returned above (a fresh `new ScalarFunction`,
                        // scalar_function-c.cpp), destroyed only at the end of this
                        // iteration. The type is a `LogicalType` owned by
                        // `overload.logical_params`, so its handle is live for this borrow;
                        // `duckdb_scalar_function_add_parameter` copies it
                        // (`arguments.push_back(*logical_type)`), keeping no pointer.
                        unsafe {
                            duckdb_scalar_function_add_parameter(
                                func,
                                overload.logical_params[logical_idx].1.as_raw(),
                            );
                        }
                        logical_idx += 1;
                    } else if simple_idx < overload.params.len() {
                        let lt = LogicalType::new(overload.params[simple_idx]);
                        // SAFETY: `func` is the handle `duckdb_create_scalar_function`
                        // returned above (a fresh `new ScalarFunction`,
                        // scalar_function-c.cpp), destroyed only at the end of this
                        // iteration. `lt` was created on the line above and is dropped only
                        // after this call; `duckdb_scalar_function_add_parameter` copies
                        // the type.
                        unsafe {
                            duckdb_scalar_function_add_parameter(func, lt.as_raw());
                        }
                        simple_idx += 1;
                    }
                }
            }

            // Set return type
            // SAFETY: `func` is the handle `duckdb_create_scalar_function` returned above
            // (a fresh `new ScalarFunction`, scalar_function-c.cpp), destroyed only at the
            // end of this iteration. `ret_raw` borrows `overload.return_logical` (owned by
            // the builder) or `_ret_lt_owner`, both alive to the end of this iteration;
            // `SetReturnType` copies the type.
            unsafe {
                duckdb_scalar_function_set_return_type(func, ret_raw);
            }

            // Set callback
            // SAFETY: `func` is the handle `duckdb_create_scalar_function` returned above
            // (a fresh `new ScalarFunction`, scalar_function-c.cpp), destroyed only at the
            // end of this iteration. `function` is a non-null `ScalarFn` (an `extern "C"`
            // fn pointer with the signature DuckDB calls); DuckDB only stores it in the
            // function's `CScalarFunctionInfo`.
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

            // Set bind / init callbacks if configured (`DuckDB` 1.5.0+)
            #[cfg(feature = "duckdb-1-5")]
            if let Some(bind_fn) = overload.bind {
                // SAFETY: func is a valid scalar function handle.
                unsafe {
                    duckdb_scalar_function_set_bind(func, Some(super::single::raw_bind(bind_fn)));
                }
            }
            #[cfg(feature = "duckdb-1-5")]
            if let Some(init_fn) = overload.init {
                // SAFETY: func is a valid scalar function handle.
                unsafe {
                    duckdb_scalar_function_set_init(func, Some(super::single::raw_init(init_fn)));
                }
            }

            // Set varargs type if configured (stable C API since v1.2.0)
            if let Some(ref varargs) = overload.varargs {
                // SAFETY: func is a valid scalar function handle, and every
                // overload's varargs type was checked above.
                unsafe { varargs.set_on(func) };
            }

            // Set volatile flag if configured (stable C API since v1.2.0)
            if overload.volatile {
                // SAFETY: func is a valid scalar function handle.
                unsafe {
                    duckdb_scalar_function_set_volatile(func);
                }
            }

            // Set extra info if provided
            if let Some(info) = &overload.extra_info {
                // SAFETY: func is valid; data and destroy are provided by caller.
                unsafe {
                    duckdb_scalar_function_set_extra_info(func, info.data(), info.destroy());
                    // DuckDB owns the allocation from here.
                    info.mark_transferred();
                }
            }

            // Add to set
            // SAFETY: `func` is the handle `duckdb_create_scalar_function` returned above
            // (a fresh `new ScalarFunction`, scalar_function-c.cpp), destroyed only at the
            // end of this iteration. `set` came from `duckdb_create_scalar_function_set`,
            // which returns null only for an empty name, and this call null-checks both
            // handles (returning `DuckDBError`). `AddFunction(T function)` takes the
            // function by value (function_set.hpp), so the set keeps its own copy, not
            // `func`.
            unsafe {
                duckdb_add_scalar_function_to_set(set, func);
            }

            // Destroy individual function (ownership transferred to set)
            // SAFETY: `func` is the handle `duckdb_create_scalar_function` returned above
            // (a fresh `new ScalarFunction`, scalar_function-c.cpp), destroyed only at the
            // end of this iteration. The set holds its own copy of the function (and shares
            // the extra info through its `shared_ptr`), so this single destroy of `func`
            // frees nothing the set uses; it also nulls `func`.
            unsafe {
                duckdb_destroy_scalar_function(&raw mut func);
            }
        }

        if register_error.is_none() {
            // SAFETY: `con` is a valid, open connection per this function's `# Safety`
            // clause. `set` is the set created above, not yet destroyed (the null an empty
            // name yields is refused by DuckDB's own null check). Registration copies the
            // set into the catalog (`CreateScalarFunctionInfo`) and catches its own
            // exceptions, so destroying `set` below is still ours to do.
            let result = unsafe { duckdb_register_scalar_function_set(con, set) };
            if result != DuckDBSuccess {
                register_error = Some(ExtensionError::new(format!(
                    "duckdb_register_scalar_function_set failed for '{name}': {hint}",
                    hint = crate::error::REGISTRATION_FAILURE_HINT
                )));
            }
        }

        // SAFETY: set was created above and must be destroyed.
        unsafe {
            duckdb_destroy_scalar_function_set(&raw mut set);
        }

        register_error.map_or_else(
            || {
                super::collision::record_registered(snapshot, &name, rendered);
                Ok(())
            },
            Err,
        )
    }
}

impl core::fmt::Debug for ScalarFunctionSetBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScalarFunctionSetBuilder")
            .field("name", &self.name)
            .field("overloads", &self.overloads.len())
            .finish()
    }
}
