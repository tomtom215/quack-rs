// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Prepared-statement binds that no other test makes (`bind_str`, `bind_null`,
//! `clear_bindings`) and the query-progress readout of a connection with nothing
//! running. (A running query's progress depends on the progress-bar thread's
//! timing, which no test can pin down without flaking.)

use quack_rs::query::OwnedConnection;

use super::Fixture;

/// The row `statement` returns: `(length, is_null, text)`.
fn row(stmt: &quack_rs::query::PreparedStatement) -> (i64, bool, Option<String>) {
    let mut result = stmt.execute().expect("execute");
    let chunk = result.next_chunk().expect("fetch").expect("one row");
    assert_eq!(chunk.size(), 1);
    // SAFETY: the columns are (BIGINT, BOOLEAN, VARCHAR) and row 0 exists.
    unsafe {
        let text = chunk.reader(2);
        (
            chunk.reader(0).read_i64(0),
            chunk.reader(1).read_bool(0),
            text.is_valid(0).then(|| text.read_str(0).to_owned()),
        )
    }
}

#[test]
fn bind_str_bind_null_and_clear_bindings_drive_a_reused_statement() {
    let fx = Fixture::open();
    // SAFETY: `db` is open for the fixture's lifetime.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let stmt = con
        .prepare("SELECT length(?::VARCHAR), ?::VARCHAR IS NULL, ?::VARCHAR")
        .expect("prepare");
    assert_eq!(stmt.parameter_count(), 3);

    stmt.bind_str(1, "a\0b").expect("bind_str with a NUL");
    stmt.bind_null(2).expect("bind_null");
    stmt.bind_str(3, "h\u{e9}llo \u{4e16}\u{754c}")
        .expect("bind_str");
    assert_eq!(
        row(&stmt),
        (3, true, Some("h\u{e9}llo \u{4e16}\u{754c}".to_owned())),
        "the NUL is part of the string; NULL is NULL"
    );

    // Cleared, nothing is bound: executing says so.
    stmt.clear_bindings().expect("clear_bindings");
    let err = stmt.execute().map(drop).expect_err("no values are bound");
    assert!(
        err.as_str().contains("not provided") || err.as_str().contains("prepared statement"),
        "{err}"
    );

    // And the statement binds afresh: other values, NULL where text was.
    stmt.bind_str(1, "").expect("bind the empty string");
    stmt.bind_str(2, "x").expect("bind_str");
    stmt.bind_null(3).expect("bind_null");
    assert_eq!(row(&stmt), (0, false, None));

    // An index outside the statement is refused, and says which.
    for index in [0, 4] {
        let err = stmt.bind_str(index, "x").expect_err("out of range");
        assert!(err.as_str().contains(&index.to_string()), "{index}: {err}");
        let err = stmt.bind_null(index).expect_err("out of range");
        assert!(err.as_str().contains(&index.to_string()), "{index}: {err}");
    }
}

#[test]
fn query_progress_is_unavailable_on_an_idle_connection() {
    let fx = Fixture::open();
    // SAFETY: `db` is open for the fixture's lifetime.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let idle = con.progress();
    assert!(idle.percentage < 0.0, "{idle:?}");
    assert_eq!(idle.rows_processed, 0);
    let from_handle = con.interrupt_handle().progress();
    assert_eq!(from_handle, idle);
}
