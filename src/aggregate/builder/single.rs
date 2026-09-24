// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::CString;
use std::os::raw::c_void;

use libduckdb_sys::duckdb_delete_callback_t;

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};
use crate::validate::validate_function_name;

mod register;

/// Builder for registering a single-signature `DuckDB` aggregate function.
///
/// # Known `DuckDB` limitation
///
/// **Two query shapes make every C-API aggregate read out of bounds.** In
/// them `DuckDB` calls `update` with a state array holding **one** state while
/// passing `count > 1` rows, so the callback reads `states[1..count]` past the
/// end of the array — undefined behaviour in *any* C-API aggregate, whether
/// built with quack-rs or by hand:
///
/// - **Window aggregates whose frame is the whole partition**, e.g.
///   `agg(x) OVER ()` — `WindowConstantAggregator`
///   (`src/function/window/window_constant_aggregator.cpp`, ~lines 106 and
///   296–299 in `DuckDB` 1.5.5).
/// - **Ordered aggregates**, e.g. `agg(x ORDER BY y)` —
///   `src/function/aggregate/sorted_aggregate_function.cpp`, ~lines 630–633.
///
/// Both pass a `CONSTANT_VECTOR` of states because the function has no
/// `simple_update` (the C API cannot set one), and `CAPIAggregateUpdate`
/// (`src/main/capi/aggregate_function-c.cpp`, ~lines 92–110) hands the
/// vector's data pointer to the extension without flattening it. This is a
/// defect in `DuckDB`'s C API, not in quack-rs, and it cannot be detected
/// from inside the callback — reading `states[1]` to check is itself the
/// out-of-bounds read. Reported upstream as
/// [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Until it is fixed, do not use C-API aggregates in
/// those two query shapes.
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
/// unsafe extern "C" fn update(
///     _: duckdb_function_info,
///     _: duckdb_data_chunk,
///     _: *mut duckdb_aggregate_state,
/// ) {}
/// unsafe extern "C" fn combine(
///     _: duckdb_function_info,
///     _: *mut duckdb_aggregate_state,
///     _: *mut duckdb_aggregate_state,
///     _: idx_t,
/// ) {}
/// unsafe extern "C" fn finalize(
///     _: duckdb_function_info,
///     _: *mut duckdb_aggregate_state,
///     _: duckdb_vector,
///     _: idx_t,
///     _: idx_t,
/// ) {}
///
/// /// # Safety
/// ///
/// /// `con` must be a valid, open connection.
/// unsafe fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
///     // SAFETY: `con` is valid per this function's contract.
///     unsafe {
///         AggregateFunctionBuilder::new("word_count")
///             .param(TypeId::Varchar)
///             .returns(TypeId::BigInt)
///             .state_size(state_size)
///             .init(state_init)
///             .update(update)
///             .combine(combine)
///             .finalize(finalize)
///             .register(con)
///     }
/// }
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
    pub(super) extra_info: Option<crate::extra_info::ExtraInfo>,
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
            extra_info: None,
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
            extra_info: None,
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

    /// Installs [`FfiState<T>`][crate::aggregate::FfiState]'s `state_size`,
    /// `init` and `destructor` callbacks together, so they cannot describe
    /// different states (setting them one by one, a size callback for one `T`
    /// with an init callback for another wrote past `DuckDB`'s allocation).
    /// `update`, `combine` and `finalize` still read the state through
    /// `FfiState::<T>::with_state` / `with_state_mut` with the same `T`.
    pub fn ffi_state<T: crate::aggregate::AggregateState>(self) -> Self {
        self.state_size(crate::aggregate::FfiState::<T>::size_callback)
            .init(crate::aggregate::FfiState::<T>::init_callback)
            .destructor(crate::aggregate::FfiState::<T>::destroy_callback)
    }

    /// Sets the `state_size` callback.
    ///
    /// The `state_size`, `init` and `destructor` callbacks must describe the
    /// same state: `DuckDB` allocates what `state_size` returns and hands that
    /// allocation to `init` and, later, to `destructor` and the other
    /// callbacks. [`FfiState<A>`][crate::aggregate::FfiState]'s size callback
    /// with `FfiState<B>`'s init callback writes a `B` into space sized for an
    /// `A`, past the allocation when `B` is larger. Prefer
    /// [`ffi_state`][Self::ffi_state], which installs all three for one `T`.
    pub fn state_size(mut self, f: StateSizeFn) -> Self {
        self.state_size = Some(f);
        self
    }

    /// Sets the `state_init` callback.
    ///
    /// The `state_size`, `init` and `destructor` callbacks must describe the
    /// same state: `DuckDB` allocates what `state_size` returns and hands that
    /// allocation to `init` and, later, to `destructor` and the other
    /// callbacks. [`FfiState<A>`][crate::aggregate::FfiState]'s size callback
    /// with `FfiState<B>`'s init callback writes a `B` into space sized for an
    /// `A`, past the allocation when `B` is larger. Prefer
    /// [`ffi_state`][Self::ffi_state], which installs all three for one `T`.
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
    /// Required when you use [`FfiState<T>`][crate::aggregate::FfiState]: its
    /// `destroy_callback` is what drops each `T`.
    ///
    /// When none is set, `register` installs a no-op destructor rather than
    /// none at all. `DuckDB` evaluates an aggregate without a state destructor
    /// as a *streaming* window for running frames
    /// (`agg(x) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)`), and
    /// through the C API that path feeds `update` the first row of each chunk
    /// in place of every later one — a wrong answer with no error (Pitfall
    /// L13). The cost is the streaming shortcut: such windows are evaluated by
    /// `DuckDB`'s general window operator instead.
    pub fn destructor(mut self, f: DestroyFn) -> Self {
        self.destructor = Some(f);
        self
    }

    /// Sets the NULL handling behaviour for this aggregate function.
    ///
    /// This does **not** decide whether `update` sees NULL rows: it receives
    /// every row under either setting, so an aggregate that ignores NULLs must
    /// skip rows whose
    /// [`VectorReader::is_valid`][crate::vector::VectorReader::is_valid] is
    /// false. [`SpecialNullHandling`][NullHandling::SpecialNullHandling]
    /// declares that the aggregate may return non-NULL for NULL input; see
    /// [`NullHandling`] for the one planner decision that reads it.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
        self
    }

    /// Attaches arbitrary data to this aggregate function.
    ///
    /// The data pointer is available inside callbacks via
    /// `duckdb_aggregate_function_get_extra_info`. The `destroy` callback is
    /// called by `DuckDB` when the function is dropped to free the data.
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

    /// The completeness checks that need no `DuckDB` call: a return type and
    /// the five required callbacks. [`MockRegistrar`][crate::testing::MockRegistrar]
    /// runs them too, so a builder it accepts is not refused at `LOAD` for a
    /// missing part.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        if self.return_type.is_none() && self.return_logical.is_none() {
            return Err(ExtensionError::new("return type not set"));
        }
        let missing = [
            ("state_size", self.state_size.is_none()),
            ("init", self.init.is_none()),
            ("update", self.update.is_none()),
            ("combine", self.combine.is_none()),
            ("finalize", self.finalize.is_none()),
        ]
        .into_iter()
        .find_map(|(name, absent)| absent.then_some(name));
        missing.map_or(Ok(()), |callback| {
            Err(ExtensionError::new(format!("{callback} callback not set")))
        })
    }
}

impl core::fmt::Debug for AggregateFunctionBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        f.debug_struct("AggregateFunctionBuilder")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("logical_params", &self.logical_params.len())
            .field("return_type", &self.return_type)
            .field("return_logical", &self.return_logical)
            .field("state_size", &Callback::of(&self.state_size))
            .field("init", &Callback::of(&self.init))
            .field("update", &Callback::of(&self.update))
            .field("combine", &Callback::of(&self.combine))
            .field("finalize", &Callback::of(&self.finalize))
            .field("destructor", &Callback::of(&self.destructor))
            .field("null_handling", &self.null_handling)
            .field("extra_info", &Callback::of(&self.extra_info))
            .finish()
    }
}
