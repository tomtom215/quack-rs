// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ChunkWriter::vector`, `writer` and `struct_writer` (and the
//! `DataChunk::struct_writer` / `into_chunk_writer` they mirror) against a
//! chunk `DuckDB` allocated: a `BIGINT`, a `STRUCT`, a `MAP` and a nullable
//! `INTEGER` column are filled over more than one vector, appended to a table,
//! and read back through SQL.
//!
//! Row `r` of the data (the rows are `0..N`, `N` one more than a vector holds):
//!
//! - `id`: `r`,
//! - `s`: NULL when `r % 7 == 3`, else `{'a': r * 2, 'b': 'b{r}'}` with `a` NULL
//!   when `r % 5 == 1` and `b` NULL when `r % 11 == 4`,
//! - `m`: NULL when `r % 9 == 5`, else `r % 3` entries `'k{r}_{j}' -> r + j`,
//! - `v`: NULL when `r % 10 == 0`, else `r % 1000` as an `INTEGER`.

use quack_rs::appender::Appender;
use quack_rs::chunk_writer::ChunkWriter;
use quack_rs::data_chunk::DataChunk;
use quack_rs::query::OwnedDataChunk;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::complex::MapVector;

use super::vector_support::count;
use super::Fixture;

const TABLE_DDL: &str = "CREATE TABLE t (id BIGINT, s STRUCT(a INTEGER, b VARCHAR), \
                         m MAP(VARCHAR, BIGINT), v INTEGER)";

fn chunk_types() -> [LogicalType; 4] {
    [
        LogicalType::new(TypeId::BigInt),
        LogicalType::struct_type(&[("a", TypeId::Integer), ("b", TypeId::Varchar)]),
        LogicalType::map(TypeId::Varchar, TypeId::BigInt),
        LogicalType::new(TypeId::Integer),
    ]
}

/// Fills `writer`'s chunk from row `first` until it is full or the rows run
/// out, returning the first row not written.
///
/// # Safety
///
/// `writer` must wrap a chunk of `chunk_types()`.
unsafe fn fill(mut writer: ChunkWriter, first: i64, last: i64) -> i64 {
    assert_eq!(writer.column_count(), 4);
    // SAFETY: the columns are BIGINT, STRUCT(2 fields), MAP, INTEGER.
    let mut ids = unsafe { writer.writer(0) };
    let mut structs = unsafe { writer.struct_writer(1, 2) };
    let map = unsafe { writer.vector(2) };
    let mut ints = unsafe { writer.writer(3) };
    let mut map_parent = unsafe { writer.writer(2) };
    assert_eq!(map_parent.as_raw(), map, "writer(c) wraps vector(c)");

    let mut r = first;
    let mut entries = 0_usize;
    while r < last {
        let Some(row) = writer.next_row() else { break };
        unsafe { ids.write_i64(row, r) };

        if r % 7 == 3 {
            unsafe { structs.set_row_null(row) };
        } else {
            if r % 5 == 1 {
                unsafe { structs.set_null(row, 0) };
            } else {
                unsafe { structs.write_i32(row, 0, (r * 2) as i32) };
            }
            if r % 11 == 4 {
                unsafe { structs.set_null(row, 1) };
            } else {
                unsafe { structs.write_varchar(row, 1, &format!("b{r}")) };
            }
        }

        if r % 9 == 5 {
            unsafe { map_parent.set_null(row) };
            unsafe { MapVector::set_entry(map, row, entries as u64, 0) };
        } else {
            let len = (r % 3) as usize;
            unsafe { MapVector::reserve(map, entries + len) };
            let mut keys = unsafe { MapVector::key_writer(map) };
            let mut values = unsafe { MapVector::value_writer(map) };
            unsafe { MapVector::set_entry(map, row, entries as u64, len as u64) };
            for j in 0..len {
                unsafe { keys.write_varchar(entries + j, &format!("k{r}_{j}")) };
                unsafe { values.write_i64(entries + j, r + j as i64) };
            }
            entries += len;
            unsafe { MapVector::set_size(map, entries) };
        }

        if r % 10 == 0 {
            unsafe { ints.set_null(row) };
        } else {
            unsafe { ints.write_i32(row, (r % 1000) as i32) };
        }
        r += 1;
    }
    // Dropping the writer sets the chunk's size to the rows handed out.
    r
}

#[test]
fn chunk_writer_columns_fill_a_chunk_that_is_appended_and_read_back() {
    let fx = Fixture::open();
    fx.query(TABLE_DDL);
    let vector_size = quack_rs::vector::vector_size() as i64;
    let total = vector_size + 1;

    let mut chunk = OwnedDataChunk::new(&chunk_types()).expect("chunk");
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"t") }.expect("appender");

    // The first chunk fills up: `next_row` stops at the capacity.
    // SAFETY: the chunk has the types `fill` expects, and is writable.
    let next = unsafe { fill(ChunkWriter::new(chunk.as_raw()), 0, total) };
    assert_eq!(next, vector_size, "the first chunk holds a full vector");
    assert_eq!(
        chunk.size() as i64,
        vector_size,
        "dropping the writer sets the size"
    );
    appender
        .append_chunk(&chunk)
        .expect("append the full chunk");

    // The last row goes into a second chunk, through `DataChunk`.
    chunk.reset();
    assert_eq!(chunk.size(), 0);
    // SAFETY: as above; `DataChunk::from_raw` views the same live chunk.
    let next = unsafe {
        fill(
            DataChunk::from_raw(chunk.as_raw()).into_chunk_writer(),
            next,
            total,
        )
    };
    assert_eq!(next, total);
    assert_eq!(chunk.size(), 1);
    appender.append_chunk(&chunk).expect("append the last row");
    appender.close().expect("close");

    let ids = |sql: &str| count(&fx, sql);
    assert_eq!(ids("SELECT count(*) FROM t"), total);
    assert_eq!(ids("SELECT count(DISTINCT id) FROM t"), total);
    assert_eq!(
        ids("SELECT sum(id)::BIGINT FROM t"),
        total * (total - 1) / 2
    );

    // `s`: the struct column, through `struct_writer`.
    let model_s = "CASE WHEN id % 7 = 3 THEN NULL ELSE {\
        'a': CASE WHEN id % 5 = 1 THEN NULL ELSE (id * 2)::INTEGER END, \
        'b': CASE WHEN id % 11 = 4 THEN NULL ELSE 'b' || id END} END";
    assert_eq!(
        ids(&format!(
            "SELECT count(*) FROM t WHERE s IS DISTINCT FROM ({model_s})"
        )),
        0,
        "struct column"
    );
    let want_null_structs = (0..total).filter(|r| r % 7 == 3).count() as i64;
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE s IS NULL"),
        want_null_structs
    );
    // Fields of the NULL rows are NULL too (`set_row_null` clears them).
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE s.a IS NOT NULL AND id % 7 = 3"),
        0
    );
    let want_null_a = (0..total).filter(|r| r % 7 != 3 && r % 5 == 1).count() as i64;
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE s IS NOT NULL AND s.a IS NULL"),
        want_null_a
    );
    let want_null_b = (0..total).filter(|r| r % 7 != 3 && r % 11 == 4).count() as i64;
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE s IS NOT NULL AND s.b IS NULL"),
        want_null_b
    );

    // `m`: the map column, through `vector` and the `MapVector` calls.
    let model_m = "CASE WHEN id % 9 = 5 THEN NULL ELSE \
        map(list_transform(range(id % 3), lambda j: 'k' || id || '_' || j), \
            list_transform(range(id % 3), lambda j: id + j)) END";
    assert_eq!(
        ids(&format!(
            "SELECT count(*) FROM t WHERE m IS DISTINCT FROM ({model_m})"
        )),
        0,
        "map column"
    );
    let want_entries: i64 = (0..total).filter(|r| r % 9 != 5).map(|r| r % 3).sum();
    assert_eq!(
        ids("SELECT coalesce(sum(cardinality(m)), 0)::BIGINT FROM t"),
        want_entries
    );
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE m IS NULL"),
        (0..total).filter(|r| r % 9 == 5).count() as i64
    );

    // `v`: a nullable column, through `writer`.
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE v IS DISTINCT FROM \
             CASE WHEN id % 10 = 0 THEN NULL ELSE (id % 1000)::INTEGER END"),
        0,
        "integer column"
    );
    assert_eq!(
        ids("SELECT count(*) FROM t WHERE v IS NULL"),
        (0..total).filter(|r| r % 10 == 0).count() as i64
    );
}

/// `DataChunk::struct_writer` is `ChunkWriter::struct_writer` for a chunk a
/// callback was handed: both fill the same STRUCT column.
#[test]
fn data_chunk_struct_writer_matches_chunk_writer_struct_writer() {
    let fx = Fixture::open();
    fx.query("CREATE TABLE u (s STRUCT(a INTEGER, b VARCHAR))");
    let ty = [LogicalType::struct_type(&[
        ("a", TypeId::Integer),
        ("b", TypeId::Varchar),
    ])];
    let chunk = OwnedDataChunk::new(&ty).expect("chunk");
    // SAFETY: column 0 is a two-field STRUCT; rows 0..4 are within capacity.
    unsafe {
        let mut via_data_chunk = chunk.struct_writer(0, 2);
        via_data_chunk.write_i32(0, 0, 10);
        via_data_chunk.write_varchar(0, 1, "from DataChunk");
        via_data_chunk.write_i32(1, 0, 11);
        via_data_chunk.set_null(1, 1);
        via_data_chunk.set_row_null(2);

        let writer = ChunkWriter::with_capacity(chunk.as_raw(), 4);
        let mut via_chunk_writer = writer.struct_writer(0, 2);
        via_chunk_writer.write_i32(3, 0, 13);
        via_chunk_writer.write_varchar(3, 1, "from ChunkWriter, longer than twelve");
        writer.finish_with_size(4);
    }
    assert_eq!(chunk.size(), 4);
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"u") }.expect("appender");
    appender.append_chunk(&chunk).expect("append");
    appender.close().expect("close");

    let rows = super::vector_support::text_rows(
        &fx,
        "SELECT (s IS NULL)::VARCHAR, s.a::VARCHAR, s.b FROM u ORDER BY s.a NULLS LAST",
    );
    let cells: Vec<Vec<Option<&str>>> = rows
        .iter()
        .map(|r| r.iter().map(Option::as_deref).collect())
        .collect();
    assert_eq!(
        cells,
        [
            vec![Some("false"), Some("10"), Some("from DataChunk")],
            vec![Some("false"), Some("11"), None],
            vec![
                Some("false"),
                Some("13"),
                Some("from ChunkWriter, longer than twelve")
            ],
            vec![Some("true"), None, None],
        ]
    );
}
