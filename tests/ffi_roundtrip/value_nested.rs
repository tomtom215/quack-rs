// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Nested `Value` construction and inspection against a live `DuckDB`.

use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;

use super::Fixture;

/// The audit's F-V6: `list_value` / `array_value` refused a `LIST` / `ARRAY`
/// element type outright, to catch callers passing the container type by
/// mistake — which also refused every legitimate list of lists. `DuckDB`
/// builds them (`Value::LIST` casts each item to the element type).
#[test]
fn lists_and_arrays_of_lists_and_arrays_can_be_built() {
    let _fx = Fixture::open();
    let bigint = LogicalType::new(TypeId::BigInt);
    let inner = Value::list_value(&bigint, &[Value::bigint(1), Value::bigint(2)]).expect("[1, 2]");
    let list_of_lists =
        Value::list_value(&LogicalType::list(TypeId::BigInt), &[inner]).expect("[[1, 2]]");
    assert_eq!(list_of_lists.as_str().expect("renders"), "[[1, 2]]");
    assert_eq!(list_of_lists.list_len(), 1);
    assert_eq!(
        list_of_lists.list_child(0).map(|c| c.list_len()),
        Some(2),
        "the child is itself a list"
    );

    let row = Value::array_value(&bigint, &[Value::bigint(1), Value::bigint(2)]).expect("[1, 2]");
    let array_of_arrays =
        Value::array_value(&LogicalType::array(TypeId::BigInt, 2), &[row]).expect("[[1, 2]]");
    assert_eq!(array_of_arrays.as_str().expect("renders"), "[[1, 2]]");

    // An empty list of lists is fine too.
    let empty = Value::list_value(&LogicalType::list(TypeId::Varchar), &[]).expect("[]");
    assert_eq!(empty.as_str().expect("renders"), "[]");
}

/// The mistake the old check existed for — passing the container type with
/// scalar items — still gets the explanatory error.
#[test]
fn passing_the_container_type_by_mistake_is_still_explained() {
    let _fx = Fixture::open();
    let err = Value::list_value(
        &LogicalType::list(TypeId::BigInt),
        &[Value::bigint(1), Value::bigint(2)],
    )
    .expect_err("a BIGINT does not cast to BIGINT[]");
    assert!(err.to_string().contains("element"), "{err}");
    let err = Value::array_value(&LogicalType::array(TypeId::BigInt, 2), &[Value::bigint(1)])
        .expect_err("a BIGINT does not cast to BIGINT[2]");
    assert!(err.to_string().contains("element"), "{err}");
}
