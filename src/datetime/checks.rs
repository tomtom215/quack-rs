// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Pure-Rust mirrors of the checks `DuckDB` makes before it throws.
//!
//! Several of the C API conversions call `DuckDB` routines that throw a C++
//! `ConversionException` on bad input, with no `try`/`catch` in the C API
//! wrapper. A C++ exception that reaches Rust aborts the process, so the
//! wrappers in [`super`] run these checks first and return `None` instead.
//! Each one mirrors the `DuckDB` 1.5 source it cites exactly, so a value that
//! passes is one `DuckDB` accepts.

use super::{Date, Time, TIMESTAMP_INFINITY_MICROS, TIMESTAMP_NEGATIVE_INFINITY_MICROS};

/// Microseconds in a day (`Interval::MICROS_PER_DAY`).
pub const MICROS_PER_DAY: i64 = 86_400_000_000;

/// The largest `TIME WITH TIME ZONE` offset `DuckDB` can encode, in seconds:
/// `+15:59:59` (`dtime_tz_t::MAX_OFFSET = 16 * 60 * 60 - 1`). The smallest is
/// its negation.
pub const TIME_TZ_MAX_OFFSET_SECONDS: i32 = 16 * 60 * 60 - 1;

/// The widest `DECIMAL` `DuckDB` supports (`Decimal::MAX_WIDTH_INT128`).
pub const DECIMAL_MAX_WIDTH: u8 = 38;

// `Date::DATE_MIN_*` / `DATE_MAX_*` in `duckdb/common/types/date.hpp`: the
// earliest date is 5877642-06-25 BC (year -5877641) and the latest is
// 5881580-07-10, days -(2^31 - 2) and 2^31 - 2.
const DATE_MIN: (i32, i32, i32) = (-5_877_641, 6, 25);
const DATE_MAX: (i32, i32, i32) = (5_881_580, 7, 10);

/// `Date::NORMAL_DAYS` / `Date::LEAP_DAYS`, indexed by month (1–12).
const NORMAL_DAYS: [i32; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const LEAP_DAYS: [i32; 13] = [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `Date::IsLeapYear`: the proleptic Gregorian rule.
const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Returns `true` if `DuckDB` can represent `date` as a `DATE`.
///
/// This is `DuckDB`'s own `Date::IsValid`: the month is 1–12, the day exists in
/// that month of that (proleptic Gregorian) year, and the date lies between
/// 5877642-06-25 BC and 5881580-07-10. [`date_to_days`][super::date_to_days]
/// returns `None` exactly when this returns `false`.
#[must_use]
pub const fn is_valid_date(date: Date) -> bool {
    let (year, month, day) = (date.year, date.month as i32, date.day as i32);
    if month < 1 || month > 12 || day < 1 {
        return false;
    }
    if year < DATE_MIN.0
        || (year == DATE_MIN.0 && (month < DATE_MIN.1 || (month == DATE_MIN.1 && day < DATE_MIN.2)))
    {
        return false;
    }
    if year > DATE_MAX.0
        || (year == DATE_MAX.0 && (month > DATE_MAX.1 || (month == DATE_MAX.1 && day > DATE_MAX.2)))
    {
        return false;
    }
    #[allow(clippy::cast_sign_loss, reason = "month is 1..=12 here")]
    let month = month as usize;
    if is_leap_year(year) {
        day <= LEAP_DAYS[month]
    } else {
        day <= NORMAL_DAYS[month]
    }
}

/// `Time::FromTime`: the time of day in microseconds, with no range check.
///
/// `DuckDB` accepts any field values here (it never throws), so this does too;
/// with `i8` / `i32` fields the arithmetic cannot overflow an `i64`.
pub(super) const fn time_micros(time: Time) -> i64 {
    let mut micros = time.hour as i64;
    micros = micros * 60 + time.min as i64;
    micros = micros * 60 + time.sec as i64;
    micros * 1_000_000 + time.micros as i64
}

/// Returns `true` if `Time::Convert` accepts `micros`: `DuckDB`'s `TIME` range,
/// `00:00:00`–`24:00:00` inclusive.
///
/// `Time::Convert` does no range check, but it ends in
/// `D_ASSERT(Time::IsValidTime(..))`, which a `DuckDB` built with assertions
/// (a debug build) turns into a process abort for every other value.
pub(super) const fn time_decomposes(micros: i64) -> bool {
    micros >= 0 && micros <= MICROS_PER_DAY
}

/// The day `Timestamp::GetDate` assigns a finite timestamp to (floor division).
const fn timestamp_days(micros: i64) -> i64 {
    let negative = (micros < 0) as i64;
    (micros + negative) / MICROS_PER_DAY - negative
}

/// Returns `true` if `Timestamp::Convert` decomposes `micros` without throwing.
///
/// It throws for the two infinity sentinels (whose day, `±i32::MAX`, times
/// [`MICROS_PER_DAY`] overflows) and for every finite value below
/// `-106_751_991 * MICROS_PER_DAY` — the first ~4 hours of the `i64` range,
/// down to `i64::MIN` — whose floor-divided day likewise overflows when
/// multiplied back.
pub(super) const fn timestamp_decomposes(micros: i64) -> bool {
    micros != TIMESTAMP_INFINITY_MICROS
        && micros != TIMESTAMP_NEGATIVE_INFINITY_MICROS
        && timestamp_days(micros).checked_mul(MICROS_PER_DAY).is_some()
}

/// `Timestamp::TryFromDatetime`: `days * MICROS_PER_DAY + time_micros`,
/// or `None` where `DuckDB`'s `FromDatetime` would throw — on overflow, or when
/// the result is one of the infinity sentinels.
pub(super) const fn timestamp_from_parts(days: i32, time_micros: i64) -> Option<i64> {
    let Some(day_micros) = (days as i64).checked_mul(MICROS_PER_DAY) else {
        return None;
    };
    let Some(micros) = day_micros.checked_add(time_micros) else {
        return None;
    };
    if micros == TIMESTAMP_INFINITY_MICROS || micros == TIMESTAMP_NEGATIVE_INFINITY_MICROS {
        return None;
    }
    Some(micros)
}

/// Returns `true` if `dtime_tz_t` can encode this time and offset.
///
/// `duckdb_create_time_tz` never throws, but it packs the time into the high
/// 40 bits and `MAX_OFFSET - offset` into the low 24 with no checks: an
/// offset beyond ±[`TIME_TZ_MAX_OFFSET_SECONDS`] or a negative time corrupts
/// the other field. `DuckDB`'s own `TIME` range is `00:00:00`–`24:00:00`.
pub(super) const fn time_tz_encodable(micros: i64, offset_seconds: i32) -> bool {
    time_decomposes(micros)
        && offset_seconds >= -TIME_TZ_MAX_OFFSET_SECONDS
        && offset_seconds <= TIME_TZ_MAX_OFFSET_SECONDS
}

/// Returns `true` if `duckdb_decimal_to_double` can convert this width and
/// scale: it indexes `NumericHelper::DOUBLE_POWERS_OF_TEN` (40 entries) and
/// `Hugeint::POWERS_OF_TEN` (39) by `scale` with no bounds check. This is the
/// same rule `duckdb_double_to_decimal` enforces in the other direction.
pub(super) const fn decimal_is_valid(width: u8, scale: u8) -> bool {
    width <= DECIMAL_MAX_WIDTH && scale <= width
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn date(year: i32, month: i8, day: i8) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn months_and_days_out_of_range_are_invalid() {
        assert!(is_valid_date(date(2026, 1, 31)));
        assert!(!is_valid_date(date(2026, 13, 1)));
        assert!(!is_valid_date(date(2026, 0, 1)));
        assert!(!is_valid_date(date(2026, -1, 1)));
        assert!(!is_valid_date(date(2026, 4, 31)));
        assert!(!is_valid_date(date(2026, 1, 0)));
        assert!(!is_valid_date(date(2026, 1, 32)));
        assert!(!is_valid_date(date(2026, 1, -5)));
        // The inclusive ends of each field range: December and the 1st exist.
        assert!(is_valid_date(date(2026, 12, 1)));
        assert!(is_valid_date(date(2026, 12, 31)));
        assert!(is_valid_date(date(2026, 1, 1)));
    }

    #[test]
    fn february_follows_the_gregorian_leap_rule() {
        assert!(is_valid_date(date(2024, 2, 29)));
        assert!(!is_valid_date(date(2026, 2, 29)));
        assert!(is_valid_date(date(2000, 2, 29)));
        assert!(!is_valid_date(date(1900, 2, 29)));
        // Proleptic and BCE years follow the same rule.
        assert!(is_valid_date(date(-400, 2, 29)));
        assert!(!is_valid_date(date(-100, 2, 29)));
    }

    #[test]
    fn the_representable_range_ends_exactly_where_duckdb_says() {
        assert!(is_valid_date(date(-5_877_641, 6, 25)));
        assert!(!is_valid_date(date(-5_877_641, 6, 24)));
        assert!(!is_valid_date(date(-5_877_641, 5, 30)));
        assert!(!is_valid_date(date(-5_877_642, 12, 31)));
        assert!(!is_valid_date(date(i32::MIN, 1, 1)));
        assert!(is_valid_date(date(5_881_580, 7, 10)));
        assert!(!is_valid_date(date(5_881_580, 7, 11)));
        assert!(!is_valid_date(date(5_881_580, 8, 1)));
        assert!(!is_valid_date(date(5_881_581, 1, 1)));
        assert!(!is_valid_date(date(i32::MAX, 1, 1)));
    }

    #[test]
    fn timestamp_days_floors_toward_negative_infinity() {
        assert_eq!(timestamp_days(0), 0);
        assert_eq!(timestamp_days(1), 0);
        assert_eq!(timestamp_days(MICROS_PER_DAY - 1), 0);
        assert_eq!(timestamp_days(MICROS_PER_DAY), 1);
        assert_eq!(timestamp_days(-1), -1);
        assert_eq!(timestamp_days(-MICROS_PER_DAY), -1);
        assert_eq!(timestamp_days(-MICROS_PER_DAY - 1), -2);
    }

    #[test]
    fn timestamps_that_duckdb_cannot_decompose_are_caught() {
        assert!(timestamp_decomposes(0));
        assert!(timestamp_decomposes(-1));
        assert!(timestamp_decomposes(i64::MAX - 1));
        // The earliest instant whose day survives the multiply-back; one
        // microsecond earlier (and everything down to `i64::MIN`, ~4 hours of
        // the range) floors to a day whose micros overflow.
        assert!(timestamp_decomposes(-106_751_991 * MICROS_PER_DAY));
        assert!(!timestamp_decomposes(-106_751_991 * MICROS_PER_DAY - 1));
        assert!(!timestamp_decomposes(-i64::MAX + 1));
        assert!(!timestamp_decomposes(TIMESTAMP_INFINITY_MICROS));
        assert!(!timestamp_decomposes(TIMESTAMP_NEGATIVE_INFINITY_MICROS));
        // Finite, but its floor-divided day overflows when multiplied back.
        assert!(!timestamp_decomposes(i64::MIN));
    }

    #[test]
    fn timestamp_composition_rejects_overflow_and_the_sentinels() {
        assert_eq!(timestamp_from_parts(1, 5), Some(MICROS_PER_DAY + 5));
        assert_eq!(timestamp_from_parts(-1, 0), Some(-MICROS_PER_DAY));
        assert_eq!(timestamp_from_parts(i32::MAX, 0), None);
        assert_eq!(timestamp_from_parts(i32::MIN, 0), None);
        // The last day that fits, pushed over the edge by its time.
        let last_day = i32::try_from(i64::MAX / MICROS_PER_DAY).unwrap();
        assert!(timestamp_from_parts(last_day, 0).is_some());
        assert_eq!(
            timestamp_from_parts(last_day, i64::MAX % MICROS_PER_DAY + 1),
            None
        );
        // Landing exactly on infinity is refused too.
        assert_eq!(
            timestamp_from_parts(last_day, i64::MAX % MICROS_PER_DAY),
            None
        );
    }

    #[test]
    fn time_micros_matches_duckdb_from_time() {
        let t = Time {
            hour: 23,
            min: 59,
            sec: 59,
            micros: 999_999,
        };
        assert_eq!(time_micros(t), MICROS_PER_DAY - 1);
    }

    /// `Time::Convert` followed by `Time::IsValidTime`, transcribed from
    /// `DuckDB` v1.5.5 (`src/common/types/time.cpp`). C++ integer division
    /// truncates toward zero, as Rust's does.
    fn duckdb_convert_is_valid(micros: i64) -> bool {
        let mut time = micros;
        let hour = time / 3_600_000_000;
        time -= hour * 3_600_000_000;
        let min = time / 60_000_000;
        time -= min * 60_000_000;
        let sec = time / 1_000_000;
        time -= sec * 1_000_000;
        let micros = time;
        if !(0..24).contains(&hour) {
            return hour == 24 && min == 0 && sec == 0 && micros == 0;
        }
        (0..60).contains(&min) && (0..=60).contains(&sec) && (0..=1_000_000).contains(&micros)
    }

    #[test]
    fn time_decomposes_is_exactly_what_duckdb_asserts() {
        let mut probes = vec![i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1];
        for edge in [0, MICROS_PER_DAY, 3_600_000_000, 60_000_000, 1_000_000] {
            for delta in -2..=2 {
                probes.push(edge + delta);
                probes.push(-edge + delta);
            }
        }
        // Every hour boundary of the day and the one after it.
        for hour in -1..=26 {
            probes.push(hour * 3_600_000_000);
            probes.push(hour * 3_600_000_000 - 1);
        }
        for micros in probes {
            assert_eq!(
                time_decomposes(micros),
                duckdb_convert_is_valid(micros),
                "{micros}"
            );
        }
        assert!(time_decomposes(0));
        assert!(time_decomposes(MICROS_PER_DAY));
        assert!(!time_decomposes(-1));
        assert!(!time_decomposes(MICROS_PER_DAY + 1));
    }

    #[test]
    fn time_tz_max_offset_is_duckdbs_plus_15_59_59() {
        // `dtime_tz_t::MAX_OFFSET` in DuckDB 1.5: 15h 59m 59s, in seconds.
        assert_eq!(TIME_TZ_MAX_OFFSET_SECONDS, 57_599);
        assert_eq!(TIME_TZ_MAX_OFFSET_SECONDS, 15 * 3_600 + 59 * 60 + 59);
    }

    #[test]
    fn time_tz_bounds() {
        assert!(time_tz_encodable(0, 0));
        assert!(time_tz_encodable(
            MICROS_PER_DAY,
            TIME_TZ_MAX_OFFSET_SECONDS
        ));
        assert!(time_tz_encodable(0, -TIME_TZ_MAX_OFFSET_SECONDS));
        assert!(!time_tz_encodable(0, TIME_TZ_MAX_OFFSET_SECONDS + 1));
        assert!(!time_tz_encodable(0, -TIME_TZ_MAX_OFFSET_SECONDS - 1));
        assert!(!time_tz_encodable(-1, 0));
        assert!(!time_tz_encodable(MICROS_PER_DAY + 1, 0));
    }

    #[test]
    fn decimal_bounds() {
        assert!(decimal_is_valid(38, 38));
        assert!(decimal_is_valid(18, 3));
        assert!(!decimal_is_valid(39, 0));
        assert!(!decimal_is_valid(10, 11));
        assert!(!decimal_is_valid(38, 40));
    }
}
