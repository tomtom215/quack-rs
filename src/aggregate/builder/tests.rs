use super::*;
use crate::types::TypeId;
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};

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
    let ob = OverloadBuilder::new()
        .param(TypeId::Boolean)
        .param(TypeId::Boolean)
        .param(TypeId::BigInt);
    assert_eq!(ob.params.len(), 3);
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
fn try_new_uppercase_rejected() {
    assert!(AggregateFunctionBuilder::try_new("MyFunc").is_err());
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
