// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Bound expressions (`DuckDB` 1.5.0+).
//!
//! [`Expression`] is an RAII wrapper around `DuckDB`'s `duckdb_expression` handle.
//! Extension authors obtain one from a scalar function's *bind* callback via
//! [`ScalarBindInfo::argument`][crate::scalar::ScalarBindInfo::argument], which
//! lets the bind phase inspect each argument's static type and — when the
//! argument is a constant — fold it to a concrete [`Value`].
//!
//! This is the canonical way to implement scalar functions whose behaviour
//! depends on a constant argument (for example a format string or a precision)
//! that should be validated or pre-computed once at bind time rather than on
//! every row.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::scalar::{RawScalarBindInfo, ScalarBindInfo};
//!
//! unsafe extern "C" fn my_bind(info: RawScalarBindInfo) {
//!     let bind = unsafe { ScalarBindInfo::new(info) };
//!     if let Some(arg) = unsafe { bind.argument(0) } {
//!         // Inspect the argument's static return type at bind time.
//!         let _ty = arg.return_type();
//!         if arg.is_foldable() {
//!             // With a `ClientContext`, `arg.fold(&ctx)` pre-computes the constant.
//!         }
//!     }
//! }
//! ```

use libduckdb_sys::{
    duckdb_destroy_expression, duckdb_expression, duckdb_expression_fold,
    duckdb_expression_is_foldable, duckdb_expression_return_type, duckdb_value,
};

use crate::client_context::ClientContext;
use crate::error_data::{DuckDbErrorType, ErrorData};
use crate::types::LogicalType;
use crate::value::Value;

/// RAII wrapper for a `duckdb_expression`.
///
/// Automatically destroyed when dropped.
pub struct Expression {
    raw: duckdb_expression,
}

impl Expression {
    /// Wraps a raw `duckdb_expression` handle, taking ownership.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid `duckdb_expression` returned by a `DuckDB` API call
    /// (e.g. `duckdb_scalar_function_bind_get_argument`). The caller must not
    /// destroy the handle after this call.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_expression) -> Self {
        Self { raw }
    }

    /// Returns the raw handle without consuming the `Expression`.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_expression {
        self.raw
    }

    /// Returns `true` if the underlying handle is null.
    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.raw.is_null()
    }

    /// Returns the static return type of this expression, or `None` if the
    /// handle is null.
    #[must_use]
    pub fn return_type(&self) -> Option<LogicalType> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: self.raw is a non-null, valid duckdb_expression. The returned
        // logical type is owned by the caller and freed by LogicalType on drop.
        let raw = unsafe { duckdb_expression_return_type(self.raw) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw is a non-null logical type handle owned by the caller.
        Some(unsafe { LogicalType::from_raw(raw) })
    }

    /// Returns `true` if this expression is *foldable* — i.e. it is constant and
    /// can be evaluated to a single [`Value`] via [`fold`][Expression::fold]
    /// without per-row input.
    #[must_use]
    pub fn is_foldable(&self) -> bool {
        if self.raw.is_null() {
            return false;
        }
        // SAFETY: self.raw is a non-null, valid duckdb_expression.
        unsafe { duckdb_expression_is_foldable(self.raw) }
    }

    /// Folds this (constant) expression into a single [`Value`].
    ///
    /// Only succeeds when [`is_foldable`][Expression::is_foldable] returns
    /// `true`.
    ///
    /// The returned [`Value`] always has a non-null handle, but may hold SQL
    /// `NULL` (`fold` of `NULL::INTEGER`); its typed getters return `None` for
    /// that.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if evaluation fails, and an
    /// [`DuckDbErrorType::InvalidInput`] error when the expression handle is
    /// null or the expression is not foldable (for example it references a
    /// column). `duckdb_expression_fold` reports that last case by returning
    /// *no* error and leaving the output value unset.
    ///
    /// For an evaluation failure, `DuckDB` 1.5.5 builds the error from
    /// `ex.what()` — the exception's JSON form, such as
    /// `{"exception_type":"Conversion","exception_message":"Could not convert
    /// string 'abc' to INT64",...}` — and always tags it `INVALID_INPUT`
    /// (`src/main/capi/expression-c.cpp`). This unpacks it: the returned error
    /// carries the plain `exception_message` and the
    /// [`DuckDbErrorType`] named by `exception_type`. Text that does not parse
    /// as that JSON is returned unchanged, still typed `InvalidInput`.
    pub fn fold(&self, context: &ClientContext) -> Result<Value, ErrorData> {
        let mut out_value: duckdb_value = std::ptr::null_mut();
        // SAFETY: self.raw and context.as_raw() are valid; out_value is a valid
        // out-pointer that DuckDB writes an owned duckdb_value into.
        let err_raw =
            unsafe { duckdb_expression_fold(context.as_raw(), self.raw, &raw mut out_value) };
        // SAFETY: duckdb_expression_fold returns an owned duckdb_error_data.
        let err = unsafe { ErrorData::from_raw(err_raw) };
        if err.has_error() {
            // SAFETY: out_value may have been left null/invalid; destroy any value.
            if !out_value.is_null() {
                // SAFETY: `out_value` is the handle DuckDB just wrote, and taking ownership
                // here is what frees it.
                drop(unsafe { Value::from_raw(out_value) });
            }
            return Err(unpack_exception_json(err));
        }
        if out_value.is_null() {
            // `duckdb_expression_fold` returns early, with neither an error nor
            // a value, for a null or non-foldable expression.
            return Err(ErrorData::new(
                DuckDbErrorType::InvalidInput,
                "Expression::fold: the expression is not foldable (it is not constant)",
            ));
        }
        // SAFETY: folding succeeded and out_value is non-null, so it is an
        // owned duckdb_value.
        Ok(unsafe { Value::from_raw(out_value) })
    }
}

/// Replaces an error whose message is `DuckDB`'s exception JSON with one
/// carrying the plain message and the named error type. Anything else is
/// returned unchanged.
fn unpack_exception_json(err: ErrorData) -> ErrorData {
    let Some(raw) = err.message() else {
        return err;
    };
    let Some((type_name, message)) = parse_exception_json(&raw) else {
        return err;
    };
    ErrorData::new(error_type_from_name(&type_name), &message)
}

/// Maps `Exception::ExceptionTypeToString` names (`EXCEPTION_MAP` in
/// `src/common/exception.cpp`) back to [`DuckDbErrorType`]. An unknown name
/// maps to [`DuckDbErrorType::Invalid`], as `DuckDB`'s own reverse lookup does.
fn error_type_from_name(name: &str) -> DuckDbErrorType {
    use DuckDbErrorType as T;
    match name {
        "Out of Range" => T::OutOfRange,
        "Conversion" => T::Conversion,
        "Unknown Type" => T::UnknownType,
        "Decimal" => T::Decimal,
        "Mismatch Type" => T::MismatchType,
        "Divide by Zero" => T::DivideByZero,
        "Object Size" => T::ObjectSize,
        "Invalid type" => T::InvalidType,
        "Serialization" => T::Serialization,
        "TransactionContext" => T::Transaction,
        "Not implemented" => T::NotImplemented,
        "Expression" => T::Expression,
        "Catalog" => T::Catalog,
        "Parser" => T::Parser,
        "Binder" => T::Binder,
        "Planner" => T::Planner,
        "Scheduler" => T::Scheduler,
        "Executor" => T::Executor,
        "Constraint" => T::Constraint,
        "Index" => T::Index,
        "Stat" => T::Stat,
        "Connection" => T::Connection,
        "Syntax" => T::Syntax,
        "Settings" => T::Settings,
        "Optimizer" => T::Optimizer,
        "NullPointer" => T::NullPointer,
        "IO" => T::Io,
        "INTERRUPT" => T::Interrupt,
        "FATAL" => T::Fatal,
        "INTERNAL" => T::Internal,
        "Invalid Input" => T::InvalidInput,
        "Out of Memory" => T::OutOfMemory,
        "Permission" => T::Permission,
        "Parameter Not Resolved" => T::ParameterNotResolved,
        "Parameter Not Allowed" => T::ParameterNotAllowed,
        "Dependency" => T::Dependency,
        "Missing Extension" => T::MissingExtension,
        "HTTP" => T::Http,
        "Extension Autoloading" => T::Autoload,
        "Sequence" => T::Sequence,
        "Invalid Configuration" => T::InvalidConfiguration,
        _ => T::Invalid,
    }
}

/// Parses `DuckDB`'s exception JSON — a flat object whose values are all
/// strings, written by `StringUtil::ExceptionToJSONMap` — and returns its
/// `exception_type` and `exception_message`.
///
/// Returns `None` for anything else (not an object, a non-string value, a
/// missing key, trailing text), so the caller can fall back to the raw text.
fn parse_exception_json(raw: &str) -> Option<(String, String)> {
    let mut chars = raw.trim().chars().peekable();
    let mut exception_type = None;
    let mut exception_message = None;
    let skip_ws = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        while chars.peek().is_some_and(char::is_ascii_whitespace) {
            chars.next();
        }
    };
    if chars.next()? != '{' {
        return None;
    }
    skip_ws(&mut chars);
    if chars.peek() == Some(&'}') {
        return None;
    }
    loop {
        skip_ws(&mut chars);
        let key = parse_json_string(&mut chars)?;
        skip_ws(&mut chars);
        if chars.next()? != ':' {
            return None;
        }
        skip_ws(&mut chars);
        let value = parse_json_string(&mut chars)?;
        match key.as_str() {
            "exception_type" => exception_type = Some(value),
            "exception_message" => exception_message = Some(value),
            _ => {}
        }
        skip_ws(&mut chars);
        match chars.next()? {
            ',' => {}
            '}' => break,
            _ => return None,
        }
    }
    skip_ws(&mut chars);
    if chars.next().is_some() {
        return None;
    }
    Some((exception_type?, exception_message?))
}

/// Parses one JSON string literal, including its quotes, decoding escapes
/// (with `\u` surrogate pairs).
fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let high = parse_hex4(chars)?;
                    let code = if (0xD800..0xDC00).contains(&high) {
                        if chars.next()? != '\\' || chars.next()? != 'u' {
                            return None;
                        }
                        let low = parse_hex4(chars)?;
                        if !(0xDC00..0xE000).contains(&low) {
                            return None;
                        }
                        0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                    } else {
                        high
                    };
                    out.push(char::from_u32(code)?);
                }
                _ => return None,
            },
            c => out.push(c),
        }
    }
}

/// Reads four hex digits of a `\u` escape.
fn parse_hex4(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<u32> {
    let mut code = 0;
    for _ in 0..4 {
        code = code * 16 + chars.next()?.to_digit(16)?;
    }
    Some(code)
}

impl Drop for Expression {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: self.raw is a valid duckdb_expression that we own.
            unsafe { duckdb_destroy_expression(&raw mut self.raw) };
        }
    }
}

crate::debug_repr::impl_handle_debug!(Expression.raw);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_expression_is_null() {
        let expr = unsafe { Expression::from_raw(std::ptr::null_mut()) };
        assert!(expr.is_null());
        assert!(!expr.is_foldable());
        assert!(expr.return_type().is_none());
    }

    #[test]
    fn exception_json_is_unpacked_into_type_and_message() {
        let raw = r#"{"exception_type":"Conversion","exception_message":"Could not convert string 'abc' to INT64","position":"20"}"#;
        assert_eq!(
            parse_exception_json(raw),
            Some((
                "Conversion".to_owned(),
                "Could not convert string 'abc' to INT64".to_owned()
            ))
        );
        assert_eq!(
            error_type_from_name("Out of Range"),
            DuckDbErrorType::OutOfRange
        );
        assert_eq!(
            error_type_from_name("Conversion"),
            DuckDbErrorType::Conversion
        );
        assert_eq!(
            error_type_from_name("no such type"),
            DuckDbErrorType::Invalid
        );
    }

    #[test]
    fn exception_json_escapes_are_decoded() {
        let raw = r#"{ "exception_message" : "a\"b\\c\n\u00e9\ud83e\udd86\/" , "exception_type" : "IO" }"#;
        assert_eq!(
            parse_exception_json(raw),
            Some(("IO".to_owned(), "a\"b\\c\n\u{e9}\u{1f986}/".to_owned()))
        );
    }

    #[test]
    fn text_that_is_not_exception_json_is_not_parsed() {
        for raw in [
            "plain text",
            "",
            "{}",
            r#"{"exception_type":"IO"}"#,
            r#"{"exception_type":"IO","exception_message":"m","n":1}"#,
            r#"{"exception_type":"IO","exception_message":"m"} trailing"#,
            r#"{"exception_type":"IO","exception_message":"unterminated}"#,
            r#"{"exception_type":"IO","exception_message":"\ud800"}"#,
        ] {
            assert_eq!(parse_exception_json(raw), None, "{raw}");
        }
    }

    /// Every name `Exception::ExceptionTypeToString` can produce maps to the
    /// error type with the same code. The pairs are `EXCEPTION_MAP`
    /// (`src/common/exception.cpp`) joined with the `ExceptionType` codes
    /// (`exception.hpp`) of `DuckDB` 1.5.5, extracted by script; the C API's
    /// `duckdb_error_type` uses the same codes. `NETWORK` (25) has no name.
    #[test]
    fn every_exception_type_name_maps_to_the_error_type_with_its_code() {
        for (code, name) in [
            (0, "Invalid"),
            (1, "Out of Range"),
            (2, "Conversion"),
            (3, "Unknown Type"),
            (4, "Decimal"),
            (5, "Mismatch Type"),
            (6, "Divide by Zero"),
            (7, "Object Size"),
            (8, "Invalid type"),
            (9, "Serialization"),
            (10, "TransactionContext"),
            (11, "Not implemented"),
            (12, "Expression"),
            (13, "Catalog"),
            (14, "Parser"),
            (24, "Binder"),
            (15, "Planner"),
            (16, "Scheduler"),
            (17, "Executor"),
            (18, "Constraint"),
            (19, "Index"),
            (20, "Stat"),
            (21, "Connection"),
            (22, "Syntax"),
            (23, "Settings"),
            (26, "Optimizer"),
            (27, "NullPointer"),
            (28, "IO"),
            (29, "INTERRUPT"),
            (30, "FATAL"),
            (31, "INTERNAL"),
            (32, "Invalid Input"),
            (33, "Out of Memory"),
            (34, "Permission"),
            (35, "Parameter Not Resolved"),
            (36, "Parameter Not Allowed"),
            (37, "Dependency"),
            (39, "Missing Extension"),
            (38, "HTTP"),
            (40, "Extension Autoloading"),
            (41, "Sequence"),
            (42, "Invalid Configuration"),
        ] {
            let mapped = error_type_from_name(name);
            assert_eq!(u64::from(mapped.to_raw()), code, "{name}");
            assert_eq!(mapped, DuckDbErrorType::from_raw(mapped.to_raw()), "{name}");
        }
    }

    #[test]
    fn every_single_character_json_escape_is_decoded() {
        let raw = r#"{"exception_type":"IO","exception_message":"\"\\\/\b\f\n\r\t"}"#;
        assert_eq!(
            parse_exception_json(raw),
            Some(("IO".to_owned(), "\"\\/\u{8}\u{c}\n\r\t".to_owned()))
        );
        assert_eq!(
            parse_exception_json(r#"{"exception_type":"IO","exception_message":"\q"}"#),
            None
        );
    }

    #[test]
    fn size_of_expression_is_one_pointer() {
        assert_eq!(
            std::mem::size_of::<Expression>(),
            std::mem::size_of::<usize>()
        );
    }
}
