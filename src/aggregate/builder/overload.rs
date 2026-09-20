// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! One overload within an [`AggregateFunctionSetBuilder`].
//!
//! Split out of `set.rs` so both files stay inside the 500-line guideline in
//! `CONTRIBUTING.md`.
//!
//! [`AggregateFunctionSetBuilder`]: super::AggregateFunctionSetBuilder

use crate::aggregate::callbacks::{
    CombineFn, DestroyFn, FinalizeFn, StateInitFn, StateSizeFn, UpdateFn,
};
use crate::types::{LogicalType, NullHandling, TypeId};

/// Specification for one overload within a function set.
///
/// This is the owned, post-build form of [`AggregateOverloadBuilder`]; the
/// builder is consumed into one of these when it is handed to
/// [`AggregateFunctionSetBuilder::overload`] or produced by
/// [`AggregateFunctionSetBuilder::overloads`].
///
/// [`AggregateFunctionSetBuilder::overload`]: super::AggregateFunctionSetBuilder::overload
/// [`AggregateFunctionSetBuilder::overloads`]: super::AggregateFunctionSetBuilder::overloads
pub(super) struct OverloadSpec {
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

/// A builder for one overload within an [`AggregateFunctionSetBuilder`].
///
/// Obtain one either from the closure passed to
/// [`AggregateFunctionSetBuilder::overloads`] (one per arity in a range) or by
/// calling [`AggregateOverloadBuilder::new`] and handing it to
/// [`AggregateFunctionSetBuilder::overload`].
///
/// # Return types
///
/// An overload may carry its own return type via [`returns`][Self::returns] or
/// [`returns_logical`][Self::returns_logical]. `DuckDB` resolves an aggregate
/// overload from the **parameter types and arity only** — the return type takes
/// no part in resolution — so overloads in one set are free to return different
/// types, exactly as `DuckDB`'s own `arg_max` does:
///
/// ```text
/// arg_max(ANY, ANY)      -> ANY
/// arg_max(ANY, ANY, ANY) -> ANY[]
/// ```
///
/// If an overload sets no return type, the set-level default from
/// [`AggregateFunctionSetBuilder::returns`] /
/// [`AggregateFunctionSetBuilder::returns_logical`] applies. Registration fails
/// if an overload has neither.
///
/// [`AggregateFunctionSetBuilder`]: super::AggregateFunctionSetBuilder
/// [`AggregateFunctionSetBuilder::overload`]: super::AggregateFunctionSetBuilder::overload
/// [`AggregateFunctionSetBuilder::overloads`]: super::AggregateFunctionSetBuilder::overloads
/// [`AggregateFunctionSetBuilder::returns`]: super::AggregateFunctionSetBuilder::returns
/// [`AggregateFunctionSetBuilder::returns_logical`]: super::AggregateFunctionSetBuilder::returns_logical
///
/// # Example
///
/// Two overloads under one name with **different** return types:
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
/// //             .state_size(int_state_size)
/// //             .init(int_init)
/// //             .update(int_update)
/// //             .combine(int_combine)
/// //             .finalize(int_finalize),
/// //     )
/// //     .overload(
/// //         AggregateOverloadBuilder::new()
/// //             .param(TypeId::Varchar)
/// //             .returns(TypeId::Varchar)
/// //             .state_size(str_state_size)
/// //             .init(str_init)
/// //             .update(str_update)
/// //             .combine(str_combine)
/// //             .finalize(str_finalize),
/// //     )
/// //     .register(con)
/// ```
#[must_use]
pub struct AggregateOverloadBuilder {
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

impl AggregateOverloadBuilder {
    /// Creates a new `AggregateOverloadBuilder` with no parameters, no return
    /// type and no callbacks set.
    pub fn new() -> Self {
        Self {
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

    /// Adds a positional parameter to this overload.
    ///
    /// For complex types like `LIST(BIGINT)`, use
    /// [`param_logical`][Self::param_logical].
    pub fn param(mut self, type_id: TypeId) -> Self {
        self.params.push(type_id);
        self
    }

    /// Adds a positional parameter with a complex [`LogicalType`].
    ///
    /// Use this for parameterized types that [`TypeId`] cannot express, such as
    /// `LIST(BIGINT)`, `MAP(VARCHAR, INTEGER)`, or `STRUCT(...)`.
    #[mutants::skip] // position arithmetic tested via E2E
    pub fn param_logical(mut self, logical_type: LogicalType) -> Self {
        let position = self.params.len() + self.logical_params.len();
        self.logical_params.push((position, logical_type));
        self
    }

    /// Sets the return type for **this overload only**, overriding any
    /// set-level default.
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the return type for **this overload only** to a complex
    /// [`LogicalType`], overriding any set-level default.
    ///
    /// Use this for parameterized return types that [`TypeId`] cannot express,
    /// such as `LIST(BOOLEAN)`, `MAP(VARCHAR, INTEGER)`, or `STRUCT(...)`.
    ///
    /// If both `returns` and `returns_logical` are called on the same overload,
    /// the logical type takes precedence.
    // Building a `LogicalType` calls `duckdb_create_logical_type`, which panics
    // without a live dispatch table, so no `--lib` test can assert on what this
    // stores. Covered end-to-end instead, by the DECIMAL(18,2) overload in
    // `one_aggregate_set_serves_overloads_with_different_return_types`. Same
    // reasoning as `ScalarOverloadBuilder::returns_logical`.
    #[mutants::skip] // tested via E2E
    pub fn returns_logical(mut self, logical_type: LogicalType) -> Self {
        self.return_logical = Some(logical_type);
        self
    }

    /// Sets the `state_size` callback for this overload.
    pub const fn state_size(mut self, f: StateSizeFn) -> Self {
        self.state_size = Some(f);
        self
    }

    /// Sets the `init` callback for this overload.
    pub const fn init(mut self, f: StateInitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the `update` callback for this overload.
    pub const fn update(mut self, f: UpdateFn) -> Self {
        self.update = Some(f);
        self
    }

    /// Sets the `combine` callback for this overload.
    pub const fn combine(mut self, f: CombineFn) -> Self {
        self.combine = Some(f);
        self
    }

    /// Sets the `finalize` callback for this overload.
    pub const fn finalize(mut self, f: FinalizeFn) -> Self {
        self.finalize = Some(f);
        self
    }

    /// Sets the optional destructor callback for this overload.
    pub const fn destructor(mut self, f: DestroyFn) -> Self {
        self.destructor = Some(f);
        self
    }

    /// Sets the NULL handling behaviour for this overload.
    ///
    /// By default, `DuckDB` skips NULL rows in aggregate functions
    /// ([`DefaultNullHandling`][NullHandling::DefaultNullHandling]).
    /// Set to [`SpecialNullHandling`][NullHandling::SpecialNullHandling] to receive
    /// NULL values in your `update` callback.
    pub const fn null_handling(mut self, handling: NullHandling) -> Self {
        self.null_handling = handling;
        self
    }

    /// Consumes this builder into the owned spec the set stores.
    pub(super) fn into_spec(self) -> OverloadSpec {
        OverloadSpec {
            params: self.params,
            logical_params: self.logical_params,
            return_type: self.return_type,
            return_logical: self.return_logical,
            state_size: self.state_size,
            init: self.init,
            update: self.update,
            combine: self.combine,
            finalize: self.finalize,
            destructor: self.destructor,
            null_handling: self.null_handling,
        }
    }
}

impl Default for AggregateOverloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for AggregateOverloadBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        f.debug_struct("AggregateOverloadBuilder")
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
            .finish()
    }
}
