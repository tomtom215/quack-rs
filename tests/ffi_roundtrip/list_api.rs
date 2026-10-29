// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The raw `ListVector` write workflow — `reserve`, `child_writer`,
//! `set_entry`, `set_size` — and its read side (`get_child`, `get_size`,
//! `get_entry`, `child_reader`), against a live `DuckDB`. `ListBuilder` is the
//! safe way to do this; these tests drive the calls it is built from, with
//! NULL lists, empty lists, NULL elements and long strings, on both sides of
//! the vector size.
//!
//! The function under test turns a `BIGINT` x into a list:
//!
//! - `x % 7 == 3`: a NULL list,
//! - `x % 5 == 0` (and not the above): an empty list,
//! - otherwise `x % 4 + 1` elements, element `j` being `x * 100 + j` (or its
//!   text, for the `VARCHAR` variant) — NULL when `(x + j) % 3 == 0`.

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::complex::ListVector;
use quack_rs::vector::VectorWriter;

use super::vector_support::{count, edge_rows, text_rows};
use super::Fixture;

/// How many elements the list for `x` has, or `None` for a NULL list.
const fn len_of(x: i64) -> Option<usize> {
    if x % 7 == 3 {
        None
    } else if x % 5 == 0 {
        Some(0)
    } else {
        Some((x % 4) as usize + 1)
    }
}

/// Element `j` of the list for `x`; `None` is a NULL element.
const fn element_of(x: i64, j: usize) -> Option<i64> {
    if (x + j as i64) % 3 == 0 {
        None
    } else {
        Some(x * 100 + j as i64)
    }
}

/// The text of element `j`: short for even `j`, past the 12 inline bytes for odd.
fn text_of(x: i64, j: usize) -> String {
    let v = x * 100 + j as i64;
    if j % 2 == 0 {
        v.to_string()
    } else {
        format!("element number {v}")
    }
}

const BIGINT_MODEL: &str = "CASE WHEN x % 7 = 3 THEN NULL \
    WHEN x % 5 = 0 THEN []::BIGINT[] \
    ELSE list_transform(range(x % 4 + 1), lambda j: \
      CASE WHEN (x + j) % 3 = 0 THEN NULL ELSE x * 100 + j END) END";

const VARCHAR_MODEL: &str = "CASE WHEN x % 7 = 3 THEN NULL \
    WHEN x % 5 = 0 THEN []::VARCHAR[] \
    ELSE list_transform(range(x % 4 + 1), lambda j: \
      CASE WHEN (x + j) % 3 = 0 THEN NULL \
           WHEN j % 2 = 0 THEN (x * 100 + j)::VARCHAR \
           ELSE 'element number ' || (x * 100 + j) END) END";

fn register(fx: &Fixture, name: &str, element: TypeId, callback: ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(element))
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

// One `reserve` for the whole chunk.
quack_rs::scalar_callback!(build_bigint_list, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let total: usize = (0..rows)
        .map(|row| len_of(unsafe { reader.read_i64(row) }).unwrap_or(0))
        .sum();
    unsafe { ListVector::reserve(output, total) };
    let mut elements = unsafe { ListVector::child_writer(output) };
    assert_eq!(elements.as_raw(), unsafe { ListVector::get_child(output) });
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut at = 0_usize;
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        let Some(len) = len_of(x) else {
            unsafe { parent.set_null(row) };
            unsafe { ListVector::set_entry(output, row, at as u64, 0) };
            continue;
        };
        unsafe { ListVector::set_entry(output, row, at as u64, len as u64) };
        for j in 0..len {
            match element_of(x, j) {
                Some(v) => unsafe { elements.write_i64(at + j, v) },
                None => unsafe { elements.set_null(at + j) },
            }
        }
        at += len;
    }
    assert_eq!(at, total);
    unsafe { ListVector::set_size(output, at) };
    assert_eq!(unsafe { ListVector::get_size(output) }, at);
    // Each row's entry reads back as it was written.
    let mut want_offset = 0_u64;
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        let entry = unsafe { ListVector::get_entry(output, row) };
        assert_eq!(entry.offset, want_offset, "row {row}: offset");
        assert_eq!(
            entry.length as usize,
            len_of(x).unwrap_or(0),
            "row {row}: length"
        );
        want_offset += entry.length;
    }
});

// `VARCHAR` elements, the child grown a row at a time: each row reserves what
// it needs and fetches the child writer again, so strings written before a
// reallocation have to survive it.
quack_rs::scalar_callback!(build_varchar_list_growing, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut at = 0_usize;
    for row in 0..rows {
        let x = unsafe { reader.read_i64(row) };
        let Some(len) = len_of(x) else {
            unsafe { parent.set_null(row) };
            unsafe { ListVector::set_entry(output, row, at as u64, 0) };
            continue;
        };
        unsafe { ListVector::reserve(output, at + len) };
        let mut elements = unsafe { ListVector::child_writer(output) };
        unsafe { ListVector::set_entry(output, row, at as u64, len as u64) };
        for j in 0..len {
            if element_of(x, j).is_none() {
                unsafe { elements.set_null(at + j) };
            } else {
                unsafe { elements.write_varchar(at + j, &text_of(x, j)) };
            }
        }
        at += len;
        unsafe { ListVector::set_size(output, at) };
        assert_eq!(unsafe { ListVector::get_size(output) }, at, "row {row}");
    }
});

fn check_list_function(fx: &Fixture, function: &str, model: &str, n: usize) {
    let table = format!("(SELECT x, {model} AS want FROM range({n}) t(x))");

    let differing = count(
        fx,
        &format!("SELECT count(*) FROM {table} WHERE {function}(x) IS DISTINCT FROM want"),
    );
    assert_eq!(differing, 0, "{function} n={n}: lists differ");

    // Materialised into a table, which copies the child by the size `set_size`
    // recorded rather than by following the row entries.
    fx.query(&format!(
        "CREATE OR REPLACE TEMP TABLE got AS SELECT x, {function}(x) AS v FROM range({n}) t(x)"
    ));
    assert_eq!(count(fx, "SELECT count(*) FROM got"), n as i64);
    let stored_differ = count(
        fx,
        &format!(
            "SELECT count(*) FROM got JOIN (SELECT x, {model} AS want FROM range({n}) t(x)) w \
             USING (x) WHERE got.v IS DISTINCT FROM w.want"
        ),
    );
    assert_eq!(stored_differ, 0, "{function} n={n}: stored lists differ");

    let rows = text_rows(
        fx,
        &format!("SELECT {function}(x)::VARCHAR, want::VARCHAR FROM {table} ORDER BY x"),
    );
    assert_eq!(rows.len(), n);
    for (x, row) in rows.iter().enumerate() {
        assert_eq!(row[0], row[1], "{function} n={n} x={x}");
        assert_eq!(row[0].is_none(), len_of(x as i64).is_none(), "x={x}");
    }

    // The Rust model, independently of the SQL one.
    let elements = count(
        fx,
        &format!("SELECT coalesce(sum(len({function}(x))), 0)::BIGINT FROM {table}"),
    );
    let want_elements: usize = (0..n).map(|x| len_of(x as i64).unwrap_or(0)).sum();
    assert_eq!(elements, want_elements as i64, "{function} n={n}: elements");
    let nulls = count(
        fx,
        &format!("SELECT count(*) FROM {table} WHERE {function}(x) IS NULL"),
    );
    assert_eq!(
        nulls,
        (0..n).filter(|&x| len_of(x as i64).is_none()).count() as i64,
        "{function} n={n}: NULL lists"
    );
    let null_elements = count(
        fx,
        &format!(
            "SELECT coalesce(sum(len(list_filter({function}(x), lambda e: e IS NULL))), 0)::BIGINT \
             FROM {table}"
        ),
    );
    let want_null_elements: usize = (0..n as i64)
        .map(|x| {
            (0..len_of(x).unwrap_or(0))
                .filter(|&j| element_of(x, j).is_none())
                .count()
        })
        .sum();
    assert_eq!(
        null_elements, want_null_elements as i64,
        "{function} n={n}: NULL elements"
    );
}

#[test]
fn list_vector_write_workflow_builds_lists() {
    let fx = Fixture::open();
    register(&fx, "build_bigint_list", TypeId::BigInt, build_bigint_list);
    for n in edge_rows() {
        check_list_function(&fx, "build_bigint_list", BIGINT_MODEL, n);
    }
    check_list_function(&fx, "build_bigint_list", BIGINT_MODEL, 4100);
}

#[test]
fn list_vector_reserve_growth_keeps_strings_written_before_it() {
    let fx = Fixture::open();
    register(
        &fx,
        "build_varchar_list_growing",
        TypeId::Varchar,
        build_varchar_list_growing,
    );
    for n in edge_rows() {
        check_list_function(&fx, "build_varchar_list_growing", VARCHAR_MODEL, n);
    }
    check_list_function(&fx, "build_varchar_list_growing", VARCHAR_MODEL, 4100);
}

/// Rows pinned to literal text, so the SQL model is not the only witness.
#[test]
fn list_vector_literal_rows() {
    let fx = Fixture::open();
    register(&fx, "build_bigint_list", TypeId::BigInt, build_bigint_list);
    let rows = text_rows(
        &fx,
        "SELECT build_bigint_list(x)::VARCHAR FROM range(8) t(x) ORDER BY x",
    );
    let got: Vec<&str> = rows
        .iter()
        .map(|r| r[0].as_deref().unwrap_or("NULL"))
        .collect();
    // Element j of row x is NULL when (x + j) % 3 == 0.
    assert_eq!(
        got,
        [
            "[]",                    // x = 0: empty (x % 5 == 0)
            "[100, 101]",            // x = 1
            "[200, NULL, 202]",      // x = 2: (2 + 1) % 3 == 0
            "NULL",                  // x = 3: x % 7 == 3
            "[400]",                 // x = 4: one element
            "[]",                    // x = 5: empty
            "[NULL, 601, 602]",      // x = 6: (6 + 0) % 3 == 0
            "[700, 701, NULL, 703]", // x = 7: (7 + 2) % 3 == 0
        ]
    );
}

/// `max_child_capacity` on live `LIST` and `MAP` vectors: the largest power of
/// two of elements whose widest buffer stays within 2^37 bytes — the number
/// `DuckDB` aborts the process above, which `ListVector::reserve` and
/// `MapVector::reserve` tell their callers to check first.
#[cfg(all(feature = "duckdb-1-5", target_pointer_width = "64"))]
mod capacity {
    use quack_rs::types::{LogicalType, TypeId};
    use quack_rs::vector::{max_child_capacity, OwnedVector};

    use super::Fixture;

    fn capacity_of(ty: &LogicalType) -> usize {
        let vector = OwnedVector::new(ty, 1).expect("vector");
        // SAFETY: a live LIST or MAP vector.
        unsafe { max_child_capacity(vector.as_raw()) }
    }

    #[test]
    fn max_child_capacity_follows_the_widest_buffer_of_the_child() {
        let _fx = Fixture::open();
        let list = |child: TypeId| capacity_of(&LogicalType::list(child));
        // 2^37 bytes over the element size, rounded down to a power of two.
        assert_eq!(list(TypeId::Boolean), 1 << 37);
        assert_eq!(list(TypeId::TinyInt), 1 << 37);
        assert_eq!(list(TypeId::SmallInt), 1 << 36);
        assert_eq!(list(TypeId::Integer), 1 << 35);
        assert_eq!(list(TypeId::BigInt), 1 << 34);
        assert_eq!(list(TypeId::Double), 1 << 34);
        // `duckdb_string_t` and HUGEINT are 16 bytes.
        assert_eq!(list(TypeId::Varchar), 1 << 33);
        assert_eq!(list(TypeId::HugeInt), 1 << 33);

        // A STRUCT child is as wide as its widest field.
        let narrow = LogicalType::struct_type(&[("a", TypeId::TinyInt), ("b", TypeId::BigInt)]);
        let wide = LogicalType::struct_type(&[("a", TypeId::TinyInt), ("b", TypeId::Varchar)]);
        assert_eq!(
            capacity_of(&LogicalType::list_from_logical(&narrow)),
            1 << 34
        );
        assert_eq!(capacity_of(&LogicalType::list_from_logical(&wide)), 1 << 33);

        // An ARRAY child multiplies: 4000 bytes per element, and 2^37 / 4000 is
        // 34,359,738, whose largest power of two below it is 2^25.
        let arrays = LogicalType::array(TypeId::Integer, 1000);
        assert_eq!(
            capacity_of(&LogicalType::list_from_logical(&arrays)),
            1 << 25
        );

        // A MAP's child is the STRUCT(key, value) of its two types.
        assert_eq!(
            capacity_of(&LogicalType::map(TypeId::TinyInt, TypeId::TinyInt)),
            1 << 37
        );
        assert_eq!(
            capacity_of(&LogicalType::map(TypeId::TinyInt, TypeId::BigInt)),
            1 << 34
        );
        assert_eq!(
            capacity_of(&LogicalType::map(TypeId::Varchar, TypeId::BigInt)),
            1 << 33
        );
    }
}
