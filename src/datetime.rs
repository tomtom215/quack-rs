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
//! // …and back again.
//! assert_eq!(unsafe { datetime::date_to_days(date) }, days);
//! ```
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
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn date_to_days(date: Date) -> i32 {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_to_date(date.into()) }.days
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
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_from_micros(micros: i64) -> Time {
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_from_time(duckdb_time { micros }) }.into()
}

/// Composes a wall-clock time into a `TIME` (microseconds since midnight).
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
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_tz_bits(micros_since_midnight: i64, offset_seconds: i32) -> u64 {
    // SAFETY: forwarded from this function's own contract.
    unsafe { libduckdb_sys::duckdb_create_time_tz(micros_since_midnight, offset_seconds) }.bits
}

/// Unpacks `DuckDB`'s `TIME WITH TIME ZONE` bit representation.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn time_tz_from_bits(bits: u64) -> TimeTz {
    // SAFETY: forwarded from this function's own contract.
    let raw = unsafe { duckdb_from_time_tz(duckdb_time_tz { bits }) };
    TimeTz {
        time: raw.time.into(),
        offset_seconds: raw.offset,
    }
}

// ─── TIMESTAMP ───────────────────────────────────────────────────────────────

/// Decomposes a `TIMESTAMP` (microseconds since the epoch) into date and time.
///
/// Check [`is_finite_timestamp`] first.
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn timestamp_from_micros(micros: i64) -> Timestamp {
    // SAFETY: forwarded from this function's own contract.
    let raw: duckdb_timestamp_struct =
        unsafe { duckdb_from_timestamp(duckdb_timestamp { micros }) };
    Timestamp {
        date: raw.date.into(),
        time: raw.time.into(),
    }
}

/// Composes date and time into a `TIMESTAMP` (microseconds since the epoch).
///
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn timestamp_to_micros(timestamp: Timestamp) -> i64 {
    let raw = duckdb_timestamp_struct {
        date: timestamp.date.into(),
        time: timestamp.time.into(),
    };
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_to_timestamp(raw) }.micros
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
/// # Safety
///
/// See [`date_from_days`].
#[must_use]
pub unsafe fn decimal_to_f64(decimal: Decimal) -> f64 {
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
    // SAFETY: forwarded from this function's own contract.
    unsafe { duckdb_decimal_to_double(raw) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_struct_round_trips_through_ffi_types() {
        let date = Date {
            year: 2026,
            month: 8,
            day: 18,
        };
        let raw: duckdb_date_struct = date.into();
        assert_eq!(raw.year, 2026);
        assert_eq!(raw.month, 8);
        assert_eq!(raw.day, 18);
        assert_eq!(Date::from(raw), date);
    }

    #[test]
    fn time_struct_round_trips_through_ffi_types() {
        let time = Time {
            hour: 23,
            min: 59,
            sec: 58,
            micros: 123_456,
        };
        let raw: duckdb_time_struct = time.into();
        assert_eq!(Time::from(raw), time);
    }

    #[test]
    fn decimal_is_ordered_and_hashable() {
        use std::collections::HashSet;
        let a = Decimal {
            width: 18,
            scale: 3,
            value: 1_500,
        };
        let b = Decimal {
            width: 18,
            scale: 3,
            value: 2_500,
        };
        assert!(a < b);
        let set: HashSet<Decimal> = [a, b, a].into_iter().collect();
        assert_eq!(set.len(), 2);
    }
}

/// Conversions checked against a live `DuckDB`.
#[cfg(all(test, feature = "_duckdb-testing"))]
mod live_tests {
    use super::*;
    use crate::testing::InMemoryDb;

    #[test]
    fn epoch_day_zero_is_1970_01_01() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        let date = unsafe { date_from_days(0) };
        assert_eq!(
            date,
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
        );
    }

    #[test]
    fn date_round_trips_across_leap_years_and_bce() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for days in [
            -1_000_000_i32,
            -719_162, // 0001-01-01
            -1,
            0,
            1,
            59,     // 1970-03-01
            10_957, // 2000-01-01
            11_017, // 2000-03-01, just past a leap day
            20_685, // 2026-08-18
            1_000_000,
        ] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let date = unsafe { date_from_days(days) };
            assert_eq!(unsafe { date_to_days(date) }, days, "round trip for {days}");
        }
    }

    #[test]
    fn duckdb_agrees_with_our_conversion() {
        // Cross-check against DuckDB's SQL layer rather than trusting the C API
        // wrapper in isolation.
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        for days in [0_i32, 20_685, -719_162] {
            // `INTERVAL {n} DAY` will not parse a negative literal, so add the
            // interval as an expression instead.
            let sql =
                format!("SELECT strftime(DATE '1970-01-01' + INTERVAL ({days}) DAY, '%Y-%m-%d')");
            let expected: String = db.query_one(&sql).expect("query");
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let date = unsafe { date_from_days(days) };
            let actual = format!("{:04}-{:02}-{:02}", date.year, date.month, date.day);
            assert_eq!(actual, expected, "for {days} days since the epoch");
        }
    }

    #[test]
    fn infinity_sentinels_match_the_documented_constants() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert!(is_finite_date(0));
            assert!(!is_finite_date(DATE_INFINITY_DAYS));
            assert!(!is_finite_date(DATE_NEGATIVE_INFINITY_DAYS));
            // -infinity is -i32::MAX, so i32::MIN is one step beyond it and is a
            // finite (if nonsensical) date. Getting this backwards would make a
            // caller treat a real date as infinity.
            assert!(is_finite_date(i32::MIN));

            assert!(is_finite_timestamp(0));
            assert!(!is_finite_timestamp(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp(TIMESTAMP_NEGATIVE_INFINITY_MICROS));
            assert!(is_finite_timestamp(i64::MIN));

            assert!(is_finite_timestamp_s(0));
            assert!(is_finite_timestamp_ms(0));
            assert!(is_finite_timestamp_ns(0));
            assert!(!is_finite_timestamp_s(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp_ms(TIMESTAMP_INFINITY_MICROS));
            assert!(!is_finite_timestamp_ns(TIMESTAMP_INFINITY_MICROS));
        }
    }

    #[test]
    fn duckdb_sql_agrees_that_the_sentinels_are_infinite() {
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        let rendered: String = db
            .query_one("SELECT ('infinity'::DATE)::VARCHAR")
            .expect("query");
        assert_eq!(rendered, "infinity");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        assert!(!unsafe { is_finite_date(DATE_INFINITY_DAYS) });
    }

    #[test]
    fn time_round_trips_including_microsecond_precision() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for micros in [0_i64, 1, 999_999, 1_000_000, 86_399_999_999] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let time = unsafe { time_from_micros(micros) };
            assert_eq!(unsafe { time_to_micros(time) }, micros, "for {micros} us");
        }
        // SAFETY: dispatch table initialised above.
        let end_of_day = unsafe { time_from_micros(86_399_999_999) };
        assert_eq!(
            end_of_day,
            Time {
                hour: 23,
                min: 59,
                sec: 59,
                micros: 999_999
            }
        );
    }

    #[test]
    fn timestamp_round_trips() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        for micros in [0_i64, 1, -1, 1_700_000_000_000_000, -1_700_000_000_000_000] {
            // SAFETY: InMemoryDb::open() initialised the dispatch table.
            let ts = unsafe { timestamp_from_micros(micros) };
            assert_eq!(
                unsafe { timestamp_to_micros(ts) },
                micros,
                "for {micros} us"
            );
        }
    }

    #[test]
    fn time_tz_round_trips_with_offset() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            let bits = time_tz_bits(12 * 3_600 * 1_000_000, -5 * 3_600);
            let decoded = time_tz_from_bits(bits);
            assert_eq!(decoded.time.hour, 12);
            assert_eq!(decoded.offset_seconds, -5 * 3_600);
        }
    }

    #[test]
    fn hugeint_conversions_match_duckdb() {
        let db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            assert!((hugeint_to_f64(0) - 0.0).abs() < f64::EPSILON);
            assert!((hugeint_to_f64(1) - 1.0).abs() < f64::EPSILON);
            assert!((hugeint_to_f64(-1) + 1.0).abs() < f64::EPSILON);
            assert_eq!(f64_to_hugeint(42.0), 42);
            assert_eq!(f64_to_hugeint(-42.0), -42);
            assert_eq!(f64_to_uhugeint(42.0), 42);
            assert!((uhugeint_to_f64(u128::from(u64::MAX)) - 1.844_674_407_370_955e19).abs() < 1e6);
        }
        // Cross-check the sign handling of the split representation against SQL.
        let expected: f64 = db
            .query_one("SELECT (-170141183460469231731687303715884105728)::HUGEINT::DOUBLE")
            .expect("query");
        // SAFETY: dispatch table initialised above.
        let actual = unsafe { hugeint_to_f64(i128::MIN) };
        assert!(
            (actual - expected).abs() / expected.abs() < 1e-12,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn decimal_conversions_preserve_width_and_scale() {
        let _db = InMemoryDb::open().expect("open in-memory DuckDB");
        // SAFETY: InMemoryDb::open() initialised the dispatch table.
        unsafe {
            let decimal = f64_to_decimal(12.345, 18, 3);
            assert_eq!(decimal.width, 18);
            assert_eq!(decimal.scale, 3);
            assert_eq!(decimal.value, 12_345);
            assert!((decimal_to_f64(decimal) - 12.345).abs() < 1e-9);

            let negative = f64_to_decimal(-12.345, 18, 3);
            assert_eq!(negative.value, -12_345);
            assert!((decimal_to_f64(negative) + 12.345).abs() < 1e-9);
        }
    }
}
