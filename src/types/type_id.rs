// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Ergonomic enum of all `DuckDB` column types.
//!
//! [`TypeId`] wraps the `DUCKDB_TYPE_*` integer constants from `libduckdb-sys` and
//! provides a safe, exhaustive enum for use in builder APIs.

use libduckdb_sys::{
    DUCKDB_TYPE, DUCKDB_TYPE_DUCKDB_TYPE_ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_BIGINT,
    DUCKDB_TYPE_DUCKDB_TYPE_BIT, DUCKDB_TYPE_DUCKDB_TYPE_BLOB, DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN,
    DUCKDB_TYPE_DUCKDB_TYPE_DATE, DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL, DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE,
    DUCKDB_TYPE_DUCKDB_TYPE_ENUM, DUCKDB_TYPE_DUCKDB_TYPE_FLOAT, DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT,
    DUCKDB_TYPE_DUCKDB_TYPE_INTEGER, DUCKDB_TYPE_DUCKDB_TYPE_INTERVAL,
    DUCKDB_TYPE_DUCKDB_TYPE_LIST, DUCKDB_TYPE_DUCKDB_TYPE_MAP, DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT,
    DUCKDB_TYPE_DUCKDB_TYPE_STRUCT, DUCKDB_TYPE_DUCKDB_TYPE_TIME,
    DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP, DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_MS,
    DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_NS, DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_S,
    DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_TZ, DUCKDB_TYPE_DUCKDB_TYPE_TIME_TZ,
    DUCKDB_TYPE_DUCKDB_TYPE_TINYINT, DUCKDB_TYPE_DUCKDB_TYPE_UBIGINT,
    DUCKDB_TYPE_DUCKDB_TYPE_UHUGEINT, DUCKDB_TYPE_DUCKDB_TYPE_UINTEGER,
    DUCKDB_TYPE_DUCKDB_TYPE_UNION, DUCKDB_TYPE_DUCKDB_TYPE_USMALLINT,
    DUCKDB_TYPE_DUCKDB_TYPE_UTINYINT, DUCKDB_TYPE_DUCKDB_TYPE_UUID,
    DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
};
#[cfg(feature = "duckdb-1-5")]
use libduckdb_sys::{
    DUCKDB_TYPE_DUCKDB_TYPE_ANY, DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM,
    DUCKDB_TYPE_DUCKDB_TYPE_INTEGER_LITERAL, DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL,
    DUCKDB_TYPE_DUCKDB_TYPE_STRING_LITERAL, DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS,
};
#[cfg(feature = "duckdb-1-5-3")]
use libduckdb_sys::{DUCKDB_TYPE_DUCKDB_TYPE_GEOMETRY, DUCKDB_TYPE_DUCKDB_TYPE_VARIANT};

/// Identifies a `DuckDB` column type.
///
/// Use this in the aggregate function builders instead of the raw `DUCKDB_TYPE_*`
/// integer constants. This enum is non-exhaustive — new variants may be added as
/// `DuckDB` adds new types.
///
/// # Example
///
/// ```rust
/// use quack_rs::types::TypeId;
///
/// let t = TypeId::BigInt;
/// assert_eq!(t.to_duckdb_type(), libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TypeId {
    /// `BOOLEAN` — true/false
    Boolean,
    /// `TINYINT` — 8-bit signed integer
    TinyInt,
    /// `SMALLINT` — 16-bit signed integer
    SmallInt,
    /// `INTEGER` — 32-bit signed integer
    Integer,
    /// `BIGINT` — 64-bit signed integer
    BigInt,
    /// `UTINYINT` — 8-bit unsigned integer
    UTinyInt,
    /// `USMALLINT` — 16-bit unsigned integer
    USmallInt,
    /// `UINTEGER` — 32-bit unsigned integer
    UInteger,
    /// `UBIGINT` — 64-bit unsigned integer
    UBigInt,
    /// `HUGEINT` — 128-bit signed integer
    HugeInt,
    /// `FLOAT` — 32-bit floating-point
    Float,
    /// `DOUBLE` — 64-bit floating-point
    Double,
    /// `TIMESTAMP` — microseconds since epoch
    Timestamp,
    /// `TIMESTAMP WITH TIME ZONE` — timezone-aware timestamp
    TimestampTz,
    /// `DATE` — days since epoch
    Date,
    /// `TIME` — microseconds since midnight
    Time,
    /// `INTERVAL` — { months, days, micros }
    Interval,
    /// `VARCHAR` — UTF-8 string
    Varchar,
    /// `BLOB` — binary data
    Blob,
    /// `DECIMAL` — fixed-point decimal (width, scale)
    Decimal,
    /// `TIMESTAMP_S` — seconds since epoch
    TimestampS,
    /// `TIMESTAMP_MS` — milliseconds since epoch
    TimestampMs,
    /// `TIMESTAMP_NS` — nanoseconds since epoch
    TimestampNs,
    /// `ENUM` — enumeration type
    Enum,
    /// `LIST` — variable-length list
    List,
    /// `STRUCT` — named fields (row type)
    Struct,
    /// `MAP` — key-value pairs (LIST of STRUCT)
    Map,
    /// `UUID` — 128-bit UUID
    Uuid,
    /// `UNION` — tagged union of types
    Union,
    /// `BIT` — bitstring
    Bit,
    /// `TIME WITH TIME ZONE` — timezone-aware time
    TimeTz,
    /// `UHUGEINT` — 128-bit unsigned integer
    UHugeInt,
    /// `ARRAY` — fixed-length array
    Array,
    /// `TIME_NS` — time of day with nanosecond precision (`DuckDB` 1.5.0+)
    #[cfg(feature = "duckdb-1-5")]
    TimeNs,
    /// `ANY` — wildcard type for function signatures (`DuckDB` 1.5.0+)
    ///
    /// Used in function overload resolution to accept any input type.
    /// Not a concrete column type — typically used only in builder APIs.
    #[cfg(feature = "duckdb-1-5")]
    Any,
    /// `VARINT` — variable-length integer (`DuckDB` 1.5.0+)
    ///
    /// Arbitrary-precision integer stored as a variable-length encoding.
    /// Maps to `DUCKDB_TYPE_BIGNUM` in the C API.
    #[cfg(feature = "duckdb-1-5")]
    Varint,
    /// `SQLNULL` — explicit SQL NULL type (`DuckDB` 1.5.0+)
    ///
    /// Represents the type of a bare `NULL` literal before type resolution.
    #[cfg(feature = "duckdb-1-5")]
    SqlNull,
    /// `INTEGER_LITERAL` — integer literal type used during overload resolution (`DuckDB` 1.5.0+)
    ///
    /// Internal type representing an unresolved integer literal in SQL. Not a
    /// concrete column type — used by `DuckDB`'s type resolution system.
    #[cfg(feature = "duckdb-1-5")]
    IntegerLiteral,
    /// `STRING_LITERAL` — string literal type used during overload resolution (`DuckDB` 1.5.0+)
    ///
    /// Internal type representing an unresolved string literal in SQL. Not a
    /// concrete column type — used by `DuckDB`'s type resolution system.
    #[cfg(feature = "duckdb-1-5")]
    StringLiteral,
    /// `GEOMETRY` — spatial geometry value (`DuckDB` 1.5.x; requires `duckdb-1-5-3`)
    ///
    /// Present in the C type enum as `DUCKDB_TYPE_GEOMETRY` (40). Gated behind
    /// the `duckdb-1-5-3` feature; see that feature's documentation for the
    /// version-floor rationale.
    #[cfg(feature = "duckdb-1-5-3")]
    Geometry,
    /// `VARIANT` — self-describing nested value, e.g. for Iceberg v3 (`DuckDB` 1.5.3; requires `duckdb-1-5-3`)
    ///
    /// Added to the C type enum as `DUCKDB_TYPE_VARIANT` (41) in `DuckDB` 1.5.3.
    #[cfg(feature = "duckdb-1-5-3")]
    Variant,
}

impl TypeId {
    /// Converts this `TypeId` to the corresponding `DuckDB` C API type constant.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::types::TypeId;
    ///
    /// assert_eq!(
    ///     TypeId::Timestamp.to_duckdb_type(),
    ///     libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP,
    /// );
    /// ```
    #[must_use]
    pub const fn to_duckdb_type(self) -> DUCKDB_TYPE {
        match self {
            Self::Boolean => DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN,
            Self::TinyInt => DUCKDB_TYPE_DUCKDB_TYPE_TINYINT,
            Self::SmallInt => DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT,
            Self::Integer => DUCKDB_TYPE_DUCKDB_TYPE_INTEGER,
            Self::BigInt => DUCKDB_TYPE_DUCKDB_TYPE_BIGINT,
            Self::UTinyInt => DUCKDB_TYPE_DUCKDB_TYPE_UTINYINT,
            Self::USmallInt => DUCKDB_TYPE_DUCKDB_TYPE_USMALLINT,
            Self::UInteger => DUCKDB_TYPE_DUCKDB_TYPE_UINTEGER,
            Self::UBigInt => DUCKDB_TYPE_DUCKDB_TYPE_UBIGINT,
            Self::HugeInt => DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT,
            Self::Float => DUCKDB_TYPE_DUCKDB_TYPE_FLOAT,
            Self::Double => DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE,
            Self::Timestamp => DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP,
            Self::TimestampTz => DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_TZ,
            Self::Date => DUCKDB_TYPE_DUCKDB_TYPE_DATE,
            Self::Time => DUCKDB_TYPE_DUCKDB_TYPE_TIME,
            Self::Interval => DUCKDB_TYPE_DUCKDB_TYPE_INTERVAL,
            Self::Varchar => DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
            Self::Blob => DUCKDB_TYPE_DUCKDB_TYPE_BLOB,
            Self::Decimal => DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL,
            Self::TimestampS => DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_S,
            Self::TimestampMs => DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_MS,
            Self::TimestampNs => DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_NS,
            Self::Enum => DUCKDB_TYPE_DUCKDB_TYPE_ENUM,
            Self::List => DUCKDB_TYPE_DUCKDB_TYPE_LIST,
            Self::Struct => DUCKDB_TYPE_DUCKDB_TYPE_STRUCT,
            Self::Map => DUCKDB_TYPE_DUCKDB_TYPE_MAP,
            Self::Uuid => DUCKDB_TYPE_DUCKDB_TYPE_UUID,
            Self::Union => DUCKDB_TYPE_DUCKDB_TYPE_UNION,
            Self::Bit => DUCKDB_TYPE_DUCKDB_TYPE_BIT,
            Self::TimeTz => DUCKDB_TYPE_DUCKDB_TYPE_TIME_TZ,
            Self::UHugeInt => DUCKDB_TYPE_DUCKDB_TYPE_UHUGEINT,
            Self::Array => DUCKDB_TYPE_DUCKDB_TYPE_ARRAY,
            #[cfg(feature = "duckdb-1-5")]
            Self::TimeNs => DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS,
            #[cfg(feature = "duckdb-1-5")]
            Self::Any => DUCKDB_TYPE_DUCKDB_TYPE_ANY,
            #[cfg(feature = "duckdb-1-5")]
            Self::Varint => DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM,
            #[cfg(feature = "duckdb-1-5")]
            Self::SqlNull => DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL,
            #[cfg(feature = "duckdb-1-5")]
            Self::IntegerLiteral => DUCKDB_TYPE_DUCKDB_TYPE_INTEGER_LITERAL,
            #[cfg(feature = "duckdb-1-5")]
            Self::StringLiteral => DUCKDB_TYPE_DUCKDB_TYPE_STRING_LITERAL,
            #[cfg(feature = "duckdb-1-5-3")]
            Self::Geometry => DUCKDB_TYPE_DUCKDB_TYPE_GEOMETRY,
            #[cfg(feature = "duckdb-1-5-3")]
            Self::Variant => DUCKDB_TYPE_DUCKDB_TYPE_VARIANT,
        }
    }

    /// Converts a raw `DUCKDB_TYPE` constant back into a [`TypeId`].
    ///
    /// Recognizes every variant available in the active feature set, including
    /// the `duckdb-1-5` type-enum values (`TIME_NS`, `ANY`, `BIGNUM`/`VARINT`,
    /// `SQLNULL`, `INTEGER_LITERAL`, `STRING_LITERAL`) and the `duckdb-1-5-3`
    /// values (`GEOMETRY`, `VARIANT`) when those features are enabled.
    ///
    /// # Panics
    ///
    /// Panics if `raw` does not correspond to any `TypeId` variant available in
    /// the current feature configuration — for example, a 1.5.x type value when
    /// the `duckdb-1-5` feature is disabled, or a future/unknown type value.
    #[must_use]
    pub const fn from_duckdb_type(raw: DUCKDB_TYPE) -> Self {
        // Using if-else chain because match on non-primitive constants is not
        // allowed in const context.
        if raw == DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN {
            Self::Boolean
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TINYINT {
            Self::TinyInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT {
            Self::SmallInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_INTEGER {
            Self::Integer
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_BIGINT {
            Self::BigInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UTINYINT {
            Self::UTinyInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_USMALLINT {
            Self::USmallInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UINTEGER {
            Self::UInteger
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UBIGINT {
            Self::UBigInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT {
            Self::HugeInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_FLOAT {
            Self::Float
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE {
            Self::Double
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP {
            Self::Timestamp
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_TZ {
            Self::TimestampTz
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_DATE {
            Self::Date
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIME {
            Self::Time
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_INTERVAL {
            Self::Interval
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR {
            Self::Varchar
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_BLOB {
            Self::Blob
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL {
            Self::Decimal
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_S {
            Self::TimestampS
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_MS {
            Self::TimestampMs
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_NS {
            Self::TimestampNs
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_ENUM {
            Self::Enum
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_LIST {
            Self::List
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_STRUCT {
            Self::Struct
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_MAP {
            Self::Map
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UUID {
            Self::Uuid
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UNION {
            Self::Union
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_BIT {
            Self::Bit
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIME_TZ {
            Self::TimeTz
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_UHUGEINT {
            Self::UHugeInt
        } else if raw == DUCKDB_TYPE_DUCKDB_TYPE_ARRAY {
            Self::Array
        } else {
            // DuckDB 1.5.0+ type-enum values (feature-gated). Kept in a nested
            // block because `#[cfg]` cannot be attached to an `else if` arm.
            #[cfg(feature = "duckdb-1-5")]
            {
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS {
                    return Self::TimeNs;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_ANY {
                    return Self::Any;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM {
                    return Self::Varint;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL {
                    return Self::SqlNull;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_INTEGER_LITERAL {
                    return Self::IntegerLiteral;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_STRING_LITERAL {
                    return Self::StringLiteral;
                }
            }
            // DuckDB 1.5.3+ type-enum values (feature-gated).
            #[cfg(feature = "duckdb-1-5-3")]
            {
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_GEOMETRY {
                    return Self::Geometry;
                }
                if raw == DUCKDB_TYPE_DUCKDB_TYPE_VARIANT {
                    return Self::Variant;
                }
            }
            panic!("unknown DUCKDB_TYPE value")
        }
    }

    /// Returns a human-readable SQL type name for this `TypeId`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::types::TypeId;
    ///
    /// assert_eq!(TypeId::BigInt.sql_name(), "BIGINT");
    /// assert_eq!(TypeId::Varchar.sql_name(), "VARCHAR");
    /// ```
    #[must_use]
    pub const fn sql_name(self) -> &'static str {
        match self {
            Self::Boolean => "BOOLEAN",
            Self::TinyInt => "TINYINT",
            Self::SmallInt => "SMALLINT",
            Self::Integer => "INTEGER",
            Self::BigInt => "BIGINT",
            Self::UTinyInt => "UTINYINT",
            Self::USmallInt => "USMALLINT",
            Self::UInteger => "UINTEGER",
            Self::UBigInt => "UBIGINT",
            Self::HugeInt => "HUGEINT",
            Self::Float => "FLOAT",
            Self::Double => "DOUBLE",
            Self::Timestamp => "TIMESTAMP",
            Self::TimestampTz => "TIMESTAMPTZ",
            Self::Date => "DATE",
            Self::Time => "TIME",
            Self::Interval => "INTERVAL",
            Self::Varchar => "VARCHAR",
            Self::Blob => "BLOB",
            Self::Decimal => "DECIMAL",
            Self::TimestampS => "TIMESTAMP_S",
            Self::TimestampMs => "TIMESTAMP_MS",
            Self::TimestampNs => "TIMESTAMP_NS",
            Self::Enum => "ENUM",
            Self::List => "LIST",
            Self::Struct => "STRUCT",
            Self::Map => "MAP",
            Self::Uuid => "UUID",
            Self::Union => "UNION",
            Self::Bit => "BIT",
            Self::TimeTz => "TIMETZ",
            Self::UHugeInt => "UHUGEINT",
            Self::Array => "ARRAY",
            #[cfg(feature = "duckdb-1-5")]
            Self::TimeNs => "TIME_NS",
            #[cfg(feature = "duckdb-1-5")]
            Self::Any => "ANY",
            #[cfg(feature = "duckdb-1-5")]
            Self::Varint => "VARINT",
            #[cfg(feature = "duckdb-1-5")]
            Self::SqlNull => "SQLNULL",
            #[cfg(feature = "duckdb-1-5")]
            Self::IntegerLiteral => "INTEGER_LITERAL",
            #[cfg(feature = "duckdb-1-5")]
            Self::StringLiteral => "STRING_LITERAL",
            #[cfg(feature = "duckdb-1-5-3")]
            Self::Geometry => "GEOMETRY",
            #[cfg(feature = "duckdb-1-5-3")]
            Self::Variant => "VARIANT",
        }
    }
}

impl std::fmt::Display for TypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sql_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_types_round_trip_display() {
        let types = [
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
            TypeId::Float,
            TypeId::Double,
            TypeId::Timestamp,
            TypeId::TimestampTz,
            TypeId::Date,
            TypeId::Time,
            TypeId::Interval,
            TypeId::Varchar,
            TypeId::Blob,
            TypeId::Decimal,
            TypeId::TimestampS,
            TypeId::TimestampMs,
            TypeId::TimestampNs,
            TypeId::Enum,
            TypeId::List,
            TypeId::Struct,
            TypeId::Map,
            TypeId::Uuid,
            TypeId::Union,
            TypeId::Bit,
            TypeId::TimeTz,
            TypeId::UHugeInt,
            TypeId::Array,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::TimeNs,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::Any,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::Varint,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::SqlNull,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::IntegerLiteral,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::StringLiteral,
            #[cfg(feature = "duckdb-1-5-3")]
            TypeId::Geometry,
            #[cfg(feature = "duckdb-1-5-3")]
            TypeId::Variant,
        ];
        for t in types {
            // sql_name should not be empty and should match Display
            assert!(!t.sql_name().is_empty());
            assert_eq!(t.sql_name(), format!("{t}"));
        }
    }

    #[test]
    fn bigint_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::BigInt.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_BIGINT
        );
    }

    #[test]
    fn boolean_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Boolean.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN
        );
    }

    #[test]
    fn timestamp_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Timestamp.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP
        );
    }

    #[test]
    fn varchar_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Varchar.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR
        );
    }

    #[test]
    fn type_id_clone_copy() {
        let t = TypeId::Integer;
        let t2 = t; // Copy
        assert_eq!(t, t2);
        let t3 = t;
        assert_eq!(t, t3);
    }

    #[test]
    fn type_id_debug() {
        let s = format!("{:?}", TypeId::Interval);
        assert!(s.contains("Interval"));
    }

    // ---- DuckDB 1.5.0+ variant tests ----

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn time_ns_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::TimeNs.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_TIME_NS
        );
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn any_maps_to_correct_duckdb_type() {
        assert_eq!(TypeId::Any.to_duckdb_type(), DUCKDB_TYPE_DUCKDB_TYPE_ANY);
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn varint_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Varint.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_BIGNUM
        );
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn sql_null_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::SqlNull.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL
        );
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn duckdb_1_5_variants_sql_names() {
        assert_eq!(TypeId::TimeNs.sql_name(), "TIME_NS");
        assert_eq!(TypeId::Any.sql_name(), "ANY");
        assert_eq!(TypeId::Varint.sql_name(), "VARINT");
        assert_eq!(TypeId::SqlNull.sql_name(), "SQLNULL");
        assert_eq!(TypeId::IntegerLiteral.sql_name(), "INTEGER_LITERAL");
        assert_eq!(TypeId::StringLiteral.sql_name(), "STRING_LITERAL");
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn duckdb_1_5_variants_display_matches_sql_name() {
        assert_eq!(format!("{}", TypeId::TimeNs), "TIME_NS");
        assert_eq!(format!("{}", TypeId::Any), "ANY");
        assert_eq!(format!("{}", TypeId::Varint), "VARINT");
        assert_eq!(format!("{}", TypeId::SqlNull), "SQLNULL");
        assert_eq!(format!("{}", TypeId::IntegerLiteral), "INTEGER_LITERAL");
        assert_eq!(format!("{}", TypeId::StringLiteral), "STRING_LITERAL");
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn duckdb_1_5_variants_hash_eq() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypeId::TimeNs);
        set.insert(TypeId::Any);
        set.insert(TypeId::Varint);
        set.insert(TypeId::SqlNull);
        set.insert(TypeId::IntegerLiteral);
        set.insert(TypeId::StringLiteral);
        assert_eq!(set.len(), 6);
        assert!(set.contains(&TypeId::TimeNs));
        assert!(set.contains(&TypeId::Any));
        assert!(set.contains(&TypeId::Varint));
        assert!(set.contains(&TypeId::SqlNull));
        assert!(set.contains(&TypeId::IntegerLiteral));
        assert!(set.contains(&TypeId::StringLiteral));
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn integer_literal_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::IntegerLiteral.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_INTEGER_LITERAL
        );
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn string_literal_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::StringLiteral.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_STRING_LITERAL
        );
    }

    #[test]
    fn from_duckdb_type_roundtrip() {
        let variants = [
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
            TypeId::Float,
            TypeId::Double,
            TypeId::Timestamp,
            TypeId::TimestampTz,
            TypeId::Date,
            TypeId::Time,
            TypeId::Interval,
            TypeId::Varchar,
            TypeId::Blob,
            TypeId::Decimal,
            TypeId::TimestampS,
            TypeId::TimestampMs,
            TypeId::TimestampNs,
            TypeId::Enum,
            TypeId::List,
            TypeId::Struct,
            TypeId::Map,
            TypeId::Uuid,
            TypeId::Union,
            TypeId::Bit,
            TypeId::TimeTz,
            TypeId::UHugeInt,
            TypeId::Array,
            // DuckDB 1.5.0+ variants: from_duckdb_type must handle these too
            // (previously it panicked on them — see the panic-gap fix).
            #[cfg(feature = "duckdb-1-5")]
            TypeId::TimeNs,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::Any,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::Varint,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::SqlNull,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::IntegerLiteral,
            #[cfg(feature = "duckdb-1-5")]
            TypeId::StringLiteral,
            #[cfg(feature = "duckdb-1-5-3")]
            TypeId::Geometry,
            #[cfg(feature = "duckdb-1-5-3")]
            TypeId::Variant,
        ];
        for &tid in &variants {
            let raw = tid.to_duckdb_type();
            let back = TypeId::from_duckdb_type(raw);
            assert_eq!(back, tid, "roundtrip failed for {tid:?}");
        }
    }

    #[cfg(feature = "duckdb-1-5-3")]
    #[test]
    fn geometry_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Geometry.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_GEOMETRY
        );
    }

    #[cfg(feature = "duckdb-1-5-3")]
    #[test]
    fn variant_maps_to_correct_duckdb_type() {
        assert_eq!(
            TypeId::Variant.to_duckdb_type(),
            DUCKDB_TYPE_DUCKDB_TYPE_VARIANT
        );
    }

    #[cfg(feature = "duckdb-1-5-3")]
    #[test]
    fn geometry_variant_sql_names_and_display() {
        assert_eq!(TypeId::Geometry.sql_name(), "GEOMETRY");
        assert_eq!(TypeId::Variant.sql_name(), "VARIANT");
        assert_eq!(format!("{}", TypeId::Geometry), "GEOMETRY");
        assert_eq!(format!("{}", TypeId::Variant), "VARIANT");
    }
}
