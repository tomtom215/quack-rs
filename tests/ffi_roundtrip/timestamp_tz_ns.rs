// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `TIMESTAMPTZ_NS`, the type `DuckDB` 2.0.0 added (type id 42).
//!
//! quack-rs supports it without the two C functions 2.0.0 added for it
//! (`duckdb_create_timestamp_tz_ns` / `duckdb_get_timestamp_tz_ns`), which sit
//! in slots 547 and 548 of an API struct that a build against the 1.5.2 –
//! 1.5.6 bindings, and every engine before 2.0.0, does not have. So the same
//! binary reads and writes the type on 2.0.0 and never calls past the end of
//! the struct on 1.5.x. Each test therefore holds the engine to its own
//! contract: on 2.0 the type works end to end and agrees with `DuckDB`'s SQL;
//! before 2.0 there is no such type, and every call that would need it
//! returns an error or `None` rather than aborting.

use quack_rs::data_chunk::DataChunk;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;
use quack_rs::vector::VectorWriter;

use super::{register_echo, Fixture};

echo_fn!(
    echo_timestamp_tz_ns,
    read_timestamp_tz_ns,
    write_timestamp_tz_ns
);

/// Nanoseconds since the epoch of `2026-08-18 12:00:00.123456789 UTC`.
const NANOS: i64 = 1_787_054_400_123_456_789;

/// The smallest finite payload `Value::timestamp_tz_ns` accepts.
const MIN_FINITE: i64 = -9_223_286_400_000_000_000;

/// The payloads the Value tests run: the epoch and its neighbours, an
/// ordinary instant, both ends of the finite range and both infinities.
const EDGES: [i64; 9] = [
    0,
    1,
    -1,
    NANOS,
    MIN_FINITE,
    MIN_FINITE + 1,
    i64::MAX - 1,
    i64::MAX,
    -i64::MAX,
];

/// How `DuckDB` renders `nanos` as `TIMESTAMPTZ_NS`, asked of its SQL.
fn sql_render(fx: &Fixture, nanos: i64) -> String {
    let literal = match nanos {
        i64::MAX => "'infinity'".to_owned(),
        n if n == -i64::MAX => "'-infinity'".to_owned(),
        n => format!("make_timestamp_ns({n}::BIGINT)"),
    };
    fx.scalar(
        &format!("SELECT CAST({literal}::TIMESTAMPTZ_NS AS VARCHAR)"),
        // SAFETY: one VARCHAR column, row 0.
        |r, i| unsafe { r.read_str(i).to_owned() },
    )
    .expect("not NULL")
}

#[test]
fn the_type_id_is_reported_for_a_column_and_creatable_only_where_it_exists() {
    let fx = Fixture::open();
    if fx.engine_major() < 2 {
        // No such type: creating it fails with an error, not an abort. The id
        // never reaches the engine, since a DuckDB built with assertions (the
        // `bundled-test` build) aborts the process on a type id it does not know.
        let err = LogicalType::try_new(TypeId::TimestampTzNs).expect_err("no such type");
        assert!(err.to_string().contains("added in DuckDB 2.0.0"), "{err}");
        return;
    }
    let ty = LogicalType::try_new(TypeId::TimestampTzNs).expect("2.0 has the type");
    // SAFETY: `ty` is a live handle.
    assert_eq!(unsafe { ty.try_get_type_id() }, Some(TypeId::TimestampTzNs));
    let result = fx.query("SELECT TIMESTAMPTZ_NS '2026-08-18 12:00:00.123456789+00' AS t");
    assert_eq!(result.column_type(0), Some(TypeId::TimestampTzNs));
}

#[test]
fn a_scalar_function_over_the_type_round_trips_nanoseconds_and_nulls() {
    let fx = Fixture::open();
    if fx.engine_major() < 2 {
        let err = quack_rs::scalar::ScalarFunctionBuilder::try_new("echo_ts_tz_ns")
            .expect("name")
            .param(TypeId::TimestampTzNs)
            .returns(TypeId::TimestampTzNs)
            .function(echo_timestamp_tz_ns);
        // SAFETY: `con` is open; the registration is expected to fail.
        let result = unsafe { err.register(fx.con()) };
        assert!(result.is_err(), "a DuckDB without the type refuses it");
        return;
    }
    register_echo(
        fx.con(),
        "echo_ts_tz_ns",
        TypeId::TimestampTzNs,
        TypeId::TimestampTzNs,
        echo_timestamp_tz_ns,
    );
    // SAFETY (every read below): one TIMESTAMPTZ_NS column, row 0.
    let got = fx
        .scalar(
            "SELECT echo_ts_tz_ns(TIMESTAMPTZ_NS '2026-08-18 12:00:00.123456789+00')",
            |r, i| unsafe { r.read_timestamp_tz_ns(i) },
        )
        .expect("not NULL");
    assert_eq!(got, NANOS);
    // The function wrote what DuckDB itself calls that instant.
    assert_eq!(
        fx.scalar(
            "SELECT epoch_ns(echo_ts_tz_ns(TIMESTAMPTZ_NS '2026-08-18 12:00:00.123456789+00'))",
            |r, i| unsafe { r.read_i64(i) },
        ),
        Some(NANOS)
    );
    // A NULL stays NULL, and a column of values keeps every row.
    assert_eq!(
        fx.scalar(
            "SELECT echo_ts_tz_ns(NULL::TIMESTAMPTZ_NS)",
            |r, i| unsafe { r.read_timestamp_tz_ns(i) }
        ),
        None
    );
    assert_eq!(
        fx.scalar(
            "SELECT sum(epoch_ns(echo_ts_tz_ns(make_timestamp_ns(i * 1000)::TIMESTAMPTZ_NS)) - i * 1000) \
             FROM range(5000) t(i)",
            |r, i| unsafe { r.read_i128(i) },
        ),
        Some(0)
    );
}

#[test]
fn value_constructor_and_getter_round_trip_and_render_like_duckdb() {
    let fx = Fixture::open();
    if fx.engine_major() < 2 {
        for nanos in EDGES {
            assert!(
                Value::timestamp_tz_ns(nanos).is_err(),
                "{nanos}: an engine before 2.0.0 cannot make the value"
            );
        }
        // And the reader has nothing to read, however the value was made.
        let ns = Value::timestamp_ns(NANOS).expect("in range");
        assert_eq!(ns.as_timestamp_tz_ns(), None);
        return;
    }
    fx.query("SET TimeZone = 'UTC'");
    for nanos in EDGES {
        let value = Value::timestamp_tz_ns(nanos).unwrap_or_else(|e| panic!("{nanos}: {e}"));
        assert_eq!(value.type_id(), Some(TypeId::TimestampTzNs), "{nanos}");
        assert_eq!(value.as_timestamp_tz_ns(), Some(nanos), "{nanos}");
        // The same 64 bits read as the plain-nanosecond sibling.
        assert_eq!(value.as_timestamp_ns(), Some(nanos), "{nanos}");
        assert_eq!(
            value.as_str().expect("renders"),
            sql_render(&fx, nanos),
            "{nanos}: Value rendering vs. SQL"
        );
    }
    // Out of range is refused before DuckDB sees it.
    assert!(Value::timestamp_tz_ns(i64::MIN).is_err());
    assert!(Value::timestamp_tz_ns(MIN_FINITE - 1).is_err());
}

/// The getter is a casting accessor like its siblings: it reads other
/// temporal types as `TIMESTAMPTZ_NS`, and refuses the conversions `DuckDB`
/// would overflow (or throw on) instead of aborting.
#[test]
fn the_getter_casts_other_temporal_values_and_refuses_overflow() {
    let fx = Fixture::open();
    if fx.engine_major() < 2 {
        return;
    }
    fx.query("SET TimeZone = 'UTC'");
    let micros = 1_787_054_400_123_456_i64;
    assert_eq!(
        Value::timestamp(micros)
            .expect("in range")
            .as_timestamp_tz_ns(),
        Some(micros * 1_000)
    );
    assert_eq!(
        Value::timestamp_ms(1_787_054_400_123)
            .expect("in range")
            .as_timestamp_tz_ns(),
        Some(1_787_054_400_123_000_000)
    );
    assert_eq!(
        Value::timestamp_s(1_787_054_400)
            .expect("in range")
            .as_timestamp_tz_ns(),
        Some(1_787_054_400_000_000_000)
    );
    // Infinity stays infinity through the conversion.
    assert_eq!(
        Value::timestamp(i64::MAX)
            .expect("infinity")
            .as_timestamp_tz_ns(),
        Some(i64::MAX)
    );
    // A TIMESTAMP too far from the epoch for nanoseconds: DuckDB's own
    // cast raises an error here, which the getter must report as None.
    assert_eq!(
        Value::timestamp(i64::MAX - 1)
            .expect("in range")
            .as_timestamp_tz_ns(),
        None
    );
    assert_eq!(
        Value::timestamp(-9_223_372_022_400_000_000)
            .expect("in range")
            .as_timestamp_tz_ns(),
        None
    );
    // Types DuckDB does not convert to TIMESTAMPTZ_NS yield None.
    assert_eq!(Value::integer(5).as_timestamp_tz_ns(), None);
    assert_eq!(Value::boolean(true).as_timestamp_tz_ns(), None);
    // The converse direction, for the casts `DuckDB`'s default cast set
    // defines out of `TIMESTAMPTZ_NS` (`TimestampTzNsCastSwitch`: `VARCHAR`,
    // `TIMETZ`, `TIMESTAMP_NS`, `TIMESTAMPTZ`, `TIMESTAMP_MS` and
    // `TIMESTAMP_S`; it has none to a plain `TIMESTAMP`, so `as_timestamp`
    // is `None`). The full matrix against SQL is in `value_temporal`.
    // `NANOS` ends in `…123_456_789`: DuckDB rounds away from the epoch when it
    // scales nanoseconds down (`TryCastTimestampBase`), so µs gets `…457`.
    let value = Value::timestamp_tz_ns(NANOS).expect("in range");
    assert_eq!(value.as_timestamp_tz(), Some(1_787_054_400_123_457));
    assert_eq!(value.as_timestamp_ms(), Some(1_787_054_400_123));
    assert_eq!(value.as_timestamp_s(), Some(1_787_054_400));
    assert_eq!(
        value.as_str().expect("renders"),
        "2026-08-18 12:00:00.123456789+00"
    );
}

/// Every type the linked engine can produce is a type quack-rs knows.
///
/// `test_all_types()` returns one column per SQL type the engine has, so this
/// fails the day a `DuckDB` release adds a type with an id the crate has no
/// `TypeId` for: `TIMESTAMPTZ_NS` (id 42, added in 2.0.0) was found that way,
/// as a `LogicalType::get_type_id` panic, long after it shipped.
#[test]
fn every_type_the_engine_has_is_a_type_quack_rs_knows() {
    // `TypeId::Geometry` and `TypeId::Variant` exist only with `duckdb-1-5-3`,
    // so without it the engine's `geometry` and `variant` columns are expected
    // to have no `TypeId`; with it, none may be missing.
    #[cfg(not(feature = "duckdb-1-5-3"))]
    const FEATURE_GATED: &[&str] = &["geometry", "variant"];
    #[cfg(feature = "duckdb-1-5-3")]
    const FEATURE_GATED: &[&str] = &[];

    let fx = Fixture::open();
    let result = fx.query("SELECT * FROM test_all_types()");
    let unknown: Vec<String> = (0..result.column_count())
        .filter(|&i| result.column_type(i).is_none())
        .map(|i| {
            result
                .column_name(i)
                .unwrap_or_else(|| format!("column {i}"))
        })
        .filter(|name| !FEATURE_GATED.contains(&name.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "columns whose type has no TypeId (engine major {}): {unknown:?}",
        fx.engine_major()
    );
    assert!(
        result.column_count() > 40,
        "the sweep saw the engine's types"
    );
}
