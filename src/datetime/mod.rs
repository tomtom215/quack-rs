// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Calendar conversions for `DuckDB`'s temporal types.
//!
//! `VectorReader`/`VectorWriter` move `DATE`, `TIME` and `TIMESTAMP` as the raw
//! integers `DuckDB` stores: days since 1970-01-01, microseconds since midnight,
//! microseconds since the epoch. Turning those into year/month/day means
//! implementing the proleptic Gregorian calendar — including `DuckDB`'s
//! infinity sentinels — which is exactly the kind of thing an extension should
//! not be reimplementing.
//!
//! `DuckDB` already exposes the conversions (`duckdb_from_date`,
//! `duckdb_to_date`, `duckdb_from_time`, `duckdb_from_timestamp`, …) and they
//! sit in the **stable** prefix of the C extension API, so they work on every
//! release from v1.2.0 onwards and need no feature flag. This module wraps them
//! in plain Rust structs.
//!
//! Using `DuckDB`'s own routines also means the results agree with `DuckDB`'s
//! SQL semantics exactly, rather than approximately.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::datetime;
//!
//! // Inside a callback, given a DATE read as days-since-epoch:
//! # let days = 0_i32;
//! let date = unsafe { datetime::date_from_days(days) };
//! assert_eq!((date.year, date.month, date.day), (1970, 1, 1));
//!
//! // …and back again. Composing can fail (a month of 13, a 30th of
//! // February), so it returns an `Option`.
//! assert_eq!(unsafe { datetime::date_to_days(date) }, Some(days));
//! ```
//!
//! # Invalid input returns `None`, never aborts
//!
//! Several of `DuckDB`'s conversions throw a C++ exception on bad input — a
//! `DATE` of 2026-13-01, decomposing the `infinity` `TIMESTAMP` — and the C API
//! does not catch it, so it would abort the whole process. The wrappers here
//! check first, mirroring `DuckDB`'s own conditions, and return `None`:
//! [`date_to_days`], [`time_from_micros`], [`time_tz_from_bits`],
//! [`timestamp_from_micros`], [`timestamp_to_micros`], [`time_tz_bits`] and
//! [`decimal_to_f64`]. [`is_valid_date`] exposes the date check on its own.
//! Two of those (`time_from_micros`, `time_tz_from_bits`) guard a debug
//! assertion rather than an exception: a release build of `DuckDB` returns
//! nonsense fields there, and a build with assertions aborts.
//!
//! # Infinity
//!
//! `DuckDB` reserves two values of `DATE` and of `TIMESTAMP` for `infinity` and
//! `-infinity`. Decomposing one of those into a calendar date is meaningless, so
//! check with [`is_finite_date`] / [`is_finite_timestamp`] first, or compare
//! against the constants below.
//!
//! Note the exact values: negative infinity is `-i32::MAX` / `-i64::MAX`, **not**
//! `i32::MIN` / `i64::MIN`. `i32::MIN` is an ordinary (if absurd) finite date.

use libduckdb_sys::{
    duckdb_date, duckdb_date_struct, duckdb_decimal, duckdb_decimal_to_double,
    duckdb_double_to_decimal, duckdb_double_to_hugeint, duckdb_double_to_uhugeint,
    duckdb_from_date, duckdb_from_time, duckdb_from_time_tz, duckdb_from_timestamp, duckdb_hugeint,
    duckdb_hugeint_to_double, duckdb_is_finite_date, duckdb_is_finite_timestamp,
    duckdb_is_finite_timestamp_ms, duckdb_is_finite_timestamp_ns, duckdb_is_finite_timestamp_s,
    duckdb_time, duckdb_time_struct, duckdb_time_tz, duckdb_timestamp, duckdb_timestamp_ms,
    duckdb_timestamp_ns, duckdb_timestamp_s, duckdb_timestamp_struct, duckdb_to_date,
    duckdb_to_time, duckdb_to_timestamp, duckdb_uhugeint, duckdb_uhugeint_to_double,
};

mod checks;
#[cfg(test)]
mod tests;

pub use checks::{is_valid_date, DECIMAL_MAX_WIDTH, MICROS_PER_DAY, TIME_TZ_MAX_OFFSET_SECONDS};

/// A calendar date, as `DuckDB` decomposes a `DATE`.
///
/// `month` is 1–12 and `day` is 1–31; `year` may be negative (BCE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Proleptic Gregorian year. Negative values are BCE.
    pub year: i32,
    /// Month of year, 1–12.
    pub month: i8,
    /// Day of month, 1–31.
    pub day: i8,
}

/// A wall-clock time, as `DuckDB` decomposes a `TIME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time {
    /// Hour of day, 0–23.
    pub hour: i8,
    /// Minute of hour, 0–59.
    pub min: i8,
    /// Second of minute, 0–59.
    pub sec: i8,
    /// Microseconds within the second, 0–999999.
    pub micros: i32,
}

/// A `TIME WITH TIME ZONE`, decomposed into wall-clock time plus UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeTz {
    /// The wall-clock time.
    pub time: Time,
    /// Offset from UTC in seconds.
    pub offset_seconds: i32,
}

/// A `TIMESTAMP`, decomposed into date and time parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// The calendar date.
    pub date: Date,
    /// The wall-clock time.
    pub time: Time,
}

impl From<duckdb_date_struct> for Date {
    fn from(value: duckdb_date_struct) -> Self {
        Self {
            year: value.year,
            month: value.month,
            day: value.day,
        }
    }
}

impl From<Date> for duckdb_date_struct {
    fn from(value: Date) -> Self {
        Self {
            year: value.year,
            month: value.month,
            day: value.day,
        }
    }
}

impl From<duckdb_time_struct> for Time {
    fn from(value: duckdb_time_struct) -> Self {
        Self {
            hour: value.hour,
            min: value.min,
            sec: value.sec,
            micros: value.micros,
        }
    }
}

impl From<Time> for duckdb_time_struct {
    fn from(value: Time) -> Self {
        Self {
            hour: value.hour,
            min: value.min,
            sec: value.sec,
            micros: value.micros,
        }
    }
}

/// The `DATE` value `DuckDB` uses for `infinity`, in days since 1970-01-01.
///
/// Matches `duckdb::date_t::infinity()`.
pub const DATE_INFINITY_DAYS: i32 = i32::MAX;

/// The `DATE` value `DuckDB` uses for `-infinity`, in days since 1970-01-01.
///
/// Matches `duckdb::date_t::ninfinity()`, which is `-i32::MAX` — one greater
/// than `i32::MIN`, so `i32::MIN` itself is a finite date.
pub const DATE_NEGATIVE_INFINITY_DAYS: i32 = -i32::MAX;

/// The `TIMESTAMP` value `DuckDB` uses for `infinity`, in microseconds since the
/// epoch.
///
/// Matches `duckdb::timestamp_t::infinity()`.
pub const TIMESTAMP_INFINITY_MICROS: i64 = i64::MAX;

/// The `TIMESTAMP` value `DuckDB` uses for `-infinity`, in microseconds since
/// the epoch.
///
/// Matches `duckdb::timestamp_t::ninfinity()`, which is `-i64::MAX`.
pub const TIMESTAMP_NEGATIVE_INFINITY_MICROS: i64 = -i64::MAX;

// ─── DATE ────────────────────────────────────────────────────────────────────

/// Decomposes a `DATE` (days since 1970-01-01) into a calendar date.
///
/// Check [`is_finite_date`] first: `DuckDB` reserves extreme values for
/// `infinity` / `-infinity`, which have no calendar representation.
/// `duckdb_from_date` does not check for them — it cannot fail — so a
/// sentinel decomposes into a date that looks real: `infinity` becomes
/// 5881580-07-11, one day past the largest date `DuckDB` accepts.
///
/// # Safety
///
/// The `DuckDB` C API dispatch table must be initialised — it always is inside a
/// callback or a registration closure.
#[must_use]
pub unsafe fn date_from_days(days: i32) -> Date {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_from_date(duckdb_date { days }) }.into()
}

/// Composes a calendar date into a `DATE` (days since 1970-01-01).
///
/// Returns `None` if `DuckDB` cannot represent the date — a month outside
/// 1–12, a day the month does not have, or a year beyond `DuckDB`'s range; see
/// [`is_valid_date`]. `DuckDB` throws on such a date, which would abort the
/// process, so it is checked here first.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn date_to_days(date: Date) -> Option<i32> {
    if !is_valid_date(date) {
        return None;
    }
    // SAFETY: forwarded from this function's own contract; the date passed
    // `Date::IsValid`, so `Date::FromDate` does not throw.
    Some(unsafe { duckdb_to_date(date.into()) }.days)
}

/// Returns `false` for `DuckDB`'s `infinity` / `-infinity` `DATE` sentinels.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn is_finite_date(days: i32) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_is_finite_date(duckdb_date { days }) }
}

// ─── TIME ────────────────────────────────────────────────────────────────────

/// Decomposes a `TIME` (microseconds since midnight) into a wall-clock time.
///
/// Returns `None` outside `DuckDB`'s `TIME` range, 0 to [`MICROS_PER_DAY`]
/// (`00:00:00`–`24:00:00`). `duckdb_from_time` does not check the range: a
/// release build of `DuckDB` decomposes such a value into out-of-range fields
/// (`-1` gives `micros == -1`), and a build with assertions enabled aborts the
/// process, so it is checked here first.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_from_micros(micros: i64) -> Option<Time> {
    if !checks::time_decomposes(micros) {
        return None;
    }
    // SAFETY: forwarded from this function's own contract; `micros` is in
    // `Time::IsValidTime`'s range, so `Time::Convert`'s assertion holds.
    Some(unsafe { duckdb_from_time(duckdb_time { micros }) }.into())
}

/// Composes a wall-clock time into a `TIME` (microseconds since midnight).
///
/// Like `DuckDB`, this does not range-check the fields: it computes
/// `((hour * 60 + min) * 60 + sec) * 1_000_000 + micros`, so an out-of-range
/// field gives an out-of-range `TIME` rather than an error. It cannot throw.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_to_micros(time: Time) -> i64 {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_to_time(time.into()) }.micros
}

/// Packs a wall-clock time and UTC offset into `DuckDB`'s `TIME WITH TIME ZONE`
/// bit representation.
///
/// `offset_seconds` is the offset from UTC in seconds.
///
/// Returns `None` unless `micros_since_midnight` is between 0 and
/// [`MICROS_PER_DAY`] inclusive (`00:00:00`–`24:00:00`) and `offset_seconds` within
/// ±[`TIME_TZ_MAX_OFFSET_SECONDS`] (±15:59:59). `DuckDB` packs the two fields
/// without checking either, so a value outside those ranges would silently
/// corrupt the other field.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_tz_bits(micros_since_midnight: i64, offset_seconds: i32) -> Option<u64> {
    if !checks::time_tz_encodable(micros_since_midnight, offset_seconds) {
        return None;
    }
    // SAFETY: forwarded from this function's own contract.
    Some(
        unsafe { libduckdb_sys::duckdb_create_time_tz(micros_since_midnight, offset_seconds) }.bits,
    )
}

/// Unpacks `DuckDB`'s `TIME WITH TIME ZONE` bit representation.
///
/// Returns `None` unless `bits` is an encoding [`time_tz_bits`] can produce:
/// a time of day between `00:00:00` and `24:00:00` and an offset within
/// ±[`TIME_TZ_MAX_OFFSET_SECONDS`]. The 40-bit time field can hold about 12.7
/// days; `duckdb_from_time_tz` decomposes it with `Time::Convert`, whose
/// assertion aborts a `DuckDB` built with assertions on anything past a day
/// (see [`time_from_micros`]).
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_tz_from_bits(bits: u64) -> Option<TimeTz> {
    // `dtime_tz_t`: the time in the high 40 bits, `MAX_OFFSET - offset` in
    // the low 24; both fit their targets without loss.
    #[allow(clippy::cast_possible_wrap, reason = "40 bits fit an i64")]
    let micros = (bits >> 24) as i64;
    #[allow(clippy::cast_possible_truncation, reason = "24 bits fit an i32")]
    let offset_seconds = TIME_TZ_MAX_OFFSET_SECONDS - (bits & 0x00FF_FFFF) as i32;
    if !checks::time_tz_encodable(micros, offset_seconds) {
        return None;
    }
    // SAFETY: forwarded from this function's own contract; the time field is
    // in `Time::IsValidTime`'s range, so `Time::Convert`'s assertion holds.
    let raw = unsafe { duckdb_from_time_tz(duckdb_time_tz { bits }) };
    Some(TimeTz {
        time: raw.time.into(),
        offset_seconds: raw.offset,
    })
}

// ─── TIMESTAMP ───────────────────────────────────────────────────────────────

/// Decomposes a `TIMESTAMP` (microseconds since the epoch) into date and time.
///
/// Returns `None` for the `infinity` / `-infinity` sentinels (see
/// [`is_finite_timestamp`]) and for the finite values below
/// day `-106_751_991` (that many times [`MICROS_PER_DAY`]; the first ~4 hours of the `i64` range,
/// including `i64::MIN`), whose day overflows when `DuckDB` multiplies it back
/// into microseconds. `DuckDB` throws on all of them, which would abort the
/// process, so they are checked here first.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn timestamp_from_micros(micros: i64) -> Option<Timestamp> {
    if !checks::timestamp_decomposes(micros) {
        return None;
    }
    // SAFETY: forwarded from this function's own contract; the check above
    // rules out every input on which `Timestamp::Convert` throws.
    let raw: duckdb_timestamp_struct =
        unsafe { duckdb_from_timestamp(duckdb_timestamp { micros }) };
    Some(Timestamp {
        date: raw.date.into(),
        time: raw.time.into(),
    })
}

/// Composes date and time into a `TIMESTAMP` (microseconds since the epoch).
///
/// Returns `None` if the date is invalid (see [`is_valid_date`]), if the
/// result overflows an `i64`, or if it would equal one of the infinity
/// sentinels — every case in which `DuckDB` throws, which would abort the
/// process. The time fields are not range-checked, exactly as in
/// [`time_to_micros`].
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn timestamp_to_micros(timestamp: Timestamp) -> Option<i64> {
    // SAFETY: forwarded from this function's own contract.
    let days = unsafe { date_to_days(timestamp.date) }?;
    checks::timestamp_from_parts(days, checks::time_micros(timestamp.time))?;
    let raw = duckdb_timestamp_struct {
        date: timestamp.date.into(),
        time: timestamp.time.into(),
    };
    // SAFETY: forwarded from this function's own contract; the checks above
    // mirror every throw in `Date::FromDate` and `Timestamp::FromDatetime`.
    Some(unsafe { duckdb_to_timestamp(raw) }.micros)
}

/// Returns `false` for `DuckDB`'s `infinity` / `-infinity` `TIMESTAMP`
/// sentinels.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn is_finite_timestamp(micros: i64) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_is_finite_timestamp(duckdb_timestamp { micros }) }
}

/// `TIMESTAMP_S` variant of [`is_finite_timestamp`].
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn is_finite_timestamp_s(seconds: i64) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_is_finite_timestamp_s(duckdb_timestamp_s { seconds }) }
}

/// `TIMESTAMP_MS` variant of [`is_finite_timestamp`].
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn is_finite_timestamp_ms(millis: i64) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_is_finite_timestamp_ms(duckdb_timestamp_ms { millis }) }
}

/// `TIMESTAMP_NS` variant of [`is_finite_timestamp`].
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn is_finite_timestamp_ns(nanos: i64) -> bool {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_is_finite_timestamp_ns(duckdb_timestamp_ns { nanos }) }
}

// ─── Wide integers and DECIMAL ───────────────────────────────────────────────

/// Converts a `HUGEINT` to `f64` the way `DuckDB` does.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn hugeint_to_f64(value: i128) -> f64 {
    let raw = duckdb_hugeint {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        lower: value as u64,
        #[allow(clippy::cast_possible_truncation)]
        upper: (value >> 64) as i64,
    };
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_hugeint_to_double(raw) }
}

/// Converts an `f64` to `HUGEINT` the way `DuckDB` does.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn f64_to_hugeint(value: f64) -> i128 {
    // SAFETY: forwarded from this function's own contract.
    let raw = unsafe { duckdb_double_to_hugeint(value) };
    (i128::from(raw.upper) << 64) | i128::from(raw.lower)
}

/// Converts a `UHUGEINT` to `f64` the way `DuckDB` does.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn uhugeint_to_f64(value: u128) -> f64 {
    let raw = duckdb_uhugeint {
        #[allow(clippy::cast_possible_truncation)]
        lower: value as u64,
        #[allow(clippy::cast_possible_truncation)]
        upper: (value >> 64) as u64,
    };
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_uhugeint_to_double(raw) }
}

/// Converts an `f64` to `UHUGEINT` the way `DuckDB` does.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn f64_to_uhugeint(value: f64) -> u128 {
    // SAFETY: forwarded from this function's own contract.
    let raw = unsafe { duckdb_double_to_uhugeint(value) };
    (u128::from(raw.upper) << 64) | u128::from(raw.lower)
}

/// A `DECIMAL` value: an unscaled `i128` plus its declared width and scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal {
    /// Total number of significant digits.
    pub width: u8,
    /// Number of digits after the decimal point.
    pub scale: u8,
    /// The unscaled value: the represented number is `value / 10^scale`.
    pub value: i128,
}

/// Converts an `f64` into a `DECIMAL` of the given width and scale.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn f64_to_decimal(value: f64, width: u8, scale: u8) -> Decimal {
    // SAFETY: forwarded from this function's own contract.
    let raw = unsafe { duckdb_double_to_decimal(value, width, scale) };
    Decimal {
        width: raw.width,
        scale: raw.scale,
        value: (i128::from(raw.value.upper) << 64) | i128::from(raw.value.lower),
    }
}

/// Converts a `DECIMAL` to `f64` the way `DuckDB` does.
///
/// Returns `None` if `width` exceeds [`DECIMAL_MAX_WIDTH`] (38) or `scale`
/// exceeds `width`. `DuckDB` indexes its powers-of-ten tables by `scale`
/// without a bounds check, so such a scale reads past the table (returning
/// `inf`, `NaN` or garbage).
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn decimal_to_f64(decimal: Decimal) -> Option<f64> {
    if !checks::decimal_is_valid(decimal.width, decimal.scale) {
        return None;
    }
    let raw = duckdb_decimal {
        width: decimal.width,
        scale: decimal.scale,
        value: duckdb_hugeint {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            lower: decimal.value as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (decimal.value >> 64) as i64,
        },
    };
    // SAFETY: forwarded from this function's own contract; `scale <= 38`
    // keeps DuckDB's table lookups in bounds.
    Some(unsafe { duckdb_decimal_to_double(raw) })
}
