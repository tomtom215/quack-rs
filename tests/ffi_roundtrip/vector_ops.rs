// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `OwnedVector::into_raw`, `vector_get_column_type` and the `ops` functions
//! `copy_selected`, `slice`, `reference_value` and `reference_vector`, against
//! live `DuckDB` vectors.
//!
//! `slice`, `reference_value` and `reference_vector` hand the engine a vector
//! that is no longer flat, so their results are checked through SQL; the
//! flat ones (`copy_selected`, `into_raw`) are read back directly.

use std::sync::Mutex;

use libduckdb_sys::{duckdb_destroy_vector, duckdb_vector};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::selection_vector::SelectionVector;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;
use quack_rs::vector::ops::{copy_selected, reference_value, reference_vector, slice};
use quack_rs::vector::{vector_get_column_type, OwnedVector, VectorReader, VectorWriter};

use super::vector_support::{count, edge_rows, text_rows};
use super::Fixture;

// ─── OwnedVector::into_raw and vector_get_column_type ────────────────────────

#[test]
fn into_raw_hands_over_a_live_vector_the_caller_destroys() {
    let _fx = Fixture::open();
    let owned = OwnedVector::new(&LogicalType::new(TypeId::BigInt), 16).expect("vector");
    let as_raw = owned.as_raw();
    // SAFETY: a live BIGINT vector with 16 rows.
    let mut writer = unsafe { VectorWriter::from_vector(as_raw) };
    for row in 0..16 {
        // SAFETY: `row` is below the capacity.
        unsafe { writer.write_i64(row, row as i64 * 1_000_003) };
    }

    let mut raw = owned.into_raw();
    assert_eq!(raw, as_raw, "into_raw returns the same handle");
    // `into_raw` did not destroy the vector: it is still readable, with its type.
    // SAFETY: `raw` is live until the `duckdb_destroy_vector` below.
    let reader = unsafe { VectorReader::from_vector(raw, 16) };
    for row in 0..16 {
        // SAFETY: `row < 16`.
        assert_eq!(unsafe { reader.read_i64(row) }, row as i64 * 1_000_003);
    }
    // SAFETY: as above.
    let ty = unsafe { vector_get_column_type(raw) };
    // SAFETY: `ty` is a live logical type.
    assert_eq!(unsafe { ty.get_type_id() }, TypeId::BigInt);
    // The caller is responsible for the one destroy; a second would crash.
    // SAFETY: `raw` came from `duckdb_create_vector` and is destroyed once.
    unsafe { duckdb_destroy_vector(&raw mut raw) };
    assert!(raw.is_null(), "duckdb_destroy_vector clears the handle");
}

#[test]
fn vector_get_column_type_reports_the_full_type() {
    let _fx = Fixture::open();
    let ty = LogicalType::struct_type_from_logical(&[
        ("id", LogicalType::new(TypeId::BigInt)),
        ("price", LogicalType::decimal(18, 3)),
        ("tags", LogicalType::list(TypeId::Varchar)),
        ("attrs", LogicalType::map(TypeId::Varchar, TypeId::Integer)),
        ("pos", LogicalType::array(TypeId::Double, 3)),
    ]);
    let owned = OwnedVector::new(&ty, 4).expect("vector");
    // SAFETY: a live STRUCT vector.
    let got = unsafe { vector_get_column_type(owned.as_raw()) };
    // SAFETY: `got` and its children are live logical types.
    unsafe {
        assert_eq!(got.get_type_id(), TypeId::Struct);
        assert_eq!(got.struct_child_count(), 5);
        let names: Vec<String> = (0..5).map(|i| got.struct_child_name(i)).collect();
        assert_eq!(names, ["id", "price", "tags", "attrs", "pos"]);
        assert_eq!(got.struct_child_type(0).get_type_id(), TypeId::BigInt);
        let price = got.struct_child_type(1);
        assert_eq!(price.get_type_id(), TypeId::Decimal);
        assert_eq!((price.decimal_width(), price.decimal_scale()), (18, 3));
        let tags = got.struct_child_type(2);
        assert_eq!(tags.get_type_id(), TypeId::List);
        assert_eq!(tags.list_child_type().get_type_id(), TypeId::Varchar);
        let attrs = got.struct_child_type(3);
        assert_eq!(attrs.get_type_id(), TypeId::Map);
        assert_eq!(attrs.map_key_type().get_type_id(), TypeId::Varchar);
        assert_eq!(attrs.map_value_type().get_type_id(), TypeId::Integer);
        let pos = got.struct_child_type(4);
        assert_eq!(pos.get_type_id(), TypeId::Array);
        assert_eq!(pos.array_size(), 3);
        assert_eq!(pos.array_child_type().get_type_id(), TypeId::Double);
    }
}

// ─── copy_selected ───────────────────────────────────────────────────────────

/// The text row `r` of the source vector holds: NULL for `r % 5 == 2`, a short
/// string for even rows, one past the 12 inline bytes for odd rows.
fn source_text(r: usize) -> Option<String> {
    match r % 5 {
        2 => None,
        _ if r % 2 == 0 => Some(format!("s{r}")),
        _ => Some(format!("a string that is not inline: {r}")),
    }
}

fn varchar_source(rows: usize) -> OwnedVector {
    let vector = OwnedVector::new(&LogicalType::new(TypeId::Varchar), rows).expect("source");
    // SAFETY: a live VARCHAR vector with `rows` rows.
    let mut writer = unsafe { VectorWriter::from_vector(vector.as_raw()) };
    for r in 0..rows {
        match source_text(r) {
            // SAFETY: `r < rows`.
            Some(text) => unsafe { writer.write_varchar(r, &text) },
            None => unsafe { writer.set_null(r) },
        }
    }
    vector
}

fn read_texts(vector: &OwnedVector, rows: usize) -> Vec<Option<String>> {
    // SAFETY: a flat VARCHAR vector with at least `rows` rows.
    let reader = unsafe { VectorReader::from_vector(vector.as_raw(), rows) };
    (0..rows)
        .map(|r| unsafe { reader.is_valid(r).then(|| reader.read_str(r).to_owned()) })
        .collect()
}

fn selection(indices: &[u32]) -> SelectionVector {
    let mut sel = SelectionVector::new(indices.len()).expect("selection");
    sel.as_mut_slice().copy_from_slice(indices);
    sel
}

#[test]
fn copy_selected_copies_strings_and_nulls_in_selection_order() {
    let _fx = Fixture::open();
    for n in edge_rows() {
        let src = varchar_source(n);
        // Every row but those `r % 3 == 1`, last to first.
        let picked: Vec<u32> = (0..n as u32).filter(|r| r % 3 != 1).rev().collect();
        let sel = selection(&picked);
        let dst = OwnedVector::new(&LogicalType::new(TypeId::Varchar), picked.len())
            .expect("destination");
        // SAFETY: same type, every index is a source row, `dst` has room.
        unsafe { copy_selected(src.as_raw(), dst.as_raw(), &sel, picked.len(), 0, 0) };
        // The copy owns its strings: the source can go before it is read.
        drop(src);
        let want: Vec<Option<String>> = picked.iter().map(|&r| source_text(r as usize)).collect();
        assert_eq!(read_texts(&dst, picked.len()), want, "n={n}");
    }
}

/// `duckdb.h`: the number of rows copied is `src_count - src_offset`, taken
/// from `sel[src_offset..src_count]`, and written from `dst[dst_offset]`.
#[test]
fn copy_selected_honours_src_offset_and_dst_offset() {
    let _fx = Fixture::open();
    let src = varchar_source(50);
    let picked: Vec<u32> = vec![49, 3, 17, 0, 22, 8, 31, 12];
    let sel = selection(&picked);
    let dst = OwnedVector::new(&LogicalType::new(TypeId::Varchar), 20).expect("destination");
    // Pre-fill the destination so rows the copy must not touch are visible.
    // SAFETY: a live VARCHAR vector with 20 rows.
    let mut writer = unsafe { VectorWriter::from_vector(dst.as_raw()) };
    for r in 0..20 {
        unsafe { writer.write_varchar(r, &format!("untouched {r}")) };
    }

    // Copy `sel[2..7]` (five rows) to `dst[5..10]`.
    // SAFETY: indices are source rows; `dst` has room for 5 + 5 rows.
    unsafe { copy_selected(src.as_raw(), dst.as_raw(), &sel, 7, 2, 5) };

    let got = read_texts(&dst, 20);
    for (r, cell) in got.iter().enumerate() {
        let want = if (5..10).contains(&r) {
            source_text(picked[2 + (r - 5)] as usize)
        } else {
            Some(format!("untouched {r}"))
        };
        assert_eq!(*cell, want, "dst row {r}");
    }
}

// ─── slice ───────────────────────────────────────────────────────────────────

// Writes x * 10 (NULL where x % 5 == 0) into every row, then swaps neighbouring
// rows with a selection vector: row `i` becomes the row that was `i ^ 1`.
quack_rs::scalar_callback!(swap_neighbours, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        if x % 5 == 0 {
            unsafe { writer.set_null(row) };
        } else {
            unsafe { writer.write_i64(row, x * 10) };
        }
    }
    let mut sel = SelectionVector::new(rows).expect("selection");
    for (i, slot) in sel.as_mut_slice().iter_mut().enumerate() {
        // A lone last row (odd chunk size) stays where it is.
        *slot = if (i ^ 1) < rows {
            (i ^ 1) as u32
        } else {
            i as u32
        };
    }
    // SAFETY: every index is a row of `output`, and nothing reads or writes
    // `output` through a reader or writer afterwards.
    unsafe { slice(output, &sel, rows) };
});

fn register(fx: &Fixture, name: &str, param: TypeId, ret: TypeId, callback: ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(param)
            .returns(ret)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

#[test]
fn slice_reorders_the_result_without_copying_it() {
    let fx = Fixture::open();
    register(
        &fx,
        "swap_neighbours",
        TypeId::BigInt,
        TypeId::BigInt,
        swap_neighbours,
    );
    for n in edge_rows() {
        let rows = text_rows(
            &fx,
            &format!("SELECT swap_neighbours(x)::VARCHAR FROM range({n}) t(x) ORDER BY x"),
        );
        assert_eq!(rows.len(), n);
        for (x, row) in rows.iter().enumerate() {
            let partner = if (x ^ 1) < n { x ^ 1 } else { x };
            let want = (partner % 5 != 0).then(|| (partner * 10).to_string());
            assert_eq!(row[0], want, "n={n} x={x}");
        }
        // And DuckDB can aggregate over the sliced vector.
        let sum = count(
            &fx,
            &format!("SELECT coalesce(sum(swap_neighbours(x)), 0)::BIGINT FROM range({n}) t(x)"),
        );
        let want: usize = (0..n).filter(|x| x % 5 != 0).map(|x| x * 10).sum();
        assert_eq!(sum, want as i64, "n={n}: sum is unchanged by the swap");
    }
}

// ─── reference_value ─────────────────────────────────────────────────────────

quack_rs::scalar_callback!(constant_bigint, |_info, _input, output| {
    let value = Value::bigint(7_000_000_007);
    // SAFETY: `output` is a BIGINT vector and the value is a BIGINT.
    unsafe { reference_value(output, &value) };
});

quack_rs::scalar_callback!(constant_varchar, |_info, _input, output| {
    let value = Value::varchar("a constant that is longer than twelve bytes");
    // SAFETY: `output` is a VARCHAR vector and the value is a VARCHAR.
    unsafe { reference_value(output, &value) };
});

#[test]
fn reference_value_fills_every_row_with_one_constant() {
    let fx = Fixture::open();
    register(
        &fx,
        "constant_bigint",
        TypeId::BigInt,
        TypeId::BigInt,
        constant_bigint,
    );
    register(
        &fx,
        "constant_varchar",
        TypeId::BigInt,
        TypeId::Varchar,
        constant_varchar,
    );
    for n in edge_rows() {
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM range({n}) t(x) WHERE constant_bigint(x) = 7000000007"
                )
            ),
            n as i64,
            "n={n}: BIGINT"
        );
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM range({n}) t(x) \
                     WHERE constant_varchar(x) = 'a constant that is longer than twelve bytes'"
                )
            ),
            n as i64,
            "n={n}: VARCHAR"
        );
        // A different row count in the same query: the constant follows it.
        assert_eq!(
            count(
                &fx,
                &format!("SELECT sum(constant_bigint(x)) FROM range({n}) t(x)")
            ),
            7_000_000_007 * n as i64,
            "n={n}: sum"
        );
    }
}

// ─── reference_vector ────────────────────────────────────────────────────────

/// Vectors the callback below made (as addresses, so the list can be shared
/// between threads), kept alive until the test has read every result:
/// `reference_vector`'s contract is that the source outlives every read of the
/// target. The test destroys them, which is what `OwnedVector::into_raw` hands
/// over.
static KEPT: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Destroys every vector the callback below kept.
fn destroy_kept() {
    for address in KEPT.lock().expect("lock").drain(..) {
        let mut vector = address as duckdb_vector;
        // SAFETY: the address came from `OwnedVector::into_raw`, which gave up
        // ownership, and is destroyed exactly once here.
        unsafe { duckdb_destroy_vector(&raw mut vector) };
    }
}

// Builds the answer in a vector of its own — x * 3, NULL where x % 4 == 0 —
// and makes `output` share it.
quack_rs::scalar_callback!(via_referenced_vector, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let staging =
        OwnedVector::new(&LogicalType::new(TypeId::BigInt), rows).expect("staging vector");
    let mut writer = unsafe { VectorWriter::from_vector(staging.as_raw()) };
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        if x % 4 == 0 {
            unsafe { writer.set_null(row) };
        } else {
            unsafe { writer.write_i64(row, x * 3) };
        }
    }
    // SAFETY: both are BIGINT vectors; `staging` is kept until the test ends
    // its reads, and `writer` is not used again.
    unsafe { reference_vector(output, staging.as_raw()) };
    KEPT.lock().expect("lock").push(staging.into_raw() as usize);
});

#[test]
fn reference_vector_shares_the_source_data_and_nulls() {
    let fx = Fixture::open();
    register(
        &fx,
        "via_referenced_vector",
        TypeId::BigInt,
        TypeId::BigInt,
        via_referenced_vector,
    );
    for n in edge_rows() {
        let rows = text_rows(
            &fx,
            &format!("SELECT via_referenced_vector(x)::VARCHAR FROM range({n}) t(x) ORDER BY x"),
        );
        assert_eq!(rows.len(), n);
        for (x, row) in rows.iter().enumerate() {
            let want = (x % 4 != 0).then(|| (x * 3).to_string());
            assert_eq!(row[0], want, "n={n} x={x}");
        }
        destroy_kept();
    }
}
