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
}
