// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! One overload within a [`ScalarFunctionSetBuilder`].
//!
//! Split out of `set.rs` so both files stay inside the 500-line guideline in
//! `CONTRIBUTING.md`, mirroring `aggregate/builder/overload.rs`.
//!
//! [`ScalarFunctionSetBuilder`]: super::ScalarFunctionSetBuilder

use std::os::raw::c_void;

use libduckdb_sys::duckdb_delete_callback_t;

use crate::error::ExtensionError;
use crate::types::{LogicalType, NullHandling, TypeId};

use super::single::ScalarFn;
#[cfg(feature = "duckdb-1-5")]
use super::single::{ScalarBindFn, ScalarInitFn};

/// Specification for one overload within a scalar function set: the owned,
/// post-build form of [`ScalarOverloadBuilder`].
pub(super) struct ScalarOverloadSpec {
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

impl ScalarOverloadSpec {
    /// Checks that overload `index` has everything registration needs,
    /// without any `DuckDB` call.
    pub(super) fn check_complete(&self, index: usize) -> Result<(), ExtensionError> {
        if self.return_type.is_none() && self.return_logical.is_none() {
            return Err(ExtensionError::new(format!(
                "overload {index} has no return type: call `returns` or `returns_logical`"
            )));
        }
        if self.function.is_none() {
            return Err(ExtensionError::new(format!(
                "overload {index} has no function callback: call `function`"
            )));
        }
        Ok(())
    }
}

/// A builder for one overload within a [`ScalarFunctionSetBuilder`].
///
/// [`ScalarFunctionSetBuilder`]: super::ScalarFunctionSetBuilder
#[must_use]
pub struct ScalarOverloadBuilder {
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

impl ScalarOverloadBuilder {
    /// Creates a new `ScalarOverloadBuilder`.
    pub fn new() -> Self {
        Self {
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

    /// Sets the return type for this overload.
    ///
    /// For complex return types like `LIST(BIGINT)`, use
    /// [`returns_logical`][Self::returns_logical] instead.
    pub const fn returns(mut self, type_id: TypeId) -> Self {
        self.return_type = Some(type_id);
        self
    }

    /// Sets the return type to a complex [`LogicalType`] for this overload.
    ///
    /// Use this for parameterized return types that [`TypeId`] cannot express,
    /// such as `LIST(BOOLEAN)`, `LIST(TIMESTAMP)`, `MAP(VARCHAR, INTEGER)`, etc.
    ///
    /// If both `returns` and `returns_logical` are called, the logical type takes
    /// precedence.
    #[mutants::skip] // tested via E2E
    pub fn returns_logical(mut self, logical_type: LogicalType) -> Self {
        self.return_logical = Some(logical_type);
        self
    }

    /// Sets the scalar function callback for this overload.
    pub fn function(mut self, f: ScalarFn) -> Self {
        self.function = Some(f);
        self
    }

    /// Marks this overload as accepting variadic arguments of the given type,
    /// after its fixed parameters.
    ///
    /// Mirrors [`ScalarFunctionBuilder::varargs`][super::ScalarFunctionBuilder::varargs],
    /// including the error from `register` for a composite `type_id`.
    #[mutants::skip] // tested via E2E
    pub fn varargs(mut self, type_id: TypeId) -> Self {
        self.varargs = Some(super::signature::Varargs::Id(type_id));
        self
    }

    /// Marks this overload as accepting variadic arguments with a complex type.
    ///
    /// Mirrors
    /// [`ScalarFunctionBuilder::varargs_logical`][super::ScalarFunctionBuilder::varargs_logical].
    #[mutants::skip] // tested via E2E
    pub fn varargs_logical(mut self, logical_type: LogicalType) -> Self {
        self.varargs = Some(super::signature::Varargs::Logical(logical_type));
        self
    }

    /// Marks this overload as volatile: re-evaluated for every row even when
    /// its arguments are constant.
    ///
    /// Mirrors [`ScalarFunctionBuilder::volatile`][super::ScalarFunctionBuilder::volatile].
    /// Each overload of a set has its own stability.
    pub const fn volatile(mut self) -> Self {
        self.volatile = true;
        self
    }

    /// Sets a bind callback for this overload (`DuckDB` 1.5.0+).
    ///
    /// Mirrors [`ScalarFunctionBuilder::bind`][super::ScalarFunctionBuilder::bind].
    ///
    /// Guard it against panics with
    /// [`scalar_bind_callback!`](crate::scalar_bind_callback), **not**
    /// `table_bind_callback!`: the two share a C signature, so the compiler
    /// accepts either, but the table macro reports a panic through the table
    /// function's `duckdb_bind_set_error`, which writes past the smaller scalar
    /// bind info and corrupts the stack.
    #[cfg(feature = "duckdb-1-5")]
    pub fn bind(mut self, f: ScalarBindFn) -> Self {
        self.bind = Some(f);
        self
    }

    /// Sets an init callback for this overload (`DuckDB` 1.5.0+).
    ///
    /// Mirrors [`ScalarFunctionBuilder::init`][super::ScalarFunctionBuilder::init].
    ///
    /// Guard it against panics with
    /// [`scalar_init_callback!`](crate::scalar_init_callback), **not**
    /// `table_init_callback!`: the two share a C signature, so the compiler
    /// accepts either, but the table macro reports a panic through the table
    /// function's `duckdb_init_set_error`, which writes past the smaller scalar
    /// init info and corrupts the stack.
    #[cfg(feature = "duckdb-1-5")]
    pub fn init(mut self, f: ScalarInitFn) -> Self {
        self.init = Some(f);
        self
    }

    /// Sets the NULL handling behaviour for this overload.
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

    /// Attaches arbitrary data to this overload.
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

    /// Consumes this builder into the owned spec the set stores.
    pub(super) fn into_spec(self) -> ScalarOverloadSpec {
        ScalarOverloadSpec {
            params: self.params,
            logical_params: self.logical_params,
            return_type: self.return_type,
            return_logical: self.return_logical,
            function: self.function,
            null_handling: self.null_handling,
            extra_info: self.extra_info,
            varargs: self.varargs,
            volatile: self.volatile,
            #[cfg(feature = "duckdb-1-5")]
            bind: self.bind,
            #[cfg(feature = "duckdb-1-5")]
            init: self.init,
        }
    }
}

impl Default for ScalarOverloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for ScalarOverloadBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        let mut s = f.debug_struct("ScalarOverloadBuilder");
        s.field("params", &self.params)
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
