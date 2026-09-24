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
