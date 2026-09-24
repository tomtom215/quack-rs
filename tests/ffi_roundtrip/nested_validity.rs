// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `VectorWriter::set_valid` on nested rows, against a live `DuckDB`.

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::complex::ArrayVector;
use quack_rs::vector::{StructWriter, VectorWriter};

use super::Fixture;

// NULL every row, then mark the even rows valid again and write their field.
quack_rs::scalar_callback!(struct_null_then_fill, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut fields = unsafe { StructWriter::new(output, 1) };
    unsafe { parent.set_null_range(0..chunk.size()) };
    for row in 0..chunk.size() {
        let x = unsafe { reader.read_i64(row) };
        if x % 2 == 0 {
            unsafe { parent.set_valid(row) };
            unsafe { fields.write_i64(row, 0, x) };
        }
    }
});

// The same with `BIGINT[2]`: NULL every row, then fill the even rows.
quack_rs::scalar_callback!(array_null_then_fill, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut elements = unsafe { VectorWriter::from_vector(ArrayVector::get_child(output)) };
    unsafe { parent.set_null_range(0..chunk.size()) };
    for row in 0..chunk.size() {
        let x = unsafe { reader.read_i64(row) };
        if x % 2 == 0 {
            unsafe { parent.set_valid(row) };
            unsafe {
                elements.write_i64(row * 2, x);
                elements.write_i64(row * 2 + 1, -x);
            }
        }
    }
});

// Write a row's field NULL, then mark the (already valid) row valid.
quack_rs::scalar_callback!(field_null_then_valid, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut parent = unsafe { VectorWriter::from_vector(output) };
    let mut fields = unsafe { StructWriter::new(output, 1) };
    for row in 0..chunk.size() {
        unsafe { fields.set_null(row, 0) };
        unsafe { parent.set_valid(row) };
    }
});

/// The fourth audit's V4. `set_null` on a `STRUCT` or `ARRAY` row also
/// NULLs every field or element below it, as `DuckDB`'s own
/// `FlatVector::SetNull` does, but `set_valid` restored only the row's own
/// bit — although its docs said it undid `set_null`. Fields written after it
/// read back NULL: `(f(4)).a` was NULL and `f(4)` rendered `{'a': NULL}`.
#[test]
fn set_valid_undoes_set_null_below_a_struct_or_array_row() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::new("struct_null_then_fill")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::struct_type(&[("a", TypeId::BigInt)]))
            .function(struct_null_then_fill)
            .register(fx.con())
            .expect("register struct_null_then_fill");
        ScalarFunctionBuilder::new("array_null_then_fill")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::array(TypeId::BigInt, 2))
            .function(array_null_then_fill)
            .register(fx.con())
            .expect("register array_null_then_fill");
    }
    let rendered = |sql: &str| {
        fx.scalar(sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .expect("one non-NULL row")
    };
    assert_eq!(
        rendered(
            "SELECT string_agg(coalesce(struct_null_then_fill(x)::VARCHAR, 'NULL'), ' ' \
             ORDER BY x) FROM range(6) t(x)"
        ),
        "{'a': 0} NULL {'a': 2} NULL {'a': 4} NULL"
    );
    assert_eq!(
        rendered(
            "SELECT string_agg(coalesce((struct_null_then_fill(x)).a::VARCHAR, 'NULL'), ' ' \
             ORDER BY x) FROM range(6) t(x)"
        ),
        "0 NULL 2 NULL 4 NULL"
    );
    assert_eq!(
        rendered(
            "SELECT string_agg(coalesce(array_null_then_fill(x)::VARCHAR, 'NULL'), ' ' \
             ORDER BY x) FROM range(4) t(x)"
        ),
        "[0, 0] NULL [2, -2] NULL"
    );
}

/// `set_valid` on a row that is already valid leaves the fields alone, so a
/// field NULL written before it survives.
#[test]
fn set_valid_on_a_valid_row_keeps_its_field_nulls() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::new("field_null_then_valid")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::struct_type(&[("a", TypeId::BigInt)]))
            .function(field_null_then_valid)
            .register(fx.con())
            .expect("register field_null_then_valid");
    }
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM range(3000) t(x) \
             WHERE field_null_then_valid(x) IS NOT NULL AND (field_null_then_valid(x)).a IS NULL",
            |r, i| unsafe { r.read_i64(i) },
        ),
        Some(3000)
    );
}
