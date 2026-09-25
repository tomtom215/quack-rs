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
    let int = LogicalType::new(TypeId::Integer);
    let mut types = [int.as_raw()];
    // SAFETY: one live logical type; DuckDB copies it and returns an owned chunk.
    let chunk = unsafe {
        OwnedDataChunk::from_raw(libduckdb_sys::duckdb_create_data_chunk(
            types.as_mut_ptr(),
            1,
        ))
    };
    // SAFETY: the chunk has one INTEGER column and room for row 0.
    unsafe { chunk.writer(0).write_i32(0, 0) };
    appender
        .append_default_to_chunk(&chunk, 0, 0)
        .expect("default written");
    // SAFETY: row 0 of an INTEGER column.
    assert_eq!(unsafe { chunk.reader(0).read_i32(0) }, 42);
    appender.close().expect("close");
}
