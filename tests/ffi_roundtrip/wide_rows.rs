// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Every fixed-width and string type through a real scalar function over
//! thousands of *distinct* rows, NULLs included.
//!
//! `VectorReader::read_*` and `VectorWriter::write_*` scale the row index by
//! the type's width. The echo tests in `tests/ffi_roundtrip.rs` read a single
//! constant row for several types (`echo_u128`, `echo_i128` and the
//! timestamp family among them), and a stride that is wrong for every row but
//! the first (`idx / 16`, `idx + 15`, `idx * 17`) reads row 0 correctly. Here
//! each type gets 5,000 rows: two and a half vectors, so the engine hands the
//! callback several chunks and every word of the validity mask is used, and
//! every row's value is different, so a read from the wrong row shows.
//!
//! The comparison is between two `VARCHAR` casts made by `DuckDB` itself (the
//! input column and the echoed column), not between two reads through the
//! reader under test.

use quack_rs::types::TypeId;

use super::vector_support::text_rows;
use super::{register_echo, Fixture};

const ROWS: usize = 5_000;

struct Case {
    name: &'static str,
    id: TypeId,
    sql_type: &'static str,
    /// An expression of the row number `i` (0..5000) giving a different value
    /// for each row, inside the type's range.
    value: &'static str,
    callback: quack_rs::scalar::ScalarFn,
}

fn cases() -> Vec<Case> {
    macro_rules! case {
        ($name:literal, $id:expr, $ty:literal, $value:literal, $cb:path) => {
            Case {
                name: $name,
                id: $id,
                sql_type: $ty,
                value: $value,
                callback: $cb,
            }
        };
    }
    vec![
        case!("i8", TypeId::TinyInt, "TINYINT", "(i % 251) - 125", super::echo_i8),
        case!("i16", TypeId::SmallInt, "SMALLINT", "((i * 37) % 65000) - 32500", super::echo_i16),
        case!("i32", TypeId::Integer, "INTEGER", "i * 400009 - 1000000000", super::echo_i32),
        case!("i64", TypeId::BigInt, "BIGINT", "i * 1844674407370955 - 4611686018427387904", super::echo_i64),
        case!("u8", TypeId::UTinyInt, "UTINYINT", "i % 256", super::echo_u8),
        case!("u16", TypeId::USmallInt, "USMALLINT", "(i * 13) % 65536", super::echo_u16),
        case!("u32", TypeId::UInteger, "UINTEGER", "i * 858993", super::echo_u32),
        case!("u64", TypeId::UBigInt, "UBIGINT", "i::UBIGINT * 3689348814741910::UBIGINT", super::echo_u64),
        case!(
            "i128",
            TypeId::HugeInt,
            "HUGEINT",
            "(i - 2500) * 1000000007000000007000000007::HUGEINT",
            super::echo_i128
        ),
        case!(
            "u128",
            TypeId::UHugeInt,
            "UHUGEINT",
            "i::UHUGEINT * 68056473384187692692674921486353::UHUGEINT + i::UHUGEINT",
            super::echo_u128
        ),
        case!("f32", TypeId::Float, "FLOAT", "i * 0.37 - 900", super::echo_f32),
        case!("f64", TypeId::Double, "DOUBLE", "i * 0.123456789 - 300", super::echo_f64),
        case!("bool", TypeId::Boolean, "BOOLEAN", "i % 3 = 0", super::echo_bool),
        case!(
            "date",
            TypeId::Date,
            "DATE",
            "DATE '1970-01-01' + (i * 997 - 2000000)::INTEGER",
            super::echo_date
        ),
        case!(
            "time",
            TypeId::Time,
            "TIME",
            "TIME '00:00:00' + to_microseconds((i * 17000011) % 86400000000)",
            super::echo_time
        ),
        case!(
            "timestamp",
            TypeId::Timestamp,
            "TIMESTAMP",
            "TIMESTAMP '1970-01-01' + to_microseconds(i * 123456789012)",
            super::echo_timestamp
        ),
        case!(
            "timestamp_s",
            TypeId::TimestampS,
            "TIMESTAMP_S",
            "(TIMESTAMP '2000-01-01 00:00:00' + to_seconds(i * 86413))::TIMESTAMP_S",
            super::echo_timestamp_s
        ),
        case!(
            "timestamp_ms",
            TypeId::TimestampMs,
            "TIMESTAMP_MS",
            "(TIMESTAMP '2000-01-01 00:00:00' + to_milliseconds(i * 86413007))::TIMESTAMP_MS",
            super::echo_timestamp_ms
        ),
        case!(
            "timestamp_ns",
            TypeId::TimestampNs,
            "TIMESTAMP_NS",
            "(TIMESTAMP '2000-01-01 00:00:00' + to_microseconds(i * 86413007))::TIMESTAMP_NS",
            super::echo_timestamp_ns
        ),
        case!(
            "timestamp_tz",
            TypeId::TimestampTz,
            "TIMESTAMPTZ",
            "(TIMESTAMP '1970-01-01' + to_microseconds(i * 123456789012))::TIMESTAMPTZ",
            super::echo_timestamp_tz
        ),
        case!(
            "time_tz",
            TypeId::TimeTz,
            "TIMETZ",
            "('00:00:00'::TIME + to_microseconds((i * 17000011) % 86400000000))::TIMETZ",
            super::echo_time_tz
        ),
        case!(
            "uuid",
            TypeId::Uuid,
            "UUID",
            "printf('%08x-%04x-4%03x-8%03x-%012x', i, i % 65536, i % 4096, (i * 7) % 4096, i * 1099511)::UUID",
            super::echo_uuid
        ),
        case!(
            "interval",
            TypeId::Interval,
            "INTERVAL",
            "to_months(i % 1000) + to_days(i % 31) + to_microseconds(i * 1001)",
            super::echo_interval
        ),
        case!(
            "varchar",
            TypeId::Varchar,
            "VARCHAR",
            "repeat(chr((97 + (i % 26))::INTEGER), i % 40) || i::VARCHAR",
            super::echo_str
        ),
        case!(
            "blob",
            TypeId::Blob,
            "BLOB",
            "(repeat(chr((97 + (i % 26))::INTEGER), i % 40) || i::VARCHAR)::BLOB",
            super::echo_blob
        ),
    ]
}

#[test]
fn every_type_echoes_thousands_of_distinct_rows_in_place() {
    let fx = Fixture::open();
    for case in cases() {
        let function = format!("wide_{}", case.name);
        register_echo(fx.con(), &function, case.id, case.id, case.callback);
        let table = format!("wide_t_{}", case.name);
        fx.query(&format!(
            "CREATE TABLE {table} AS SELECT i, CASE WHEN i % 7 = 3 THEN NULL \
             ELSE ({})::{} END AS v FROM range({ROWS}) r(i)",
            case.value, case.sql_type
        ));
        let rows = text_rows(
            &fx,
            &format!(
                "SELECT i::VARCHAR, {function}(v)::VARCHAR, v::VARCHAR FROM {table} ORDER BY i"
            ),
        );
        assert_eq!(rows.len(), ROWS, "{}", case.name);
        let mut distinct = std::collections::HashSet::new();
        let mut nulls = 0;
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(
                row[1], row[2],
                "{}: row {i} was changed by the echo (echoed {:?}, input {:?})",
                case.name, row[1], row[2]
            );
            match &row[2] {
                Some(text) => {
                    distinct.insert(text.clone());
                }
                None => nulls += 1,
            }
        }
        assert_eq!(
            nulls,
            (0..ROWS).filter(|i| i % 7 == 3).count(),
            "{}",
            case.name
        );
        // The comparison above proves nothing if the rows are all alike.
        let wanted = if matches!(case.name, "bool") {
            2
        } else if case.name == "u8" || case.name == "i8" {
            200
        } else {
            1_000
        };
        assert!(
            distinct.len() >= wanted,
            "{}: only {} distinct values among {ROWS} rows",
            case.name,
            distinct.len()
        );
    }
}
