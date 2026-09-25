// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Accessors the fifth audit's end-to-end mutation run found untested:
//! `StructWriter::child_vector` / `child_list_vector` and
//! `InMemoryDb::execute`'s row count each survived returning a constant.

use quack_rs::testing::InMemoryDb;

/// `execute` reports the rows a statement changed.
#[test]
fn in_memory_db_execute_returns_the_rows_changed() {
    let db = InMemoryDb::open().expect("open");
    db.execute_batch("CREATE TABLE r (a INTEGER)")
        .expect("create");
    assert_eq!(db.execute("INSERT INTO r VALUES (1), (2), (3)"), Ok(3));
    assert_eq!(db.execute("DELETE FROM r WHERE a > 1"), Ok(2));
}

/// Each field handle is the struct vector's own child, as
/// `duckdb_struct_vector_get_child` returns it.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn struct_writer_child_vectors_are_the_structs_fields() {
    use quack_rs::types::{LogicalType, TypeId};
    use quack_rs::vector::{OwnedVector, StructWriter};

    let _db = InMemoryDb::open().expect("initialise the dispatch table");
    let ty = LogicalType::struct_type_from_logical(&[
        ("a", LogicalType::new(TypeId::BigInt)),
        ("b", LogicalType::list(TypeId::Integer)),
    ]);
    let vector = OwnedVector::new(&ty, 4).expect("vector");
    // SAFETY: a live STRUCT vector with two fields.
    let writer = unsafe { StructWriter::new(vector.as_raw(), 2) };
    for i in 0..2 {
        // SAFETY: `i` is a field index of the live struct vector.
        let child = unsafe { libduckdb_sys::duckdb_struct_vector_get_child(vector.as_raw(), i) };
        assert_eq!(writer.child_vector(i as usize), child, "field {i}");
    }
    // SAFETY: field 1 is the LIST field.
    let list = unsafe { libduckdb_sys::duckdb_struct_vector_get_child(vector.as_raw(), 1) };
    assert_eq!(writer.child_list_vector(1), list);
}
