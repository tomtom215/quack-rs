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

/// Converts a `&str` to `CString` without panicking.
#[mutants::skip] // private FFI helper — tested in replacement_scan::tests
fn str_to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let pos = s.bytes().position(|b| b == 0).unwrap_or(s.len());
        CString::new(&s.as_bytes()[..pos]).unwrap_or_default()
    })
}

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
    /// If `message` contains an interior null byte it is truncated at that point.
    #[mutants::skip]
    pub fn set_error(&self, message: &str) {
        let c_msg = str_to_cstring(message);
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
    /// If `message` contains an interior null byte it is truncated at that point.
    ///
    /// # Safety
    ///
    /// `output` must be the same `duckdb_vector` passed to the cast callback.
    pub unsafe fn set_row_error(&self, message: &str, row: idx_t, output: duckdb_vector) {
        let c_msg = str_to_cstring(message);
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
    source: Option<TypeId>,
    source_logical: Option<LogicalType>,
    target: Option<TypeId>,
    target_logical: Option<LogicalType>,
    function: Option<CastFn>,
    implicit_cost: Option<i64>,
    extra_info: Option<crate::extra_info::ExtraInfo>,
}

// SAFETY: CastFunctionBuilder owns the extra_info pointer and LogicalType handles
// until registration. The raw pointers are only sent across threads as part of the
// builder, which extension authors typically use on a single thread.
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
    /// `ptr` must remain valid until `DuckDB` calls `destroy`, or for the
    /// lifetime of the database if `destroy` is `None`.
    pub unsafe fn extra_info(
        mut self,
        ptr: *mut c_void,
        destroy: duckdb_delete_callback_t,
    ) -> Self {
        // SAFETY: forwarded from this method's own contract.
        self.extra_info = Some(unsafe { crate::extra_info::ExtraInfo::new(ptr, destroy) });
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
        // See `ScalarFunctionBuilder::register` -- validate before allocating.
        if let Some(id) = self.source {
            LogicalType::check_slot(id, "cast function source type")?;
        }
        if let Some(id) = self.target {
            LogicalType::check_slot(id, "cast function target type")?;
        }
        let function = self
            .function
            .ok_or_else(|| ExtensionError::new("cast function callback not set"))?;

        // SAFETY: allocates a new cast function handle.
        let mut cast = unsafe { duckdb_create_cast_function() };

        // Resolve source type: prefer explicit LogicalType over TypeId.
        let src_lt = if let Some(lt) = self.source_logical {
            lt
        } else if let Some(id) = self.source {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("cast source type not set"));
        };
        // SAFETY: cast and src_lt.as_raw() are valid.
        unsafe {
            duckdb_cast_function_set_source_type(cast, src_lt.as_raw());
        }

        // Resolve target type: prefer explicit LogicalType over TypeId.
        let tgt_lt = if let Some(lt) = self.target_logical {
            lt
        } else if let Some(id) = self.target {
            LogicalType::new(id)
        } else {
            return Err(ExtensionError::new("cast target type not set"));
        };
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
        if let Some(info) = self.extra_info {
            // SAFETY: ptr validity is the caller's responsibility per the safety
            // contract on extra_info().
            unsafe {
                duckdb_cast_function_set_extra_info(cast, info.data(), info.destroy());
                // DuckDB owns the allocation from here.
                info.mark_transferred();
            }
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
            Err(ExtensionError::new("duckdb_register_cast_function failed"))
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
