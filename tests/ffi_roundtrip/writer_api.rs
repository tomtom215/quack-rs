// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `VectorWriter` and `VectorReader` methods no other end-to-end test runs:
//! `read_timestamp_tz` / `write_timestamp_tz` (the echo function that names
//! them is registered in `ffi_roundtrip.rs` but never called), `write_str`
//! (the alias for `write_varchar`), and the `Ok` path of `try_write_varchar` /
//! `try_write_blob`.
//!
//! The `Err` path of the two `try_` writers needs a value over 4 GiB; it is
//! covered by `check_string_len`'s unit test in `src/vector/string.rs`.

use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::vector_support::{count, edge_rows, text_rows};
use super::Fixture;

fn register(fx: &Fixture, name: &str, param: TypeId, ret: TypeId, callback: ScalarFn) {
    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(param)
            .returns(ret)
            .function(callback)
            .register(fx.con())
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

// ─── TIMESTAMPTZ ─────────────────────────────────────────────────────────────

quack_rs::scalar_callback!(tstz_echo, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            unsafe { writer.write_timestamp_tz(row, reader.read_timestamp_tz(row)) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
});

// TIMESTAMPTZ -> BIGINT: the microseconds `read_timestamp_tz` sees.
quack_rs::scalar_callback!(tstz_to_micros, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            unsafe { writer.write_i64(row, reader.read_timestamp_tz(row)) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
});

// BIGINT -> TIMESTAMPTZ: the instant `write_timestamp_tz` stores.
quack_rs::scalar_callback!(micros_to_tstz, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            unsafe { writer.write_timestamp_tz(row, reader.read_i64(row)) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
});

/// Microseconds since the epoch for row `x`: before it for small `x`, after it
/// for large, with a non-zero microsecond part.
const MICROS: &str = "(x * 86400000017 - 90000000000000)";

#[test]
fn timestamp_tz_reads_and_writes_the_instant() {
    let fx = Fixture::open();
    register(
        &fx,
        "tstz_echo",
        TypeId::TimestampTz,
        TypeId::TimestampTz,
        tstz_echo,
    );
    register(
        &fx,
        "tstz_to_micros",
        TypeId::TimestampTz,
        TypeId::BigInt,
        tstz_to_micros,
    );
    register(
        &fx,
        "micros_to_tstz",
        TypeId::BigInt,
        TypeId::TimestampTz,
        micros_to_tstz,
    );

    // Fixed points first: a known instant, and one microsecond before the epoch.
    for (literal, micros) in [
        (
            "TIMESTAMPTZ '2026-08-18 12:34:56.789012+00'",
            1_787_056_496_789_012_i64,
        ),
        ("TIMESTAMPTZ '1969-12-31 23:59:59.999999+00'", -1),
        ("TIMESTAMPTZ '1970-01-01 02:00:00+02'", 0),
    ] {
        assert_eq!(
            count(&fx, &format!("SELECT tstz_to_micros({literal})")),
            micros,
            "{literal}: read_timestamp_tz"
        );
        assert_eq!(
            count(&fx, &format!("SELECT epoch_us(micros_to_tstz({micros}))")),
            micros,
            "{micros}: write_timestamp_tz"
        );
        assert_eq!(
            count(&fx, &format!("SELECT epoch_us(tstz_echo({literal}))")),
            micros,
            "{literal}: echo"
        );
    }

    for n in edge_rows() {
        // DuckDB's own comparison, over a column with NULLs in it.
        let table = format!(
            "(SELECT x, CASE WHEN x % 5 = 0 THEN NULL ELSE make_timestamptz({MICROS}) END AS ts \
             FROM range({n}) t(x))"
        );
        assert_eq!(
            count(
                &fx,
                &format!("SELECT count(*) FROM {table} WHERE tstz_echo(ts) IS DISTINCT FROM ts")
            ),
            0,
            "n={n}: echo"
        );
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM {table} WHERE tstz_to_micros(ts) IS DISTINCT FROM epoch_us(ts)"
                )
            ),
            0,
            "n={n}: read_timestamp_tz"
        );
        assert_eq!(
            count(
                &fx,
                &format!(
                    "SELECT count(*) FROM {table} WHERE \
                     micros_to_tstz({MICROS}) IS DISTINCT FROM make_timestamptz({MICROS})"
                )
            ),
            0,
            "n={n}: write_timestamp_tz"
        );
        // The NULL rows are the model's, so the comparison above was not vacuous.
        assert_eq!(
            count(
                &fx,
                &format!("SELECT count(*) FROM {table} WHERE tstz_echo(ts) IS NULL")
            ),
            (0..n).filter(|x| x % 5 == 0).count() as i64,
            "n={n}: NULLs"
        );
    }
}

// ─── write_str and the try_ writers ──────────────────────────────────────────

/// The text row `x` is expected to hold: empty, one byte, exactly 12 bytes
/// (the most `DuckDB` stores inline), 13 (the first that is not), multi-byte
/// UTF-8, and a number. `None` (NULL) for `x % 7 == 3`.
fn text_for(x: i64) -> Option<String> {
    if x % 7 == 3 {
        return None;
    }
    Some(match x % 6 {
        0 => String::new(),
        1 => "a".to_owned(),
        2 => "123456789012".to_owned(),
        3 => "1234567890123".to_owned(),
        4 => "héllo wörld — ünïcode €".to_owned(),
        _ => format!("row {x}"),
    })
}

/// The bytes row `x` is expected to hold as a BLOB, as lowercase hex.
fn blob_hex_for(x: i64) -> Option<String> {
    if x % 7 == 3 {
        return None;
    }
    let bytes: Vec<u8> = match x % 6 {
        0 => vec![],
        1 => vec![0x00],
        2 => vec![0xFF; 12],
        3 => (0..13).collect(),
        4 => vec![0x80, 0xFE, 0x00, 0xC3, 0x28],
        _ => (0..(x % 40) as u8).map(|b| b.wrapping_mul(7)).collect(),
    };
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

quack_rs::scalar_callback!(text_via_write_str, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        match text_for(unsafe { reader.read_i64(row) }) {
            Some(text) => unsafe { writer.write_str(row, &text) },
            None => unsafe { writer.set_null(row) },
        }
    }
});

quack_rs::scalar_callback!(text_via_try_write_varchar, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        match text_for(unsafe { reader.read_i64(row) }) {
            Some(text) => unsafe { writer.try_write_varchar(row, &text) }.expect("short string"),
            None => unsafe { writer.set_null(row) },
        }
    }
});

quack_rs::scalar_callback!(blob_via_try_write_blob, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let x = unsafe { reader.read_i64(row) };
        match blob_hex_for(x) {
            Some(hex) => {
                let bytes: Vec<u8> = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
                    .collect();
                unsafe { writer.try_write_blob(row, &bytes) }.expect("short blob");
            }
            None => unsafe { writer.set_null(row) },
        }
    }
});

#[test]
fn write_str_and_the_try_writers_store_strings_and_blobs() {
    let fx = Fixture::open();
    register(
        &fx,
        "text_via_write_str",
        TypeId::BigInt,
        TypeId::Varchar,
        text_via_write_str,
    );
    register(
        &fx,
        "text_via_try_write_varchar",
        TypeId::BigInt,
        TypeId::Varchar,
        text_via_try_write_varchar,
    );
    register(
        &fx,
        "blob_via_try_write_blob",
        TypeId::BigInt,
        TypeId::Blob,
        blob_via_try_write_blob,
    );
    for n in edge_rows() {
        for function in ["text_via_write_str", "text_via_try_write_varchar"] {
            let rows = text_rows(
                &fx,
                &format!("SELECT {function}(x) FROM range({n}) t(x) ORDER BY x"),
            );
            assert_eq!(rows.len(), n);
            for (x, row) in rows.iter().enumerate() {
                assert_eq!(row[0], text_for(x as i64), "{function} n={n} x={x}");
            }
            // The engine's own view of the bytes: lengths in bytes, not chars.
            let byte_total = count(
                &fx,
                &format!(
                    "SELECT coalesce(sum(strlen({function}(x))), 0)::BIGINT FROM range({n}) t(x)"
                ),
            );
            let want: usize = (0..n as i64).filter_map(text_for).map(|t| t.len()).sum();
            assert_eq!(byte_total, want as i64, "{function} n={n}: bytes");
        }
        let rows = text_rows(
            &fx,
            &format!("SELECT hex(blob_via_try_write_blob(x)) FROM range({n}) t(x) ORDER BY x"),
        );
        assert_eq!(rows.len(), n);
        for (x, row) in rows.iter().enumerate() {
            // `hex` is upper case.
            assert_eq!(
                row[0].as_deref().map(str::to_lowercase),
                blob_hex_for(x as i64),
                "blob n={n} x={x}"
            );
        }
        let byte_total = count(
            &fx,
            &format!(
                "SELECT coalesce(sum(octet_length(blob_via_try_write_blob(x))), 0)::BIGINT \
                 FROM range({n}) t(x)"
            ),
        );
        let want: usize = (0..n as i64)
            .filter_map(blob_hex_for)
            .map(|h| h.len() / 2)
            .sum();
        assert_eq!(byte_total, want as i64, "blob n={n}: bytes");
    }
}
