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
    // 10^38 < 2^127, so every power up to MAX_DECIMAL_WIDTH fits in a u128.
    let limit = 10_u128.pow(u32::from(width));
    if unscaled.unsigned_abs() >= limit {
        return Err(ExtensionError::new(format!(
            "unscaled value {unscaled} has more than {width} digits and does not fit \
             DECIMAL({width}, {scale})"
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
}
