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
mod defaults;
mod hugeint;

// Re-exported unqualified so the eight call sites across this module and
// `query` keep their existing `crate::value::hugeint_from_i128` paths.
pub(crate) use hugeint::{
    hugeint_from_i128, hugeint_to_i128, uhugeint_from_u128, uhugeint_to_u128,
};

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
        hugeint_to_i128(unsafe { duckdb_get_hugeint(self.raw) })
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
        uhugeint_to_u128(unsafe { libduckdb_sys::duckdb_get_uuid(self.raw) })
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
            value: hugeint_to_i128(raw.value),
        }
    }

    /// Extracts a `UHUGEINT` as a `u128`.
    #[inline]
    #[must_use]
    pub fn as_u128(&self) -> u128 {
        // SAFETY: self.raw is valid per constructor contract.
        uhugeint_to_u128(unsafe { libduckdb_sys::duckdb_get_uhugeint(self.raw) })
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

    /// Field names of a `STRUCT` value, in order.
    ///
    /// `DuckDB`'s C API exposes a struct value's children by position only —
    /// the names live on the value's `LogicalType`. Reading them by hand means
    /// calling `duckdb_get_value_type`, whose result `duckdb.h` says "must not
    /// be destroyed" because the *value* owns it, which is exactly the kind of
    /// borrowed handle it is easy to hand to
    /// [`LogicalType::from_raw`][crate::types::LogicalType::from_raw] and then
    /// double-free (pitfall P11). This does the walk without exposing it.
    ///
    /// Returns an empty vector for a null handle or a non-`STRUCT` value.
    ///
    /// Pair it with [`struct_child`][Self::struct_child], which is positional:
    ///
    /// ```rust,no_run
    /// # use quack_rs::value::Value;
    /// # fn demo(options: &Value) -> Option<String> {
    /// let names = options.struct_field_names();
    /// let idx = names.iter().position(|n| n == "compression")?;
    /// options.struct_child(idx)?.as_str().ok()
    /// # }
    /// ```
    #[must_use]
    pub fn struct_field_names(&self) -> Vec<String> {
        if self.raw.is_null() {
            return Vec::new();
        }
        // SAFETY: `self.raw` is a valid duckdb_value per the constructor
        // contract. The returned type is owned by the value — duckdb.h: "The
        // type itself must not be destroyed" — so it is only read through, never
        // wrapped in `LogicalType` and never destroyed here.
        let logical = unsafe { libduckdb_sys::duckdb_get_value_type(self.raw) };
        if logical.is_null() {
            return Vec::new();
        }
        // SAFETY: `logical` is non-null and lives as long as `self`.
        let count = unsafe { libduckdb_sys::duckdb_struct_type_child_count(logical) };
        (0..count)
            .map(|i| {
                // SAFETY: `i < count`. DuckDB allocates the name with
                // `duckdb_malloc`, so it is freed with `duckdb_free`.
                let ptr = unsafe { libduckdb_sys::duckdb_struct_type_child_name(logical, i) };
                if ptr.is_null() {
                    return String::new();
                }
                // SAFETY: `ptr` is a NUL-terminated string owned by us.
                let owned = unsafe { std::ffi::CStr::from_ptr(ptr) }
                    .to_str()
                    .unwrap_or_default()
                    .to_owned();
                // SAFETY: allocated by DuckDB, freed exactly once here.
                unsafe { duckdb_free(ptr.cast::<std::os::raw::c_void>()) };
                owned
            })
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
        let raw = uhugeint_from_u128(bits);
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

    // ── The rest of DuckDB's scalar constructors ─────────────────────────
    //
    // Every one of these lives in the *stable* prefix of `duckdb_ext_api_v1`,
    // so they need no feature gate and work against every DuckDB from v1.2.0.

    /// Creates a `TINYINT` value.
    #[inline]
    #[must_use]
    pub fn tinyint(value: i8) -> Self {
        Self {
            // SAFETY: a plain scalar DuckDB accepts unconditionally; the
            // returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_int8(value) },
        }
    }

    /// Creates a `SMALLINT` value.
    #[inline]
    #[must_use]
    pub fn smallint(value: i16) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_int16(value) },
        }
    }

    /// Creates an `INTEGER` value.
    #[inline]
    #[must_use]
    pub fn integer(value: i32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_int32(value) },
        }
    }

    /// Creates a `UTINYINT` value.
    #[inline]
    #[must_use]
    pub fn utinyint(value: u8) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint8(value) },
        }
    }

    /// Creates a `USMALLINT` value.
    #[inline]
    #[must_use]
    pub fn usmallint(value: u16) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint16(value) },
        }
    }

    /// Creates a `UINTEGER` value.
    #[inline]
    #[must_use]
    pub fn uinteger(value: u32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint32(value) },
        }
    }

    /// Creates a `UBIGINT` value.
    #[inline]
    #[must_use]
    pub fn ubigint(value: u64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint64(value) },
        }
    }

    /// Creates a `HUGEINT` value.
    #[inline]
    #[must_use]
    pub fn hugeint(value: i128) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_hugeint(hugeint_from_i128(value)) },
        }
    }

    /// Creates a `UHUGEINT` value.
    #[inline]
    #[must_use]
    pub fn uhugeint(value: u128) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uhugeint(uhugeint_from_u128(value)) },
        }
    }

    /// Creates a `FLOAT` value.
    #[inline]
    #[must_use]
    pub fn float(value: f32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_float(value) },
        }
    }

    /// Creates a `TIME` value from microseconds since midnight.
    #[inline]
    #[must_use]
    pub fn time(micros_since_midnight: i64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_time(libduckdb_sys::duckdb_time {
                    micros: micros_since_midnight,
                })
            },
        }
    }

    /// Creates a `TIME WITH TIME ZONE` value from its packed 64-bit encoding.
    ///
    /// `DuckDB` packs `TIME_TZ` as 40 bits of microseconds and 24 bits of UTC
    /// offset. Build the encoding with
    /// [`time_tz_bits`][crate::datetime::time_tz_bits] rather than assembling it
    /// by hand.
    #[inline]
    #[must_use]
    pub fn time_tz(bits: u64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_time_tz_value(libduckdb_sys::duckdb_time_tz { bits })
            },
        }
    }

    /// Creates a `TIMESTAMP WITH TIME ZONE` value from microseconds since the
    /// epoch.
    #[inline]
    #[must_use]
    pub fn timestamp_tz(micros: i64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_timestamp_tz(libduckdb_sys::duckdb_timestamp {
                    micros,
                })
            },
        }
    }

    /// Creates a `TIMESTAMP_S` value from seconds since the epoch.
    #[inline]
    #[must_use]
    pub fn timestamp_s(seconds: i64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_timestamp_s(libduckdb_sys::duckdb_timestamp_s {
                    seconds,
                })
            },
        }
    }

    /// Creates a `TIMESTAMP_MS` value from milliseconds since the epoch.
    #[inline]
    #[must_use]
    pub fn timestamp_ms(millis: i64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_timestamp_ms(libduckdb_sys::duckdb_timestamp_ms {
                    millis,
                })
            },
        }
    }

    /// Creates a `TIMESTAMP_NS` value from nanoseconds since the epoch.
    #[inline]
    #[must_use]
    pub fn timestamp_ns(nanos: i64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_timestamp_ns(libduckdb_sys::duckdb_timestamp_ns {
                    nanos,
                })
            },
        }
    }

    /// Creates an `INTERVAL` value.
    ///
    /// `DuckDB` intervals are `{ months, days, micros }` and deliberately do not
    /// collapse into a single duration — see
    /// [`DuckInterval`][crate::interval::DuckInterval] (pitfall P8).
    #[inline]
    #[must_use]
    pub fn interval(value: crate::interval::DuckInterval) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe {
                libduckdb_sys::duckdb_create_interval(libduckdb_sys::duckdb_interval {
                    months: value.months,
                    days: value.days,
                    micros: value.micros,
                })
            },
        }
    }

    /// Creates a `BLOB` value from arbitrary bytes.
    ///
    /// Unlike [`varchar`][Self::varchar] this is byte-exact in both directions:
    /// [`as_blob`][Self::as_blob] returns what went in, NUL bytes and all.
    #[must_use]
    pub fn blob(bytes: &[u8]) -> Self {
        // SAFETY: `bytes` is valid for the duration of the call; DuckDB copies it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_blob(
                bytes.as_ptr(),
                libduckdb_sys::idx_t::try_from(bytes.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self { raw }
    }

    /// Creates a `DECIMAL(width, scale)` value from its unscaled integer.
    ///
    /// `unscaled` is the value multiplied by `10^scale` — `DECIMAL(18, 3)`
    /// holding `1.234` has `unscaled == 1234`.
    ///
    /// # Errors
    ///
    /// `duckdb.h`: "The width must be between 1 and 38, and the scale must not
    /// exceed the width" — otherwise `duckdb_create_decimal` returns null,
    /// reported here as an error rather than a `Value` with a null handle.
    pub fn decimal(width: u8, scale: u8, unscaled: i128) -> Result<Self, ExtensionError> {
        // SAFETY: a plain by-value struct; DuckDB validates width/scale itself
        // and reports a violation by returning null.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_decimal(libduckdb_sys::duckdb_decimal {
                width,
                scale,
                value: hugeint_from_i128(unscaled),
            })
        };
        if raw.is_null() {
            return Err(ExtensionError::new(format!(
                "duckdb_create_decimal returned null for DECIMAL({width}, {scale}): \
                 width must be 1..=38 and scale must not exceed width"
            )));
        }
        Ok(Self { raw })
    }

    // ── Composite constructors ───────────────────────────────────────────
    //
    // DuckDB *copies* every input value (`UnwrapValue` + `emplace_back` in
    // `duckdb_value-c.cpp`), so the caller keeps ownership of the children and
    // they are freed normally when their `Value`s drop.

    /// Creates a `STRUCT` value.
    ///
    /// `fields` must line up positionally with `struct_type`'s fields.
    ///
    /// # Errors
    ///
    /// - The number of `fields` does not match the type's field count. This
    ///   check is quack-rs's, and it is not cosmetic:
    ///   `duckdb_create_struct_value` takes **no count argument** and reads
    ///   `values[0..StructType::GetChildCount(type)]`, so passing a short slice
    ///   reads past its end.
    /// - `struct_type` is not a `STRUCT`, or contains an `ANY` / `INVALID` child
    ///   type — `duckdb_create_struct_value` returns null for those.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
    /// let ty = LogicalType::struct_type(&[("x", TypeId::BigInt), ("y", TypeId::Varchar)]);
    /// let v = Value::struct_value(&ty, &[Value::bigint(1), Value::varchar("two")])?;
    /// # let _ = v;
    /// # Ok(())
    /// # }
    /// ```
    pub fn struct_value(
        struct_type: &crate::types::LogicalType,
        fields: &[Self],
    ) -> Result<Self, ExtensionError> {
        // SAFETY: `struct_type` is a live handle for the duration of the call.
        let expected = usize::try_from(unsafe { struct_type.struct_child_count() }).unwrap_or(0);
        if expected != fields.len() {
            return Err(ExtensionError::new(format!(
                "Value::struct_value: the type has {expected} field(s) but {} value(s) were \
                 given; duckdb_create_struct_value reads one value per field and would read \
                 out of bounds",
                fields.len()
            )));
        }
        let mut raws: Vec<duckdb_value> = fields.iter().map(Self::as_raw).collect();
        // SAFETY: `raws` has exactly `expected` entries, which is what DuckDB
        // reads; every entry is a live handle owned by `fields`, and DuckDB
        // copies rather than takes them.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_struct_value(struct_type.as_raw(), raws.as_mut_ptr())
        };
        Self::checked(raw, "duckdb_create_struct_value")
    }

    /// Creates a `LIST` value from its **element** type.
    ///
    /// # `element_type`, not the list type
    ///
    /// `duckdb.h` is self-contradictory here: the prose says "Creates a list
    /// value from a child (element) type", while the `@param` line says "The
    /// type of the list". The implementation settles it —
    /// `duckdb_create_list_value` forwards to
    /// `Value::LIST(const LogicalType &child_type, vector<Value>)`, so it is the
    /// **element** type, and passing `LogicalType::list(..)` instead makes
    /// `DuckDB` try to cast every element to a list and return null.
    ///
    /// For `LIST<BIGINT>`, pass `LogicalType::new(TypeId::BigInt)`.
    ///
    /// # Errors
    ///
    /// Returns an error when `duckdb_create_list_value` reports failure: an
    /// `ANY` or `INVALID` element type, or an item that will not cast to
    /// `element_type`. Passing a `LIST` type is called out specifically, since
    /// it is the mistake the header invites.
    pub fn list_value(
        element_type: &crate::types::LogicalType,
        items: &[Self],
    ) -> Result<Self, ExtensionError> {
        Self::reject_container_type(element_type, crate::types::TypeId::List, "list_value")?;
        let mut raws: Vec<duckdb_value> = items.iter().map(Self::as_raw).collect();
        // SAFETY: the count is passed explicitly and matches `raws`; every entry
        // is a live handle owned by `items`.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_list_value(
                element_type.as_raw(),
                raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked(raw, "duckdb_create_list_value")
    }

    /// Creates an `ARRAY` (fixed-size list) value from its **element** type.
    ///
    /// As with [`list_value`][Self::list_value], this is the element type, not
    /// the array type — `duckdb_create_array_value` forwards to
    /// `Value::ARRAY(const LogicalType &child_type, vector<Value>)`, which
    /// *derives* the array type as `ARRAY(child_type, values.size())`. The
    /// resulting array's size is therefore `items.len()`; there is nothing to
    /// keep in sync.
    ///
    /// # Errors
    ///
    /// Returns an error when `duckdb_create_array_value` reports failure: an
    /// `ANY` / `INVALID` element type, an item that will not cast to
    /// `element_type`, or a length at or above `DuckDB`'s maximum array size.
    pub fn array_value(
        element_type: &crate::types::LogicalType,
        items: &[Self],
    ) -> Result<Self, ExtensionError> {
        Self::reject_container_type(element_type, crate::types::TypeId::Array, "array_value")?;
        let mut raws: Vec<duckdb_value> = items.iter().map(Self::as_raw).collect();
        // SAFETY: as in `list_value`.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_array_value(
                element_type.as_raw(),
                raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked(raw, "duckdb_create_array_value")
    }

    /// Catches the "passed the container type instead of the element type"
    /// mistake that `duckdb.h`'s contradictory `@param` text invites.
    ///
    /// `DuckDB` would report it as a bare null return after failing to cast
    /// every element to a container.
    fn reject_container_type(
        element_type: &crate::types::LogicalType,
        container: crate::types::TypeId,
        method: &str,
    ) -> Result<(), ExtensionError> {
        // SAFETY: `element_type` is a live handle for the duration of the call.
        let id = unsafe {
            crate::types::TypeId::try_from_duckdb_type(libduckdb_sys::duckdb_get_type_id(
                element_type.as_raw(),
            ))
        };
        if id == Some(container) {
            return Err(ExtensionError::new(format!(
                "Value::{method} takes the *element* type, not the {} type: pass the type of \
                 one item (duckdb.h's prose says \"child (element) type\" while its @param \
                 line says \"the type of the {}\"; the implementation takes the element type)",
                container.sql_name(),
                container.sql_name().to_lowercase()
            )));
        }
        Ok(())
    }

    /// Creates an `ENUM` value from its dictionary index.
    ///
    /// # Errors
    ///
    /// Returns an error when `enum_type` is not an `ENUM` or `index` is outside
    /// its dictionary.
    pub fn enum_value(
        enum_type: &crate::types::LogicalType,
        index: u64,
    ) -> Result<Self, ExtensionError> {
        // SAFETY: `enum_type` is a live handle for the duration of the call.
        let raw = unsafe { libduckdb_sys::duckdb_create_enum_value(enum_type.as_raw(), index) };
        Self::checked(raw, "duckdb_create_enum_value")
    }

    /// Wraps a possibly-null constructor result, naming the C function that
    /// produced it.
    fn checked(raw: duckdb_value, api_func: &'static str) -> Result<Self, ExtensionError> {
        if raw.is_null() {
            return Err(ExtensionError::new(format!(
                "{api_func} returned null: the logical type and the supplied values do not \
                 form a valid value (check the type's kind, its child types, and the value count)"
            )));
        }
        Ok(Self { raw })
    }

    /// Returns `true` when this value's SQL type is `SQLNULL`.
    ///
    /// This is **not** [`is_null`][Self::is_null], which reports whether the
    /// *handle* is null — a value that failed to construct, or one moved out
    /// with [`into_raw`][Self::into_raw]. A SQL `NULL` has a perfectly valid
    /// handle.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::value::Value;
    ///
    /// let v = Value::null_value();
    /// assert!(!v.is_null(), "the handle is fine");
    /// assert!(v.is_sql_null(), "the value is SQL NULL");
    /// ```
    #[must_use]
    pub fn is_sql_null(&self) -> bool {
        if self.raw.is_null() {
            return false;
        }
        // SAFETY: `self.raw` is a valid duckdb_value per the constructor contract.
        unsafe { libduckdb_sys::duckdb_is_null_value(self.raw) }
    }

    /// Returns the dictionary index of an `ENUM` value.
    ///
    /// `duckdb.h`: "A `uint64_t`, or `MinValue<uint64>` if the value cannot be
    /// converted" — i.e. `0` for a non-`ENUM` value, which is also a legitimate
    /// index, so check [`type_id`][Self::type_id] first when the type is not
    /// already known.
    #[must_use]
    pub fn as_enum_index(&self) -> u64 {
        if self.raw.is_null() {
            return 0;
        }
        // SAFETY: `self.raw` is a valid duckdb_value per the constructor contract.
        unsafe { libduckdb_sys::duckdb_get_enum_value(self.raw) }
    }

    /// Creates a `MAP` value from parallel key and value slices.
    ///
    /// Named `map` rather than `map_value` because
    /// [`map_value`][Self::map_value] is already the accessor that reads the
    /// value of the `index`-th entry.
    ///
    /// `map_type` is the `MAP` type itself. Requires `duckdb-1-5`:
    /// `duckdb_create_map_value` sits in the unstable region of the C API
    /// struct.
    ///
    /// # Errors
    ///
    /// Returns an error when the slices differ in length, or when
    /// `duckdb_create_map_value` reports failure — `map_type` is not a `MAP`,
    /// a child type is `ANY` / `INVALID`, or a key repeats.
    #[cfg(feature = "duckdb-1-5")]
    pub fn map(
        map_type: &crate::types::LogicalType,
        keys: &[Self],
        values: &[Self],
    ) -> Result<Self, ExtensionError> {
        if keys.len() != values.len() {
            return Err(ExtensionError::new(format!(
                "Value::map: {} key(s) but {} value(s)",
                keys.len(),
                values.len()
            )));
        }
        let mut key_raws: Vec<duckdb_value> = keys.iter().map(Self::as_raw).collect();
        let mut val_raws: Vec<duckdb_value> = values.iter().map(Self::as_raw).collect();
        // SAFETY: both slices have exactly `entry_count` live handles, checked
        // equal above; DuckDB copies rather than takes them.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_map_value(
                map_type.as_raw(),
                key_raws.as_mut_ptr(),
                val_raws.as_mut_ptr(),
                libduckdb_sys::idx_t::try_from(key_raws.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self::checked(raw, "duckdb_create_map_value")
    }

    /// Creates a `UNION` value for the member at `tag_index`.
    ///
    /// Requires `duckdb-1-5`: `duckdb_create_union_value` sits in the unstable
    /// region of the C API struct.
    ///
    /// # Errors
    ///
    /// Returns an error when `union_type` is not a `UNION`, `tag_index` is out
    /// of range, or `value`'s type does not equal that member's declared type —
    /// `DuckDB` compares them exactly and returns null on a mismatch.
    #[cfg(feature = "duckdb-1-5")]
    pub fn union_value(
        union_type: &crate::types::LogicalType,
        tag_index: u64,
        value: &Self,
    ) -> Result<Self, ExtensionError> {
        // SAFETY: both handles are live for the duration of the call, and
        // DuckDB copies the value rather than taking it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_union_value(union_type.as_raw(), tag_index, value.as_raw())
        };
        Self::checked(raw, "duckdb_create_union_value")
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
