// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Statement metadata before execution, and the `from_raw` / `into_raw`
//! escape hatches of the query owners (eighth review: the apigaps P1
//! candidates and APIGAPS-9).

use quack_rs::query::{OwnedConnection, PreparedStatement, QueryResult, StatementType};
use quack_rs::types::TypeId;

use super::Fixture;

fn owned(fx: &Fixture) -> OwnedConnection {
    // SAFETY: the fixture's database is open for the whole test.
    unsafe { OwnedConnection::open(fx.db()) }.expect("connect")
}

/// Whether the engine is at least `major.minor.patch`.
fn engine_at_least(version: (u64, u64, u64)) -> bool {
    // SAFETY: the fixture initialised the C API.
    let engine = unsafe { quack_rs::abi::engine_version() }.expect("version");
    quack_rs::abi::parse_version(&engine).is_some_and(|v| v >= version)
}

#[test]
fn prepared_statements_and_results_report_their_statement_type() {
    let fx = Fixture::open();
    let con = owned(&fx);
    con.execute("CREATE TABLE qm(a INTEGER, b VARCHAR)")
        .expect("create");
    for (sql, expected) in [
        ("SELECT 1", StatementType::Select),
        ("FROM qm", StatementType::Select),
        ("INSERT INTO qm VALUES (1, 'x')", StatementType::Insert),
        ("UPDATE qm SET a = 2", StatementType::Update),
        ("DELETE FROM qm", StatementType::Delete),
        ("CREATE TABLE qm2(x INTEGER)", StatementType::Create),
        ("DROP TABLE IF EXISTS qm2", StatementType::Drop),
        ("EXPLAIN SELECT 1", StatementType::Explain),
        ("SET threads = 2", StatementType::Set),
        ("BEGIN TRANSACTION", StatementType::Transaction),
    ] {
        let stmt = con.prepare(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        assert_eq!(stmt.statement_type(), expected, "prepared {sql}");
    }
    // `MERGE INTO` has a C enum value from 1.5.5; before it the C API
    // reports the statement as INVALID.
    let merge = con
        .prepare("MERGE INTO qm USING (SELECT 1 AS a) s ON qm.a = s.a WHEN MATCHED THEN DELETE")
        .expect("prepare MERGE");
    let expected = if engine_at_least((1, 5, 5)) {
        StatementType::MergeInto
    } else {
        StatementType::Invalid
    };
    assert_eq!(merge.statement_type(), expected);

    for (sql, expected) in [
        ("SELECT 42", StatementType::Select),
        ("INSERT INTO qm VALUES (3, 'y')", StatementType::Insert),
        ("CREATE TABLE qm3(x INTEGER)", StatementType::Create),
    ] {
        let result = con.query(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        assert_eq!(result.statement_type(), expected, "result of {sql}");
    }
}

/// A parameter whose type the binder infers reads as that type; one it
/// cannot (`b || $1`) and an index out of range read as `None`. Before this
/// the type was reachable only through raw FFI.
#[test]
fn parameter_types_are_inferred_or_none() {
    let fx = Fixture::open();
    let con = owned(&fx);
    con.execute("CREATE TABLE qp(a INTEGER, b VARCHAR, d DATE)")
        .expect("create");
    let stmt = con
        .prepare("SELECT a, b || $1 AS bb FROM qp WHERE a > $2 AND d = $3")
        .expect("prepare");
    assert_eq!(stmt.parameter_count(), 3);
    let type_of = |i| {
        stmt.parameter_type(i)
            // SAFETY: the C API is initialised.
            .and_then(|t| unsafe { t.try_get_type_id() })
    };
    assert_eq!(type_of(1), None, "`b || $1` leaves the type open");
    assert_eq!(type_of(2), Some(TypeId::Integer));
    assert_eq!(type_of(3), Some(TypeId::Date));
    assert!(stmt.parameter_type(0).is_none(), "parameters are 1-based");
    assert!(stmt.parameter_type(4).is_none());
}

/// `columns` is the result shape before execution, `None` while it depends
/// on unbound parameters (where `DuckDB` reports one placeholder column).
#[cfg(feature = "duckdb-1-5")]
#[test]
fn prepared_columns_are_known_before_execution() {
    let fx = Fixture::open();
    let con = owned(&fx);
    con.execute("CREATE TABLE qc(a INTEGER, b VARCHAR)")
        .expect("create");
    let shape = |sql: &str| {
        con.prepare(sql)
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .columns()
            .map(|cols| {
                cols.into_iter()
                    // SAFETY: the C API is initialised.
                    .map(|(name, ty)| (name, unsafe { ty.try_get_type_id() }))
                    .collect::<Vec<_>>()
            })
    };
    assert_eq!(
        shape("SELECT a, b AS label, a + 1.5 AS x FROM qc WHERE a > $1"),
        Some(vec![
            ("a".to_owned(), Some(TypeId::Integer)),
            ("label".to_owned(), Some(TypeId::Varchar)),
            ("x".to_owned(), Some(TypeId::Decimal)),
        ])
    );
    assert_eq!(
        shape("INSERT INTO qc VALUES (1, 'x')"),
        Some(vec![("Count".to_owned(), Some(TypeId::BigInt))])
    );
    assert_eq!(shape("SELECT b || $1 AS bb FROM qc"), None, "depends on $1");
    assert_eq!(shape("SELECT $1"), None);
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn table_names_lists_the_tables_a_query_touches() {
    let fx = Fixture::open();
    let con = owned(&fx);
    con.execute("CREATE TABLE ta(x INTEGER); CREATE TABLE tb(x INTEGER)")
        .expect("create");
    let join = "SELECT * FROM ta JOIN main.tb ON ta.x = tb.x";
    let mut names = con.table_names(join, false).expect("names");
    names.sort();
    assert_eq!(names, ["ta", "tb"]);
    let mut qualified = con.table_names(join, true).expect("names");
    qualified.sort();
    assert_eq!(qualified, ["main.tb", "ta"]);
    assert_eq!(
        con.table_names("SELECT 1", false).expect("no tables"),
        Vec::<String>::new()
    );
    // Names only: the tables need not exist.
    assert_eq!(
        con.table_names("SELECT * FROM not_created_yet", false)
            .expect("names"),
        ["not_created_yet"]
    );
    // The documented failures.
    assert!(con.table_names("SELEC nonsense", false).is_err());
    assert!(
        con.table_names("SELECT 1; SELECT 2", false).is_err(),
        "one statement only"
    );
    assert!(
        con.table_names("SELECT * FROM ta JOIN tb USING (x)", false)
            .is_err(),
        "USING needs the columns, which the names-only bind does not resolve"
    );
    let err = con
        .table_names("SELECT 1\0", false)
        .expect_err("interior NUL");
    assert!(err.as_str().contains("NUL"), "{err}");
    // The free function, on the raw connection.
    // SAFETY: the fixture's connection is open.
    let names = unsafe { quack_rs::query::table_names(fx.con(), "FROM ta", false) }.expect("names");
    assert_eq!(names, ["ta"]);
}

/// `into_raw` then `from_raw` hands the same handle back to RAII, and a raw
/// handle from the C API can be adopted: each one is used, then destroyed
/// exactly once by its owner's `Drop` (a double destroy would crash, a
/// missed one leaks under the `LeakSanitizer` job).
#[test]
fn query_owners_round_trip_through_raw_handles() {
    let fx = Fixture::open();
    let con = owned(&fx);

    // PreparedStatement.
    let stmt = con.prepare("SELECT 40 + 2").expect("prepare");
    let raw = stmt.into_raw();
    // SAFETY: `raw` came from `into_raw` and is owned by nothing else.
    let stmt = unsafe { PreparedStatement::from_raw(raw) };
    let mut result = stmt.execute().expect("execute");
    let chunk = result.next_chunk().expect("fetch").expect("a row");
    // SAFETY: one INTEGER row.
    assert_eq!(unsafe { chunk.reader(0).read_i32(0) }, 42);
    drop((chunk, result, stmt));

    // A statement prepared by the raw C call.
    let mut raw: libduckdb_sys::duckdb_prepared_statement = std::ptr::null_mut();
    // SAFETY: the connection is open; the out-pointer is valid.
    let state = unsafe {
        libduckdb_sys::duckdb_prepare(con.as_raw(), c"SELECT ?::INTEGER".as_ptr(), &raw mut raw)
    };
    assert_eq!(state, libduckdb_sys::DuckDBSuccess);
    // SAFETY: a fresh statement owned by nothing else.
    let stmt = unsafe { PreparedStatement::from_raw(raw) };
    assert_eq!(stmt.parameter_count(), 1);
    assert_eq!(stmt.statement_type(), StatementType::Select);

    // A failed prepare adopted through `from_raw`: every method reports
    // failure and nothing crashes (on 1.x `duckdb_prepared_statement_type`
    // would dereference the missing statement data).
    let mut raw: libduckdb_sys::duckdb_prepared_statement = std::ptr::null_mut();
    // SAFETY: as above.
    let state =
        unsafe { libduckdb_sys::duckdb_prepare(con.as_raw(), c"SELEC 1".as_ptr(), &raw mut raw) };
    assert_ne!(state, libduckdb_sys::DuckDBSuccess);
    // SAFETY: DuckDB allocates the statement even on failure; owned by nothing else.
    let failed = unsafe { PreparedStatement::from_raw(raw) };
    assert_eq!(failed.statement_type(), StatementType::Invalid);
    assert_eq!(failed.parameter_count(), 0);
    assert!(failed.execute().is_err());
    #[cfg(feature = "duckdb-1-5")]
    assert!(failed.columns().is_none());

    // QueryResult.
    let result = con.query("SELECT 'kept' AS s").expect("query");
    let raw = result.into_raw();
    // SAFETY: `raw` came from `into_raw` and is owned by nothing else.
    let mut result = unsafe { QueryResult::from_raw(raw) };
    assert_eq!(result.column_name(0).as_deref(), Some("s"));
    let chunk = result.next_chunk().expect("fetch").expect("a row");
    // SAFETY: one VARCHAR row.
    assert_eq!(unsafe { chunk.reader(0).read_str(0) }, "kept");
    drop(chunk);
    assert!(result.next_chunk().expect("end").is_none());

    // OwnedConnection.
    let raw = con.into_raw();
    // SAFETY: `raw` came from `into_raw` and is owned by nothing else.
    let con = unsafe { OwnedConnection::from_raw(raw) };
    assert_eq!(con.execute("SELECT 1").expect("still open"), 0);
    let mut fresh: libduckdb_sys::duckdb_connection = std::ptr::null_mut();
    // SAFETY: the database is open.
    assert_eq!(
        unsafe { libduckdb_sys::duckdb_connect(fx.db(), &raw mut fresh) },
        libduckdb_sys::DuckDBSuccess
    );
    // SAFETY: a fresh connection owned by nothing else.
    let adopted = unsafe { OwnedConnection::from_raw(fresh) };
    assert!(adopted.query("SELECT 1").is_ok());
}
