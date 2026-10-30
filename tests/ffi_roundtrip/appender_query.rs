// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `Appender::for_query` (`duckdb-1-5`) and the appender's
//! `from_raw` / `into_raw` (eighth review: apigaps candidate slot 409 and
//! APIGAPS-9).

use quack_rs::appender::Appender;

use super::Fixture;

fn count(fx: &Fixture, sql: &str) -> Option<i64> {
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
}

/// An appender adopted back from its raw handle keeps appending, and is
/// closed and destroyed once by `Drop`.
#[test]
fn an_appender_round_trips_through_its_raw_handle() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ar(a INTEGER)");
    // SAFETY: `con` is open.
    let appender = unsafe { Appender::new(fx.con(), None, c"ar") }.expect("appender");
    appender.row(|row| row.append_i32(1)).expect("row 1");
    let raw = appender.into_raw();
    // SAFETY: `raw` came from `into_raw`, at a row boundary, owned by nothing else.
    let appender = unsafe { Appender::from_raw(raw) };
    appender.row(|row| row.append_i32(2)).expect("row 2");
    appender.close().expect("close");
    drop(appender);
    assert_eq!(count(&fx, "SELECT sum(a)::BIGINT FROM ar"), Some(3));
}

#[cfg(feature = "duckdb-1-5")]
mod for_query {
    use quack_rs::appender::Appender;
    use quack_rs::types::{LogicalType, TypeId};

    use super::{count, Fixture};

    fn types() -> [LogicalType; 2] {
        [
            LogicalType::new(TypeId::Integer),
            LogicalType::new(TypeId::Varchar),
        ]
    }

    /// AUTHT-13: with `duckdb-1-5` a row closure's error type is `ErrorData`,
    /// and `?` on a `Value` constructor's `ExtensionError` now converts.
    #[test]
    fn an_extension_error_propagates_out_of_a_row_closure() {
        use quack_rs::error_data::DuckDbErrorType;
        use quack_rs::value::Value;

        let fx = Fixture::open();
        fx.query("CREATE TABLE ae(d DECIMAL(4, 0))");
        // SAFETY: `con` is open.
        let appender = unsafe { Appender::new(fx.con(), None, c"ae") }.expect("appender");
        appender
            .row(|row| row.append_value(&Value::decimal(4, 0, 12)?))
            .expect("a valid DECIMAL(4, 0)");
        let err = appender
            .row(|row| row.append_value(&Value::decimal(4, 0, 10_000)?))
            .expect_err("10000 has five digits");
        assert_eq!(err.error_type(), DuckDbErrorType::Invalid);
        assert!(
            err.message()
                .is_some_and(|m| m.contains("more than 4 digits")),
            "{err}"
        );
        appender
            .close()
            .expect("the first row is written; nothing was half-appended");
        assert_eq!(count(&fx, "SELECT count(*) FROM ae"), Some(1));
    }

    #[test]
    fn appended_rows_feed_the_query_through_the_default_names() {
        let fx = Fixture::open();
        fx.query("CREATE TABLE aq(a INTEGER, s VARCHAR)");
        // SAFETY: `con` is open.
        let appender = unsafe {
            Appender::for_query(
                fx.con(),
                "INSERT INTO aq SELECT col1 * 10, upper(col2) FROM appended_data",
                &types(),
                None,
                None,
            )
        }
        .expect("for_query");
        assert_eq!(appender.column_count(), 2);
        for (i, s) in [(4, "abc"), (5, "de")] {
            appender
                .row(|row| {
                    row.append_i32(i)?;
                    row.append_str(s)
                })
                .expect("row");
        }
        appender.close().expect("close runs the query");
        assert_eq!(
            count(
                &fx,
                "SELECT count(*) FROM aq WHERE (a, s) IN ((40, 'ABC'), (50, 'DE'))"
            ),
            Some(2)
        );
    }

    /// The reviewer's "upsert from a batch": a named table and columns, and
    /// `INSERT OR REPLACE` over a primary key.
    #[test]
    fn a_named_batch_upserts_into_a_table() {
        let fx = Fixture::open();
        fx.query("CREATE TABLE users(id INTEGER PRIMARY KEY, name VARCHAR)");
        fx.query("INSERT INTO users VALUES (1, 'old'), (2, 'keep')");
        // SAFETY: `con` is open.
        let upsert = unsafe {
            Appender::for_query(
                fx.con(),
                "INSERT OR REPLACE INTO users SELECT id, name FROM batch",
                &types(),
                Some("batch"),
                Some(&["id", "name"]),
            )
        }
        .expect("for_query");
        for (id, name) in [(1, "new"), (3, "added")] {
            upsert
                .row(|row| {
                    row.append_i32(id)?;
                    row.append_str(name)
                })
                .expect("row");
        }
        upsert.close().expect("close");
        assert_eq!(
            count(
                &fx,
                "SELECT count(*) FROM users WHERE (id, name) IN ((1, 'new'), (2, 'keep'), (3, 'added'))"
            ),
            Some(3)
        );
        assert_eq!(count(&fx, "SELECT count(*) FROM users"), Some(3));
    }

    #[test]
    fn bad_arguments_are_refused_before_duckdb_is_called() {
        let fx = Fixture::open();
        // SAFETY (every call): `con` is open.
        let err = unsafe { Appender::for_query(fx.con(), "SELECT 1", &[], None, None) }
            .err()
            .expect("no column types");
        assert!(
            err.to_string().contains("at least one column type"),
            "{err}"
        );
        let err = unsafe {
            Appender::for_query(fx.con(), "SELECT 1", &types(), None, Some(&["only_one"]))
        }
        .err()
        .expect("one name for two types");
        assert!(err.to_string().contains("1 column name(s) for 2"), "{err}");
        let err = unsafe { Appender::for_query(fx.con(), "SELECT\0 1", &types(), None, None) }
            .err()
            .expect("NUL in SQL");
        assert!(err.to_string().contains("NUL"), "{err}");
        let err =
            unsafe { Appender::for_query(fx.con(), "SELECT 1", &types(), Some("b\0ad"), None) }
                .err()
                .expect("NUL in the table name");
        assert!(err.to_string().contains("table name"), "{err}");
        let err = unsafe {
            Appender::for_query(fx.con(), "SELECT 1", &types(), None, Some(&["a", "b\0"]))
        }
        .err()
        .expect("NUL in a column name");
        assert!(err.to_string().contains("column name"), "{err}");
    }

    /// `DuckDB` stores the query unparsed; a bad one fails at the flush, as
    /// the docs say, and the connection stays usable.
    #[test]
    fn a_query_that_does_not_bind_fails_at_close() {
        let fx = Fixture::open();
        // SAFETY: `con` is open.
        let appender = unsafe {
            Appender::for_query(
                fx.con(),
                "INSERT INTO no_such_table SELECT * FROM appended_data",
                &types(),
                None,
                None,
            )
        }
        .expect("creating it does not look at the SQL");
        appender
            .row(|row| {
                row.append_i32(1)?;
                row.append_str("x")
            })
            .expect("row");
        let err = appender.close().expect_err("the flush binds the query");
        assert!(err.to_string().contains("no_such_table"), "{err}");
        drop(appender);
        assert_eq!(count(&fx, "SELECT 1::BIGINT"), Some(1));
    }
}
