// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The hot paths that skip a C API call: `VectorReader::is_valid` and
//! `VectorWriter::set_null` test and clear the validity bit directly (the
//! layout `duckdb.h` documents for `duckdb_vector_get_validity`), and
//! `ListBuilder` keeps its child writer between rows. Each is held to the C
//! API's own answer, at the word edges (63, 64, 65) and at the vector edge.

use libduckdb_sys::{
    duckdb_data_chunk_get_vector, duckdb_validity_row_is_valid, duckdb_vector_get_validity,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::{ListBuilder, VectorWriter};

use super::Fixture;

// Per row: bit 0 is set when `is_valid` agrees with `duckdb_validity_row_is_valid`
// on the input vector, bit 1 when the reader calls the row valid.
quack_rs::scalar_callback!(validity_probe, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mask = unsafe { duckdb_vector_get_validity(duckdb_data_chunk_get_vector(input, 0)) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let valid = unsafe { reader.is_valid(row) };
        let truth = unsafe { duckdb_validity_row_is_valid(mask, row as u64) };
        unsafe { writer.write_i64(row, i64::from(valid == truth) | (i64::from(valid) << 1)) };
    }
});

fn register_probe(fx: &Fixture) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("validity_probe")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            // The probe must see NULL rows: under the default handling the engine
            // never calls a function on a constant NULL (DuckDB 2.0 folds an
            // all-NULL column to one) and masks the output of the rows it does call.
            .null_handling(quack_rs::types::NullHandling::SpecialNullHandling)
            .function(validity_probe)
            .register(fx.con())
            .expect("register");
    }
}

#[test]
fn is_valid_agrees_with_the_c_api_at_every_row() {
    let fx = Fixture::open();
    register_probe(&fx);
    // Row counts around the 64-row words and the 2048-row vector; null
    // densities from half the rows down to none (the engine then hands over no
    // mask at all) and all of them.
    for rows in [1, 2, 63, 64, 65, 127, 128, 129, 2047, 2048, 2049, 5000] {
        for modulus in [1, 2, 3, 97, 1_000_000_007_u64] {
            let sql = format!(
                "SELECT (min(p & 1) = 1)::VARCHAR || ',' || (sum(p >> 1) = count(x))::VARCHAR \
                 FROM (SELECT validity_probe(x) AS p, x FROM \
                   (SELECT CASE WHEN hash(i) % {modulus} = 0 THEN NULL ELSE i END AS x \
                    FROM range({rows}) t(i)))"
            );
            assert_eq!(
                fx.scalar(&sql, |r, i| unsafe { r.read_str(i).to_owned() })
                    .as_deref(),
                Some("true,true"),
                "{sql}"
            );
        }
    }
}

/// Whether the callback below nulls chunk-local `row`: a third of the rows plus
/// the last bit of every word, so each word of the mask is written both ways.
const fn nulled(row: usize) -> bool {
    row % 3 == 0 || row % 64 == 63 || row == 64
}

// Writes `row` or NULL per `nulled`, then reads the output mask back through
// the C API: any row whose validity differs from the pattern (a wrong bit, or a
// neighbour cleared with it) poisons every value with -1.
quack_rs::scalar_callback!(null_pattern, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if nulled(row) {
            unsafe { writer.set_null(row) };
        } else {
            unsafe { writer.write_i64(row, row as i64) };
        }
    }
    let mask = unsafe { duckdb_vector_get_validity(output) };
    let wrong = (0..chunk.size())
        .filter(|&row| {
            let valid = mask.is_null() || unsafe { duckdb_validity_row_is_valid(mask, row as u64) };
            valid == nulled(row)
        })
        .count();
    if wrong != 0 {
        for row in 0..chunk.size() {
            unsafe { writer.write_i64(row, -1) };
        }
    }
});

#[test]
fn set_null_clears_exactly_one_bit() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("null_pattern")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(null_pattern)
            .register(fx.con())
            .expect("register");
    }
    for rows in [1, 64, 65, 130, 2048, 2049, 5000] {
        let sql = format!(
            "SELECT (count(*) FILTER (WHERE v = -1))::VARCHAR || ',' || \
                    (count(*) FILTER (WHERE v IS NULL) > 0)::VARCHAR || ',' || \
                    (count(v) > 0)::VARCHAR \
             FROM (SELECT null_pattern(i) AS v FROM range({rows}) t(i))"
        );
        // Row 0 is always nulled; from two rows on, row 1 is not.
        assert_eq!(
            fx.scalar(&sql, |r, i| unsafe { r.read_str(i).to_owned() })
                .as_deref(),
            Some(format!("0,true,{}", rows > 1).as_str()),
            "{sql}"
        );
    }
}

// Row `r` holds `n = input` elements, element `j` NULL when `(n + j) % 3 == 0`
// and `n * 1000 + j` otherwise. A chunk of 2048 rows holds ~40,000 elements, so
// the child grows through a dozen reserves: a writer kept across one would write
// into the old buffer, and the first NULL element allocates the child's mask in
// the middle of the chunk.
quack_rs::scalar_callback!(growing_list, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let n = unsafe { reader.read_i64(row) };
        let len = usize::try_from(n).expect("non-negative");
        // SAFETY: `row` is in the chunk; the closure writes exactly
        // `base..base + len`.
        unsafe {
            builder.push_row(row, len, |w, base| {
                for j in 0..len {
                    if (len + j) % 3 == 0 {
                        w.set_null(base + j);
                    } else {
                        w.write_i64(base + j, n * 1000 + j as i64);
                    }
                }
            });
        }
    }
    unsafe { builder.finish() };
});

#[test]
fn a_list_builder_keeps_correct_elements_across_reserves_and_nulls() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("growing_list")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::BigInt))
            .function(growing_list)
            .register(fx.con())
            .expect("register");
    }
    let sql = "WITH t AS (SELECT i, i % 40 AS n FROM range(20000) t(i)), \
         got AS (SELECT i, growing_list(n) AS g FROM t), \
         want AS (SELECT i, list(CASE WHEN (n + j) % 3 = 0 THEN NULL ELSE n * 1000 + j END \
                                 ORDER BY j) AS w \
                  FROM (SELECT i, n, unnest(range(n)) AS j FROM t) GROUP BY i) \
         SELECT (count(*) FILTER (WHERE g IS DISTINCT FROM coalesce(w, []::BIGINT[])))::VARCHAR \
                || ',' || count(*)::VARCHAR \
         FROM got LEFT JOIN want USING (i)";
    assert_eq!(
        fx.scalar(sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .as_deref(),
        Some("0,20000"),
        "{sql}"
    );
}

// A `ListBuilder` on the inner LIST of a `LIST(LIST(BIGINT))`, made once per
// chunk and kept across the outer builder's rows. Each outer reserve that grows
// reallocates the inner LIST vector's entry array (it is the outer child's
// data, L18), so a builder that cached that array wrote into freed memory
// (SIGSEGV on DuckDB 1.5.6 at 20,000 rows).
quack_rs::scalar_callback!(hoisted_inner_list, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let inner_vec = unsafe { quack_rs::vector::complex::ListVector::get_child(output) };
    let mut outer = unsafe { ListBuilder::new(output) };
    let mut inner = unsafe { ListBuilder::new(inner_vec) };
    for row in 0..chunk.size() {
        let x = unsafe { reader.read_i64(row) };
        let n = usize::try_from(x % 7).expect("non-negative");
        unsafe {
            outer.push_row(row, n, |_w, base| {
                for j in 0..n {
                    let v = x * 100 + j as i64;
                    inner.push_row(base + j, 2, |w, b| {
                        w.write_i64(b, v);
                        w.write_i64(b + 1, -v);
                    });
                }
            });
        }
    }
    unsafe { inner.finish() };
    unsafe { outer.finish() };
});

#[test]
fn a_list_builder_on_an_inner_list_survives_the_outer_lists_reserves() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("hoisted_inner_list")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list_from_logical(&LogicalType::list(
                TypeId::BigInt,
            )))
            .function(hoisted_inner_list)
            .register(fx.con())
            .expect("register");
    }
    let sql = "WITH t AS (SELECT i AS x FROM range(20000) t(i)) \
         SELECT (count(*) FILTER (WHERE hoisted_inner_list(x) IS DISTINCT FROM \
                 list_transform(range(x % 7), lambda j: [x * 100 + j, -(x * 100 + j)])))::VARCHAR \
                || ',' || count(*)::VARCHAR FROM t";
    assert_eq!(
        fx.scalar(sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .as_deref(),
        Some("0,20000"),
        "{sql}"
    );
}
