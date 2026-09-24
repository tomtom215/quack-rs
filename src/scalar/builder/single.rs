// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::cell::RefCell;
use std::ffi::CString;

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    duckdb_bind_info, duckdb_init_info, duckdb_scalar_function_set_bind,
    duckdb_scalar_function_set_init,
};
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_connection, duckdb_create_scalar_function, duckdb_data_chunk, duckdb_delete_callback_t,
    duckdb_destroy_scalar_function, duckdb_function_info, duckdb_register_scalar_function,
    duckdb_scalar_function_add_parameter, duckdb_scalar_function_set_extra_info,
    duckdb_scalar_function_set_function, duckdb_scalar_function_set_name,
    duckdb_scalar_function_set_return_type, duckdb_scalar_function_set_special_handling,
    duckdb_scalar_function_set_volatile, duckdb_vector, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::types::logical_type::SlotCheck;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::validate::validate_function_name;

/// The scalar function bind callback signature (`DuckDB` 1.5.0+).
///
/// Called once during query planning. Use this to inspect arguments and
/// allocate per-query state via `duckdb_scalar_function_set_bind_data`.
///
/// **Breaking** in 0.18.0: the argument is a
/// [`RawScalarBindInfo`][crate::scalar::RawScalarBindInfo], so that a table
/// function's bind callback, which `DuckDB` passes a different struct, cannot
/// be registered here.
#[cfg(feature = "duckdb-1-5")]
pub type ScalarBindFn = unsafe extern "C" fn(info: crate::scalar::RawScalarBindInfo);

/// The scalar function init callback signature (`DuckDB` 1.5.0+).
///
/// Called once per thread before execution begins. Use this to allocate
/// per-thread local state via `duckdb_scalar_function_init_set_state`.
///
/// **Breaking** in 0.18.0: the argument is a
/// [`RawScalarInitInfo`][crate::scalar::RawScalarInitInfo]; see
/// [`ScalarBindFn`].
#[cfg(feature = "duckdb-1-5")]
pub type ScalarInitFn = unsafe extern "C" fn(info: crate::scalar::RawScalarInitInfo);

/// `f` as the callback type `duckdb_scalar_function_set_bind` takes.
#[cfg(feature = "duckdb-1-5")]
pub(super) const fn raw_bind(f: ScalarBindFn) -> unsafe extern "C" fn(duckdb_bind_info) {
    // SAFETY: `RawScalarBindInfo` is `#[repr(transparent)]` over
    // `duckdb_bind_info`, so the two function pointer types have the same ABI
    // and `DuckDB` calls `f` with exactly the argument it declares.
    unsafe { core::mem::transmute::<ScalarBindFn, unsafe extern "C" fn(duckdb_bind_info)>(f) }
}

/// `f` as the callback type `duckdb_scalar_function_set_init` takes.
#[cfg(feature = "duckdb-1-5")]
pub(super) const fn raw_init(f: ScalarInitFn) -> unsafe extern "C" fn(duckdb_init_info) {
    // SAFETY: as in `raw_bind`, for `RawScalarInitInfo` over
    // `duckdb_init_info`.
    unsafe { core::mem::transmute::<ScalarInitFn, unsafe extern "C" fn(duckdb_init_info)>(f) }
}

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
    pub(super) extra_info: Option<crate::extra_info::ExtraInfo>,
    pub(super) varargs: Option<super::signature::Varargs>,
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
            varargs: None,
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
            varargs: None,
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
    /// additional arguments that match the given type. Maps to
    /// `duckdb_scalar_function_set_varargs`, part of the stable C API since
    /// v1.2.0, so no feature flag is needed.
    ///
    /// A composite `type_id` (see [`TypeId::is_composite`]) makes
    /// [`register`][Self::register] return an error naming the varargs slot;
    /// use [`varargs_logical`][Self::varargs_logical] for those types.
    #[mutants::skip] // tested via E2E
    pub fn varargs(mut self, type_id: TypeId) -> Self {
        self.varargs = Some(super::signature::Varargs::Id(type_id));
        self
    }

    /// Marks this function as accepting variadic arguments with a complex type.
    ///
    /// Identical to [`varargs`][Self::varargs] but accepts a [`LogicalType`]
    /// for parameterized types.
    #[mutants::skip] // tested via E2E
    pub fn varargs_logical(mut self, logical_type: LogicalType) -> Self {
        self.varargs = Some(super::signature::Varargs::Logical(logical_type));
        self
    }

    /// Marks this function as volatile.
    ///
    /// Volatile functions are re-evaluated for every row, even when called with
    /// the same arguments (e.g. `random()`). Non-volatile functions may be
    /// optimized by `DuckDB` to only execute once for constant arguments.
    /// Maps to `duckdb_scalar_function_set_volatile`, part of the stable C API
    /// since v1.2.0, so no feature flag is needed.
    #[mutants::skip] // tested via E2E
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
    ///
    /// Guard it against panics with
    /// [`scalar_bind_callback!`](crate::scalar_bind_callback), **not**
    /// `table_bind_callback!`: the two share a C signature, so the compiler
    /// accepts either, but the table macro reports a panic through the table
    /// function's `duckdb_bind_set_error`, which writes past the smaller scalar
    /// bind info and corrupts the stack.
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
    ///
    /// Guard it against panics with
    /// [`scalar_init_callback!`](crate::scalar_init_callback), **not**
    /// `table_init_callback!`: the two share a C signature, so the compiler
    /// accepts either, but the table macro reports a panic through the table
    /// function's `duckdb_init_set_error`, which writes past the smaller scalar
    /// init info and corrupts the stack.
    #[cfg(feature = "duckdb-1-5")]
    #[mutants::skip] // DuckDB 1.5+ feature, tested via E2E
    pub fn init(mut self, f: ScalarInitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the NULL handling behaviour for this function.
    ///
    /// Under either setting the callback receives NULL rows. With the default,
    /// [`DefaultNullHandling`][NullHandling::DefaultNullHandling], the callback
    /// **promises** NULL-in-NULL-out and must keep that promise itself —
    /// `DuckDB` does not write NULL for it (pitfall L8; see [`NullHandling`] and
    /// [`DataChunk::propagate_nulls`][crate::data_chunk::DataChunk::propagate_nulls]).
    /// [`SpecialNullHandling`][NullHandling::SpecialNullHandling] declares that
    /// it may return non-NULL for NULL input.
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
    /// - `data` must stay valid until `destroy` frees it (with no `destroy`,
    ///   for as long as the database lives), and `destroy` must be able to
    ///   free it exactly once with no one else freeing it — so one pointer
    ///   must not be given to two builders or overloads, each of which hands
    ///   its `destroy` to `DuckDB` separately.
    /// - The pointee must be `Send + Sync`. `DuckDB` hands the same pointer to
    ///   the callbacks on every thread that executes the function, possibly at
    ///   the same moment, and `destroy` runs on whichever thread releases the
    ///   function — or, if the builder is dropped unregistered, the thread
    ///   that drops it.
    /// - `destroy` must not unwind: it is an `extern "C" fn`, so a panic
    ///   escaping it aborts the process. Wrap a body that can panic in
    ///   [`catch_ffi_panic`][crate::callback::catch_ffi_panic].
    /// - The pointee must be the type the installed function reads it as. A
    ///   closure-built function reads its own `extra_info`, so the builder
    ///   inside a [`TypedScalarFunctionBuilder`][crate::scalar::TypedScalarFunctionBuilder]
    ///   must not be given another.
    ///
    /// The typical pattern is to box your data:
    /// `Box::into_raw(Box::new(my_data)).cast()`.
    pub unsafe fn extra_info(
        mut self,
        data: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        // SAFETY: forwarded from this method's own contract.
        self.extra_info = Some(unsafe { crate::extra_info::ExtraInfo::new(data, destroy) });
        self
    }

    /// Checks everything that needs no `DuckDB` call, returning the callback.
    ///
    /// Composite `TypeId`s are rejected before any `DuckDB` handle exists, so
    /// the diagnostic names the offending slot instead of surfacing as an
    /// opaque `duckdb_register_scalar_function failed`. See
    /// `TypeId::is_composite`.
    /// The completeness checks that need no `DuckDB` call: a return type and
    /// a function callback. [`MockRegistrar`][crate::testing::MockRegistrar]
    /// runs them too, so a builder it accepts is not refused at `LOAD` for a
    /// missing part.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        if self.return_logical.is_none() && self.return_type.is_none() {
            return Err(ExtensionError::new("return type not set"));
        }
        if self.function.is_none() {
            return Err(ExtensionError::new("function callback not set"));
        }
        Ok(())
    }

    /// Refuses a type [`register`][Self::register] refuses before its first
    /// `DuckDB` call; `slot` checks each `TypeId` (see [`SlotCheck`]).
    pub(crate) fn check_types(&self, slot: SlotCheck) -> Result<(), ExtensionError> {
        for (i, id) in self.params.iter().enumerate() {
            slot(*id, &format!("scalar function parameter {i}"))?;
        }
        if let Some(id) = self.return_type {
            slot(id, "scalar function return type")?;
        }
        if let Some(ref varargs) = self.varargs {
            varargs.check(slot, "scalar function varargs")?;
        }
        crate::table::type_check::refuse_any_return(
            "scalar function return type",
            self.return_type,
            self.return_logical.as_ref(),
        )
    }

    fn check_complete(&self) -> Result<ScalarFn, ExtensionError> {
        self.check_parts()?;
        self.check_types(LogicalType::check_slot)?;
        self.function
            .ok_or_else(|| ExtensionError::new("function callback not set"))
    }

    /// This function's signature, rendered as `duckdb_functions()` prints it
    /// (`None` if a type is not rendered), labelled for error messages.
    fn rendered_signature(&self) -> Vec<(String, Option<super::collision::Rendered>)> {
        let mut signature = super::signature::merged_params(&self.params, &self.logical_params);
        if let Some(ref varargs) = self.varargs {
            signature.push(varargs.param_ref());
        }
        // SAFETY: every logical parameter is a live handle owned by this
        // builder, and rendering one needs only the C API, which the
        // `register` caller's contract initialises.
        let rendered = unsafe { super::collision::render_signature(&signature) };
        vec![(String::from("scalar function"), rendered)]
    }

    /// Registers the scalar function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The return type was not set.
    /// - The function callback was not set.
    /// - A parameter, varargs or return type was given as a bare composite
    ///   [`TypeId`][crate::types::TypeId] (`DECIMAL`, `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY`,
    ///   `UNION`), which carries parameters a `TypeId` cannot express. Build
    ///   it as a [`LogicalType`][crate::types::LogicalType] and use the `*_logical` method; the error
    ///   names the slot.
    /// - A scalar function with this name and parameter types already exists
    ///   (see "Name collisions").
    /// - `DuckDB` reports a registration failure.
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
    /// quack-rs therefore refuses, before registering, when
    /// `duckdb_functions()` already lists a scalar with this name whose
    /// parameter and varargs types accept the same call — the same types, or,
    /// with varargs, the same types at some argument count (`f(BIGINT)` beside
    /// an existing `f(BIGINT, BIGINT...)`: `DuckDB` would find `f(1)`
    /// ambiguous). Types are compared as `DuckDB` prints them, aliases
    /// included, so `abs(myint)` for `CREATE TYPE myint AS INTEGER` is its own
    /// overload. The check covers every parameter type except `STRUCT`,
    /// `UNION`, `ENUM` and the `SQLNULL` / literal pseudo-types; a signature
    /// containing one of those is registered unchecked. `DuckDB` itself
    /// refuses a name that belongs to an aggregate function or a macro (such
    /// as the built-in `list_sum`).
    ///
    /// # Cost
    ///
    /// Each call lists the catalog once — one scan of `duckdb_functions()`,
    /// 17–21 ms per call in a release build against `DuckDB` 1.5.5 (measured
    /// over 300 registrations). Registering through the entry point's
    /// [`Connection`](crate::connection::Connection) instead (its
    /// [`Registrar`](crate::connection::Registrar) methods) lists the catalog
    /// once per extension load: 300 registrations took 19–36 ms in all.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // SAFETY: forwarded from this function's own contract.
        unsafe { self.register_with(con, None) }
    }

    /// [`register`][Self::register], checking signatures against `snapshot`
    /// (listed on first use) instead of listing the catalog for this call.
    ///
    /// # Safety
    ///
    /// As [`register`][Self::register].
    pub(crate) unsafe fn register_with(
        self,
        con: duckdb_connection,
        snapshot: Option<&RefCell<Option<super::collision::ExistingScalars>>>,
    ) -> Result<(), ExtensionError> {
        let function = self.check_complete()?;
        let name = self.name.to_string_lossy().into_owned();
        let rendered = self.rendered_signature();
        // SAFETY: `con` is valid per this function's contract.
        unsafe { super::collision::refuse_taken_signatures(con, snapshot, &name, &rendered)? };

        // Resolve return type: prefer explicit LogicalType over TypeId. One of
        // the two is set, checked above.
        let ret_lt = match (self.return_logical, self.return_type) {
            (Some(lt), _) => lt,
            (None, Some(id)) => LogicalType::for_slot(id, "scalar function return type")?,
            (None, None) => return Err(ExtensionError::new("return type not set")),
        };

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
        if let Some(info) = self.extra_info {
            // SAFETY: func is valid; data and destroy are provided by caller.
            unsafe {
                duckdb_scalar_function_set_extra_info(func, info.data(), info.destroy());
                // DuckDB owns the allocation from here; `ExtraInfo::drop` must
                // not free it as well.
                info.mark_transferred();
            }
        }

        // Set bind callback if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if let Some(bind_fn) = self.bind {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_bind(func, Some(raw_bind(bind_fn)));
            }
        }

        // Set init callback if configured (`DuckDB` 1.5.0+)
        #[cfg(feature = "duckdb-1-5")]
        if let Some(init_fn) = self.init {
            // SAFETY: func is a valid scalar function handle.
            unsafe {
                duckdb_scalar_function_set_init(func, Some(raw_init(init_fn)));
            }
        }

        // Set varargs type if configured (stable C API since v1.2.0)
        if let Some(ref varargs) = self.varargs {
            // SAFETY: func is a valid scalar function handle, and the varargs
            // type was checked above.
            unsafe { varargs.set_on(func) };
        }

        // Set volatile flag if configured (stable C API since v1.2.0)
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
            super::collision::record_registered(snapshot, &name, rendered);
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_scalar_function failed for '{name}': {hint}",
                hint = crate::error::REGISTRATION_FAILURE_HINT
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
            .field("extra_info", &Callback::of(&self.extra_info))
            .field("varargs", &self.varargs)
            .field("volatile", &self.volatile);
        #[cfg(feature = "duckdb-1-5")]
        s.field("bind", &Callback::of(&self.bind))
            .field("init", &Callback::of(&self.init));
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::super::collision::Rendered;
    use super::ScalarFunctionBuilder;
    use crate::types::TypeId;

    /// Signatures given as `TypeId`s render with no `DuckDB` call, in the
    /// notation `duckdb_functions()` prints, labelled for the error message.
    #[test]
    fn a_type_id_signature_renders_as_the_catalog_prints_it() {
        let builder = ScalarFunctionBuilder::new("f")
            .param(TypeId::BigInt)
            .param(TypeId::TimestampTz)
            .varargs(TypeId::Varchar);
        assert_eq!(
            builder.rendered_signature(),
            vec![(
                String::from("scalar function"),
                Some(Rendered {
                    fixed: vec![
                        String::from("BIGINT"),
                        String::from("TIMESTAMP WITH TIME ZONE")
                    ],
                    varargs: Some(String::from("VARCHAR")),
                })
            )]
        );
    }
}
