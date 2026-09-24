// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ListBuilder`'s ceiling is `DuckDB`'s: 2^37 **bytes** per child buffer
//! (`Vector::Resize`), not 2^37 elements, checked after the reservation is
//! rounded up to a power of two. A row past it is written as NULL.
//!
//! Before the fix the builder allowed 2^37 elements of any type, so a
//! `BIGINT` list of 2^34 + 1 elements — or an `INTEGER[1000]` list of
//! 2^25 + 1 — reached `duckdb_list_vector_reserve`, whose
//! `OutOfRangeException` escaped the C API and aborted the process ("Rust
//! cannot catch foreign exceptions"). Nothing is allocated for a refused row,
//! so these tests need no memory. Every length here is past the limit: a
//! length under it would reserve that much memory, and the callback writes no
//! elements.

use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::ListBuilder;

use super::Fixture;

quack_rs::scalar_callback!(oversized_list, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let n = unsafe { reader.read_i64(row) } as usize;
        // SAFETY: `row` is in the chunk; the closure is only called for a row
        // the builder accepted, and every length passed here is past the limit.
        unsafe { builder.push_row(row, n, |_, _| unreachable!("every row is past the limit")) };
    }
    assert!(builder.overflowed() || chunk.size() == 0);
    unsafe { builder.finish() };
});

fn register(fx: &Fixture, name: &str, child: &LogicalType) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list_from_logical(child))
            .function(oversized_list)
            .register(fx.con())
            .expect("register");
    }
}

/// The largest power of two whose elements fit in 2^37 bytes, plus one: the
/// first refused length.
const CASES: &[(&str, u64)] = &[
    ("of_bigint", (1 << 34) + 1),
    ("of_varchar", (1 << 33) + 1),
    // A STRUCT's widest field decides: HUGEINT, 16 bytes.
    ("of_struct", (1 << 33) + 1),
    // An ARRAY multiplies: 1000 INTEGERs are 4000 bytes per element, and
    // `DuckDB` rounds a reservation up to a power of two before checking it
    // (`VectorListBuffer::Reserve`), so 2^25 + 1 elements are checked as
    // 2^26: 268 GB.
    ("of_array", (1 << 25) + 1),
    // One-byte elements: the limit is 2^37 itself.
    ("of_boolean", (1 << 37) + 1),
];

#[test]
fn a_row_past_duckdbs_byte_ceiling_is_null_not_an_abort() {
    let fx = Fixture::open();
    let child = |name: &str| match name {
        "of_bigint" => LogicalType::new(TypeId::BigInt),
        "of_varchar" => LogicalType::new(TypeId::Varchar),
        "of_struct" => LogicalType::struct_type_from_logical(&[
            ("a", LogicalType::new(TypeId::TinyInt)),
            ("b", LogicalType::new(TypeId::HugeInt)),
        ]),
        "of_array" => LogicalType::array(TypeId::Integer, 1000),
        _ => LogicalType::new(TypeId::Boolean),
    };
    for (name, len) in CASES {
        register(&fx, name, &child(name));
        let sql = format!("SELECT ({name}({len}) IS NULL)::VARCHAR");
        assert_eq!(
            fx.scalar(&sql, |r, i| unsafe { r.read_str(i).to_owned() })
                .as_deref(),
            Some("true"),
            "{sql}"
        );
    }
}
