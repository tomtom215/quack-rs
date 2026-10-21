// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A streaming result that stops early must not look like a finished one.
//!
//! `duckdb_fetch_chunk` returns null both at the end of the rows and when a
//! `Fetch` fails; on failure it records the error on the result
//! (`QueryResult::SetError`). `next_chunk` used to return `None` in both
//! cases, so a query failing part-way through a stream — or, up to `DuckDB`
//! 1.5.x, a stream invalidated by another statement on the connection — read
//! as a complete, shorter result (audit F-V4: 1,871,872 of 3,000,000 rows,
//! then a clean `None`).
//!
//! `DuckDB` 2.0.0 changed what a second statement does to an open stream: it
//! is refused with a "Resource In Use" error and the stream carries on
//! (`ClientContext::InitialCleanup`, `client_context.cpp`), and dropping an
//! unfinished stream aborts its query at once (`QueryResult::Close`,
//! `query_result.cpp`). Both behaviours are pinned per engine below.

use quack_rs::query::OwnedConnection;

use super::Fixture;

/// A runtime error two million rows in.
#[test]
fn a_query_failing_mid_stream_is_an_error_not_an_end() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    // One thread keeps the rows before the error deterministic.
    con.execute("SET threads = 1").expect("threads");
    let statement = con
        .prepare(
            "SELECT CASE WHEN i < 2000000 THEN i ELSE error('boom') END \
             FROM range(3000000) t(i)",
        )
        .expect("prepare");
    let mut result = statement.execute_streaming().expect("execute");
    let mut rows = 0_usize;
    let err = loop {
        match result.next_chunk() {
            Ok(Some(chunk)) => rows += chunk.size(),
            Ok(None) => panic!("the stream ended cleanly after {rows} rows"),
            Err(err) => break err,
        }
    };
    assert!(rows < 3_000_000, "{rows}");
    assert!(err.to_string().contains("boom"), "{err}");
    // The error is sticky: a caller that ignored it cannot mistake the next
    // call for the end of the rows.
    assert!(result.next_chunk().is_err());
    assert!(result.next_chunk().is_err());
}

/// Up to `DuckDB` 1.5.x, running another statement on the connection
/// invalidates the stream, and the stream must say so rather than end
/// cleanly. 2.0.0 refuses the other statement instead and leaves the stream
/// whole.
#[test]
fn another_statement_on_the_connection_ends_a_stream_with_an_error() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let statement = con
        .prepare("SELECT i FROM range(300000) t(i)")
        .expect("prepare");
    let mut result = statement.execute_streaming().expect("execute");
    let first = result.next_chunk().expect("fetch").expect("a first chunk");
    assert!(first.size() > 0);
    if fx.engine_major() >= 2 {
        let err = con
            .query("SELECT 1")
            .err()
            .expect("DuckDB 2.0 refuses a statement while a stream is open");
        assert!(
            err.to_string().contains("Resource In Use") && err.to_string().contains("open result"),
            "{err}"
        );
        let mut rows = first.size();
        while let Some(chunk) = result.next_chunk().expect("the stream is intact") {
            rows += chunk.size();
        }
        assert_eq!(rows, 300_000);
        con.query("SELECT 1")
            .expect("a drained stream no longer holds the connection");
        return;
    }
    con.query("SELECT 1").expect("another statement");
    let err = result
        .next_chunk()
        .expect_err("the stream was invalidated, not finished");
    assert!(err.to_string().contains("closed"), "{err}");
}

/// An appender writes through its connection's client context, so its flush
/// is "another statement" too. On 1.x it ends the open stream the same way;
/// on 2.0.0 the flush is refused while the stream is open — and, because a
/// failed flush discards the appender's buffer there, the rows are lost.
#[test]
fn an_appender_flush_while_a_stream_is_open_on_its_connection() {
    use quack_rs::appender::Appender;

    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    con.execute("CREATE TABLE qs_app (x INTEGER)")
        .expect("create");
    // SAFETY: `con` is open and the table exists.
    let appender =
        unsafe { Appender::new(con.as_raw(), None, c"qs_app") }.expect("create appender");
    appender.row(|row| row.append_i32(7)).expect("buffered");
    let statement = con
        .prepare("SELECT i FROM range(300000) t(i)")
        .expect("prepare");
    let mut result = statement.execute_streaming().expect("execute");
    assert!(result.next_chunk().expect("fetch").is_some());
    let flushed = appender.flush();
    if fx.engine_major() >= 2 {
        let err = flushed.expect_err("DuckDB 2.0 refuses the flush");
        assert!(err.to_string().contains("open result"), "{err}");
        #[cfg(feature = "duckdb-1-5")]
        assert_eq!(
            err.error_type(),
            quack_rs::error_data::DuckDbErrorType::ResourceInUse,
            "{err}"
        );
        while result.next_chunk().expect("the stream is intact").is_some() {}
        appender.close().expect("nothing left to write");
        drop(appender);
        assert_eq!(
            fx.scalar("SELECT count(*) FROM qs_app", |r, i| unsafe {
                r.read_i64(i)
            }),
            Some(0),
            "the refused flush discarded the buffered row"
        );
        return;
    }
    flushed.expect("DuckDB 1.x writes the row");
    let err = result.next_chunk().expect_err("the flush ended the stream");
    assert!(err.to_string().contains("closed"), "{err}");
    drop(appender);
    assert_eq!(
        fx.scalar("SELECT count(*) FROM qs_app", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(1)
    );
}

/// A clean end stays `Ok(None)` on every later call: `DuckDB` would record an
/// error ("unsuccessful or closed") if a finished stream were fetched again,
/// so the wrapper does not ask.
#[test]
fn a_finished_stream_keeps_returning_none() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let statement = con
        .prepare("SELECT i FROM range(5000) t(i)")
        .expect("prepare");
    let mut result = statement.execute_streaming().expect("execute");
    let mut rows = 0_usize;
    while let Some(chunk) = result.next_chunk().expect("no error") {
        rows += chunk.size();
    }
    assert_eq!(rows, 5_000);
    assert!(result.next_chunk().expect("still no error").is_none());
    assert!(result.next_chunk().expect("still no error").is_none());

    // A materialised result ends the same way.
    let mut result = con.query("SELECT 42").expect("query");
    assert!(result.next_chunk().expect("fetch").is_some());
    assert!(result.next_chunk().expect("fetch").is_none());
    assert!(result.next_chunk().expect("fetch").is_none());
}

/// Live `Cursor` values: the bind template and every clone of it.
static CURSORS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

struct Cursor {
    next: i64,
    end: i64,
}
impl Clone for Cursor {
    fn clone(&self) -> Self {
        CURSORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self {
            next: self.next,
            end: self.end,
        }
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        CURSORS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Up to 1.5.x `DuckDB` keeps an abandoned stream's operator states on the
/// connection until the next statement there, not until the `QueryResult` or
/// the `PreparedStatement` is dropped. 2.0.0 aborts the query when the
/// unfinished result is destroyed (`QueryResult::Close`), so the states go
/// with it. The documented behaviour (Known Limitations, "An abandoned stream
/// keeps its table-function state") is pinned here per engine: if either
/// changes, this fails and the page needs updating.
#[test]
fn an_abandoned_stream_keeps_its_table_state_until_the_next_statement() {
    use quack_rs::table::TableFunctionBuilder;
    use quack_rs::types::TypeId;
    use std::sync::atomic::Ordering;

    let fx = Fixture::open();
    let cursor = TableFunctionBuilder::new("stream_cursor")
        .param(TypeId::BigInt)
        .with_state::<Cursor, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: the function declares one BIGINT parameter.
            let end = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
            CURSORS.fetch_add(1, Ordering::SeqCst);
            Ok(Cursor { next: 0, end })
        })
        .scan(|cursor, chunk| {
            // SAFETY: column 0 is the BIGINT result column.
            let mut writer = unsafe { chunk.writer(0) };
            let mut rows = 0;
            while rows < 2048 && cursor.next < cursor.end {
                // SAFETY: `rows` is below the chunk's capacity of 2048.
                unsafe { writer.write_i64(rows, cursor.next) };
                cursor.next += 1;
                rows += 1;
            }
            // SAFETY: `rows` rows were written.
            unsafe { chunk.set_size(rows) };
            Ok(())
        })
        .build()
        .expect("build");
    // SAFETY: the fixture's connection is open.
    unsafe { cursor.register(fx.con()) }.expect("register");
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let statement = con
        .prepare("SELECT n FROM stream_cursor(10000000)")
        .expect("prepare");
    let mut result = statement.execute_streaming().expect("execute");
    assert!(result.next_chunk().expect("fetch").is_some());
    drop(result);
    drop(statement);
    if fx.engine_major() >= 2 {
        assert_eq!(
            CURSORS.load(Ordering::SeqCst),
            0,
            "DuckDB 2.0 frees an abandoned stream's state when the result is dropped"
        );
        con.execute("SELECT 1").expect("next statement");
        return;
    }
    assert!(
        CURSORS.load(Ordering::SeqCst) > 0,
        "DuckDB now frees an abandoned stream's state when the result is dropped"
    );
    con.execute("SELECT 1").expect("next statement");
    assert_eq!(CURSORS.load(Ordering::SeqCst), 0);
}
