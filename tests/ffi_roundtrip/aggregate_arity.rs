// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Aggregates that take no argument, which `DuckDB` 2.0 counts as zero rows.
//!
//! The C API hands `update` a chunk and the state array, and no row count: the
//! rows are `chunk.size()`. Up to 1.5.x `CAPIAggregateUpdate` set the chunk's
//! cardinality; 2.0.0 does not (`aggregate_function-c.cpp`), and a chunk with
//! no column reports a size of 0. So `SELECT my_count() FROM t` over 100 rows
//! answers 100 on 1.5.6 and 0 on 2.0.0 — and a windowed call never reaches
//! `update` on either. The registration is refused instead of left to answer
//! wrongly.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateOverloadBuilder,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::Fixture;

const unsafe extern "C" fn size(_info: duckdb_function_info) -> idx_t {
    8
}

const unsafe extern "C" fn init(_info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { state.cast::<i64>().write(0) };
}

/// Counts one per row of the chunk, ignoring any column.
unsafe extern "C" fn count_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    for row in 0..chunk.size() {
        unsafe { *(*states.add(row)).cast::<i64>() += 1 };
    }
}

unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        unsafe { *(*target.add(i)).cast::<i64>() += *(*source.add(i)).cast::<i64>() };
    }
}

unsafe extern "C" fn finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        unsafe { writer.write_i64(offset as usize + i, *(*source.add(i)).cast::<i64>()) };
    }
}

fn count_builder(name: &str) -> AggregateFunctionBuilder {
    AggregateFunctionBuilder::try_new(name)
        .expect("name")
        .returns(TypeId::BigInt)
        .state_size(size)
        .init(init)
        .update(count_update)
        .combine(combine)
        .finalize(finalize)
}

#[test]
fn a_zero_argument_aggregate_is_refused_at_registration() {
    let fx = Fixture::open();

    // SAFETY: `con` is open; the callbacks match their declared signatures.
    let err = unsafe { count_builder("a_cnt0").register(fx.con()) }
        .expect_err("a zero-argument aggregate would count no rows on DuckDB 2.0");
    assert!(err.as_str().contains("no parameter"), "{err}");
    assert!(err.as_str().contains("a_cnt0"), "names the function: {err}");

    // The same in a function set: every overload is checked, including one
    // the `overloads` range generates for n = 0.
    let overload = |n: usize, b: AggregateOverloadBuilder| {
        let b = b
            .state_size(size)
            .init(init)
            .update(count_update)
            .combine(combine)
            .finalize(finalize);
        (0..n).fold(b, |b, _| b.param(TypeId::BigInt))
    };
    let set = AggregateFunctionSetBuilder::try_new("a_cnt_set")
        .expect("name")
        .returns(TypeId::BigInt)
        .overloads(0..=2, overload);
    // SAFETY: as above.
    let err = unsafe { set.register(fx.con()) }.expect_err("overload 0 takes no parameter");
    assert!(err.as_str().contains("overload 0"), "{err}");
    assert!(err.as_str().contains("no parameter"), "{err}");

    // An aggregate with a parameter registers, and counts every row on this
    // engine, ungrouped and grouped. This is the documented spelling of a
    // zero-argument count: one parameter, and a macro in front of it.
    // SAFETY: as above.
    unsafe {
        count_builder("a_cnt1")
            .param(TypeId::BigInt)
            .register(fx.con())
    }
    .expect("one parameter registers");
    fx.query("CREATE MACRO a_cnt() AS a_cnt1(1::BIGINT)");
    let n = |sql: &str| {
        // SAFETY: one BIGINT column, row `i`.
        fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
    };
    assert_eq!(n("SELECT a_cnt() FROM range(100)"), Some(100));
    assert_eq!(
        n("SELECT max(c) FROM (SELECT a_cnt() AS c FROM range(1000) t(i) GROUP BY i % 7)"),
        Some(143)
    );
}
