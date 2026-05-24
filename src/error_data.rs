// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Structured error data (`DuckDB` 1.5.0+).
//!
//! [`ErrorData`] is an RAII wrapper around `DuckDB`'s `duckdb_error_data` handle —
//! the structured error type returned by several 1.5.0 C API surfaces, including
//! expression folding ([`Expression::fold`][crate::expression::Expression::fold]),
//! the file system API ([`crate::file_system`]), and the appender error accessor
//! ([`Appender::error_data`][crate::appender::Appender::error_data]).
//!
//! Unlike a bare error string, an `ErrorData` carries both a human-readable
//! message and a machine-readable [`DuckDbErrorType`] category, so an extension
//! can branch on the kind of failure (e.g. distinguishing `IO` from
//! `OutOfMemory`).
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::error_data::{DuckDbErrorType, ErrorData};
//!
//! // Construct a structured error to hand back to DuckDB.
//! let err = ErrorData::new(DuckDbErrorType::InvalidInput, "row index out of range");
//! assert!(err.has_error());
//! assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
//! ```

use std::ffi::{CStr, CString};
use std::fmt;

use libduckdb_sys::{
    duckdb_create_error_data, duckdb_destroy_error_data, duckdb_error_data,
    duckdb_error_data_error_type, duckdb_error_data_has_error, duckdb_error_data_message,
    duckdb_error_type, duckdb_error_type_DUCKDB_ERROR_BINDER,
    duckdb_error_type_DUCKDB_ERROR_CATALOG, duckdb_error_type_DUCKDB_ERROR_CONNECTION,
    duckdb_error_type_DUCKDB_ERROR_CONSTRAINT, duckdb_error_type_DUCKDB_ERROR_CONVERSION,
    duckdb_error_type_DUCKDB_ERROR_DECIMAL, duckdb_error_type_DUCKDB_ERROR_DEPENDENCY,
    duckdb_error_type_DUCKDB_ERROR_DIVIDE_BY_ZERO, duckdb_error_type_DUCKDB_ERROR_EXECUTOR,
    duckdb_error_type_DUCKDB_ERROR_EXPRESSION, duckdb_error_type_DUCKDB_ERROR_FATAL,
    duckdb_error_type_DUCKDB_ERROR_HTTP, duckdb_error_type_DUCKDB_ERROR_INDEX,
    duckdb_error_type_DUCKDB_ERROR_INTERNAL, duckdb_error_type_DUCKDB_ERROR_INTERRUPT,
    duckdb_error_type_DUCKDB_ERROR_INVALID, duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT,
    duckdb_error_type_DUCKDB_ERROR_INVALID_TYPE, duckdb_error_type_DUCKDB_ERROR_IO,
    duckdb_error_type_DUCKDB_ERROR_MISMATCH_TYPE, duckdb_error_type_DUCKDB_ERROR_MISSING_EXTENSION,
    duckdb_error_type_DUCKDB_ERROR_NETWORK, duckdb_error_type_DUCKDB_ERROR_NOT_IMPLEMENTED,
    duckdb_error_type_DUCKDB_ERROR_NULL_POINTER, duckdb_error_type_DUCKDB_ERROR_OBJECT_SIZE,
    duckdb_error_type_DUCKDB_ERROR_OPTIMIZER, duckdb_error_type_DUCKDB_ERROR_OUT_OF_MEMORY,
    duckdb_error_type_DUCKDB_ERROR_OUT_OF_RANGE,
    duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_ALLOWED,
    duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_RESOLVED, duckdb_error_type_DUCKDB_ERROR_PARSER,
    duckdb_error_type_DUCKDB_ERROR_PERMISSION, duckdb_error_type_DUCKDB_ERROR_PLANNER,
    duckdb_error_type_DUCKDB_ERROR_SCHEDULER, duckdb_error_type_DUCKDB_ERROR_SERIALIZATION,
    duckdb_error_type_DUCKDB_ERROR_SETTINGS, duckdb_error_type_DUCKDB_ERROR_STAT,
    duckdb_error_type_DUCKDB_ERROR_SYNTAX, duckdb_error_type_DUCKDB_ERROR_TRANSACTION,
    duckdb_error_type_DUCKDB_ERROR_UNKNOWN_TYPE, duckdb_valid_utf8_check, idx_t,
};

use crate::error::ExtensionError;

/// The category of a `DuckDB` error, mirroring `duckdb_error_type`.
///
/// Unknown or future error categories map to [`DuckDbErrorType::Invalid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DuckDbErrorType {
    /// No specific category / invalid.
    Invalid,
    /// Value out of the representable range.
    OutOfRange,
    /// A type conversion failed.
    Conversion,
    /// An unknown type was encountered.
    UnknownType,
    /// A decimal-specific error.
    Decimal,
    /// A type mismatch.
    MismatchType,
    /// Division by zero.
    DivideByZero,
    /// An object-size error.
    ObjectSize,
    /// An invalid type was supplied.
    InvalidType,
    /// A (de)serialization error.
    Serialization,
    /// A transaction error.
    Transaction,
    /// The requested feature is not implemented.
    NotImplemented,
    /// An error in an expression.
    Expression,
    /// A catalog error (e.g. missing table).
    Catalog,
    /// A parser error.
    Parser,
    /// A planner error.
    Planner,
    /// A scheduler error.
    Scheduler,
    /// An executor error.
    Executor,
    /// A constraint violation.
    Constraint,
    /// An index error.
    Index,
    /// A statistics error.
    Stat,
    /// A connection error.
    Connection,
    /// A syntax error.
    Syntax,
    /// A settings error.
    Settings,
    /// A binder error.
    Binder,
    /// A network error.
    Network,
    /// An optimizer error.
    Optimizer,
    /// A null-pointer error.
    NullPointer,
    /// An I/O error.
    Io,
    /// The query was interrupted.
    Interrupt,
    /// A fatal error — the database is in an unusable state.
    Fatal,
    /// An internal error (a `DuckDB` bug).
    Internal,
    /// Invalid input was supplied.
    InvalidInput,
    /// Out of memory.
    OutOfMemory,
    /// A permission error.
    Permission,
    /// A prepared-statement parameter could not be resolved.
    ParameterNotResolved,
    /// A prepared-statement parameter is not allowed here.
    ParameterNotAllowed,
    /// A dependency error.
    Dependency,
    /// An HTTP error.
    Http,
    /// A required extension is missing.
    MissingExtension,
}

impl DuckDbErrorType {
    /// Converts to the `DuckDB` C API constant.
    #[must_use]
    pub(crate) const fn to_raw(self) -> duckdb_error_type {
        match self {
            Self::Invalid => duckdb_error_type_DUCKDB_ERROR_INVALID,
            Self::OutOfRange => duckdb_error_type_DUCKDB_ERROR_OUT_OF_RANGE,
            Self::Conversion => duckdb_error_type_DUCKDB_ERROR_CONVERSION,
            Self::UnknownType => duckdb_error_type_DUCKDB_ERROR_UNKNOWN_TYPE,
            Self::Decimal => duckdb_error_type_DUCKDB_ERROR_DECIMAL,
            Self::MismatchType => duckdb_error_type_DUCKDB_ERROR_MISMATCH_TYPE,
            Self::DivideByZero => duckdb_error_type_DUCKDB_ERROR_DIVIDE_BY_ZERO,
            Self::ObjectSize => duckdb_error_type_DUCKDB_ERROR_OBJECT_SIZE,
            Self::InvalidType => duckdb_error_type_DUCKDB_ERROR_INVALID_TYPE,
            Self::Serialization => duckdb_error_type_DUCKDB_ERROR_SERIALIZATION,
            Self::Transaction => duckdb_error_type_DUCKDB_ERROR_TRANSACTION,
            Self::NotImplemented => duckdb_error_type_DUCKDB_ERROR_NOT_IMPLEMENTED,
            Self::Expression => duckdb_error_type_DUCKDB_ERROR_EXPRESSION,
            Self::Catalog => duckdb_error_type_DUCKDB_ERROR_CATALOG,
            Self::Parser => duckdb_error_type_DUCKDB_ERROR_PARSER,
            Self::Planner => duckdb_error_type_DUCKDB_ERROR_PLANNER,
            Self::Scheduler => duckdb_error_type_DUCKDB_ERROR_SCHEDULER,
            Self::Executor => duckdb_error_type_DUCKDB_ERROR_EXECUTOR,
            Self::Constraint => duckdb_error_type_DUCKDB_ERROR_CONSTRAINT,
            Self::Index => duckdb_error_type_DUCKDB_ERROR_INDEX,
            Self::Stat => duckdb_error_type_DUCKDB_ERROR_STAT,
            Self::Connection => duckdb_error_type_DUCKDB_ERROR_CONNECTION,
            Self::Syntax => duckdb_error_type_DUCKDB_ERROR_SYNTAX,
            Self::Settings => duckdb_error_type_DUCKDB_ERROR_SETTINGS,
            Self::Binder => duckdb_error_type_DUCKDB_ERROR_BINDER,
            Self::Network => duckdb_error_type_DUCKDB_ERROR_NETWORK,
            Self::Optimizer => duckdb_error_type_DUCKDB_ERROR_OPTIMIZER,
            Self::NullPointer => duckdb_error_type_DUCKDB_ERROR_NULL_POINTER,
            Self::Io => duckdb_error_type_DUCKDB_ERROR_IO,
            Self::Interrupt => duckdb_error_type_DUCKDB_ERROR_INTERRUPT,
            Self::Fatal => duckdb_error_type_DUCKDB_ERROR_FATAL,
            Self::Internal => duckdb_error_type_DUCKDB_ERROR_INTERNAL,
            Self::InvalidInput => duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT,
            Self::OutOfMemory => duckdb_error_type_DUCKDB_ERROR_OUT_OF_MEMORY,
            Self::Permission => duckdb_error_type_DUCKDB_ERROR_PERMISSION,
            Self::ParameterNotResolved => duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_RESOLVED,
            Self::ParameterNotAllowed => duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_ALLOWED,
            Self::Dependency => duckdb_error_type_DUCKDB_ERROR_DEPENDENCY,
            Self::Http => duckdb_error_type_DUCKDB_ERROR_HTTP,
            Self::MissingExtension => duckdb_error_type_DUCKDB_ERROR_MISSING_EXTENSION,
        }
    }

    /// Converts from the `DuckDB` C API constant. Unknown values map to
    /// [`Invalid`][DuckDbErrorType::Invalid].
    #[must_use]
    pub(crate) const fn from_raw(raw: duckdb_error_type) -> Self {
        match raw {
            x if x == duckdb_error_type_DUCKDB_ERROR_OUT_OF_RANGE => Self::OutOfRange,
            x if x == duckdb_error_type_DUCKDB_ERROR_CONVERSION => Self::Conversion,
            x if x == duckdb_error_type_DUCKDB_ERROR_UNKNOWN_TYPE => Self::UnknownType,
            x if x == duckdb_error_type_DUCKDB_ERROR_DECIMAL => Self::Decimal,
            x if x == duckdb_error_type_DUCKDB_ERROR_MISMATCH_TYPE => Self::MismatchType,
            x if x == duckdb_error_type_DUCKDB_ERROR_DIVIDE_BY_ZERO => Self::DivideByZero,
            x if x == duckdb_error_type_DUCKDB_ERROR_OBJECT_SIZE => Self::ObjectSize,
            x if x == duckdb_error_type_DUCKDB_ERROR_INVALID_TYPE => Self::InvalidType,
            x if x == duckdb_error_type_DUCKDB_ERROR_SERIALIZATION => Self::Serialization,
            x if x == duckdb_error_type_DUCKDB_ERROR_TRANSACTION => Self::Transaction,
            x if x == duckdb_error_type_DUCKDB_ERROR_NOT_IMPLEMENTED => Self::NotImplemented,
            x if x == duckdb_error_type_DUCKDB_ERROR_EXPRESSION => Self::Expression,
            x if x == duckdb_error_type_DUCKDB_ERROR_CATALOG => Self::Catalog,
            x if x == duckdb_error_type_DUCKDB_ERROR_PARSER => Self::Parser,
            x if x == duckdb_error_type_DUCKDB_ERROR_PLANNER => Self::Planner,
            x if x == duckdb_error_type_DUCKDB_ERROR_SCHEDULER => Self::Scheduler,
            x if x == duckdb_error_type_DUCKDB_ERROR_EXECUTOR => Self::Executor,
            x if x == duckdb_error_type_DUCKDB_ERROR_CONSTRAINT => Self::Constraint,
            x if x == duckdb_error_type_DUCKDB_ERROR_INDEX => Self::Index,
            x if x == duckdb_error_type_DUCKDB_ERROR_STAT => Self::Stat,
            x if x == duckdb_error_type_DUCKDB_ERROR_CONNECTION => Self::Connection,
            x if x == duckdb_error_type_DUCKDB_ERROR_SYNTAX => Self::Syntax,
            x if x == duckdb_error_type_DUCKDB_ERROR_SETTINGS => Self::Settings,
            x if x == duckdb_error_type_DUCKDB_ERROR_BINDER => Self::Binder,
            x if x == duckdb_error_type_DUCKDB_ERROR_NETWORK => Self::Network,
            x if x == duckdb_error_type_DUCKDB_ERROR_OPTIMIZER => Self::Optimizer,
            x if x == duckdb_error_type_DUCKDB_ERROR_NULL_POINTER => Self::NullPointer,
            x if x == duckdb_error_type_DUCKDB_ERROR_IO => Self::Io,
            x if x == duckdb_error_type_DUCKDB_ERROR_INTERRUPT => Self::Interrupt,
            x if x == duckdb_error_type_DUCKDB_ERROR_FATAL => Self::Fatal,
            x if x == duckdb_error_type_DUCKDB_ERROR_INTERNAL => Self::Internal,
            x if x == duckdb_error_type_DUCKDB_ERROR_INVALID_INPUT => Self::InvalidInput,
            x if x == duckdb_error_type_DUCKDB_ERROR_OUT_OF_MEMORY => Self::OutOfMemory,
            x if x == duckdb_error_type_DUCKDB_ERROR_PERMISSION => Self::Permission,
            x if x == duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_RESOLVED => {
                Self::ParameterNotResolved
            }
            x if x == duckdb_error_type_DUCKDB_ERROR_PARAMETER_NOT_ALLOWED => {
                Self::ParameterNotAllowed
            }
            x if x == duckdb_error_type_DUCKDB_ERROR_DEPENDENCY => Self::Dependency,
            x if x == duckdb_error_type_DUCKDB_ERROR_HTTP => Self::Http,
            x if x == duckdb_error_type_DUCKDB_ERROR_MISSING_EXTENSION => Self::MissingExtension,
            _ => Self::Invalid,
        }
    }

    /// Returns a short, human-readable label for this error category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::OutOfRange => "out of range",
            Self::Conversion => "conversion",
            Self::UnknownType => "unknown type",
            Self::Decimal => "decimal",
            Self::MismatchType => "type mismatch",
            Self::DivideByZero => "divide by zero",
            Self::ObjectSize => "object size",
            Self::InvalidType => "invalid type",
            Self::Serialization => "serialization",
            Self::Transaction => "transaction",
            Self::NotImplemented => "not implemented",
            Self::Expression => "expression",
            Self::Catalog => "catalog",
            Self::Parser => "parser",
            Self::Planner => "planner",
            Self::Scheduler => "scheduler",
            Self::Executor => "executor",
            Self::Constraint => "constraint",
            Self::Index => "index",
            Self::Stat => "statistics",
            Self::Connection => "connection",
            Self::Syntax => "syntax",
            Self::Settings => "settings",
            Self::Binder => "binder",
            Self::Network => "network",
            Self::Optimizer => "optimizer",
            Self::NullPointer => "null pointer",
            Self::Io => "I/O",
            Self::Interrupt => "interrupt",
            Self::Fatal => "fatal",
            Self::Internal => "internal",
            Self::InvalidInput => "invalid input",
            Self::OutOfMemory => "out of memory",
            Self::Permission => "permission",
            Self::ParameterNotResolved => "parameter not resolved",
            Self::ParameterNotAllowed => "parameter not allowed",
            Self::Dependency => "dependency",
            Self::Http => "HTTP",
            Self::MissingExtension => "missing extension",
        }
    }
}

impl fmt::Display for DuckDbErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RAII wrapper for a `duckdb_error_data` handle.
///
/// Automatically destroyed when dropped. Carries a structured error category
/// ([`DuckDbErrorType`]) and a human-readable message.
///
/// Implements [`Display`][std::fmt::Display] and [`std::error::Error`], and
/// converts into [`ExtensionError`] via [`From`] (or
/// [`into_extension_error`][ErrorData::into_extension_error]) so it can be
/// propagated with `?`.
pub struct ErrorData {
    raw: duckdb_error_data,
}

impl ErrorData {
    /// Creates a new structured error with the given category and message.
    ///
    /// If `message` contains an interior null byte it is truncated at that point.
    #[must_use]
    pub fn new(error_type: DuckDbErrorType, message: &str) -> Self {
        let c_msg = CString::new(message).unwrap_or_else(|_| {
            let pos = message
                .bytes()
                .position(|b| b == 0)
                .unwrap_or(message.len());
            CString::new(&message.as_bytes()[..pos]).unwrap_or_default()
        });
        // SAFETY: error_type.to_raw() is a valid duckdb_error_type and c_msg is a
        // valid null-terminated string for the duration of the call.
        let raw = unsafe { duckdb_create_error_data(error_type.to_raw(), c_msg.as_ptr()) };
        Self { raw }
    }

    /// Wraps a raw `duckdb_error_data` handle, taking ownership.
    ///
    /// # Safety
    ///
    /// `raw` must be a `duckdb_error_data` returned by a `DuckDB` API call (it may
    /// be null). The caller must not destroy the handle after this call.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_error_data) -> Self {
        Self { raw }
    }

    /// Returns the raw handle without consuming the `ErrorData`.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_error_data {
        self.raw
    }

    /// Returns `true` if the underlying handle is null.
    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.raw.is_null()
    }

    /// Returns `true` if this handle represents an actual error.
    #[must_use]
    pub fn has_error(&self) -> bool {
        if self.raw.is_null() {
            return false;
        }
        // SAFETY: self.raw is a non-null, valid duckdb_error_data.
        unsafe { duckdb_error_data_has_error(self.raw) }
    }

    /// Returns the error category.
    #[must_use]
    pub fn error_type(&self) -> DuckDbErrorType {
        if self.raw.is_null() {
            return DuckDbErrorType::Invalid;
        }
        // SAFETY: self.raw is a non-null, valid duckdb_error_data.
        let raw = unsafe { duckdb_error_data_error_type(self.raw) };
        DuckDbErrorType::from_raw(raw)
    }

    /// Returns the error message, or `None` if the handle is null or the message
    /// is not valid UTF-8.
    ///
    /// The returned string is owned by `DuckDB` and copied into a Rust `String`.
    #[must_use]
    pub fn message(&self) -> Option<String> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: self.raw is a non-null, valid duckdb_error_data. The returned
        // pointer is owned by the error data and remains valid while it lives.
        let ptr = unsafe { duckdb_error_data_message(self.raw) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: ptr is a valid null-terminated string owned by the error data.
        unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .ok()
            .map(String::from)
    }

    /// Consumes the `ErrorData` and converts it into an [`ExtensionError`].
    ///
    /// Useful for propagating a structured `DuckDB` error through `?`.
    #[must_use]
    pub fn into_extension_error(self) -> ExtensionError {
        let msg = self
            .message()
            .unwrap_or_else(|| "unknown DuckDB error".to_owned());
        ExtensionError::new(msg)
    }

    /// Consumes the `ErrorData` and returns the raw handle.
    ///
    /// The caller takes ownership and must call `duckdb_destroy_error_data`.
    #[inline]
    #[must_use]
    pub const fn into_raw(self) -> duckdb_error_data {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }
}

impl Drop for ErrorData {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: self.raw is a valid duckdb_error_data that we own.
            unsafe { duckdb_destroy_error_data(&raw mut self.raw) };
        }
    }
}

impl fmt::Debug for ErrorData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorData")
            .field("error_type", &self.error_type())
            .field("message", &self.message())
            .finish()
    }
}

impl fmt::Display for ErrorData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.message() {
            Some(msg) => write!(f, "{}: {msg}", self.error_type()),
            None => f.write_str(self.error_type().as_str()),
        }
    }
}

impl std::error::Error for ErrorData {}

impl From<ErrorData> for ExtensionError {
    #[inline]
    fn from(err: ErrorData) -> Self {
        err.into_extension_error()
    }
}

/// Checks whether `bytes` form a valid UTF-8 string according to `DuckDB`'s
/// validator (`DuckDB` 1.5.0+).
///
/// `DuckDB` enforces stricter rules than Rust in some cases (e.g. rejecting
/// certain code points), so this is useful when validating externally-sourced
/// bytes before handing them to `DuckDB` string APIs.
///
/// # Errors
///
/// Returns the structured [`ErrorData`] describing the first validation failure.
pub fn check_valid_utf8(bytes: &[u8]) -> Result<(), ErrorData> {
    let len = idx_t::try_from(bytes.len()).unwrap_or(idx_t::MAX);
    // SAFETY: bytes.as_ptr() is valid for `len` bytes; DuckDB only reads them.
    let raw = unsafe { duckdb_valid_utf8_check(bytes.as_ptr().cast(), len) };
    // SAFETY: duckdb_valid_utf8_check returns an owned duckdb_error_data handle.
    let err = unsafe { ErrorData::from_raw(raw) };
    if err.has_error() {
        Err(err)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_VARIANTS: [DuckDbErrorType; 40] = [
        DuckDbErrorType::Invalid,
        DuckDbErrorType::OutOfRange,
        DuckDbErrorType::Conversion,
        DuckDbErrorType::UnknownType,
        DuckDbErrorType::Decimal,
        DuckDbErrorType::MismatchType,
        DuckDbErrorType::DivideByZero,
        DuckDbErrorType::ObjectSize,
        DuckDbErrorType::InvalidType,
        DuckDbErrorType::Serialization,
        DuckDbErrorType::Transaction,
        DuckDbErrorType::NotImplemented,
        DuckDbErrorType::Expression,
        DuckDbErrorType::Catalog,
        DuckDbErrorType::Parser,
        DuckDbErrorType::Planner,
        DuckDbErrorType::Scheduler,
        DuckDbErrorType::Executor,
        DuckDbErrorType::Constraint,
        DuckDbErrorType::Index,
        DuckDbErrorType::Stat,
        DuckDbErrorType::Connection,
        DuckDbErrorType::Syntax,
        DuckDbErrorType::Settings,
        DuckDbErrorType::Binder,
        DuckDbErrorType::Network,
        DuckDbErrorType::Optimizer,
        DuckDbErrorType::NullPointer,
        DuckDbErrorType::Io,
        DuckDbErrorType::Interrupt,
        DuckDbErrorType::Fatal,
        DuckDbErrorType::Internal,
        DuckDbErrorType::InvalidInput,
        DuckDbErrorType::OutOfMemory,
        DuckDbErrorType::Permission,
        DuckDbErrorType::ParameterNotResolved,
        DuckDbErrorType::ParameterNotAllowed,
        DuckDbErrorType::Dependency,
        DuckDbErrorType::Http,
        DuckDbErrorType::MissingExtension,
    ];

    #[test]
    fn error_type_round_trip_all_variants() {
        for variant in ALL_VARIANTS {
            let raw = variant.to_raw();
            assert_eq!(
                DuckDbErrorType::from_raw(raw),
                variant,
                "round-trip failed for {variant:?}"
            );
        }
    }

    #[test]
    fn error_type_unknown_raw_maps_to_invalid() {
        assert_eq!(DuckDbErrorType::from_raw(9999), DuckDbErrorType::Invalid);
    }

    #[test]
    fn error_type_distinct_raw_values() {
        for (i, a) in ALL_VARIANTS.iter().enumerate() {
            for b in ALL_VARIANTS.iter().skip(i + 1) {
                assert_ne!(
                    a.to_raw(),
                    b.to_raw(),
                    "variants {a:?} and {b:?} share a raw value"
                );
            }
        }
    }

    #[test]
    fn null_error_data_has_no_error() {
        let err = unsafe { ErrorData::from_raw(std::ptr::null_mut()) };
        assert!(err.is_null());
        assert!(!err.has_error());
        assert_eq!(err.error_type(), DuckDbErrorType::Invalid);
        assert!(err.message().is_none());
    }

    #[test]
    fn into_raw_forgets_handle() {
        let err = unsafe { ErrorData::from_raw(std::ptr::null_mut()) };
        let raw = err.into_raw();
        assert!(raw.is_null());
    }

    #[test]
    fn error_type_display_matches_as_str() {
        for variant in ALL_VARIANTS {
            assert!(!variant.as_str().is_empty(), "{variant:?} has empty label");
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn error_type_labels_are_distinct() {
        for (i, a) in ALL_VARIANTS.iter().enumerate() {
            for b in ALL_VARIANTS.iter().skip(i + 1) {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a label");
            }
        }
    }

    #[test]
    fn null_error_data_debug_and_display() {
        // Null handle: error_type()/message() short-circuit without FFI calls,
        // so Debug/Display are safe to exercise in a unit test.
        let err = unsafe { ErrorData::from_raw(std::ptr::null_mut()) };
        assert_eq!(err.to_string(), "invalid");
        let dbg = format!("{err:?}");
        assert!(dbg.contains("ErrorData"), "debug was: {dbg}");
        assert!(dbg.contains("Invalid"), "debug was: {dbg}");
    }

    #[test]
    fn from_error_data_for_extension_error() {
        let err = unsafe { ErrorData::from_raw(std::ptr::null_mut()) };
        let ext: ExtensionError = err.into();
        assert_eq!(ext.as_str(), "unknown DuckDB error");
    }

    #[test]
    fn error_data_usable_as_std_error() {
        fn takes_error(_e: &dyn std::error::Error) {}
        let err = unsafe { ErrorData::from_raw(std::ptr::null_mut()) };
        takes_error(&err);
    }
}
