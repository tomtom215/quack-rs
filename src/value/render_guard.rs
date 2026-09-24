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
//! The C API can read a `LIST`'s elements, a `STRUCT`'s fields and a `MAP`'s
//! entries, so those are checked element by element. It exposes no way to
//! read an `ARRAY`'s elements or a `UNION`'s member, so one of those whose
//! type contains any timestamp or time type is treated as unrenderable.

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

impl Value {
    /// Whether rendering this value to text cannot make `DuckDB` throw: see
    /// the [module docs](self).
    pub(super) fn renderable(&self) -> bool {
        if self.raw.is_null() || self.is_sql_null() {
            return true;
        }
        let Some(id) = self.type_id() else {
            return true;
        };
        match id {
            _ if is_checked_temporal(id) => self.temporal_payload_in_range(id),
            TypeId::List => (0..self.list_len())
                .all(|i| self.list_child(i).is_none_or(|child| child.renderable())),
            TypeId::Struct => (0..self.struct_field_names().len())
                .all(|i| self.struct_child(i).is_none_or(|child| child.renderable())),
            TypeId::Map => (0..self.map_len()).all(|i| {
                self.map_key(i).is_none_or(|key| key.renderable())
                    && self.map_value(i).is_none_or(|value| value.renderable())
            }),
            TypeId::Array | TypeId::Union => !self.type_contains_checked_temporal(),
            _ => true,
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

    /// Whether this value's type contains a temporal type anywhere below the
    /// top level.
    fn type_contains_checked_temporal(&self) -> bool {
        // SAFETY: `self.raw` is a live, non-null value; the returned type is
        // owned by the value and is not destroyed here (`duckdb.h`).
        let ty = unsafe { libduckdb_sys::duckdb_get_value_type(self.raw) };
        if ty.is_null() {
            return true;
        }
        // SAFETY: `ty` is live for as long as `self` is.
        unsafe {
            crate::table::type_check::contains_type(ty, &|raw| {
                TypeId::try_from_duckdb_type(raw).is_some_and(is_checked_temporal)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_checked_temporal, is_time_ns};
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

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn time_ns_is_checked() {
        assert!(is_time_ns(TypeId::TimeNs));
        assert!(is_checked_temporal(TypeId::TimeNs));
    }
}
