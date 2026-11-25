// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `Appender` methods the fifth audit's end-to-end mutation run found
//! untested: a schema-qualified table, `column_type`, `clear_columns` and
//! `append_default_to_chunk` each survived being replaced by a no-op.

use quack_rs::appender::Appender;
use quack_rs::types::TypeId;

use super::Fixture;

#[test]
fn an_appender_writes_to_a_table_in_another_schema() {
    let fx = Fixture::open();
    fx.query("CREATE SCHEMA other");
    fx.query("CREATE TABLE other.t (a INTEGER)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), Some(c"other"), c"t") }.expect("create");
    appender.row(|row| row.append_i32(5)).expect("row");
    appender.close().expect("close");
    let sum = fx.scalar("SELECT sum(a)::BIGINT FROM other.t", |r, i| unsafe {
        r.read_i64(i)
    });
    assert_eq!(sum, Some(5));
}

#[test]
fn column_type_and_clear_columns() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE cols (a INTEGER, b VARCHAR DEFAULT 'x')");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"cols") }.expect("create");
    let id = |i| {
        appender
            .column_type(i)
            // SAFETY: the type is live while `t` is.
            .map(|t| unsafe { t.get_type_id() })
    };
    assert_eq!(id(0), Some(TypeId::Integer));
    assert_eq!(id(1), Some(TypeId::Varchar));
    assert!(appender.column_type(2).is_none());

    appender.add_column(c"a").expect("add_column");
    assert_eq!(appender.column_count(), 1);
    appender.clear_columns().expect("clear_columns");
    assert_eq!(appender.column_count(), 2);
    appender.close().expect("close");
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn append_default_to_chunk_writes_the_columns_default() {
    use quack_rs::query::OwnedDataChunk;
    use quack_rs::types::LogicalType;

    let fx = Fixture::open();
    fx.query("CREATE TABLE defs (a INTEGER DEFAULT 42)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"defs") }.expect("create");
    let chunk = OwnedDataChunk::new(&[LogicalType::new(TypeId::Integer)]).expect("chunk");
    // SAFETY: the chunk has one INTEGER column and room for row 0.
    unsafe { chunk.writer(0).write_i32(0, 0) };
    appender
        .append_default_to_chunk(&chunk, 0, 0)
        .expect("default written");
    // SAFETY: row 0 of an INTEGER column.
    assert_eq!(unsafe { chunk.reader(0).read_i32(0) }, 42);
    appender.close().expect("close");
}

/// QRS-008 (`AUDIT.md` section 16): `DuckDB` 2.0.1 writes a `VARCHAR` default
/// into the chunk without checking the row in a release build, so row 2048 of
/// a 2048-row chunk wrote past the vector and returned success, and row 2^40
/// crashed the process. Both indices are now checked against the chunk before
/// `DuckDB` sees them, and a refused call writes nothing.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn append_default_to_chunk_refuses_a_column_or_row_outside_the_chunk() {
    use quack_rs::query::OwnedDataChunk;
    use quack_rs::types::LogicalType;

    let fx = Fixture::open();
    fx.query("CREATE TABLE defs_s (s VARCHAR DEFAULT 'dflt', i INTEGER DEFAULT 7)");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"defs_s") }.expect("create");
    let chunk = OwnedDataChunk::new(&[
        LogicalType::new(TypeId::Varchar),
        LogicalType::new(TypeId::Integer),
    ])
    .expect("chunk");
    let capacity = u64::try_from(chunk.capacity()).expect("fits");
    let last = capacity - 1;

    for (col, row) in [
        (0, capacity),
        (0, capacity + 1),
        (0, 1 << 40),
        (0, u64::MAX),
        (1, capacity),
        (2, 0),
        (999, 0),
        (u64::MAX, 0),
    ] {
        let err = appender
            .append_default_to_chunk(&chunk, col, row)
            .expect_err(&format!("col {col}, row {row} must be refused"));
        let text = err.to_string();
        let what = if col >= 2 { "column" } else { "row" };
        assert!(
            text.contains(&format!("{what} {}", if col >= 2 { col } else { row })),
            "col {col}, row {row}: {text}"
        );
    }

    // The last row of each column is still written.
    appender
        .append_default_to_chunk(&chunk, 0, last)
        .expect("VARCHAR default in the last row");
    appender
        .append_default_to_chunk(&chunk, 1, last)
        .expect("INTEGER default in the last row");
    let last = usize::try_from(last).expect("fits");
    // SAFETY: `last` is below the chunk's capacity; both columns were written.
    unsafe {
        assert_eq!(chunk.reader(0).read_str(last), "dflt");
        assert_eq!(chunk.reader(1).read_i32(last), 7);
    }
    appender.close().expect("close");
}
