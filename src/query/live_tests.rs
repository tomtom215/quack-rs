// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use crate::testing::InMemoryDb;
use crate::types::TypeId;

/// Opens a raw connection against a fresh in-memory database.
///
/// Returns the database handle too: it must outlive the connection.
unsafe fn open_raw() -> (libduckdb_sys::duckdb_database, duckdb_connection) {
    let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
    let mut con: duckdb_connection = std::ptr::null_mut();
    // SAFETY: standard open/connect sequence against an in-memory database.
    unsafe {
        assert_eq!(
            libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
            DuckDBSuccess
        );
        assert_eq!(
            libduckdb_sys::duckdb_connect(db, &raw mut con),
            DuckDBSuccess
        );
    }
    (db, con)
}

/// Closes what `open_raw` produced.
///
/// # Safety
///
/// `con` and `db` must come from `open_raw` and not have been closed.
unsafe fn close_raw(mut db: libduckdb_sys::duckdb_database, mut con: duckdb_connection) {
    unsafe {
        libduckdb_sys::duckdb_disconnect(&raw mut con);
        libduckdb_sys::duckdb_close(&raw mut db);
    }
}

#[test]
fn query_reads_scalar_results() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let mut result =
        unsafe { query(con, "SELECT 42 AS answer, 'hi' AS greeting") }.expect("query succeeds");
    assert_eq!(result.column_count(), 2);
    assert_eq!(result.column_name(0).as_deref(), Some("answer"));
    assert_eq!(result.column_name(1).as_deref(), Some("greeting"));
    assert_eq!(result.column_type(0), Some(TypeId::Integer));
    assert_eq!(result.column_type(1), Some(TypeId::Varchar));
    assert_eq!(result.column_name(2), None);
    assert_eq!(result.column_type(2), None);

    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    assert_eq!(chunk.size(), 1);
    // SAFETY: column 0 is INTEGER, column 1 is VARCHAR, row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_i32(0), 42);
        assert_eq!(chunk.reader(1).read_str(0), "hi");
    }
    drop(chunk);
    assert!(result.next_chunk().expect("fetch").is_none());

    drop(result);
    unsafe { close_raw(db, con) };
}

#[test]
fn query_surfaces_duckdb_errors() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let err =
        unsafe { query(con, "SELECT * FROM no_such_table") }.expect_err("missing table must fail");
    assert!(err.as_str().contains("no_such_table"), "{err}");

    // The connection is still usable after a failed query.
    let ok = unsafe { query(con, "SELECT 1") };
    assert!(ok.is_ok());

    unsafe { close_raw(db, con) };
}

#[test]
fn execute_reports_rows_changed() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    unsafe { execute(con, "CREATE TABLE t(i INTEGER)") }.expect("create");
    let changed = unsafe { execute(con, "INSERT INTO t VALUES (1), (2), (3)") }.expect("insert");
    assert_eq!(changed, 3);

    unsafe { close_raw(db, con) };
}

#[test]
fn multi_chunk_results_are_fully_drained() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    // More rows than one vector holds, so DuckDB returns several chunks.
    let rows = crate::vector::vector_size() * 3 + 7;
    let sql = format!("SELECT i FROM range({rows}) t(i)");
    let mut result = unsafe { query(con, &sql) }.expect("range query");

    let mut seen: u64 = 0;
    let mut chunks = 0;
    while let Some(chunk) = result.next_chunk().expect("fetch") {
        chunks += 1;
        for row in 0..chunk.size() {
            // SAFETY: `range()` yields BIGINT, and `row` is in bounds.
            assert_eq!(
                unsafe { chunk.reader(0).read_i64(row) },
                i64::try_from(seen).expect("row counter fits in i64")
            );
            seen += 1;
        }
    }
    assert_eq!(seen, rows);
    assert!(chunks > 1, "expected several chunks, got {chunks}");

    drop(result);
    unsafe { close_raw(db, con) };
}

#[test]
fn prepared_statements_bind_and_execute() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let stmt = unsafe { prepare(con, "SELECT ? + ?") }.expect("prepare");
    assert_eq!(stmt.parameter_count(), 2);
    stmt.bind_i64(1, 20).expect("bind 1");
    stmt.bind_i64(2, 22).expect("bind 2");
    let mut result = stmt.execute().expect("execute");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    // SAFETY: the expression yields BIGINT and row 0 exists.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 42);

    drop(chunk);
    drop(result);
    drop(stmt);
    unsafe { close_raw(db, con) };
}

#[test]
fn prepared_statements_are_reusable_after_clearing() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let stmt = unsafe { prepare(con, "SELECT ?::BIGINT * 2") }.expect("prepare");
    for input in [1_i64, 7, 100] {
        stmt.clear_bindings().expect("clear");
        stmt.bind_i64(1, input).expect("bind");
        let mut result = stmt.execute().expect("execute");
        let chunk = result.next_chunk().expect("fetch").expect("one chunk");
        // SAFETY: the expression yields BIGINT and row 0 exists.
        assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, input * 2);
    }

    drop(stmt);
    unsafe { close_raw(db, con) };
}

#[test]
fn binding_a_string_avoids_sql_injection() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    unsafe { execute(con, "CREATE TABLE t(s VARCHAR)") }.expect("create");
    let stmt = unsafe { prepare(con, "INSERT INTO t VALUES (?)") }.expect("prepare");
    // Text that would be catastrophic if interpolated into SQL.
    stmt.bind_str(1, "'); DROP TABLE t; --").expect("bind");
    stmt.execute().expect("insert");
    drop(stmt);

    let mut result = unsafe { query(con, "SELECT s FROM t") }.expect("select");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    assert_eq!(chunk.size(), 1);
    // SAFETY: column 0 is VARCHAR and row 0 exists.
    assert_eq!(
        unsafe { chunk.reader(0).read_str(0) },
        "'); DROP TABLE t; --"
    );

    drop(chunk);
    drop(result);
    unsafe { close_raw(db, con) };
}

#[test]
fn named_parameters_resolve_by_name() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let stmt = unsafe { prepare(con, "SELECT $needle::BIGINT") }.expect("prepare");
    let index = stmt.parameter_index("needle").expect("named parameter");
    assert_eq!(stmt.parameter_name(index).as_deref(), Some("needle"));
    assert_eq!(stmt.parameter_index("nope"), None);
    stmt.bind_i64(index, 5).expect("bind");
    let mut result = stmt.execute().expect("execute");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    // SAFETY: the expression yields BIGINT and row 0 exists.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 5);

    drop(chunk);
    drop(result);
    drop(stmt);
    unsafe { close_raw(db, con) };
}

#[test]
fn prepare_surfaces_parse_errors() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    let err = unsafe { prepare(con, "SELECT FROM WHERE") }.expect_err("syntax error");
    assert_ne!(err.as_str(), "");

    unsafe { close_raw(db, con) };
}

#[test]
fn null_and_blob_bindings_round_trip() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let (db, con) = unsafe { open_raw() };

    unsafe { execute(con, "CREATE TABLE t(b BLOB, n INTEGER)") }.expect("create");
    let stmt = unsafe { prepare(con, "INSERT INTO t VALUES (?, ?)") }.expect("prepare");
    stmt.bind_blob(1, &[0x00, 0xFF, 0x80]).expect("bind blob");
    stmt.bind_null(2).expect("bind null");
    stmt.execute().expect("insert");
    drop(stmt);

    let mut result = unsafe { query(con, "SELECT b, n FROM t") }.expect("select");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    // SAFETY: column 0 is BLOB, column 1 is INTEGER, row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_blob(0), &[0x00, 0xFF, 0x80]);
        assert!(!chunk.reader(1).is_valid(0));
    }

    drop(chunk);
    drop(result);
    unsafe { close_raw(db, con) };
}

#[test]
fn owned_connection_outlives_the_database_handle() {
    let _guard = InMemoryDb::open().expect("dispatch table");
    let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
    // SAFETY: standard open against an in-memory database.
    unsafe {
        assert_eq!(
            libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
            DuckDBSuccess
        );
    }
    // SAFETY: `db` was just opened.
    let con = unsafe { OwnedConnection::open(db) }.expect("connect");

    // Close the database handle, mirroring what happens when extension
    // loading finishes and DuckDB drops its DatabaseWrapper. The connection
    // holds its own reference, so it stays usable.
    // SAFETY: `db` is closed exactly once; `con` keeps the instance alive.
    unsafe { libduckdb_sys::duckdb_close(&raw mut db) };

    con.execute("CREATE TABLE t(i INTEGER)").expect("create");
    con.execute("INSERT INTO t VALUES (1), (2)")
        .expect("insert");
    let mut result = con.query("SELECT count(*) FROM t").expect("count");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    // SAFETY: count(*) yields BIGINT and row 0 exists.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 2);
}
