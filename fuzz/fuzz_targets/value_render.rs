// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!
//! Fuzzes `Value` rendering and the typed getters over arbitrary temporal
//! payloads, alone and nested in lists, against a real `DuckDB`.
//!
//! `duckdb_get_varchar`, `duckdb_value_to_string` and the converting getters
//! cast without catching, so a payload outside a type's range used to abort
//! the process (`docs/upstream-duckdb-reports.md`, items 13 and 14). SQL
//! produces such payloads (`make_timestamp(-9223372036854775808)`), so the
//! values here are built with the raw `duckdb_create_*` constructors, which
//! take any payload, rather than `Value`'s checked ones. Every renderer and
//! getter must return — an error is fine, an abort is the bug.
//!
//! Needs the `live` feature and a libduckdb (`DUCKDB_LIB_DIR`).
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use quack_rs::testing::InMemoryDb;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;

/// Keeps the `DuckDB` the dispatch table was populated from alive.
struct Live(#[allow(dead_code)] InMemoryDb);
// SAFETY: never used after construction; it only has to outlive the run.
unsafe impl Send for Live {}
// SAFETY: as above.
unsafe impl Sync for Live {}

fn live() {
    static DB: OnceLock<Live> = OnceLock::new();
    DB.get_or_init(|| Live(InMemoryDb::open().expect("populate the C API dispatch table")));
}

fn word(bytes: &[u8]) -> i64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(bytes);
    i64::from_le_bytes(raw)
}

/// One value of a temporal type chosen by `kind`, holding `payload` as is.
fn raw_value(kind: u8, payload: i64, extra: i64) -> (TypeId, Value) {
    use libduckdb_sys as ffi;
    // SAFETY: each constructor takes a plain by-value struct and returns an
    // owned value, whatever the payload; `Value` takes ownership.
    unsafe {
        let (id, raw) = match kind % 10 {
            0 => (TypeId::Date, ffi::duckdb_create_date(ffi::duckdb_date { days: payload as i32 })),
            1 => (TypeId::Time, ffi::duckdb_create_time(ffi::duckdb_time { micros: payload })),
            2 => (
                TypeId::TimeTz,
                ffi::duckdb_create_time_tz_value(ffi::duckdb_time_tz { bits: payload as u64 }),
            ),
            3 => (
                TypeId::TimeNs,
                ffi::duckdb_create_time_ns(ffi::duckdb_time_ns { nanos: payload }),
            ),
            4 => (
                TypeId::Timestamp,
                ffi::duckdb_create_timestamp(ffi::duckdb_timestamp { micros: payload }),
            ),
            5 => (
                TypeId::TimestampTz,
                ffi::duckdb_create_timestamp_tz(ffi::duckdb_timestamp { micros: payload }),
            ),
            6 => (
                TypeId::TimestampS,
                ffi::duckdb_create_timestamp_s(ffi::duckdb_timestamp_s { seconds: payload }),
            ),
            7 => (
                TypeId::TimestampMs,
                ffi::duckdb_create_timestamp_ms(ffi::duckdb_timestamp_ms { millis: payload }),
            ),
            8 => (
                TypeId::TimestampNs,
                ffi::duckdb_create_timestamp_ns(ffi::duckdb_timestamp_ns { nanos: payload }),
            ),
            _ => (
                TypeId::Interval,
                ffi::duckdb_create_interval(ffi::duckdb_interval {
                    months: payload as i32,
                    days: (payload >> 32) as i32,
                    micros: extra,
                }),
            ),
        };
        (id, Value::from_raw(raw))
    }
}

/// Every renderer and getter; the results are irrelevant, returning is not.
fn exercise(v: &Value) {
    let _ = v.as_str();
    let _ = v.display_string();
    let _ = format!("{v:?}");
    let _ = v.type_id();
    let _ = v.is_sql_null();
    let _ = (v.as_bool(), v.as_i8(), v.as_i16(), v.as_i32(), v.as_i64(), v.as_i128());
    let _ = (v.as_u8(), v.as_u16(), v.as_u32(), v.as_u64(), v.as_u128());
    let _ = (v.as_f32(), v.as_f64(), v.as_date(), v.as_time(), v.as_time_tz());
    let _ = (v.as_time_ns(), v.as_timestamp(), v.as_timestamp_tz());
    let _ = (v.as_timestamp_s(), v.as_timestamp_ms(), v.as_timestamp_ns());
    let _ = (v.as_interval(), v.as_uuid(), v.as_decimal(), v.as_enum_index());
}

fuzz_target!(|data: &[u8]| {
    live();
    // Records of 17 bytes: kind, payload, and an extra word for INTERVAL.
    let mut values: Vec<(TypeId, Value)> = data
        .chunks_exact(17)
        .take(8)
        .map(|r| raw_value(r[0], word(&r[1..9]), word(&r[9..17])))
        .collect();
    for (_, v) in &values {
        exercise(v);
    }
    // Nest runs of one type in a list, and that list in another.
    if let Some((id, _)) = values.first() {
        let id = *id;
        let same: Vec<Value> = values
            .drain(..)
            .filter(|(t, _)| *t == id)
            .map(|(_, v)| v)
            .collect();
        let element = LogicalType::new(id);
        if let Ok(list) = Value::list_value(&element, &same) {
            exercise(&list);
            let list_type = LogicalType::list_from_logical(&element);
            if let Ok(outer) = Value::list_value(&list_type, &[list]) {
                exercise(&outer);
            }
        }
    }
});
