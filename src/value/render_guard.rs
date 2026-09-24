// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Whether `DuckDB` can render a value without failing.
//!
//! [`Value::as_str`], [`Value::display_string`] and `Debug` hand the value to
//! `duckdb_get_varchar` / `duckdb_value_to_string`, which cast it to text with
//! no `try` (`duckdb_value-c.cpp`). For a timestamp whose payload is outside
//! the range `DuckDB`'s own rendering handles, that cast throws "Date out of
//! range in timestamp conversion" through the C API, and the process aborts.
//! The quack-rs constructors refuse such payloads, but `DuckDB` builds them
//! itself from ordinary SQL — `make_timestamp(-9223372036854775808)` as a
//! table function's parameter, for one — so the check has to be made on the
//! value, at rendering time.
//!
//! `DECIMAL` is the same: `sum` over a `DECIMAL(38, s)` column checks for
//! `HUGEINT` overflow but not for the declared width, so plain SQL builds a
//! `DECIMAL(38, 0)` holding `i128::MIN`, whose rendering throws "Negation of
//! HUGEINT is out of range" through the C API, and a `DECIMAL(38, 38)` holding
//! 1.2, whose first character `DuckDB` never writes
//! (`DecimalToString::FormatDecimal` skips the integer digits when
//! `width == scale`): the text starts with a stray byte, and rendering throws
//! when that byte is not valid UTF-8. A `DECIMAL` renders only if its payload
//! fits its width (`docs/upstream-duckdb-reports.md`, items 13 and 22).
//!
//! The check is an allow-list: a type renders unchecked only if every payload
//! of it renders ([`renders_at_any_payload`]). `VARIANT` (whose content the C
//! API cannot see), `GEOMETRY` (whose WKB `DuckDB` accepts malformed and then
//! fails to print) and any type this crate does not know are refused.
//!
//! The C API can read a `LIST`'s elements, a `STRUCT`'s fields and a `MAP`'s
//! entries, so those are checked element by element. It exposes no way to
//! read an `ARRAY`'s elements or a `UNION`'s member, so one of those is
//! refused when its type contains anything that would need a check.

use super::temporal_checks::{temporal_in_range, time_tz_in_range};
use super::Value;
use crate::types::TypeId;

/// The temporal types whose payload [`temporal_in_range`] or
/// [`time_tz_in_range`] can judge.
const fn is_checked_temporal(id: TypeId) -> bool {
    matches!(
        id,
        TypeId::Timestamp
            | TypeId::TimestampTz
            | TypeId::TimestampS
            | TypeId::TimestampMs
            | TypeId::TimestampNs
            | TypeId::Time
            | TypeId::TimeTz
    ) || is_time_ns(id)
}

/// `TIME_NS`, which only exists with `duckdb-1-5`. One function with a cfg'd
/// body rather than two cfg'd functions, so every build compiles, and a
/// mutation test can reach, the body that runs.
const fn is_time_ns(id: TypeId) -> bool {
    #[cfg(feature = "duckdb-1-5")]
    {
        matches!(id, TypeId::TimeNs)
    }
    #[cfg(not(feature = "duckdb-1-5"))]
    {
        let _ = id;
        false
    }
}

/// Whether `DuckDB` renders every payload of a value of type `id`, so that no
/// check is needed. The allow-list: a type not named here is refused unless
/// [`Value::renderable`] checks it.
const fn renders_at_any_payload(id: TypeId) -> bool {
    matches!(
        id,
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
            | TypeId::Varchar
            | TypeId::Blob
            | TypeId::Date
            | TypeId::Interval
            | TypeId::Uuid
            | TypeId::Enum
            | TypeId::Bit
            | TypeId::Varint
            | TypeId::SqlNull
    )
}

/// Whether a type found inside an `ARRAY` or `UNION` — whose elements the C
/// API cannot read — makes the container unrenderable: anything that is not
/// on the allow-list, including a type this crate does not know (`None`).
/// Nested containers are looked through, since their leaves are visited too.
const fn blocks_opaque_container(id: Option<TypeId>) -> bool {
    match id {
        Some(TypeId::List | TypeId::Struct | TypeId::Map | TypeId::Array | TypeId::Union) => false,
        Some(id) => !renders_at_any_payload(id),
        None => true,
    }
}

impl Value {
    /// Whether rendering this value to text cannot make `DuckDB` throw: see
    /// the [module docs](self).
    pub(super) fn renderable(&self) -> bool {
        if self.raw.is_null() || self.is_sql_null() {
            return true;
        }
        let Some(id) = self.type_id() else {
            return false;
        };
        match id {
            _ if is_checked_temporal(id) => self.temporal_payload_in_range(id),
            TypeId::Decimal => self.as_decimal().is_some_and(|d| {
                super::checks::validate_decimal(d.width, d.scale, d.value).is_ok()
            }),
            TypeId::List => (0..self.list_len())
                .all(|i| self.list_child(i).is_none_or(|child| child.renderable())),
            TypeId::Struct => (0..self.struct_field_names().len())
                .all(|i| self.struct_child(i).is_none_or(|child| child.renderable())),
            TypeId::Map => (0..self.map_len()).all(|i| {
                self.map_key(i).is_none_or(|key| key.renderable())
                    && self.map_value(i).is_none_or(|value| value.renderable())
            }),
            TypeId::Array | TypeId::Union => !self.type_contains_unchecked_payload(),
            _ => renders_at_any_payload(id),
        }
    }

    /// Whether this temporal value's raw payload is one `DuckDB` renders.
    fn temporal_payload_in_range(&self, id: TypeId) -> bool {
        if id == TypeId::TimeTz {
            return self.time_tz_payload().is_some_and(time_tz_in_range);
        }
        self.temporal_payload(id)
            .is_some_and(|v| temporal_in_range(id, v))
    }

    /// Whether this value's type contains, anywhere below the top level, a
    /// type whose payload would need a check ([`blocks_opaque_container`]).
    fn type_contains_unchecked_payload(&self) -> bool {
        // SAFETY: `self.raw` is a live, non-null value; the returned type is
        // owned by the value and is not destroyed here (`duckdb.h`).
        let ty = unsafe { libduckdb_sys::duckdb_get_value_type(self.raw) };
        if ty.is_null() {
            return true;
        }
        // SAFETY: `ty` is live for as long as `self` is.
        unsafe {
            crate::table::type_check::contains_type(ty, &|raw| {
                blocks_opaque_container(TypeId::try_from_duckdb_type(raw))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{blocks_opaque_container, is_checked_temporal, is_time_ns, renders_at_any_payload};
    use crate::types::TypeId;

    /// Exactly the types `temporal_in_range` / `time_tz_in_range` judge are
    /// checked; `DATE` and `INTERVAL` render at any payload, and so do the
    /// non-temporal types.
    #[test]
    fn exactly_the_timestamp_and_time_types_are_checked() {
        for id in [
            TypeId::Timestamp,
            TypeId::TimestampTz,
            TypeId::TimestampS,
            TypeId::TimestampMs,
            TypeId::TimestampNs,
            TypeId::Time,
            TypeId::TimeTz,
        ] {
            assert!(is_checked_temporal(id), "{id:?}");
            assert!(!is_time_ns(id), "{id:?}");
        }
        for id in [
            TypeId::Date,
            TypeId::Interval,
            TypeId::BigInt,
            TypeId::Varchar,
            TypeId::List,
            TypeId::Struct,
        ] {
            assert!(!is_checked_temporal(id), "{id:?}");
            assert!(!is_time_ns(id), "{id:?}");
        }
    }

    /// The allow-list: types every payload of which renders. Checked or
    /// refused types are not on it.
    #[test]
    fn only_types_whose_every_payload_renders_are_allowed_unchecked() {
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
            TypeId::Varchar,
            TypeId::Blob,
            TypeId::Date,
            TypeId::Interval,
            TypeId::Uuid,
            TypeId::Enum,
            TypeId::Bit,
            TypeId::Varint,
            TypeId::SqlNull,
        ] {
            assert!(renders_at_any_payload(id), "{id:?}");
            assert!(!blocks_opaque_container(Some(id)), "{id:?}");
        }
        for id in [
            TypeId::Decimal,
            TypeId::Timestamp,
            TypeId::Time,
            TypeId::TimeTz,
            TypeId::Any,
            TypeId::IntegerLiteral,
            TypeId::StringLiteral,
        ] {
            assert!(!renders_at_any_payload(id), "{id:?}");
            assert!(blocks_opaque_container(Some(id)), "{id:?}");
        }
        for id in [
            TypeId::List,
            TypeId::Struct,
            TypeId::Map,
            TypeId::Array,
            TypeId::Union,
        ] {
            assert!(
                !renders_at_any_payload(id),
                "{id:?}: checked element by element"
            );
            assert!(
                !blocks_opaque_container(Some(id)),
                "{id:?}: its leaves are visited"
            );
        }
        assert!(
            blocks_opaque_container(None),
            "a type quack-rs does not know"
        );
    }

    #[cfg(feature = "duckdb-1-5-3")]
    #[test]
    fn variant_and_geometry_are_refused() {
        for id in [TypeId::Variant, TypeId::Geometry] {
            assert!(!renders_at_any_payload(id), "{id:?}");
            assert!(blocks_opaque_container(Some(id)), "{id:?}");
        }
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn time_ns_is_checked() {
        assert!(is_time_ns(TypeId::TimeNs));
        assert!(is_checked_temporal(TypeId::TimeNs));
    }
}
