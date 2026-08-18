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

mod blob;

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
/// - [`as_blob`][Value::as_blob] — BLOB → `Vec<u8>`
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
    /// # Embedded NUL bytes
    ///
    /// `duckdb_get_varchar` returns a NUL-terminated `char *`, so a value whose
    /// text contains an interior NUL is **truncated at the first one**. `DuckDB`
    /// itself stores the full bytes; only this read path is limited. If the text
    /// may contain NULs, keep it in a `BLOB` and use
    /// [`as_blob`][Self::as_blob].
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

    /// Returns the **SQL literal** representation of this value, as `DuckDB`
    /// would render it (`DuckDB` 1.5.0+).
    ///
    /// Note "SQL literal", not "text". A VARCHAR comes back quoted and typed
    /// values carry an explicit cast:
    ///
    /// | Value | `display_string()` |
    /// |-------|--------------------|
    /// | `Value::varchar("hello")` | `'hello'` |
    /// | `Value::bigint(-42)` | `-42` |
    /// | `Value::date(0)` | `'1970-01-01'::DATE` |
    /// | `Value::timestamp(0)` | `'1970-01-01 00:00:00'::TIMESTAMP` |
    ///
    /// Use [`as_str`][Self::as_str] for a VARCHAR's contents. This is for
    /// diagnostics and error messages, where it works for any value type.
    ///
    /// Returns `None` if the handle is null or the rendered text is not valid
    /// UTF-8.
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

    // ── Temporal, DECIMAL and UUID extraction ────────────────────────────
    //
    // A table function declared with `.named_param("since", TypeId::Timestamp)`
    // hands the bind callback a `duckdb_value`, and until now the only way to
    // read it was `as_str()` plus reparsing DuckDB's rendering. These are the
    // `duckdb_get_*` counterparts, all in the stable prefix of the C API.

    /// Extracts a `DATE` as days since 1970-01-01.
    ///
    /// Returns 0 if the value is not a `DATE`. Decode it with
    /// [`datetime::date_from_days`][crate::datetime::date_from_days].
    #[inline]
    #[must_use]
    pub fn as_date(&self) -> i32 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_date(self.raw) }.days
    }

    /// Extracts a `TIME` as microseconds since midnight.
    ///
    /// Returns 0 if the value is not a `TIME`.
    #[inline]
    #[must_use]
    pub fn as_time(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_time(self.raw) }.micros
    }

    /// Extracts a `TIMETZ` as `DuckDB`'s packed 64-bit representation.
    ///
    /// Decode it with
    /// [`datetime::time_tz_from_bits`][crate::datetime::time_tz_from_bits].
    #[inline]
    #[must_use]
    pub fn as_time_tz(&self) -> u64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_time_tz(self.raw) }.bits
    }

    /// Extracts a `TIMESTAMP` as microseconds since the epoch.
    ///
    /// Returns 0 if the value is not a `TIMESTAMP`.
    #[inline]
    #[must_use]
    pub fn as_timestamp(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_timestamp(self.raw) }.micros
    }

    /// Extracts a `TIMESTAMPTZ` as microseconds since the epoch, in UTC.
    #[inline]
    #[must_use]
    pub fn as_timestamp_tz(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_timestamp_tz(self.raw) }.micros
    }

    /// Extracts a `TIMESTAMP_S` as seconds since the epoch.
    #[inline]
    #[must_use]
    pub fn as_timestamp_s(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_timestamp_s(self.raw) }.seconds
    }

    /// Extracts a `TIMESTAMP_MS` as milliseconds since the epoch.
    #[inline]
    #[must_use]
    pub fn as_timestamp_ms(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_timestamp_ms(self.raw) }.millis
    }

    /// Extracts a `TIMESTAMP_NS` as nanoseconds since the epoch.
    #[inline]
    #[must_use]
    pub fn as_timestamp_ns(&self) -> i64 {
        // SAFETY: self.raw is valid per constructor contract.
        unsafe { libduckdb_sys::duckdb_get_timestamp_ns(self.raw) }.nanos
    }

    /// Extracts an `INTERVAL`.
    #[inline]
    #[must_use]
    pub fn as_interval(&self) -> crate::interval::DuckInterval {
        // SAFETY: self.raw is valid per constructor contract.
        let raw = unsafe { libduckdb_sys::duckdb_get_interval(self.raw) };
        crate::interval::DuckInterval {
            months: raw.months,
            days: raw.days,
            micros: raw.micros,
        }
    }

    /// Extracts a `UUID` as its **textual** 128 bits, matching
    /// [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid]
    /// and [`uuid`][Self::uuid].
    ///
    /// `DuckDB` undoes its internal top-bit flip itself here, so this is the
    /// value the UUID renders as — not the raw `HUGEINT` a `UUID` vector holds.
    #[inline]
    #[must_use]
    pub fn as_uuid(&self) -> u128 {
        // SAFETY: self.raw is valid per constructor contract.
        let raw = unsafe { libduckdb_sys::duckdb_get_uuid(self.raw) };
        (u128::from(raw.upper) << 64) | u128::from(raw.lower)
    }

    /// Extracts a `DECIMAL` as its width, scale and unscaled value.
    ///
    /// The represented number is `value / 10^scale`.
    #[inline]
    #[must_use]
    pub fn as_decimal(&self) -> crate::datetime::Decimal {
        // SAFETY: self.raw is valid per constructor contract.
        let raw = unsafe { libduckdb_sys::duckdb_get_decimal(self.raw) };
        crate::datetime::Decimal {
            width: raw.width,
            scale: raw.scale,
            value: (i128::from(raw.value.upper) << 64) | i128::from(raw.value.lower),
        }
    }

    /// Extracts a `UHUGEINT` as a `u128`.
    #[inline]
    #[must_use]
    pub fn as_u128(&self) -> u128 {
        // SAFETY: self.raw is valid per constructor contract.
        let raw = unsafe { libduckdb_sys::duckdb_get_uhugeint(self.raw) };
        (u128::from(raw.upper) << 64) | u128::from(raw.lower)
    }

    // ── LIST / STRUCT / MAP extraction ───────────────────────────────────

    /// Number of elements in a `LIST` value.
    ///
    /// Returns 0 for non-`LIST` values.
    #[inline]
    #[must_use]
    pub fn list_len(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { libduckdb_sys::duckdb_get_list_size(self.raw) }).unwrap_or(0)
    }

    /// Element `index` of a `LIST` value, or `None` if out of range.
    ///
    /// The returned [`Value`] owns its handle.
    #[must_use]
    pub fn list_child(&self, index: usize) -> Option<Self> {
        if index >= self.list_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `list_len`.
        let raw = unsafe {
            libduckdb_sys::duckdb_get_list_child(self.raw, index as libduckdb_sys::idx_t)
        };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Collects a `LIST` value into a `Vec` of owned [`Value`]s.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use quack_rs::value::Value;
    /// # fn demo(paths: &Value) {
    /// let files: Vec<String> = paths
    ///     .list_items()
    ///     .iter()
    ///     .filter_map(|v| v.as_str().ok())
    ///     .collect();
    /// # }
    /// ```
    #[must_use]
    pub fn list_items(&self) -> Vec<Self> {
        (0..self.list_len())
            .filter_map(|i| self.list_child(i))
            .collect()
    }

    /// Field `index` of a `STRUCT` value, or `None` if the handle is null or the
    /// index is out of range.
    ///
    /// Field names come from the value's `LogicalType`, not from the value
    /// itself; `DuckDB`'s C API exposes children by position.
    #[must_use]
    pub fn struct_child(&self, index: usize) -> Option<Self> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: self.raw is valid; DuckDB returns null for an out-of-range index.
        let raw = unsafe {
            libduckdb_sys::duckdb_get_struct_child(self.raw, index as libduckdb_sys::idx_t)
        };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Number of key/value pairs in a `MAP` value.
    #[inline]
    #[must_use]
    pub fn map_len(&self) -> usize {
        // SAFETY: self.raw is valid per constructor contract.
        usize::try_from(unsafe { libduckdb_sys::duckdb_get_map_size(self.raw) }).unwrap_or(0)
    }

    /// Key at `index` of a `MAP` value, or `None` if out of range.
    #[must_use]
    pub fn map_key(&self, index: usize) -> Option<Self> {
        if index >= self.map_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `map_len`.
        let raw =
            unsafe { libduckdb_sys::duckdb_get_map_key(self.raw, index as libduckdb_sys::idx_t) };
        (!raw.is_null()).then(|| Self { raw })
    }

    /// Value at `index` of a `MAP` value, or `None` if out of range.
    #[must_use]
    pub fn map_value(&self, index: usize) -> Option<Self> {
        if index >= self.map_len() {
            return None;
        }
        // SAFETY: `index` was bounds-checked against `map_len`.
        let raw =
            unsafe { libduckdb_sys::duckdb_get_map_value(self.raw, index as libduckdb_sys::idx_t) };
        (!raw.is_null()).then(|| Self { raw })
    }

    // ── Construction ─────────────────────────────────────────────────────
    //
    // Needed for `ConfigOptionBuilder::default_value` and anywhere else DuckDB
    // wants a `duckdb_value` rather than a Rust scalar.

    /// Creates a `BOOLEAN` value.
    #[inline]
    #[must_use]
    pub fn boolean(value: bool) -> Self {
        // SAFETY: duckdb_create_bool accepts any bool and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_bool(value) },
        }
    }

    /// Creates a `BIGINT` value.
    #[inline]
    #[must_use]
    pub fn bigint(value: i64) -> Self {
        // SAFETY: duckdb_create_int64 accepts any i64 and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_int64(value) },
        }
    }

    /// Creates a `DOUBLE` value.
    #[inline]
    #[must_use]
    pub fn double(value: f64) -> Self {
        // SAFETY: duckdb_create_double accepts any f64 and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_double(value) },
        }
    }

    /// Creates a `DATE` value from days since 1970-01-01.
    #[inline]
    #[must_use]
    pub fn date(days: i32) -> Self {
        // SAFETY: duckdb_create_date accepts any day count.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_date(libduckdb_sys::duckdb_date { days }) },
        }
    }

    /// Creates a `TIMESTAMP` value from microseconds since the epoch.
    #[inline]
    #[must_use]
    pub fn timestamp(micros: i64) -> Self {
        // SAFETY: duckdb_create_timestamp accepts any microsecond count.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe {
                libduckdb_sys::duckdb_create_timestamp(libduckdb_sys::duckdb_timestamp { micros })
            },
        }
    }

    /// Creates a `VARCHAR` value.
    ///
    /// The length is passed explicitly, so no `CString` conversion can fail and
    /// `DuckDB` stores every byte — but note that [`as_str`][Self::as_str] reads
    /// back through a NUL-terminated C string and will truncate at an interior
    /// NUL.
    #[must_use]
    pub fn varchar(value: &str) -> Self {
        // SAFETY: `value` is valid for the duration of the call; DuckDB copies it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_varchar_length(
                value.as_ptr().cast::<c_char>(),
                libduckdb_sys::idx_t::try_from(value.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self { raw }
    }

    /// Creates a `UUID` value from its **textual** 128 bits, matching
    /// [`VectorWriter::write_uuid`][crate::vector::VectorWriter::write_uuid]
    /// and [`as_uuid`][Self::as_uuid].
    ///
    /// `DuckDB` applies its internal top-bit flip itself here, so these are the
    /// bits the value renders as — not the raw `HUGEINT` a `UUID` vector holds.
    #[inline]
    #[must_use]
    pub fn uuid(bits: u128) -> Self {
        let raw = libduckdb_sys::duckdb_uhugeint {
            #[allow(clippy::cast_possible_truncation)]
            lower: bits as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (bits >> 64) as u64,
        };
        // SAFETY: duckdb_create_uuid accepts any 128-bit pattern.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_uuid(raw) },
        }
    }

    /// Creates a SQL `NULL` value.
    #[inline]
    #[must_use]
    pub fn null_value() -> Self {
        // SAFETY: duckdb_create_null_value takes no arguments and returns an
        // owned SQLNULL value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_null_value() },
        }
    }

    /// Returns the [`TypeId`][crate::types::TypeId] this value actually holds.
    ///
    /// Every `as_*` accessor *reinterprets* the value as a chosen physical
    /// type without checking: reading a `VARCHAR` with
    /// [`as_i64`][Self::as_i64] returns garbage rather than an error. This is
    /// the check that makes those accessors safe to use on a value whose type
    /// you did not choose — a named parameter, a bound constant, a config
    /// option.
    ///
    /// Returns `None` for a null handle, and for a type id introduced by a
    /// newer `DuckDB` than this build of quack-rs knows.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::TypeId;
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(value: &Value) -> Option<i64> {
    /// match value.type_id()? {
    ///     TypeId::BigInt => Some(value.as_i64()),
    ///     TypeId::Integer => Some(i64::from(value.as_i32())),
    ///     _ => None,
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn type_id(&self) -> Option<crate::types::TypeId> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: `self.raw` is a valid duckdb_value per the constructor
        // contract. The returned logical type is owned by the value — duckdb.h
        // states "The type itself must not be destroyed" — so it is read
        // directly rather than wrapped in `LogicalType`, which frees on drop.
        let logical = unsafe { libduckdb_sys::duckdb_get_value_type(self.raw) };
        if logical.is_null() {
            return None;
        }
        // SAFETY: `logical` is non-null and valid for as long as `self` is.
        let raw_id = unsafe { libduckdb_sys::duckdb_get_type_id(logical) };
        crate::types::TypeId::try_from_duckdb_type(raw_id)
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

impl core::fmt::Debug for Value {
    /// Prints the value's type and, where `DuckDB` can render it, its contents.
    ///
    /// Like [`LogicalType`][crate::types::LogicalType]'s impl this calls into
    /// `DuckDB`, so it avoids every path that could panic while formatting.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.raw.is_null() {
            return f.write_str("Value(<null handle>)");
        }
        let mut out = f.debug_struct("Value");
        match self.type_id() {
            Some(type_id) => out.field("type", &type_id),
            None => out.field("type", &"<unknown>"),
        };
        #[cfg(feature = "duckdb-1-5")]
        if let Some(rendered) = self.display_string() {
            out.field("value", &rendered);
        }
        out.finish()
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

/// `Value` accessors exercised against a live `DuckDB`.
///
/// These go through real `duckdb_value` handles produced by SQL, which is the
/// only way to be sure each `duckdb_get_*` is paired with the right SQL type.
#[cfg(all(test, feature = "_duckdb-testing"))]
mod live_tests {
    use super::Value;
    use crate::datetime;
    use crate::testing::InMemoryDb;

    /// Evaluates `expr` and returns the result as an owned `Value`.
    ///
    /// Uses `duckdb_create_*` round-tripping through SQL is not possible from
    /// the C API, so this builds the value with the constructors under test and
    /// checks it against `DuckDB`'s own rendering.
    #[cfg(feature = "duckdb-1-5")]
    fn rendered(value: &Value) -> String {
        value
            .display_string()
            .expect("DuckDB should render every value")
    }

    /// `display_string` wraps `duckdb_value_to_string`, which lives past the
    /// stable prefix, so this assertion set needs `duckdb-1-5`.
    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn scalar_constructors_render_as_duckdb_would() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        assert_eq!(rendered(&Value::boolean(true)), "true");
        assert_eq!(rendered(&Value::bigint(-42)), "-42");
        // `duckdb_value_to_string` renders a SQL *literal*, so a VARCHAR is
        // quoted. This is easy to trip over when using it for diagnostics.
        assert_eq!(rendered(&Value::varchar("hello")), "'hello'");
        // Typed values carry an explicit cast, which is what makes this a SQL
        // *literal* rather than a display string.
        assert_eq!(rendered(&Value::date(0)), "'1970-01-01'::DATE");
        assert_eq!(
            rendered(&Value::timestamp(0)),
            "'1970-01-01 00:00:00'::TIMESTAMP"
        );
        assert!(Value::null_value().display_string().is_some());
    }

    #[test]
    fn temporal_constructors_round_trip_through_accessors() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");

        // 2026-08-18 is 20685 days after the epoch; check against the calendar
        // conversion rather than restating the number.
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let days = unsafe {
            datetime::date_to_days(datetime::Date {
                year: 2026,
                month: 8,
                day: 18,
            })
        };
        assert_eq!(Value::date(days).as_date(), days);

        let micros = 1_700_000_000_000_000_i64;
        assert_eq!(Value::timestamp(micros).as_timestamp(), micros);
        assert_eq!(Value::bigint(i64::MIN).as_i64(), i64::MIN);
        assert_eq!(Value::bigint(i64::MAX).as_i64(), i64::MAX);
    }

    #[test]
    fn as_str_truncates_at_an_embedded_nul() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // `duckdb_create_varchar_length` stores all three bytes, but
        // `duckdb_get_varchar` hands back a NUL-terminated C string, so the read
        // path truncates. Pinned so the documented caveat stays accurate.
        let value = Value::varchar("a\0b");
        assert_eq!(value.as_str().expect("utf8"), "a");
    }

    #[test]
    fn uuid_round_trips_including_the_high_bit() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // `duckdb_get_uuid` returns an *unsigned* hugeint. Assembling its halves
        // directly as i128 overflows once the upper half's high bit is set —
        // which is true for half of all UUIDs, and panics in a debug build.
        for bits in [0_u128, 1, u128::MAX, 1 << 127, (1 << 127) - 1] {
            assert_eq!(
                Value::uuid(bits).as_uuid(),
                bits,
                "round trip for {bits:#034x}"
            );
        }
    }

    #[test]
    fn list_and_map_accessors_bounds_check() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // A scalar is not a list; the accessors must report that rather than
        // reading out of range.
        let scalar = Value::bigint(1);
        assert_eq!(scalar.list_len(), 0);
        assert!(scalar.list_child(0).is_none());
        assert_eq!(scalar.map_len(), 0);
        assert!(scalar.map_key(0).is_none());
        assert!(scalar.map_value(0).is_none());
    }

    #[test]
    fn accessors_on_a_null_handle_do_not_crash() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: a null handle is explicitly part of `Value`'s contract — it is
        // what `duckdb_bind_get_named_parameter` returns for an absent parameter.
        let value = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(value.is_null());
        assert!(value.as_str().is_err());
        #[cfg(feature = "duckdb-1-5")]
        assert!(value.display_string().is_none());
        assert!(value.struct_child(0).is_none());
    }
}
