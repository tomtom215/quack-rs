// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Typed scalar closures that return `Option` (SQL NULL results), the `try_*`
//! constructors (closure errors become query errors), and `map3`, against a
//! live `DuckDB`.

use std::sync::atomic::{AtomicUsize, Ordering};

use quack_rs::scalar::ScalarFunctionBuilder;

use super::Fixture;

/// Runs `sql`, which must fail, and returns the error text.
fn error_of(fx: &Fixture, sql: &str) -> String {
    // SAFETY: the fixture's connection is open.
    let err = unsafe { quack_rs::query::query(fx.con(), sql) }.expect_err(sql);
    format!("{err}")
}

fn count(fx: &Fixture, sql: &str) -> i64 {
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
        .unwrap_or_else(|| panic!("{sql} returned NULL"))
}

fn register(
    fx: &Fixture,
    built: Result<quack_rs::scalar::TypedScalarFunctionBuilder, quack_rs::error::ExtensionError>,
) {
    // SAFETY: the fixture's connection is open.
    unsafe { built.expect("build").register(fx.con()) }.expect("register");
}

/// A table with one NULL in each column, so a NULL can be put in any argument
/// position from a real column rather than a constant-folded literal.
fn null_table(fx: &Fixture) {
    fx.query(
        "CREATE TABLE nulls(a BIGINT, b BIGINT, c BIGINT, s VARCHAR, t VARCHAR);
         INSERT INTO nulls VALUES
             (1, 2, 3, 'x', 'y'),
             (NULL, 2, 3, NULL, 'y'),
             (1, NULL, 3, 'x', NULL),
             (1, 2, NULL, 'x', 'y'),
             (4, 5, 6, 'p', 'q')",
    );
}

#[test]
fn option_results_mix_null_and_values_across_chunks() {
    let fx = Fixture::open();
    register(
        &fx,
        ScalarFunctionBuilder::map1("even_or_null", |x: i64| (x % 2 == 0).then_some(x)),
    );
    register(
        &fx,
        ScalarFunctionBuilder::map1_str("email_domain", |s: &str| {
            s.split_once('@').map(|(_, domain)| domain.to_owned())
        }),
    );

    // 5000 rows is three chunks; 2048 is not a multiple of 2 or 3, so the
    // same vector slot is NULL in one chunk and valid in the next.
    assert_eq!(
        count(
            &fx,
            "SELECT count(*) FROM range(5000) t(i) \
             WHERE (even_or_null(i) IS NULL) <> (i % 2 = 1)"
        ),
        0,
        "every odd row, and only odd rows, must be NULL"
    );
    assert_eq!(
        fx.scalar(
            "SELECT sum(even_or_null(i)) FROM range(5000) t(i)",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some((0..5000_i128).filter(|i| i % 2 == 0).sum())
    );

    let emails = "SELECT i, CASE WHEN i % 3 = 0 THEN 'user' || i || '@host' || i \
                  ELSE 'nobody' || i END AS e FROM range(5000) t(i)";
    assert_eq!(
        count(
            &fx,
            &format!(
                "SELECT count(*) FROM ({emails}) \
                 WHERE email_domain(e) IS DISTINCT FROM \
                       CASE WHEN i % 3 = 0 THEN 'host' || i END"
            )
        ),
        0,
        "Some(domain) for every third row, NULL for the rest"
    );
    assert_eq!(
        count(
            &fx,
            &format!("SELECT count(email_domain(e)) FROM ({emails})")
        ),
        1667
    );
}

#[test]
fn option_results_keep_null_argument_propagation() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    let fx = Fixture::open();
    null_table(&fx);
    register(
        &fx,
        ScalarFunctionBuilder::map1("positive_or_null", |x: i64| {
            CALLS.fetch_add(1, Ordering::SeqCst);
            (x > 1).then_some(x)
        }),
    );

    assert_eq!(
        count(&fx, "SELECT count(positive_or_null(a)) FROM nulls"),
        1
    );
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        4,
        "the closure must not be called for the NULL argument"
    );
    assert_eq!(
        count(
            &fx,
            "SELECT count(*) FROM nulls WHERE positive_or_null(a) IS NULL"
        ),
        4,
        "three `None` results plus the NULL argument"
    );
}

#[test]
fn try_closures_propagate_null_arguments() {
    let fx = Fixture::open();
    null_table(&fx);
    register(
        &fx,
        ScalarFunctionBuilder::try_map1("try_neg", |x: i64| Ok::<_, String>(-x)),
    );
    register(
        &fx,
        ScalarFunctionBuilder::try_map2("try_sub", |a: i64, b: i64| {
            a.checked_sub(b).ok_or("overflow")
        }),
    );
    register(
        &fx,
        ScalarFunctionBuilder::try_map1_str("try_len", |s: &str| {
            i64::try_from(s.len()).map_err(|e| e.to_string())
        }),
    );
    register(
        &fx,
        ScalarFunctionBuilder::try_map2_str("try_cat", |a: &str, b: &str| {
            Ok::<_, String>(format!("{a}{b}"))
        }),
    );

    assert_eq!(count(&fx, "SELECT count(try_neg(a)) FROM nulls"), 4);
    assert_eq!(count(&fx, "SELECT count(try_sub(a, b)) FROM nulls"), 3);
    assert_eq!(count(&fx, "SELECT count(try_sub(b, a)) FROM nulls"), 3);
    assert_eq!(count(&fx, "SELECT count(try_len(s)) FROM nulls"), 4);
    assert_eq!(count(&fx, "SELECT count(try_cat(s, t)) FROM nulls"), 3);
    assert_eq!(count(&fx, "SELECT count(try_cat(t, s)) FROM nulls"), 3);
}

#[test]
fn map3_propagates_a_null_in_any_argument() {
    let fx = Fixture::open();
    null_table(&fx);
    register(
        &fx,
        ScalarFunctionBuilder::map3("clamp3", |x: i64, lo: i64, hi: i64| x.clamp(lo, hi.max(lo))),
    );
    register(
        &fx,
        ScalarFunctionBuilder::try_map3("try_sum3", |a: i64, b: i64, c: i64| {
            a.checked_add(b)
                .and_then(|ab| ab.checked_add(c))
                .ok_or("try_sum3 overflow")
        }),
    );

    for f in ["clamp3", "try_sum3"] {
        for args in ["a, b, c", "b, c, a", "c, a, b"] {
            assert_eq!(
                count(&fx, &format!("SELECT count({f}({args})) FROM nulls")),
                2,
                "{f}({args}) must be NULL wherever any argument is"
            );
        }
    }
    assert_eq!(
        fx.scalar(
            "SELECT clamp3(10::BIGINT, 1::BIGINT, 5::BIGINT)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(5)
    );
    assert_eq!(
        fx.scalar("SELECT sum(try_sum3(a, b, c)) FROM nulls", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(6 + 15)
    );
}

#[test]
fn try_closures_return_values_and_nulls_on_ok() {
    let fx = Fixture::open();
    register(
        &fx,
        ScalarFunctionBuilder::try_map1_str("parse_or_null", |s: &str| {
            if s.is_empty() {
                return Ok(None);
            }
            s.parse::<i64>().map(Some)
        }),
    );

    let input = "SELECT i, CASE WHEN i % 5 = 0 THEN '' ELSE i::VARCHAR END AS s \
                 FROM range(5000) t(i)";
    assert_eq!(
        count(
            &fx,
            &format!(
                "SELECT count(*) FROM ({input}) \
                 WHERE parse_or_null(s) IS DISTINCT FROM CASE WHEN i % 5 <> 0 THEN i END"
            )
        ),
        0,
        "Ok(Some) parses, Ok(None) is NULL, across three chunks"
    );
}

#[test]
fn an_err_fails_the_query_with_its_message() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    let fx = Fixture::open();
    register(
        &fx,
        ScalarFunctionBuilder::try_map1("reject_three", |x: i64| {
            CALLS.fetch_add(1, Ordering::SeqCst);
            if x == 3 {
                Err(format!("reject_three: refusing {x}"))
            } else {
                Ok(x)
            }
        }),
    );
    register(
        &fx,
        ScalarFunctionBuilder::try_map1_str("strict_domain", |s: &str| {
            s.split_once('@')
                .map(|(_, domain)| domain.to_owned())
                .ok_or_else(|| format!("not an email address: {s:?}"))
        }),
    );

    // One chunk: the first `Err` stops it, so rows 4..10 never reach the
    // closure.
    let err = error_of(&fx, "SELECT reject_three(i) FROM range(10) t(i)");
    assert!(err.contains("reject_three: refusing 3"), "{err}");
    assert_eq!(CALLS.load(Ordering::SeqCst), 4, "rows 0..=3, then stop");

    // A failing row in a later chunk still fails the whole query.
    let err = error_of(
        &fx,
        "SELECT sum(length(strict_domain(CASE WHEN i = 4321 THEN 'oops' \
         ELSE 'a@b' END))) FROM range(5000) t(i)",
    );
    assert!(err.contains("not an email address: \"oops\""), "{err}");

    // The connection survives, and the function still works.
    assert_eq!(
        fx.scalar("SELECT strict_domain('me@example.com')", |r, i| unsafe {
            r.read_str(i).to_owned()
        }),
        Some("example.com".to_owned())
    );
}

#[test]
fn a_panic_in_a_try_closure_is_a_query_error() {
    let fx = Fixture::open();
    register(
        &fx,
        ScalarFunctionBuilder::try_map2("try_boom", |a: i64, b: i64| {
            assert!(a != 13, "try closure deliberately exploded");
            Ok::<_, String>(a + b)
        }),
    );

    let err = error_of(&fx, "SELECT try_boom(i, 1::BIGINT) FROM range(100) t(i)");
    assert!(err.contains("try closure deliberately exploded"), "{err}");
    assert_eq!(
        fx.scalar("SELECT try_boom(1::BIGINT, 2::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(3)
    );
}
