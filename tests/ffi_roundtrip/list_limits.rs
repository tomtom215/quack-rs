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

// One ordinary row starts the builder (which applies the child type's
// ceiling), then the caller raises the limit, then a row past the ceiling.
quack_rs::scalar_callback!(limit_raised_after_start, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) };
    // SAFETY: row 0 is in the chunk (the query passes one row); the closure
    // writes the one element it was given.
    unsafe { builder.push_row(0, 1, |w, base| w.write_i64(base, 7)) };
    builder = builder.with_element_limit(usize::MAX);
    let n = unsafe { reader.read_i64(0) } as usize;
    // SAFETY: as above; this row is past the limit, so the closure never runs.
    unsafe { builder.push_row(0, n, |_, _| unreachable!("past the limit")) };
    assert!(builder.overflowed());
    unsafe { builder.finish() };
});

/// `with_element_limit` after the first row used to replace the child type's
/// ceiling, which `start` applies once, with 2^37 elements: a `BIGINT` row of
/// 2^34 + 1 then reached `duckdb_list_vector_reserve` and aborted the process.
#[test]
fn raising_the_limit_after_the_first_row_keeps_duckdbs_ceiling() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("limit_raised")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::BigInt))
            .function(limit_raised_after_start)
            .register(fx.con())
            .expect("register");
    }
    let sql = format!(
        "SELECT (limit_raised({}) IS NULL)::VARCHAR",
        (1_u64 << 34) + 1
    );
    assert_eq!(
        fx.scalar(&sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .as_deref(),
        Some("true")
    );
}

// Each input row is a length; a builder limited to 3 elements writes the rows
// that fit and reports the overflow through `try_finish`.
quack_rs::scalar_callback!(limited_to_three, |info, input, output| {
    let info = unsafe { quack_rs::scalar::ScalarFunctionInfo::new(info) };
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) }.with_element_limit(3);
    for row in 0..chunk.size() {
        let n = usize::try_from(unsafe { reader.read_i64(row) }).unwrap_or(0);
        // SAFETY: `row` is in the chunk; the closure writes the `n` elements
        // it is given.
        unsafe {
            builder.push_row(row, n, |w, base| {
                for i in 0..n {
                    w.write_i64(base + i, 1);
                }
            });
        }
    }
    if let Err(e) = unsafe { builder.try_finish() } {
        info.set_error(&e.to_string());
    }
});

/// QRS-012 (`AUDIT.md` section 16): `finish` returns nothing, so a row
/// refused for the element limit was a silent NULL unless the caller asked
/// `overflowed`. `try_finish` reports it, and stays quiet when every row fits.
#[test]
fn try_finish_reports_a_row_refused_for_the_element_limit() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("limited_to_three")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::BigInt))
            .function(limited_to_three)
            .register(fx.con())
            .expect("register");
    }
    // Three elements in all: exactly the limit.
    let fits = "SELECT string_agg(l::VARCHAR, ';' ORDER BY n) \
                FROM (SELECT n, limited_to_three(n) AS l FROM (VALUES (1), (2)) t(n))";
    assert_eq!(
        fx.scalar(fits, |r, i| unsafe { r.read_str(i).to_owned() })
            .as_deref(),
        Some("[1];[1, 1]")
    );
    // SAFETY: the connection is open.
    let err = unsafe {
        quack_rs::query::query(
            fx.con(),
            "SELECT limited_to_three(n) FROM (VALUES (2), (2)) t(n)",
        )
    }
    .err()
    .expect("the second row passes the limit");
    assert!(
        err.as_str().contains("past its limit of 3 element(s)"),
        "{}",
        err.as_str()
    );
}
