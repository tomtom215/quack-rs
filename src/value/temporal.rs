// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Temporal constructors — `Value::date`, `time`, `timestamp_s`, …
//!
//! # Why most of them return `Result`
//!
//! `DuckDB`'s `duckdb_create_time`, `_time_tz_value`, `_time_ns`,
//! `_timestamp`, `_timestamp_tz`, `_timestamp_s`, `_timestamp_ms` and
//! `_timestamp_ns` store any 64-bit payload without a check. Everything done
//! with an out-of-range one afterwards goes wrong: rendering it
//! ([`as_str`][Value::as_str], `Debug`, `display_string`) or casting it runs
//! `DuckDB` code that throws a C++ exception through the C API (a process
//! abort from Rust), reads out of bounds (`Value::time_ns(i64::MAX)` crashed
//! `StringAsTime` with `SIGSEGV`) or prints garbage (`Value::time(-1)` rendered
//! `00:00:00.00000/`). These constructors accept exactly the range `DuckDB`'s
//! own SQL produces (see `checks::temporal_in_range`) and return an error for
//! anything else.
//!
//! `DATE` and `INTERVAL` stay infallible: `DuckDB` renders and converts every
//! `i32` day count and every interval without throwing.

use libduckdb_sys::{
    duckdb_create_date, duckdb_create_interval, duckdb_create_time, duckdb_create_time_tz_value,
    duckdb_create_timestamp, duckdb_create_timestamp_ms, duckdb_create_timestamp_ns,
    duckdb_create_timestamp_s, duckdb_create_timestamp_tz, duckdb_date, duckdb_interval,
    duckdb_time, duckdb_time_tz, duckdb_timestamp, duckdb_timestamp_ms, duckdb_timestamp_ns,
    duckdb_timestamp_s,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{duckdb_create_time_ns, duckdb_time_ns};

use super::temporal_checks::{temporal_in_range, time_tz_in_range};
use super::Value;
use crate::error::ExtensionError;
use crate::types::TypeId;

/// The error for an out-of-range payload: names the constructor, the value,
/// the SQL type and its range.
fn out_of_range(method: &str, value: impl std::fmt::Display, range: &str) -> ExtensionError {
    ExtensionError::new(format!(
        "Value::{method}({value}) is outside DuckDB's range for the type: {range}. DuckDB \
         would store it unchecked and then fail, crash or print garbage rendering or casting it"
    ))
}

/// `Ok(v)` if `v` is in range for `type_id`, else the error for `method`.
fn check(type_id: TypeId, v: i64, method: &str, range: &str) -> Result<i64, ExtensionError> {
    if temporal_in_range(type_id, v) {
        Ok(v)
    } else {
        Err(out_of_range(method, v, range))
    }
}

/// The range text for the microsecond-based `TIMESTAMP` types: the finite
/// span `DuckDB` can render and convert, and the two infinities.
const TIMESTAMP_RANGE: &str = "TIMESTAMP 290309-12-22 (BC) 00:00:00 to \
                               294247-01-10 04:00:54.775806, or ±infinity (±i64::MAX)";

impl Value {
    /// Creates a `DATE` value from days since 1970-01-01.
    ///
    /// Infallible: `DuckDB` renders and converts every `i32`. `i32::MAX` and
    /// `-i32::MAX` are `infinity` and `-infinity`
    /// ([`datetime::DATE_INFINITY_DAYS`][crate::datetime::DATE_INFINITY_DAYS]).
    #[inline]
    #[must_use]
    pub fn date(days: i32) -> Self {
        Self {
            // SAFETY: a plain by-value struct DuckDB accepts unconditionally;
            // the returned handle is owned by this `Value`.
            raw: unsafe { duckdb_create_date(duckdb_date { days }) },
        }
    }

    /// Creates a `TIME` value from microseconds since midnight.
    ///
    /// # Errors
    ///
    /// `micros_since_midnight` is outside `0..=86_400_000_000`
    /// (`00:00:00`–`24:00:00`, `DuckDB`'s `TIME` range).
    pub fn time(micros_since_midnight: i64) -> Result<Self, ExtensionError> {
        let micros = check(
            TypeId::Time,
            micros_since_midnight,
            "time",
            "TIME 0..=86_400_000_000 microseconds, 00:00:00 to 24:00:00",
        )?;
        Ok(Self {
            // SAFETY: a plain by-value struct, range-checked above; the
            // returned handle is owned by this `Value`.
            raw: unsafe { duckdb_create_time(duckdb_time { micros }) },
        })
    }

    /// Creates a `TIME WITH TIME ZONE` value from its packed 64-bit encoding.
    ///
    /// `DuckDB` packs `TIME_TZ` as 40 bits of microseconds and 24 bits of UTC
    /// offset. Build the encoding with
    /// [`time_tz_bits`][crate::datetime::time_tz_bits] rather than assembling it
    /// by hand; every encoding it returns is accepted here.
    ///
    /// # Errors
    ///
    /// The time field exceeds `24:00:00` or the offset field encodes an
    /// offset beyond `±15:59:59`.
    pub fn time_tz(bits: u64) -> Result<Self, ExtensionError> {
        if !time_tz_in_range(bits) {
            return Err(out_of_range(
                "time_tz",
                format_args!("{bits:#x}"),
                "TIMETZ time 00:00:00 to 24:00:00 in the high 40 bits, offset ±15:59:59 in the low 24",
            ));
        }
        Ok(Self {
            // SAFETY: a plain by-value struct, range-checked above.
            raw: unsafe { duckdb_create_time_tz_value(duckdb_time_tz { bits }) },
        })
    }

    /// Creates a `TIME_NS` value (time of day with nanosecond precision) from a
    /// raw nanosecond count (`DuckDB` 1.5.0+).
    ///
    /// Pairs with [`as_time_ns`][Value::as_time_ns] and the
    /// [`TypeId::TimeNs`][crate::types::TypeId::TimeNs] column type.
    ///
    /// # Errors
    ///
    /// `nanos` is outside `0..=86_400_000_000_000` (`00:00:00`–`24:00:00`).
    #[cfg(feature = "duckdb-1-5")]
    pub fn time_ns(nanos: i64) -> Result<Self, ExtensionError> {
        let nanos = check(
            TypeId::TimeNs,
            nanos,
            "time_ns",
            "TIME_NS 0..=86_400_000_000_000 nanoseconds, 00:00:00 to 24:00:00",
        )?;
        // SAFETY: a plain by-value struct, range-checked above.
        let raw = unsafe { duckdb_create_time_ns(duckdb_time_ns { nanos }) };
        Ok(Self { raw })
    }

    /// Creates a `TIMESTAMP` value from microseconds since the epoch.
    ///
    /// # Errors
    ///
    /// `micros` is below `-9_223_372_022_400_000_000`
    /// (`290309-12-22 (BC) 00:00:00`) and is not `-i64::MAX` (`-infinity`).
    /// Every larger value is valid; `i64::MAX` is `infinity`.
    pub fn timestamp(micros: i64) -> Result<Self, ExtensionError> {
        let micros = check(TypeId::Timestamp, micros, "timestamp", TIMESTAMP_RANGE)?;
        Ok(Self {
            // SAFETY: a plain by-value struct, range-checked above.
            raw: unsafe { duckdb_create_timestamp(duckdb_timestamp { micros }) },
        })
    }

    /// Creates a `TIMESTAMP WITH TIME ZONE` value from microseconds since the
    /// epoch.
    ///
    /// # Errors
    ///
    /// As [`timestamp`][Self::timestamp].
    pub fn timestamp_tz(micros: i64) -> Result<Self, ExtensionError> {
        let micros = check(TypeId::TimestampTz, micros, "timestamp_tz", TIMESTAMP_RANGE)?;
        Ok(Self {
            // SAFETY: a plain by-value struct, range-checked above.
            raw: unsafe { duckdb_create_timestamp_tz(duckdb_timestamp { micros }) },
        })
    }

    /// Creates a `TIMESTAMP_S` value from seconds since the epoch.
    ///
    /// # Errors
    ///
    /// `seconds` is outside `-9_223_372_022_400..=9_223_372_036_854` and is
    /// not `±i64::MAX` (`±infinity`): `DuckDB` converts a `TIMESTAMP_S` to
    /// microseconds to render or cast it, and throws when that overflows.
    pub fn timestamp_s(seconds: i64) -> Result<Self, ExtensionError> {
        let seconds = check(TypeId::TimestampS, seconds, "timestamp_s", TIMESTAMP_RANGE)?;
        // SAFETY: a plain by-value struct, range-checked above.
        let raw = unsafe { duckdb_create_timestamp_s(duckdb_timestamp_s { seconds }) };
        Ok(Self { raw })
    }

    /// Creates a `TIMESTAMP_MS` value from milliseconds since the epoch.
    ///
    /// # Errors
    ///
    /// `millis` is outside
    /// `-9_223_372_022_400_000..=9_223_372_036_854_775` and is not
    /// `±i64::MAX` (`±infinity`), for the reason given on
    /// [`timestamp_s`][Self::timestamp_s].
    pub fn timestamp_ms(millis: i64) -> Result<Self, ExtensionError> {
        let millis = check(TypeId::TimestampMs, millis, "timestamp_ms", TIMESTAMP_RANGE)?;
        // SAFETY: a plain by-value struct, range-checked above.
        let raw = unsafe { duckdb_create_timestamp_ms(duckdb_timestamp_ms { millis }) };
        Ok(Self { raw })
    }

    /// Creates a `TIMESTAMP_NS` value from nanoseconds since the epoch.
    ///
    /// # Errors
    ///
    /// `nanos` is below `-9_223_286_400_000_000_000` (`1677-09-22 00:00:00`)
    /// and is not `-i64::MAX` (`-infinity`). Every larger value is valid;
    /// `i64::MAX` is `infinity`.
    pub fn timestamp_ns(nanos: i64) -> Result<Self, ExtensionError> {
        let nanos = check(
            TypeId::TimestampNs,
            nanos,
            "timestamp_ns",
            "TIMESTAMP_NS 1677-09-22 00:00:00 to 2262-04-11 23:47:16.854775806, or ±infinity (±i64::MAX)",
        )?;
        // SAFETY: a plain by-value struct, range-checked above.
        let raw = unsafe { duckdb_create_timestamp_ns(duckdb_timestamp_ns { nanos }) };
        Ok(Self { raw })
    }

    /// Creates an `INTERVAL` value.
    ///
    /// `DuckDB` intervals are `{ months, days, micros }` and deliberately do not
    /// collapse into a single duration — see
    /// [`DuckInterval`][crate::interval::DuckInterval] (pitfall P8).
    /// Infallible: every combination renders.
    ///
    /// Note that the derived `PartialEq` on `DuckInterval` compares the three
    /// fields, while `DuckDB`'s SQL `=` compares normalized intervals:
    /// `INTERVAL '1 month' = INTERVAL '30 days'` and
    /// `INTERVAL '1 day' = INTERVAL '24 hours'` are both `true` in SQL, but the
    /// corresponding `DuckInterval`s are not equal.
    #[inline]
    #[must_use]
    pub fn interval(value: crate::interval::DuckInterval) -> Self {
        Self {
            // SAFETY: a plain by-value struct DuckDB accepts unconditionally.
            raw: unsafe {
                duckdb_create_interval(duckdb_interval {
                    months: value.months,
                    days: value.days,
                    micros: value.micros,
                })
            },
        }
    }
}
