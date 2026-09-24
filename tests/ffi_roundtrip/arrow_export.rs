// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `arrow::data_chunk_to_arrow` refuses the values `DuckDB` would export as
//! different values without an error (`docs/upstream-duckdb-reports.md`,
//! item 16): an `INTERVAL` whose microseconds overflow Arrow's nanoseconds,
//! and a `HUGEINT` / `UHUGEINT` with more digits than the `decimal128(38, 0)`
//! it is exported as. Before the check each of these exported without an
//! error and came back as a different value.

use quack_rs::arrow::{data_chunk_to_arrow, ArrowOptions};
use quack_rs::query::OwnedConnection;

use super::Fixture;

fn connect(fx: &Fixture) -> OwnedConnection {
    // SAFETY: the fixture's database outlives the connection.
    unsafe { OwnedConnection::open(fx.db()) }.expect("connect")
}

/// Exports the first chunk of `sql`'s result: `Ok` or the error message.
fn export(con: &OwnedConnection, sql: &str) -> Result<(), String> {
    let mut result = con.query(sql).expect(sql);
    let options = ArrowOptions::from_connection(con).expect("options");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    data_chunk_to_arrow(&options, &chunk)
        .map(drop)
        .map_err(|e| e.message().unwrap_or_default())
}

fn assert_refused(con: &OwnedConnection, sql: &str) {
    let message = export(con, sql).expect_err(sql);
    assert!(message.contains("item 16"), "{sql}: {message}");
}

/// The largest interval that exports exactly: `i64::MAX / 1000`
/// microseconds is 2,562,047.78 hours.
const LAST_HOUR: &str = "INTERVAL 2562047 HOUR";
const FIRST_BAD_HOUR: &str = "INTERVAL 2562048 HOUR";

#[test]
fn an_interval_whose_nanoseconds_overflow_is_refused_at_any_depth() {
    let fx = Fixture::open();
    let con = connect(&fx);
    for sql in [
        format!("SELECT {FIRST_BAD_HOUR} AS v"),
        format!("SELECT -{FIRST_BAD_HOUR} AS v"),
        format!("SELECT [{LAST_HOUR}, {FIRST_BAD_HOUR}] AS v"),
        format!("SELECT {{'a': 1, 'b': {FIRST_BAD_HOUR}}} AS v"),
        format!("SELECT MAP {{'k': {FIRST_BAD_HOUR}}} AS v"),
        format!("SELECT [{FIRST_BAD_HOUR}]::INTERVAL[1] AS v"),
        format!(
            "SELECT i, CASE WHEN i = 2047 THEN {FIRST_BAD_HOUR} END AS v FROM range(2048) t(i)"
        ),
    ] {
        assert_refused(&con, &sql);
    }
    for sql in [
        format!("SELECT {LAST_HOUR} AS v"),
        format!("SELECT [{LAST_HOUR}, NULL] AS v"),
        "SELECT NULL::INTERVAL AS v".to_owned(),
    ] {
        export(&con, &sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
}

#[test]
fn a_hugeint_wider_than_decimal128_is_refused_unless_exported_losslessly() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let wide = "SELECT '-170141183460469231731687303715884105728'::HUGEINT AS v";
    assert_refused(&con, wide);
    assert_refused(
        &con,
        "SELECT ['170141183460469231731687303715884105727'::HUGEINT] AS v",
    );
    export(
        &con,
        "SELECT '-99999999999999999999999999999999999999'::HUGEINT AS v",
    )
    .expect("38 digits fit");
    con.execute("SET arrow_lossless_conversion = true")
        .expect("set");
    export(&con, wide).expect("a lossless export keeps every HUGEINT");
}

#[test]
fn a_uhugeint_with_more_than_38_digits_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    // 10^38: 39 digits, below 2^127, so its bits survive but it breaks the
    // declared precision.
    assert_refused(
        &con,
        "SELECT '100000000000000000000000000000000000000'::UHUGEINT AS v",
    );
    // 2^127 and the maximum: exported as negative numbers.
    assert_refused(
        &con,
        "SELECT '170141183460469231731687303715884105728'::UHUGEINT AS v",
    );
    assert_refused(
        &con,
        "SELECT '340282366920938463463374607431768211455'::UHUGEINT AS v",
    );
    export(
        &con,
        "SELECT '99999999999999999999999999999999999999'::UHUGEINT AS v",
    )
    .expect("38 digits fit");
}
