// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `OwnedDataChunk::new`, `capacity` and `reset` against a live `DuckDB`:
//! a chunk built in Rust and appended to a table reads back the same rows.

use quack_rs::appender::Appender;
use quack_rs::query::OwnedDataChunk;
use quack_rs::types::{LogicalType, TypeId};

use super::Fixture;

#[test]
fn a_chunk_built_with_new_appends_its_rows() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE t (n BIGINT, s VARCHAR)");
    let chunk = OwnedDataChunk::new(&[
        LogicalType::new(TypeId::BigInt),
        LogicalType::new(TypeId::Varchar),
    ])
    .expect("chunk");
    assert_eq!(chunk.column_count(), 2);
    assert_eq!(chunk.size(), 0);
    // SAFETY: column 0 is BIGINT, column 1 VARCHAR, rows 0..3 are within capacity.
    unsafe {
        let mut n = chunk.writer(0);
        let mut s = chunk.writer(1);
        for (row, word) in ["a", "a string longer than twelve bytes", "c"]
            .iter()
            .enumerate()
        {
            n.write_i64(row, i64::try_from(row).unwrap() + 10);
            s.write_varchar(row, word);
        }
        chunk.set_size(3);
    }
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"t") }.expect("appender");
    appender.append_chunk(&chunk).expect("append_chunk");
    appender.close().expect("close");

    let sum = fx.scalar("SELECT sum(n)::BIGINT FROM t", |r, i| unsafe {
        r.read_i64(i)
    });
    assert_eq!(sum, Some(33));
    let joined = fx.scalar(
        "SELECT string_agg(s, '|' ORDER BY n) FROM t",
        |r, i| unsafe { r.read_str(i).to_owned() },
    );
    assert_eq!(
        joined.as_deref(),
        Some("a|a string longer than twelve bytes|c")
    );
}

#[test]
fn capacity_is_the_engines_vector_size_and_reset_empties_the_chunk() {
    let _fx = Fixture::open();
    let mut chunk = OwnedDataChunk::new(&[LogicalType::new(TypeId::Integer)]).expect("chunk");
    assert_eq!(chunk.capacity() as u64, quack_rs::vector::vector_size());
    assert!(chunk.capacity() > 0);
    // SAFETY: column 0 is INTEGER and row 0 is within capacity.
    unsafe {
        chunk.writer(0).write_i32(0, 7);
        chunk.set_size(1);
    }
    assert_eq!(chunk.size(), 1);
    chunk.reset();
    assert_eq!(chunk.size(), 0);
    assert_eq!(chunk.column_count(), 1);
}

#[test]
fn new_refuses_no_columns_and_an_any_column() {
    let _fx = Fixture::open();
    let empty = OwnedDataChunk::new(&[]).expect_err("no columns");
    assert!(empty.to_string().contains("at least one column"), "{empty}");
    let any = OwnedDataChunk::new(&[LogicalType::new(TypeId::Any)]).expect_err("ANY");
    assert!(any.to_string().contains("ANY"), "{any}");
}
