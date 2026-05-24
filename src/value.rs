// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! RAII wrapper around `DuckDB` values (`duckdb_value`).
//!
//! [`Value`] provides safe, typed access to `DuckDB` values returned from bind
//! parameter extraction, configuration options, and other APIs. It automatically
//! calls [`duckdb_destroy_value`] on drop, eliminating the manual cleanup that
//! every extension author currently has to remember.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::value::Value;
//! use quack_rs::table::BindInfo;
//! use libduckdb_sys::duckdb_bind_info;
//!
//! unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
//!     let bind = unsafe { BindInfo::new(info) };
//!     // RAII: Value is destroyed automatically when it goes out of scope.
//!     let val = unsafe { Value::from_raw(bind.get_parameter(0)) };
//!     if let Ok(s) = val.as_str() {
//!         // use s...
//!     }
//! }
//! ```

use std::ffi::CStr;
use std::os::raw::c_char;

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    duckdb_create_time_ns, duckdb_get_time_ns, duckdb_time_ns, duckdb_value_to_string,
};
use libduckdb_sys::{
    duckdb_destroy_value, duckdb_free, duckdb_get_bool, duckdb_get_double, duckdb_get_float,
    duckdb_get_hugeint, duckdb_get_int16, duckdb_get_int32, duckdb_get_int64, duckdb_get_int8,
    duckdb_get_uint16, duckdb_get_uint32, duckdb_get_uint64, duckdb_get_uint8, duckdb_get_varchar,
    duckdb_value,
};

use crate::error::ExtensionError;

/// An owned, RAII-managed `DuckDB` value.
///
/// When dropped, the underlying `duckdb_value` handle is destroyed via
/// [`duckdb_destroy_value`]. This eliminates the manual `duckdb_destroy_value`
/// calls that are easy to forget and lead to memory leaks.
///
/// # Creation
///
/// Obtain a `Value` from:
/// - [`BindInfo::get_parameter_value`][crate::table::BindInfo::get_parameter_value]
/// - [`BindInfo::get_named_parameter_value`][crate::table::BindInfo::get_named_parameter_value]
/// - [`Value::from_raw`] (escape hatch for raw `duckdb_value` handles)
///
/// # Extraction
///
/// Use typed accessors to extract the underlying data:
/// - [`as_str`][Value::as_str] — VARCHAR → `String`
/// - [`as_i32`][Value::as_i32] — INTEGER → `i32`
/// - [`as_i64`][Value::as_i64] — BIGINT → `i64`
/// - [`as_f32`][Value::as_f32] — FLOAT → `f32`
/// - [`as_f64`][Value::as_f64] — DOUBLE → `f64`
/// - [`as_bool`][Value::as_bool] — BOOLEAN → `bool`
pub struct Value {
    raw: duckdb_value,
}

impl Value {
    /// Wraps a raw `duckdb_value` handle.
    ///
    /// The returned `Value` takes ownership and will call `duckdb_destroy_value`
    /// on drop.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid `duckdb_value` obtained from a `DuckDB` API call
    /// (e.g., `duckdb_bind_get_parameter`). The caller must not destroy the
    /// value after passing it to this function.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_value) -> Self {
        Self { raw }
    }

    /// Extracts the value as a `String` (VARCHAR).
    ///
    /// Internally calls `duckdb_get_varchar` and frees the returned C string
    /// with `duckdb_free`. Returns an error if the string is not valid UTF-8
    /// or if the value handle is null.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the value is null or contains invalid UTF-8.
    pub fn as_str(&self) -> Result<String, ExtensionError> {
        if self.raw.is_null() {
            return Err(ExtensionError::new("Value is null"));
        }
        // SAFETY: self.raw is a valid duckdb_value per constructor contract.
        let c_str: *mut c_char = unsafe { duckdb_get_varchar(self.raw) };
        if c_str.is_null() {
            return Err(ExtensionError::new("duckdb_get_varchar returned null"));
        }
        // SAFETY: c_str is a valid null-terminated C string allocated by DuckDB.
        let result = unsafe { CStr::from_ptr(c_str) }
            .to_str()
            .map(str::to_owned)
            .map_err(|_| ExtensionError::new("Value contains invalid UTF-8"));
        // SAFETY: c_str was allocated by DuckDB and must be freed with duckdb_free.
        unsafe { duckdb_free(c_str.cast()) };
        result
    }

    /// Extracts the value as an `i32` (INTEGER).
    ///
    /// `DuckDB` will attempt to cast the value to INTEGER. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_i32(&self) -> i32 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_int32(self.raw) }
    }

    /// Extracts the value as an `i64` (BIGINT).
    ///
    /// `DuckDB` will attempt to cast the value to BIGINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_i64(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_int64(self.raw) }
    }

    /// Extracts the value as an `f32` (FLOAT).
    ///
    /// `DuckDB` will attempt to cast the value to FLOAT. If the value is not
    /// numeric, this returns 0.0.
    #[inline]
    #[must_use]
    pub fn as_f32(&self) -> f32 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_float(self.raw) }
    }

    /// Extracts the value as an `f64` (DOUBLE).
    ///
    /// `DuckDB` will attempt to cast the value to DOUBLE. If the value is not
    /// numeric, this returns 0.0.
    #[inline]
    #[must_use]
    pub fn as_f64(&self) -> f64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_double(self.raw) }
    }

    /// Extracts the value as a `bool` (BOOLEAN).
    ///
    /// `DuckDB` will attempt to cast the value to BOOLEAN. If the value is not
    /// convertible, this returns `false`.
    #[inline]
    #[must_use]
    pub fn as_bool(&self) -> bool {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_bool(self.raw) }
    }

    /// Extracts the value as an `i8` (TINYINT).
    ///
    /// `DuckDB` will attempt to cast the value to TINYINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_i8(&self) -> i8 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_int8(self.raw) }
    }

    /// Extracts the value as an `i16` (SMALLINT).
    ///
    /// `DuckDB` will attempt to cast the value to SMALLINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_i16(&self) -> i16 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_int16(self.raw) }
    }

    /// Extracts the value as a `u8` (UTINYINT).
    ///
    /// `DuckDB` will attempt to cast the value to UTINYINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_u8(&self) -> u8 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_uint8(self.raw) }
    }

    /// Extracts the value as a `u16` (USMALLINT).
    ///
    /// `DuckDB` will attempt to cast the value to USMALLINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_u16(&self) -> u16 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_uint16(self.raw) }
    }

    /// Extracts the value as a `u32` (UINTEGER).
    ///
    /// `DuckDB` will attempt to cast the value to UINTEGER. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_u32(&self) -> u32 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_uint32(self.raw) }
    }

    /// Extracts the value as a `u64` (UBIGINT).
    ///
    /// `DuckDB` will attempt to cast the value to UBIGINT. If the value is not
    /// numeric, this returns 0.
    #[inline]
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_uint64(self.raw) }
    }

    /// Extracts the value as an `i128` (HUGEINT).
    ///
    /// `DuckDB` returns HUGEINT as `{ lower: u64, upper: i64 }`. This method
    /// reconstructs the full `i128` value.
    #[inline]
    #[must_use]
    pub fn as_i128(&self) -> i128 {
        // SAFETY: self.raw is valid per constructor contract.
        let h = unsafe { duckdb_get_hugeint(self.raw) };
        #[allow(clippy::cast_lossless)]
        let result = (h.upper as i128) << 64 | (h.lower as i128);
        result
    }

    /// Extracts the value as a `String`, returning `default` on failure.
    ///
    /// Convenience for `val.as_str().unwrap_or_else(|_| default.to_owned())`.
    #[inline]
    #[must_use]
    pub fn as_str_or(&self, default: &str) -> String {
        self.as_str().unwrap_or_else(|_| default.to_owned())
    }

    /// Extracts the value as a `String`, returning an empty string on failure.
    ///
    /// Convenience for `val.as_str().unwrap_or_default()`.
    #[inline]
    #[must_use]
    pub fn as_str_or_default(&self) -> String {
        self.as_str().unwrap_or_default()
    }

    /// Extracts the value as an `i32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i32_or(&self, default: i32) -> i32 {
        if self.is_null() {
            default
        } else {
            self.as_i32()
        }
    }

    /// Extracts the value as an `i64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i64_or(&self, default: i64) -> i64 {
        if self.is_null() {
            default
        } else {
            self.as_i64()
        }
    }

    /// Extracts the value as an `f32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_f32_or(&self, default: f32) -> f32 {
        if self.is_null() {
            default
        } else {
            self.as_f32()
        }
    }

    /// Extracts the value as an `f64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_f64_or(&self, default: f64) -> f64 {
        if self.is_null() {
            default
        } else {
            self.as_f64()
        }
    }

    /// Extracts the value as a `bool`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_bool_or(&self, default: bool) -> bool {
        if self.is_null() {
            default
        } else {
            self.as_bool()
        }
    }

    /// Extracts the value as an `i8`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i8_or(&self, default: i8) -> i8 {
        if self.is_null() {
            default
        } else {
            self.as_i8()
        }
    }

    /// Extracts the value as an `i16`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i16_or(&self, default: i16) -> i16 {
        if self.is_null() {
            default
        } else {
            self.as_i16()
        }
    }

    /// Extracts the value as a `u8`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u8_or(&self, default: u8) -> u8 {
        if self.is_null() {
            default
        } else {
            self.as_u8()
        }
    }

    /// Extracts the value as a `u16`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u16_or(&self, default: u16) -> u16 {
        if self.is_null() {
            default
        } else {
            self.as_u16()
        }
    }

    /// Extracts the value as a `u32`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u32_or(&self, default: u32) -> u32 {
        if self.is_null() {
            default
        } else {
            self.as_u32()
        }
    }

    /// Extracts the value as a `u64`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_u64_or(&self, default: u64) -> u64 {
        if self.is_null() {
            default
        } else {
            self.as_u64()
        }
    }

    /// Extracts the value as an `i128`, returning `default` if the handle is null.
    #[inline]
    #[must_use]
    pub fn as_i128_or(&self, default: i128) -> i128 {
        if self.is_null() {
            default
        } else {
            self.as_i128()
        }
    }

    /// Creates a `TIME_NS` value (time of day with nanosecond precision) from a
    /// raw nanosecond count (`DuckDB` 1.5.0+).
    ///
    /// Pairs with [`as_time_ns`][Value::as_time_ns] and the
    /// [`TypeId::TimeNs`][crate::types::TypeId::TimeNs] column type.
    #[cfg(feature = "duckdb-1-5")]
    #[inline]
    #[must_use]
    pub fn time_ns(nanos: i64) -> Self {
        // SAFETY: duckdb_create_time_ns accepts any nanosecond count and returns
        // an owned duckdb_value.
        let raw = unsafe { duckdb_create_time_ns(duckdb_time_ns { nanos }) };
        Self { raw }
    }

    /// Extracts the value as a `TIME_NS` nanosecond count (`DuckDB` 1.5.0+).
    ///
    /// Returns 0 if the value is not a `TIME_NS`.
    #[cfg(feature = "duckdb-1-5")]
    #[inline]
    #[must_use]
    pub fn as_time_ns(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { duckdb_get_time_ns(self.raw) }.nanos
    }

    /// Returns the canonical string representation of this value, as `DuckDB`
    /// would render it (`DuckDB` 1.5.0+).
    ///
    /// Returns `None` if the handle is null or the rendered text is not valid
    /// UTF-8. This is primarily useful for diagnostics and error messages, where
    /// it works for any value type (not just VARCHAR).
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn display_string(&self) -> Option<String> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: self.raw is a valid duckdb_value per constructor contract.
        let c_str: *mut c_char = unsafe { duckdb_value_to_string(self.raw) };
        if c_str.is_null() {
            return None;
        }
        // SAFETY: c_str is a valid null-terminated string allocated by DuckDB.
        let result = unsafe { CStr::from_ptr(c_str) }
            .to_str()
            .ok()
            .map(str::to_owned);
        // SAFETY: c_str was allocated by DuckDB and must be freed with duckdb_free.
        unsafe { duckdb_free(c_str.cast()) };
        result
    }

    /// Returns `true` if the underlying handle is null.
    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.raw.is_null()
    }

    /// Returns the raw `duckdb_value` handle without consuming the `Value`.
    ///
    /// The `Value` still owns the handle and will destroy it on drop.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_value {
        self.raw
    }

    /// Consumes the `Value` and returns the raw `duckdb_value` handle.
    ///
    /// The caller takes ownership and is responsible for calling
    /// `duckdb_destroy_value` when done.
    #[inline]
    #[must_use]
    pub const fn into_raw(self) -> duckdb_value {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: self.raw is a valid duckdb_value that we own.
            unsafe { duckdb_destroy_value(&raw mut self.raw) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_value_is_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(val.is_null());
    }

    #[test]
    fn null_value_as_str_returns_error() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(val.as_str().is_err());
    }

    #[test]
    fn into_raw_prevents_double_free() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        let raw = val.into_raw();
        assert!(raw.is_null());
        // No double-free: Value was forgotten via into_raw.
    }

    #[test]
    fn size_of_value() {
        assert_eq!(std::mem::size_of::<Value>(), std::mem::size_of::<usize>());
    }

    #[test]
    fn as_str_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_str_or("fallback"), "fallback");
    }

    #[test]
    fn as_str_or_default_returns_empty_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_str_or_default(), "");
    }

    #[test]
    fn as_i64_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i64_or(99), 99);
    }

    #[test]
    fn as_i32_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(val.as_i32_or(42), 42);
    }

    #[test]
    fn as_bool_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(val.as_bool_or(true));
        assert!(!val.as_bool_or(false));
    }

    #[test]
    fn as_f64_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!((val.as_f64_or(2.72) - 2.72).abs() < f64::EPSILON);
    }

    #[test]
    fn as_f32_or_returns_default_for_null() {
        let val = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!((val.as_f32_or(2.5) - 2.5).abs() < f32::EPSILON);
    }
}
