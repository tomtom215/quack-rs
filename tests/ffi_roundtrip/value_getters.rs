// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Each typed `Value` getter against a live `DuckDB`.
//!
//! The fifth audit's end-to-end mutation run found that a getter returning
//! `None` unconditionally passed every test for eleven of them: none was
//! read at its own type. It also found the guard that stops `DuckDB` from
//! casting an out-of-range `TIMETZ` untested.

use quack_rs::interval::DuckInterval;
use quack_rs::value::Value;

use super::Fixture;

#[test]
fn every_typed_getter_reads_a_value_of_its_own_type() {
    let _fx = Fixture::open();
    assert_eq!(Value::tinyint(-5).as_i8(), Some(-5));
    assert_eq!(Value::smallint(-300).as_i16(), Some(-300));
    assert_eq!(Value::utinyint(250).as_u8(), Some(250));
    assert_eq!(Value::usmallint(65_000).as_u16(), Some(65_000));
    assert_eq!(Value::uinteger(4_000_000_000).as_u32(), Some(4_000_000_000));
    assert_eq!(Value::ubigint(u64::MAX).as_u64(), Some(u64::MAX));
    assert_eq!(Value::hugeint(i128::MIN + 1).as_i128(), Some(i128::MIN + 1));
    assert_eq!(Value::uhugeint(u128::MAX).as_u128(), Some(u128::MAX));
    assert_eq!(Value::float(1.5).as_f32(), Some(1.5));
    assert_eq!(
        Value::timestamp_tz(1_700_000_000_000_000)
            .expect("in range")
            .as_timestamp_tz(),
        Some(1_700_000_000_000_000)
    );
    let interval = DuckInterval {
        months: 14,
        days: -3,
        micros: 5_000_000,
    };
    assert_eq!(Value::interval(interval).as_interval(), Some(interval));
}

/// A `TIMETZ` whose offset field is out of range is not cast: `as_time`
/// reads `None`, where the payload's time part alone (1,000 us) would pass
/// `as_time`'s own range check. A valid `TIMETZ` still casts.
#[test]
fn an_out_of_range_timetz_is_not_cast() {
    let _fx = Fixture::open();
    let valid = Value::time_tz((1000_u64 << 24) | 0x0000_0FFF).expect("in range");
    assert!(valid.as_time().is_some());

    let bits = (1000_u64 << 24) | 0x00FF_FFFF;
    // SAFETY: DuckDB returns an owned value for any payload; `Value` destroys it.
    let bad = unsafe {
        Value::from_raw(libduckdb_sys::duckdb_create_time_tz_value(
            libduckdb_sys::duckdb_time_tz { bits },
        ))
    };
    assert_eq!(bad.as_time(), None);
}

/// A `TIME_NS` one nanosecond past midnight's end is not cast: `DuckDB`'s
/// cast to `TIME` would truncate it to exactly 24:00:00, a `TIME` that
/// `as_time` accepts, so only the source-range check refuses it.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn an_out_of_range_time_ns_is_not_cast() {
    let _fx = Fixture::open();
    const NANOS_PER_DAY: i64 = 86_400_000_000_000;
    let valid = Value::time_ns(1_000).expect("in range");
    assert_eq!(valid.as_time(), Some(1));
    // SAFETY: DuckDB returns an owned value for any payload; `Value` destroys it.
    let bad = unsafe {
        Value::from_raw(libduckdb_sys::duckdb_create_time_ns(
            libduckdb_sys::duckdb_time_ns {
                nanos: NANOS_PER_DAY + 1,
            },
        ))
    };
    assert_eq!(bad.as_time(), None);
}
