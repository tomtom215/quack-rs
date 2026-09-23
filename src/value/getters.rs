// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The typed scalar accessors — `Value::as_i64`, `as_timestamp`, `as_decimal`, …
//!
//! # Why every getter returns `Option`
//!
//! `DuckDB`'s `duckdb_get_*` scalar getters (`CAPIGetValue` in
//! `duckdb_value-c.cpp`) have three hazards, none of which the C API guards:
//!
//! 1. **A SQL `NULL` aborts the process.** The cast of a `NULL` succeeds, then
//!    `Value::GetValue<T>` throws `InternalException`, and a C++ exception
//!    cannot unwind through Rust.
//! 2. **A null handle is dereferenced.** `UnwrapValue` has no null check.
//! 3. **The value is mutated.** `CAPIGetValue` calls the non-`const`
//!    `DefaultTryCastAs`, which overwrites the value's type and payload with
//!    the cast result. `Value::double(1.5).as_i32()` turned the value into
//!    `DOUBLE 2.0`.
//!
//! A failed cast returns `NullValue<T>` — `T::MIN`, `NaN`, `false` — which is
//! also a legitimate value, so the result alone cannot say whether it worked.
//!
//! 4. **Some temporal casts throw.** `DefaultTryCastAs` reports a failed cast
//!    only for pairs `DuckDB` binds through an error-reporting `TryCast` loop.
//!    A few temporal pairs (`time_casts.cpp`) use `TemplatedCastLoop` with a
//!    throwing operator instead: `as_time()` of `'infinity'::TIMESTAMP`, or
//!    `as_timestamp_ns()` of any `TIMESTAMP` after 2262, threw straight
//!    through the C API.
//!
//! `Value::read_cast` addresses all of them: it refuses null handles, SQL
//! `NULL`s and non-scalar source types before any call, checks the payload of
//! a source whose cast could throw against the exact condition `DuckDB` throws
//! on (`checks::cast_guard`), runs the getter on a private *copy* of the
//! value, and reads success off the copy's type afterwards
//! (`DefaultTryCastAs` rewrites the type only when the cast succeeds).

#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::duckdb_get_time_ns;
use libduckdb_sys::{
    duckdb_create_list_value, duckdb_get_bool, duckdb_get_date, duckdb_get_decimal,
    duckdb_get_double, duckdb_get_enum_value, duckdb_get_float, duckdb_get_hugeint,
    duckdb_get_int16, duckdb_get_int32, duckdb_get_int64, duckdb_get_int8, duckdb_get_interval,
    duckdb_get_list_child, duckdb_get_time, duckdb_get_time_tz, duckdb_get_timestamp,
    duckdb_get_timestamp_ms, duckdb_get_timestamp_ns, duckdb_get_timestamp_s,
    duckdb_get_timestamp_tz, duckdb_get_uhugeint, duckdb_get_uint16, duckdb_get_uint32,
    duckdb_get_uint64, duckdb_get_uint8, duckdb_get_uuid, duckdb_get_value_type, duckdb_value,
};

use super::checks::{cast_guard, is_scalar_cast_source, temporal_in_range, time_tz_in_range};
use super::{hugeint_to_i128, uhugeint_to_u128, Value};
use crate::types::TypeId;

impl Value {
    /// Deep-copies the value into a new owned handle.
    ///
    /// The C API has no `duckdb_copy_value`, so this wraps the value in a
    /// one-element `LIST` of its own type (`Value::LIST` copies each child,
    /// and casting a value to its own type is a plain copy) and takes the
    /// child back out (`duckdb_get_list_child` copies again). Both calls
    /// report failure by returning null rather than throwing.
    fn duplicate(&self) -> Option<Self> {
        if self.raw.is_null() {
            return None;
        }
        // SAFETY: `self.raw` is a live, non-null value handle. The returned
        // type is owned by the value and is only read, never destroyed.
        let ty = unsafe { duckdb_get_value_type(self.raw) };
        if ty.is_null() {
            return None;
        }
        let mut child = self.raw;
        // SAFETY: `ty` is valid while `self` lives; `child` points to exactly
        // one live handle, matching `value_count == 1`. DuckDB copies it.
        let list = unsafe { duckdb_create_list_value(ty, &raw mut child, 1) };
        // SAFETY: `list` is either null (never dereferenced by `Value`'s
        // accessors or `Drop`) or an owned handle returned by DuckDB.
        let list = unsafe { Self::from_raw(list) };
        if list.is_null() {
            return None;
        }
        // SAFETY: `list` is a live `LIST` with one child; index 0 is in range.
        let copy = unsafe { duckdb_get_list_child(list.raw, 0) };
        // SAFETY: `copy` is null or a new owned handle.
        let copy = unsafe { Self::from_raw(copy) };
        (!copy.is_null()).then_some(copy)
    }

    /// Runs a `DuckDB` scalar getter that casts to `target`, without its three
    /// hazards (see the module docs).
    ///
    /// `read` receives a live, non-null handle holding a non-NULL value of a
    /// type in [`is_scalar_cast_source`]. It must only pass it to the
    /// `duckdb_get_*` function whose target type is `target`.
    fn read_cast<R>(&self, target: TypeId, read: impl FnOnce(duckdb_value) -> R) -> Option<R> {
        let source = self.type_id()?;
        if self.is_sql_null() || !is_scalar_cast_source(source) {
            return None;
        }
        // A pair `DuckDB` converts with a throwing loop: check the payload
        // first. `cast_guard(t, t)` is `None`, so the same-type read inside
        // `timestamp_payload` does not come back here.
        if let Some(guard) = cast_guard(source, target) {
            if !guard.accepts(self.timestamp_payload(source)?) {
                return None;
            }
        }
        let copy = self.duplicate()?;
        let out = read(copy.raw);
        // `DefaultTryCastAs` assigns the target type only when the cast
        // succeeded; on failure the copy keeps its source type and `out` is
        // DuckDB's `NullValue<T>` sentinel, which must not be reported.
        (copy.type_id() == Some(target)).then_some(out)
    }

    /// The raw `int64` of a `TIMESTAMP`-family value, read at its own type.
    ///
    /// Reading a value at its own type never casts: `Value::TryCastAs` copies
    /// it when the source and target types are equal. `None` for any other
    /// type.
    fn timestamp_payload(&self, source: TypeId) -> Option<i64> {
        // SAFETY (every arm): `read_cast` passes a live, non-NULL handle of
        // exactly the getter's type.
        match source {
            TypeId::Timestamp => {
                self.read_cast(source, |v| unsafe { duckdb_get_timestamp(v) }.micros)
            }
            TypeId::TimestampTz => {
                self.read_cast(source, |v| unsafe { duckdb_get_timestamp_tz(v) }.micros)
            }
            TypeId::TimestampS => {
                self.read_cast(source, |v| unsafe { duckdb_get_timestamp_s(v) }.seconds)
            }
            TypeId::TimestampMs => {
                self.read_cast(source, |v| unsafe { duckdb_get_timestamp_ms(v) }.millis)
            }
            TypeId::TimestampNs => {
                self.read_cast(source, |v| unsafe { duckdb_get_timestamp_ns(v) }.nanos)
            }
            _ => None,
        }
    }

    /// Whether the handle is non-null, the value is not SQL `NULL`, and its
    /// type is exactly `expected`. Used by the getters that do not cast.
    fn holds(&self, expected: TypeId) -> bool {
        self.type_id() == Some(expected) && !self.is_sql_null()
    }

    /// Reads the value as an `i32`, casting it to `INTEGER` the way SQL's
    /// `TRY_CAST` would.
    ///
    /// Returns `None` if the handle is null, the value is SQL `NULL`, its type
    /// is not a scalar (`LIST`, `STRUCT`, `BLOB`, `ENUM`, …), or the cast fails
    /// (`'abc'`, or a number out of range). Casts follow `DuckDB`: a `DOUBLE`
    /// `1.5` reads as `2`, a `VARCHAR` `'42'` as `42`.
    ///
    /// The value itself is never modified. The same rules apply to every
    /// casting `as_*` accessor; see [`as_i32_or`][Self::as_i32_or] and its
    /// siblings for a defaulting form.
    #[must_use]
    pub fn as_i32(&self) -> Option<i32> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Integer, |v| unsafe { duckdb_get_int32(v) })
    }

    /// Reads the value as an `i64` (`BIGINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::BigInt, |v| unsafe { duckdb_get_int64(v) })
    }

    /// Reads the value as an `f32` (`FLOAT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_f32(&self) -> Option<f32> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Float, |v| unsafe { duckdb_get_float(v) })
    }

    /// Reads the value as an `f64` (`DOUBLE`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Double, |v| unsafe { duckdb_get_double(v) })
    }

    /// Reads the value as a `bool` (`BOOLEAN`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Boolean, |v| unsafe { duckdb_get_bool(v) })
    }

    /// Reads the value as an `i8` (`TINYINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_i8(&self) -> Option<i8> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TinyInt, |v| unsafe { duckdb_get_int8(v) })
    }

    /// Reads the value as an `i16` (`SMALLINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_i16(&self) -> Option<i16> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::SmallInt, |v| unsafe { duckdb_get_int16(v) })
    }

    /// Reads the value as a `u8` (`UTINYINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_u8(&self) -> Option<u8> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::UTinyInt, |v| unsafe { duckdb_get_uint8(v) })
    }

    /// Reads the value as a `u16` (`USMALLINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_u16(&self) -> Option<u16> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::USmallInt, |v| unsafe { duckdb_get_uint16(v) })
    }

    /// Reads the value as a `u32` (`UINTEGER`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::UInteger, |v| unsafe { duckdb_get_uint32(v) })
    }

    /// Reads the value as a `u64` (`UBIGINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::UBigInt, |v| unsafe { duckdb_get_uint64(v) })
    }

    /// Reads the value as an `i128` (`HUGEINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_i128(&self) -> Option<i128> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::HugeInt, |v| unsafe { duckdb_get_hugeint(v) })
            .map(hugeint_to_i128)
    }

    /// Reads the value as a `u128` (`UHUGEINT`). `None` on a null handle, SQL
    /// `NULL`, non-scalar type or failed cast; see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_u128(&self) -> Option<u128> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::UHugeInt, |v| unsafe { duckdb_get_uhugeint(v) })
            .map(uhugeint_to_u128)
    }

    // ── Temporal and UUID ────────────────────────────────────────────────
    //
    // A table function declared with `.named_param("since", TypeId::Timestamp)`
    // hands the bind callback a `duckdb_value`; these read it without
    // reparsing DuckDB's rendering. They cast like the numeric getters, so a
    // `VARCHAR` '2024-01-01' reads as a `DATE`.
    //
    // A temporal getter also returns `None` when the conversion is one DuckDB
    // refuses in SQL: the time of an infinite timestamp, a TIMESTAMP after
    // 2262 as TIMESTAMP_NS, or a result outside the target type's range (the
    // TIMESTAMP maximum rounds to a TIMESTAMP_S that DuckDB cannot convert
    // back). Each agrees with `CAST(x AS target)` in SQL.

    /// Reads a `DATE` as days since 1970-01-01. Decode it with
    /// [`datetime::date_from_days`][crate::datetime::date_from_days].
    ///
    /// `None` on a null handle, SQL `NULL`, non-scalar type or failed cast;
    /// see [`as_i32`][Self::as_i32].
    #[must_use]
    pub fn as_date(&self) -> Option<i32> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Date, |v| unsafe { duckdb_get_date(v) }.days)
    }

    /// Reads a `TIME` as microseconds since midnight. `None` on a null handle,
    /// SQL `NULL`, non-scalar type or failed cast — including the time of an
    /// infinite `TIMESTAMP`, which `DuckDB` refuses (it used to abort the
    /// process here).
    #[must_use]
    pub fn as_time(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Time, |v| unsafe { duckdb_get_time(v) }.micros)
            .filter(|&v| temporal_in_range(TypeId::Time, v))
    }

    /// Reads a `TIMETZ` as `DuckDB`'s packed 64-bit representation. Decode it
    /// with [`datetime::time_tz_from_bits`][crate::datetime::time_tz_from_bits].
    ///
    /// `None` on a null handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_time_tz(&self) -> Option<u64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimeTz, |v| unsafe { duckdb_get_time_tz(v) }.bits)
            .filter(|&bits| time_tz_in_range(bits))
    }

    /// Reads a `TIME_NS` as nanoseconds since midnight (`DuckDB` 1.5.0+).
    ///
    /// `None` on a null handle, SQL `NULL`, non-scalar type or failed cast.
    #[cfg(feature = "duckdb-1-5")]
    #[must_use]
    pub fn as_time_ns(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimeNs, |v| unsafe { duckdb_get_time_ns(v) }.nanos)
            .filter(|&v| temporal_in_range(TypeId::TimeNs, v))
    }

    /// Reads a `TIMESTAMP` as microseconds since the epoch. `None` on a null
    /// handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_timestamp(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(
            TypeId::Timestamp,
            |v| unsafe { duckdb_get_timestamp(v) }.micros,
        )
        .filter(|&v| temporal_in_range(TypeId::Timestamp, v))
    }

    /// Reads a `TIMESTAMPTZ` as microseconds since the epoch, in UTC. `None`
    /// on a null handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_timestamp_tz(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimestampTz, |v| {
            unsafe { duckdb_get_timestamp_tz(v) }.micros
        })
        .filter(|&v| temporal_in_range(TypeId::TimestampTz, v))
    }

    /// Reads a `TIMESTAMP_S` as seconds since the epoch. `None` on a null
    /// handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_timestamp_s(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimestampS, |v| {
            unsafe { duckdb_get_timestamp_s(v) }.seconds
        })
        .filter(|&v| temporal_in_range(TypeId::TimestampS, v))
    }

    /// Reads a `TIMESTAMP_MS` as milliseconds since the epoch. `None` on a
    /// null handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_timestamp_ms(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimestampMs, |v| {
            unsafe { duckdb_get_timestamp_ms(v) }.millis
        })
        .filter(|&v| temporal_in_range(TypeId::TimestampMs, v))
    }

    /// Reads a `TIMESTAMP_NS` as nanoseconds since the epoch. `None` on a
    /// null handle, SQL `NULL`, non-scalar type or failed cast — including a
    /// timestamp outside `TIMESTAMP_NS`'s range (before 1677-09-22 or after
    /// 2262-04-11), which `DuckDB` refuses (it used to abort the process here).
    #[must_use]
    pub fn as_timestamp_ns(&self) -> Option<i64> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::TimestampNs, |v| {
            unsafe { duckdb_get_timestamp_ns(v) }.nanos
        })
        .filter(|&v| temporal_in_range(TypeId::TimestampNs, v))
    }

    /// Reads an `INTERVAL`. `None` on a null handle, SQL `NULL`, non-scalar
    /// type or failed cast.
    #[must_use]
    pub fn as_interval(&self) -> Option<crate::interval::DuckInterval> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        let raw = self.read_cast(TypeId::Interval, |v| unsafe { duckdb_get_interval(v) })?;
        Some(crate::interval::DuckInterval {
            months: raw.months,
            days: raw.days,
            micros: raw.micros,
        })
    }

    /// Reads a `UUID` as its **textual** 128 bits, matching
    /// [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid]
    /// and [`uuid`][Self::uuid].
    ///
    /// `DuckDB` undoes its internal top-bit flip itself here, so this is the
    /// value the UUID renders as — not the raw `HUGEINT` a `UUID` vector holds.
    /// `None` on a null handle, SQL `NULL`, non-scalar type or failed cast.
    #[must_use]
    pub fn as_uuid(&self) -> Option<u128> {
        // SAFETY: `read_cast` passes a live, non-NULL scalar handle.
        self.read_cast(TypeId::Uuid, |v| unsafe { duckdb_get_uuid(v) })
            .map(uhugeint_to_u128)
    }

    // ── Non-casting getters ──────────────────────────────────────────────

    /// Reads a `DECIMAL` as its width, scale and unscaled value; the
    /// represented number is `value / 10^scale`.
    ///
    /// Does not cast: returns `None` unless the value is a non-NULL `DECIMAL`
    /// (and for a null handle).
    #[must_use]
    pub fn as_decimal(&self) -> Option<crate::datetime::Decimal> {
        if !self.holds(TypeId::Decimal) {
            return None;
        }
        // SAFETY: `self.raw` is a live handle holding a non-NULL DECIMAL;
        // `duckdb_get_decimal` reads it through a const reference.
        let raw = unsafe { duckdb_get_decimal(self.raw) };
        Some(crate::datetime::Decimal {
            width: raw.width,
            scale: raw.scale,
            value: hugeint_to_i128(raw.value),
        })
    }

    /// Returns the dictionary index of an `ENUM` value.
    ///
    /// Does not cast: returns `None` unless the value is a non-NULL `ENUM`
    /// (and for a null handle). The C function returns `0` in those cases,
    /// which is also a legitimate index.
    #[must_use]
    pub fn as_enum_index(&self) -> Option<u64> {
        if !self.holds(TypeId::Enum) {
            return None;
        }
        // SAFETY: `self.raw` is a live handle holding a non-NULL ENUM;
        // `duckdb_get_enum_value` reads a copy of it.
        Some(unsafe { duckdb_get_enum_value(self.raw) })
    }
}
