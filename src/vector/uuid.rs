// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Converting between a `UUID`'s textual bits and `DuckDB`'s vector storage.
//!
//! A `UUID` column is physically a `HUGEINT`, but the 128 bits in the vector are
//! **not** the 128 bits you see in the text form. `DuckDB` flips the top bit so
//! that comparing the signed integers orders UUIDs the same way comparing their
//! strings does (`BaseUUID::FromUHugeint` in `src/common/types/uuid.cpp`
//! subtracts `2^63` from the upper half; that is a top-bit flip).
//!
//! The consequence is easy to hit and silent when you do:
//!
//! ```text
//! SELECT '11111111-2222-3333-4444-555555555555'::UUID
//!   vector storage : 0x91111111222233334444555555555555   <- read_i128
//!   textual bits   : 0x11111111222233334444555555555555   <- Value::as_uuid, uuid crates
//! ```
//!
//! [`VectorReader::read_uuid`][crate::vector::VectorReader::read_uuid] and
//! [`VectorWriter::write_uuid`][crate::vector::VectorWriter::write_uuid] apply
//! the flip for you and speak in **textual bits** (`u128`), which is what every
//! Rust `Uuid` type holds. These functions are for the raw path — when you have
//! reached for `read_i128` / `write_i128` on a `UUID` column yourself.

/// The bit `DuckDB` flips to make signed `HUGEINT` ordering match `UUID` string
/// ordering.
const UUID_SIGN_BIT: u128 = 1 << 127;

/// Converts `DuckDB`'s vector storage for a `UUID` into the UUID's textual
/// 128 bits.
///
/// # Example
///
/// ```rust
/// use quack_rs::vector::uuid::{uuid_from_storage, uuid_to_storage};
///
/// let textual = 0x1111_1111_2222_3333_4444_5555_5555_5555_u128;
/// assert_eq!(uuid_from_storage(uuid_to_storage(textual)), textual);
/// ```
#[inline]
#[must_use]
#[allow(
    clippy::cast_sign_loss,
    reason = "reinterpreting the bit pattern is the entire operation"
)]
pub const fn uuid_from_storage(storage: i128) -> u128 {
    (storage as u128) ^ UUID_SIGN_BIT
}

/// Converts a UUID's textual 128 bits into `DuckDB`'s vector storage.
///
/// # Example
///
/// ```rust
/// use quack_rs::vector::uuid::uuid_to_storage;
///
/// // The nil UUID sits at the very bottom of DuckDB's signed ordering.
/// assert_eq!(uuid_to_storage(0), i128::MIN);
/// ```
#[inline]
#[must_use]
#[allow(
    clippy::cast_possible_wrap,
    reason = "reinterpreting the bit pattern is the entire operation"
)]
pub const fn uuid_to_storage(bits: u128) -> i128 {
    (bits ^ UUID_SIGN_BIT) as i128
}

#[cfg(test)]
mod tests {
    use super::{uuid_from_storage, uuid_to_storage};

    #[test]
    fn conversions_are_inverses() {
        for bits in [
            0,
            u128::MAX,
            1,
            UUID_SIGN_BIT_TEST,
            0x1111_1111_2222_3333_4444_5555_5555_5555,
        ] {
            assert_eq!(
                uuid_from_storage(uuid_to_storage(bits)),
                bits,
                "{bits:#034x}"
            );
        }
        for storage in [i128::MIN, i128::MAX, 0, -1] {
            assert_eq!(uuid_to_storage(uuid_from_storage(storage)), storage);
        }
    }

    const UUID_SIGN_BIT_TEST: u128 = 1 << 127;

    #[test]
    fn ordering_matches_duckdbs_intent() {
        // The flip exists so that signed HUGEINT ordering matches UUID string
        // ordering: the nil UUID must be the smallest storage value.
        assert_eq!(uuid_to_storage(0), i128::MIN);
        assert_eq!(uuid_to_storage(u128::MAX), i128::MAX);
        assert!(uuid_to_storage(1) > uuid_to_storage(0));
    }
}
