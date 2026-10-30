// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `BIT` and `BIGNUM` values (eighth review, APIGAPS-3).
//!
//! `duckdb_get_bit` and `duckdb_get_bignum` abort the process on a SQL
//! `NULL` and on a value that does not cast (`'abc'`, a `BLOB`, `inf`), and
//! `duckdb_get_bit` segfaults on a null handle (reproduced on 1.5.6 before
//! the wrappers were written). Every one of those inputs is run here through
//! the wrappers, which must answer `None`.

use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::{Value, UNRENDERABLE};

use super::Fixture;

/// Inputs neither type can be read from: `NULL`s (untyped and typed), a null
/// handle, text that is not a number or a bit string, and a `LIST`.
fn unreadable(typed_null: TypeId) -> Vec<(&'static str, Value)> {
    vec![
        ("SQL NULL", Value::null_value()),
        // SAFETY: a null handle is part of `Value`'s contract.
        ("null handle", unsafe {
            Value::from_raw(std::ptr::null_mut())
        }),
        ("VARCHAR 'abc'", Value::varchar("abc")),
        (
            "LIST",
            Value::list_value(&LogicalType::new(TypeId::Integer), &[Value::integer(1)])
                .expect("[1]"),
        ),
        (
            "typed NULL",
            Value::null_value()
                .try_cast(&LogicalType::new(typed_null))
                .expect("a typed NULL"),
        ),
    ]
}

#[test]
fn as_bit_refuses_every_input_duckdb_get_bit_aborts_on() {
    let _fx = Fixture::open();
    for (label, value) in unreadable(TypeId::Bit) {
        assert_eq!(value.as_bit(), None, "as_bit of {label}");
    }
}

#[test]
fn as_bignum_refuses_every_input_duckdb_get_bignum_aborts_on() {
    let _fx = Fixture::open();
    let mut inputs = unreadable(TypeId::Varint);
    // Also aborting in `duckdb_get_bignum` (probe, 1.5.6): no BLOB cast, and
    // no BIGNUM for a non-finite DOUBLE.
    inputs.push(("BLOB", Value::blob(&[1, 2, 3])));
    inputs.push(("DOUBLE inf", Value::double(f64::INFINITY)));
    inputs.push(("DOUBLE nan", Value::double(f64::NAN)));
    for (label, value) in inputs {
        assert_eq!(value.as_bignum(), None, "as_bignum of {label}");
    }
}

#[test]
fn a_bit_value_round_trips_and_casts_like_sql() {
    let _fx = Fixture::open();
    let bits = [false, true, false, true];
    let value = Value::bit(&bits).expect("'0101'");
    assert_eq!(value.type_id(), Some(TypeId::Bit));
    assert_eq!(value.as_str().expect("renders"), "0101");
    assert_eq!(value.as_bit(), Some(bits.to_vec()));

    // 1 to 20 bits, across byte boundaries, render the way SQL writes them.
    for len in 1..=20_usize {
        let bits: Vec<bool> = (0..len).map(|i| i % 3 == 1).collect();
        let text: String = bits.iter().map(|&b| if b { '1' } else { '0' }).collect();
        let value = Value::bit(&bits).expect("non-empty");
        assert_eq!(value.as_str().expect("renders"), text, "len {len}");
        assert_eq!(value.as_bit(), Some(bits.clone()), "len {len}");
        // And SQL's own cast of the text gives the same bits.
        let from_sql = Value::varchar(&text).as_bit();
        assert_eq!(from_sql, Some(bits), "'{text}'::BIT");
    }

    // Casts: an INTEGER is its 32 bits.
    let five = Value::integer(5).as_bit().expect("INTEGER casts to BIT");
    assert_eq!(five.len(), 32);
    assert_eq!(&five[29..], [true, false, true]);
    assert!(five[..29].iter().all(|b| !b));

    let err = Value::bit(&[]).expect_err("no empty BIT");
    assert!(err.as_str().contains("at least one bit"), "{err}");
}

/// The raw constructor stores any bytes; one with no data byte made
/// `as_str` write 8 characters into a 0-byte buffer (`Bit::ToString`), and
/// one with padding 9 threw. Both are now refused, and read as `None`.
#[test]
fn a_malformed_bit_from_the_raw_constructor_is_neither_rendered_nor_read() {
    let _fx = Fixture::open();
    for bytes in [vec![0_u8], vec![9, 0xff], vec![8, 0x00]] {
        // SAFETY: `bytes` is live for the call; DuckDB copies it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_bit(libduckdb_sys::duckdb_bit {
                data: bytes.as_ptr().cast_mut(),
                size: bytes.len() as libduckdb_sys::idx_t,
            })
        };
        // SAFETY: an owned handle from DuckDB.
        let value = unsafe { Value::from_raw(raw) };
        assert_eq!(value.type_id(), Some(TypeId::Bit));
        assert_eq!(value.as_bit(), None, "{bytes:?}");
        assert_eq!(
            value.as_str().expect_err("refused").as_str(),
            UNRENDERABLE,
            "{bytes:?}"
        );
        assert!(
            value.try_cast(&LogicalType::new(TypeId::Varchar)).is_none(),
            "{bytes:?}"
        );
    }
}

#[test]
fn a_bignum_round_trips_canonically_and_casts_like_sql() {
    let _fx = Fixture::open();
    let two_64 = [1_u8, 0, 0, 0, 0, 0, 0, 0, 0];
    let big = Value::bignum(&two_64, true).expect("-(2^64)");
    assert_eq!(big.type_id(), Some(TypeId::Varint));
    assert_eq!(big.as_str().expect("renders"), "-18446744073709551616");
    assert_eq!(big.as_bignum(), Some((two_64.to_vec(), true)));

    // Leading zeros are dropped; zero is never negative ("-0" before).
    let five = Value::bignum(&[0, 0, 5], true).expect("-5");
    assert_eq!(five.as_str().expect("renders"), "-5");
    assert_eq!(five.as_bignum(), Some((vec![5], true)));
    for (magnitude, negative) in [(&[][..], true), (&[0_u8, 0][..], true), (&[][..], false)] {
        let zero = Value::bignum(magnitude, negative).expect("zero");
        assert_eq!(
            zero.as_str().expect("renders"),
            "0",
            "{magnitude:?} {negative}"
        );
        assert_eq!(zero.as_bignum(), Some((vec![0], false)));
    }

    // Casts follow SQL: integers convert, as does a numeric string.
    assert_eq!(Value::bigint(-5).as_bignum(), Some((vec![5], true)));
    assert_eq!(
        Value::hugeint(i128::MAX)
            .as_bignum()
            .map(|(m, n)| (m.len(), n)),
        Some((16, false))
    );
    assert_eq!(
        Value::varchar("123456789012345678901234567890").as_bignum(),
        Value::bignum(
            &123_456_789_012_345_678_901_234_567_890_u128.to_be_bytes(),
            false
        )
        .expect("BIGNUM")
        .as_bignum()
    );
}
