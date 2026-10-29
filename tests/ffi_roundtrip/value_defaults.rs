// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The `Value::as_*_or` defaulting getters, on a value that is there.
//!
//! The unit tests in `src/value/defaults.rs` reach only the null-handle
//! branch (no engine is available to them), where a getter that returned its
//! default unconditionally passes. Here each getter reads a real value of its
//! own type, and a real value it cannot convert, and must tell them apart.

use quack_rs::value::Value;

use super::Fixture;

/// Each getter, given a value of its own type, returns that value rather than
/// its default. The defaults are chosen so none can equal the value.
#[test]
fn a_defaulting_getter_returns_the_value_when_there_is_one() {
    let _fx = Fixture::open();
    assert_eq!(Value::tinyint(-100).as_i8_or(7), -100);
    assert_eq!(Value::smallint(-30_000).as_i16_or(7), -30_000);
    assert_eq!(Value::integer(-2_000_000_000).as_i32_or(7), -2_000_000_000);
    assert_eq!(Value::bigint(i64::MIN + 1).as_i64_or(7), i64::MIN + 1);
    assert_eq!(Value::hugeint(i128::MIN + 1).as_i128_or(7), i128::MIN + 1);
    assert_eq!(Value::utinyint(250).as_u8_or(7), 250);
    assert_eq!(Value::usmallint(65_000).as_u16_or(7), 65_000);
    assert_eq!(Value::uinteger(4_000_000_000).as_u32_or(7), 4_000_000_000);
    assert_eq!(Value::ubigint(u64::MAX).as_u64_or(7), u64::MAX);
    assert!((Value::float(-2.5).as_f32_or(7.0) - -2.5).abs() < f32::EPSILON);
    assert!((Value::double(1e300).as_f64_or(7.0) - 1e300).abs() < f64::EPSILON * 1e300);
    assert!(Value::boolean(true).as_bool_or(false));
    assert!(!Value::boolean(false).as_bool_or(true));
    assert_eq!(Value::varchar("hello").as_str_or("dflt"), "hello");
    assert_eq!(Value::varchar("hello").as_str_or_default(), "hello");
    assert_eq!(
        Value::varchar("").as_str_or("dflt"),
        "",
        "an empty string is a value"
    );
}

/// A SQL `NULL` and a value that cannot be converted both give the default.
#[test]
fn a_defaulting_getter_returns_its_default_for_null_and_for_an_impossible_cast() {
    let _fx = Fixture::open();
    let null = Value::null_value();
    assert_eq!(null.as_i8_or(7), 7);
    assert_eq!(null.as_i16_or(7), 7);
    assert_eq!(null.as_i32_or(7), 7);
    assert_eq!(null.as_i64_or(7), 7);
    assert_eq!(null.as_i128_or(7), 7);
    assert_eq!(null.as_u8_or(7), 7);
    assert_eq!(null.as_u16_or(7), 7);
    assert_eq!(null.as_u32_or(7), 7);
    assert_eq!(null.as_u64_or(7), 7);
    assert!((null.as_f32_or(7.5) - 7.5).abs() < f32::EPSILON);
    assert!((null.as_f64_or(7.5) - 7.5).abs() < f64::EPSILON);
    assert!(null.as_bool_or(true));
    assert!(!null.as_bool_or(false));
    assert_eq!(null.as_str_or("dflt"), "dflt");
    assert_eq!(null.as_str_or_default(), "");

    // Text that is not a number, read as one.
    let word = Value::varchar("not a number");
    assert_eq!(word.as_i8_or(7), 7);
    assert_eq!(word.as_i16_or(7), 7);
    assert_eq!(word.as_i32_or(7), 7);
    assert_eq!(word.as_i64_or(7), 7);
    assert_eq!(word.as_u8_or(7), 7);
    assert_eq!(word.as_u16_or(7), 7);
    assert_eq!(word.as_u32_or(7), 7);
    assert_eq!(word.as_u64_or(7), 7);
    assert!((word.as_f32_or(7.5) - 7.5).abs() < f32::EPSILON);
    assert!((word.as_f64_or(7.5) - 7.5).abs() < f64::EPSILON);

    // A number too large for the width asked for.
    let big = Value::bigint(5_000_000_000);
    assert_eq!(big.as_i32_or(7), 7);
    assert_eq!(big.as_i16_or(7), 7);
    assert_eq!(big.as_i8_or(7), 7);
    assert_eq!(big.as_u32_or(7), 7);
    assert_eq!(big.as_u16_or(7), 7);
    assert_eq!(big.as_u8_or(7), 7);
    // ...and a negative one for an unsigned width.
    assert_eq!(Value::bigint(-1).as_u64_or(7), 7);
    assert_eq!(Value::bigint(-1).as_u8_or(7), 7);
}

/// Widening and narrowing casts that do fit go through.
#[test]
fn a_defaulting_getter_converts_a_value_of_another_type_that_fits() {
    let _fx = Fixture::open();
    assert_eq!(Value::bigint(100).as_i8_or(7), 100);
    assert_eq!(Value::bigint(30_000).as_i16_or(7), 30_000);
    assert_eq!(Value::bigint(2_000_000_000).as_i32_or(7), 2_000_000_000);
    assert_eq!(Value::tinyint(-5).as_i64_or(7), -5);
    assert_eq!(Value::bigint(200).as_u8_or(7), 200);
    assert_eq!(Value::bigint(60_000).as_u16_or(7), 60_000);
    assert_eq!(Value::bigint(4_000_000_000).as_u32_or(7), 4_000_000_000);
    assert_eq!(Value::integer(5).as_u64_or(7), 5);
    assert!((Value::integer(3).as_f64_or(7.5) - 3.0).abs() < f64::EPSILON);
    assert!((Value::integer(3).as_f32_or(7.5) - 3.0).abs() < f32::EPSILON);
    assert_eq!(Value::varchar("42").as_i32_or(7), 42);
    assert_eq!(Value::integer(42).as_str_or("dflt"), "42");
}
