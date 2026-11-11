// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Pure-Rust preconditions checked before a `Value` call reaches `DuckDB`.
//!
//! Kept apart from the FFI wrappers so the `--lib` mutation run, which has no
//! live engine, can still kill their mutants with ordinary unit tests.

use crate::error::ExtensionError;
use crate::types::TypeId;

/// The widest `DECIMAL` `DuckDB` supports (`Decimal::MAX_WIDTH_DECIMAL`).
const MAX_DECIMAL_WIDTH: u8 = 38;

/// `10^n` for every `n` in `0..=MAX_DECIMAL_WIDTH`, built at compile time so the
/// per-row digit check in [`unscaled_fits`] is one comparison, not a `pow`.
/// `10^38` is below `2^127`, so every entry fits a `u128`.
const POWERS_OF_TEN: [u128; MAX_DECIMAL_WIDTH as usize + 1] = {
    let mut table = [1_u128; MAX_DECIMAL_WIDTH as usize + 1];
    let mut i = 1;
    while i < table.len() {
        table[i] = table[i - 1] * 10;
        i += 1;
    }
    table
};

/// Whether `unscaled` has at most `width` decimal digits, i.e.
/// `|unscaled| < 10^width`. False for a `width` outside `1..=38`.
fn unscaled_fits(width: u8, unscaled: i128) -> bool {
    width != 0
        && POWERS_OF_TEN
            .get(usize::from(width))
            .is_some_and(|limit| unscaled.unsigned_abs() < *limit)
}

/// Whether a value of type `source` may be handed to one of `DuckDB`'s scalar
/// `duckdb_get_*` getters, which cast it to the requested type first.
///
/// Every listed type is a plain scalar whose casts `DuckDB` reports through
/// `TryCast`'s error channel. Nested, `BLOB`, `BIT`, `ENUM` and every type this
/// build does not name are refused without a call: some of their cast paths
/// bind a cast function that throws (`StructCastSwitch`,
/// `AggregateStateToBlobCast`), and a C++ exception that reaches Rust aborts
/// the process.
pub(super) const fn is_scalar_cast_source(source: TypeId) -> bool {
    match source {
        TypeId::Boolean
        | TypeId::TinyInt
        | TypeId::SmallInt
        | TypeId::Integer
        | TypeId::BigInt
        | TypeId::UTinyInt
        | TypeId::USmallInt
        | TypeId::UInteger
        | TypeId::UBigInt
        | TypeId::HugeInt
        | TypeId::UHugeInt
        | TypeId::Float
        | TypeId::Double
        | TypeId::Decimal
        | TypeId::Varchar
        | TypeId::Date
        | TypeId::Time
        | TypeId::TimeTz
        | TypeId::Timestamp
        | TypeId::TimestampTz
        | TypeId::TimestampS
        | TypeId::TimestampMs
        | TypeId::TimestampNs
        // `TimestampTzNsCastSwitch` (`time_casts.cpp`, DuckDB 2.0.0): every
        // listed cast reports failure through `TryCast`, and the one that
        // fails on a non-finite input is guarded in `cast_guard`. On an engine
        // before 2.0.0 no value has this type.
        | TypeId::TimestampTzNs
        | TypeId::Interval
        | TypeId::Uuid => true,
        #[cfg(feature = "duckdb-1-5")]
        TypeId::TimeNs => true,
        _ => false,
    }
}

/// Checks a `DECIMAL(width, scale)` and its unscaled value before `DuckDB`
/// sees them.
///
/// `DuckDB` does not validate the unscaled value against the width:
/// `duckdb_create_decimal` narrows it with a throwing `NumericCast` (an abort
/// from Rust) when it does not fit the physical type, and stores an
/// out-of-range value that does fit (`DECIMAL(4,0)` holding `10000`);
/// `duckdb_bind_decimal` does not validate width or scale at all, and silently
/// drops the high 64 bits for `width <= 18`.
///
/// # Errors
///
/// - `width` is not in `1..=38`;
/// - `scale > width`;
/// - `|unscaled| >= 10^width`, i.e. the number has more digits than the
///   declared width allows.
pub fn validate_decimal(width: u8, scale: u8, unscaled: i128) -> Result<(), ExtensionError> {
    if width == 0 || width > MAX_DECIMAL_WIDTH {
        return Err(ExtensionError::new(format!(
            "DECIMAL({width}, {scale}): width must be between 1 and {MAX_DECIMAL_WIDTH}"
        )));
    }
    if scale > width {
        return Err(ExtensionError::new(format!(
            "DECIMAL({width}, {scale}): scale must not exceed width"
        )));
    }
    if !unscaled_fits(width, unscaled) {
        return Err(ExtensionError::new(format!(
            "unscaled value {unscaled} has more than {width} digits and does not fit \
             DECIMAL({width}, {scale})"
        )));
    }
    Ok(())
}

/// Checks an unscaled `DECIMAL` value against a width alone, for a caller that
/// writes the integer into a vector and does not know the scale.
///
/// The scale does not change which unscaled integers a width holds, so this
/// is [`validate_decimal`] without its scale test. It exists because
/// `VectorWriter::write_decimal` narrows the integer to `i16`, `i32` or `i64`
/// with `as`, which wraps a value the physical type cannot hold (`70000` in a
/// `DECIMAL(4, 0)` is stored as `4464`) and stores one that fits the integer but
/// has more digits than the width (`10000` in a `DECIMAL(4, 0)`).
///
/// # Errors
///
/// - `width` is not in `1..=38`;
/// - `|unscaled| >= 10^width`.
pub fn validate_unscaled_decimal(width: u8, unscaled: i128) -> Result<(), ExtensionError> {
    if width == 0 || width > MAX_DECIMAL_WIDTH {
        return Err(ExtensionError::new(format!(
            "DECIMAL width must be between 1 and {MAX_DECIMAL_WIDTH}, got {width}"
        )));
    }
    if !unscaled_fits(width, unscaled) {
        return Err(ExtensionError::new(format!(
            "unscaled value {unscaled} has more than {width} digits and does not fit a \
             DECIMAL of width {width}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_width_bounds() {
        assert!(validate_decimal(0, 0, 0).is_err());
        assert!(validate_decimal(1, 0, 0).is_ok());
        assert!(validate_decimal(38, 0, 0).is_ok());
        assert!(validate_decimal(39, 0, 0).is_err());
        assert!(validate_decimal(50, 2, 0).is_err());
        // An invalid width must be reported as one, not as a digit count that
        // an unrepresentable width happens to fail too.
        for (width, scale) in [(0_u8, 0_u8), (39, 0), (50, 2), (255, 0)] {
            let err = validate_decimal(width, scale, 0).unwrap_err().to_string();
            assert!(err.contains("width must be between 1 and 38"), "{err}");
            assert!(err.contains(&format!("DECIMAL({width}, {scale})")), "{err}");
        }
    }

    #[test]
    fn decimal_scale_bound() {
        assert!(validate_decimal(4, 4, 0).is_ok());
        assert!(validate_decimal(4, 5, 0).is_err());
    }

    #[test]
    fn decimal_digit_limit() {
        assert!(validate_decimal(4, 0, 9_999).is_ok());
        assert!(validate_decimal(4, 0, -9_999).is_ok());
        assert!(validate_decimal(4, 0, 10_000).is_err());
        assert!(validate_decimal(4, 0, -10_000).is_err());
        assert!(validate_decimal(4, 0, (1_i128 << 70) + 7).is_err());
        assert!(validate_decimal(18, 0, 1_i128 << 64).is_err());
        let max38 = 10_i128.pow(38) - 1;
        assert!(validate_decimal(38, 0, max38).is_ok());
        assert!(validate_decimal(38, 0, -max38).is_ok());
        assert!(validate_decimal(38, 0, max38 + 1).is_err());
        assert!(validate_decimal(38, 0, i128::MIN).is_err());
        assert!(validate_decimal(38, 0, i128::MAX).is_err());
    }

    #[test]
    fn powers_of_ten_table_is_exact() {
        for (n, entry) in POWERS_OF_TEN.iter().enumerate() {
            assert_eq!(*entry, 10_u128.pow(u32::try_from(n).unwrap()), "10^{n}");
        }
        assert_eq!(POWERS_OF_TEN.len(), usize::from(MAX_DECIMAL_WIDTH) + 1);
    }

    #[test]
    fn unscaled_decimal_boundaries_at_every_width() {
        for width in 1..=MAX_DECIMAL_WIDTH {
            let max = 10_i128.pow(u32::from(width)) - 1;
            let ok = |v: i128| validate_unscaled_decimal(width, v).is_ok();
            assert!(ok(0), "width {width}: 0");
            assert!(ok(1), "width {width}: 1");
            assert!(ok(max), "width {width}: {max}");
            assert!(ok(-max), "width {width}: -{max}");
            assert!(!ok(max + 1), "width {width}: {}", max + 1);
            assert!(!ok(-max - 1), "width {width}: {}", -max - 1);
            assert!(!ok(i128::MAX), "width {width}: i128::MAX");
            assert!(!ok(i128::MIN), "width {width}: i128::MIN");
        }
    }

    #[test]
    fn unscaled_decimal_refuses_a_width_outside_one_to_thirty_eight() {
        // Zero fits no width, and a width past 38 is not a DECIMAL.
        for width in [0_u8, 39, 40, 255] {
            let err = validate_unscaled_decimal(width, 0).unwrap_err().to_string();
            assert!(err.contains("between 1 and 38"), "width {width}: {err}");
            assert!(
                err.contains(&format!("got {width}")),
                "width {width}: {err}"
            );
        }
        assert!(!unscaled_fits(0, 0));
        assert!(!unscaled_fits(39, 0));
    }

    #[test]
    fn unscaled_decimal_agrees_with_validate_decimal_at_scale_zero() {
        for width in 0..=41_u8 {
            for value in [
                0_i128,
                1,
                9_999,
                10_000,
                70_000,
                i128::from(i64::MAX),
                i128::MAX,
            ] {
                assert_eq!(
                    validate_unscaled_decimal(width, value).is_ok(),
                    validate_decimal(width, 0, value).is_ok(),
                    "width {width}, value {value}"
                );
            }
        }
    }

    /// The case the check exists for: the `as i16` in `write_decimal` turns
    /// 70000 into 4464, a value that looks valid, and the check refuses it.
    #[test]
    // The `as` is the cast under test: it is what `write_decimal` does.
    #[allow(clippy::cast_possible_truncation)]
    fn unscaled_decimal_refuses_what_a_narrowing_cast_would_wrap() {
        assert_eq!(70_000_i128 as i16, 4_464);
        let err = validate_unscaled_decimal(4, 70_000)
            .unwrap_err()
            .to_string();
        assert!(err.contains("70000"), "{err}");
        assert!(err.contains("more than 4 digits"), "{err}");
        // 10000 fits an i16, so it would be stored unchanged, one digit too wide.
        assert!(i16::try_from(10_000_i128).is_ok());
        assert!(validate_unscaled_decimal(4, 10_000).is_err());
    }

    #[test]
    fn scalar_cast_sources() {
        assert!(is_scalar_cast_source(TypeId::BigInt));
        assert!(is_scalar_cast_source(TypeId::Varchar));
        assert!(is_scalar_cast_source(TypeId::Decimal));
        assert!(is_scalar_cast_source(TypeId::Uuid));
        assert!(is_scalar_cast_source(TypeId::Interval));
        assert!(!is_scalar_cast_source(TypeId::Blob));
        assert!(!is_scalar_cast_source(TypeId::Bit));
        assert!(!is_scalar_cast_source(TypeId::Enum));
        assert!(!is_scalar_cast_source(TypeId::List));
        assert!(!is_scalar_cast_source(TypeId::Struct));
        assert!(!is_scalar_cast_source(TypeId::Map));
        assert!(!is_scalar_cast_source(TypeId::Array));
        assert!(!is_scalar_cast_source(TypeId::Union));
    }

    #[test]
    fn every_listed_scalar_is_a_cast_source() {
        for id in [
            TypeId::Boolean,
            TypeId::TinyInt,
            TypeId::SmallInt,
            TypeId::Integer,
            TypeId::BigInt,
            TypeId::UTinyInt,
            TypeId::USmallInt,
            TypeId::UInteger,
            TypeId::UBigInt,
            TypeId::HugeInt,
            TypeId::UHugeInt,
            TypeId::Float,
            TypeId::Double,
            TypeId::Decimal,
            TypeId::Varchar,
            TypeId::Date,
            TypeId::Time,
            TypeId::TimeTz,
            TypeId::Timestamp,
            TypeId::TimestampTz,
            TypeId::TimestampS,
            TypeId::TimestampMs,
            TypeId::TimestampNs,
            TypeId::TimestampTzNs,
            TypeId::Interval,
            TypeId::Uuid,
        ] {
            assert!(is_scalar_cast_source(id), "{id:?}");
        }
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn time_ns_is_a_cast_source() {
        assert!(is_scalar_cast_source(TypeId::TimeNs));
    }
}
