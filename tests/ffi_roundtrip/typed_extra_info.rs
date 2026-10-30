// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `extra_info_typed` / `extra_info_ref` on casts and aggregates (eighth
//! review, APIGAPS-8): a typed value with a generated destructor, read back
//! by type, where a different type reads as `None` instead of reinterpreting
//! memory.

use std::sync::atomic::{AtomicUsize, Ordering};

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionInfo, AggregateFunctionSetBuilder,
    AggregateOverloadBuilder,
};
use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo};
use quack_rs::data_chunk::DataChunk;
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};

use super::Fixture;

// ─── Cast ────────────────────────────────────────────────────────────────────

/// What the cast adds to each blob's length.
struct Offset(i64);
/// A second type, to be asked for by mistake.
struct NotAnOffset(#[allow(dead_code)] i64);

static OFFSET_DROPS: AtomicUsize = AtomicUsize::new(0);
impl Drop for Offset {
    fn drop(&mut self) {
        OFFSET_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

quack_rs::cast_callback!(blob_len_plus, |info, count, input, output| {
    // SAFETY: `info` is the live cast info.
    let info = unsafe { CastFunctionInfo::new(info) };
    // SAFETY: this cast's extra info is only set with `extra_info_typed`.
    let Some(offset) = (unsafe { info.extra_info_ref::<Offset>() }) else {
        info.set_error("the extra info is not an Offset");
        return false;
    };
    let rows = count as usize;
    // SAFETY: `input` holds `count` BLOB rows; `output` is the BIGINT result.
    let reader = unsafe { VectorReader::from_vector(input, rows) };
    // SAFETY: as above.
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        // SAFETY: `row < count`.
        let len = unsafe { reader.read_blob(row) }.len() as i64;
        // SAFETY: `row < count`.
        unsafe { writer.write_i64(row, len + offset.0) };
    }
    true
});

#[test]
fn a_cast_reads_its_typed_extra_info_and_frees_it_once() {
    let before = OFFSET_DROPS.load(Ordering::SeqCst);
    {
        let fx = Fixture::open();
        // SAFETY: `con` is open; the callback matches `CastFn`.
        unsafe {
            CastFunctionBuilder::new(TypeId::Blob, TypeId::BigInt)
                .function(blob_len_plus)
                .extra_info_typed(Offset(100))
                .register(fx.con())
        }
        .expect("register");
        assert_eq!(
            fx.scalar(
                "SELECT CAST('\\x01\\x02\\x03'::BLOB AS BIGINT)",
                |r, i| unsafe { r.read_i64(i) }
            ),
            Some(103)
        );
        assert_eq!(
            OFFSET_DROPS.load(Ordering::SeqCst),
            before,
            "alive while registered"
        );
    }
    assert_eq!(
        OFFSET_DROPS.load(Ordering::SeqCst) - before,
        1,
        "freed once, when the database closed"
    );
}

#[test]
fn a_cast_asking_for_the_wrong_type_gets_none() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches `CastFn`.
    unsafe {
        CastFunctionBuilder::new(TypeId::Blob, TypeId::BigInt)
            .function(blob_len_plus)
            .extra_info_typed(NotAnOffset(100))
            .register(fx.con())
    }
    .expect("register");
    // SAFETY: `con` is open.
    let err = unsafe { quack_rs::query::query(fx.con(), "SELECT CAST('\\x01'::BLOB AS BIGINT)") }
        .map(drop)
        .expect_err("the callback refuses");
    assert!(err.as_str().contains("not an Offset"), "{err}");
}

#[test]
fn an_unregistered_builder_frees_its_typed_extra_info() {
    // Its own type and counter: the tests run in parallel.
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Unused;
    impl Drop for Unused {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    drop(CastFunctionBuilder::new(TypeId::Blob, TypeId::BigInt).extra_info_typed(Unused));
    drop(AggregateFunctionBuilder::new("never_registered").extra_info_typed(Unused));
    assert_eq!(DROPS.load(Ordering::SeqCst), 2);
}

// ─── Aggregate ───────────────────────────────────────────────────────────────

/// What the aggregate multiplies its sum by.
struct Scale(i64);

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

/// Writes `sum * scale`, or `-1` when the extra info is not a `Scale`.
unsafe extern "C" fn scaled_finalize(
    info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let info = unsafe { AggregateFunctionInfo::new(info) };
    // SAFETY: these aggregates' extra info is only set with `extra_info_typed`.
    let scale = unsafe { info.extra_info_ref::<Scale>() }.map_or(-1, |s| s.0);
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        let sum = unsafe { *(*source.add(i)).cast::<i64>() };
        let value = if scale < 0 { -1 } else { sum * scale };
        unsafe { writer.write_i64(offset as usize + i, value) };
    }
}

#[test]
fn an_aggregate_and_an_overload_read_their_typed_extra_info() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match their signatures.
    unsafe {
        AggregateFunctionBuilder::new("scaled_sum")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(sum_size)
            .init(sum_init)
            .update(sum_update)
            .combine(sum_combine)
            .finalize(scaled_finalize)
            .extra_info_typed(Scale(3))
            .register(fx.con())
            .expect("register scaled_sum");
        AggregateFunctionSetBuilder::new("scaled_sum_set")
            .returns(TypeId::BigInt)
            .overload(
                AggregateOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .state_size(sum_size)
                    .init(sum_init)
                    .update(sum_update)
                    .combine(sum_combine)
                    .finalize(scaled_finalize)
                    .extra_info_typed(Scale(5)),
            )
            .register(fx.con())
            .expect("register scaled_sum_set");
        AggregateFunctionBuilder::new("wrongly_scaled_sum")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(sum_size)
            .init(sum_init)
            .update(sum_update)
            .combine(sum_combine)
            .finalize(scaled_finalize)
            .extra_info_typed(String::from("not a Scale"))
            .register(fx.con())
            .expect("register wrongly_scaled_sum");
    }
    let read = |agg: &str| {
        fx.scalar(
            &format!("SELECT {agg}(x) FROM range(1, 11) t(x)"),
            |r, i| unsafe { r.read_i64(i) },
        )
    };
    assert_eq!(read("scaled_sum"), Some(165));
    assert_eq!(read("scaled_sum_set"), Some(275));
    assert_eq!(
        read("wrongly_scaled_sum"),
        Some(-1),
        "a String is not a Scale"
    );
}
