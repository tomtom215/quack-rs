// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Conversions between Rust's 128-bit integers and `DuckDB`'s split-word
//! `HUGEINT` / `UHUGEINT` records.
//!
//! These four functions are the only pure arithmetic in the `value` module —
//! everything else there calls into `DuckDB` — and they are the easiest place
//! in the crate to be silently wrong: a shift in the wrong direction still
//! compiles, still round-trips zero, and still round-trips any value that fits
//! in 64 bits, so the common cases in the end-to-end suite would not notice.
//!
//! They live in their own file so that the mutation-testing gate keeps
//! examining them. `mutants.toml` excludes `src/value.rs` and `src/query.rs`
//! wholesale, because every other function in those two modules is a thin
//! wrapper over a `DuckDB` C call that the `--lib` run cannot reach without a
//! live engine. This module is reachable, so it stays in.

/// Splits an `i128` into `DuckDB`'s `{ lower: u64, upper: i64 }` `HUGEINT`.
#[inline]
pub const fn hugeint_from_i128(value: i128) -> libduckdb_sys::duckdb_hugeint {
    libduckdb_sys::duckdb_hugeint {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        lower: value as u64,
        #[allow(clippy::cast_possible_truncation)]
        upper: (value >> 64) as i64,
    }
}

/// Splits a `u128` into `DuckDB`'s `{ lower: u64, upper: u64 }` `UHUGEINT`.
#[inline]
pub const fn uhugeint_from_u128(value: u128) -> libduckdb_sys::duckdb_uhugeint {
    libduckdb_sys::duckdb_uhugeint {
        #[allow(clippy::cast_possible_truncation)]
        lower: value as u64,
        #[allow(clippy::cast_possible_truncation)]
        upper: (value >> 64) as u64,
    }
}

/// Reassembles `DuckDB`'s `{ lower: u64, upper: i64 }` `HUGEINT` into an `i128`.
///
/// The inverse of [`hugeint_from_i128`]. Both directions live here, next to each
/// other and unit-tested, because the whole crate used to open-code this
/// arithmetic at five separate call sites: a shift in the wrong direction still
/// compiles and still round-trips anything that fits in 64 bits.
///
/// The halves are combined with `+` rather than `|`. They occupy disjoint bits,
/// so the two are identical here — but `|` has an *equivalent mutant*: swapping
/// it for `^` cannot change the answer for any input, so no test can ever kill
/// it. `+` has no such twin, and `-` or `*` in its place fails the round trips
/// below at once. It cannot overflow: `i64::MIN << 64` is exactly `i128::MIN`
/// and the widest `lower` adds `2^64 - 1`, which the extremes in
/// `the_128_bit_helpers_are_exact_inverses` exercise directly — a debug build
/// would panic there if that reasoning were wrong.
#[inline]
pub const fn hugeint_to_i128(raw: libduckdb_sys::duckdb_hugeint) -> i128 {
    ((raw.upper as i128) << 64) + (raw.lower as i128)
}

/// Reassembles `DuckDB`'s `{ lower: u64, upper: u64 }` `UHUGEINT` into a `u128`.
///
/// The inverse of [`uhugeint_from_u128`]. `+` rather than `|` for the reason
/// given on [`hugeint_to_i128`]; `u64::MAX` in both halves is exactly
/// `u128::MAX`, so this cannot overflow either.
#[inline]
pub const fn uhugeint_to_u128(raw: libduckdb_sys::duckdb_uhugeint) -> u128 {
    ((raw.upper as u128) << 64) + (raw.lower as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pin both halves against values whose upper and lower words differ: a
    // round trip alone would survive swapping the two directions together.

    #[test]
    fn hugeint_splits_into_the_low_and_high_words_the_right_way_round() {
        // 1 << 64 is exactly "upper = 1, lower = 0". A left shift would give 0.
        let one_shifted = hugeint_from_i128(1_i128 << 64);
        assert_eq!(one_shifted.lower, 0);
        assert_eq!(one_shifted.upper, 1);

        // A value with distinct words, so swapping them is visible.
        let mixed = hugeint_from_i128((0x0123_4567_89ab_cdef_i128 << 64) | 0x1122_3344_5566_7788);
        assert_eq!(mixed.lower, 0x1122_3344_5566_7788);
        assert_eq!(mixed.upper, 0x0123_4567_89ab_cdef);

        // Small positive: upper must be 0, not a shifted copy of the value.
        let small = hugeint_from_i128(42);
        assert_eq!(small.lower, 42);
        assert_eq!(small.upper, 0);

        // Negative values sign-extend the upper word; `>>` on i128 is arithmetic.
        let minus_one = hugeint_from_i128(-1);
        assert_eq!(minus_one.lower, u64::MAX);
        assert_eq!(minus_one.upper, -1);

        let min = hugeint_from_i128(i128::MIN);
        assert_eq!(min.lower, 0);
        assert_eq!(min.upper, i64::MIN);

        let max = hugeint_from_i128(i128::MAX);
        assert_eq!(max.lower, u64::MAX);
        assert_eq!(max.upper, i64::MAX);
    }

    #[test]
    fn uhugeint_splits_into_the_low_and_high_words_the_right_way_round() {
        let one_shifted = uhugeint_from_u128(1_u128 << 64);
        assert_eq!(one_shifted.lower, 0);
        assert_eq!(one_shifted.upper, 1);

        let mixed = uhugeint_from_u128((0x0123_4567_89ab_cdef_u128 << 64) | 0x1122_3344_5566_7788);
        assert_eq!(mixed.lower, 0x1122_3344_5566_7788);
        assert_eq!(mixed.upper, 0x0123_4567_89ab_cdef);

        let small = uhugeint_from_u128(42);
        assert_eq!(small.lower, 42);
        assert_eq!(small.upper, 0);

        let max = uhugeint_from_u128(u128::MAX);
        assert_eq!(max.lower, u64::MAX);
        assert_eq!(max.upper, u64::MAX);
    }

    #[test]
    fn the_128_bit_helpers_are_exact_inverses() {
        // Every `as_i128` / `as_u128` / `as_uuid` / `as_decimal` accessor and
        // every 128-bit bind now routes through these four, so a round trip at
        // the extremes covers all of them at once.
        for value in [
            0_i128,
            1,
            -1,
            42,
            -42,
            i128::from(i64::MAX),
            i128::from(i64::MIN),
            (0x0123_4567_89ab_cdef_i128 << 64) | 0x1122_3344_5566_7788,
            i128::MAX,
            i128::MIN,
        ] {
            assert_eq!(
                hugeint_to_i128(hugeint_from_i128(value)),
                value,
                "i128 round trip for {value}"
            );
        }

        for value in [
            0_u128,
            1,
            42,
            u128::from(u64::MAX),
            1_u128 << 64,
            (0x0123_4567_89ab_cdef_u128 << 64) | 0x1122_3344_5566_7788,
            u128::MAX,
        ] {
            assert_eq!(
                uhugeint_to_u128(uhugeint_from_u128(value)),
                value,
                "u128 round trip for {value}"
            );
        }
    }

    #[test]
    fn the_128_bit_helpers_read_the_words_the_right_way_round() {
        // A round trip alone would survive swapping *both* directions, so pin
        // the word order against a hand-built record too.
        let raw = libduckdb_sys::duckdb_hugeint {
            lower: 0x1122_3344_5566_7788,
            upper: 0x0123_4567_89ab_cdef,
        };
        assert_eq!(
            hugeint_to_i128(raw),
            (0x0123_4567_89ab_cdef_i128 << 64) | 0x1122_3344_5566_7788
        );

        let raw = libduckdb_sys::duckdb_uhugeint { lower: 0, upper: 1 };
        assert_eq!(uhugeint_to_u128(raw), 1_u128 << 64);

        // Sign extension: upper = -1, lower = MAX is exactly -1.
        let minus_one = libduckdb_sys::duckdb_hugeint {
            lower: u64::MAX,
            upper: -1,
        };
        assert_eq!(hugeint_to_i128(minus_one), -1);
    }
}
