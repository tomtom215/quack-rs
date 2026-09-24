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
mod checks;
mod composite;
mod defaults;
mod getters;
mod hugeint;
mod nested;
mod render_guard;
mod scalars;
mod temporal;
pub(crate) mod temporal_checks;

pub(crate) use checks::validate_decimal;

// Re-exported unqualified so the eight call sites across this module and
// `query` keep their existing `crate::value::hugeint_from_i128` paths.
pub(crate) use hugeint::{
    hugeint_from_i128, hugeint_to_i128, uhugeint_from_u128, uhugeint_to_u128,
};

use std::ffi::CStr;
use std::os::raw::c_char;

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::duckdb_value_to_string;
use libduckdb_sys::{duckdb_destroy_value, duckdb_free, duckdb_get_varchar, duckdb_value};

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
/// - [`as_str`][Value::as_str] — any scalar rendered as text → `Result<String>`
/// - [`as_blob`][Value::as_blob] — BLOB → `Result<Vec<u8>>`
/// - [`as_i32`][Value::as_i32] — cast to INTEGER → `Option<i32>`
/// - [`as_i64`][Value::as_i64] — cast to BIGINT → `Option<i64>`
/// - [`as_f32`][Value::as_f32] — cast to FLOAT → `Option<f32>`
/// - [`as_f64`][Value::as_f64] — cast to DOUBLE → `Option<f64>`
/// - [`as_bool`][Value::as_bool] — cast to BOOLEAN → `Option<bool>`
///
/// No accessor calls into `DuckDB` for a null handle or a SQL `NULL` — the
/// scalar getters return `None`, `as_str`/`as_blob` an error — and none
/// modifies the value. The `as_*_or(default)` forms (e.g.
/// [`as_i64_or`][Value::as_i64_or]) fold every `None` into a default.
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
    /// Returns `ExtensionError` if the handle is null, the value is SQL `NULL`
    /// (`duckdb_get_varchar` would throw on it, aborting the process), the
    /// text is not valid UTF-8, or the value holds a timestamp `DuckDB` cannot
    /// render — see [`UNRENDERABLE`]. `DuckDB` builds such timestamps from
    /// ordinary SQL (`make_timestamp(-9223372036854775808)`), and rendering one
    /// throws through the C API, which aborted the process.
    pub fn as_str(&self) -> Result<String, ExtensionError> {
        if self.raw.is_null() {
            return Err(ExtensionError::new("Value is null"));
        }
        if self.is_sql_null() {
            return Err(ExtensionError::new("Value is SQL NULL"));
        }
        if !self.renderable() {
            return Err(ExtensionError::new(UNRENDERABLE));
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
    /// Returns `None` if the handle is null, the rendered text is not valid
    /// UTF-8, or the value holds a timestamp `DuckDB` cannot render (see
    /// [`as_str`][Self::as_str]).
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn display_string(&self) -> Option<String> {
        if self.raw.is_null() || !self.renderable() {
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

    /// Returns the [`TypeId`][crate::types::TypeId] this value actually holds.
    ///
    /// The casting `as_*` accessors follow SQL's `TRY_CAST` — a `VARCHAR`
    /// `'42'` reads as `42` through [`as_i64`][Self::as_i64]. Check the type
    /// first when a value whose type you did not choose — a named parameter,
    /// a bound constant, a config option — must be of one particular type.
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
    ///     TypeId::BigInt | TypeId::Integer => value.as_i64(),
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

/// What [`Value::as_str`] reports for a value `DuckDB` cannot render.
///
/// That is a value holding a timestamp or time payload outside the range
/// `DuckDB` converts, or an `ARRAY` / `UNION` of a temporal type, whose
/// elements the C API gives no way to check first.
pub const UNRENDERABLE: &str = "Value cannot be rendered: it holds a timestamp or time payload \
     outside the range DuckDB converts (or an ARRAY / UNION of a temporal type, which cannot be \
     checked), and DuckDB's rendering would throw through the C API and abort the process";

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
            rendered(&Value::timestamp(0).expect("in range")),
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
        }
        .expect("2026-08-18 is a valid date");
        assert_eq!(Value::date(days).as_date(), Some(days));

        let micros = 1_700_000_000_000_000_i64;
        assert_eq!(
            Value::timestamp(micros).expect("in range").as_timestamp(),
            Some(micros)
        );
        assert_eq!(Value::bigint(i64::MIN).as_i64(), Some(i64::MIN));
        assert_eq!(Value::bigint(i64::MAX).as_i64(), Some(i64::MAX));
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
                Some(bits),
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
