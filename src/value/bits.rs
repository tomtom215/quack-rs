// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `BIT` and `BIGNUM` values.
//!
//! # Why the C getters are never called on an arbitrary value
//!
//! `duckdb_get_bit` and `duckdb_get_bignum` (`duckdb_value-c.cpp`, identical
//! in 1.4.4, 1.5.6 and 2.0.0 apart from a `malloc` null check 2.0 added) run
//! `DefaultCastAs(BIT | BIGNUM)` and then `StringValue::Get` with no `try`.
//! The cast throws for a value that does not convert (`'abc'`, a `BLOB`, an
//! infinite `DOUBLE`) and `StringValue::Get` throws for a SQL `NULL`; either
//! exception reaches Rust and aborts the process. A null handle is
//! dereferenced. So the getters here only ever hand `DuckDB` a non-null,
//! non-`NULL` value whose type is already the target: the value itself, or a
//! cast copy made by [`Value::try_cast`], which cannot throw.
//!
//! # Why the constructors validate
//!
//! `duckdb_create_bit` and `duckdb_create_bignum` copy whatever bytes they
//! are given. A `BIT` with no data byte or a padding count above 7, or a
//! `BIGNUM` with an empty magnitude, is a value `DuckDB` cannot use: rendering
//! the `BIGNUM` throws (`Bignum::GetByteArray`: "Invalid blob size"), a `BIT`
//! with padding 9 makes `Bit::ToString` allocate a length that wrapped below
//! zero, and one holding only its padding byte makes it write 8 characters
//! into a 0-byte buffer. A leading zero byte or a "negative zero" breaks the
//! canonical form `Bignum::Verify` asserts (debug builds) and `DuckDB`'s
//! byte-wise comparisons rely on. [`Value::bit`] and [`Value::bignum`] build
//! only well-formed values.

use libduckdb_sys::{
    duckdb_bignum, duckdb_bit, duckdb_create_bignum, duckdb_create_bit, duckdb_free,
    duckdb_get_bignum, duckdb_get_bit, idx_t,
};

use super::Value;
use crate::error::ExtensionError;
use crate::types::{LogicalType, TypeId};

/// The largest `BIGNUM` magnitude `DuckDB` stores, in bytes
/// (`Bignum::MAX_DATA_SIZE`): the 3-byte header holds the length in 23 bits.
pub(super) const BIGNUM_MAX_BYTES: usize = 8_388_607;

/// `bits` (first = leftmost) in `DuckDB`'s `BIT` layout: a byte holding the
/// number of padding bits, then the bits right-aligned in whole bytes, with
/// the padding bits at the front of the first data byte set to 1
/// (`Bit::Finalize`). `None` for no bits: SQL has no empty `BIT`.
pub(super) fn encode_bits(bits: &[bool]) -> Option<Vec<u8>> {
    if bits.is_empty() {
        return None;
    }
    let padding = (8 - bits.len() % 8) % 8;
    let mut out = Vec::with_capacity(1 + (padding + bits.len()) / 8);
    // `padding` is below 8.
    out.push(u8::try_from(padding).ok()?);
    let mut byte = 0_u8;
    let mut filled = 0;
    for bit in std::iter::repeat_n(true, padding).chain(bits.iter().copied()) {
        // `* 2 + bit` rather than `<< 1 | bit`, whose `|` -> `^` mutant is the
        // same program (the low bit is always 0 there) and so could never be
        // killed. `byte` is at most 127 before its eighth bit, so this cannot
        // overflow.
        byte = byte * 2 + u8::from(bit);
        filled += 1;
        if filled == 8 {
            out.push(byte);
            byte = 0;
            filled = 0;
        }
    }
    Some(out)
}

/// Whether `bytes` are a `BIT` payload `DuckDB` can read: a padding count of
/// at most 7 followed by at least one data byte. `Bit::ToString` and
/// `Bit::BitLength` assume both without checking.
pub(super) const fn bit_layout_ok(bytes: &[u8]) -> bool {
    matches!(bytes.split_first(), Some((&padding, data)) if padding <= 7 && !data.is_empty())
}

/// The bits of a `BIT` value's bytes, or `None` when the bytes are not a
/// well-formed `BIT` ([`bit_layout_ok`]). The padding bits' own values are
/// ignored, as `Bit::ToString` ignores them.
pub(super) fn decode_bits(bytes: &[u8]) -> Option<Vec<bool>> {
    if !bit_layout_ok(bytes) {
        return None;
    }
    let (&padding, data) = bytes.split_first()?;
    Some(
        data.iter()
            .flat_map(|&byte| (0..8).rev().map(move |i| byte & (1 << i) != 0))
            .skip(usize::from(padding))
            .collect(),
    )
}

/// The canonical form of a `BIGNUM` magnitude and sign: no leading zero byte,
/// zero as the single byte `0` and never negative (`Bignum::Verify`).
///
/// # Errors
///
/// The magnitude, without its leading zeros, is longer than
/// [`BIGNUM_MAX_BYTES`].
pub(super) fn canonical_bignum(
    magnitude_be: &[u8],
    negative: bool,
) -> Result<(Vec<u8>, bool), ExtensionError> {
    let first = magnitude_be
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(magnitude_be.len());
    let digits = magnitude_be.get(first..).unwrap_or_default();
    if digits.is_empty() {
        return Ok((vec![0], false));
    }
    if digits.len() > BIGNUM_MAX_BYTES {
        return Err(ExtensionError::new(format!(
            "Value::bignum: a magnitude of {} bytes exceeds DuckDB's BIGNUM maximum of \
             {BIGNUM_MAX_BYTES} bytes",
            digits.len()
        )));
    }
    Ok((digits.to_vec(), negative))
}

impl Value {
    /// Calls `read` with this value if it is a non-`NULL` value of type
    /// `target`, or else with its [`try_cast`][Self::try_cast] to `target`.
    /// `None` for a null handle, a SQL `NULL`, an unknown type, or a failed
    /// cast. `read` therefore only ever sees a live, non-`NULL` handle whose
    /// type is exactly `target`.
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    fn with_type<R>(&self, target: TypeId, read: impl FnOnce(&Self) -> Option<R>) -> Option<R> {
        if self.raw.is_null() || self.is_sql_null() {
            return None;
        }
        if self.type_id()? == target {
            return read(self);
        }
        let copy = self.try_cast(&LogicalType::try_new(target).ok()?)?;
        if copy.is_sql_null() || copy.type_id() != Some(target) {
            return None;
        }
        read(&copy)
    }

    /// The raw bytes of a non-`NULL` `BIT` value (padding byte, then data).
    ///
    /// Only call on a live, non-`NULL` value of type `BIT`.
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    pub(super) fn bit_bytes(&self) -> Option<Vec<u8>> {
        // SAFETY: the caller passes a live, non-NULL BIT value, so
        // `DefaultCastAs(BIT)` is a copy and `StringValue::Get` does not throw;
        // DuckDB `malloc`s the returned buffer (null only on allocation
        // failure, from 2.0.0).
        let bit: duckdb_bit = unsafe { duckdb_get_bit(self.raw) };
        if bit.data.is_null() {
            return None;
        }
        let len = usize::try_from(bit.size).ok();
        // SAFETY: `bit.data` holds `bit.size` bytes, live until freed below.
        let bytes = len.map(|len| unsafe { std::slice::from_raw_parts(bit.data, len) }.to_vec());
        // SAFETY: `malloc`ed by `duckdb_get_bit`; `duckdb_free` is `free`.
        unsafe { duckdb_free(bit.data.cast()) };
        bytes
    }

    /// Creates a `BIT` value from its bits, leftmost first: `[false, true,
    /// false, true]` is `'0101'::BIT`.
    ///
    /// # Errors
    ///
    /// `bits` is empty: SQL has no empty `BIT` (`''::BIT` is an error), and
    /// `DuckDB` cannot render one.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::value::Value;
    ///
    /// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
    /// let flags = Value::bit(&[false, true, false, true])?;
    /// assert_eq!(flags.as_str()?, "0101");
    /// assert_eq!(flags.as_bit(), Some(vec![false, true, false, true]));
    /// # Ok(())
    /// # }
    /// ```
    pub fn bit(bits: &[bool]) -> Result<Self, ExtensionError> {
        let bytes = encode_bits(bits)
            .ok_or_else(|| ExtensionError::new("Value::bit: a BIT needs at least one bit"))?;
        let size = idx_t::try_from(bytes.len())
            .map_err(|_| ExtensionError::new("Value::bit: too many bits"))?;
        // SAFETY: `bytes` is a well-formed BIT payload of `size` bytes, live
        // for the call; `Value::BIT` copies it into a new owned value.
        let raw = unsafe {
            duckdb_create_bit(duckdb_bit {
                data: bytes.as_ptr().cast_mut(),
                size,
            })
        };
        Self::checked(raw, "duckdb_create_bit")
    }

    /// Reads a `BIT` value as its bits, leftmost first.
    ///
    /// A value of another type is cast to `BIT` the way SQL's `TRY_CAST`
    /// would (through [`try_cast`][Self::try_cast]): a `VARCHAR` `'0101'`
    /// reads as four bits, an `INTEGER` as its 32 bits.
    ///
    /// Returns `None` for a null handle, a SQL `NULL`, a value that does not
    /// cast to `BIT` (`'abc'`, a `DOUBLE`), and a `BIT` whose bytes are not
    /// well formed. `duckdb_get_bit` itself aborts the process on the first
    /// two and segfaults on a null handle; it is only ever called here on a
    /// non-`NULL` `BIT` (see the module docs).
    ///
    /// Works on every supported `DuckDB` (1.4.4 to 2.0.x).
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn as_bit(&self) -> Option<Vec<bool>> {
        self.with_type(TypeId::Bit, |bit| decode_bits(&bit.bit_bytes()?))
    }

    /// Creates a `BIGNUM` (arbitrary-precision integer) value from its
    /// magnitude, most significant byte first, and its sign.
    ///
    /// Leading zero bytes are dropped, and zero (an empty or all-zero
    /// magnitude) is stored as non-negative, which is the canonical form
    /// `DuckDB` requires; `duckdb_create_bignum` would store either as given.
    ///
    /// # Errors
    ///
    /// The magnitude is longer than `DuckDB`'s maximum of 8,388,607 bytes.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::value::Value;
    ///
    /// # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
    /// // -(2^64)
    /// let big = Value::bignum(&[1, 0, 0, 0, 0, 0, 0, 0, 0], true)?;
    /// assert_eq!(big.as_str()?, "-18446744073709551616");
    /// assert_eq!(big.as_bignum(), Some((vec![1, 0, 0, 0, 0, 0, 0, 0, 0], true)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn bignum(magnitude_be: &[u8], negative: bool) -> Result<Self, ExtensionError> {
        let (mut digits, negative) = canonical_bignum(magnitude_be, negative)?;
        let size = idx_t::try_from(digits.len())
            .map_err(|_| ExtensionError::new("Value::bignum: magnitude too long"))?;
        // SAFETY: `digits` is a canonical magnitude of `size` (>= 1) bytes,
        // live for the call; `Bignum::FromByteArray` only reads it.
        let raw = unsafe {
            duckdb_create_bignum(duckdb_bignum {
                data: digits.as_mut_ptr(),
                size,
                is_negative: negative,
            })
        };
        Self::checked(raw, "duckdb_create_bignum")
    }

    /// Reads a `BIGNUM` value as its magnitude (most significant byte first,
    /// no leading zero byte; zero is `[0]`) and whether it is negative.
    ///
    /// An integer or floating-point value is cast to `BIGNUM` the way SQL's
    /// `TRY_CAST` would: a `BIGINT` `-5` reads as `([5], true)`.
    ///
    /// Returns `None` for a null handle, a SQL `NULL` and a value that does
    /// not cast to `BIGNUM` (`'abc'`, a `BLOB`, an infinite `DOUBLE`), on
    /// all of which `duckdb_get_bignum` itself aborts the process.
    ///
    /// A `BIGNUM` `DuckDB` builds is always well formed. One built through
    /// the raw `duckdb_create_bignum` with an empty magnitude is not, and
    /// reading it here — or rendering it with [`as_str`][Self::as_str] —
    /// throws inside `DuckDB` and aborts; [`bignum`][Self::bignum] never
    /// builds one.
    ///
    /// Works on every supported `DuckDB` (1.4.4 to 2.0.x).
    // Every path past the null-handle check calls DuckDB, so a `--lib` mutation run
    // with no engine cannot reach it; covered end to end in tests/ffi_roundtrip.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub fn as_bignum(&self) -> Option<(Vec<u8>, bool)> {
        self.with_type(TypeId::Varint, |value| {
            // SAFETY: `with_type` passes a live, non-NULL BIGNUM value, so
            // `DefaultCastAs(BIGNUM)` is a copy and `StringValue::Get` does
            // not throw; DuckDB `malloc`s the returned magnitude.
            let raw: duckdb_bignum = unsafe { duckdb_get_bignum(value.raw) };
            if raw.data.is_null() {
                return None;
            }
            let len = usize::try_from(raw.size).ok();
            // SAFETY: `raw.data` holds `raw.size` bytes until freed below.
            let digits =
                len.map(|len| unsafe { std::slice::from_raw_parts(raw.data, len) }.to_vec());
            // SAFETY: `malloc`ed by `duckdb_get_bignum`; `duckdb_free` is `free`.
            unsafe { duckdb_free(raw.data.cast()) };
            Some((digits?, raw.is_negative))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_encode_right_aligned_with_padding_set() {
        assert_eq!(encode_bits(&[]), None);
        // '0101': 4 padding bits (set), then 0101.
        assert_eq!(
            encode_bits(&[false, true, false, true]),
            Some(vec![4, 0b1111_0101])
        );
        // Exactly one byte: no padding.
        assert_eq!(encode_bits(&[true; 8]), Some(vec![0, 0xff]));
        assert_eq!(encode_bits(&[false; 8]), Some(vec![0, 0x00]));
        // Nine bits: 7 padding bits in the first byte.
        let nine = [true, false, false, false, false, false, false, false, true];
        assert_eq!(encode_bits(&nine), Some(vec![7, 0b1111_1111, 0b0000_0001]));
        assert_eq!(encode_bits(&[false]), Some(vec![7, 0b1111_1110]));
    }

    #[test]
    fn bits_decode_what_they_encode() {
        for len in 1_usize..=40 {
            let bits: Vec<bool> = (0..len).map(|i| (i * 7 + len) % 3 == 0).collect();
            let bytes = encode_bits(&bits).expect("non-empty");
            assert_eq!(bytes.len(), 1 + len.div_ceil(8), "len {len}");
            assert_eq!(decode_bits(&bytes), Some(bits), "len {len}");
        }
    }

    #[test]
    fn malformed_bit_bytes_decode_to_none() {
        assert_eq!(decode_bits(&[]), None, "no padding byte");
        assert_eq!(decode_bits(&[0]), None, "padding byte only");
        assert_eq!(decode_bits(&[8, 0xff]), None, "padding above 7");
        assert_eq!(decode_bits(&[9, 0xff]), None, "padding above 7");
        // The padding bits' values are ignored.
        assert_eq!(
            decode_bits(&[4, 0b0000_0101]),
            Some(vec![false, true, false, true])
        );
        assert_eq!(decode_bits(&[7, 0x01]), Some(vec![true]));
    }

    #[test]
    fn bignum_magnitudes_are_canonicalised() {
        assert_eq!(canonical_bignum(&[], false).ok(), Some((vec![0], false)));
        assert_eq!(canonical_bignum(&[], true).ok(), Some((vec![0], false)));
        assert_eq!(canonical_bignum(&[0, 0], true).ok(), Some((vec![0], false)));
        assert_eq!(
            canonical_bignum(&[0, 0, 5], true).ok(),
            Some((vec![5], true))
        );
        assert_eq!(
            canonical_bignum(&[1, 0], false).ok(),
            Some((vec![1, 0], false))
        );
        assert_eq!(
            canonical_bignum(&[0, 1, 0], true).ok(),
            Some((vec![1, 0], true))
        );
    }

    #[test]
    fn a_bignum_longer_than_duckdb_stores_is_refused() {
        let mut max = vec![0_u8; BIGNUM_MAX_BYTES];
        max[0] = 1;
        assert!(canonical_bignum(&max, false).is_ok());
        max.push(0);
        let err = canonical_bignum(&max, false).expect_err("one byte too long");
        assert!(err.as_str().contains("8388607"), "{err}");
        // Leading zeros do not count against the limit.
        let mut padded = vec![0_u8; 4];
        padded.extend_from_slice(&[1; 16]);
        assert_eq!(
            canonical_bignum(&padded, true).ok(),
            Some((vec![1; 16], true))
        );
    }

    #[test]
    fn getters_on_a_null_handle_do_not_call_duckdb() {
        // SAFETY: a null handle is part of `Value`'s contract.
        let value = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert_eq!(value.as_bit(), None);
        assert_eq!(value.as_bignum(), None);
    }
}
