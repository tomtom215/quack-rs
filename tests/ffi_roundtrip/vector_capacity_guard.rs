// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `OwnedVector::new` refuses a capacity whose nested `ARRAY` child would
//! exceed `DuckDB`'s per-vector bound, whichever composite type reaches it.
//!
//! `max_elements_per_vector` walks the type with one arm per composite kind.
//! The existing tests (`src/vector/ops.rs`) reach an `ARRAY` through `ARRAY`,
//! `STRUCT` and `LIST` only: deleting the `MAP` arm, the `UNION` arm, or the
//! key or value half of the `MAP` arm left every test green (found by
//! mutation testing, `delete match arm MAP|UNION in max_elements_per_vector`),
//! and with those arms gone an `ARRAY(INT, 99999)` inside a `MAP` or a
//! `UNION` is allocated at a capacity whose size computation `DuckDB` does
//! with an unchecked multiply.

use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::OwnedVector;

use super::Fixture;

/// 2^21 rows of an `ARRAY(INT, 99_999)` is 2.1e11 elements, over the 2^37
/// bound; 4 rows is far under it.
const TOO_MANY_ROWS: usize = 1 << 21;

#[test]
fn an_array_inside_any_composite_counts_against_the_capacity_bound() {
    let _fx = Fixture::open();
    let array = LogicalType::array(TypeId::Integer, 99_999);
    let int = LogicalType::new(TypeId::Integer);

    let shapes: Vec<(&str, LogicalType)> = vec![
        ("ARRAY", LogicalType::array_from_logical(&int, 99_999)),
        ("LIST(ARRAY)", LogicalType::list_from_logical(&array)),
        (
            "MAP(INT, ARRAY)",
            LogicalType::map_from_logical(&int, &array),
        ),
        (
            "MAP(ARRAY, INT)",
            LogicalType::map_from_logical(&array, &int),
        ),
        (
            "STRUCT(INT, ARRAY)",
            LogicalType::struct_type_from_logical(&[
                ("a", LogicalType::new(TypeId::Integer)),
                ("b", LogicalType::array(TypeId::Integer, 99_999)),
            ]),
        ),
        (
            "UNION(INT, ARRAY)",
            LogicalType::union_type_from_logical(&[
                ("a", LogicalType::new(TypeId::Integer)),
                ("b", LogicalType::array(TypeId::Integer, 99_999)),
            ]),
        ),
        (
            "UNION(ARRAY, INT)",
            LogicalType::union_type_from_logical(&[
                ("a", LogicalType::array(TypeId::Integer, 99_999)),
                ("b", LogicalType::new(TypeId::Integer)),
            ]),
        ),
    ];
    for (name, shape) in &shapes {
        let err = OwnedVector::new(shape, TOO_MANY_ROWS)
            .err()
            .unwrap_or_else(|| panic!("{name}: {TOO_MANY_ROWS} rows must be refused"));
        assert!(err.as_str().contains("out of range"), "{name}: {err}");
        assert!(
            OwnedVector::new(shape, 4).is_ok(),
            "{name}: a small capacity must still be accepted"
        );
    }
}
