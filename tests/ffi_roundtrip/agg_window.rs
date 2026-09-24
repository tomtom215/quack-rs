// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Aggregates in the two `DuckDB` evaluation paths that call `update` and
//! `finalize` one row at a time: the streaming window and the sorted
//! aggregate (`agg(x ORDER BY y)`).

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateOverloadBuilder,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::{ListBuilder, VectorWriter};

use super::Fixture;

// A plain-old-data `BIGINT` sum: the state is one `i64` in DuckDB's own
// allocation, so no destructor is needed — exactly the aggregate an author
// registers without one.

const unsafe extern "C" fn sum_size(_info: duckdb_function_info) -> idx_t {
    8
}

const unsafe extern "C" fn sum_init(_info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { state.cast::<i64>().write(0) };
}

unsafe extern "C" fn sum_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            let state = unsafe { (*states.add(row)).cast::<i64>() };
            unsafe { *state += reader.read_i64(row) };
        }
    }
}

unsafe extern "C" fn sum_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        unsafe { *(*target.add(i)).cast::<i64>() += *(*source.add(i)).cast::<i64>() };
    }
}

unsafe extern "C" fn sum_finalize(
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

/// Counts the rows where `agg` disagrees with the built-in `sum` over a
/// running frame. Both are in one window operator, so `DuckDB` evaluates them
/// the same way: streamed when every aggregate there can be streamed.
fn running_sum_mismatches(fx: &Fixture, agg: &str) -> Option<i64> {
    fx.scalar(
        &format!(
            "SELECT count(*) FILTER (WHERE a IS DISTINCT FROM b) FROM (\
               SELECT {agg}(x) OVER w AS a, sum(x) OVER w AS b FROM range(1, 3001) t(x) \
               WINDOW w AS (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW))"
        ),
        |r, i| unsafe { r.read_i64(i) },
    )
}

/// The fourth audit's V1 (Pitfall L13). `DuckDB` streams a running-frame
/// window only for aggregates with no state destructor, and its streaming
/// loop slices the argument chunk one row per `update` call; the C API's
/// `update` wrapper flattens that slice in place on the first call, so every
/// later row re-read row 0. Before `register` installed a no-op destructor,
/// this sum returned 1, 2, 3, … instead of 1, 3, 6, …: 2999 of 3000 rows
/// wrong.
#[test]
fn an_aggregate_without_a_destructor_is_right_in_a_running_window() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("pod_sum")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(sum_size)
            .init(sum_init)
            .update(sum_update)
            .combine(sum_combine)
            .finalize(sum_finalize)
            .register(fx.con())
            .expect("register pod_sum");
        AggregateFunctionSetBuilder::new("pod_sum_set")
            .returns(TypeId::BigInt)
            .overload(
                AggregateOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .state_size(sum_size)
                    .init(sum_init)
                    .update(sum_update)
                    .combine(sum_combine)
                    .finalize(sum_finalize),
            )
            .register(fx.con())
            .expect("register pod_sum_set");
    }
    assert_eq!(running_sum_mismatches(&fx, "pod_sum"), Some(0));
    assert_eq!(running_sum_mismatches(&fx, "pod_sum_set"), Some(0));
}

// A `LIST(BIGINT)` aggregate whose state is the last value seen and whose
// `finalize` writes `[value]` with `ListBuilder`.

unsafe extern "C" fn last_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        unsafe { *(*states.add(row)).cast::<i64>() = reader.read_i64(row) };
    }
}

unsafe extern "C" fn last_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        unsafe { *(*target.add(i)).cast::<i64>() = *(*source.add(i)).cast::<i64>() };
    }
}

unsafe extern "C" fn list_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut builder = unsafe { ListBuilder::new(result) };
    for i in 0..count as usize {
        let value = unsafe { *(*source.add(i)).cast::<i64>() };
        unsafe {
            builder.push_row(offset as usize + i, 1, |writer, base| {
                writer.write_i64(base, value);
            });
        }
    }
    unsafe { builder.finish() };
}

/// The fourth audit's V2. `agg(x ORDER BY y)` finalizes through
/// `SortedAggregateFunction`, which calls `finalize` once per group on the
/// same result vector with an increasing `offset`. `ListBuilder` used to
/// start at child offset 0 every time, so each call overwrote the elements
/// of every earlier row: 2996 of 3000 groups came back with another group's
/// list. It now appends after the elements already in the vector.
///
/// One-row groups keep the query clear of Pitfall L11, which crashes
/// `agg(x ORDER BY y)` for groups of more than one row. The group key is a
/// copy of `x` rather than `x` itself: `DuckDB` drops an aggregate's
/// `ORDER BY` on a grouping column as redundant, which would skip the sorted
/// path this test exists for.
#[test]
fn a_list_builder_finalize_called_once_per_row_keeps_every_row() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("last_as_list")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::BigInt))
            .state_size(sum_size)
            .init(sum_init)
            .update(last_update)
            .combine(last_combine)
            .finalize(list_finalize)
            .register(fx.con())
            .expect("register last_as_list");
    }
    // The built-in `list` is the oracle.
    let wrong = fx.scalar(
        "SELECT count(*) FILTER (WHERE got IS DISTINCT FROM want) FROM (\
           SELECT last_as_list(x ORDER BY x) AS got, list(x) AS want \
           FROM (SELECT range AS x, range AS g FROM range(3000)) GROUP BY g)",
        |r, i| unsafe { r.read_i64(i) },
    );
    assert_eq!(wrong, Some(0));
}
