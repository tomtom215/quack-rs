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
//! use quack_rs::scalar::ScalarBindInfo;
//! use libduckdb_sys::duckdb_bind_info;
//!
//! unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
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
use crate::error_data::ErrorData;
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
    /// Only valid when [`is_foldable`][Expression::is_foldable] returns `true`.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if folding fails (for example because
    /// the expression is not constant).
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
                drop(unsafe { Value::from_raw(out_value) });
            }
            return Err(err);
        }
        // SAFETY: folding succeeded, so out_value is an owned duckdb_value.
        Ok(unsafe { Value::from_raw(out_value) })
    }
}

impl Drop for Expression {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: self.raw is a valid duckdb_expression that we own.
            unsafe { duckdb_destroy_expression(&raw mut self.raw) };
        }
    }
}

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
    fn size_of_expression_is_one_pointer() {
        assert_eq!(
            std::mem::size_of::<Expression>(),
            std::mem::size_of::<usize>()
        );
    }
}
