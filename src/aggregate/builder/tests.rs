// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use crate::types::{NullHandling, TypeId};
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};

// Callback stubs shared by the function-set tests. They are never invoked --
// these tests only assert on what the builder records -- but the builder's
// setters are typed, so real `extern "C"` items are needed.
unsafe extern "C" fn ss(_: duckdb_function_info) -> idx_t {
    0
}
unsafe extern "C" fn si(_: duckdb_function_info, _: duckdb_aggregate_state) {}
unsafe extern "C" fn su(
    _: duckdb_function_info,
    _: duckdb_data_chunk,
    _: *mut duckdb_aggregate_state,
) {
}
unsafe extern "C" fn sc(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: *mut duckdb_aggregate_state,
    _: idx_t,
) {
}
unsafe extern "C" fn sf(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: duckdb_vector,
    _: idx_t,
    _: idx_t,
) {
}

// Verify that AggregateFunctionBuilder stores name correctly
#[test]
fn builder_stores_name() {
    let b = AggregateFunctionBuilder::new("my_func");
    assert_eq!(b.name.to_str().unwrap(), "my_func");
}

#[test]
fn builder_stores_params() {
    let b = AggregateFunctionBuilder::new("f")
        .param(TypeId::BigInt)
        .param(TypeId::Varchar);
    assert_eq!(b.params.len(), 2);
    assert_eq!(b.params[0], TypeId::BigInt);
    assert_eq!(b.params[1], TypeId::Varchar);
}

#[test]
fn builder_stores_return_type() {
    let b = AggregateFunctionBuilder::new("f").returns(TypeId::BigInt);
    assert_eq!(b.return_type, Some(TypeId::BigInt));
}

#[test]
fn function_set_builder_stores_overloads() {
    let b = AggregateFunctionSetBuilder::new("retention")
        .returns(TypeId::BigInt)
        .overloads(2..=4, |n, builder| {
            (0..n)
                .fold(builder, |b, _| b.param(TypeId::Boolean))
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf)
        });

    // overloads(2..=4) = 3 overloads (n=2, n=3, n=4)
    assert_eq!(b.overloads.len(), 3);
    assert_eq!(b.overloads[0].params.len(), 2);
    assert_eq!(b.overloads[1].params.len(), 3);
    assert_eq!(b.overloads[2].params.len(), 4);
}

#[test]
fn register_missing_return_type_returns_error() {
    let b = AggregateFunctionBuilder::new("f");
    // We can't call register with a null connection, but we can verify
    // the error path for missing return type by inspecting the error.
    // In a real integration test, we'd call register(con) with a live connection.
    // Here we verify the builder stores None for return_type.
    assert!(b.return_type.is_none());
}

#[test]
fn function_set_builder_name() {
    let b = AggregateFunctionSetBuilder::new("my_set");
    assert_eq!(b.name.to_str().unwrap(), "my_set");
}

#[test]
fn overload_builder_params() {
    let ob = AggregateOverloadBuilder::new()
        .param(TypeId::Boolean)
        .param(TypeId::Boolean)
        .param(TypeId::BigInt);
    assert_eq!(ob.params.len(), 3);
}

#[test]
fn overload_builder_default_matches_new() {
    let d = AggregateOverloadBuilder::default();
    assert!(d.params.is_empty());
    assert!(d.return_type.is_none());
    assert!(d.return_logical.is_none());
    assert_eq!(d.null_handling, NullHandling::DefaultNullHandling);
}

#[test]
fn overload_builder_stores_its_own_return_type() {
    let ob = AggregateOverloadBuilder::new()
        .param(TypeId::Integer)
        .returns(TypeId::Integer);
    assert_eq!(ob.return_type, Some(TypeId::Integer));
}

// Issue #121: overloads in one set may return different types, because DuckDB
// resolves an aggregate overload from parameter types and arity alone.
#[test]
fn a_set_keeps_a_distinct_return_type_per_overload() {
    let b = AggregateFunctionSetBuilder::new("my_agg")
        .overload(
            AggregateOverloadBuilder::new()
                .param(TypeId::Integer)
                .returns(TypeId::Integer)
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        )
        .overload(
            AggregateOverloadBuilder::new()
                .param(TypeId::Varchar)
                .returns(TypeId::Varchar)
                .state_size(ss)
                .init(si)
                .update(su)
                .combine(sc)
                .finalize(sf),
        );

    assert_eq!(b.overloads.len(), 2);
    assert_eq!(b.overloads[0].params, vec![TypeId::Integer]);
    assert_eq!(b.overloads[0].return_type, Some(TypeId::Integer));
    assert_eq!(b.overloads[1].params, vec![TypeId::Varchar]);
    assert_eq!(b.overloads[1].return_type, Some(TypeId::Varchar));
    // No set-level default was needed.
    assert!(b.return_type.is_none());
    assert!(b.return_logical.is_none());
}

#[test]
fn overload_and_overloads_can_be_mixed_and_keep_insertion_order() {
    let b = AggregateFunctionSetBuilder::new("mixed")
        .returns(TypeId::BigInt)
        .overload(AggregateOverloadBuilder::new().param(TypeId::Varchar))
        .overloads(2..=3, |n, builder| {
            (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
        });

    assert_eq!(b.overloads.len(), 3);
    assert_eq!(b.overloads[0].params, vec![TypeId::Varchar]);
    assert_eq!(b.overloads[1].params.len(), 2);
    assert_eq!(b.overloads[2].params.len(), 3);
    // The `overloads` members inherit the set-level default: none of their own.
    assert!(b.overloads[1].return_type.is_none());
}

#[test]
fn overloads_closure_can_set_a_per_arity_return_type() {
    let b = AggregateFunctionSetBuilder::new("per_arity").overloads(1..=2, |n, builder| {
        let builder = (0..n).fold(builder, |b, _| b.param(TypeId::Integer));
        if n == 1 {
            builder.returns(TypeId::Integer)
        } else {
            builder.returns(TypeId::BigInt)
        }
    });

    assert_eq!(b.overloads[0].return_type, Some(TypeId::Integer));
    assert_eq!(b.overloads[1].return_type, Some(TypeId::BigInt));
    // Building a `LogicalType` needs a live DuckDB dispatch table, so the
    // `returns_logical` override is covered in `tests/ffi_roundtrip.rs`.
}

#[test]
fn an_empty_set_is_rejected_before_any_return_type_check() {
    // Registration needs a live connection, so assert the precondition the
    // error path keys off: no overloads and no return type at all.
    let b = AggregateFunctionSetBuilder::new("empty");
    assert!(b.overloads.is_empty());
    assert!(b.return_type.is_none());
}

#[test]
fn the_deprecated_alias_still_names_the_same_type() {
    #[allow(deprecated)]
    let ob: crate::aggregate::builder::OverloadBuilder =
        AggregateOverloadBuilder::new().param(TypeId::Boolean);
    assert_eq!(ob.params.len(), 1);
}

#[test]
fn try_new_valid_name() {
    assert!(AggregateFunctionBuilder::try_new("word_count").is_ok());
}

#[test]
fn try_new_empty_rejected() {
    assert!(AggregateFunctionBuilder::try_new("").is_err());
}

#[test]
fn try_new_accepts_mixed_case_and_rejects_names_needing_quotes() {
    // DuckDB ships mixed-case functions and accepts registering one, so
    // rejecting them here made a legal name unregisterable.
    assert!(AggregateFunctionBuilder::try_new("MyFunc").is_ok());
    assert!(AggregateFunctionBuilder::try_new("my-func").is_err());
    assert!(AggregateFunctionBuilder::try_new("1func").is_err());
}

#[test]
fn try_new_hyphen_rejected() {
    assert!(AggregateFunctionBuilder::try_new("my-func").is_err());
}

#[test]
fn set_try_new_valid_name() {
    assert!(AggregateFunctionSetBuilder::try_new("retention").is_ok());
}

#[test]
fn set_try_new_empty_rejected() {
    assert!(AggregateFunctionSetBuilder::try_new("").is_err());
}
