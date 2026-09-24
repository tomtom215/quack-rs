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
    let chunk = source.next_chunk().expect("fetch").expect("one chunk");
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

/// The fourth audit's DOC-1, pinned. `append_default` on a column with no
/// `DEFAULT` appends `NULL` (the docs said it was an error), and it is a
/// non-constant `DEFAULT` that fails — one `column_has_default` reports as
/// `true`. (`clear`, which recovers the poisoned appender, needs `duckdb-1-5`.)
#[cfg(feature = "duckdb-1-5")]
#[test]
fn append_default_fills_null_without_a_default_and_fails_for_a_non_constant_one() {
    use quack_rs::table_description::TableDescription;

    let fx = Fixture::open();
    fx.query("CREATE SEQUENCE ap_seq");
    fx.query(
        "CREATE TABLE ap_defaults (a INTEGER, b INTEGER DEFAULT 7, \
         c BIGINT DEFAULT nextval('ap_seq'))",
    );
    // SAFETY: `con` is open and the table exists.
    let description =
        unsafe { TableDescription::create(fx.con(), "main", "ap_defaults") }.expect("describe");
    assert_eq!(
        (0..3)
            .map(|i| description.column_has_default(i))
            .collect::<Vec<_>>(),
        [Some(false), Some(true), Some(true)]
    );
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_defaults") }.expect("create");
    appender
        .row(|row| {
            row.append_default()?;
            row.append_default()?;
            row.append_i64(1)
        })
        .expect("no DEFAULT gives NULL; a constant DEFAULT gives its value");
    // Flushed, so the `clear` below (which drops every buffered row) keeps it.
    appender.flush().expect("flush");
    let err = appender
        .row(|row| {
            row.append_i32(1)?;
            row.append_i32(2)?;
            row.append_default()
        })
        .expect_err("a non-constant DEFAULT cannot be appended");
    assert!(
        err.to_string().contains("AppendDefault is not supported"),
        "{err}"
    );
    appender.clear().expect("clear the half-written row");
    appender.close().expect("close");
    drop(appender);
    assert_eq!(
        fx.scalar(
            "SELECT (a IS NULL AND b = 7)::BIGINT FROM ap_defaults",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(1)
    );
}

/// `DuckDB`'s `EndRow` ends the row (its column counter back to 0, the row
/// counted) *before* the automatic flush every 204,800 rows, so a flush that
/// fails there — a `NOT NULL` violation in any buffered row — leaves no row
/// half-written. `row` still took the failure for a half-written row: it
/// poisoned the appender and blamed "a row failed after 1 of its values",
/// and every later append was refused. Now the row counts as ended, the
/// constraint error is reported as it is, and appending carries on.
#[test]
fn a_failing_automatic_flush_does_not_poison_the_appender() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_autoflush (x INTEGER NOT NULL)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_autoflush") }.expect("create");
    appender
        .row(|row| row.append_null())
        .expect("the NULL is only buffered; DuckDB checks NOT NULL at flush time");
    let mut failure = None;
    for i in 1..300_000_u32 {
        if let Err(e) = appender.row(|row| row.append_i32(1)) {
            failure = Some((i, e.to_string()));
            break;
        }
    }
    let (at, err) = failure.expect("the automatic flush fails on the NULL");
    assert_eq!(at, 204_799, "the 204,800th row triggers the flush: {err}");
    assert!(err.contains("NOT NULL"), "{err}");
    assert!(!err.contains("poisoned"), "{err}");

    appender
        .row(|row| row.append_i32(2))
        .expect("the appender is not poisoned");
    let err = appender
        .close()
        .expect_err("the NULL row is still buffered");
    let text = err.to_string();
    assert!(text.contains("NOT NULL"), "{text}");
    assert!(!text.contains("poisoned"), "{text}");
}

/// Nothing ties an appender to its connection's lifetime. `DuckDB`'s appender
/// buffers into its own chunk and holds only a weak reference to the client
/// context, which it checks before writing (`Appender::FlushInternal`), so
/// appending after the connection closes is not a use-after-free, but the
/// rows can no longer be written: `close`
/// reports `DuckDB`'s "closed connection" error and the rows are gone. Pinned
/// so the documented consequence stays accurate.
#[test]
fn rows_buffered_after_the_connection_closes_are_lost_with_an_error() {
    use quack_rs::query::OwnedConnection;
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_closed_con (x INTEGER)");
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(con.as_raw(), None, c"ap_closed_con") }.expect("create");
    appender.row(|row| row.append_i32(1)).expect("buffered");
    drop(con);
    appender
        .row(|row| row.append_i32(2))
        .expect("appends still succeed: the rows are only buffered");
    let err = appender.close().expect_err("the rows cannot be written");
    assert!(err.to_string().contains("closed connection"), "{err}");
    drop(appender);
    assert_eq!(count(&fx, "ap_closed_con"), 0);
}

/// A `row` closure that panics after its first value leaves the same
/// half-written row as one that returns an error there, so it poisons the
/// appender the same way. Before the fix the panic left it unpoisoned: the
/// caller could finish the row by hand, and `close` committed `(1, 2)`, half
/// of it from the closure that had panicked.
#[test]
fn a_row_closure_that_panics_mid_row_poisons_the_appender() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE ap_panic (a INTEGER, b INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"ap_panic") }.expect("create");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        appender.row(|row| {
            row.append_i32(1)?;
            panic!("the closure panics mid-row");
        })
    }));
    assert!(panicked.is_err(), "the panic reaches the caller");
    let finish = appender.append_i32(2).and_then(|()| appender.end_row());
    let err = finish.expect_err("a poisoned appender refuses the rest of the row");
    assert!(err.to_string().contains("poisoned"), "{err}");
    assert!(appender.close().is_err(), "close reports the abandoned row");
    drop(appender);
    assert_eq!(
        count(&fx, "ap_panic"),
        0,
        "no half-written row is committed"
    );
}
