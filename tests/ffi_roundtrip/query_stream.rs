// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A streaming result that stops early must not look like a finished one.
//!
//! `duckdb_fetch_chunk` returns null both at the end of the rows and when a
//! `Fetch` fails; on failure it records the error on the result
//! (`QueryResult::SetError`). `next_chunk` used to return `None` in both
//! cases, so a query failing part-way through a stream — or a stream
//! invalidated by another statement on the connection — read as a complete,
//! shorter result (audit F-V4: 1,871,872 of 3,000,000 rows, then a clean
//! `None`).

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

/// Running another statement on the connection invalidates the stream.
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
    con.query("SELECT 1").expect("another statement");
    let err = result
        .next_chunk()
        .expect_err("the stream was invalidated, not finished");
    assert!(err.to_string().contains("closed"), "{err}");
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

/// `DuckDB` keeps an abandoned stream's operator states on the connection
/// until the next statement there, not until the `QueryResult` or the
/// `PreparedStatement` is dropped. The documented behaviour (Known
/// Limitations, "An abandoned stream keeps its table-function state") is
/// pinned here: if `DuckDB` starts freeing them on drop, this fails and the
/// page needs updating.
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
    assert!(
        CURSORS.load(Ordering::SeqCst) > 0,
        "DuckDB now frees an abandoned stream's state when the result is dropped"
    );
    con.execute("SELECT 1").expect("next statement");
    assert_eq!(CURSORS.load(Ordering::SeqCst), 0);
}
