// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Binding a [`PreparedStatement`]'s parameters: the typed `bind_*` methods and `bind_value`.

use libduckdb_sys::{
    duckdb_bind_blob, duckdb_bind_boolean, duckdb_bind_double, duckdb_bind_int64, duckdb_bind_null,
    duckdb_bind_varchar_length, idx_t, DuckDBSuccess,
};

use super::PreparedStatement;
use crate::error::ExtensionError;

impl PreparedStatement {
    /// Binds a `BIGINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` rejects the binding (bad index or type).
    pub fn bind_i64(&self, index: usize, value: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_int64(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `DOUBLE` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_f64(&self, index: usize, value: f64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_double(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `BOOLEAN` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_bool(&self, index: usize, value: bool) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_boolean(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `VARCHAR` at 1-based `index`.
    ///
    /// The length is passed explicitly, so embedded NUL bytes are preserved and
    /// no `CString` conversion can fail.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_str(&self, index: usize, value: &str) -> Result<(), ExtensionError> {
        // SAFETY: `value` is valid for the duration of the call; the length is
        // passed explicitly so the pointer need not be NUL-terminated.
        let state = unsafe {
            duckdb_bind_varchar_length(
                self.statement,
                index as idx_t,
                value.as_ptr().cast::<std::os::raw::c_char>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            )
        };
        self.check(state, index)
    }

    /// Binds a `BLOB` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_blob(&self, index: usize, value: &[u8]) -> Result<(), ExtensionError> {
        // SAFETY: `value` is valid for the duration of the call.
        let state = unsafe {
            duckdb_bind_blob(
                self.statement,
                index as idx_t,
                value.as_ptr().cast::<std::os::raw::c_void>(),
                idx_t::try_from(value.len()).unwrap_or(idx_t::MAX),
            )
        };
        self.check(state, index)
    }

    /// Binds SQL `NULL` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_null(&self, index: usize) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { duckdb_bind_null(self.statement, index as idx_t) },
            index,
        )
    }

    /// Binds a `TINYINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i8(&self, index: usize, value: i8) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int8(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `SMALLINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i16(&self, index: usize, value: i16) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int16(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds an `INTEGER` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i32(&self, index: usize, value: i32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_int32(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UTINYINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u8(&self, index: usize, value: u8) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint8(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `USMALLINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u16(&self, index: usize, value: u16) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint16(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UINTEGER` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u32(&self, index: usize, value: u32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint32(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `UBIGINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u64(&self, index: usize, value: u64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uint64(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `FLOAT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_f32(&self, index: usize, value: f32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_float(self.statement, index as idx_t, value) },
            index,
        )
    }

    /// Binds a `HUGEINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_i128(&self, index: usize, value: i128) -> Result<(), ExtensionError> {
        let raw = crate::value::hugeint_from_i128(value);
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_hugeint(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `UHUGEINT` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_u128(&self, index: usize, value: u128) -> Result<(), ExtensionError> {
        let raw = crate::value::uhugeint_from_u128(value);
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_uhugeint(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `DECIMAL(width, scale)` at 1-based `index`.
    ///
    /// `unscaled` is the value multiplied by `10^scale`.
    ///
    /// # Errors
    ///
    /// Returns an error, without calling `DuckDB`, when `width` is not in
    /// `1..=38`, `scale > width`, or `unscaled` has more than `width` digits.
    /// `duckdb_bind_decimal` validates none of these: a bad width or an
    /// `unscaled` too wide for the physical type throws (aborting the
    /// process), and for `width <= 18` it keeps only the low 64 bits of
    /// `unscaled`, silently binding a different number. Otherwise see
    /// [`bind_i64`][Self::bind_i64].
    pub fn bind_decimal(
        &self,
        index: usize,
        width: u8,
        scale: u8,
        unscaled: i128,
    ) -> Result<(), ExtensionError> {
        crate::value::validate_decimal(width, scale, unscaled)?;
        let raw = libduckdb_sys::duckdb_decimal {
            width,
            scale,
            value: crate::value::hugeint_from_i128(unscaled),
        };
        // SAFETY: `self.statement` is valid for this value's lifetime, and the
        // decimal was validated above so `Value::DECIMAL` cannot throw.
        self.check(
            unsafe { libduckdb_sys::duckdb_bind_decimal(self.statement, index as idx_t, raw) },
            index,
        )
    }

    /// Binds a `DATE` at 1-based `index`, as days since 1970-01-01.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_date(&self, index: usize, days: i32) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_date(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_date { days },
                )
            },
            index,
        )
    }

    /// Binds a `TIME` at 1-based `index`, as microseconds since midnight.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_time(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_time(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_time { micros },
                )
            },
            index,
        )
    }

    /// Binds a `TIMESTAMP` at 1-based `index`, as microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_timestamp(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_timestamp(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_timestamp { micros },
                )
            },
            index,
        )
    }

    /// Binds a `TIMESTAMP WITH TIME ZONE` at 1-based `index`, as microseconds
    /// since the epoch.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_timestamp_tz(&self, index: usize, micros: i64) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_timestamp_tz(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_timestamp { micros },
                )
            },
            index,
        )
    }

    /// Binds an `INTERVAL` at 1-based `index`.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    pub fn bind_interval(
        &self,
        index: usize,
        value: crate::interval::DuckInterval,
    ) -> Result<(), ExtensionError> {
        // SAFETY: `self.statement` is valid for this value's lifetime.
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_interval(
                    self.statement,
                    index as idx_t,
                    libduckdb_sys::duckdb_interval {
                        months: value.months,
                        days: value.days,
                        micros: value.micros,
                    },
                )
            },
            index,
        )
    }

    /// Binds an arbitrary [`Value`][crate::value::Value] at 1-based `index`.
    ///
    /// This is the escape hatch for everything the typed `bind_*` methods do
    /// not cover — `STRUCT`, `LIST`, `MAP`, `ARRAY`, `UNION`, `ENUM`, `UUID`,
    /// `BIT`, and any type a future `DuckDB` adds. `DuckDB` copies the value, so
    /// `value` may be dropped immediately afterwards.
    ///
    /// # Errors
    ///
    /// See [`bind_i64`][Self::bind_i64].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::types::{LogicalType, TypeId};
    /// use quack_rs::value::Value;
    ///
    /// # fn demo(stmt: &quack_rs::query::PreparedStatement)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// let ty = LogicalType::list(TypeId::BigInt);
    /// let list = Value::list_value(&ty, &[Value::bigint(1), Value::bigint(2)])?;
    /// stmt.bind_value(1, &list)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn bind_value(
        &self,
        index: usize,
        value: &crate::value::Value,
    ) -> Result<(), ExtensionError> {
        if value.is_null() {
            return Err(ExtensionError::new(format!(
                "bind_value: the Value at parameter {index} has a null handle"
            )));
        }
        // SAFETY: `self.statement` is valid for this value's lifetime, and
        // `value` outlives the call (DuckDB copies it).
        self.check(
            unsafe {
                libduckdb_sys::duckdb_bind_value(self.statement, index as idx_t, value.as_raw())
            },
            index,
        )
    }

    fn check(
        &self,
        state: libduckdb_sys::duckdb_state,
        index: usize,
    ) -> Result<(), ExtensionError> {
        if state == DuckDBSuccess {
            Ok(())
        } else {
            Err(ExtensionError::new(format!(
                "failed to bind parameter {index} (statement has {} parameter(s))",
                self.parameter_count()
            )))
        }
    }
}
