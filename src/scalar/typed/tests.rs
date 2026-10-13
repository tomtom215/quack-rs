// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Unit tests for the closure-based scalar constructors.

use super::*;

#[test]
fn scalar_value_type_ids_match_their_rust_widths() {
    assert_eq!(<i8 as ScalarValue>::type_id(), TypeId::TinyInt);
    assert_eq!(<u64 as ScalarValue>::type_id(), TypeId::UBigInt);
    assert_eq!(<f32 as ScalarValue>::type_id(), TypeId::Float);
    assert_eq!(<i128 as ScalarValue>::type_id(), TypeId::HugeInt);
    assert_eq!(<bool as ScalarValue>::type_id(), TypeId::Boolean);
    assert_eq!(<String as ScalarOut>::type_id(), TypeId::Varchar);
    assert_eq!(<Vec<u8> as ScalarOut>::type_id(), TypeId::Blob);
}

/// Building a typed function touches no `DuckDB` API: the only `DuckDB`
/// work is in `register`. Before the third audit `from_exec` built a
/// `LogicalType` per slot to check it, so every constructor panicked in a
/// unit test (no dispatch table) and built handles nobody needed.
#[test]
fn every_typed_constructor_builds_without_a_live_duckdb() {
    let built = [
        ScalarFunctionBuilder::map1("m1", |x: i64| x * 2),
        ScalarFunctionBuilder::map2("m2", |a: i32, b: f64| f64::from(a) + b),
        ScalarFunctionBuilder::map1_opt("m1o", |x: Option<u8>| x.map(u16::from)),
        ScalarFunctionBuilder::map2_opt("m2o", |a: Option<i128>, b: Option<bool>| {
            a.filter(|_| b.unwrap_or(false))
        }),
        ScalarFunctionBuilder::map1_str("m1s", |s: &str| s.to_uppercase()),
        ScalarFunctionBuilder::map2_str("m2s", |a: &str, b: &str| [a, b].concat().into_bytes()),
    ];
    let names: Vec<_> = built
        .into_iter()
        .map(|b| {
            b.expect("builds with no DuckDB")
                .volatile()
                .name()
                .to_owned()
        })
        .collect();
    assert_eq!(names, ["m1", "m2", "m1o", "m2o", "m1s", "m2s"]);
}

#[test]
fn an_invalid_name_is_rejected_before_anything_is_allocated() {
    let err = ScalarFunctionBuilder::map1("has spaces", |x: i64| x)
        .expect_err("a name with a space is not a SQL identifier");
    assert_ne!(err.as_str(), "");
}

#[test]
fn an_option_result_has_the_column_type_of_its_payload() {
    assert_eq!(<Option<i64> as ScalarOut>::type_id(), TypeId::BigInt);
    assert_eq!(<Option<bool> as ScalarOut>::type_id(), TypeId::Boolean);
    assert_eq!(<Option<String> as ScalarOut>::type_id(), TypeId::Varchar);
    assert_eq!(<Option<Vec<u8>> as ScalarOut>::type_id(), TypeId::Blob);
    assert_eq!(
        <Option<Option<u16>> as ScalarOut>::type_id(),
        TypeId::USmallInt
    );
}

#[test]
fn a_closure_error_is_reported_verbatim() {
    assert_eq!(
        fallible::user_error(&"not an email address").as_str(),
        "not an email address"
    );
    let parse = "x".parse::<i64>().expect_err("not a number");
    assert_eq!(fallible::user_error(&parse).as_str(), parse.to_string());
}

/// The constructors added for NULL results, fallible closures and three
/// arguments build without a live `DuckDB`, like the originals.
#[test]
fn option_and_fallible_constructors_build_without_a_live_duckdb() {
    let built = [
        ScalarFunctionBuilder::map1("o1", |x: i64| (x > 0).then_some(x)),
        ScalarFunctionBuilder::map1_str("o1s", |s: &str| {
            s.split_once('@').map(|(_, d)| d.to_owned())
        }),
        ScalarFunctionBuilder::map2("o2", |a: i32, b: i32| a.checked_div(b)),
        ScalarFunctionBuilder::map3("m3", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi)),
        ScalarFunctionBuilder::try_map1("t1", |x: i64| x.checked_mul(x).ok_or("overflow")),
        ScalarFunctionBuilder::try_map2("t2", |a: i64, b: i64| {
            a.checked_div(b).ok_or_else(|| format!("{a}/0"))
        }),
        ScalarFunctionBuilder::try_map3("t3", |x: i64, lo: i64, hi: i64| {
            if lo > hi {
                Err("empty range")
            } else {
                Ok(Some(x.clamp(lo, hi)))
            }
        }),
        ScalarFunctionBuilder::try_map1_str("t1s", |s: &str| s.parse::<u32>()),
        ScalarFunctionBuilder::try_map2_str("t2s", |a: &str, b: &str| {
            Ok::<_, std::fmt::Error>(a.find(b).map(|i| i as u64))
        }),
    ];
    let names: Vec<_> = built
        .into_iter()
        .map(|b| b.expect("builds with no DuckDB").name().to_owned())
        .collect();
    assert_eq!(
        names,
        ["o1", "o1s", "o2", "m3", "t1", "t2", "t3", "t1s", "t2s"]
    );
}

#[test]
fn a_fallible_constructor_still_validates_the_name() {
    let err = ScalarFunctionBuilder::try_map1("bad name", |x: i64| Ok::<_, String>(x))
        .expect_err("a name with a space is not a SQL identifier");
    assert_ne!(err.as_str(), "");
}
