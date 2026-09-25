// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Which buffers a `LIST` reserve moves.
//!
//! `duckdb_list_vector_reserve` resizes the list's child with
//! `Vector::Resize`, which walks the child's STRUCT fields and ARRAY elements
//! and reallocates each one's data and validity buffer, stopping at a nested
//! `LIST` (whose child is a buffer of its own). A writer or bitmap that
//! caches a pointer into any vector the walk reaches dangles after the
//! reserve, not only one on the list's direct child; the `# Safety`
//! sections of `VectorWriter::from_vector`, `StructWriter::new`,
//! `StructVector::field_writer` and `ValidityBitmap::ensure_writable` say so.

use libduckdb_sys::{
    duckdb_array_vector_get_child, duckdb_list_vector_get_child, duckdb_list_vector_reserve,
    duckdb_struct_vector_get_child, duckdb_vector, duckdb_vector_ensure_validity_writable,
    duckdb_vector_get_data, duckdb_vector_get_validity,
};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::OwnedVector;

/// The data and validity addresses of `v`, allocating its mask first.
fn buffers(v: duckdb_vector) -> (usize, usize) {
    // SAFETY: `v` is a live vector of the `OwnedVector` below.
    unsafe {
        duckdb_vector_ensure_validity_writable(v);
        (
            duckdb_vector_get_data(v) as usize,
            duckdb_vector_get_validity(v) as usize,
        )
    }
}

#[test]
fn a_list_reserve_moves_every_buffer_below_its_child_up_to_the_next_list() {
    // Initialises the C API dispatch table.
    let _fx = super::Fixture::open();
    // LIST<STRUCT<a BIGINT, b INTEGER[2], c BIGINT[]>>
    let element = LogicalType::struct_type_from_logical(&[
        ("a", LogicalType::new(TypeId::BigInt)),
        ("b", LogicalType::array(TypeId::Integer, 2)),
        ("c", LogicalType::list(TypeId::BigInt)),
    ]);
    let list = OwnedVector::new(&LogicalType::list_from_logical(&element), 1).expect("vector");
    // The child, field a, field b, b's elements, field c, c's elements.
    // SAFETY: each handle is a child of the live `list`, of the type above.
    let vectors = unsafe {
        let strukt = duckdb_list_vector_get_child(list.as_raw());
        let b = duckdb_struct_vector_get_child(strukt, 1);
        let c = duckdb_struct_vector_get_child(strukt, 2);
        [
            strukt,
            duckdb_struct_vector_get_child(strukt, 0),
            b,
            duckdb_array_vector_get_child(b),
            c,
            duckdb_list_vector_get_child(c),
        ]
    };
    let before = vectors.map(buffers);
    // SAFETY: `list` is a live LIST vector; 1 << 16 elements is far below
    // the byte ceiling for this element type.
    let state = unsafe { duckdb_list_vector_reserve(list.as_raw(), 1 << 16) };
    assert_eq!(state, libduckdb_sys::duckdb_state_DuckDBSuccess);
    let after = vectors.map(buffers);
    let moved: Vec<(bool, bool)> = before
        .iter()
        .zip(&after)
        .map(|(x, y)| (x.0 != y.0, x.1 != y.1))
        .collect();
    // (data moved, validity moved), in the order of `vectors`.
    assert_eq!(
        moved,
        vec![
            (false, true), // a STRUCT has no data buffer of its own
            (true, true),
            (false, true), // nor has an ARRAY
            (true, true),
            (true, true), // c's list entries
            (false, false),
        ]
    );
}
