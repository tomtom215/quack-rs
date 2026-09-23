// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! What happens to buffered rows when an append fails half-way through a
//! row, when a row is left unfinished, and when the appender is used after
//! `close`.
//!
//! `DuckDB`'s appender counts values into the current row
//! (`BaseAppender::column`). A value that fails to convert throws *before*
//! that counter moves, but the row's earlier values already moved it, and
//! nothing in the C API moves it back: `EndRow` wants every column,
//! `Flush` wants none, and `Close` flushes only in those two states — in any
//! other it returns success without writing anything.

use quack_rs::appender::Appender;

use super::Fixture;

fn count(fx: &Fixture, table: &str) -> i64 {
    fx.scalar(&format!("SELECT count(*) FROM {table}"), |r, i| unsafe {
        r.read_i64(i)
    })
    .expect("count(*) is never NULL")
}

/// The audit's F-V3: 100 good rows, then a row whose second value does not
/// convert. Before the fix `row` failed, `close` returned `Ok`, and the table
/// was empty — every buffered row silently gone.
#[test]
fn a_value_failing_mid_row_makes_close_report_the_lost_rows() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_mid (a INTEGER, b INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_mid") }.expect("create");
    for i in 0..100 {
        appender
            .row(|row| {
                row.append_i32(i)?;
                row.append_i32(i)
            })
            .expect("good row");
    }
    let bad = appender.row(|row| {
        row.append_i32(1)?;
        row.append_str("oops")
    });
    assert!(bad.is_err(), "'oops' is not an INTEGER");

    // The appender is poisoned: later rows are refused up front instead of
    // failing with DuckDB's "Too many appends for chunk!".
    let later = appender
        .row(|row| {
            row.append_i32(7)?;
            row.append_i32(7)
        })
        .expect_err("a poisoned appender refuses further rows");
    assert!(later.to_string().contains("100"), "{later}");

    let err = appender
        .close()
        .expect_err("close must not report success for rows it did not write");
    let text = err.to_string();
    assert!(text.contains("100 row"), "names the lost rows: {text}");
    drop(appender);
    assert_eq!(count(&fx, "ap_mid"), 0, "DuckDB wrote none of them");
}

/// A value failing as the *first* of its row leaves `DuckDB`'s counter at zero,
/// so nothing is lost and the appender carries on.
#[test]
fn a_value_failing_first_in_its_row_loses_nothing() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_first (a INTEGER, b INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_first") }.expect("create");
    appender
        .row(|row| {
            row.append_i32(1)?;
            row.append_i32(1)
        })
        .expect("good row");
    assert!(appender
        .row(|row| {
            row.append_str("nope")?;
            row.append_i32(2)
        })
        .is_err());
    appender
        .row(|row| {
            row.append_i32(3)?;
            row.append_i32(3)
        })
        .expect("the appender is still usable");
    appender.close().expect("both good rows are written");
    drop(appender);
    assert_eq!(count(&fx, "ap_first"), 2);
}

/// `close` with a row started but not ended wrote nothing and returned `Ok`.
#[test]
fn closing_with_an_unfinished_row_is_an_error() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_open_row (a INTEGER, b INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_open_row") }.expect("create");
    for i in 0..3 {
        appender
            .row(|row| {
                row.append_i32(i)?;
                row.append_i32(i)
            })
            .expect("good row");
    }
    appender.append_i32(9).expect("first value of a fourth row");
    let err = appender.close().expect_err("the fourth row is unfinished");
    assert!(err.to_string().contains("3 row"), "{err}");
    drop(appender);
    assert_eq!(count(&fx, "ap_open_row"), 0);
}

/// With `duckdb-1-5`, `clear` discards the buffered rows and the half-written
/// one, and the appender works again.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn clear_recovers_a_poisoned_appender() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_clear (a INTEGER, b INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_clear") }.expect("create");
    appender
        .row(|row| {
            row.append_i32(0)?;
            row.append_i32(0)
        })
        .expect("good row");
    assert!(appender
        .row(|row| {
            row.append_i32(1)?;
            row.append_str("nan")
        })
        .is_err());
    appender.clear().expect("clear resets the appender");
    appender
        .row(|row| {
            row.append_i32(10)?;
            row.append_i32(20)
        })
        .expect("usable after clear");
    appender.close().expect("close");
    drop(appender);
    let got = fx.scalar("SELECT sum(a) + sum(b) FROM ap_clear", |r, i| unsafe {
        r.read_i128(i)
    });
    assert_eq!(got, Some(30), "only the row appended after clear");
}

/// The audit's F-V10: `close` documented that no further rows may be
/// appended, but `append_i32` after it returned `Ok`.
#[test]
fn the_appender_refuses_work_after_close() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_closed (a INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_closed") }.expect("create");
    appender.row(|row| row.append_i32(1)).expect("row");
    appender.close().expect("close");
    assert!(appender.append_i32(2).is_err(), "append after close");
    assert!(appender.end_row().is_err(), "end_row after close");
    assert!(appender.row(|row| row.append_i32(3)).is_err());
    assert!(appender.flush().is_err(), "flush after close");
    let err = appender.append_null().expect_err("closed");
    assert!(err.to_string().contains("closed"), "{err}");
    appender.close().expect("closing twice is harmless");
    drop(appender);
    assert_eq!(count(&fx, "ap_closed"), 1);
}

/// Buffered rows are written by column *position* at flush time. A column
/// dropped and another added by a different connection in between means a
/// value lands in the new column: `DuckDB`'s behaviour, documented on the
/// module, pinned here.
#[test]
fn buffered_rows_are_written_by_position_at_flush_time() {
    use quack_rs::query::OwnedConnection;

    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_alter (a INTEGER, b INTEGER)");
    // SAFETY: the fixture's database outlives the connection.
    let other = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_alter") }.expect("create");
    appender
        .row(|row| {
            row.append_i32(1)?;
            row.append_i32(2)
        })
        .expect("row");
    other
        .execute("ALTER TABLE ap_alter DROP COLUMN b")
        .expect("drop");
    other
        .execute("ALTER TABLE ap_alter ADD COLUMN z VARCHAR")
        .expect("add");
    let closed = appender.close();
    drop(appender);
    let rows = fx.scalar(
        "SELECT string_agg(a::VARCHAR || '|' || coalesce(z, 'NULL'), ',') FROM ap_alter",
        |r, i| unsafe { r.read_str(i).to_owned() },
    );
    assert!(closed.is_ok(), "{closed:?}");
    assert_eq!(rows.as_deref(), Some("1|2"), "b's value lands in z");
}

/// `append_chunk` in the middle of a row is refused (an automatic flush it
/// triggers would fail on the open row after buffering the chunk), and the
/// documented ordering of chunks against buffered rows holds.
#[test]
fn append_chunk_in_the_middle_of_a_row_is_refused() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_chunk (a INTEGER, b INTEGER)");
    let mut source = fx.query("SELECT 5::INTEGER AS a, 6::INTEGER AS b");
    let chunk = source.next_chunk().expect("one chunk");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_chunk") }.expect("create");
    appender
        .row(|row| {
            row.append_i32(1)?;
            row.append_i32(1)
        })
        .expect("good row");
    appender.append_i32(2).expect("first value of a second row");
    assert!(appender.append_chunk(&chunk).is_err());
    appender
        .append_i32(2)
        .expect("the row can still be finished");
    appender.end_row().expect("and ended");
    appender.append_chunk(&chunk).expect("between rows is fine");
    appender.close().expect("close");
    drop(appender);
    let order = fx.scalar(
        "SELECT string_agg(a::VARCHAR, ',' ORDER BY rowid) FROM ap_chunk",
        |r, i| unsafe { r.read_str(i).to_owned() },
    );
    // Pins the ordering `append_chunk` documents: the chunk goes straight to
    // DuckDB's table-bound buffer, ahead of the two rows still waiting in its
    // row buffer.
    assert_eq!(order.as_deref(), Some("5,1,2"));
}
