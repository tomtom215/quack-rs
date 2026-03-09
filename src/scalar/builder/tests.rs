use super::*;
use crate::types::TypeId;
use libduckdb_sys::{duckdb_data_chunk, duckdb_function_info, duckdb_vector};

#[test]
fn builder_stores_name() {
    let b = ScalarFunctionBuilder::new("my_scalar");
    assert_eq!(b.name.to_str().unwrap(), "my_scalar");
}

#[test]
fn builder_stores_params() {
    let b = ScalarFunctionBuilder::new("f")
        .param(TypeId::BigInt)
        .param(TypeId::Varchar);
    assert_eq!(b.params.len(), 2);
    assert_eq!(b.params[0], TypeId::BigInt);
    assert_eq!(b.params[1], TypeId::Varchar);
}

#[test]
fn builder_stores_return_type() {
    let b = ScalarFunctionBuilder::new("f").returns(TypeId::Double);
    assert_eq!(b.return_type, Some(TypeId::Double));
}

#[test]
fn builder_missing_return_type() {
    let b = ScalarFunctionBuilder::new("f");
    assert!(b.return_type.is_none());
}

#[test]
fn builder_missing_function() {
    let b = ScalarFunctionBuilder::new("f");
    assert!(b.function.is_none());
}

#[test]
fn builder_stores_function() {
    unsafe extern "C" fn my_func(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: duckdb_vector,
    ) {
    }

    let b = ScalarFunctionBuilder::new("f").function(my_func);
    assert!(b.function.is_some());
}

#[test]
fn try_new_valid_name() {
    let b = ScalarFunctionBuilder::try_new("word_count");
    assert!(b.is_ok());
}

#[test]
fn try_new_empty_rejected() {
    assert!(ScalarFunctionBuilder::try_new("").is_err());
}

#[test]
fn try_new_uppercase_rejected() {
    assert!(ScalarFunctionBuilder::try_new("MyFunc").is_err());
}

#[test]
fn try_new_hyphen_rejected() {
    assert!(ScalarFunctionBuilder::try_new("my-func").is_err());
}

// --- ScalarFunctionSetBuilder tests ---

#[test]
fn set_builder_stores_name() {
    let b = ScalarFunctionSetBuilder::new("my_set");
    assert_eq!(b.name.to_str().unwrap(), "my_set");
}

#[test]
fn set_builder_stores_overloads() {
    unsafe extern "C" fn f1(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
    unsafe extern "C" fn f2(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let b = ScalarFunctionSetBuilder::new("my_add")
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Integer)
                .param(TypeId::Integer)
                .returns(TypeId::Integer)
                .function(f1),
        )
        .overload(
            ScalarOverloadBuilder::new()
                .param(TypeId::Double)
                .param(TypeId::Double)
                .returns(TypeId::Double)
                .function(f2),
        );

    assert_eq!(b.overloads.len(), 2);
    assert_eq!(b.overloads[0].params.len(), 2);
    assert_eq!(b.overloads[1].params.len(), 2);
}

#[test]
fn set_try_new_valid_name() {
    assert!(ScalarFunctionSetBuilder::try_new("my_add").is_ok());
}

#[test]
fn set_try_new_empty_rejected() {
    assert!(ScalarFunctionSetBuilder::try_new("").is_err());
}

#[test]
fn overload_builder_default() {
    let ob = ScalarOverloadBuilder::default();
    assert!(ob.params.is_empty());
    assert!(ob.return_type.is_none());
    assert!(ob.function.is_none());
}

#[test]
fn overload_builder_stores_fields() {
    unsafe extern "C" fn f(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}

    let ob = ScalarOverloadBuilder::new()
        .param(TypeId::BigInt)
        .returns(TypeId::Varchar)
        .function(f);
    assert_eq!(ob.params.len(), 1);
    assert_eq!(ob.return_type, Some(TypeId::Varchar));
    assert!(ob.function.is_some());
}
