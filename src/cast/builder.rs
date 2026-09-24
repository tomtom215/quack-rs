// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering custom `DuckDB` cast functions.

use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_cast_function_get_cast_mode, duckdb_cast_function_get_extra_info,
    duckdb_cast_function_set_error, duckdb_cast_function_set_extra_info,
    duckdb_cast_function_set_function, duckdb_cast_function_set_implicit_cast_cost,
    duckdb_cast_function_set_row_error, duckdb_cast_function_set_source_type,
    duckdb_cast_function_set_target_type, duckdb_cast_mode_DUCKDB_CAST_TRY, duckdb_connection,
    duckdb_create_cast_function, duckdb_delete_callback_t, duckdb_destroy_cast_function,
    duckdb_function_info, duckdb_register_cast_function, duckdb_vector, idx_t, DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::table::cstr::error_cstring;
use crate::types::{LogicalType, TypeId};

// ── Cast mode ─────────────────────────────────────────────────────────────────

/// Whether the cast is called as a regular `CAST` or a `TRY_CAST`.
///
/// In [`Try`][CastMode::Try] mode, conversion failures should write `NULL` for
/// the failed row and call [`CastFunctionInfo::set_row_error`] rather than
/// aborting the whole query. See [`CastFn`] for what `DuckDB` does with the
/// callback's return value in each mode — in particular, returning `false`
/// in `Try` mode does **not** null anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastMode {
    /// Regular `CAST` — any failure aborts the query.
    Normal,
    /// `TRY_CAST` — failures produce `NULL`; use per-row error reporting.
    Try,
}

impl CastMode {
    const fn from_raw(raw: libduckdb_sys::duckdb_cast_mode) -> Self {
        if raw == duckdb_cast_mode_DUCKDB_CAST_TRY {
            Self::Try
        } else {
            Self::Normal
        }
    }
}

// ── Callback info wrapper ──────────────────────────────────────────────────────

/// Ergonomic wrapper around the `duckdb_function_info` handle provided to a
/// cast callback.
///
/// Exposes the cast-specific methods that are only meaningful inside a cast
/// function callback.
pub struct CastFunctionInfo {
    info: duckdb_function_info,
}

impl CastFunctionInfo {
    /// Wraps a raw `duckdb_function_info` provided by `DuckDB` inside a cast
    /// callback.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_function_info` passed by `DuckDB` to a
    /// cast callback.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_function_info) -> Self {
        Self { info }
    }

    /// Returns whether this invocation is a `TRY_CAST` or a regular `CAST`.
    ///
    /// Check this inside your callback to decide between aborting on error
    /// ([`CastMode::Normal`]) and producing `NULL` with a per-row error
    /// ([`CastMode::Try`]).
    #[must_use]
    pub fn cast_mode(&self) -> CastMode {
        // SAFETY: self.info is valid per constructor contract.
        let raw = unsafe { duckdb_cast_function_get_cast_mode(self.info) };
        CastMode::from_raw(raw)
    }

    /// Retrieves the extra-info pointer previously set via
    /// [`CastFunctionBuilder::extra_info`].
    ///
    /// Returns a raw `*mut c_void`.  Cast it back to your concrete type.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the cast function is
    /// registered and `DuckDB` has not yet called the destructor.
    #[must_use]
    pub unsafe fn get_extra_info(&self) -> *mut c_void {
        // SAFETY: self.info is valid per constructor contract.
        unsafe { duckdb_cast_function_get_extra_info(self.info) }
    }

    /// Records the error `DuckDB` raises when the callback returns `false`.
    ///
    /// Use this only in [`CastMode::Normal`], where returning `false` fails the
    /// query with this message as a `Conversion Error`. In [`CastMode::Try`]
    /// the message is kept but the query does not fail, and no row becomes
    /// `NULL` because of it: use [`set_row_error`][Self::set_row_error] for
    /// every failed row instead.
    ///
    /// An interior null byte in `message` is replaced by `?`. An empty message is replaced by
    /// [`EMPTY_ERROR_PLACEHOLDER`][Self::EMPTY_ERROR_PLACEHOLDER].
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = error_cstring(message, Self::EMPTY_ERROR_PLACEHOLDER);
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_cast_function_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Reports a per-row conversion failure and sets `row` of `output` to
    /// `NULL`.
    ///
    /// `duckdb_cast_function_set_row_error` does both: it records `message` as
    /// the cast's error message and calls `FlatVector::SetNull(output, row)`
    /// (`src/main/capi/cast_function-c.cpp`). In [`CastMode::Try`] this is the
    /// only way a row becomes `NULL`; in [`CastMode::Normal`] the message is
    /// what the query fails with once the callback returns `false`.
    ///
    /// An interior null byte in `message` is replaced by `?`. An empty message is replaced by
    /// [`EMPTY_ERROR_PLACEHOLDER`][Self::EMPTY_ERROR_PLACEHOLDER].
    ///
    /// # Safety
    ///
    /// - `output` must be the same `duckdb_vector` passed to the cast callback.
    /// - `row` must be less than the `count` passed to the cast callback.
    ///   `DuckDB` does not check it: the validity-mask bounds check is a
    ///   `D_ASSERT`, compiled out of release builds
    ///   (`src/include/duckdb/common/types/validity_mask.hpp`), so an
    ///   out-of-range `row` writes past the end of the validity mask.
    pub unsafe fn set_row_error(&self, message: &str, row: idx_t, output: duckdb_vector) {
        let c_msg = error_cstring(message, Self::EMPTY_ERROR_PLACEHOLDER);
        // SAFETY: self.info is valid; output and row are caller-supplied.
        unsafe {
            duckdb_cast_function_set_row_error(self.info, c_msg.as_ptr(), row, output);
        }
    }

    /// The message [`set_error`][Self::set_error] and
    /// [`set_row_error`][Self::set_row_error] report in place of an empty one.
    ///
    /// Without it, `DuckDB` fails the query with `Conversion Error: ` followed
    /// by nothing.
    pub const EMPTY_ERROR_PLACEHOLDER: &'static str =
        "cast function reported an error without a message";
}

// ── Callback type alias ────────────────────────────────────────────────────────

/// The cast function callback signature.
///
/// - `info`   — cast function info; use [`CastFunctionInfo`] to wrap it.
/// - `count`  — number of rows in this chunk.
/// - `input`  — source vector (read from this).
/// - `output` — destination vector (write results here).
///
/// # The return value
///
/// Return `true` when every row converted. What `false` does depends on the
/// [`CastMode`]:
///
/// - **`Normal`** (`CAST`): the query fails with a `Conversion Error`
///   carrying the message from [`CastFunctionInfo::set_error`] or
///   [`set_row_error`][CastFunctionInfo::set_row_error]. Set one before
///   returning `false`. With none, a [`cast_callback!`][crate::cast_callback]
///   function reports
///   [`CAST_FAILED_WITHOUT_MESSAGE`][crate::callback::CAST_FAILED_WITHOUT_MESSAGE];
///   a hand-written callback makes `DuckDB` report `Conversion Error: `
///   followed by nothing.
/// - **`Try`** (`TRY_CAST`): **nothing**. `DuckDB` discards the return value
///   (`src/execution/expression_executor/execute_cast.cpp` calls the bound
///   cast function and ignores what it returns; `CAPICastFunction` only
///   records the message). Every row of `output` is returned as it stands —
///   a row the callback never wrote holds whatever the vector held before,
///   which can be a previous chunk's value. So in `Try` mode, call
///   [`set_row_error`][CastFunctionInfo::set_row_error] (or set the row to
///   `NULL` yourself) for **every** row that failed; returning `false` is not
///   a substitute. This matches `DuckDB`'s own casts, whose `TRY_CAST` loops
///   null each failed row individually and return `false` only as a summary.
///
/// A panic inside a [`cast_callback!`][crate::cast_callback] body is the one
/// case quack-rs handles for you: in `Try` mode every row of the chunk is set
/// to `NULL`, because what the body had written before it panicked cannot be
/// trusted.
pub type CastFn = unsafe extern "C" fn(
    info: duckdb_function_info,
    count: idx_t,
    input: duckdb_vector,
    output: duckdb_vector,
) -> bool;

// ── Builder ────────────────────────────────────────────────────────────────────

/// Builder for registering a custom `DuckDB` cast function.
///
/// A cast function converts values from a **source** type to a **target** type.
/// Registering a cast lets `DuckDB` use it both for explicit
/// `CAST(x AS Target)` syntax and (if an implicit cost is set) for automatic
/// coercions.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo, CastMode};
/// use quack_rs::types::TypeId;
/// use libduckdb_sys::{duckdb_function_info, duckdb_vector, idx_t};
///
/// unsafe extern "C" fn my_cast(
///     _info: duckdb_function_info,
///     _count: idx_t,
///     _input: duckdb_vector,
///     _output: duckdb_vector,
/// ) -> bool {
///     true // implement real conversion here
/// }
///
/// // fn register(con: libduckdb_sys::duckdb_connection)
/// //     -> Result<(), quack_rs::error::ExtensionError>
/// // {
/// //     unsafe {
/// //         CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
/// //             .function(my_cast)
/// //             .register(con)
/// //     }
/// // }
/// ```
#[must_use]
pub struct CastFunctionBuilder {
    source: Option<TypeId>,
    source_logical: Option<LogicalType>,
    target: Option<TypeId>,
    target_logical: Option<LogicalType>,
    function: Option<CastFn>,
    implicit_cost: Option<i64>,
    extra_info: Option<crate::extra_info::ExtraInfo>,
}

// SAFETY: moving a builder to another thread moves ownership of three kinds of
// raw handle, and none of them has thread affinity:
// - `LogicalType` owns a heap-allocated C++ `duckdb::LogicalType`; its type info
//   is held by a `shared_ptr` (atomic refcount), and `duckdb_destroy_logical_type`
//   is a plain `delete` that any thread may run.
// - `CastFn` is a plain function pointer.
// - `ExtraInfo` holds the user's `extra_info` pointer and may run its destructor
//   on whichever thread drops the builder. That is only sound for a pointee that
//   is `Send`, and `extra_info` is an `unsafe fn` whose contract requires exactly
//   that (`Send + Sync`, because DuckDB reads it from every thread that casts).
// The builder is not `Sync`: nothing here is shared, only moved.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for CastFunctionBuilder {}

impl CastFunctionBuilder {
    /// Creates a new builder that will cast `source` values into `target` values.
    pub const fn new(source: TypeId, target: TypeId) -> Self {
        Self {
            source: Some(source),
            source_logical: None,
            target: Some(target),
            target_logical: None,
            function: None,
            implicit_cost: None,
            extra_info: None,
        }
    }

    /// Creates a new builder using [`LogicalType`]s for source and target.
    ///
    /// Use this when the source or target types are complex (e.g.
    /// `DECIMAL(18, 3)`, `LIST(VARCHAR)`, etc.) and cannot be expressed as
    /// simple [`TypeId`] values.
    pub fn new_logical(source: LogicalType, target: LogicalType) -> Self {
        Self {
            source: None,
            source_logical: Some(source),
            target: None,
            target_logical: Some(target),
            function: None,
            implicit_cost: None,
            extra_info: None,
        }
    }

    /// Returns the source type this cast converts from (if set via [`new`][Self::new]).
    ///
    /// Returns `None` if the source was set via [`new_logical`][Self::new_logical].
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub const fn source(&self) -> Option<TypeId> {
        self.source
    }

    /// Returns the target type this cast converts to (if set via [`new`][Self::new]).
    ///
    /// Returns `None` if the target was set via [`new_logical`][Self::new_logical].
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub const fn target(&self) -> Option<TypeId> {
        self.target
    }

    /// Sets the cast callback.
    pub fn function(mut self, f: CastFn) -> Self {
        self.function = Some(f);
        self
    }

    /// Sets the implicit cast cost.
    ///
    /// When a non-negative cost is provided, `DuckDB` may use this cast
    /// automatically in expressions where an implicit coercion is needed.
    /// Lower cost means higher priority. A negative cost or omitting this
    /// method makes the cast explicit-only.
    pub const fn implicit_cost(mut self, cost: i64) -> Self {
        self.implicit_cost = Some(cost);
        self
    }

    /// Attaches extra data to the cast function.
    ///
    /// The pointer is available inside the callback via
    /// [`CastFunctionInfo::get_extra_info`].
    ///
    /// # Safety
    ///
    /// - `ptr` must remain valid until `DuckDB` calls `destroy`, or for the
    ///   lifetime of the database if `destroy` is `None`.
    /// - The pointee must be `Send + Sync`. `DuckDB` hands the same pointer to
    ///   the cast callback on every thread that executes the cast, possibly at
    ///   the same moment, and `destroy` runs on whichever thread releases the
    ///   last reference — or, if the builder is dropped unregistered, on the
    ///   thread that drops it (the builder is `Send`). Shared mutable state
    ///   behind the pointer needs a `Mutex` or atomics.
    pub unsafe fn extra_info(
        mut self,
        ptr: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        // SAFETY: forwarded from this method's own contract.
        self.extra_info = Some(unsafe { crate::extra_info::ExtraInfo::new(ptr, destroy) });
        self
    }

    /// The completeness checks that need no `DuckDB` call: a function
    /// callback, a source type and a target type.
    /// [`MockRegistrar`][crate::testing::MockRegistrar] runs them too.
    pub(crate) fn check_parts(&self) -> Result<(), ExtensionError> {
        if self.function.is_none() {
            return Err(ExtensionError::new("cast function callback not set"));
        }
        if self.source.is_none() && self.source_logical.is_none() {
            return Err(ExtensionError::new("cast source type not set"));
        }
        if self.target.is_none() && self.target_logical.is_none() {
            return Err(ExtensionError::new("cast target type not set"));
        }
        Ok(())
    }

    /// Registers the cast function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The function callback, source type or target type was not set.
    /// - The source or target type is a bare composite [`TypeId`] (`DECIMAL`,
    ///   `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY`, `UNION`); build it as a
    ///   [`LogicalType`] and use [`new_logical`][Self::new_logical].
    /// - `con` is null.
    /// - The source or target type is, or contains, `ANY` or `INVALID`
    ///   (`DuckDB` refuses these).
    /// - `DuckDB` reports a registration failure.
    ///
    /// # `extra_info` ownership
    ///
    /// Every failure above is detected in Rust before `DuckDB` is called, so
    /// the builder still owns any [`extra_info`][Self::extra_info] and frees
    /// it (exactly once) when it is dropped. `duckdb_register_cast_function`
    /// itself returns early for exactly those conditions *before* taking
    /// ownership, which is why they are checked here first. The one remaining
    /// failure — an exception inside `DuckDB` while installing the cast — is
    /// ambiguous (it may or may not have taken ownership already), so the
    /// allocation is left to `DuckDB` in that case: a possible leak, never a
    /// double free.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        // See `ScalarFunctionBuilder::register` -- validate before allocating.
        // The checks that need no DuckDB call go first.
        self.check_parts()?;
        let function = self
            .function
            .ok_or_else(|| ExtensionError::new("cast function callback not set"))?;
        if let Some(id) = self.source {
            LogicalType::check_slot(id, "cast function source type")?;
        }
        if let Some(id) = self.target {
            LogicalType::check_slot(id, "cast function target type")?;
        }
        if con.is_null() {
            return Err(ExtensionError::new(
                "cast function registration: connection is null",
            ));
        }

        // Resolve source and target types: prefer explicit LogicalType over
        // TypeId. Done before any DuckDB handle is created, so an early return
        // leaks nothing.
        let src_lt = if let Some(lt) = self.source_logical {
            lt
        } else if let Some(id) = self.source {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("cast source type not set"));
        };
        let tgt_lt = if let Some(lt) = self.target_logical {
            lt
        } else if let Some(id) = self.target {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("cast target type not set"));
        };
        for (lt, slot) in [(&src_lt, "source"), (&tgt_lt, "target")] {
            // SAFETY: `lt` owns a live handle.
            if unsafe { crate::table::type_check::contains_any_or_invalid(lt.as_raw()) } {
                return Err(ExtensionError::new(format!(
                    "cast function {slot} type must not be or contain ANY or INVALID; \
                     DuckDB refuses to register such a cast"
                )));
            }
        }

        // SAFETY: allocates a new cast function handle.
        let mut cast = unsafe { duckdb_create_cast_function() };

        // SAFETY: cast and both type handles are valid; DuckDB copies the types.
        unsafe {
            duckdb_cast_function_set_source_type(cast, src_lt.as_raw());
            duckdb_cast_function_set_target_type(cast, tgt_lt.as_raw());
        }

        // Set callback
        // SAFETY: function is a valid extern "C" fn pointer.
        unsafe {
            duckdb_cast_function_set_function(cast, Some(function));
        }

        // Set implicit cost if requested
        if let Some(cost) = self.implicit_cost {
            // SAFETY: cast is a valid handle.
            unsafe {
                duckdb_cast_function_set_implicit_cast_cost(cast, cost);
            }
        }

        // Attach extra info if provided
        if let Some(info) = self.extra_info {
            // SAFETY: ptr validity is the caller's responsibility per the safety
            // contract on extra_info().
            unsafe {
                duckdb_cast_function_set_extra_info(cast, info.data(), info.destroy());
            }
            // Every deterministic pre-ownership rejection in
            // `duckdb_register_cast_function` (null handles, missing parts,
            // ANY/INVALID types) was ruled out above, so from here DuckDB owns
            // the allocation — see "extra_info ownership" in the docs.
            info.mark_transferred();
        }

        // Register
        // SAFETY: con is a valid open connection, cast is fully configured.
        let result = unsafe { duckdb_register_cast_function(con, cast) };

        // SAFETY: cast was created above and must be destroyed after use.
        unsafe {
            duckdb_destroy_cast_function(&raw mut cast);
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_cast_function failed: {}",
                crate::error::REGISTRATION_FAILURE_HINT
            )))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

crate::debug_repr::impl_handle_debug!(CastFunctionInfo.info);

impl core::fmt::Debug for CastFunctionBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use crate::debug_repr::Callback;
        f.debug_struct("CastFunctionBuilder")
            .field("source", &self.source)
            .field("source_logical", &self.source_logical)
            .field("target", &self.target)
            .field("target_logical", &self.target_logical)
            .field("function", &Callback::of(&self.function))
            .field("implicit_cost", &self.implicit_cost)
            .field("extra_info", &Callback::of(&self.extra_info))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libduckdb_sys::{duckdb_function_info, duckdb_vector, idx_t};

    unsafe extern "C" fn noop_cast(
        _: duckdb_function_info,
        _: idx_t,
        _: duckdb_vector,
        _: duckdb_vector,
    ) -> bool {
        true
    }

    #[test]
    fn builder_stores_source_and_target() {
        let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer);
        assert_eq!(b.source(), Some(TypeId::Varchar));
        assert_eq!(b.target(), Some(TypeId::Integer));
    }

    #[test]
    fn builder_stores_function() {
        let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer).function(noop_cast);
        assert!(b.function.is_some());
    }

    #[test]
    fn builder_stores_implicit_cost() {
        let b = CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer).implicit_cost(10);
        assert_eq!(b.implicit_cost, Some(10));
    }

    #[test]
    fn register_without_a_function_is_an_error_before_touching_duckdb() {
        // The function check runs before any DuckDB call, so a null
        // connection is never dereferenced.
        // SAFETY: `register` returns before using `con`.
        let err = unsafe {
            CastFunctionBuilder::new(TypeId::BigInt, TypeId::Double).register(std::ptr::null_mut())
        }
        .expect_err("a builder without a function must be refused");
        assert!(err.as_str().contains("callback not set"), "{err}");
    }

    #[test]
    fn cast_mode_from_raw_normal() {
        use libduckdb_sys::duckdb_cast_mode_DUCKDB_CAST_NORMAL;
        assert_eq!(
            CastMode::from_raw(duckdb_cast_mode_DUCKDB_CAST_NORMAL),
            CastMode::Normal
        );
    }

    #[test]
    fn cast_mode_from_raw_try() {
        assert_eq!(
            CastMode::from_raw(duckdb_cast_mode_DUCKDB_CAST_TRY),
            CastMode::Try
        );
    }

    #[test]
    fn cast_function_info_round_trips_its_handle() {
        let raw = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
        // SAFETY: the handle is only stored and read back, never passed to DuckDB.
        let info = unsafe { CastFunctionInfo::new(raw) };
        assert_eq!(info.info, raw);
    }
}
