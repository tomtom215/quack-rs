// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `VectorWriter::decimal` (a `DecimalWriter`) against a real engine, next to
//! the unchecked `write_decimal` it guards.
//!
//! Each function takes a `HUGEINT` and writes it as the unscaled integer of a
//! `DECIMAL(width, scale)`, so any value, in range or not, can reach the
//! writer. Casting the result to `VARCHAR` shows what the engine stored.

use quack_rs::data_chunk::DataChunk;
use quack_rs::query::query;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder, ScalarFunctionInfo};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

// One callback for every width and scale: the `DecimalWriter` reads both from
// the output vector's type. It writes each `HUGEINT` argument and reports a
// refusal (a value too wide, or an output that is not DECIMAL) as the query's
// error.
quack_rs::scalar_callback!(checked, |info, input, output| {
    let info = unsafe { ScalarFunctionInfo::new(info) };
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    let mut decimal = match writer.decimal() {
        Ok(decimal) => decimal,
        Err(e) => {
            info.set_error(&e.to_string());
            return;
        }
    };
    for row in 0..chunk.size() {
        let value = unsafe { reader.read_i128(row) };
        if let Err(e) = unsafe { decimal.write(row, value) } {
            info.set_error(&e.to_string());
            return;
        }
    }
});

/// The same, with the unchecked `write_decimal`.
macro_rules! unchecked {
    ($name:ident, $width:expr) => {
        quack_rs::scalar_callback!($name, |_info, input, output| {
            let chunk = unsafe { DataChunk::from_raw(input) };
            let reader = unsafe { chunk.reader(0) };
            let mut writer = unsafe { VectorWriter::from_vector(output) };
            for row in 0..chunk.size() {
                let value = unsafe { reader.read_i128(row) };
                unsafe { writer.write_decimal(row, $width, value) };
            }
        });
    };
}

unchecked!(unchecked_4, 4);
unchecked!(unchecked_9, 9);
unchecked!(unchecked_18, 18);

fn register(fx: &Fixture, name: &str, width: u8, callback: ScalarFn) {
    register_as(fx, name, LogicalType::decimal(width, 0), callback);
}

fn register_as(fx: &Fixture, name: &str, returns: LogicalType, callback: ScalarFn) {
    // SAFETY: the connection is open and the callback matches the signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("name")
            .param(TypeId::HugeInt)
            .returns_logical(returns)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

/// What `name(value)` stores, read back as text.
fn stored(fx: &Fixture, name: &str, value: &str) -> String {
    fx.scalar(
        &format!("SELECT {name}({value}::HUGEINT)::VARCHAR"),
        |r, i| unsafe { r.read_str(i).to_owned() },
    )
    .expect("not NULL")
}

/// The error `name(value)` raises, or a panic if it succeeds.
fn refused(fx: &Fixture, name: &str, value: &str) -> String {
    // SAFETY: the connection is open.
    let result = unsafe { query(fx.con(), &format!("SELECT {name}({value}::HUGEINT)")) };
    result
        .err()
        .unwrap_or_else(|| panic!("{name}({value}) must be refused"))
        .as_str()
        .to_owned()
}

/// Both sides of every storage boundary (1 | 4/5 | 9/10 | 18/19 | 38): the
/// largest and smallest value each width holds is written as it is, and the
/// first value past either end is refused, with nothing written.
#[test]
fn decimal_writer_accepts_the_range_and_refuses_one_past_it_at_every_width() {
    let fx = Fixture::open();
    for width in [1_u8, 4, 5, 9, 10, 18, 19, 38] {
        let name = format!("checked_{width}");
        register(&fx, &name, width, checked);
        let max = "9".repeat(usize::from(width));
        for value in [max.clone(), format!("-{max}"), "0".to_owned()] {
            assert_eq!(stored(&fx, &name, &value), value, "{name}({value})");
        }
        let over = format!("1{}", "0".repeat(usize::from(width)));
        for value in [over.clone(), format!("-{over}")] {
            let message = refused(&fx, &name, &value);
            assert!(
                message.contains(&format!("more than {width} digits")),
                "{name}({value}): {message}"
            );
        }
    }
}

/// The hazard `DecimalWriter` exists for, on a real engine: the unchecked
/// writer wraps a value its physical integer cannot hold, so the query returns
/// a plausible, wrong number with no error, and the checked writer refuses the
/// same value.
#[test]
fn write_decimal_wraps_a_value_that_the_decimal_writer_refuses() {
    let fx = Fixture::open();
    register(&fx, "unchecked_4", 4, unchecked_4);
    register(&fx, "unchecked_9", 9, unchecked_9);
    register(&fx, "unchecked_18", 18, unchecked_18);
    register(&fx, "checked_4", 4, checked);
    register(&fx, "checked_9", 9, checked);
    register(&fx, "checked_18", 18, checked);

    // (width, input, what `as i16`/`as i32`/`as i64` leaves of it)
    for (width, input, wrapped) in [
        (4_u8, "70000", "4464"),           // 70000 - 65536
        (9, "4294967301", "5"),            // 2^32 + 5
        (18, "18446744073709551623", "7"), // 2^64 + 7
    ] {
        assert_eq!(
            stored(&fx, &format!("unchecked_{width}"), input),
            wrapped,
            "write_decimal({input}) at width {width}"
        );
        let message = refused(&fx, &format!("checked_{width}"), input);
        assert!(
            message.contains(&format!("more than {width} digits")),
            "DecimalWriter::write({input}) at width {width}: {message}"
        );
    }
}

/// A value that fits the physical integer but has too many digits is not
/// wrapped; the unchecked writer stores it, one digit past the declared width.
/// Whatever `DuckDB` renders for it, the checked writer never produces it.
#[test]
fn decimal_writer_refuses_a_value_with_too_many_digits_that_still_fits_the_integer() {
    let fx = Fixture::open();
    register(&fx, "unchecked_4", 4, unchecked_4);
    register(&fx, "checked_4", 4, checked);
    assert!(i16::try_from(10_000_i128).is_ok(), "10000 fits an i16");

    let stored_by_unchecked = stored(&fx, "unchecked_4", "10000");
    assert_eq!(
        stored_by_unchecked, "10000",
        "pins the documented hazard: DuckDB does not re-validate a vector it is handed"
    );
    let message = refused(&fx, "checked_4", "10000");
    assert!(message.contains("more than 4 digits"), "{message}");
}

/// QRS-004 (`AUDIT.md` section 16): the width and scale come from the output
/// vector's type, so the same callback writes `DECIMAL(10, 3)` at its own
/// scale and refuses past its own width, with no width or scale argument.
#[test]
fn decimal_writer_takes_width_and_scale_from_the_vectors_type() {
    let fx = Fixture::open();
    register_as(&fx, "checked_10_3", LogicalType::decimal(10, 3), checked);
    assert_eq!(stored(&fx, "checked_10_3", "12345"), "12.345");
    assert_eq!(stored(&fx, "checked_10_3", "-9999999999"), "-9999999.999");
    let message = refused(&fx, "checked_10_3", "10000000000");
    assert!(message.contains("more than 10 digits"), "{message}");
}

/// A vector that is not `DECIMAL` is refused before anything is written: the
/// unchecked writer's width argument would have picked the byte count.
#[test]
fn decimal_writer_refuses_a_vector_that_is_not_decimal() {
    let fx = Fixture::open();
    register_as(
        &fx,
        "checked_int",
        LogicalType::new(TypeId::Integer),
        checked,
    );
    let message = refused(&fx, "checked_int", "1");
    assert!(
        message.contains("the vector's type is INTEGER, not DECIMAL"),
        "{message}"
    );
}
