// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Pure-Rust range checks for the temporal types, derived from `DuckDB`'s
//! cast and rendering code.
//!
//! Kept apart from the FFI wrappers, like `checks`, so unit tests (and the
//! `--lib` mutation run, which has no engine) exercise them directly. The
//! getters use [`cast_guard`] and the range checks to refuse conversions
//! `DuckDB` would throw on; the temporal constructors use the range checks to
//! refuse payloads `DuckDB` cannot render.

use crate::types::TypeId;

/// `timestamp_t::infinity()` and `ninfinity()`: `±i64::MAX`, shared by every
/// `TIMESTAMP` precision.
const TS_INFINITY: i64 = i64::MAX;
const TS_NEGATIVE_INFINITY: i64 = -i64::MAX;

/// `Interval::NANOS_PER_DAY`.
const NANOS_PER_DAY: i64 = 86_400_000_000_000;

/// The input domain on which one of `DuckDB`'s *throwing* casts does not throw.
///
/// `DuckDB` binds a handful of temporal casts through
/// `VectorCastHelpers::TemplatedCastLoop<…, OP>` (`time_casts.cpp`, the only
/// file that uses it). Unlike every `TryCastLoop`, that loop has no error
/// channel: `OP` either succeeds or throws a C++ exception — which aborts the
/// process once it reaches Rust. The C API getters (`CAPIGetValue` →
/// `Value::DefaultTryCastAs`) run these loops unguarded, so the getter for
/// such a pair must check the source payload first. Each variant is the exact
/// condition under which the named `DuckDB` 1.4/1.5 routine returns rather
/// than throws; `v` is the source's raw `int64` payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CastGuard {
    /// `v` is finite. `TryCast<timestamp_t, dtime_t | dtime_tz_t>` returns
    /// `false` for `±infinity`, and `duckdb::Cast` turns that into a throw;
    /// `Timestamp::GetTime` throws on its own.
    Finite,
    /// `v * factor` does not overflow — infinities included, since the
    /// routine does not special-case them (`Timestamp::FromEpochMs` in
    /// `CastTimestampMsToDate` / `MsToTime`).
    Scales(i64),
    /// `v` is `±infinity` (passed through unchanged) or `v * factor` does not
    /// overflow (`FromEpochSeconds`, `FromEpochMs`, `GetEpochNanoSeconds`).
    InfiniteOrScales(i64),
    /// `v` is finite and `v * factor` does not overflow
    /// (`CastTimestampSecToTime`: `FromEpochSeconds` then `GetTime`).
    FiniteAndScales(i64),
    /// `v` is finite and its floor-divided day, times `NANOS_PER_DAY`, does not
    /// overflow (`Timestamp::GetTimeNs`).
    #[cfg(feature = "duckdb-1-5")]
    FiniteAndNanoDayFits,
}

impl CastGuard {
    /// Whether `DuckDB` converts `v` without throwing.
    pub(super) const fn accepts(self, v: i64) -> bool {
        let finite = v != TS_INFINITY && v != TS_NEGATIVE_INFINITY;
        match self {
            Self::Finite => finite,
            Self::Scales(factor) => v.checked_mul(factor).is_some(),
            Self::InfiniteOrScales(factor) => !finite || v.checked_mul(factor).is_some(),
            Self::FiniteAndScales(factor) => finite && v.checked_mul(factor).is_some(),
            #[cfg(feature = "duckdb-1-5")]
            Self::FiniteAndNanoDayFits => {
                // `GetTimeNs` (1.5): `(v + (v < 0)) / NANOS_PER_DAY - (v < 0)`.
                // 1.4 derives the day from `v / 1000` instead, which is never
                // smaller, so a value that passes here passes there too.
                let negative = (v < 0) as i64;
                let day = (v + negative) / NANOS_PER_DAY - negative;
                finite && day.checked_mul(NANOS_PER_DAY).is_some()
            }
        }
    }
}

/// The guard for the getter cast `source → target`, or `None` when `DuckDB`
/// binds that pair through an error-reporting `TryCast` loop (or not at all)
/// and needs no guard.
///
/// This is every `TemplatedCastLoop` entry in `DuckDB` 1.5.5's
/// `time_casts.cpp` whose target a `Value` getter reads. The entries left out
/// never throw for any input: `TIME → TIMETZ`, `TIME_NS → TIME`,
/// `TIMETZ → TIME` (plain bit arithmetic); `TIMESTAMP → DATE` and
/// `TIMESTAMP_NS → DATE / TIMESTAMP / TIMESTAMPTZ` (`GetDate`, division);
/// `TIMESTAMP(TZ) → TIMESTAMP_MS / _S` (`GetEpochRounded`, division). 1.4.4
/// binds a subset (`TIMESTAMPTZ → TIMESTAMP_NS / _MS / _S` are missing and
/// fail cleanly there).
pub(super) const fn cast_guard(source: TypeId, target: TypeId) -> Option<CastGuard> {
    const US_PER_MS: i64 = 1_000;
    const US_PER_S: i64 = 1_000_000;
    const NS_PER_S: i64 = 1_000_000_000;
    match (source, target) {
        (TypeId::Timestamp, TypeId::Time | TypeId::TimeTz)
        | (TypeId::TimestampTz, TypeId::TimeTz)
        | (TypeId::TimestampNs, TypeId::Time) => Some(CastGuard::Finite),
        (TypeId::Timestamp | TypeId::TimestampTz, TypeId::TimestampNs) => {
            Some(CastGuard::InfiniteOrScales(US_PER_MS))
        }
        (TypeId::TimestampMs, TypeId::Date | TypeId::Time) => Some(CastGuard::Scales(US_PER_MS)),
        (TypeId::TimestampMs, TypeId::Timestamp | TypeId::TimestampTz) => {
            Some(CastGuard::InfiniteOrScales(US_PER_MS))
        }
        (TypeId::TimestampMs, TypeId::TimestampNs) => Some(CastGuard::InfiniteOrScales(US_PER_S)),
        (TypeId::TimestampS, TypeId::Time) => Some(CastGuard::FiniteAndScales(US_PER_S)),
        (
            TypeId::TimestampS,
            TypeId::Date | TypeId::Timestamp | TypeId::TimestampTz | TypeId::TimestampMs,
        ) => Some(CastGuard::InfiniteOrScales(US_PER_S)),
        (TypeId::TimestampS, TypeId::TimestampNs) => Some(CastGuard::InfiniteOrScales(NS_PER_S)),
        #[cfg(feature = "duckdb-1-5")]
        (TypeId::TimestampNs, TypeId::TimeNs) => Some(CastGuard::FiniteAndNanoDayFits),
        _ => None,
    }
}

/// `Interval::MICROS_PER_DAY`.
const MICROS_PER_DAY: i64 = 86_400_000_000;

/// The earliest finite `TIMESTAMP`, `290309-12-22 (BC) 00:00:00`: the smallest
/// value whose floor-divided day, times `MICROS_PER_DAY`, still fits an
/// `int64`. `Timestamp::Convert` (every rendering) throws below it.
pub(super) const TIMESTAMP_MIN_MICROS: i64 = -106_751_991 * MICROS_PER_DAY;

/// The earliest finite `TIMESTAMP_NS`, `1677-09-22 00:00:00`, by the same rule
/// at nanosecond precision (`Timestamp::Convert(timestamp_ns_t, …)`).
pub(super) const TIMESTAMP_NS_MIN_NANOS: i64 = -106_751 * NANOS_PER_DAY;

/// `dtime_tz_t::MAX_OFFSET`: `+15:59:59`, in seconds.
const TIME_TZ_MAX_OFFSET: u64 = 16 * 60 * 60 - 1;

/// Whether `v` is a value of the 64-bit temporal type `type_id` that `DuckDB`
/// can render and convert: the range its own SQL produces.
///
/// `DuckDB`'s C constructors (`duckdb_create_time`, `_timestamp_s`, …) accept
/// any `int64` and check nothing, but later operations on an out-of-range
/// value throw (an abort from Rust), read out of bounds or print garbage:
///
/// - `TIME`: `00:00:00`–`24:00:00`, i.e. `0..=MICROS_PER_DAY` microseconds.
///   `Value::time(-1)` rendered as `00:00:00.00000/` and `i64::MAX` threw.
/// - `TIME_NS`: `0..=NANOS_PER_DAY` nanoseconds. `i64::MAX` crashed
///   `StringAsTime` with `SIGSEGV`.
/// - `TIMESTAMP`, `TIMESTAMPTZ`: `±infinity` or at least
///   [`TIMESTAMP_MIN_MICROS`]; everything up to `i64::MAX - 1` is valid.
/// - `TIMESTAMP_S` / `_MS`: `±infinity`, or a value whose conversion to
///   microseconds (`FromEpochSeconds` / `FromEpochMs`, which throw on
///   overflow) is a valid `TIMESTAMP`.
/// - `TIMESTAMP_NS`: `±infinity` or at least [`TIMESTAMP_NS_MIN_NANOS`].
///
/// Any other type returns `false`.
pub const fn temporal_in_range(type_id: TypeId, v: i64) -> bool {
    let infinite = v == TS_INFINITY || v == TS_NEGATIVE_INFINITY;
    match type_id {
        TypeId::Time => v >= 0 && v <= MICROS_PER_DAY,
        #[cfg(feature = "duckdb-1-5")]
        TypeId::TimeNs => v >= 0 && v <= NANOS_PER_DAY,
        TypeId::Timestamp | TypeId::TimestampTz => infinite || v >= TIMESTAMP_MIN_MICROS,
        TypeId::TimestampS => infinite || scaled_timestamp_in_range(v, 1_000_000),
        TypeId::TimestampMs => infinite || scaled_timestamp_in_range(v, 1_000),
        TypeId::TimestampNs => infinite || v >= TIMESTAMP_NS_MIN_NANOS,
        _ => false,
    }
}

/// Whether `v * factor` is a valid finite `TIMESTAMP` in microseconds.
const fn scaled_timestamp_in_range(v: i64, factor: i64) -> bool {
    match v.checked_mul(factor) {
        Some(micros) => micros >= TIMESTAMP_MIN_MICROS,
        None => false,
    }
}

/// Whether `bits` is a `TIMETZ` encoding `DuckDB` produces: a time of day of
/// at most `24:00:00` in the high 40 bits and an offset of at most
/// `±15:59:59` in the low 24 (stored as `MAX_OFFSET - offset`, so
/// `0..=2 * MAX_OFFSET`). `Value::time_tz(u64::MAX)` rendered as
/// `b}:25:11.627775-4644:20:16`.
pub(super) const fn time_tz_in_range(bits: u64) -> bool {
    #[allow(clippy::cast_sign_loss, reason = "MICROS_PER_DAY is positive")]
    let max_micros = MICROS_PER_DAY as u64;
    (bits >> 24) <= max_micros && (bits & 0x00FF_FFFF) <= 2 * TIME_TZ_MAX_OFFSET
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_pairs_are_exactly_the_throwing_ones() {
        use CastGuard::{Finite, FiniteAndScales, InfiniteOrScales, Scales};
        let g = cast_guard;
        assert_eq!(g(TypeId::Timestamp, TypeId::Time), Some(Finite));
        assert_eq!(g(TypeId::Timestamp, TypeId::TimeTz), Some(Finite));
        assert_eq!(g(TypeId::TimestampTz, TypeId::TimeTz), Some(Finite));
        assert_eq!(g(TypeId::TimestampNs, TypeId::Time), Some(Finite));
        assert_eq!(
            g(TypeId::Timestamp, TypeId::TimestampNs),
            Some(InfiniteOrScales(1_000))
        );
        assert_eq!(
            g(TypeId::TimestampTz, TypeId::TimestampNs),
            Some(InfiniteOrScales(1_000))
        );
        assert_eq!(g(TypeId::TimestampMs, TypeId::Date), Some(Scales(1_000)));
        assert_eq!(g(TypeId::TimestampMs, TypeId::Time), Some(Scales(1_000)));
        assert_eq!(
            g(TypeId::TimestampMs, TypeId::Timestamp),
            Some(InfiniteOrScales(1_000))
        );
        assert_eq!(
            g(TypeId::TimestampMs, TypeId::TimestampTz),
            Some(InfiniteOrScales(1_000))
        );
        assert_eq!(
            g(TypeId::TimestampMs, TypeId::TimestampNs),
            Some(InfiniteOrScales(1_000_000))
        );
        assert_eq!(
            g(TypeId::TimestampS, TypeId::Time),
            Some(FiniteAndScales(1_000_000))
        );
        for target in [
            TypeId::Date,
            TypeId::Timestamp,
            TypeId::TimestampTz,
            TypeId::TimestampMs,
        ] {
            assert_eq!(
                g(TypeId::TimestampS, target),
                Some(InfiniteOrScales(1_000_000))
            );
        }
        assert_eq!(
            g(TypeId::TimestampS, TypeId::TimestampNs),
            Some(InfiniteOrScales(1_000_000_000))
        );
        // Pairs DuckDB converts without any throwing path.
        for (source, target) in [
            (TypeId::Time, TypeId::TimeTz),
            (TypeId::TimeTz, TypeId::Time),
            (TypeId::Timestamp, TypeId::Date),
            (TypeId::Timestamp, TypeId::TimestampTz),
            (TypeId::Timestamp, TypeId::TimestampMs),
            (TypeId::Timestamp, TypeId::TimestampS),
            (TypeId::TimestampTz, TypeId::Timestamp),
            (TypeId::TimestampNs, TypeId::Date),
            (TypeId::TimestampNs, TypeId::Timestamp),
            (TypeId::Date, TypeId::Timestamp),
            (TypeId::Date, TypeId::TimestampNs),
            (TypeId::Varchar, TypeId::Time),
            (TypeId::Timestamp, TypeId::Timestamp),
            (TypeId::BigInt, TypeId::Integer),
        ] {
            assert_eq!(g(source, target), None, "{source:?} -> {target:?}");
        }
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn timestamp_ns_to_time_ns_is_guarded() {
        assert_eq!(
            cast_guard(TypeId::TimestampNs, TypeId::TimeNs),
            Some(CastGuard::FiniteAndNanoDayFits)
        );
        assert_eq!(cast_guard(TypeId::TimeNs, TypeId::Time), None);
    }

    #[test]
    fn temporal_ranges_match_duckdbs() {
        let inf = TS_INFINITY;
        let ninf = TS_NEGATIVE_INFINITY;
        // TIME: 00:00:00 ..= 24:00:00.
        assert!(temporal_in_range(TypeId::Time, 0));
        assert!(temporal_in_range(TypeId::Time, MICROS_PER_DAY));
        assert!(!temporal_in_range(TypeId::Time, -1));
        assert!(!temporal_in_range(TypeId::Time, MICROS_PER_DAY + 1));
        assert!(!temporal_in_range(TypeId::Time, i64::MAX));

        // TIMESTAMP: -infinity, 290309-12-22 (BC) ..= infinity.
        for ty in [TypeId::Timestamp, TypeId::TimestampTz] {
            assert!(temporal_in_range(ty, inf));
            assert!(temporal_in_range(ty, ninf));
            assert!(temporal_in_range(ty, inf - 1));
            assert!(temporal_in_range(ty, 0));
            assert!(temporal_in_range(ty, -9_223_372_022_400_000_000));
            assert!(!temporal_in_range(ty, -9_223_372_022_400_000_001));
            assert!(!temporal_in_range(ty, ninf + 1));
            assert!(!temporal_in_range(ty, i64::MIN));
        }

        // TIMESTAMP_S: the seconds that convert to a valid TIMESTAMP.
        let s = TypeId::TimestampS;
        assert!(temporal_in_range(s, inf));
        assert!(temporal_in_range(s, ninf));
        assert!(temporal_in_range(s, 9_223_372_036_854));
        assert!(
            !temporal_in_range(s, 9_223_372_036_855),
            "rounded TIMESTAMP max"
        );
        assert!(temporal_in_range(s, -9_223_372_022_400));
        assert!(!temporal_in_range(s, -9_223_372_022_401));
        assert!(!temporal_in_range(s, 100_000_000_000_000));
        assert!(!temporal_in_range(s, i64::MIN));

        let ms = TypeId::TimestampMs;
        assert!(temporal_in_range(ms, inf));
        assert!(temporal_in_range(ms, 9_223_372_036_854_775));
        assert!(!temporal_in_range(ms, 9_223_372_036_854_776));
        assert!(temporal_in_range(ms, -9_223_372_022_400_000));
        assert!(!temporal_in_range(ms, -9_223_372_022_400_001));
        assert!(!temporal_in_range(ms, inf - 1));

        // TIMESTAMP_NS: 1677-09-22 ..= infinity.
        let ns = TypeId::TimestampNs;
        assert!(temporal_in_range(ns, inf));
        assert!(temporal_in_range(ns, ninf));
        assert!(temporal_in_range(ns, inf - 1));
        assert!(temporal_in_range(ns, -9_223_286_400_000_000_000));
        assert!(!temporal_in_range(ns, -9_223_286_400_000_000_001));
        assert!(!temporal_in_range(ns, i64::MIN));

        assert!(!temporal_in_range(TypeId::BigInt, 0));
        assert!(!temporal_in_range(TypeId::Date, 0));
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn time_ns_range_is_a_day() {
        assert!(temporal_in_range(TypeId::TimeNs, 0));
        assert!(temporal_in_range(TypeId::TimeNs, NANOS_PER_DAY));
        assert!(!temporal_in_range(TypeId::TimeNs, NANOS_PER_DAY + 1));
        assert!(!temporal_in_range(TypeId::TimeNs, -1));
        assert!(!temporal_in_range(TypeId::TimeNs, i64::MAX));
    }

    #[test]
    fn time_tz_range_covers_both_fields() {
        let bits = |micros: u64, offset: i64| {
            (micros << 24) | u64::try_from(57_599 - offset).expect("in range")
        };
        assert!(time_tz_in_range(bits(0, 0)));
        assert!(time_tz_in_range(bits(86_400_000_000, -57_599)));
        assert!(time_tz_in_range(bits(0, 57_599)));
        assert!(!time_tz_in_range(bits(86_400_000_001, 0)));
        assert!(!time_tz_in_range(2 * 57_599 + 1));
        assert!(!time_tz_in_range(u64::MAX));
    }

    #[test]
    fn guards_accept_exactly_the_non_throwing_inputs() {
        let inf = TS_INFINITY;
        let ninf = TS_NEGATIVE_INFINITY;
        assert!(CastGuard::Finite.accepts(0));
        assert!(CastGuard::Finite.accepts(i64::MIN));
        assert!(CastGuard::Finite.accepts(inf - 1));
        assert!(!CastGuard::Finite.accepts(inf));
        assert!(!CastGuard::Finite.accepts(ninf));

        let ms = CastGuard::Scales(1_000);
        assert!(ms.accepts(i64::MAX / 1_000));
        assert!(ms.accepts(i64::MIN / 1_000));
        assert!(!ms.accepts(i64::MAX / 1_000 + 1));
        assert!(!ms.accepts(i64::MIN / 1_000 - 1));
        assert!(
            !ms.accepts(inf),
            "FromEpochMs does not special-case infinity"
        );
        assert!(!ms.accepts(ninf));

        let s = CastGuard::InfiniteOrScales(1_000_000);
        assert!(s.accepts(inf));
        assert!(s.accepts(ninf));
        assert!(s.accepts(i64::MAX / 1_000_000));
        assert!(!s.accepts(i64::MAX / 1_000_000 + 1));
        assert!(!s.accepts(i64::MIN));

        let t = CastGuard::FiniteAndScales(1_000_000);
        assert!(!t.accepts(inf));
        assert!(!t.accepts(ninf));
        assert!(t.accepts(-(i64::MAX / 1_000_000)));
        assert!(!t.accepts(-(i64::MAX / 1_000_000) - 2));
    }

    #[cfg(feature = "duckdb-1-5")]
    #[test]
    fn the_time_ns_guard_matches_get_time_ns() {
        let (inf, ninf) = (TS_INFINITY, TS_NEGATIVE_INFINITY);
        let day = CastGuard::FiniteAndNanoDayFits;
        assert!(day.accepts(0));
        assert!(day.accepts(-1));
        assert!(day.accepts(inf - 1));
        // TIMESTAMP_NS's smallest renderable instant, 1677-09-22: its day
        // times NANOS_PER_DAY is the last multiple that fits.
        assert!(day.accepts(-106_751 * NANOS_PER_DAY));
        assert!(!day.accepts(-106_751 * NANOS_PER_DAY - 1));
        assert!(!day.accepts(i64::MIN));
        assert!(!day.accepts(inf));
        assert!(!day.accepts(ninf));
    }
}
