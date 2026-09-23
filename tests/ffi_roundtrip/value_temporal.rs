// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Every `Value` getter against every temporal source type, at every edge
//! `DuckDB`'s SQL can produce: the epoch, an ordinary value, the minimum, the
//! maximum and both infinities.
//!
//! The values come from SQL — a table function's named parameters — because
//! that is how an extension receives them. Several (source, getter) pairs
//! used to abort the process: `DuckDB` binds them through
//! `TemplatedCastLoop<…, duckdb::Cast>` (`time_casts.cpp`), which throws a C++
//! exception instead of reporting a failed cast, and `as_time()` on
//! `'infinity'::TIMESTAMP` was enough. The matrix runs in-process: one abort
//! kills the whole test binary.
//!
//! Each getter is also checked against `DuckDB` itself: where
//! `CAST(literal AS target)` fails in SQL the getter must return `None`, and
//! where it succeeds the getter must return the same value.

use std::sync::Mutex;

use quack_rs::query::OwnedConnection;
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;
use quack_rs::value::Value;

use super::Fixture;

/// `(named parameter, its SQL type)` for every temporal source type.
const SOURCES: &[(&str, &str, TypeId)] = &[
    ("v_date", "DATE", TypeId::Date),
    ("v_time", "TIME", TypeId::Time),
    ("v_time_tz", "TIMETZ", TypeId::TimeTz),
    ("v_ts", "TIMESTAMP", TypeId::Timestamp),
    ("v_ts_tz", "TIMESTAMPTZ", TypeId::TimestampTz),
    ("v_ts_s", "TIMESTAMP_S", TypeId::TimestampS),
    ("v_ts_ms", "TIMESTAMP_MS", TypeId::TimestampMs),
    ("v_ts_ns", "TIMESTAMP_NS", TypeId::TimestampNs),
    #[cfg(feature = "duckdb-1-5")]
    ("v_time_ns", "TIME_NS", TypeId::TimeNs),
];

/// The instants every `TIMESTAMP`-family source is tried at, as `TIMESTAMP`
/// literals. Each is cast to the source type in SQL; the ones that type
/// cannot hold are skipped (see [`literals`]).
const TIMESTAMP_EDGES: &[&str] = &[
    "TIMESTAMP '1970-01-01 00:00:00'",
    "TIMESTAMP '2024-02-29 12:34:56.789012'",
    // Outside TIMESTAMP_NS's range on both sides: `as_timestamp_ns` of these
    // used to throw from `Timestamp::GetEpochNanoSeconds`.
    "TIMESTAMP '1000-01-01 00:00:00'",
    "TIMESTAMP '3000-01-01 00:00:00'",
    // TIMESTAMP's own minimum and maximum.
    "TIMESTAMP '290309-12-22 (BC) 00:00:00'",
    "TIMESTAMP '294247-01-10 04:00:54.775806'",
    // The largest value each coarser type can still convert back to
    // microseconds.
    "TIMESTAMP '294247-01-10 04:00:54'",
    "TIMESTAMP '294247-01-10 04:00:54.775'",
    "'infinity'::TIMESTAMP",
    "'-infinity'::TIMESTAMP",
];

/// Every SQL literal of `sql_type` this test feeds in.
fn literals(sql_type: &str) -> Vec<String> {
    match sql_type {
        "DATE" => [
            "DATE '1970-01-01'",
            "DATE '2024-02-29'",
            "DATE '5877642-06-25 (BC)'",
            "DATE '5881580-07-10'",
            "'infinity'::DATE",
            "'-infinity'::DATE",
        ]
        .map(str::to_owned)
        .to_vec(),
        "TIME" | "TIME_NS" => ["'00:00:00'", "'12:34:56.789012'", "'24:00:00'"]
            .map(|t| format!("{t}::{sql_type}"))
            .to_vec(),
        "TIMETZ" => [
            "'00:00:00+15:59:59'",
            "'12:34:56.789+05:30'",
            "'24:00:00-15:59:59'",
            "'00:00:00+00'",
        ]
        .map(|t| format!("{t}::TIMETZ"))
        .to_vec(),
        "TIMESTAMP_NS" => {
            let mut out: Vec<String> = TIMESTAMP_EDGES
                .iter()
                .map(|t| format!("({t})::TIMESTAMP_NS"))
                .collect();
            // TIMESTAMP_NS's own extremes, which no TIMESTAMP literal reaches
            // exactly.
            out.push("make_timestamp_ns(-9223286400000000000)".to_owned());
            out.push("make_timestamp_ns(9223372036854775806)".to_owned());
            out
        }
        _ => TIMESTAMP_EDGES
            .iter()
            .map(|t| format!("({t})::{sql_type}"))
            .collect(),
    }
}

/// Every getter on `value`, each rendered through `DuckDB` by a constructor of
/// its target type, keyed by that target's SQL name. `None` where the getter
/// returned `None`.
///
/// `as_str`, `display_string` and `Debug` are run too: they render the value
/// through `DuckDB`'s own (throwing) cast to `VARCHAR`.
fn every_getter(value: &Value) -> GetterResults {
    fn text(v: &Value) -> String {
        v.as_str().expect("a getter result renders")
    }
    let _ = format!("{value:?}");
    #[cfg(feature = "duckdb-1-5")]
    let _ = value.display_string();
    #[cfg_attr(not(feature = "duckdb-1-5"), allow(unused_mut))]
    let mut out = vec![
        ("VARCHAR", value.as_str().ok()),
        ("BOOLEAN", value.as_bool().map(|x| text(&Value::boolean(x)))),
        ("TINYINT", value.as_i8().map(|x| text(&Value::tinyint(x)))),
        (
            "SMALLINT",
            value.as_i16().map(|x| text(&Value::smallint(x))),
        ),
        ("INTEGER", value.as_i32().map(|x| text(&Value::integer(x)))),
        ("BIGINT", value.as_i64().map(|x| text(&Value::bigint(x)))),
        ("HUGEINT", value.as_i128().map(|x| text(&Value::hugeint(x)))),
        ("UTINYINT", value.as_u8().map(|x| text(&Value::utinyint(x)))),
        (
            "USMALLINT",
            value.as_u16().map(|x| text(&Value::usmallint(x))),
        ),
        (
            "UINTEGER",
            value.as_u32().map(|x| text(&Value::uinteger(x))),
        ),
        ("UBIGINT", value.as_u64().map(|x| text(&Value::ubigint(x)))),
        (
            "UHUGEINT",
            value.as_u128().map(|x| text(&Value::uhugeint(x))),
        ),
        ("FLOAT", value.as_f32().map(|x| text(&Value::float(x)))),
        ("DOUBLE", value.as_f64().map(|x| text(&Value::double(x)))),
        ("DATE", value.as_date().map(|x| text(&Value::date(x)))),
        (
            "TIME",
            value
                .as_time()
                .map(|x| text(&Value::time(x).expect("a getter result is in range"))),
        ),
        (
            "TIMETZ",
            value
                .as_time_tz()
                .map(|x| text(&Value::time_tz(x).expect("a getter result is in range"))),
        ),
        (
            "TIMESTAMP",
            value
                .as_timestamp()
                .map(|x| text(&Value::timestamp(x).expect("a getter result is in range"))),
        ),
        (
            "TIMESTAMPTZ",
            value
                .as_timestamp_tz()
                .map(|x| text(&Value::timestamp_tz(x).expect("a getter result is in range"))),
        ),
        (
            "TIMESTAMP_S",
            value
                .as_timestamp_s()
                .map(|x| text(&Value::timestamp_s(x).expect("a getter result is in range"))),
        ),
        (
            "TIMESTAMP_MS",
            value
                .as_timestamp_ms()
                .map(|x| text(&Value::timestamp_ms(x).expect("a getter result is in range"))),
        ),
        (
            "TIMESTAMP_NS",
            value
                .as_timestamp_ns()
                .map(|x| text(&Value::timestamp_ns(x).expect("a getter result is in range"))),
        ),
        (
            "INTERVAL",
            value.as_interval().map(|x| text(&Value::interval(x))),
        ),
        ("UUID", value.as_uuid().map(|x| text(&Value::uuid(x)))),
    ];
    #[cfg(feature = "duckdb-1-5")]
    out.push((
        "TIME_NS",
        value
            .as_time_ns()
            .map(|x| text(&Value::time_ns(x).expect("a getter result is in range"))),
    ));
    out
}

/// Every getter's result on one value: `(target SQL type, rendered result)`.
type GetterResults = Vec<(&'static str, Option<String>)>;

/// What each call of the table function saw: `(literal, results)`.
static SEEN: Mutex<Vec<(String, GetterResults)>> = Mutex::new(Vec::new());

fn register_probe(fx: &Fixture) {
    let mut builder =
        TableFunctionBuilder::new("vq_temporal").named_param("label", TypeId::Varchar);
    for &(param, _, type_id) in SOURCES {
        builder = builder.named_param(param, type_id);
    }
    let table_fn = builder
        .with_state::<bool, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: every name read here was declared above.
            let label = unsafe { bind.get_named_parameter_value("label") }
                .as_str()
                .unwrap_or_default();
            for &(param, _, _) in SOURCES {
                // SAFETY: as above.
                let value = unsafe { bind.get_named_parameter_value(param) };
                if value.is_null() {
                    continue;
                }
                let results = every_getter(&value);
                SEEN.lock()
                    .expect("not poisoned")
                    .push((label.clone(), results));
            }
            Ok(true)
        })
        .scan(|pending, chunk| {
            // SAFETY: column 0 is BIGINT; row 0 is in range; a zero size ends
            // the scan.
            unsafe {
                if std::mem::take(pending) {
                    chunk.writer(0).write_i64(0, 1);
                    chunk.set_size(1);
                } else {
                    chunk.set_size(0);
                }
            }
            Ok(())
        })
        .build()
        .expect("build vq_temporal");
    // SAFETY: `con` is open for the fixture's lifetime.
    unsafe { table_fn.register(fx.con()) }.expect("register vq_temporal");
}

/// `CAST(literal AS target)::VARCHAR` in SQL, or `None` when `DuckDB` rejects
/// the cast.
fn sql_cast(con: &OwnedConnection, literal: &str, target: &str) -> Option<String> {
    let sql = format!("SELECT CAST(CAST({literal} AS {target}) AS VARCHAR)");
    let mut result = con.query(&sql).ok()?;
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: one VARCHAR column, one row.
    unsafe { chunk.reader(0).read_str(0).to_owned() }.into()
}

/// The regression test for the audit's F-V1: before the fix, the first
/// `'infinity'::TIMESTAMP` read with `as_time` aborted the process ("Rust
/// cannot catch foreign exceptions").
#[test]
fn every_getter_on_every_temporal_edge_matches_duckdb_and_never_aborts() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    // The linked library carries ICU, whose casts depend on the time zone.
    // The C API getters use DuckDB's built-in casts, which assume UTC.
    con.execute("SET TimeZone = 'UTC'").expect("set time zone");
    register_probe(&fx);

    let (mut calls, mut some) = (0_usize, 0_usize);
    for &(param, sql_type, _) in SOURCES {
        for literal in literals(sql_type) {
            // A literal the source type cannot hold is an error in SQL too;
            // that is DuckDB's range, not a getter's.
            if sql_cast(&con, &literal, sql_type).is_none() {
                continue;
            }
            SEEN.lock().expect("not poisoned").clear();
            let sql =
                format!("SELECT n FROM vq_temporal(label := $${literal}$$, {param} := {literal})");
            con.query(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
            let seen = std::mem::take(&mut *SEEN.lock().expect("not poisoned"));
            // One bind per call; each saw exactly the one parameter set.
            assert!(!seen.is_empty(), "{sql}: the bind callback ran");
            for (label, results) in &seen {
                assert_eq!(label, &literal);
                for (target, got) in results {
                    calls += 1;
                    some += usize::from(got.is_some());
                    // With ICU loaded, SQL casts involving TIMESTAMPTZ take ICU's
                    // calendar path rather than the built-in cast the C API uses,
                    // so SQL is not an oracle for them. They are still run above:
                    // not aborting is the point.
                    if sql_type == "TIMESTAMPTZ" || *target == "TIMESTAMPTZ" {
                        continue;
                    }
                    let want = sql_cast(&con, &literal, target);
                    assert_eq!(
                        got, &want,
                        "{literal} read as {target}: getter vs CAST in SQL"
                    );
                }
            }
        }
    }
    assert!(calls > 1_000, "the matrix ran ({calls} getter calls)");
    assert!(
        some > 150,
        "and most conversions succeed ({some} of {calls})"
    );
}

/// The original report: a TIMESTAMP named parameter bound to
/// `'infinity'::TIMESTAMP`, read with `as_time` during bind.
#[test]
fn as_time_of_an_infinite_timestamp_parameter_is_none() {
    let fx = Fixture::open();
    let table_fn = TableFunctionBuilder::new("vq_inf_time")
        .named_param("t", TypeId::Timestamp)
        .with_state::<Option<i64>, _>(|bind| {
            bind.add_result_column("x", TypeId::BigInt);
            // SAFETY: "t" was declared above.
            let t = unsafe { bind.get_named_parameter_value("t") };
            Ok(Some(t.as_time().unwrap_or(-1)))
        })
        .scan(|state, chunk| {
            match state.take() {
                // SAFETY: column 0 is BIGINT; row 0 is in range.
                Some(n) => unsafe {
                    chunk.writer(0).write_i64(0, n);
                    chunk.set_size(1);
                },
                // SAFETY: ending the scan.
                None => unsafe { chunk.set_size(0) },
            }
            Ok(())
        })
        .build()
        .expect("build vq_inf_time");
    // SAFETY: `con` is open.
    unsafe { table_fn.register(fx.con()) }.expect("register vq_inf_time");
    let read = |sql: &str| fx.scalar(sql, |r, i| unsafe { r.read_i64(i) });
    assert_eq!(
        read("SELECT x FROM vq_inf_time(t := TIMESTAMP '2024-01-01 12:00:00')"),
        Some(12 * 3_600 * 1_000_000)
    );
    assert_eq!(
        read("SELECT x FROM vq_inf_time(t := 'infinity'::TIMESTAMP)"),
        Some(-1)
    );
    assert_eq!(
        read("SELECT x FROM vq_inf_time(t := '-infinity'::TIMESTAMP)"),
        Some(-1)
    );
}

// ── F-V2: temporal constructors accept only values DuckDB can use ────────────

/// `DuckDB`'s `duckdb_create_time` / `_timestamp_s` / … store any `int64`
/// unchecked, and a later `as_str()`, `Debug` or getter on an out-of-range one
/// threw through the C API (abort), crashed (`Value::time_ns(i64::MAX)`:
/// `SIGSEGV` in `StringAsTime`) or printed garbage (`Value::time(-1)` rendered
/// `00:00:00.00000/`). Those inputs are now errors, and every value just
/// inside each bound renders.
#[test]
fn temporal_constructors_reject_what_duckdb_cannot_render() {
    const DAY_US: i64 = 86_400_000_000;
    const TS_MIN: i64 = -9_223_372_022_400_000_000;
    let _fx = Fixture::open();

    // Each input that aborted, crashed or rendered garbage before.
    assert!(Value::time(-1).is_err());
    assert!(Value::time(DAY_US + 1).is_err());
    assert!(Value::time(i64::MAX).is_err());
    assert!(Value::time_tz(u64::MAX).is_err());
    assert!(Value::timestamp(i64::MIN).is_err());
    assert!(Value::timestamp(TS_MIN - 1).is_err());
    assert!(Value::timestamp_tz(i64::MIN).is_err());
    assert!(Value::timestamp_s(100_000_000_000_000).is_err());
    assert!(Value::timestamp_s(9_223_372_036_855).is_err());
    assert!(Value::timestamp_s(i64::MAX - 1).is_err());
    assert!(Value::timestamp_ms(i64::MAX - 1).is_err());
    assert!(Value::timestamp_ms(9_223_372_036_854_776).is_err());
    assert!(Value::timestamp_ns(i64::MIN).is_err());
    assert!(Value::timestamp_ns(-9_223_286_400_000_000_001).is_err());
    #[cfg(feature = "duckdb-1-5")]
    {
        assert!(Value::time_ns(i64::MAX).is_err());
        assert!(Value::time_ns(-1).is_err());
        assert!(Value::time_ns(86_400_000_000_001).is_err());
    }
    let message = Value::time(-1).expect_err("out of range").to_string();
    assert!(message.contains("TIME"), "{message}");

    // Every accepted boundary renders — through `as_str`, `Debug` and (with
    // `duckdb-1-5`) `display_string` — to what DuckDB's SQL prints.
    let ok = |v: Result<Value, _>| v.expect("in range");
    let tz_max = ((DAY_US as u64) << 24) | (2 * 57_599);
    #[cfg_attr(not(feature = "duckdb-1-5"), allow(unused_mut))]
    let mut cases: Vec<(Value, &str)> = vec![
        (ok(Value::time(0)), "00:00:00"),
        (ok(Value::time(DAY_US)), "24:00:00"),
        (ok(Value::time_tz(0)), "00:00:00+15:59:59"),
        (ok(Value::time_tz(tz_max)), "24:00:00-15:59:59"),
        (ok(Value::timestamp(TS_MIN)), "290309-12-22 (BC) 00:00:00"),
        (
            ok(Value::timestamp(i64::MAX - 1)),
            "294247-01-10 04:00:54.775806",
        ),
        (ok(Value::timestamp(i64::MAX)), "infinity"),
        (ok(Value::timestamp(-i64::MAX)), "-infinity"),
        (
            ok(Value::timestamp_tz(TS_MIN)),
            "290309-12-22 (BC) 00:00:00+00",
        ),
        (ok(Value::timestamp_tz(-i64::MAX)), "-infinity"),
        (
            ok(Value::timestamp_s(-9_223_372_022_400)),
            "290309-12-22 (BC) 00:00:00",
        ),
        (
            ok(Value::timestamp_s(9_223_372_036_854)),
            "294247-01-10 04:00:54",
        ),
        (ok(Value::timestamp_s(i64::MAX)), "infinity"),
        (
            ok(Value::timestamp_ms(-9_223_372_022_400_000)),
            "290309-12-22 (BC) 00:00:00",
        ),
        (
            ok(Value::timestamp_ms(9_223_372_036_854_775)),
            "294247-01-10 04:00:54.775",
        ),
        (ok(Value::timestamp_ms(-i64::MAX)), "-infinity"),
        (
            ok(Value::timestamp_ns(-9_223_286_400_000_000_000)),
            "1677-09-22 00:00:00",
        ),
        (
            ok(Value::timestamp_ns(i64::MAX - 1)),
            "2262-04-11 23:47:16.854775806",
        ),
        (ok(Value::timestamp_ns(i64::MAX)), "infinity"),
        // DATE and INTERVAL stay infallible: DuckDB renders every i32 day
        // (`Date::Convert` normalises by 400-year steps) and every interval
        // (`IntervalToStringCast`'s buffer is sized for the extremes).
        (Value::date(i32::MAX), "infinity"),
        (Value::date(-i32::MAX), "-infinity"),
        (
            Value::interval(quack_rs::interval::DuckInterval {
                months: i32::MIN,
                days: i32::MIN,
                micros: i64::MIN,
            }),
            "-178956970 years -8 months -2147483648 days -2562047788:00:54.775808",
        ),
    ];
    #[cfg(feature = "duckdb-1-5")]
    {
        cases.push((ok(Value::time_ns(86_400_000_000_000)), "24:00:00"));
        cases.push((ok(Value::time_ns(0)), "00:00:00"));
    }
    for (value, want) in &cases {
        assert_eq!(value.as_str().expect("renders"), *want, "{value:?}");
        let debug = format!("{value:?}");
        assert!(debug.starts_with("Value"), "{debug}");
        #[cfg(feature = "duckdb-1-5")]
        assert!(
            value.display_string().is_some_and(|s| s.contains(want)),
            "{debug}"
        );
    }
    // The two ends of DATE's i32 range that are not infinities render too.
    for days in [i32::MIN, i32::MIN + 1, i32::MAX - 1] {
        let text = Value::date(days).as_str().expect("renders");
        assert!(!text.is_empty(), "{days}: {text}");
    }
}
