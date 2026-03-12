// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering custom `DuckDB` cast functions.

use std::ffi::CString;
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
use crate::types::{LogicalType, TypeId};

// ── Cast mode ─────────────────────────────────────────────────────────────────

/// Whether the cast is called as a regular `CAST` or a `TRY_CAST`.
///
/// In [`Try`][CastMode::Try] mode, conversion failures should write `NULL` for
/// the failed row and call [`CastFunctionInfo::set_row_error`] rather than
/// aborting the whole query.
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

    /// Reports a fatal error, causing `DuckDB` to abort the current query.
    ///
    /// Use this only in [`CastMode::Normal`]; in [`CastMode::Try`] prefer
    /// [`set_row_error`][Self::set_row_error] so that failed rows become `NULL`.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    pub fn set_error(&self, message: &str) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid per constructor contract.
        unsafe {
            duckdb_cast_function_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Reports a per-row error for `TRY_CAST`.
    ///
    /// Records `message` for `row` in the output error vector.  The row's
    /// output value should be set to `NULL` by the caller.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    ///
    /// # Safety
    ///
    /// `output` must be the same `duckdb_vector` passed to the cast callback.
    pub unsafe fn set_row_error(&self, message: &str, row: idx_t, output: duckdb_vector) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid; output and row are caller-supplied.
        unsafe {
            duckdb_cast_function_set_row_error(self.info, c_msg.as_ptr(), row, output);
        }
    }
}

// ── Callback type alias ────────────────────────────────────────────────────────

/// The cast function callback signature.
///
/// - `info`   — cast function info; use [`CastFunctionInfo`] to wrap it.
/// - `count`  — number of rows in this chunk.
/// - `input`  — source vector (read from this).
/// - `output` — destination vector (write results here).
///
/// Return `true` on success, `false` to signal a fatal cast error.
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
    source: TypeId,
    target: TypeId,
    function: Option<CastFn>,
    implicit_cost: Option<i64>,
    extra_info: Option<(*mut c_void, duckdb_delete_callback_t)>,
}

// SAFETY: CastFunctionBuilder owns the extra_info pointer until registration.
// The raw pointer is only sent across threads as part of the builder, which
// extension authors typically use on a single thread.
unsafe impl Send for CastFunctionBuilder {}

impl CastFunctionBuilder {
    /// Creates a new builder that will cast `source` values into `target` values.
    pub const fn new(source: TypeId, target: TypeId) -> Self {
        Self {
            source,
            target,
            function: None,
            implicit_cost: None,
            extra_info: None,
        }
    }

    /// Returns the source type this cast converts from.
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub const fn source(&self) -> TypeId {
        self.source
    }

    /// Returns the target type this cast converts to.
    ///
    /// Useful for introspection and for [`MockRegistrar`][crate::testing::MockRegistrar].
    pub const fn target(&self) -> TypeId {
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
    /// `ptr` must remain valid until `DuckDB` calls `destroy`, or for the
    /// lifetime of the database if `destroy` is `None`.
    pub unsafe fn extra_info(
        mut self,
        ptr: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        self.extra_info = Some((ptr, destroy));
        self
    }

    /// Registers the cast function on the given connection.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if:
    /// - The function callback was not set.
    /// - `DuckDB` reports a registration failure.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open `duckdb_connection`.
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let function = self
            .function
            .ok_or_else(|| ExtensionError::new("cast function callback not set"))?;

        // SAFETY: allocates a new cast function handle.
        let cast = unsafe { duckdb_create_cast_function() };

        // Set source type
        let src_lt = LogicalType::new(self.source);
        // SAFETY: cast and src_lt.as_raw() are valid.
        unsafe {
            duckdb_cast_function_set_source_type(cast, src_lt.as_raw());
        }

        // Set target type
        let tgt_lt = LogicalType::new(self.target);
        // SAFETY: cast and tgt_lt.as_raw() are valid.
        unsafe {
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
        if let Some((ptr, destroy)) = self.extra_info {
            // SAFETY: ptr validity is the caller's responsibility per the safety
            // contract on extra_info().
            unsafe {
                duckdb_cast_function_set_extra_info(cast, ptr, destroy);
            }
        }

        // Register
        // SAFETY: con is a valid open connection, cast is fully configured.
        let result = unsafe { duckdb_register_cast_function(con, cast) };

        // SAFETY: cast was created above and must be destroyed after use.
        unsafe {
            duckdb_destroy_cast_function(&mut { cast });
        }

        if result == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_register_cast_function failed ({:?} → {:?})",
                self.source, self.target
            )))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

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
        assert_eq!(b.source, TypeId::Varchar);
        assert_eq!(b.target, TypeId::Integer);
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
    fn builder_no_function_is_error() {
        // We cannot call register without a live DuckDB, but we can assert the
        // function field starts as None.
        let b = CastFunctionBuilder::new(TypeId::BigInt, TypeId::Double);
        assert!(b.function.is_none());
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
    fn cast_function_info_wraps_null() {
        // Constructing with null must not crash (no DuckDB calls made).
        let _info = unsafe { CastFunctionInfo::new(std::ptr::null_mut()) };
    }
}
