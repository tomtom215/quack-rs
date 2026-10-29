// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The `MapVector` write workflow — `reserve`, `key_writer`, `value_writer`,
//! `set_entry`, `set_size` — and its read-side handles (`struct_child`,
//! `keys`, `values`, `get_entry`, `total_entry_count`), against a live
//! `DuckDB`. `ListBuilder::push_map_row` is the safe way to do this; these
//! tests drive the raw calls it is built from.
//!
//! The function under test turns a `BIGINT` x into a `MAP(VARCHAR, BIGINT)`:
//!
//! - `x % 7 == 3`: a NULL map,
//! - `x % 5 == 0` (and not the above): an empty map,
//! - otherwise `x % 4 + 1` entries, key `k{j}` for even `j` and a key longer
//!   than a string's 12 inline bytes for odd `j`, value `x * 100 + j` — NULL
//!   when `(x + j) % 3 == 0`.

use libduckdb_sys::duckdb_vector;
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::complex::{MapVector, StructVector};
use quack_rs::vector::{VectorReader, VectorWriter};

use super::vector_support::{count, edge_rows, text_rows};
use super::Fixture;

fn map_type() -> LogicalType {
    LogicalType::map(TypeId::Varchar, TypeId::BigInt)
}

/// The key of entry `j` of the map for `x`.
fn key_of(x: i64, j: usize) -> String {
    if j % 2 == 0 {
        format!("k{j}")
    } else {
        format!("a longer key number {x}-{j}")
    }
}

/// How many entries the map for `x` has, or `None` for a NULL map.
const fn entries_of(x: i64) -> Option<usize> {
    if x % 7 == 3 {
        None
    } else if x % 5 == 0 {
        Some(0)
    } else {
        Some((x % 4) as usize + 1)
    }
}

/// The value of entry `j` of the map for `x`; `None` is a NULL value.
const fn value_of(x: i64, j: usize) -> Option<i64> {
    if (x + j as i64) % 3 == 0 {
        None
    } else {
        Some(x * 100 + j as i64)
    }
}

/// The SQL model of the map for `x` (a column named `x`).
const MODEL: &str = "CASE WHEN x % 7 = 3 THEN NULL \
    WHEN x % 5 = 0 THEN map([]::VARCHAR[], []::BIGINT[]) \
    ELSE map(\
      list_transform(range(x % 4 + 1), lambda j: \
        CASE WHEN j % 2 = 0 THEN 'k' || j ELSE 'a longer key number ' || x || '-' || j END), \
      list_transform(range(x % 4 + 1), lambda j: \
        CASE WHEN (x + j) % 3 = 0 THEN NULL ELSE x * 100 + j END)) END";

fn register(fx: &Fixture, name: &str, param: TypeId, ret: LogicalType, callback: ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(param)
            .returns_logical(ret)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

// One `reserve` for the whole chunk, then one pass.
quack_rs::scalar_callback!(build_map, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let total: usize = (0..rows)
        .map(|row| entries_of(unsafe { reader.read_i64(row) }).unwrap_or(0))
        .sum();
    unsafe { MapVector::reserve(output, total) };
    let mut keys = unsafe { MapVector::key_writer(output) };
    let mut values = unsafe { MapVector::value_writer(output) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut at = 0_usize;
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        let Some(len) = entries_of(x) else {
            unsafe { parent.set_null(row) };
            unsafe { MapVector::set_entry(output, row, at as u64, 0) };
            continue;
        };
        unsafe { MapVector::set_entry(output, row, at as u64, len as u64) };
        for j in 0..len {
            unsafe { keys.write_varchar(at + j, &key_of(x, j)) };
            match value_of(x, j) {
                Some(v) => unsafe { values.write_i64(at + j, v) },
                None => unsafe { values.set_null(at + j) },
            }
        }
        at += len;
    }
    assert_eq!(at, total);
    unsafe { MapVector::set_size(output, at) };
    // What the engine now reports for the map's size is what was set.
    assert_eq!(unsafe { MapVector::total_entry_count(output) }, at);
});

// The same map, grown a row at a time: every row reserves what it needs, so
// the child is reallocated many times and the writers are fetched again after
// each one. Entries written before a reallocation must survive it.
quack_rs::scalar_callback!(build_map_growing, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut at = 0_usize;
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        let Some(len) = entries_of(x) else {
            unsafe { parent.set_null(row) };
            unsafe { MapVector::set_entry(output, row, at as u64, 0) };
            continue;
        };
        unsafe { MapVector::reserve(output, at + len) };
        let mut keys = unsafe { MapVector::key_writer(output) };
        let mut values = unsafe { MapVector::value_writer(output) };
        unsafe { MapVector::set_entry(output, row, at as u64, len as u64) };
        for j in 0..len {
            unsafe { keys.write_varchar(at + j, &key_of(x, j)) };
            match value_of(x, j) {
                Some(v) => unsafe { values.write_i64(at + j, v) },
                None => unsafe { values.set_null(at + j) },
            }
        }
        at += len;
        // `set_size` after every row, as `ListBuilder` does for a finished row.
        unsafe { MapVector::set_size(output, at) };
        assert_eq!(
            unsafe { MapVector::total_entry_count(output) },
            at,
            "row {row}"
        );
    }
});

fn check_map_function(fx: &Fixture, function: &str, n: usize) {
    let table = format!("(SELECT x, {MODEL} AS want FROM range({n}) t(x))");

    // Whole map, DuckDB's comparison.
    let differing = count(
        fx,
        &format!("SELECT count(*) FROM {table} WHERE {function}(x) IS DISTINCT FROM want"),
    );
    assert_eq!(differing, 0, "{function} n={n}: maps differ");

    // Materialised into a table, which copies the child by the size `set_size`
    // recorded rather than by following the row entries.
    fx.query(&format!(
        "CREATE OR REPLACE TEMP TABLE got AS SELECT x, {function}(x) AS v FROM range({n}) t(x)"
    ));
    assert_eq!(count(fx, "SELECT count(*) FROM got"), n as i64);
    let stored_differ = count(
        fx,
        &format!(
            "SELECT count(*) FROM got JOIN {table} w USING (x) WHERE got.v IS DISTINCT FROM w.want"
        ),
    );
    assert_eq!(stored_differ, 0, "{function} n={n}: stored maps differ");

    // Rendered, row by row: entry order and NULL values included.
    let rows = text_rows(
        fx,
        &format!("SELECT {function}(x)::VARCHAR, want::VARCHAR FROM {table} ORDER BY x"),
    );
    assert_eq!(rows.len(), n);
    for (x, row) in rows.iter().enumerate() {
        assert_eq!(row[0], row[1], "{function} n={n} x={x}");
        assert_eq!(row[0].is_none(), entries_of(x as i64).is_none(), "x={x}");
    }

    // Keys and values separately, and the size, so a failure names the half.
    let keys_differ = count(
        fx,
        &format!(
            "SELECT count(*) FROM {table} WHERE map_keys({function}(x)) IS DISTINCT FROM map_keys(want)"
        ),
    );
    assert_eq!(keys_differ, 0, "{function} n={n}: keys");
    let values_differ = count(
        fx,
        &format!(
            "SELECT count(*) FROM {table} WHERE map_values({function}(x)) IS DISTINCT FROM map_values(want)"
        ),
    );
    assert_eq!(values_differ, 0, "{function} n={n}: values");
    let total = count(
        fx,
        &format!("SELECT coalesce(sum(cardinality({function}(x))), 0)::BIGINT FROM {table}"),
    );
    let want_total: usize = (0..n).map(|x| entries_of(x as i64).unwrap_or(0)).sum();
    assert_eq!(total, want_total as i64, "{function} n={n}: entries");

    // The Rust model, independently of the SQL one: counts of each kind of row.
    let nulls = count(
        fx,
        &format!("SELECT count(*) FROM {table} WHERE {function}(x) IS NULL"),
    );
    assert_eq!(
        nulls,
        (0..n).filter(|&x| entries_of(x as i64).is_none()).count() as i64,
        "{function} n={n}: NULL maps"
    );
    let null_values = count(
        fx,
        &format!(
            "SELECT coalesce(sum(len(list_filter(map_values({function}(x)), lambda v: v IS NULL))), 0)::BIGINT \
             FROM {table} WHERE {function}(x) IS NOT NULL"
        ),
    );
    let want_null_values: usize = (0..n as i64)
        .map(|x| {
            (0..entries_of(x).unwrap_or(0))
                .filter(|&j| value_of(x, j).is_none())
                .count()
        })
        .sum();
    assert_eq!(
        null_values, want_null_values as i64,
        "{function} n={n}: NULL values"
    );
}

#[test]
fn map_vector_write_workflow_builds_maps() {
    let fx = Fixture::open();
    register(&fx, "build_map", TypeId::BigInt, map_type(), build_map);
    for n in edge_rows() {
        check_map_function(&fx, "build_map", n);
    }
    // Two full chunks and a partial third.
    check_map_function(&fx, "build_map", 4100);
}

#[test]
fn map_vector_reserve_growth_keeps_entries_written_before_it() {
    let fx = Fixture::open();
    register(
        &fx,
        "build_map_growing",
        TypeId::BigInt,
        map_type(),
        build_map_growing,
    );
    for n in edge_rows() {
        check_map_function(&fx, "build_map_growing", n);
    }
    check_map_function(&fx, "build_map_growing", 4100);
}

/// A few rows pinned to literal text, so the model above is not the only
/// witness: `x = 1` has two entries, `x = 2` three with a NULL value, `x = 3`
/// is NULL, `x = 5` is empty.
#[test]
fn map_vector_literal_rows() {
    let fx = Fixture::open();
    register(&fx, "build_map", TypeId::BigInt, map_type(), build_map);
    let rows = text_rows(
        &fx,
        "SELECT map_keys(m)::VARCHAR, map_values(m)::VARCHAR, cardinality(m)::VARCHAR, \
                (m IS NULL)::VARCHAR \
         FROM (SELECT build_map(x) AS m FROM range(6) t(x) ORDER BY x)",
    );
    let cell = |row: usize, col: usize| rows[row][col].as_deref();
    // x = 1: keys k0 and the long one, values 100 and 101.
    assert_eq!(cell(1, 0), Some("[k0, a longer key number 1-1]"));
    assert_eq!(cell(1, 1), Some("[100, 101]"));
    assert_eq!(cell(1, 2), Some("2"));
    // x = 2: (2 + 1) % 3 == 0, so the second value is NULL.
    assert_eq!(cell(2, 0), Some("[k0, a longer key number 2-1, k2]"));
    assert_eq!(cell(2, 1), Some("[200, NULL, 202]"));
    assert_eq!(cell(2, 2), Some("3"));
    // x = 3 is a NULL map, x = 5 an empty one.
    assert_eq!(cell(3, 3), Some("true"));
    assert_eq!(cell(3, 2), None);
    assert_eq!(cell(5, 0), Some("[]"));
    assert_eq!(cell(5, 2), Some("0"));
    assert_eq!(cell(5, 3), Some("false"));
}

// ─── Reading a map through the handles ───────────────────────────────────────

// Returns each row's entry count from `get_entry`, after checking the key and
// value handles are fields 0 and 1 of the map's struct child, that the entries
// of every valid row lie within `total_entry_count`, and that the key and
// value readers agree with the entry layout.
quack_rs::scalar_callback!(map_entry_count, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let map: duckdb_vector = unsafe { chunk.vector(0) };
    let validity = unsafe { chunk.reader(0) };
    let struct_child = unsafe { MapVector::struct_child(map) };
    assert_eq!(unsafe { MapVector::keys(map) }, unsafe {
        StructVector::get_child(struct_child, 0)
    });
    assert_eq!(unsafe { MapVector::values(map) }, unsafe {
        StructVector::get_child(struct_child, 1)
    });
    let total = unsafe { MapVector::total_entry_count(map) };
    let keys: VectorReader = unsafe { MapVector::key_reader(map, total) };
    let values: VectorReader = unsafe { MapVector::value_reader(map, total) };
    let mut out = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        if !unsafe { validity.is_valid(row) } {
            unsafe { out.set_null(row) };
            continue;
        }
        let entry = unsafe { MapVector::get_entry(map, row) };
        assert!(
            (entry.offset + entry.length) as usize <= total,
            "row {row}: {}+{} past {total}",
            entry.offset,
            entry.length
        );
        // Sum of the value lengths of this row's keys plus its non-NULL values:
        // a digest of what the readers see at the entry's offsets.
        let mut digest = 0_i64;
        for k in 0..entry.length as usize {
            let at = entry.offset as usize + k;
            digest += unsafe { keys.read_str(at) }.len() as i64;
            if unsafe { values.is_valid(at) } {
                digest += unsafe { values.read_i64(at) };
            }
        }
        unsafe { out.write_i64(row, digest) };
    }
});

#[test]
fn map_vector_read_handles_agree_with_the_engine() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::new("map_entry_count")
            .param_logical(map_type())
            .returns(TypeId::BigInt)
            .function(map_entry_count)
            .register(fx.con())
            .expect("register map_entry_count");
    }
    for n in edge_rows() {
        let model = "list_sum(list_transform(map_entries(m), \
                       lambda e: strlen(e.key) + coalesce(e.value, 0)::BIGINT))";
        let table = format!("(SELECT {MODEL} AS m FROM range({n}) t(x))");
        let differing = count(
            &fx,
            &format!(
                "SELECT count(*) FROM {table} WHERE \
                 map_entry_count(m) IS DISTINCT FROM coalesce({model}, 0) \
                 AND m IS NOT NULL"
            ),
        );
        assert_eq!(differing, 0, "n={n}");
        let null_rows = count(
            &fx,
            &format!("SELECT count(*) FROM {table} WHERE map_entry_count(m) IS NULL"),
        );
        assert_eq!(
            null_rows,
            (0..n).filter(|&x| entries_of(x as i64).is_none()).count() as i64,
            "n={n}"
        );
    }
}
