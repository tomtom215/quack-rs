// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

// These tests are deliberately exhaustive: each one walks every width, every
// boundary value, or every callback kind, which makes them long and full of
// deliberate casts at type edges. `src/` is held to the full pedantic bar (CI
// lints it with `-D warnings`); this file opts out of the style lints that
// fight that shape, and nothing else.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::items_after_statements,
    clippy::redundant_closure_for_method_calls,
    clippy::format_collect,
    clippy::manual_assert,
    clippy::err_expect,
    clippy::case_sensitive_file_extension_comparisons
)]

//! End-to-end FFI round-trips against a real `DuckDB`.
//!
//! Every test here registers a genuine `DuckDB` function built with quack-rs,
//! runs SQL against it, and checks the answer — so it exercises the whole path:
//! builder → `duckdb_register_*` → `DuckDB`'s planner and executor → the
//! `extern "C"` callback → [`VectorReader`] / [`VectorWriter`] → back out through
//! a `duckdb_result`.
//!
//! This is the coverage that matters most for this crate: the vector accessors
//! do raw pointer arithmetic against layouts that only `DuckDB` defines, so a
//! wrong offset or a wrong physical type is invisible to a mock and obvious
//! here.
//!
//! # Why this works in `cargo test`
//!
//! In `loadable-extension` mode every C API call goes through a function-pointer
//! dispatch table that `DuckDB` normally fills at extension-load time.
//! [`InMemoryDb::open`] populates it from the linked `DuckDB` via
//! `CreateAPIv1()`, so once it has been called the full C API — including
//! function registration and vector access — behaves exactly as it does inside a
//! loaded extension.
//!
//! Requires `--features bundled-test` (or `bundled-test-prebuilt`).

#![cfg(feature = "_duckdb-testing")]

use libduckdb_sys::{duckdb_connection, DuckDBSuccess};
use quack_rs::data_chunk::DataChunk;
use quack_rs::datetime;
use quack_rs::query::{query, QueryResult};
use quack_rs::scalar::{ScalarFn, ScalarFunctionBuilder};
use quack_rs::testing::InMemoryDb;
use quack_rs::types::{LogicalType, NullHandling, TypeId};
use quack_rs::vector::{VectorReader, VectorWriter};

/// A live database plus a connection, torn down in the right order on drop.
struct Fixture {
    db: libduckdb_sys::duckdb_database,
    con: duckdb_connection,
    _dispatch: InMemoryDb,
}

impl Fixture {
    fn open() -> Self {
        // Populates the loadable-extension dispatch table from the linked DuckDB.
        let dispatch = InMemoryDb::open().expect("initialise the DuckDB C API dispatch table");
        let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
        let mut con: duckdb_connection = std::ptr::null_mut();
        // SAFETY: standard open/connect against a fresh in-memory database.
        unsafe {
            assert_eq!(
                libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
                DuckDBSuccess,
                "duckdb_open"
            );
            assert_eq!(
                libduckdb_sys::duckdb_connect(db, &raw mut con),
                DuckDBSuccess,
                "duckdb_connect"
            );
        }
        Self {
            db,
            con,
            _dispatch: dispatch,
        }
    }

    const fn con(&self) -> duckdb_connection {
        self.con
    }

    /// The database handle — replacement scans register against this, not a
    /// connection.
    const fn db(&self) -> libduckdb_sys::duckdb_database {
        self.db
    }

    fn query(&self, sql: &str) -> QueryResult {
        // SAFETY: `self.con` is open for this fixture's lifetime.
        unsafe { query(self.con, sql) }.unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// Runs `sql` and returns the single value in row 0, column 0, read by
    /// `read`. `None` when that value is SQL NULL.
    fn scalar<T>(&self, sql: &str, read: impl Fn(&VectorReader, usize) -> T) -> Option<T> {
        let mut result = self.query(sql);
        let chunk = result.next_chunk().expect("at least one chunk");
        assert_eq!(chunk.size(), 1, "{sql} must return exactly one row");
        // SAFETY: the chunk has one row and at least one column.
        let reader = unsafe { chunk.reader(0) };
        // SAFETY: row 0 exists.
        if !unsafe { reader.is_valid(0) } {
            return None;
        }
        Some(read(&reader, 0))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: both handles were created in `open` and are released once.
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut self.con);
            libduckdb_sys::duckdb_close(&raw mut self.db);
        }
    }
}

// ─── Scalar functions: one per physical layout ───────────────────────────────

/// Declares a scalar function that reads one value and writes one value,
/// registering it under `$sql_name`.
macro_rules! echo_fn {
    ($fn_name:ident, $read:ident, $write:ident) => {
        quack_rs::scalar_callback!($fn_name, |_info, input, output| {
            let chunk = unsafe { DataChunk::from_raw(input) };
            let reader = unsafe { chunk.reader(0) };
            let mut writer = unsafe { VectorWriter::from_vector(output) };
            for row in 0..chunk.size() {
                if unsafe { reader.is_valid(row) } {
                    let value = unsafe { reader.$read(row) };
                    unsafe { writer.$write(row, value) };
                } else {
                    unsafe { writer.set_null(row) };
                }
            }
        });
    };
}

echo_fn!(echo_i8, read_i8, write_i8);
echo_fn!(echo_i16, read_i16, write_i16);
echo_fn!(echo_i32, read_i32, write_i32);
echo_fn!(echo_i64, read_i64, write_i64);
echo_fn!(echo_u8, read_u8, write_u8);
echo_fn!(echo_u16, read_u16, write_u16);
echo_fn!(echo_u32, read_u32, write_u32);
echo_fn!(echo_u64, read_u64, write_u64);
echo_fn!(echo_i128, read_i128, write_i128);
echo_fn!(echo_u128, read_u128, write_u128);
echo_fn!(echo_f32, read_f32, write_f32);
echo_fn!(echo_f64, read_f64, write_f64);
echo_fn!(echo_bool, read_bool, write_bool);
echo_fn!(echo_str, read_str, write_varchar);
echo_fn!(echo_blob, read_blob, write_blob);
echo_fn!(echo_date, read_date, write_date);
echo_fn!(echo_time, read_time, write_time);
echo_fn!(echo_timestamp, read_timestamp, write_timestamp);
echo_fn!(echo_timestamp_tz, read_timestamp_tz, write_timestamp_tz);
echo_fn!(echo_timestamp_s, read_timestamp_s, write_timestamp_s);
echo_fn!(echo_timestamp_ms, read_timestamp_ms, write_timestamp_ms);
echo_fn!(echo_timestamp_ns, read_timestamp_ns, write_timestamp_ns);
echo_fn!(echo_time_tz, read_time_tz, write_time_tz);
echo_fn!(echo_uuid, read_uuid, write_uuid);
echo_fn!(echo_interval, read_interval, write_interval);

/// Registers `name(param) -> ret` backed by `callback`.
fn register_echo(
    con: duckdb_connection,
    name: &str,
    param: TypeId,
    ret: TypeId,
    callback: ScalarFn,
) {
    // SAFETY: `con` is open, and the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new(name)
            .expect("valid function name")
            .param(param)
            .returns(ret)
            .function(callback)
            .register(con)
            .unwrap_or_else(|e| panic!("register {name}: {e}"));
    }
}

#[test]
fn every_integer_width_round_trips_through_real_vectors() {
    let fx = Fixture::open();

    register_echo(
        fx.con(),
        "echo_i8",
        TypeId::TinyInt,
        TypeId::TinyInt,
        echo_i8,
    );
    register_echo(
        fx.con(),
        "echo_i16",
        TypeId::SmallInt,
        TypeId::SmallInt,
        echo_i16,
    );
    register_echo(
        fx.con(),
        "echo_i32",
        TypeId::Integer,
        TypeId::Integer,
        echo_i32,
    );
    register_echo(
        fx.con(),
        "echo_i64",
        TypeId::BigInt,
        TypeId::BigInt,
        echo_i64,
    );
    register_echo(
        fx.con(),
        "echo_u8",
        TypeId::UTinyInt,
        TypeId::UTinyInt,
        echo_u8,
    );
    register_echo(
        fx.con(),
        "echo_u16",
        TypeId::USmallInt,
        TypeId::USmallInt,
        echo_u16,
    );
    register_echo(
        fx.con(),
        "echo_u32",
        TypeId::UInteger,
        TypeId::UInteger,
        echo_u32,
    );
    register_echo(
        fx.con(),
        "echo_u64",
        TypeId::UBigInt,
        TypeId::UBigInt,
        echo_u64,
    );

    // Extremes catch sign-extension and width mistakes that mid-range values hide.
    assert_eq!(
        fx.scalar("SELECT echo_i8((-128)::TINYINT)", |r, i| unsafe {
            r.read_i8(i)
        }),
        Some(i8::MIN)
    );
    assert_eq!(
        fx.scalar("SELECT echo_i8(127::TINYINT)", |r, i| unsafe {
            r.read_i8(i)
        }),
        Some(i8::MAX)
    );
    assert_eq!(
        fx.scalar("SELECT echo_i16((-32768)::SMALLINT)", |r, i| unsafe {
            r.read_i16(i)
        }),
        Some(i16::MIN)
    );
    assert_eq!(
        fx.scalar("SELECT echo_i32((-2147483648)::INTEGER)", |r, i| unsafe {
            r.read_i32(i)
        }),
        Some(i32::MIN)
    );
    assert_eq!(
        fx.scalar(
            "SELECT echo_i64((-9223372036854775808)::BIGINT)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(i64::MIN)
    );
    assert_eq!(
        fx.scalar("SELECT echo_u8(255::UTINYINT)", |r, i| unsafe {
            r.read_u8(i)
        }),
        Some(u8::MAX)
    );
    assert_eq!(
        fx.scalar("SELECT echo_u16(65535::USMALLINT)", |r, i| unsafe {
            r.read_u16(i)
        }),
        Some(u16::MAX)
    );
    assert_eq!(
        fx.scalar("SELECT echo_u32(4294967295::UINTEGER)", |r, i| unsafe {
            r.read_u32(i)
        }),
        Some(u32::MAX)
    );
    assert_eq!(
        fx.scalar(
            "SELECT echo_u64(18446744073709551615::UBIGINT)",
            |r, i| unsafe { r.read_u64(i) }
        ),
        Some(u64::MAX)
    );
}

#[test]
fn wide_integers_round_trip_at_their_extremes() {
    let fx = Fixture::open();
    register_echo(
        fx.con(),
        "echo_i128",
        TypeId::HugeInt,
        TypeId::HugeInt,
        echo_i128,
    );
    register_echo(
        fx.con(),
        "echo_u128",
        TypeId::UHugeInt,
        TypeId::UHugeInt,
        echo_u128,
    );

    // HUGEINT is stored as { lower: u64, upper: i64 }; UHUGEINT as two u64s.
    // Getting the halves or their signedness wrong shows up at the extremes.
    let hugeint_max = "170141183460469231731687303715884105727";
    let hugeint_min = "-170141183460469231731687303715884105728";
    let uhugeint_max = "340282366920938463463374607431768211455";

    assert_eq!(
        fx.scalar(
            &format!("SELECT echo_i128({hugeint_max}::HUGEINT)"),
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some(i128::MAX)
    );
    assert_eq!(
        fx.scalar(
            // Parenthesised: `-N::HUGEINT` parses as `-(N::HUGEINT)`, and the
            // positive magnitude of i128::MIN does not fit in HUGEINT.
            &format!("SELECT echo_i128(({hugeint_min})::HUGEINT)"),
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some(i128::MIN)
    );
    assert_eq!(
        fx.scalar("SELECT echo_i128((-1)::HUGEINT)", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(-1)
    );
    assert_eq!(
        fx.scalar(
            &format!("SELECT echo_u128({uhugeint_max}::UHUGEINT)"),
            |r, i| unsafe { r.read_u128(i) }
        ),
        Some(u128::MAX)
    );
    assert_eq!(
        fx.scalar(
            "SELECT echo_u128(18446744073709551616::UHUGEINT)",
            |r, i| unsafe { r.read_u128(i) }
        ),
        Some(1_u128 << 64)
    );
}

#[test]
fn floats_and_booleans_round_trip() {
    let fx = Fixture::open();
    register_echo(fx.con(), "echo_f32", TypeId::Float, TypeId::Float, echo_f32);
    register_echo(
        fx.con(),
        "echo_f64",
        TypeId::Double,
        TypeId::Double,
        echo_f64,
    );
    register_echo(
        fx.con(),
        "echo_bool",
        TypeId::Boolean,
        TypeId::Boolean,
        echo_bool,
    );

    let f32_value = fx
        .scalar("SELECT echo_f32(1.5::FLOAT)", |r, i| unsafe {
            r.read_f32(i)
        })
        .expect("not null");
    assert!((f32_value - 1.5).abs() < f32::EPSILON);

    let f64_value = fx
        .scalar("SELECT echo_f64(1e308::DOUBLE)", |r, i| unsafe {
            r.read_f64(i)
        })
        .expect("not null");
    assert!((f64_value - 1e308).abs() / 1e308 < 1e-12);

    assert!(fx
        .scalar("SELECT echo_f64('nan'::DOUBLE)", |r, i| unsafe {
            r.read_f64(i)
        })
        .expect("not null")
        .is_nan());

    assert_eq!(
        fx.scalar("SELECT echo_bool(true)", |r, i| unsafe { r.read_bool(i) }),
        Some(true)
    );
    assert_eq!(
        fx.scalar("SELECT echo_bool(false)", |r, i| unsafe { r.read_bool(i) }),
        Some(false)
    );
}

#[test]
fn strings_round_trip_across_the_inline_boundary() {
    let fx = Fixture::open();
    register_echo(
        fx.con(),
        "echo_str",
        TypeId::Varchar,
        TypeId::Varchar,
        echo_str,
    );

    // `duckdb_string_t` stores <= 12 bytes inline and longer values behind a
    // pointer. Both sides of that boundary must work, and multi-byte UTF-8 must
    // survive intact.
    for value in [
        "",
        "a",
        "abcdefghijkl",  // exactly 12 bytes: the last inline length
        "abcdefghijklm", // 13 bytes: the first pointer-format length
        "the quick brown fox jumps over the lazy dog",
        "héllo wörld ☃", // multi-byte
        "🦆🦆🦆🦆",      // 16 bytes of emoji
    ] {
        let escaped = value.replace('\'', "''");
        let got = fx.scalar(&format!("SELECT echo_str('{escaped}')"), |r, i| unsafe {
            r.read_str(i).to_owned()
        });
        assert_eq!(got.as_deref(), Some(value), "round trip for {value:?}");
    }
}

#[test]
fn blobs_preserve_arbitrary_bytes() {
    let fx = Fixture::open();
    register_echo(fx.con(), "echo_blob", TypeId::Blob, TypeId::Blob, echo_blob);

    // Bytes that are not valid UTF-8, including an embedded NUL, across both the
    // inline and pointer representations.
    let short = fx.scalar(r"SELECT echo_blob('\x00\xFF\x80'::BLOB)", |r, i| unsafe {
        r.read_blob(i).to_vec()
    });
    assert_eq!(short.as_deref(), Some(&[0x00, 0xFF, 0x80][..]));

    let long_hex: String = (0..40).map(|b: u8| format!(r"\x{b:02X}")).collect();
    let long = fx.scalar(
        &format!("SELECT echo_blob('{long_hex}'::BLOB)"),
        |r, i| unsafe { r.read_blob(i).to_vec() },
    );
    assert_eq!(long.as_deref(), Some(&(0..40).collect::<Vec<u8>>()[..]));
}

#[test]
fn temporal_types_round_trip_and_agree_with_duckdb() {
    let fx = Fixture::open();
    register_echo(fx.con(), "echo_date", TypeId::Date, TypeId::Date, echo_date);
    register_echo(fx.con(), "echo_time", TypeId::Time, TypeId::Time, echo_time);
    register_echo(
        fx.con(),
        "echo_ts",
        TypeId::Timestamp,
        TypeId::Timestamp,
        echo_timestamp,
    );
    register_echo(
        fx.con(),
        "echo_tstz",
        TypeId::TimestampTz,
        TypeId::TimestampTz,
        echo_timestamp_tz,
    );
    register_echo(
        fx.con(),
        "echo_ts_s",
        TypeId::TimestampS,
        TypeId::TimestampS,
        echo_timestamp_s,
    );
    register_echo(
        fx.con(),
        "echo_ts_ms",
        TypeId::TimestampMs,
        TypeId::TimestampMs,
        echo_timestamp_ms,
    );
    register_echo(
        fx.con(),
        "echo_ts_ns",
        TypeId::TimestampNs,
        TypeId::TimestampNs,
        echo_timestamp_ns,
    );
    register_echo(
        fx.con(),
        "echo_timetz",
        TypeId::TimeTz,
        TypeId::TimeTz,
        echo_time_tz,
    );

    // The value must survive the round trip *and* mean the same thing to DuckDB.
    assert_eq!(
        fx.scalar(
            "SELECT echo_date(DATE '2026-08-18')::VARCHAR",
            |r, i| unsafe { r.read_str(i).to_owned() }
        )
        .as_deref(),
        Some("2026-08-18")
    );

    // DATE is days since the epoch; cross-check the raw integer too.
    let days = fx
        .scalar("SELECT echo_date(DATE '2026-08-18')", |r, i| unsafe {
            r.read_date(i)
        })
        .expect("not null");
    // SAFETY: the dispatch table is live for this fixture.
    let decoded = unsafe { datetime::date_from_days(days) };
    assert_eq!((decoded.year, decoded.month, decoded.day), (2026, 8, 18));

    assert_eq!(
        fx.scalar(
            "SELECT echo_time(TIME '23:59:59.999999')::VARCHAR",
            |r, i| unsafe { r.read_str(i).to_owned() }
        )
        .as_deref(),
        Some("23:59:59.999999")
    );
    assert_eq!(
        fx.scalar(
            "SELECT echo_ts(TIMESTAMP '2026-08-18 12:34:56.789')::VARCHAR",
            |r, i| unsafe { r.read_str(i).to_owned() }
        )
        .as_deref(),
        Some("2026-08-18 12:34:56.789")
    );

    // The sub-second variants each store a different unit in the same i64.
    assert_eq!(
        fx.scalar(
            "SELECT echo_ts_s(TIMESTAMP_S '2026-08-18 12:00:00')",
            |r, i| unsafe { r.read_timestamp_s(i) }
        ),
        fx.scalar(
            "SELECT epoch(TIMESTAMP '2026-08-18 12:00:00')::BIGINT",
            |r, i| unsafe { r.read_i64(i) }
        )
    );
    let millis = fx
        .scalar(
            "SELECT echo_ts_ms(TIMESTAMP_MS '2026-08-18 12:00:00.123')",
            |r, i| unsafe { r.read_timestamp_ms(i) },
        )
        .expect("not null");
    assert_eq!(millis % 1_000, 123);
    let nanos = fx
        .scalar(
            "SELECT echo_ts_ns(TIMESTAMP_NS '2026-08-18 12:00:00.123456789')",
            |r, i| unsafe { r.read_timestamp_ns(i) },
        )
        .expect("not null");
    assert_eq!(nanos % 1_000_000_000, 123_456_789);

    // TIMETZ is a packed 64-bit value, not a plain integer.
    let bits = fx
        .scalar("SELECT echo_timetz(TIMETZ '12:00:00+02')", |r, i| unsafe {
            r.read_time_tz(i)
        })
        .expect("not null");
    // SAFETY: the dispatch table is live for this fixture.
    let decoded = unsafe { datetime::time_tz_from_bits(bits) };
    assert_eq!(decoded.time.hour, 12);
    assert_eq!(decoded.offset_seconds, 2 * 3_600);
}

#[test]
fn uuid_and_interval_round_trip() {
    let fx = Fixture::open();
    register_echo(fx.con(), "echo_uuid", TypeId::Uuid, TypeId::Uuid, echo_uuid);
    register_echo(
        fx.con(),
        "echo_iv",
        TypeId::Interval,
        TypeId::Interval,
        echo_interval,
    );

    assert_eq!(
        fx.scalar(
            "SELECT echo_uuid('11111111-2222-3333-4444-555555555555'::UUID)::VARCHAR",
            |r, i| unsafe { r.read_str(i).to_owned() }
        )
        .as_deref(),
        Some("11111111-2222-3333-4444-555555555555")
    );

    // INTERVAL is { months: i32, days: i32, micros: i64 } — three fields at
    // three offsets, so a single scalar comparison would not catch a mix-up.
    let interval = fx
        .scalar(
            "SELECT echo_iv(INTERVAL '14 months 3 days 250 microseconds')",
            |r, i| unsafe { r.read_interval(i) },
        )
        .expect("not null");
    assert_eq!(interval.months, 14);
    assert_eq!(interval.days, 3);
    assert_eq!(interval.micros, 250);
}

// ─── NULL handling ───────────────────────────────────────────────────────────

quack_rs::scalar_callback!(nullable_double, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // With SpecialNullHandling the callback sees NULL inputs itself.
        if unsafe { reader.is_valid(row) } {
            unsafe { writer.write_i64(row, reader.read_i64(row) * 2) };
        } else {
            unsafe { writer.set_null(row) };
        }
    }
});

// Writes NULL into every row, exercising the batched path.
quack_rs::scalar_callback!(all_null, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    unsafe { writer.set_null_range(0..chunk.size()) };
});

#[test]
fn null_inputs_and_outputs_are_handled() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::try_new("nullable_double")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::SpecialNullHandling)
            .function(nullable_double)
            .register(fx.con())
            .expect("register nullable_double");
        ScalarFunctionBuilder::try_new("all_null")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::SpecialNullHandling)
            .function(all_null)
            .register(fx.con())
            .expect("register all_null");
    }

    assert_eq!(
        fx.scalar("SELECT nullable_double(21::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );
    // With SpecialNullHandling the callback runs on the NULL and must emit NULL.
    assert_eq!(
        fx.scalar("SELECT nullable_double(NULL::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        None
    );

    // `set_null_range` over a whole vector, checked through SQL rather than by
    // reading the bitmap back.
    let mut result =
        fx.query("SELECT count(*) AS total, count(all_null(i)) AS non_null FROM range(5000) t(i)");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: both columns are BIGINT and row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_i64(0), 5000);
        assert_eq!(chunk.reader(1).read_i64(0), 0, "every row must be NULL");
    }
}

// ─── Vector sizes and chunking ───────────────────────────────────────────────

quack_rs::scalar_callback!(row_index_sum, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // Reading and writing at the very last index of a full vector is where
        // an off-by-one in the offset arithmetic shows up.
        unsafe { writer.write_i64(row, reader.read_i64(row) + 1) };
    }
});

#[test]
fn full_multi_chunk_scans_touch_every_row() {
    let fx = Fixture::open();
    register_echo(
        fx.con(),
        "row_index_sum",
        TypeId::BigInt,
        TypeId::BigInt,
        row_index_sum,
    );

    // Deliberately more rows than one vector holds, and not a multiple of it, so
    // the last chunk is partial.
    let rows = quack_rs::vector::vector_size() * 5 + 13;
    let sql = format!("SELECT sum(row_index_sum(i)) FROM range({rows}) t(i)");
    let expected = (0..rows as i64).map(|i| i + 1).sum::<i64>();
    assert_eq!(
        fx.scalar(&sql, |r, i| unsafe { r.read_i128(i) }),
        Some(i128::from(expected)),
        "sum over {rows} rows"
    );
}

// ─── DECIMAL: physical width selection ───────────────────────────────────────

/// Echoes a DECIMAL by reading and writing its unscaled integer.
///
/// The declared width is baked into each registration so the callback knows the
/// physical storage type.
macro_rules! decimal_echo {
    ($fn_name:ident, $width:expr) => {
        quack_rs::scalar_callback!($fn_name, |_info, input, output| {
            let chunk = unsafe { DataChunk::from_raw(input) };
            let reader = unsafe { chunk.reader(0) };
            let mut writer = unsafe { VectorWriter::from_vector(output) };
            for row in 0..chunk.size() {
                if unsafe { reader.is_valid(row) } {
                    let unscaled = unsafe { reader.read_decimal(row, $width) };
                    unsafe { writer.write_decimal(row, $width, unscaled) };
                } else {
                    unsafe { writer.set_null(row) };
                }
            }
        });
    };
}

decimal_echo!(echo_decimal_4, 4);
decimal_echo!(echo_decimal_9, 9);
decimal_echo!(echo_decimal_18, 18);
decimal_echo!(echo_decimal_38, 38);

#[test]
fn decimals_round_trip_at_every_physical_width() {
    let fx = Fixture::open();

    // DuckDB stores DECIMAL in the narrowest integer that fits the width:
    // <=4 -> i16, <=9 -> i32, <=18 -> i64, <=38 -> i128. Each boundary gets its
    // own registration so a wrong threshold reads the wrong number of bytes.
    for (name, width, scale, callback) in [
        ("dec4", 4_u8, 2_u8, echo_decimal_4 as ScalarFn),
        ("dec9", 9, 4, echo_decimal_9),
        ("dec18", 18, 6, echo_decimal_18),
        ("dec38", 38, 10, echo_decimal_38),
    ] {
        let decimal_type = LogicalType::decimal(width, scale);
        // SAFETY: `con` is open; the callback matches the declared signature.
        unsafe {
            ScalarFunctionBuilder::try_new(name)
                .expect("name")
                .param_logical(LogicalType::decimal(width, scale))
                .returns_logical(decimal_type)
                .function(callback)
                .register(fx.con())
                .unwrap_or_else(|e| panic!("register {name}: {e}"));
        }
    }

    for (name, literal, width, scale) in [
        ("dec4", "99.99", 4, 2),
        ("dec4", "-99.99", 4, 2),
        ("dec9", "99999.9999", 9, 4),
        ("dec18", "123456789012.345678", 18, 6),
        ("dec38", "1234567890123456789012345678.9012345678", 38, 10),
    ] {
        let sql = format!("SELECT {name}({literal}::DECIMAL({width},{scale}))::VARCHAR");
        assert_eq!(
            fx.scalar(&sql, |r, i| unsafe { r.read_str(i).to_owned() })
                .as_deref(),
            Some(literal),
            "{sql}"
        );
    }
}

// ─── Panic safety ────────────────────────────────────────────────────────────

quack_rs::scalar_callback!(always_panics, |_info, _input, _output| {
    panic!("deliberate panic from a scalar callback");
});

#[test]
fn a_panicking_callback_becomes_a_sql_error() {
    // `scalar_callback!` wraps the body in `catch_unwind` and reports the panic
    // through `duckdb_scalar_function_set_error`. Without that the unwind would
    // reach the `extern "C"` boundary and abort the process — taking the whole
    // database with it. This test is therefore also a canary for anyone who
    // builds the test profile with `panic = "abort"`.
    let fx = Fixture::open();
    register_echo(
        fx.con(),
        "always_panics",
        TypeId::BigInt,
        TypeId::BigInt,
        always_panics,
    );

    // SAFETY: `fx.con` is open.
    let result = unsafe { query(fx.con(), "SELECT always_panics(1::BIGINT)") };
    let err = result.err().expect("the panic must surface as an error");
    assert!(
        err.as_str().contains("deliberate panic"),
        "the panic message should reach the user: {err}"
    );

    // The connection must remain usable afterwards.
    assert_eq!(
        fx.scalar("SELECT 7::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(7)
    );
}

// ─── Table functions ─────────────────────────────────────────────────────────

#[test]
fn a_typed_table_function_streams_rows() {
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();

    struct State {
        remaining: i64,
    }

    let builder = TableFunctionBuilder::new("count_down")
        .param(TypeId::BigInt)
        .with_state::<State, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: parameter 0 was declared above.
            let n = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
            Ok(State { remaining: n })
        })
        .scan(|state, chunk| {
            // Emit one row per call so the scan loop runs many times.
            if state.remaining <= 0 {
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            let mut writer = unsafe { chunk.writer(0) };
            unsafe { writer.write_i64(0, state.remaining) };
            state.remaining -= 1;
            unsafe { chunk.set_size(1) };
            Ok(())
        })
        .build()
        .expect("build typed table function");

    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register count_down");

    assert_eq!(
        fx.scalar("SELECT sum(n) FROM count_down(100)", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(5050)
    );
    assert_eq!(
        fx.scalar("SELECT count(*) FROM count_down(0)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(0),
        "an empty scan must terminate rather than loop"
    );
}

#[test]
fn a_panicking_table_function_reports_the_panic_message() {
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();

    let builder = TableFunctionBuilder::new("boom_scan")
        .param(TypeId::BigInt)
        .with_state::<(), _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            Ok(())
        })
        .scan(|(), _chunk| {
            panic!("scan closure exploded");
        })
        .build()
        .expect("build");

    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register boom_scan");

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT * FROM boom_scan(1)") }
        .expect_err("the panic must surface as an error");
    // The payload text must survive: "closure panicked" alone does not tell the
    // user which assertion failed.
    assert!(
        err.as_str().contains("scan closure exploded"),
        "panic payload should reach the user: {err}"
    );
}

#[test]
fn a_table_function_bind_error_is_reported() {
    use quack_rs::error::ExtensionError;
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();

    let builder = TableFunctionBuilder::new("bind_fails")
        .param(TypeId::BigInt)
        .with_state::<(), _>(|_bind| Err(ExtensionError::new("n must be positive")))
        .scan(|(), chunk| {
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build");

    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register bind_fails");

    // SAFETY: `con` is open.
    let err =
        unsafe { query(fx.con(), "SELECT * FROM bind_fails(-1)") }.expect_err("bind must fail");
    assert!(err.as_str().contains("n must be positive"), "{err}");
}

// ─── Aggregate functions ─────────────────────────────────────────────────────

#[test]
fn an_aggregate_function_computes_across_chunks() {
    use libduckdb_sys::{duckdb_aggregate_state, duckdb_function_info, idx_t};
    use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};

    let fx = Fixture::open();

    #[derive(Default)]
    struct SumState {
        total: i64,
        seen: u64,
    }
    impl AggregateState for SumState {}

    unsafe extern "C" fn update(
        _info: duckdb_function_info,
        input: libduckdb_sys::duckdb_data_chunk,
        states: *mut duckdb_aggregate_state,
    ) {
        let chunk = unsafe { DataChunk::from_raw(input) };
        let reader = unsafe { chunk.reader(0) };
        for row in 0..chunk.size() {
            let Some(state) = (unsafe { FfiState::<SumState>::with_state_mut(*states.add(row)) })
            else {
                continue;
            };
            if unsafe { reader.is_valid(row) } {
                state.total += unsafe { reader.read_i64(row) };
                state.seen += 1;
            }
        }
    }

    unsafe extern "C" fn combine(
        _info: duckdb_function_info,
        source: *mut duckdb_aggregate_state,
        target: *mut duckdb_aggregate_state,
        count: idx_t,
    ) {
        for i in 0..count as usize {
            let src_total_and_seen = unsafe { FfiState::<SumState>::with_state(*source.add(i)) }
                .map(|s| (s.total, s.seen));
            let Some((total, seen)) = src_total_and_seen else {
                continue;
            };
            if let Some(tgt) = unsafe { FfiState::<SumState>::with_state_mut(*target.add(i)) } {
                // Pitfall L1: every field must be propagated, not just the sum.
                tgt.total += total;
                tgt.seen += seen;
            }
        }
    }

    unsafe extern "C" fn finalize(
        _info: duckdb_function_info,
        source: *mut duckdb_aggregate_state,
        result: libduckdb_sys::duckdb_vector,
        count: idx_t,
        offset: idx_t,
    ) {
        let mut writer = unsafe { VectorWriter::from_vector(result) };
        for i in 0..count as usize {
            let row = offset as usize + i;
            match unsafe { FfiState::<SumState>::with_state(*source.add(i)) } {
                Some(state) if state.seen > 0 => unsafe { writer.write_i64(row, state.total) },
                _ => unsafe { writer.set_null(row) },
            }
        }
    }

    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("my_sum")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<SumState>::size_callback)
            .init(FfiState::<SumState>::init_callback)
            .update(update)
            .combine(combine)
            .finalize(finalize)
            .destructor(FfiState::<SumState>::destroy_callback)
            .register(fx.con())
            .expect("register my_sum");
    }

    // More rows than one vector holds, so DuckDB uses several chunks — and,
    // with enough data, several threads and therefore `combine`.
    let rows = quack_rs::vector::vector_size() * 8;
    let expected: i64 = (0..rows as i64).sum();
    assert_eq!(
        fx.scalar(
            &format!("SELECT my_sum(i) FROM range({rows}) t(i)"),
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(expected)
    );

    // An empty group must produce NULL, not 0.
    assert_eq!(
        fx.scalar(
            "SELECT my_sum(i) FROM (SELECT NULL::BIGINT AS i) t",
            |r, i| unsafe { r.read_i64(i) }
        ),
        None
    );

    // GROUP BY exercises many independent states.
    let mut result = fx.query(
        "SELECT g, my_sum(i) FROM (SELECT i % 4 AS g, i FROM range(1000) t(i)) GROUP BY g ORDER BY g",
    );
    let chunk = result.next_chunk().expect("one chunk");
    assert_eq!(chunk.size(), 4);
    for row in 0..4usize {
        // SAFETY: both columns are BIGINT and `row` is in bounds.
        let (g, sum) = unsafe { (chunk.reader(0).read_i64(row), chunk.reader(1).read_i64(row)) };
        let want: i64 = (0..1000i64).filter(|i| i % 4 == g).sum();
        assert_eq!(sum, want, "group {g}");
    }
}

// ─── Panic guards for the remaining callback kinds ───────────────────────────

/// An aggregate whose `update` panics, wired through `aggregate_update_callback!`.
mod panicking_aggregate {
    use super::{DataChunk, VectorWriter};
    use libduckdb_sys::{duckdb_aggregate_state, duckdb_function_info, idx_t};
    use quack_rs::aggregate::{AggregateState, FfiState};

    #[derive(Default)]
    pub struct Empty;
    impl AggregateState for Empty {}

    quack_rs::aggregate_update_callback!(update, |_info, input, _states| {
        let chunk = unsafe { DataChunk::from_raw(input) };
        assert!(chunk.size() == usize::MAX, "update deliberately exploded");
    });

    quack_rs::aggregate_combine_callback!(combine, |_info, _source, _target, _count| {});

    quack_rs::aggregate_finalize_callback!(finalize, |_info, _source, result, count, offset| {
        let mut writer = unsafe { VectorWriter::from_vector(result) };
        for i in 0..count as usize {
            unsafe { writer.set_null(offset as usize + i) };
        }
    });

    pub fn state_size() -> unsafe extern "C" fn(duckdb_function_info) -> idx_t {
        FfiState::<Empty>::size_callback
    }

    pub fn state_init() -> unsafe extern "C" fn(duckdb_function_info, duckdb_aggregate_state) {
        FfiState::<Empty>::init_callback
    }

    pub fn destroy() -> unsafe extern "C" fn(*mut duckdb_aggregate_state, idx_t) {
        FfiState::<Empty>::destroy_callback
    }
}

#[test]
fn a_panicking_aggregate_update_becomes_a_sql_error() {
    use quack_rs::aggregate::AggregateFunctionBuilder;

    let fx = Fixture::open();

    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("boom_agg")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(panicking_aggregate::state_size())
            .init(panicking_aggregate::state_init())
            .update(panicking_aggregate::update)
            .combine(panicking_aggregate::combine)
            .finalize(panicking_aggregate::finalize)
            .destructor(panicking_aggregate::destroy())
            .register(fx.con())
            .expect("register boom_agg");
    }

    // Without `aggregate_update_callback!` this panic would unwind out of a
    // DuckDB worker thread and abort the process.
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT boom_agg(i) FROM range(10) t(i)") }
        .expect_err("the panic must surface as an error");
    assert!(
        err.as_str().contains("deliberately exploded"),
        "panic payload should reach the user: {err}"
    );

    // The connection survives.
    assert_eq!(
        fx.scalar("SELECT 5::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

quack_rs::cast_callback!(panicking_cast, |_info, _count, _input, _output| {
    panic!("cast deliberately exploded");
});

#[test]
fn a_panicking_cast_becomes_a_sql_error() {
    use quack_rs::cast::CastFunctionBuilder;

    let fx = Fixture::open();

    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
            .function(panicking_cast)
            .register(fx.con())
            .expect("register cast");
    }

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT CAST('7' AS INTEGER)") }
        .expect_err("the panic must surface as an error");
    assert!(
        err.as_str().contains("cast deliberately exploded"),
        "panic payload should reach the user: {err}"
    );
}

// ─── LIST and MAP construction ───────────────────────────────────────────────

quack_rs::scalar_callback!(make_range_list, |_info, input, output| {
    // Builds LIST<BIGINT> = [0, 1, ..., n-1] for each input n.
    use quack_rs::vector::ListBuilder;
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let n = if unsafe { reader.is_valid(row) } {
            unsafe { reader.read_i64(row) }.max(0) as usize
        } else {
            0
        };
        unsafe {
            builder.push_row(row, n, |writer, base| {
                for i in 0..n {
                    writer.write_i64(base + i, i as i64);
                }
            });
        }
    }
    unsafe { builder.finish() };
});

#[test]
fn list_builder_writes_correct_offsets_across_growth() {
    let fx = Fixture::open();

    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("make_range_list")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::BigInt))
            .function(make_range_list)
            .register(fx.con())
            .expect("register make_range_list");
    }

    // A single row.
    assert_eq!(
        fx.scalar("SELECT make_range_list(3)::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .as_deref(),
        Some("[0, 1, 2]")
    );

    // Empty lists must produce a zero-length entry, not NULL.
    assert_eq!(
        fx.scalar("SELECT make_range_list(0)::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .as_deref(),
        Some("[]")
    );

    // Many rows of varying length in one chunk: this is where a stale child
    // pointer or a mis-tracked offset shows up. Each row's list must be exactly
    // 0..len, and the flattened total must match.
    let mut result = fx.query(
        "SELECT count(*) AS rows,
                sum(len(l)) AS total_elements,
                count(*) FILTER (WHERE l = [x for x in range(len(l))]) AS correct
         FROM (SELECT i % 37 AS n, make_range_list(i % 37) AS l FROM range(2000) t(i))",
    );
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: all three columns are integral and row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_i64(0), 2000);
        let expected_total: i128 = (0..2000i128).map(|i| i % 37).sum();
        assert_eq!(chunk.reader(1).read_i128(0), expected_total);
        assert_eq!(
            chunk.reader(2).read_i64(0),
            2000,
            "every row's list must equal 0..len"
        );
    }
}

quack_rs::scalar_callback!(make_index_map, |_info, input, output| {
    // Builds MAP(VARCHAR, BIGINT) = { 'k0': 0, 'k1': 1, ... } for each input n.
    use quack_rs::vector::ListBuilder;
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut builder = unsafe { ListBuilder::new(output) };
    for row in 0..chunk.size() {
        let n = if unsafe { reader.is_valid(row) } {
            unsafe { reader.read_i64(row) }.max(0) as usize
        } else {
            0
        };
        unsafe {
            builder.push_map_row(row, n, |keys, values, base| {
                for i in 0..n {
                    keys.write_varchar(base + i, &format!("k{i}"));
                    values.write_i64(base + i, i as i64);
                }
            });
        }
    }
    unsafe { builder.finish() };
});

#[test]
fn list_builder_drives_map_vectors_too() {
    let fx = Fixture::open();

    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("make_index_map")
            .expect("name")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::map(TypeId::Varchar, TypeId::BigInt))
            .function(make_index_map)
            .register(fx.con())
            .expect("register make_index_map");
    }

    assert_eq!(
        fx.scalar("SELECT make_index_map(2)::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .as_deref(),
        Some("{k0=0, k1=1}")
    );
    assert_eq!(
        fx.scalar("SELECT make_index_map(0)::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .as_deref(),
        Some("{}")
    );

    // Across a full chunk, with growth.
    let mut result = fx.query(
        "SELECT count(*) FILTER (WHERE map_extract(m, 'k3') = [3]) AS hits
         FROM (SELECT make_index_map(i % 11) AS m FROM range(1500) t(i))",
    );
    let chunk = result.next_chunk().expect("one chunk");
    let expected = (0..1500i64).filter(|i| i % 11 > 3).count() as i64;
    // SAFETY: the column is BIGINT and row 0 exists.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, expected);
}

// ---------------------------------------------------------------------------
// Virtual file system (DuckDB 1.5.0+)
// ---------------------------------------------------------------------------

/// Round-trips a file through `DuckDB`'s own VFS.
///
/// The point is not that a local file works — `std::fs` would do that. It is
/// that `read`/`write` are documented to move *up to* the requested number of
/// bytes, so the looping helpers are the only ones safe to build on, and they
/// need to be checked against a real file system implementation rather than
/// assumed.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn the_virtual_file_system_round_trips_a_file() {
    use quack_rs::client_context::ClientContext;
    use quack_rs::file_system::{FileOpenOptions, FileSystem};

    let fx = Fixture::open();
    // SAFETY: `con` is open for the fixture's lifetime.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("vfs-roundtrip.bin");
    let c_path =
        std::ffi::CString::new(path.to_str().expect("utf-8 path")).expect("no interior NUL");

    // A payload big enough that a short write is plausible, and containing a NUL
    // so any accidental C-string handling shows up.
    let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    assert!(payload.contains(&0), "payload must exercise interior NUL");

    {
        let handle = fs
            .open(&c_path, &FileOpenOptions::write_create())
            .expect("open for write");
        handle.write_all(&payload).expect("write_all");
        handle.sync().expect("sync");
        handle.close().expect("close");
    }

    // Whole-file read.
    let handle = fs
        .open(&c_path, &FileOpenOptions::read_only())
        .expect("open for read");
    assert_eq!(handle.size().expect("size"), payload.len() as u64);
    assert_eq!(handle.tell().expect("tell"), 0);

    let mut read_back = Vec::new();
    let n = handle.read_to_end(&mut read_back).expect("read_to_end");
    assert_eq!(n, payload.len());
    assert_eq!(read_back, payload);
    assert_eq!(handle.tell().expect("tell at EOF"), payload.len() as u64);

    // `read_to_end` appends rather than replacing, and returns only what it added.
    handle.seek(0).expect("seek to start");
    let added = handle
        .read_to_end(&mut read_back)
        .expect("second read_to_end");
    assert_eq!(added, payload.len());
    assert_eq!(read_back.len(), payload.len() * 2);
    assert_eq!(&read_back[payload.len()..], &payload[..]);

    // `read_exact` from an arbitrary offset.
    handle.seek(1000).expect("seek");
    let mut window = [0u8; 4096];
    handle.read_exact(&mut window).expect("read_exact");
    assert_eq!(&window[..], &payload[1000..1000 + 4096]);

    // `read_exact` past the end is an error, not a silent short read.
    handle
        .seek(payload.len() as u64 - 10)
        .expect("seek near EOF");
    let mut too_big = [0u8; 64];
    let err = handle
        .read_exact(&mut too_big)
        .expect_err("read_exact past EOF must fail");
    let message = err.message().unwrap_or_default();
    assert!(
        message.contains("unexpected end of file"),
        "unexpected error: {message}"
    );

    // A plain `read` at EOF returns 0, which is how `read_to_end` terminates.
    handle.seek(payload.len() as u64).expect("seek to EOF");
    assert_eq!(handle.read(&mut window).expect("read at EOF"), 0);
    let mut empty = Vec::new();
    assert_eq!(
        handle.read_to_end(&mut empty).expect("read_to_end at EOF"),
        0
    );
    assert!(empty.is_empty());

    // Zero-length operations are no-ops, not errors.
    handle.read_exact(&mut []).expect("empty read_exact");
    handle.write_all(&[]).expect("empty write_all");
}

/// Opening a file that does not exist yields structured error data, not a panic
/// or a null handle the caller has to guess about.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn opening_a_missing_file_returns_structured_error_data() {
    use quack_rs::client_context::ClientContext;
    use quack_rs::file_system::{FileOpenOptions, FileSystem};

    let fx = Fixture::open();
    // SAFETY: `con` is open for the fixture's lifetime.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");

    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("definitely-not-here.bin");
    let c_path =
        std::ffi::CString::new(missing.to_str().expect("utf-8 path")).expect("no interior NUL");

    let err = fs
        .open(&c_path, &FileOpenOptions::read_only())
        .expect_err("opening a missing file must fail");
    assert!(
        err.message().is_some_and(|m| !m.is_empty()),
        "error data must carry a message, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Debug impls
// ---------------------------------------------------------------------------

/// The `Debug` impls that decode `DuckDB` state must actually decode it, and
/// must survive the edge cases they claim to: a null handle, and a type id this
/// build does not know.
#[test]
fn debug_impls_decode_live_duckdb_state() {
    use quack_rs::value::Value;

    let _fx = Fixture::open();

    // LogicalType renders the decoded type, not a pointer.
    let bigint = LogicalType::new(TypeId::BigInt);
    assert_eq!(format!("{bigint:?}"), "LogicalType { type_id: BigInt }");

    // DECIMAL carries width and scale, which is the whole reason two DECIMALs
    // can look identical and behave differently.
    let decimal = LogicalType::decimal(18, 3);
    let rendered = format!("{decimal:?}");
    assert!(
        rendered.contains("Decimal")
            && rendered.contains("width: 18")
            && rendered.contains("scale: 3"),
        "{rendered}"
    );

    // An alias shows up when set, and is absent when not.
    let aliased = LogicalType::new(TypeId::Integer);
    // SAFETY: `aliased` is a valid logical type.
    unsafe { aliased.set_alias("my_domain") };
    assert!(
        format!("{aliased:?}").contains("alias: \"my_domain\""),
        "{aliased:?}"
    );
    assert!(!format!("{bigint:?}").contains("alias"), "{bigint:?}");

    // Value renders its type, and (with duckdb-1-5) DuckDB's own rendering.
    let value = Value::bigint(-42);
    let rendered = format!("{value:?}");
    assert!(rendered.contains("type: BigInt"), "{rendered}");
    #[cfg(feature = "duckdb-1-5")]
    assert!(rendered.contains("-42"), "{rendered}");

    // `Value::type_id` is what makes the untyped `as_*` accessors checkable.
    assert_eq!(Value::bigint(1).type_id(), Some(TypeId::BigInt));
    assert_eq!(Value::varchar("x").type_id(), Some(TypeId::Varchar));
    assert_eq!(Value::boolean(true).type_id(), Some(TypeId::Boolean));
    assert_eq!(Value::double(1.5).type_id(), Some(TypeId::Double));
    assert_eq!(Value::date(0).type_id(), Some(TypeId::Date));
    assert_eq!(Value::timestamp(0).type_id(), Some(TypeId::Timestamp));
    assert_eq!(Value::uuid(0).type_id(), Some(TypeId::Uuid));
    #[cfg(feature = "duckdb-1-5")]
    assert_eq!(Value::null_value().type_id(), Some(TypeId::SqlNull));

    // A builder's Debug answers "did I wire the callback up?", which is the
    // question you have when `register` reports a missing function.
    let builder = ScalarFunctionBuilder::try_new("dbg_probe")
        .expect("name")
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt);
    let rendered = format!("{builder:?}");
    assert!(rendered.contains("function: unset"), "{rendered}");
    let builder = builder.function(echo_i64);
    assert!(
        format!("{builder:?}").contains("function: set"),
        "{builder:?}"
    );
}

/// A `Debug` impl that panics inside a panic message aborts the process, so the
/// decoding impls must tolerate the handles they can actually be handed.
///
/// `LogicalType` is not covered here because it cannot be: every one of its
/// constructors — `from_raw` included — asserts the handle is non-null, so a
/// null `LogicalType` is unconstructible. `Value::from_raw` has no such assert,
/// so a null `Value` is reachable and is tested.
#[test]
fn debug_of_a_null_value_does_not_dereference() {
    use quack_rs::value::Value;

    let _fx = Fixture::open();

    // SAFETY: a null handle is exactly the case under test; `Debug` must not
    // dereference it, and `Drop` already skips null.
    let null_value = unsafe { Value::from_raw(std::ptr::null_mut()) };
    assert_eq!(format!("{null_value:?}"), "Value(<null handle>)");
    assert_eq!(null_value.type_id(), None);
}

// ---------------------------------------------------------------------------
// Appender (stable prefix — no feature flag)
// ---------------------------------------------------------------------------

/// Round-trips every row-at-a-time `append_*` through a real table.
///
/// These slots (330–356) have been in the frozen stable prefix since v1.2.0 and
/// were entirely unwrapped; the physical encodings they use — `HUGEINT`'s
/// lower/upper split, `VARCHAR`'s explicit length, `INTERVAL`'s three fields —
/// are exactly the kind that a mock cannot check.
#[test]
fn the_appender_writes_every_scalar_type() {
    use quack_rs::appender::Appender;

    let fx = Fixture::open();
    fx.query(
        "CREATE TABLE every_type (
             b BOOLEAN, i8 TINYINT, i16 SMALLINT, i32 INTEGER, i64 BIGINT, i128 HUGEINT,
             u8 UTINYINT, u16 USMALLINT, u32 UINTEGER, u64 UBIGINT, u128 UHUGEINT,
             f32 FLOAT, f64 DOUBLE, s VARCHAR, bl BLOB,
             d DATE, t TIME, ts TIMESTAMP, iv INTERVAL, n INTEGER
         )",
    );

    // SAFETY: `con` is open for the fixture's lifetime and the table exists.
    let appender =
        unsafe { Appender::new(fx.con(), None, c"every_type") }.expect("create appender");
    assert_eq!(appender.column_count(), 20);

    appender
        .row(|row| {
            row.append_bool(true)?;
            row.append_i8(i8::MIN)?;
            row.append_i16(i16::MIN)?;
            row.append_i32(i32::MIN)?;
            row.append_i64(i64::MIN)?;
            row.append_i128(i128::MIN)?;
            row.append_u8(u8::MAX)?;
            row.append_u16(u16::MAX)?;
            row.append_u32(u32::MAX)?;
            row.append_u64(u64::MAX)?;
            row.append_u128(u128::MAX)?;
            row.append_f32(-1.5)?;
            row.append_f64(2.25)?;
            row.append_str("hé\u{1F600}llo")?;
            row.append_bytes(&[0x00, 0xff, 0x41])?;
            row.append_date(-1)?;
            row.append_time(3_600_000_001)?;
            row.append_timestamp(-1)?;
            row.append_interval(quack_rs::interval::DuckInterval {
                months: 13,
                days: -2,
                micros: 5,
            })?;
            row.append_null()
        })
        .expect("append row");
    appender.close().expect("close appender");

    let mut result = fx.query(
        "SELECT b, i8, i16, i32, i64, i128::VARCHAR, u8, u16, u32, u64, u128::VARCHAR,
                f32, f64, s, bl::VARCHAR, d::VARCHAR, t::VARCHAR, ts::VARCHAR, iv::VARCHAR,
                n IS NULL
         FROM every_type",
    );
    let chunk = result.next_chunk().expect("one chunk");
    assert_eq!(chunk.size(), 1);
    // SAFETY: every column below matches the declared type, and row 0 exists.
    unsafe {
        assert!(chunk.reader(0).read_bool(0));
        assert_eq!(chunk.reader(1).read_i8(0), i8::MIN);
        assert_eq!(chunk.reader(2).read_i16(0), i16::MIN);
        assert_eq!(chunk.reader(3).read_i32(0), i32::MIN);
        assert_eq!(chunk.reader(4).read_i64(0), i64::MIN);
        assert_eq!(chunk.reader(5).read_str(0), i128::MIN.to_string());
        assert_eq!(chunk.reader(6).read_u8(0), u8::MAX);
        assert_eq!(chunk.reader(7).read_u16(0), u16::MAX);
        assert_eq!(chunk.reader(8).read_u32(0), u32::MAX);
        assert_eq!(chunk.reader(9).read_u64(0), u64::MAX);
        assert_eq!(chunk.reader(10).read_str(0), u128::MAX.to_string());
        assert!((chunk.reader(11).read_f32(0) - -1.5).abs() < f32::EPSILON);
        assert!((chunk.reader(12).read_f64(0) - 2.25).abs() < f64::EPSILON);
        assert_eq!(chunk.reader(13).read_str(0), "hé\u{1F600}llo");
        assert_eq!(chunk.reader(14).read_str(0), "\\x00\\xFFA");
        assert_eq!(chunk.reader(15).read_str(0), "1969-12-31");
        assert_eq!(chunk.reader(16).read_str(0), "01:00:00.000001");
        assert_eq!(chunk.reader(17).read_str(0), "1969-12-31 23:59:59.999999");
        assert_eq!(
            chunk.reader(18).read_str(0),
            "1 year 1 month -2 days 00:00:00.000005"
        );
        assert!(
            chunk.reader(19).read_bool(0),
            "the NULL column must be NULL"
        );
    }
}

/// A `VARCHAR` with an interior NUL survives, because `append_str` uses
/// `duckdb_append_varchar_length` rather than the NUL-terminated variant.
#[test]
fn appended_varchars_keep_interior_nuls() {
    use quack_rs::appender::Appender;

    let fx = Fixture::open();
    fx.query("CREATE TABLE nul_text (s VARCHAR)");

    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"nul_text") }.expect("create");
    appender
        .row(|row| row.append_str("before\0after"))
        .expect("append");
    appender.close().expect("close");

    let mut result = fx.query("SELECT length(s), s FROM nul_text");
    let chunk = result.next_chunk().expect("chunk");
    // SAFETY: BIGINT then VARCHAR, row 0 exists.
    unsafe {
        assert_eq!(
            chunk.reader(0).read_i64(0),
            12,
            "a NUL-terminated append would have stored 6 characters"
        );
        assert_eq!(chunk.reader(1).read_str(0), "before\0after");
    }
}

/// Bulk appends across many chunks, plus the failure modes: a short row, a
/// constraint violation surfacing at flush rather than at append, and the fact
/// that a create against a missing table reports which table.
#[test]
fn the_appender_reports_its_failure_modes() {
    use quack_rs::appender::Appender;

    let fx = Fixture::open();
    fx.query("CREATE TABLE bulk (id INTEGER PRIMARY KEY, label VARCHAR)");

    // 5000 rows spans several vectors, so this exercises the appender's own
    // internal chunk flushing rather than a single buffered chunk.
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"bulk") }.expect("create");
    for i in 0..5000i32 {
        appender
            .row(|row| {
                row.append_i32(i)?;
                row.append_str(&format!("row-{i}"))
            })
            .expect("append");
    }
    appender.close().expect("close");

    let mut result = fx.query("SELECT count(*), min(id), max(id) FROM bulk");
    let chunk = result.next_chunk().expect("chunk");
    // SAFETY: three BIGINT/INTEGER columns, row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_i64(0), 5000);
        assert_eq!(chunk.reader(1).read_i32(0), 0);
        assert_eq!(chunk.reader(2).read_i32(0), 4999);
    }

    // A row with fewer values than columns is rejected by end_row.
    // SAFETY: as above.
    let appender = unsafe { Appender::new(fx.con(), None, c"bulk") }.expect("create");
    appender.append_i32(99_999).expect("first column");
    let err = appender.end_row().expect_err("short row must fail");
    assert!(
        format!("{err}").to_lowercase().contains("column"),
        "unhelpful error: {err}"
    );
    drop(appender);

    // A primary-key collision is buffered, so it surfaces at close, not append.
    // SAFETY: as above.
    let appender = unsafe { Appender::new(fx.con(), None, c"bulk") }.expect("create");
    appender
        .row(|row| {
            row.append_i32(0)?;
            row.append_str("duplicate")
        })
        .expect("the append itself succeeds — the row is only buffered");
    let err = appender
        .close()
        .expect_err("close must report the violation");
    assert!(
        format!("{err}").to_lowercase().contains("constraint")
            || format!("{err}").to_lowercase().contains("duplicate"),
        "unexpected error: {err}"
    );

    // Creating against a missing table names the table.
    // SAFETY: `con` is open; the table deliberately does not exist.
    let err = unsafe { Appender::new(fx.con(), None, c"no_such_table") }
        .err()
        .expect("create must fail");
    assert!(
        format!("{err}").contains("no_such_table"),
        "unexpected error: {err}"
    );
}

/// `error_message` reads `duckdb_appender_error` — the *stable-prefix* error
/// channel (slot 285), and the one `AppendError` resolves to when `duckdb-1-5`
/// is off.
///
/// The 1.5 build never exercises that path, so without this the stable error
/// channel would only ever be tested by CI's feature-off job.
#[test]
fn the_stable_error_channel_reports_appender_failures() {
    use quack_rs::appender::Appender;

    let fx = Fixture::open();
    fx.query("CREATE TABLE two_cols (a INTEGER, b INTEGER)");

    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"two_cols") }.expect("create");
    assert_eq!(
        appender.error_message(),
        None,
        "a healthy appender must report no error"
    );

    appender.append_i32(1).expect("first column");
    assert!(appender.end_row().is_err(), "a short row must fail");

    let message = appender
        .error_message()
        .expect("the stable channel must carry the message");
    assert!(
        message.to_lowercase().contains("column"),
        "unhelpful message: {message}"
    );
}

/// `add_column` narrows the active column list, and the omitted columns take
/// their `DEFAULT`.
#[test]
fn the_appender_can_target_a_subset_of_columns() {
    use quack_rs::appender::Appender;
    use quack_rs::table_description::TableDescription;

    let fx = Fixture::open();
    fx.query("CREATE TABLE partial (id INTEGER, note VARCHAR DEFAULT 'unset')");

    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"partial") }.expect("create");
    assert_eq!(appender.column_count(), 2);
    appender.add_column(c"id").expect("restrict to id");
    assert_eq!(appender.column_count(), 1);
    appender.row(|row| row.append_i32(7)).expect("append");
    appender.close().expect("close");

    let mut result = fx.query("SELECT id, note FROM partial");
    let chunk = result.next_chunk().expect("chunk");
    // SAFETY: INTEGER then VARCHAR, row 0 exists.
    unsafe {
        assert_eq!(chunk.reader(0).read_i32(0), 7);
        assert_eq!(chunk.reader(1).read_str(0), "unset");
    }

    // TableDescription is the way to know a DEFAULT exists before relying on it.
    // SAFETY: `con` is open and the table exists.
    let desc = unsafe { TableDescription::create(fx.con(), "main", "partial") }.expect("describe");
    assert_eq!(desc.column_name(0).as_deref(), Some("id"));
    assert_eq!(desc.column_name(1).as_deref(), Some("note"));
    assert_eq!(desc.column_name(99), None);
    assert_eq!(desc.column_has_default(0), Some(false));
    assert_eq!(desc.column_has_default(1), Some(true));
    assert_eq!(desc.column_has_default(99), None);

    // SAFETY: `con` is open; the default catalog and schema are addressed by name.
    let qualified =
        unsafe { TableDescription::with_catalog(fx.con(), None, Some("main"), "partial") }
            .expect("describe via create_ext");
    assert_eq!(qualified.column_name(0).as_deref(), Some("id"));
}

/// `append_value` is the escape hatch for types with no dedicated `append_*`,
/// and must refuse a null handle rather than letting `DuckDB` dereference it.
#[test]
fn append_value_covers_types_without_a_dedicated_method() {
    use quack_rs::appender::Appender;
    use quack_rs::value::Value;

    let fx = Fixture::open();
    fx.query("CREATE TABLE valued (u UUID, d DATE)");

    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"valued") }.expect("create");
    appender
        .row(|row| {
            row.append_value(&Value::uuid(u128::MAX))?;
            row.append_value(&Value::date(19_000))
        })
        .expect("append");
    appender.close().expect("close");

    let mut result = fx.query("SELECT u::VARCHAR, d::VARCHAR FROM valued");
    let chunk = result.next_chunk().expect("chunk");
    // SAFETY: two VARCHAR columns, row 0 exists.
    unsafe {
        assert_eq!(
            chunk.reader(0).read_str(0),
            "ffffffff-ffff-ffff-ffff-ffffffffffff"
        );
        assert_eq!(chunk.reader(1).read_str(0), "2022-01-08");
    }

    // duckdb_append_value dereferences its argument with no null check, so a
    // null handle must never reach it.
    // SAFETY: `con` is open and the table exists.
    let appender = unsafe { Appender::new(fx.con(), None, c"valued") }.expect("create");
    // SAFETY: a null handle is the case under test.
    let null_value = unsafe { Value::from_raw(std::ptr::null_mut()) };
    let err = appender
        .append_value(&null_value)
        .expect_err("a null value handle must be refused");
    assert!(format!("{err}").contains("null"), "unexpected error: {err}");
}

/// The `UUID` accessors must all speak the same 128 bits.
///
/// A `UUID` column is physically a `HUGEINT`, but `DuckDB` stores it with the
/// top bit flipped so signed integer ordering matches string ordering
/// (`BaseUUID::FromUHugeint` subtracts 2^63 from the upper half). Before this
/// was fixed, `VectorReader::read_uuid` returned the raw storage while
/// `Value::as_uuid` returned the textual bits, and the docs on both claimed
/// they matched — so handing one to the other silently changed the UUID's first
/// hex digit.
#[test]
fn every_uuid_accessor_agrees_on_which_128_bits_it_means() {
    use quack_rs::value::Value;
    use quack_rs::vector::{uuid_from_storage, uuid_to_storage};

    let fx = Fixture::open();
    let text = "11111111-2222-3333-4444-555555555555";
    let bits: u128 = 0x1111_1111_2222_3333_4444_5555_5555_5555;

    // Reading a real UUID column yields the textual bits.
    let mut result = fx.query(&format!("SELECT '{text}'::UUID"));
    let chunk = result.next_chunk().expect("chunk");
    // SAFETY: the column is UUID and row 0 exists.
    let (read_bits, raw_storage) = unsafe {
        let reader = chunk.reader(0);
        (reader.read_uuid(0), reader.read_i128(0))
    };
    assert_eq!(read_bits, bits, "read_uuid must return the textual bits");

    // ...and the raw storage really is different, so this is not a no-op.
    assert_ne!(raw_storage as u128, bits, "DuckDB's flip must be real");
    assert_eq!(uuid_from_storage(raw_storage), bits);
    assert_eq!(uuid_to_storage(bits), raw_storage);

    // `Value` agrees with the vector accessors.
    let value = Value::uuid(bits);
    assert_eq!(value.as_uuid(), bits);
    #[cfg(feature = "duckdb-1-5")]
    assert_eq!(
        value.display_string().as_deref(),
        Some(format!("'{text}'::UUID").as_str())
    );

    // And the full loop: write through a vector, render through SQL.
    register_echo(
        fx.con(),
        "echo_uuid2",
        TypeId::Uuid,
        TypeId::Uuid,
        echo_uuid,
    );
    assert_eq!(
        fx.scalar(
            &format!("SELECT echo_uuid2('{text}'::UUID)::VARCHAR"),
            |r, i| { unsafe { r.read_str(i).to_owned() } }
        )
        .as_deref(),
        Some(text)
    );

    // The extremes, where a sign-flip bug is most visible.
    for (bits, rendered) in [
        (0u128, "00000000-0000-0000-0000-000000000000"),
        (u128::MAX, "ffffffff-ffff-ffff-ffff-ffffffffffff"),
        (1u128 << 127, "80000000-0000-0000-0000-000000000000"),
    ] {
        assert_eq!(Value::uuid(bits).as_uuid(), bits);
        assert_eq!(uuid_from_storage(uuid_to_storage(bits)), bits);
        // `display_string` is a 1.5 addition; without it, ask SQL directly.
        assert_eq!(
            fx.scalar(
                &format!("SELECT '{rendered}'::UUID::VARCHAR"),
                |r, i| unsafe { r.read_str(i).to_owned() }
            )
            .as_deref(),
            Some(rendered),
            "{bits:#034x}"
        );
    }
    // The nil UUID must sit at the bottom of DuckDB's signed ordering — that is
    // the entire reason the flip exists.
    assert_eq!(uuid_to_storage(0), i128::MIN);
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

/// `list_duckdb_secrets` reads real secret metadata — and demonstrably cannot
/// read the credential, which is the point the module documentation makes.
#[test]
fn duckdb_secret_metadata_is_readable_and_the_credential_is_not() {
    use quack_rs::secrets::list_duckdb_secrets;

    let fx = Fixture::open();

    // SAFETY: `con` is open for the fixture's lifetime.
    let empty = unsafe { list_duckdb_secrets(fx.con()) }.expect("query duckdb_secrets()");
    assert!(empty.is_empty(), "a fresh database has no secrets");

    fx.query("CREATE SECRET probe_s3 (TYPE s3, KEY_ID 'AKIAEXAMPLE', SECRET 'super-secret-value')");
    fx.query("CREATE SECRET probe_http (TYPE http, EXTRA_HTTP_HEADERS MAP{'X':'Y'})");

    // SAFETY: as above.
    let secrets = unsafe { list_duckdb_secrets(fx.con()) }.expect("query duckdb_secrets()");
    assert_eq!(secrets.len(), 2);

    let s3 = secrets
        .iter()
        .find(|s| s.name == "probe_s3")
        .expect("the s3 secret");
    assert_eq!(s3.secret_type, "s3");
    assert_eq!(s3.provider, "config");
    assert!(!s3.persistent, "a session secret is not persistent");
    // scope is a VARCHAR[]; DuckDB gives an s3 secret three default prefixes.
    assert!(
        s3.scope.iter().any(|p| p == "s3://"),
        "unexpected scope: {:?}",
        s3.scope
    );
    assert!(
        s3.scope.iter().all(|p| !p.starts_with('\'')),
        "array quoting must be stripped: {:?}",
        s3.scope
    );

    // The whole reason this returns metadata and not a `SecretEntry`.
    assert!(
        s3.secret_string.contains("key_id=AKIAEXAMPLE"),
        "{}",
        s3.secret_string
    );
    assert!(
        s3.secret_string.contains("secret=redacted"),
        "DuckDB must redact the credential: {}",
        s3.secret_string
    );
    assert!(
        !s3.secret_string.contains("super-secret-value"),
        "the credential leaked: {}",
        s3.secret_string
    );

    // An empty scope round-trips as an empty vector, not as [""].
    let http = secrets
        .iter()
        .find(|s| s.name == "probe_http")
        .expect("the http secret");
    assert!(http.scope.is_empty(), "unexpected scope: {:?}", http.scope);
}

// ---------------------------------------------------------------------------
// Name validation, checked against what DuckDB actually accepts
// ---------------------------------------------------------------------------

/// `validate_function_name` gates `try_new`, so anything it rejects is a
/// function nobody can register through quack-rs. It must therefore reject only
/// names `DuckDB` genuinely cannot take.
#[test]
fn the_name_validator_accepts_every_name_duckdb_does() {
    use quack_rs::validate::{validate_extension_name, validate_function_name};

    let fx = Fixture::open();

    // Every extension name this DuckDB knows.
    let mut result = fx.query("SELECT DISTINCT extension_name FROM duckdb_extensions()");
    let mut extensions = 0;
    while let Some(chunk) = result.next_chunk() {
        for row in 0..chunk.size() {
            // SAFETY: VARCHAR column.
            let name = unsafe { chunk.reader(0).read_str(row) }.to_owned();
            extensions += 1;
            assert!(
                validate_extension_name(&name).is_ok(),
                "DuckDB ships extension {name:?} but quack-rs rejects the name"
            );
        }
    }
    assert!(
        extensions > 10,
        "expected a real extension list, got {extensions}"
    );

    // Every function DuckDB ships, minus the operators — nobody registers `+`
    // or `||` through a builder, and rejecting them is the point.
    let mut result = fx.query("SELECT DISTINCT function_name FROM duckdb_functions()");
    let (mut checked, mut operators) = (0, 0);
    while let Some(chunk) = result.next_chunk() {
        for row in 0..chunk.size() {
            // SAFETY: VARCHAR column.
            let name = unsafe { chunk.reader(0).read_str(row) }.to_owned();
            let identifier_like = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !identifier_like {
                operators += 1;
                assert!(
                    validate_function_name(&name).is_err(),
                    "{name:?} needs quoting in SQL and should be rejected"
                );
                continue;
            }
            checked += 1;
            assert!(
                validate_function_name(&name).is_ok(),
                "DuckDB ships function {name:?} but quack-rs rejects the name"
            );
        }
    }
    assert!(
        checked > 500,
        "expected DuckDB's full function list, got {checked}"
    );
    assert!(operators > 10, "expected operator names in the list");
}

/// The specific case that exposed it: `DuckDB` ships `formatReadableSize`, so a
/// camelCase name must register and be callable — under any casing, because
/// `DuckDB` identifiers are case-insensitive.
#[test]
fn a_mixed_case_function_registers_and_is_callable() {
    let fx = Fixture::open();

    // SAFETY: `con` is open; the callback matches the declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("formatReadableThing")
            .expect("DuckDB ships mixed-case functions; quack-rs must allow them")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(echo_i64)
            .register(fx.con())
            .expect("register");
    }

    for sql in [
        "SELECT formatReadableThing(7)",
        "SELECT formatreadablething(7)",
        "SELECT FORMATREADABLETHING(7)",
    ] {
        assert_eq!(
            fx.scalar(sql, |r, i| unsafe { r.read_i64(i) }),
            Some(7),
            "{sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// Replacement scans
// ---------------------------------------------------------------------------

quack_rs::replacement_scan_callback!(route_myfmt, |info, table_name, _data| {
    // SAFETY: DuckDB passes a valid NUL-terminated identifier.
    let name = unsafe { std::ffi::CStr::from_ptr(table_name) }.to_string_lossy();
    if !name.ends_with(".myfmt") {
        // Not ours — returning without touching `info` lets DuckDB fall
        // through to its own handling, which is the behaviour under test.
        return;
    }
    let digits: i64 = name
        .trim_end_matches(".myfmt")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    // SAFETY: `info` is the pointer DuckDB passed in.
    unsafe {
        quack_rs::replacement_scan::ReplacementScanInfo::new(info)
            .set_function("count_up")
            .add_i64_parameter(digits);
    }
});

quack_rs::replacement_scan_callback!(exploding_scan, |_info, table_name, _data| {
    // SAFETY: DuckDB passes a valid NUL-terminated identifier.
    let name = unsafe { std::ffi::CStr::from_ptr(table_name) }.to_string_lossy();
    if name.ends_with(".boom") {
        panic!("replacement scan deliberately exploded");
    }
});

/// A replacement scan rewrites `SELECT * FROM 'something'` into a table
/// function call. Nothing in the crate exercised this path against a real
/// `DuckDB` — the module had three unit tests, none of them registering
/// anything.
#[test]
fn a_replacement_scan_redirects_an_unknown_table() {
    use quack_rs::replacement_scan::ReplacementScanBuilder;
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();

    struct State {
        next: i64,
        limit: i64,
    }

    let table_fn = TableFunctionBuilder::new("count_up")
        .param(TypeId::BigInt)
        .with_state::<State, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: parameter 0 was declared above.
            let limit = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
            Ok(State { next: 1, limit })
        })
        .scan(|state, chunk| {
            if state.next > state.limit {
                // SAFETY: ending the scan.
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            // SAFETY: column 0 is BIGINT and row 0 is in range.
            unsafe {
                chunk.writer(0).write_i64(0, state.next);
                chunk.set_size(1);
            }
            state.next += 1;
            Ok(())
        })
        .build()
        .expect("build count_up");

    // SAFETY: `con` is open.
    unsafe { table_fn.register(fx.con()) }.expect("register count_up");

    // SAFETY: `db` is open; the callback has the required signature and no
    // extra data to clean up.
    unsafe {
        ReplacementScanBuilder::register(fx.db(), route_myfmt, std::ptr::null_mut(), None);
    }

    // The whole point: an identifier DuckDB knows nothing about becomes a call
    // to our table function.
    assert_eq!(
        fx.scalar("SELECT sum(n) FROM '10.myfmt'", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(55)
    );
    assert_eq!(
        fx.scalar("SELECT count(*) FROM '0.myfmt'", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(0)
    );

    // An identifier the callback declines must fall through to DuckDB's own
    // handling, not be swallowed.
    let err = unsafe { quack_rs::query::query(fx.con(), "SELECT * FROM 'nope.csv'") }
        .expect_err("DuckDB must still report its own error for a file it cannot find");
    let message = format!("{err}");
    assert!(
        message.contains("nope.csv"),
        "the error should name the file: {message}"
    );
}

/// A panic inside a replacement scan must reach SQL as an error, not abort.
#[test]
fn a_panicking_replacement_scan_becomes_a_sql_error() {
    use quack_rs::replacement_scan::ReplacementScanBuilder;

    let fx = Fixture::open();
    // SAFETY: `db` is open; no extra data to clean up.
    unsafe {
        ReplacementScanBuilder::register(fx.db(), exploding_scan, std::ptr::null_mut(), None);
    }

    let err = unsafe { quack_rs::query::query(fx.con(), "SELECT * FROM 'x.boom'") }
        .expect_err("the panic must surface as a SQL error");
    let message = format!("{err}");
    assert!(
        message.contains("replacement scan deliberately exploded"),
        "the panic message must reach SQL: {message}"
    );
}

// ---------------------------------------------------------------------------
// Copy functions (DuckDB 1.5.0+)
// ---------------------------------------------------------------------------

/// A `COPY ... TO 'file' (FORMAT my_format)` handler, exercised end to end.
///
/// The four-phase lifecycle — bind, global init, sink, finalize — threads two
/// separate heap allocations through `DuckDB` (`set_bind_data` and
/// `set_global_state`), each with its own destructor. Nothing in the crate
/// exercised any of it against a real `DuckDB`.
#[cfg(feature = "duckdb-1-5")]
mod copy_fn {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Column count captured at bind time, handed to the sink as bind data.
    pub struct BindData {
        pub columns: u64,
    }

    /// Rows written, plus the destination path, as global state.
    pub struct GlobalState {
        pub path: String,
        pub rows: u64,
        pub checksum: i64,
    }

    /// Counts destructor calls so a leak or a double free is visible.
    pub static BIND_DROPS: AtomicUsize = AtomicUsize::new(0);
    pub static GLOBAL_DROPS: AtomicUsize = AtomicUsize::new(0);
    /// What finalize saw, so the test can assert on it after the COPY.
    pub static FINAL_ROWS: AtomicUsize = AtomicUsize::new(0);
    pub static FINAL_CHECKSUM: AtomicUsize = AtomicUsize::new(0);
    /// The `COPY ... TO` options, as `CopyBindInfo::options` reported them:
    /// `(field name, rendered value)` for every field of the STRUCT.
    pub static OPTIONS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

    pub unsafe extern "C" fn drop_bind(ptr: *mut c_void) {
        if !ptr.is_null() {
            BIND_DROPS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` in the bind callback.
            drop(unsafe { Box::from_raw(ptr.cast::<BindData>()) });
        }
    }

    pub unsafe extern "C" fn drop_global(ptr: *mut c_void) {
        if !ptr.is_null() {
            GLOBAL_DROPS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` in the global-init callback.
            drop(unsafe { Box::from_raw(ptr.cast::<GlobalState>()) });
        }
    }
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_bind_callback!(my_format_bind, |info| {
    let bind = unsafe { quack_rs::copy_function::CopyBindInfo::new(info) };
    // The COPY options arrive as one STRUCT value, whose field names live on
    // the value's logical type rather than the value itself.
    if let (Some(options), Ok(mut slot)) = (bind.options(), copy_fn::OPTIONS.lock()) {
        slot.clear();
        for (i, name) in options.struct_field_names().into_iter().enumerate() {
            let rendered = options
                .struct_child(i)
                .map(|v| format!("{v:?}"))
                .unwrap_or_default();
            slot.push((name, rendered));
        }
    }
    let data = Box::new(copy_fn::BindData {
        columns: bind.column_count(),
    });
    // SAFETY: the pointer is a fresh Box; `drop_bind` frees it exactly once.
    unsafe {
        bind.set_bind_data(
            Box::into_raw(data).cast::<std::os::raw::c_void>(),
            Some(copy_fn::drop_bind),
        );
    }
});

#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_global_init_callback!(my_format_init, |info| {
    let init = unsafe { quack_rs::copy_function::CopyGlobalInitInfo::new(info) };
    // SAFETY: DuckDB provides the destination path for this COPY.
    let path = unsafe { init.get_file_path() };
    let state = Box::new(copy_fn::GlobalState {
        path,
        rows: 0,
        checksum: 0,
    });
    // SAFETY: the pointer is a fresh Box; `drop_global` frees it exactly once.
    unsafe {
        init.set_global_state(
            Box::into_raw(state).cast::<std::os::raw::c_void>(),
            Some(copy_fn::drop_global),
        );
    }
});

#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_sink_callback!(my_format_sink, |info, chunk| {
    let sink = unsafe { quack_rs::copy_function::CopySinkInfo::new(info) };
    // SAFETY: both were set by the bind and global-init callbacks above.
    let (bind, state) = unsafe {
        (
            &*sink.get_bind_data().cast::<copy_fn::BindData>(),
            &mut *sink.get_global_state().cast::<copy_fn::GlobalState>(),
        )
    };
    assert_eq!(bind.columns, 1, "bind data must survive into the sink");

    let chunk = unsafe { DataChunk::from_raw(chunk) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            state.checksum += unsafe { reader.read_i64(row) };
        }
        state.rows += 1;
    }
});

#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_finalize_callback!(my_format_finalize, |info| {
    use std::sync::atomic::Ordering;
    let fin = unsafe { quack_rs::copy_function::CopyFinalizeInfo::new(info) };
    // SAFETY: set by the global-init callback above.
    let state = unsafe { &*fin.get_global_state().cast::<copy_fn::GlobalState>() };
    assert!(
        state.path.ends_with(".myfmt"),
        "finalize must see the destination path, got {:?}",
        state.path
    );
    copy_fn::FINAL_ROWS.store(state.rows as usize, Ordering::SeqCst);
    copy_fn::FINAL_CHECKSUM.store(state.checksum as usize, Ordering::SeqCst);
});

#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_copy_function_runs_its_whole_lifecycle() {
    use std::sync::atomic::Ordering;

    use quack_rs::copy_function::CopyFunctionBuilder;

    let fx = Fixture::open();

    // SAFETY: `con` is open; all four callbacks have the required signatures.
    unsafe {
        CopyFunctionBuilder::try_new("my_format")
            .expect("name")
            .bind(my_format_bind)
            .global_init(my_format_init)
            .sink(my_format_sink)
            .finalize(my_format_finalize)
            .register(fx.con())
            .expect("register my_format");
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out.myfmt");
    fx.query(&format!(
        "COPY (SELECT i::BIGINT AS n FROM range(5000) t(i)) TO '{}' (FORMAT my_format, LEVEL 7)",
        path.display()
    ));

    // Every COPY option, including FORMAT itself, reaches the bind callback as a
    // field of one STRUCT value.
    let options = copy_fn::OPTIONS.lock().expect("options").clone();
    let names: Vec<&str> = options.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.iter().any(|n| n.eq_ignore_ascii_case("level")),
        "the LEVEL option should be visible, got {names:?}"
    );
    let level = options
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("level"))
        .map(|(_, v)| v.as_str())
        .unwrap_or_default();
    assert!(level.contains('7'), "LEVEL should carry its value: {level}");

    // 5000 rows spans several chunks, so the sink ran repeatedly.
    assert_eq!(copy_fn::FINAL_ROWS.load(Ordering::SeqCst), 5000);
    assert_eq!(
        copy_fn::FINAL_CHECKSUM.load(Ordering::SeqCst),
        (0..5000i64).sum::<i64>() as usize
    );

    // Both destructors must have run exactly once — a leak or a double free
    // here is invisible without counting.
    drop(fx);
    assert_eq!(
        copy_fn::BIND_DROPS.load(Ordering::SeqCst),
        1,
        "bind data destructor"
    );
    assert_eq!(
        copy_fn::GLOBAL_DROPS.load(Ordering::SeqCst),
        1,
        "global state destructor"
    );
}

// ---------------------------------------------------------------------------
// Scalar bind / init / local state (DuckDB 1.5.0+)
// ---------------------------------------------------------------------------

#[cfg(feature = "duckdb-1-5")]
mod scalar_state {
    use std::os::raw::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Constant-folded multiplier, resolved once at bind time.
    pub struct BindData {
        pub factor: i64,
    }
    /// Per-thread scratch, allocated once per execution thread.
    pub struct LocalState {
        pub calls: u64,
    }

    pub static BIND_DROPS: AtomicUsize = AtomicUsize::new(0);
    pub static STATE_DROPS: AtomicUsize = AtomicUsize::new(0);

    pub unsafe extern "C" fn drop_bind(ptr: *mut c_void) {
        if !ptr.is_null() {
            BIND_DROPS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` in the bind callback.
            drop(unsafe { Box::from_raw(ptr.cast::<BindData>()) });
        }
    }

    pub unsafe extern "C" fn drop_state(ptr: *mut c_void) {
        if !ptr.is_null() {
            STATE_DROPS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` in the init callback.
            drop(unsafe { Box::from_raw(ptr.cast::<LocalState>()) });
        }
    }
}

/// Bind callback: fold the constant second argument once, instead of reading it
/// on every row.
#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn scaled_bind(info: libduckdb_sys::duckdb_bind_info) {
    use quack_rs::scalar::ScalarBindInfo;

    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    assert_eq!(bind.argument_count(), 2);
    // SAFETY: argument 1 exists per the assertion above.
    let factor = unsafe { bind.argument(1) }
        .and_then(|expr| {
            if !expr.is_foldable() {
                return None;
            }
            // SAFETY: inside a bind callback, so the context is live.
            let ctx = unsafe { bind.get_client_context() };
            expr.fold(&ctx).ok().map(|v| v.as_i64())
        })
        .unwrap_or(1);

    let data = Box::new(scalar_state::BindData { factor });
    // SAFETY: a fresh Box; `drop_bind` frees it exactly once.
    unsafe {
        bind.set_bind_data(
            Box::into_raw(data).cast::<std::os::raw::c_void>(),
            Some(scalar_state::drop_bind),
        );
    }
}

/// Init callback: allocate per-thread scratch.
#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn scaled_init(info: libduckdb_sys::duckdb_init_info) {
    use quack_rs::scalar::ScalarInitInfo;

    // SAFETY: DuckDB passes a valid init info.
    let init = unsafe { ScalarInitInfo::new(info) };
    let state = Box::new(scalar_state::LocalState { calls: 0 });
    // SAFETY: a fresh Box; `drop_state` frees it exactly once per thread.
    unsafe {
        init.set_state(
            Box::into_raw(state).cast::<std::os::raw::c_void>(),
            Some(scalar_state::drop_state),
        );
    }
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(scaled_exec, |info, input, output| {
    use quack_rs::scalar::ScalarFunctionInfo;

    // SAFETY: DuckDB passes a valid function info, and both were set by the
    // bind and init callbacks above.
    let (factor, state) = unsafe {
        let f = ScalarFunctionInfo::new(info);
        (
            (*f.get_bind_data().cast::<scalar_state::BindData>()).factor,
            &mut *f.get_state().cast::<scalar_state::LocalState>(),
        )
    };
    state.calls += 1;

    // SAFETY: argument 0 is BIGINT and the output vector matches.
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // SAFETY: `row` is within the chunk.
        unsafe { writer.write_i64(row, reader.read_i64(row).wrapping_mul(factor)) };
    }
});

/// Scalar bind data and per-thread local state, threaded through a real query.
///
/// The bind callback constant-folds its second argument — the whole reason
/// `Expression::fold` exists — and both allocations must be freed exactly once.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn scalar_bind_data_and_local_state_survive_a_real_query() {
    use std::sync::atomic::Ordering;

    let fx = Fixture::open();

    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("scaled")
            .expect("name")
            .param(TypeId::BigInt)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(scaled_bind)
            .init(scaled_init)
            .function(scaled_exec)
            .register(fx.con())
            .expect("register scaled");
    }

    // The second argument is a constant, so bind folds it once.
    assert_eq!(
        fx.scalar("SELECT scaled(21, 2)", |r, i| unsafe { r.read_i64(i) }),
        Some(42)
    );
    // Across many rows, and with a folded expression rather than a literal.
    assert_eq!(
        fx.scalar(
            "SELECT sum(scaled(i, 3 * 2)) FROM range(1000) t(i)",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some((0..1000i128).sum::<i128>() * 6)
    );

    drop(fx);
    assert!(
        scalar_state::BIND_DROPS.load(Ordering::SeqCst) >= 2,
        "bind data must be freed once per bind, got {}",
        scalar_state::BIND_DROPS.load(Ordering::SeqCst)
    );
    assert!(
        scalar_state::STATE_DROPS.load(Ordering::SeqCst) >= 2,
        "local state must be freed, got {}",
        scalar_state::STATE_DROPS.load(Ordering::SeqCst)
    );
}

// ---------------------------------------------------------------------------
// Catalog, config options, selection vectors, instance cache
// ---------------------------------------------------------------------------

/// Catalog lookup, which the docs say must happen inside an active
/// transaction — a claim nothing checked.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn catalog_lookup_finds_a_table_inside_a_transaction() {
    use quack_rs::catalog::{CatalogEntry, CatalogEntryType};
    use quack_rs::client_context::ClientContext;

    let fx = Fixture::open();
    fx.query("CREATE TABLE catalog_probe (id INTEGER)");
    fx.query("CREATE VIEW catalog_probe_v AS SELECT * FROM catalog_probe");

    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");

    // `duckdb_catalog_get_entry` requires an active transaction; a plain
    // connection is in auto-commit, so one is started explicitly.
    fx.query("BEGIN TRANSACTION");

    // An empty name is not "the default catalog" — DuckDB returns null for it
    // outright (`strlen(name) == 0` is an explicit early return).
    // SAFETY: inside a transaction.
    assert!(
        unsafe { ctx.catalog(c"") }.is_none(),
        "an empty catalog name is rejected, not defaulted"
    );

    // The in-memory database's catalog is named `memory`.
    // SAFETY: inside a transaction, as the C API requires.
    let catalog = unsafe { ctx.catalog(c"memory") }.expect("the memory catalog");
    assert_eq!(catalog.type_name(), Some("duckdb"));

    // SAFETY: catalog and context are valid and a transaction is active.
    let table = unsafe {
        CatalogEntry::lookup(
            catalog.as_raw(),
            ctx.as_raw(),
            c"main",
            c"catalog_probe",
            CatalogEntryType::Table,
        )
    }
    .expect("the table must be found");
    assert_eq!(table.name(), Some("catalog_probe"));
    assert_eq!(table.entry_type(), CatalogEntryType::Table);

    // SAFETY: as above.
    let view = unsafe {
        CatalogEntry::lookup(
            catalog.as_raw(),
            ctx.as_raw(),
            c"main",
            c"catalog_probe_v",
            CatalogEntryType::View,
        )
    }
    .expect("the view must be found");
    assert_eq!(view.name(), Some("catalog_probe_v"));
    assert_eq!(view.entry_type(), CatalogEntryType::View);

    // A name that does not exist is `None`, not a null handle to dereference.
    // SAFETY: as above.
    assert!(unsafe {
        CatalogEntry::lookup(
            catalog.as_raw(),
            ctx.as_raw(),
            c"main",
            c"no_such_table",
            CatalogEntryType::Table,
        )
    }
    .is_none());

    drop(table);
    drop(view);
    drop(catalog);
    fx.query("COMMIT");
}

/// An extension-defined `SET`/`SELECT current_setting()` option.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_registered_config_option_is_settable_and_readable() {
    use quack_rs::client_context::ClientContext;
    use quack_rs::config_option::ConfigOptionBuilder;

    let fx = Fixture::open();

    // SAFETY: `con` is open.
    unsafe {
        ConfigOptionBuilder::try_new("quack_probe_setting")
            .expect("name")
            .description("A setting registered by the test")
            .expect("description")
            .option_type(TypeId::Varchar)
            .default_value("fallback")
            .expect("default")
            .register(fx.con())
            .expect("register the option");
    }

    assert_eq!(
        fx.scalar("SELECT current_setting('quack_probe_setting')", |r, i| {
            unsafe { r.read_str(i).to_owned() }
        })
        .as_deref(),
        Some("fallback"),
        "the declared default must be what DuckDB reports"
    );

    fx.query("SET quack_probe_setting = 'changed'");
    assert_eq!(
        fx.scalar("SELECT current_setting('quack_probe_setting')", |r, i| {
            unsafe { r.read_str(i).to_owned() }
        })
        .as_deref(),
        Some("changed")
    );

    // And the same value through the client context, which is how a callback
    // would read it.
    // SAFETY: `con` is open.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    assert_eq!(
        ctx.config_option(c"quack_probe_setting").as_deref(),
        Some("changed")
    );

    // `ctx.config_option` on a *missing* option is deliberately not exercised:
    // DuckDB 1.5.5's `duckdb_client_context_get_config_option` calls
    // `TryGetCurrentSetting(...).GetScope()` without first checking the lookup
    // succeeded, and `GetScope()` asserts `scope != SettingScope::INVALID`.
    // Against a DuckDB built with debug assertions — which this test suite uses
    // — that aborts the process. See `ClientContext::config_option`'s docs.
    //
    // The abort-free way to ask whether a setting exists is SQL:
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_settings() WHERE name = 'no_such_setting_at_all'",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(0)
    );
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_settings() WHERE name = 'quack_probe_setting'",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(1),
        "a registered option must appear in duckdb_settings()"
    );
}

/// A selection vector's buffer must be the one `DuckDB` allocated, writable
/// through the slice, and readable back.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_selection_vector_round_trips_its_indices() {
    use quack_rs::selection_vector::SelectionVector;

    let _fx = Fixture::open();

    let mut sel = SelectionVector::new(2048);
    assert_eq!(sel.as_slice().len(), 2048);

    for (i, slot) in sel.as_mut_slice().iter_mut().enumerate() {
        *slot = (2047 - i) as u32;
    }
    assert_eq!(sel.as_slice()[0], 2047);
    assert_eq!(sel.as_slice()[2047], 0);

    // A zero-length vector must not hand out a dangling non-empty slice.
    let empty = SelectionVector::new(0);
    assert!(empty.as_slice().is_empty());
}

/// The instance cache must hand back the *same* database for the same path.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn the_instance_cache_shares_one_database() {
    use quack_rs::instance_cache::InstanceCache;

    let _fx = Fixture::open();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cached.duckdb");
    let c_path = std::ffi::CString::new(path.to_str().expect("utf-8")).expect("no interior NUL");

    let cache = InstanceCache::new();
    // `get_or_create` is safe — it takes a `&CStr` and an `Option<&DbConfig>`,
    // so there is no raw pointer for the caller to get wrong.
    let first = cache.get_or_create(&c_path, None).expect("open");
    let second = cache.get_or_create(&c_path, None).expect("reopen");

    // Written through one handle, visible through the other — that is what
    // "same instance" means, and comparing raw pointers would not prove it.
    let mut con: duckdb_connection = std::ptr::null_mut();
    // SAFETY: `first` is an open database.
    unsafe {
        assert_eq!(
            libduckdb_sys::duckdb_connect(first, &raw mut con),
            DuckDBSuccess
        );
    }
    // SAFETY: `con` is open.
    unsafe {
        quack_rs::query::query(
            con,
            "CREATE TABLE shared (n INTEGER); INSERT INTO shared VALUES (7)",
        )
    }
    .expect("write through the first handle");

    let mut con2: duckdb_connection = std::ptr::null_mut();
    // SAFETY: `second` is an open database.
    unsafe {
        assert_eq!(
            libduckdb_sys::duckdb_connect(second, &raw mut con2),
            DuckDBSuccess
        );
    }
    // SAFETY: `con2` is open.
    let mut result = unsafe { quack_rs::query::query(con2, "SELECT n FROM shared") }
        .expect("read through the second handle");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: INTEGER column, row 0.
    assert_eq!(unsafe { chunk.reader(0).read_i32(0) }, 7);

    drop(chunk);
    drop(result);
    // SAFETY: both connections and databases are live and closed in order.
    unsafe {
        libduckdb_sys::duckdb_disconnect(&raw mut con);
        libduckdb_sys::duckdb_disconnect(&raw mut con2);
        let mut a = first;
        let mut b = second;
        libduckdb_sys::duckdb_close(&raw mut a);
        libduckdb_sys::duckdb_close(&raw mut b);
    }
}

// ---------------------------------------------------------------------------
// Panics in user `Drop` / `Default` must not unwind out of an `extern "C"` fn.
//
// Rust 1.81+ turns such an unwind into `panic_cannot_unwind` — a hard process
// abort, on a DuckDB worker thread, with no way for the host application to
// recover. Before these guards, `SELECT panicky_agg(i) FROM range(10)` killed
// the test binary with SIGABRT from inside
// `duckdb::RowOperations::DestroyStates`.
// ---------------------------------------------------------------------------

mod panicking_lifecycle {
    use quack_rs::aggregate::{AggregateState, FfiState};
    use quack_rs::vector::{VectorReader, VectorWriter};

    /// Counts rows; explodes when dropped.
    #[derive(Default)]
    pub struct DropBomb {
        pub n: i64,
    }

    impl Drop for DropBomb {
        fn drop(&mut self) {
            panic!("state destructor deliberately exploded");
        }
    }

    impl AggregateState for DropBomb {}

    /// Explodes while being constructed.
    pub struct DefaultBomb {
        pub n: i64,
    }

    impl Default for DefaultBomb {
        fn default() -> Self {
            panic!("state initialiser deliberately exploded");
        }
    }

    impl AggregateState for DefaultBomb {}

    macro_rules! bomb_callbacks {
        ($modname:ident, $state:ty) => {
            pub mod $modname {
                use super::*;

                /// # Safety
                /// Called by `DuckDB` with its own valid handles.
                pub unsafe extern "C" fn update(
                    _info: libduckdb_sys::duckdb_function_info,
                    chunk: libduckdb_sys::duckdb_data_chunk,
                    states: *mut libduckdb_sys::duckdb_aggregate_state,
                ) {
                    let reader = unsafe { VectorReader::new(chunk, 0) };
                    for row in 0..reader.row_count() {
                        if let Some(s) =
                            unsafe { FfiState::<$state>::with_state_mut(*states.add(row)) }
                        {
                            s.n += unsafe { reader.read_i64(row) };
                        }
                    }
                }

                /// # Safety
                /// Called by `DuckDB` with its own valid handles.
                pub unsafe extern "C" fn combine(
                    _info: libduckdb_sys::duckdb_function_info,
                    source: *mut libduckdb_sys::duckdb_aggregate_state,
                    target: *mut libduckdb_sys::duckdb_aggregate_state,
                    count: libduckdb_sys::idx_t,
                ) {
                    for i in 0..count as usize {
                        if let (Some(s), Some(t)) = (
                            unsafe { FfiState::<$state>::with_state(*source.add(i)) },
                            unsafe { FfiState::<$state>::with_state_mut(*target.add(i)) },
                        ) {
                            t.n += s.n;
                        }
                    }
                }

                /// # Safety
                /// Called by `DuckDB` with its own valid handles.
                pub unsafe extern "C" fn finalize(
                    _info: libduckdb_sys::duckdb_function_info,
                    states: *mut libduckdb_sys::duckdb_aggregate_state,
                    result: libduckdb_sys::duckdb_vector,
                    count: libduckdb_sys::idx_t,
                    offset: libduckdb_sys::idx_t,
                ) {
                    let mut w = unsafe { VectorWriter::new(result) };
                    for i in 0..count as usize {
                        let idx = i + offset as usize;
                        match unsafe { FfiState::<$state>::with_state(*states.add(i)) } {
                            Some(s) => unsafe { w.write_i64(idx, s.n) },
                            None => unsafe { w.set_null(idx) },
                        }
                    }
                }
            }
        };
    }

    bomb_callbacks!(drop_bomb, DropBomb);
    bomb_callbacks!(default_bomb, DefaultBomb);
}

#[test]
fn a_panicking_aggregate_state_destructor_does_not_abort() {
    use panicking_lifecycle::{drop_bomb, DropBomb};
    use quack_rs::aggregate::{AggregateFunctionBuilder, FfiState};

    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("drop_bomb_agg")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<DropBomb>::size_callback)
            .init(FfiState::<DropBomb>::init_callback)
            .update(drop_bomb::update)
            .combine(drop_bomb::combine)
            .finalize(drop_bomb::finalize)
            .destructor(FfiState::<DropBomb>::destroy_callback)
            .register(fx.con())
            .expect("register drop_bomb_agg");
    }

    // The aggregate itself still computes the right answer: the destructor runs
    // after finalize. DuckDB's state destructor has no error channel at all
    // (`CAPIAggregateDestructor` takes no info and returns nothing), so the
    // panic is contained and discarded rather than reported.
    assert_eq!(
        fx.scalar(
            "SELECT drop_bomb_agg(i) FROM range(10) t(i)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(45)
    );

    // Reaching this line at all is the assertion that matters: before the fix
    // the process died with SIGABRT partway through the query above.
    assert_eq!(
        fx.scalar("SELECT 5::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

#[test]
fn a_panicking_aggregate_state_initialiser_becomes_a_sql_error() {
    use panicking_lifecycle::{default_bomb, DefaultBomb};
    use quack_rs::aggregate::{AggregateFunctionBuilder, FfiState};

    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        AggregateFunctionBuilder::new("default_bomb_agg")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<DefaultBomb>::size_callback)
            .init(FfiState::<DefaultBomb>::init_callback)
            .update(default_bomb::update)
            .combine(default_bomb::combine)
            .finalize(default_bomb::finalize)
            .destructor(FfiState::<DefaultBomb>::destroy_callback)
            .register(fx.con())
            .expect("register default_bomb_agg");
    }

    // `CAPIAggregateStateInit` *does* check the error flag and throws, so unlike
    // the destructor this one surfaces to the user with its payload intact.
    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT default_bomb_agg(i) FROM range(10) t(i)") }
        .expect_err("the panic must surface as an error");
    assert!(
        err.as_str().contains("deliberately exploded"),
        "panic payload should reach the user: {err}"
    );

    // The connection survives.
    assert_eq!(
        fx.scalar("SELECT 5::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

#[test]
fn a_panicking_bind_state_destructor_does_not_abort() {
    use quack_rs::table::{BindInfo, TableFunctionBuilder};

    struct BindDropBomb;
    impl Drop for BindDropBomb {
        fn drop(&mut self) {
            panic!("bind-state destructor deliberately exploded");
        }
    }

    let fx = Fixture::open();
    let builder = TableFunctionBuilder::new("bind_bomb")
        .with_state::<BindDropBomb, _>(|bind: &BindInfo| {
            bind.add_result_column("n", TypeId::BigInt);
            Ok(BindDropBomb)
        })
        .scan(|_state, chunk| {
            // Emit nothing: the point is the teardown, not the data.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build()
        .expect("build bind_bomb");
    // SAFETY: `con` is open.
    unsafe { builder.register(fx.con()) }.expect("register bind_bomb");

    // SAFETY: `con` is open.
    let mut result =
        unsafe { query(fx.con(), "SELECT count(*) FROM bind_bomb()") }.expect("scan runs");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: BIGINT column, row 0.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 0);
    drop(chunk);
    drop(result);

    // The bind/init state is dropped at query teardown; before the fix that
    // unwound out of `duckdb_delete_callback_t` and aborted.
    assert_eq!(
        fx.scalar("SELECT 5::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

#[test]
fn composite_type_ids_are_rejected_where_a_primitive_is_required() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();

    unsafe extern "C" fn never_called(
        _: libduckdb_sys::duckdb_function_info,
        _: libduckdb_sys::duckdb_data_chunk,
        _: libduckdb_sys::duckdb_vector,
    ) {
    }

    // `duckdb_create_logical_type` returns a *non-null* handle wrapping
    // LogicalTypeId::INVALID for these, so a null check never fires. Registration
    // used to fail with "duckdb_register_scalar_function failed", naming neither
    // the parameter nor the fix.
    // SAFETY: `con` is open.
    let err = unsafe {
        ScalarFunctionBuilder::new("composite_param")
            .param(TypeId::Struct)
            .returns(TypeId::BigInt)
            .function(never_called)
            .register(fx.con())
    }
    .expect_err("a STRUCT parameter must be rejected");
    let msg = err.as_str();
    assert!(msg.contains("parameter 0"), "names the slot: {msg}");
    assert!(msg.contains("STRUCT"), "names the type: {msg}");
    assert!(
        msg.contains("LogicalType::struct_type"),
        "names the fix: {msg}"
    );

    // SAFETY: `con` is open.
    let err = unsafe {
        ScalarFunctionBuilder::new("composite_return")
            .param(TypeId::BigInt)
            .returns(TypeId::Decimal)
            .function(never_called)
            .register(fx.con())
    }
    .expect_err("a bare DECIMAL return type must be rejected");
    assert!(
        err.as_str().contains("LogicalType::decimal"),
        "names the fix: {err}"
    );

    // The `*_logical` route still works, which is the point of the message.
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::new("composite_ok")
            .param_logical(LogicalType::struct_type(&[("x", TypeId::BigInt)]))
            .returns_logical(LogicalType::decimal(18, 3))
            .function(never_called)
            .register(fx.con())
    }
    .expect("the logical-type route registers");
}

// ---------------------------------------------------------------------------
// Values, parameter binding, streaming, cancellation.
//
// Everything below goes through a real DuckDB: a value is built with the Rust
// API, bound to a prepared statement, and read back out of the result, so a
// wrong physical encoding shows up as a wrong answer rather than as a passing
// mock.
// ---------------------------------------------------------------------------

/// Binds one value into `SELECT ?` and renders the answer as text, which is the
/// one representation every type shares.
fn round_trip_value(fx: &Fixture, value: &quack_rs::value::Value) -> String {
    // SAFETY: `con` is open.
    let stmt = unsafe { quack_rs::query::prepare(fx.con(), "SELECT CAST(? AS VARCHAR)") }
        .expect("prepare SELECT ?");
    stmt.bind_value(1, value).expect("bind_value");
    let mut result = stmt.execute().expect("execute");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: one VARCHAR column, one row.
    let reader = unsafe { chunk.reader(0) };
    // SAFETY: row 0 exists.
    assert!(
        unsafe { reader.is_valid(0) },
        "the bound value came back NULL"
    );
    // SAFETY: VARCHAR column, row 0.
    unsafe { reader.read_str(0) }.to_owned()
}

#[test]
fn every_scalar_value_constructor_round_trips_through_a_bound_parameter() {
    use quack_rs::interval::DuckInterval;
    use quack_rs::value::Value;

    let fx = Fixture::open();

    let cases: Vec<(Value, &str)> = vec![
        (Value::boolean(true), "true"),
        (Value::tinyint(i8::MIN), "-128"),
        (Value::smallint(i16::MIN), "-32768"),
        (Value::integer(i32::MIN), "-2147483648"),
        (Value::bigint(i64::MIN), "-9223372036854775808"),
        (Value::utinyint(u8::MAX), "255"),
        (Value::usmallint(u16::MAX), "65535"),
        (Value::uinteger(u32::MAX), "4294967295"),
        (Value::ubigint(u64::MAX), "18446744073709551615"),
        (
            Value::hugeint(i128::from(i64::MIN) * 2),
            "-18446744073709551616",
        ),
        (
            Value::uhugeint(u128::from(u64::MAX) + 1),
            "18446744073709551616",
        ),
        (Value::float(0.5), "0.5"),
        (Value::double(0.25), "0.25"),
        (Value::varchar("héllo"), "héllo"),
        (Value::date(0), "1970-01-01"),
        (Value::time(3_600_000_000), "01:00:00"),
        (Value::timestamp(0), "1970-01-01 00:00:00"),
        (Value::timestamp_s(60), "1970-01-01 00:01:00"),
        (Value::timestamp_ms(1_500), "1970-01-01 00:00:01.5"),
        (Value::timestamp_ns(1_500_000_000), "1970-01-01 00:00:01.5"),
        (
            Value::interval(DuckInterval {
                months: 1,
                days: 2,
                micros: 3_000_000,
            }),
            "1 month 2 days 00:00:03",
        ),
        (
            Value::uuid(0x1111_1111_2222_3333_4444_5555_5555_5555),
            "11111111-2222-3333-4444-555555555555",
        ),
    ];

    for (value, expected) in cases {
        let type_id = value.type_id();
        assert_eq!(
            round_trip_value(&fx, &value),
            expected,
            "value of type {type_id:?} did not survive the round trip"
        );
    }

    // DECIMAL carries width and scale, so it gets its own shape.
    let dec = Value::decimal(18, 3, 1_234).expect("DECIMAL(18, 3)");
    assert_eq!(round_trip_value(&fx, &dec), "1.234");
    assert!(
        Value::decimal(39, 3, 0).is_err(),
        "width 39 exceeds DuckDB's DECIMAL limit and must be reported"
    );
    assert!(
        Value::decimal(4, 9, 0).is_err(),
        "scale above width must be reported"
    );

    // BLOB is byte-exact where VARCHAR is not.
    let blob = Value::blob(&[0x00, 0xff, 0x41]);
    assert_eq!(round_trip_value(&fx, &blob), r"\x00\xFFA");

    // SQL NULL has a perfectly good handle: `is_null` and `is_sql_null` are
    // different questions and the API must not conflate them.
    let null = Value::null_value();
    assert!(!null.is_null(), "the handle is valid");
    assert!(null.is_sql_null(), "the value is SQL NULL");
    assert!(!Value::bigint(1).is_sql_null());
}

#[test]
fn composite_value_constructors_build_values_duckdb_accepts() {
    use quack_rs::value::Value;

    let fx = Fixture::open();

    // The constructors take the *element* type: duckdb.h's prose says "child
    // (element) type" while its @param line says "the type of the list", and
    // the implementation (`Value::LIST(child_type, values)`) settles it.
    let element_ty = LogicalType::new(TypeId::BigInt);
    let list = Value::list_value(&element_ty, &[Value::bigint(1), Value::bigint(2)])
        .expect("build a LIST value");
    assert_eq!(round_trip_value(&fx, &list), "[1, 2]");

    // Passing the LIST type is the mistake the header invites; DuckDB reports
    // it as a bare null, so quack-rs names it.
    let list_ty = LogicalType::list(TypeId::BigInt);
    let err = Value::list_value(&list_ty, &[Value::bigint(1)])
        .expect_err("a LIST type where an element type belongs must be refused");
    assert!(err.as_str().contains("element"), "{err}");

    let struct_ty = LogicalType::struct_type(&[("x", TypeId::BigInt), ("y", TypeId::Varchar)]);
    let strct = Value::struct_value(&struct_ty, &[Value::bigint(7), Value::varchar("seven")])
        .expect("build a STRUCT value");
    assert_eq!(round_trip_value(&fx, &strct), "{'x': 7, 'y': seven}");

    // ARRAY derives its size from the value count, so there is nothing to keep
    // in sync -- and again it is the element type that goes in.
    let int_ty = LogicalType::new(TypeId::Integer);
    let array = Value::array_value(
        &int_ty,
        &[Value::integer(1), Value::integer(2), Value::integer(3)],
    )
    .expect("build an ARRAY value");
    assert_eq!(round_trip_value(&fx, &array), "[1, 2, 3]");

    let enum_ty = LogicalType::enum_type(&["red", "green", "blue"]);
    let green = Value::enum_value(&enum_ty, 1).expect("build an ENUM value");
    assert_eq!(round_trip_value(&fx, &green), "green");
    assert_eq!(green.as_enum_index(), 1);

    // A short field slice must be refused rather than passed on:
    // `duckdb_create_struct_value` takes no count and reads one value per field
    // in the *type*, so a short slice reads past its end.
    let err = Value::struct_value(&struct_ty, &[Value::bigint(7)])
        .expect_err("a one-value slice for a two-field struct must be refused");
    assert!(err.as_str().contains("2 field"), "{err}");
    assert!(err.as_str().contains("out of bounds"), "{err}");

    // A value that will not cast to the element type is DuckDB's own check,
    // surfaced as an error rather than a Value with a null handle.
    assert!(
        Value::array_value(&struct_ty, &[Value::integer(1)]).is_err(),
        "an INTEGER cannot become a STRUCT element"
    );
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn map_and_union_values_build_and_read_back() {
    use quack_rs::value::Value;

    let fx = Fixture::open();

    let map_ty = LogicalType::map(TypeId::Varchar, TypeId::BigInt);
    let map = Value::map(
        &map_ty,
        &[Value::varchar("a"), Value::varchar("b")],
        &[Value::bigint(1), Value::bigint(2)],
    )
    .expect("build a MAP value");
    assert_eq!(round_trip_value(&fx, &map), "{a=1, b=2}");
    assert!(
        Value::map(&map_ty, &[Value::varchar("a")], &[]).is_err(),
        "mismatched key/value counts must be refused"
    );

    let union_ty = LogicalType::union_type(&[("num", TypeId::BigInt), ("txt", TypeId::Varchar)]);
    let tagged = Value::union_value(&union_ty, 1, &Value::varchar("hi")).expect("build a UNION");
    assert_eq!(round_trip_value(&fx, &tagged), "hi");
    assert!(
        Value::union_value(&union_ty, 1, &Value::bigint(1)).is_err(),
        "DuckDB compares the value's type to the member's exactly"
    );
    assert!(
        Value::union_value(&union_ty, 9, &Value::bigint(1)).is_err(),
        "an out-of-range tag must be refused"
    );
}

#[test]
fn every_typed_bind_reaches_duckdb_with_the_right_width() {
    use quack_rs::interval::DuckInterval;

    let fx = Fixture::open();

    /// Binds through `bind`, then asserts `SELECT CAST(? AS VARCHAR)` renders
    /// `expected`.
    fn check(
        fx: &Fixture,
        expected: &str,
        bind: impl Fn(
            &quack_rs::query::PreparedStatement,
        ) -> Result<(), quack_rs::error::ExtensionError>,
    ) {
        // SAFETY: `con` is open.
        let stmt = unsafe { quack_rs::query::prepare(fx.con(), "SELECT CAST(? AS VARCHAR)") }
            .expect("prepare");
        bind(&stmt).expect("bind");
        let mut result = stmt.execute().expect("execute");
        let chunk = result.next_chunk().expect("one chunk");
        // SAFETY: one VARCHAR column, one row.
        let got = unsafe { chunk.reader(0).read_str(0) }.to_owned();
        assert_eq!(got, expected);
    }

    check(&fx, "-128", |s| s.bind_i8(1, i8::MIN));
    check(&fx, "-32768", |s| s.bind_i16(1, i16::MIN));
    check(&fx, "-2147483648", |s| s.bind_i32(1, i32::MIN));
    check(&fx, "-9223372036854775808", |s| s.bind_i64(1, i64::MIN));
    check(&fx, "255", |s| s.bind_u8(1, u8::MAX));
    check(&fx, "65535", |s| s.bind_u16(1, u16::MAX));
    check(&fx, "4294967295", |s| s.bind_u32(1, u32::MAX));
    check(&fx, "18446744073709551615", |s| s.bind_u64(1, u64::MAX));
    check(&fx, "0.5", |s| s.bind_f32(1, 0.5));
    check(&fx, "0.25", |s| s.bind_f64(1, 0.25));
    check(&fx, "-18446744073709551616", |s| {
        s.bind_i128(1, i128::from(i64::MIN) * 2)
    });
    check(&fx, "18446744073709551616", |s| {
        s.bind_u128(1, u128::from(u64::MAX) + 1)
    });
    check(&fx, "1.234", |s| s.bind_decimal(1, 18, 3, 1_234));
    check(&fx, "1970-01-01", |s| s.bind_date(1, 0));
    check(&fx, "01:00:00", |s| s.bind_time(1, 3_600_000_000));
    check(&fx, "1970-01-01 00:00:00", |s| s.bind_timestamp(1, 0));
    check(&fx, "1 month 2 days 00:00:03", |s| {
        s.bind_interval(
            1,
            DuckInterval {
                months: 1,
                days: 2,
                micros: 3_000_000,
            },
        )
    });
}

#[test]
fn column_logical_type_keeps_what_column_type_throws_away() {
    let fx = Fixture::open();

    let result = fx.query("SELECT {'a': 1::BIGINT, 'b': 'x'} AS s, 1.25::DECIMAL(9, 4) AS d");

    // The collapsed view: correct, but a STRUCT is just "Struct".
    assert_eq!(result.column_type(0), Some(TypeId::Struct));
    assert_eq!(result.column_type(1), Some(TypeId::Decimal));

    let s_ty = result.column_logical_type(0).expect("STRUCT logical type");
    // SAFETY: `s_ty` is a live STRUCT logical type.
    unsafe {
        assert_eq!(s_ty.struct_child_count(), 2);
        assert_eq!(s_ty.struct_child_name(0), "a");
        assert_eq!(s_ty.struct_child_type(0).get_type_id(), TypeId::BigInt);
        assert_eq!(s_ty.struct_child_name(1), "b");
        assert_eq!(s_ty.struct_child_type(1).get_type_id(), TypeId::Varchar);
    }

    let d_ty = result.column_logical_type(1).expect("DECIMAL logical type");
    // SAFETY: `d_ty` is a live DECIMAL logical type.
    unsafe {
        assert_eq!(d_ty.decimal_width(), 9);
        assert_eq!(d_ty.decimal_scale(), 4);
    }

    assert!(result.column_logical_type(99).is_none());
}

#[test]
fn result_kind_distinguishes_rows_from_row_counts() {
    use quack_rs::query::ResultKind;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE k(i INTEGER)") }.expect("create");

    assert_eq!(fx.query("SELECT 1").result_kind(), ResultKind::Rows);
    // SAFETY: `con` is open.
    let inserted = unsafe { query(fx.con(), "INSERT INTO k VALUES (1), (2)") }.expect("insert");
    assert_eq!(inserted.result_kind(), ResultKind::ChangedRows);
    assert_eq!(inserted.rows_changed(), 2);
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_streaming_result_reads_the_same_rows_as_a_materialised_one() {
    let fx = Fixture::open();

    // Enough rows to span several chunks, so the streaming path actually loops.
    const SQL: &str = "SELECT i FROM range(10000) t(i)";

    // SAFETY: `con` is open.
    let stmt = unsafe { quack_rs::query::prepare(fx.con(), SQL) }.expect("prepare");
    let mut streamed = stmt.execute_streaming().expect("execute_streaming");
    assert!(
        streamed.is_streaming(),
        "DuckDB chose to materialise a plain range scan; the assertions below \
         would then prove nothing about the streaming path"
    );

    let mut sum: i64 = 0;
    let mut chunks = 0;
    while let Some(chunk) = streamed.next_chunk() {
        chunks += 1;
        // SAFETY: one BIGINT column.
        let reader = unsafe { chunk.reader(0) };
        for row in 0..chunk.size() {
            // SAFETY: `row` is in range and the column is BIGINT.
            sum += unsafe { reader.read_i64(row) };
        }
    }
    assert!(chunks > 1, "a 10000-row result must span several chunks");
    assert_eq!(sum, (0..10_000_i64).sum::<i64>());

    // The materialised path must agree.
    // SAFETY: `con` is open.
    let stmt2 = unsafe { quack_rs::query::prepare(fx.con(), SQL) }.expect("prepare");
    let mut materialised = stmt2.execute().expect("execute");
    assert!(!materialised.is_streaming());
    let mut sum2: i64 = 0;
    while let Some(chunk) = materialised.next_chunk() {
        // SAFETY: one BIGINT column.
        let reader = unsafe { chunk.reader(0) };
        for row in 0..chunk.size() {
            // SAFETY: `row` is in range and the column is BIGINT.
            sum2 += unsafe { reader.read_i64(row) };
        }
    }
    assert_eq!(sum, sum2);
}

#[test]
fn an_interrupt_handle_cancels_a_query_from_another_thread() {
    use quack_rs::query::OwnedConnection;

    let fx = Fixture::open();
    // SAFETY: `db` is open for the fixture's lifetime.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("open an owned connection");

    let handle = con.interrupt_handle();
    // A cross join over a billion rows cannot complete before the canceller
    // fires, so this is a race the test always wins.
    let outcome = std::thread::scope(|scope| {
        scope.spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            handle.cancel();
        });
        con.query("SELECT count(*) FROM range(1000000000000) t(i) WHERE i % 7 = 3")
    });

    let err = outcome.expect_err("the query must be cancelled, not completed");
    assert!(
        err.as_str().to_lowercase().contains("interrupt"),
        "the error should say the query was interrupted: {err}"
    );

    // The connection is reusable afterwards: interruption is not corruption.
    let mut ok = con
        .query("SELECT 42::BIGINT")
        .expect("the connection survives");
    let chunk = ok.next_chunk().expect("one chunk");
    // SAFETY: one BIGINT column, one row.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 42);
}

// ---------------------------------------------------------------------------
// NULL propagation, and the typed scalar API built on top of it.
// ---------------------------------------------------------------------------

// Writes 999 into every row and never looks at input validity.
quack_rs::scalar_callback!(always_999, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 999) };
    }
});

// The same, followed by `propagate_nulls`.
quack_rs::scalar_callback!(always_999_propagating, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 999) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

#[test]
fn default_null_handling_does_not_propagate_nulls_for_scalar_functions() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::new("raw_999")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::DefaultNullHandling)
            .function(always_999)
            .register(fx.con())
            .expect("register raw_999");
        ScalarFunctionBuilder::new("propagating_999")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::DefaultNullHandling)
            .function(always_999_propagating)
            .register(fx.con())
            .expect("register propagating_999");
    }

    // A *literal* NULL is constant-folded before the function is reached, so the
    // obvious spot check passes even for the broken function. This is why the
    // bug is easy to ship.
    assert_eq!(
        fx.scalar("SELECT raw_999(NULL::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        None,
        "a literal NULL argument is constant-folded to NULL"
    );

    // From a real column it is a different story. This assertion pins DuckDB's
    // actual behaviour (verified against 1.5.4): DEFAULT_NULL_HANDLING is a
    // promise the *function* makes, enforced only by a debug-build assertion in
    // `VerifyNullHandling`. If a future DuckDB starts propagating, this test
    // fails and the documentation gets revisited.
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE nulls_t(i BIGINT)") }.expect("create");
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "INSERT INTO nulls_t VALUES (1), (NULL), (3)") }.expect("insert");

    assert_eq!(
        fx.scalar("SELECT count(raw_999(i)) FROM nulls_t", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(3),
        "DuckDB does not NULL out the result for the NULL row: all three are non-NULL"
    );

    // `propagate_nulls` restores SQL semantics.
    assert_eq!(
        fx.scalar("SELECT count(propagating_999(i)) FROM nulls_t", |r, i| {
            unsafe { r.read_i64(i) }
        }),
        Some(2),
        "with propagate_nulls the NULL row yields NULL"
    );
}

#[test]
fn typed_scalar_closures_cover_the_common_shapes() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();

    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_double", |x: i64| x * 2)
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map2("t_add", |a: i64, b: i64| a + b)
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map1("t_half", |x: f64| x / 2.0)
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map1_str("t_len", |s: &str| s.chars().count() as i64)
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map1_str("t_shout", |s: &str| s.to_uppercase())
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map2_str("t_join", |a: &str, b: &str| format!("{a}|{b}"))
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map1_opt("t_or_zero", |x: Option<i64>| Some(x.unwrap_or(0)))
            .expect("build")
            .register(fx.con())
            .expect("register");
        ScalarFunctionBuilder::map2_opt("t_coalesce", |a: Option<i64>, b: Option<i64>| a.or(b))
            .expect("build")
            .register(fx.con())
            .expect("register");
    }

    assert_eq!(
        fx.scalar("SELECT t_double(21::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );
    assert_eq!(
        fx.scalar("SELECT t_add(20::BIGINT, 22::BIGINT)", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(42)
    );
    assert!(
        (fx.scalar("SELECT t_half(9.0::DOUBLE)", |r, i| unsafe {
            r.read_f64(i)
        })
        .expect("non-NULL")
            - 4.5)
            .abs()
            < f64::EPSILON
    );
    assert_eq!(
        fx.scalar("SELECT t_len('héllo')", |r, i| unsafe { r.read_i64(i) }),
        Some(5),
        "the closure sees a &str borrowed from the vector, not bytes"
    );
    assert_eq!(
        fx.scalar("SELECT t_shout('quack')", |r, i| unsafe {
            r.read_str(i).to_owned()
        }),
        Some("QUACK".to_owned())
    );
    assert_eq!(
        fx.scalar("SELECT t_join('a', 'b')", |r, i| unsafe {
            r.read_str(i).to_owned()
        }),
        Some("a|b".to_owned())
    );

    // NULL propagation, from a real column rather than a folded literal.
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE typed_t(i BIGINT, s VARCHAR)") }.expect("create");
    // SAFETY: `con` is open.
    unsafe {
        query(
            fx.con(),
            "INSERT INTO typed_t VALUES (1, 'a'), (NULL, NULL), (3, 'c')",
        )
    }
    .expect("insert");

    assert_eq!(
        fx.scalar("SELECT count(t_double(i)) FROM typed_t", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(2),
        "map1 must NULL out the row whose argument is NULL"
    );
    assert_eq!(
        fx.scalar("SELECT count(t_len(s)) FROM typed_t", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(2),
        "map1_str must NULL out the row whose argument is NULL"
    );
    assert_eq!(
        fx.scalar(
            "SELECT count(t_add(i, 1::BIGINT)) FROM typed_t",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(2),
        "map2 must NULL out a row where *either* argument is NULL"
    );
    assert_eq!(
        fx.scalar("SELECT count(t_join(s, 'z')) FROM typed_t", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(2),
        "map2_str must NULL out a row where *either* argument is NULL"
    );

    // The `_opt` variants see the NULLs instead.
    assert_eq!(
        fx.scalar("SELECT sum(t_or_zero(i)) FROM typed_t", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(4),
        "map1_opt is called for the NULL row and substitutes 0"
    );
    assert_eq!(
        fx.scalar(
            "SELECT t_coalesce(NULL::BIGINT, 7::BIGINT)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(7)
    );
}

#[test]
fn a_panicking_typed_closure_becomes_a_sql_error() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_boom", |x: i64| {
            assert!(x != 13, "closure deliberately exploded");
            x
        })
        .expect("build")
        .register(fx.con())
        .expect("register");
    }

    assert_eq!(
        fx.scalar("SELECT t_boom(1::BIGINT)", |r, i| unsafe { r.read_i64(i) }),
        Some(1)
    );

    // SAFETY: `con` is open.
    let err = unsafe { query(fx.con(), "SELECT t_boom(13::BIGINT)") }
        .expect_err("the panic must surface as an error");
    assert!(
        err.as_str().contains("deliberately exploded"),
        "panic payload should reach the user: {err}"
    );

    // The connection survives.
    assert_eq!(
        fx.scalar("SELECT 5::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

#[test]
fn typed_closures_run_once_per_chunk_over_a_multi_chunk_scan() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        ScalarFunctionBuilder::map1("t_inc", |x: i64| x + 1)
            .expect("build")
            .register(fx.con())
            .expect("register");
    }

    // 10000 rows spans several chunks, so the per-chunk executor is exercised
    // repeatedly and every row index in a full vector is written.
    assert_eq!(
        fx.scalar("SELECT sum(t_inc(i)) FROM range(10000) t(i)", |r, i| {
            unsafe { r.read_i128(i) }
        }),
        Some((1..=10_000_i128).sum::<i128>())
    );
}

#[test]
fn a_custom_logical_type_is_registered_and_usable_from_sql() {
    let fx = Fixture::open();

    let mood = LogicalType::enum_type(&["sad", "ok", "happy"]);
    // SAFETY: the type is live, and `con` is open.
    unsafe {
        mood.set_alias("mood");
        mood.register(fx.con()).expect("register the type");
    }

    // The alias now names a real catalog type.
    assert_eq!(
        fx.scalar("SELECT 'happy'::mood::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        }),
        Some("happy".to_owned())
    );
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE m(v mood)") }.expect("use it as a column type");
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "INSERT INTO m VALUES ('ok'), ('sad')") }.expect("insert");
    assert_eq!(
        fx.scalar("SELECT count(*) FROM m WHERE v = 'ok'", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(1)
    );

    // A type with no alias has no name to register under, and the error says so
    // rather than repeating DuckDB's bare failure code.
    let anonymous = LogicalType::enum_type(&["a", "b"]);
    // SAFETY: `con` is open.
    let err = unsafe { anonymous.register(fx.con()) }.expect_err("no alias, no registration");
    assert!(err.as_str().contains("no alias"), "{err}");

    // Registering the same name twice is a catalog conflict, reported with the
    // name in it.
    let again = LogicalType::enum_type(&["x"]);
    // SAFETY: the type is live, and `con` is open.
    let err = unsafe {
        again.set_alias("mood");
        again.register(fx.con())
    }
    .expect_err("the name is taken");
    assert!(err.as_str().contains("mood"), "{err}");
}

// ---------------------------------------------------------------------------
// Nested types as scalar-function *input*.
//
// `list_builder_*` above cover writing LIST and MAP. Reading them back is the
// other half, and it is the half that does raw offset arithmetic against a
// layout only DuckDB defines: a LIST row is a `{offset, length}` into one flat
// child vector shared by the whole chunk, and those offsets are not `row * len`
// for anything but a uniform column. A mock cannot catch a mistake here.
// ---------------------------------------------------------------------------

quack_rs::scalar_callback!(struct_to_text, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.struct_reader(0, 2) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let n = unsafe { reader.read_i64(row, 0) };
        let s = unsafe { reader.read_str(row, 1) };
        unsafe { writer.write_varchar(row, &format!("{n}:{s}")) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

quack_rs::scalar_callback!(list_sum, |_info, input, output| {
    use quack_rs::vector::complex::ListVector;
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let vector = unsafe { chunk.vector(0) };
    // One child reader for the whole chunk: every row's elements live in the
    // same flat child vector, addressed by that row's offset.
    let total = unsafe { ListVector::get_size(vector) };
    let child = unsafe { ListVector::child_reader(vector, total) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let entry = unsafe { ListVector::get_entry(vector, row) };
        let mut sum = 0_i64;
        for i in 0..entry.length as usize {
            let idx = entry.offset as usize + i;
            if unsafe { child.is_valid(idx) } {
                sum += unsafe { child.read_i64(idx) };
            }
        }
        unsafe { writer.write_i64(row, sum) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

quack_rs::scalar_callback!(map_lookup, |_info, input, output| {
    use quack_rs::vector::complex::MapVector;
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let vector = unsafe { chunk.vector(0) };
    let wanted = unsafe { chunk.reader(1) };
    let total = unsafe { MapVector::total_entry_count(vector) };
    let keys = unsafe { MapVector::key_reader(vector, total) };
    let values = unsafe { MapVector::value_reader(vector, total) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let entry = unsafe { MapVector::get_entry(vector, row) };
        let key = unsafe { wanted.read_str(row) };
        let mut found = None;
        for i in 0..entry.length as usize {
            let idx = entry.offset as usize + i;
            if unsafe { keys.read_str(idx) } == key {
                found = Some(unsafe { values.read_i64(idx) });
                break;
            }
        }
        match found {
            Some(v) => unsafe { writer.write_i64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
});

quack_rs::scalar_callback!(array_sum, |_info, input, output| {
    use quack_rs::vector::complex::ArrayVector;
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let vector = unsafe { chunk.vector(0) };
    // An ARRAY's child vector has `parent_rows * array_size` elements laid out
    // contiguously, so row `r`'s elements start at `r * SIZE`. There is no
    // offset table: that is the difference from LIST.
    const SIZE: usize = 3;
    let child = unsafe { ArrayVector::get_child(vector) };
    let reader = unsafe { VectorReader::from_vector(child, rows * SIZE) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..rows {
        let mut sum = 0_i32;
        for i in 0..SIZE {
            sum += unsafe { reader.read_i32(row * SIZE + i) };
        }
        unsafe { writer.write_i32(row, sum) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

#[test]
fn nested_types_read_correctly_as_scalar_arguments() {
    let fx = Fixture::open();

    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::new("q_struct_to_text")
            .param_logical(LogicalType::struct_type(&[
                ("n", TypeId::BigInt),
                ("s", TypeId::Varchar),
            ]))
            .returns(TypeId::Varchar)
            .function(struct_to_text)
            .register(fx.con())
            .expect("register q_struct_to_text");
        ScalarFunctionBuilder::new("q_list_sum")
            .param_logical(LogicalType::list(TypeId::BigInt))
            .returns(TypeId::BigInt)
            .function(list_sum)
            .register(fx.con())
            .expect("register q_list_sum");
        ScalarFunctionBuilder::new("q_map_lookup")
            .param_logical(LogicalType::map(TypeId::Varchar, TypeId::BigInt))
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .null_handling(NullHandling::SpecialNullHandling)
            .function(map_lookup)
            .register(fx.con())
            .expect("register q_map_lookup");
        ScalarFunctionBuilder::new("q_array_sum")
            .param_logical(LogicalType::array(TypeId::Integer, 3))
            .returns(TypeId::Integer)
            .function(array_sum)
            .register(fx.con())
            .expect("register q_array_sum");
    }

    assert_eq!(
        fx.scalar(
            "SELECT q_struct_to_text({'n': 7::BIGINT, 's': 'x'})",
            |r, i| { unsafe { r.read_str(i).to_owned() } }
        ),
        Some("7:x".to_owned())
    );

    // Variable-length lists in one chunk: row offsets are cumulative, not
    // `row * length`, so a reader that assumed uniform stride would be wrong for
    // every row after the first.
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE lists(l BIGINT[])") }.expect("create");
    // SAFETY: `con` is open.
    unsafe {
        query(
            fx.con(),
            "INSERT INTO lists VALUES ([1,2,3]), ([]), ([10]), ([100,200]), (NULL)",
        )
    }
    .expect("insert");

    let mut result = fx.query("SELECT q_list_sum(l) AS s FROM lists");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: one BIGINT column with five rows.
    let reader = unsafe { chunk.reader(0) };
    let got: Vec<Option<i64>> = (0..chunk.size())
        // SAFETY: `row` is in range.
        .map(|row| unsafe { reader.is_valid(row).then(|| reader.read_i64(row)) })
        .collect();
    assert_eq!(got, vec![Some(6), Some(0), Some(10), Some(300), None]);
    drop(chunk);
    drop(result);

    // A list containing NULL elements: the child vector's validity is what
    // decides, not the parent's.
    assert_eq!(
        fx.scalar("SELECT q_list_sum([1, NULL, 3]::BIGINT[])", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(4)
    );

    // MAP: keys and values are two parallel child vectors behind one offset
    // table, exactly like LIST.
    assert_eq!(
        fx.scalar(
            "SELECT q_map_lookup(MAP{'a':1,'b':2}, 'b')",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(2)
    );
    assert_eq!(
        fx.scalar("SELECT q_map_lookup(MAP{'a':1}, 'zz')", |r, i| unsafe {
            r.read_i64(i)
        }),
        None,
        "a missing key must produce NULL, not a stale value"
    );

    // ARRAY: fixed stride, no offset table.
    assert_eq!(
        fx.scalar("SELECT q_array_sum([1,2,3]::INTEGER[3])", |r, i| unsafe {
            r.read_i32(i)
        }),
        Some(6)
    );
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "CREATE TABLE arrs(a INTEGER[3])") }.expect("create");
    // SAFETY: `con` is open.
    unsafe { query(fx.con(), "INSERT INTO arrs VALUES ([1,2,3]), ([10,20,30])") }.expect("insert");
    assert_eq!(
        fx.scalar("SELECT sum(q_array_sum(a)) FROM arrs", |r, i| unsafe {
            r.read_i128(i)
        }),
        Some(66),
        "the second row must read elements 3..6 of the child, not 0..3"
    );
}

quack_rs::scalar_callback!(split_pair, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let rows = chunk.size();
    let reader = unsafe { chunk.reader(0) };
    let mut n = unsafe { quack_rs::vector::complex::StructVector::field_writer(output, 0) };
    let mut s = unsafe { quack_rs::vector::complex::StructVector::field_writer(output, 1) };
    for row in 0..rows {
        let text = unsafe { reader.read_str(row) };
        let (head, tail) = text.split_once(':').unwrap_or((text, ""));
        unsafe { n.write_i64(row, head.parse::<i64>().unwrap_or(-1)) };
        unsafe { s.write_varchar(row, tail) };
    }
});

#[test]
fn a_struct_output_vector_is_written_field_by_field() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::new("q_split_pair")
            .param(TypeId::Varchar)
            .returns_logical(LogicalType::struct_type(&[
                ("n", TypeId::BigInt),
                ("s", TypeId::Varchar),
            ]))
            .function(split_pair)
            .register(fx.con())
            .expect("register q_split_pair");
    }

    assert_eq!(
        fx.scalar("SELECT q_split_pair('42:hello')::VARCHAR", |r, i| unsafe {
            r.read_str(i).to_owned()
        }),
        Some("{'n': 42, 's': hello}".to_owned())
    );

    // Over a full multi-chunk scan, so every row index of a full vector is
    // written in both child vectors.
    assert_eq!(
        fx.scalar(
            "SELECT sum((q_split_pair(i::VARCHAR || ':x')).n) FROM range(5000) t(i)",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some((0..5000_i128).sum::<i128>())
    );
}

#[test]
fn a_registration_failure_names_the_three_things_it_can_be() {
    let fx = Fixture::open();

    unsafe extern "C" fn never_called(
        _: libduckdb_sys::duckdb_function_info,
        _: libduckdb_sys::duckdb_data_chunk,
        _: libduckdb_sys::duckdb_vector,
    ) {
    }

    // `list_sum` is a DuckDB built-in. `duckdb_register_scalar_function` reports
    // the collision as a bare DuckDBError with no message, which is
    // indistinguishable from a type error or a missing callback — so the error
    // has to name all three, and point at the query that settles it.
    // SAFETY: `con` is open.
    let err = unsafe {
        ScalarFunctionBuilder::new("list_sum")
            .param_logical(LogicalType::list(TypeId::BigInt))
            .returns(TypeId::BigInt)
            .function(never_called)
            .register(fx.con())
    }
    .expect_err("the name collides with a built-in");
    let msg = err.as_str();
    assert!(msg.contains("list_sum"), "names the function: {msg}");
    assert!(msg.contains("duckdb_functions()"), "names the check: {msg}");

    // Sanity: the built-in really is there, so the advice is actionable.
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_functions() WHERE function_name = 'list_sum'",
            |r, i| unsafe { r.read_i64(i) }
        )
        .map(|n| n > 0),
        Some(true)
    );
}

// ---------------------------------------------------------------------------
// Typed scalar bind data / local state.
//
// The raw `set_bind_data` route above makes the extension author write an
// `unsafe extern "C" fn drop_bind` that calls `Box::from_raw` — the same
// abort-on-panicking-Drop hazard quack-rs removed from its own destructors.
// These do the same job with the destructor generated and panic-safe.
// ---------------------------------------------------------------------------

#[cfg(feature = "duckdb-1-5")]
mod typed_scalar_state {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static BIND_DROPS: AtomicUsize = AtomicUsize::new(0);
    pub static STATE_DROPS: AtomicUsize = AtomicUsize::new(0);

    /// Folded once at bind time; explodes when dropped.
    pub struct Factor(pub i64);
    impl Drop for Factor {
        fn drop(&mut self) {
            BIND_DROPS.fetch_add(1, Ordering::SeqCst);
            panic!("typed bind data destructor deliberately exploded");
        }
    }

    /// Per-thread scratch.
    #[derive(Default)]
    pub struct Calls(pub u64);
    impl Drop for Calls {
        fn drop(&mut self) {
            STATE_DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn typed_bind(info: libduckdb_sys::duckdb_bind_info) {
    use quack_rs::scalar::{ScalarBindData, ScalarBindInfo};
    use typed_scalar_state::Factor;

    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    // SAFETY: two parameters were declared, so argument 1 exists.
    let factor = unsafe { bind.argument(1) }
        .and_then(|expr| {
            if !expr.is_foldable() {
                return None;
            }
            // SAFETY: inside a bind callback, so the context is live.
            let ctx = unsafe { bind.get_client_context() };
            expr.fold(&ctx).ok().map(|v| v.as_i64())
        })
        .unwrap_or(1);
    ScalarBindData::set(&bind, Factor(factor));
}

#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn typed_init(info: libduckdb_sys::duckdb_init_info) {
    use quack_rs::scalar::{ScalarInitInfo, ScalarLocalState};
    use typed_scalar_state::Calls;

    // SAFETY: DuckDB passes a valid init info.
    let init = unsafe { ScalarInitInfo::new(info) };
    ScalarLocalState::set(&init, Calls::default());
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(typed_scaled, |info, input, output| {
    use quack_rs::scalar::{ScalarBindData, ScalarFunctionInfo, ScalarLocalState};
    use typed_scalar_state::{Calls, Factor};

    // SAFETY: DuckDB passes a valid function info.
    let fninfo = unsafe { ScalarFunctionInfo::new(info) };
    // SAFETY: `typed_bind` stored a `Factor`, and nothing else did.
    let factor = unsafe { ScalarBindData::<Factor>::get(&fninfo) }.map_or(1, |f| f.0);
    // SAFETY: `typed_init` stored a `Calls` on this thread; no other borrow is live.
    if let Some(calls) = unsafe { ScalarLocalState::<Calls>::get_mut(&fninfo) } {
        calls.0 += 1;
    }

    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * factor) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

#[cfg(feature = "duckdb-1-5")]
#[test]
fn typed_scalar_bind_data_and_local_state_need_no_hand_written_destructor() {
    use quack_rs::scalar::ScalarFunctionBuilder;
    use std::sync::atomic::Ordering;

    let fx = Fixture::open();
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::new("typed_scaled")
            .param(TypeId::BigInt)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .bind(typed_bind)
            .init(typed_init)
            .function(typed_scaled)
            .register(fx.con())
            .expect("register typed_scaled");
    }

    // The second argument is constant-folded at bind time, so the row loop never
    // reads it.
    assert_eq!(
        fx.scalar(
            "SELECT typed_scaled(21::BIGINT, 2::BIGINT)",
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(42)
    );
    assert_eq!(
        fx.scalar(
            "SELECT sum(typed_scaled(i, 3::BIGINT)) FROM range(1000) t(i)",
            |r, i| unsafe { r.read_i128(i) }
        ),
        Some((0..1000_i128).sum::<i128>() * 3)
    );

    drop(fx);

    // `Factor::drop` panics. Through a hand-written `extern "C"` destructor that
    // would abort the process; the generated one contains it, so reaching this
    // line at all is the assertion.
    assert!(
        typed_scalar_state::BIND_DROPS.load(Ordering::SeqCst) >= 2,
        "bind data must be freed once per bind, got {}",
        typed_scalar_state::BIND_DROPS.load(Ordering::SeqCst)
    );
    assert!(
        typed_scalar_state::STATE_DROPS.load(Ordering::SeqCst) >= 2,
        "local state must be freed, got {}",
        typed_scalar_state::STATE_DROPS.load(Ordering::SeqCst)
    );
}

// ─── Arrow C Data Interface ──────────────────────────────────────────────────

/// End-to-end exercises for [`quack_rs::arrow`].
///
/// The point of these is that they use no Arrow library at all: a chunk is
/// exported through the C Data Interface and re-imported through it, and the
/// values are compared with the ones the query produced. If an ownership rule in
/// the wrapper were wrong, this is where a double free or a leak shows up — the
/// `leak-check` and `miri` CI jobs run this file.
#[cfg(feature = "duckdb-1-5-4")]
mod arrow_interop {
    use super::Fixture;
    use quack_rs::arrow::{
        data_chunk_from_arrow, data_chunk_to_arrow, schema_from_arrow, to_arrow_schema, ArrowArray,
        ArrowOptions, ArrowSchema, RawArrowSchema,
    };
    use quack_rs::error_data::DuckDbErrorType;
    use quack_rs::query::{OwnedDataChunk, QueryResult};
    use quack_rs::types::{LogicalType, TypeId};
    use std::ffi::CString;

    /// Every column of a result, as `(name, logical type)`, in order.
    fn columns(result: &QueryResult) -> Vec<(String, LogicalType)> {
        (0..result.column_count())
            .map(|i| {
                (
                    result.column_name(i).expect("column name"),
                    result.column_logical_type(i).expect("column type"),
                )
            })
            .collect()
    }

    /// Borrows `columns` in the shape `to_arrow_schema` takes.
    fn as_pairs(columns: &[(String, LogicalType)]) -> Vec<(&str, &LogicalType)> {
        columns
            .iter()
            .map(|(name, ty)| (name.as_str(), ty))
            .collect()
    }

    /// A release callback that frees nothing, for hand-built schemas whose
    /// strings are owned by the test.
    unsafe extern "C" fn noop_release(schema: *mut RawArrowSchema) {
        // SAFETY: DuckDB and quack-rs both pass a valid pointer, and the Arrow
        // specification requires the callback to null its own `release`.
        unsafe { (*schema).release = None };
    }

    #[test]
    fn a_chunk_survives_a_full_round_trip_through_the_c_data_interface() {
        let fx = Fixture::open();
        let sql = "SELECT i::INTEGER AS id, \
                        (i * 100)::BIGINT AS big, \
                        CASE WHEN i % 3 = 0 THEN NULL ELSE 'row-' || i END AS label \
                   FROM range(7) t(i) ORDER BY i";
        let mut result = fx.query(sql);
        let cols = columns(&result);
        assert_eq!(cols.len(), 3);

        let options = result.arrow_options().expect("result arrow options");
        let mut schema = to_arrow_schema(&options, &as_pairs(&cols)).expect("to_arrow_schema");

        let chunk = result.next_chunk().expect("one chunk");
        assert_eq!(chunk.size(), 7);
        let array = data_chunk_to_arrow(&options, &chunk).expect("data_chunk_to_arrow");
        assert_eq!(array.len(), 7, "the array carries every row");
        assert_eq!(array.child_count(), 3, "one child array per column");

        // The Arrow array owns its own copy of the data, so the chunk it came
        // from can go away before the import.
        drop(chunk);
        drop(result);

        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let converted = unsafe { schema_from_arrow(fx.con(), &mut schema) }.expect("converted");
        assert_eq!(converted.column_count(), 3);

        // SAFETY: same connection; `array` was produced from `schema`'s columns.
        let imported =
            unsafe { data_chunk_from_arrow(fx.con(), array, &converted) }.expect("import");

        assert_eq!(imported.column_count(), 3);
        assert_eq!(imported.size(), 7);
        // SAFETY: the chunk has three columns and seven rows.
        let (ids, bigs, labels) =
            unsafe { (imported.reader(0), imported.reader(1), imported.reader(2)) };
        for row in 0..7 {
            // SAFETY: `row` is in range for all three readers.
            unsafe {
                assert!(ids.is_valid(row), "id {row} is never NULL");
                assert_eq!(ids.read_i32(row), i32::try_from(row).unwrap());
                assert_eq!(bigs.read_i64(row), i64::try_from(row).unwrap() * 100);
                if row % 3 == 0 {
                    assert!(!labels.is_valid(row), "label {row} must survive as NULL");
                } else {
                    assert!(labels.is_valid(row));
                    assert_eq!(labels.read_str(row), format!("row-{row}"));
                }
            }
        }
    }

    #[test]
    fn to_arrow_schema_produces_a_struct_with_one_named_child_per_column() {
        let fx = Fixture::open();
        let result = fx.query("SELECT 1::INTEGER AS id, 2::BIGINT AS big, 'x' AS label");
        let cols = columns(&result);
        let options = result.arrow_options().expect("arrow options");
        let schema = to_arrow_schema(&options, &as_pairs(&cols)).expect("to_arrow_schema");

        assert!(!schema.is_released());
        assert_eq!(schema.format(), Some("+s"), "a record batch is a struct");
        assert_eq!(schema.child_count(), 3);

        let id = schema.child(0).expect("child 0");
        assert_eq!(id.name(), Some("id"));
        assert_eq!(id.format(), Some("i"), "INTEGER is Arrow int32");

        let big = schema.child(1).expect("child 1");
        assert_eq!(big.name(), Some("big"));
        assert_eq!(big.format(), Some("l"), "BIGINT is Arrow int64");

        let label = schema.child(2).expect("child 2");
        assert_eq!(label.name(), Some("label"));
        // Which string encoding DuckDB picks depends on the connection's arrow
        // options (offset size, string views), so accept the whole family rather
        // than pinning one and calling it a guarantee.
        let format = label.format().expect("a format string");
        assert!(
            matches!(format, "u" | "U" | "vu"),
            "VARCHAR should be an Arrow string encoding, got {format:?}"
        );

        assert!(schema.child(3).is_none(), "no fourth column");
    }

    #[test]
    fn a_schema_and_an_array_can_be_released_early_and_the_drop_is_a_no_op() {
        let fx = Fixture::open();
        let mut result = fx.query("SELECT 42::INTEGER AS answer");
        let cols = columns(&result);
        let options = result.arrow_options().expect("arrow options");
        let mut schema = to_arrow_schema(&options, &as_pairs(&cols)).expect("to_arrow_schema");
        let chunk = result.next_chunk().expect("one chunk");
        let mut array = data_chunk_to_arrow(&options, &chunk).expect("data_chunk_to_arrow");

        // Calling DuckDB's own release callbacks — this is where a wrong
        // ownership assumption would abort. Reaching the assertions is the test.
        schema.release();
        array.release();
        assert!(schema.is_released());
        assert!(array.is_released());
        // Idempotent, and the implicit drops below add nothing.
        schema.release();
        array.release();
    }

    #[test]
    fn arrow_options_are_available_from_a_connection_as_well_as_a_result() {
        let fx = Fixture::open();
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let from_con = unsafe { ArrowOptions::from_connection(fx.con()) }.expect("from_connection");
        assert!(!from_con.as_raw().is_null());

        let id = LogicalType::new(TypeId::Integer);
        let schema = to_arrow_schema(&from_con, &[("id", &id)]).expect("to_arrow_schema");
        assert_eq!(schema.child_count(), 1);
    }

    #[test]
    fn from_connection_rejects_a_null_connection() {
        // The fixture is what populates the loadable-extension dispatch table;
        // without it the call would not reach DuckDB at all.
        let _fx = Fixture::open();
        // SAFETY: a null connection is exactly the case DuckDB reports by
        // writing null to the out-parameter.
        let err = unsafe { ArrowOptions::from_connection(std::ptr::null_mut()) }
            .expect_err("a null connection has no arrow options");
        assert!(err.to_string().contains("null"), "{err}");
    }

    #[test]
    fn to_arrow_schema_rejects_a_name_with_an_interior_nul() {
        let fx = Fixture::open();
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let options = unsafe { ArrowOptions::from_connection(fx.con()) }.expect("arrow options");
        let id = LogicalType::new(TypeId::Integer);
        let err = to_arrow_schema(&options, &[("bad\0name", &id)])
            .expect_err("DuckDB reads schema names as C strings");
        assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
        assert!(
            err.message().unwrap_or_default().contains("NUL"),
            "{:?}",
            err.message()
        );
    }

    #[test]
    fn schema_from_arrow_refuses_an_already_released_schema() {
        let fx = Fixture::open();
        let mut schema = ArrowSchema::empty();
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let err = unsafe { schema_from_arrow(fx.con(), &mut schema) }
            .expect_err("a released schema has no usable children pointer");
        assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
        assert!(
            err.message().unwrap_or_default().contains("released"),
            "{:?}",
            err.message()
        );
    }

    #[test]
    fn schema_from_arrow_reports_an_arrow_type_duckdb_cannot_map() {
        let fx = Fixture::open();

        // Declared before `schema` so they outlive it: locals drop in reverse.
        let root_format = CString::new("+s").unwrap();
        let child_format = CString::new("?").unwrap();
        let child_name = CString::new("unsupported").unwrap();
        let mut child = RawArrowSchema::empty();
        child.format = child_format.as_ptr();
        child.name = child_name.as_ptr();
        child.release = Some(noop_release);
        let mut child_ptr: *mut RawArrowSchema = &raw mut child;
        let mut root = RawArrowSchema::empty();
        root.format = root_format.as_ptr();
        root.n_children = 1;
        root.children = &raw mut child_ptr;
        root.release = Some(noop_release);

        // SAFETY: `noop_release` frees nothing; the strings are owned above.
        let mut schema = unsafe { ArrowSchema::from_raw(root) };
        assert_eq!(schema.child_count(), 1);

        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let err = unsafe { schema_from_arrow(fx.con(), &mut schema) }
            .expect_err("'?' is not an Arrow format string DuckDB knows");
        let message = err.message().unwrap_or_default();
        assert!(
            message.contains("Unsupported") || message.contains("unsupported"),
            "{message}"
        );
    }

    #[test]
    fn data_chunk_from_arrow_refuses_an_already_released_array() {
        let fx = Fixture::open();
        let result = fx.query("SELECT 1::INTEGER AS id");
        let cols = columns(&result);
        let options = result.arrow_options().expect("arrow options");
        let mut schema = to_arrow_schema(&options, &as_pairs(&cols)).expect("to_arrow_schema");
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let converted = unsafe { schema_from_arrow(fx.con(), &mut schema) }.expect("converted");

        // SAFETY: same connection; an empty array is already released.
        let err = unsafe { data_chunk_from_arrow(fx.con(), ArrowArray::empty(), &converted) }
            .expect_err("DuckDB would dereference a released array");
        assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
        assert!(
            err.message().unwrap_or_default().contains("released"),
            "{:?}",
            err.message()
        );
    }

    #[test]
    fn data_chunk_from_arrow_refuses_an_array_with_the_wrong_child_count() {
        let fx = Fixture::open();
        // A two-column schema…
        let mut two = fx.query("SELECT 1::INTEGER AS a, 2::INTEGER AS b");
        let two_cols = columns(&two);
        let options = two.arrow_options().expect("arrow options");
        let mut schema = to_arrow_schema(&options, &as_pairs(&two_cols)).expect("to_arrow_schema");
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let converted = unsafe { schema_from_arrow(fx.con(), &mut schema) }.expect("converted");
        assert_eq!(converted.column_count(), 2);
        drop(two.next_chunk());

        // …fed a one-column array. DuckDB would read `children[1]` past the end.
        let mut one = fx.query("SELECT 9::INTEGER AS only");
        let one_chunk = one.next_chunk().expect("one chunk");
        let array = data_chunk_to_arrow(&options, &one_chunk).expect("data_chunk_to_arrow");
        assert_eq!(array.child_count(), 1);

        // SAFETY: same connection; the mismatch is caught before DuckDB is called.
        let err = unsafe { data_chunk_from_arrow(fx.con(), array, &converted) }
            .expect_err("a one-column array cannot fill a two-column schema");
        assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
        let message = err.message().unwrap_or_default();
        assert!(message.contains('1') && message.contains('2'), "{message}");
    }

    #[test]
    fn a_zero_row_array_is_refused_rather_than_aborting_a_debug_duckdb() {
        let fx = Fixture::open();
        let result = fx.query("SELECT 1::INTEGER AS id");
        let cols = columns(&result);
        let options = result.arrow_options().expect("arrow options");
        let mut schema = to_arrow_schema(&options, &as_pairs(&cols)).expect("to_arrow_schema");
        // SAFETY: `fx.con()` is open for the fixture's lifetime.
        let converted = unsafe { schema_from_arrow(fx.con(), &mut schema) }.expect("converted");

        // An empty *result* yields no chunks at all, so the zero-row case has to
        // come from a chunk built directly - which is also the shape an
        // extension produces when a scan finds nothing.
        let mut types = [cols[0].1.as_raw()];
        // SAFETY: one valid logical type, which DuckDB copies.
        let raw_chunk = unsafe { libduckdb_sys::duckdb_create_data_chunk(types.as_mut_ptr(), 1) };
        assert!(!raw_chunk.is_null(), "duckdb_create_data_chunk");
        // SAFETY: the chunk is ours to own and destroy.
        let chunk = unsafe { OwnedDataChunk::from_raw(raw_chunk) };
        // SAFETY: zero is always within capacity.
        unsafe { chunk.set_size(0) };

        // Exporting a zero-row chunk is fine, and produces a well-formed empty
        // Arrow array.
        let array = data_chunk_to_arrow(&options, &chunk).expect("data_chunk_to_arrow");
        assert_eq!(array.len(), 0);
        assert_eq!(array.child_count(), 1);

        // Importing one is not. DuckDB passes `arrow_array->length` through as
        // the chunk's *capacity*, and a capacity of zero reaches
        // `Allocator::AllocateData(0)`, whose `D_ASSERT(size > 0)` aborts a
        // debug build of DuckDB while a release build silently carries on. That
        // is refused here so the behaviour does not depend on how the engine was
        // compiled — reaching this assertion under a debug DuckDB is the test.
        // SAFETY: same connection; one child array, one schema column.
        let err = unsafe { data_chunk_from_arrow(fx.con(), array, &converted) }
            .expect_err("a zero-row array must be refused, not handed to DuckDB");
        assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
        assert!(
            err.message().unwrap_or_default().contains("zero-row"),
            "{:?}",
            err.message()
        );
    }
}

// ─── COPY ... FROM ───────────────────────────────────────────────────────────

/// A read-only copy function: `COPY tbl FROM 'file' (FORMAT lines)` driven by a
/// quack-rs table function.
///
/// The point of this module is the whole chain — `build_handle` →
/// `copy_from` → `duckdb_copy_function_set_copy_from_function` →
/// `CCopyFromBind` → the table function's own bind and scan — with the target
/// table's schema read back through
/// [`BindInfo::result_column_count`][quack_rs::table::BindInfo::result_column_count]
/// rather than declared.
#[cfg(feature = "duckdb-1-5")]
mod copy_from {
    use super::Fixture;
    use quack_rs::copy_function::CopyFunctionBuilder;
    use quack_rs::error::ExtensionError;
    use quack_rs::table::TableFunctionBuilder;
    use quack_rs::types::TypeId;

    /// Rows parsed at bind time, handed to the scan one chunk at a time.
    struct Reader {
        rows: Vec<(i32, String)>,
        next: usize,
        /// The target table's columns, as `CCopyFromBind` supplied them.
        schema: Vec<(String, Option<TypeId>)>,
    }

    /// Registers `lines`: a `COPY … FROM` handler for a trivial `id,label`
    /// text format, with `skip_rows` as a COPY option.
    fn register_lines_format(con: libduckdb_sys::duckdb_connection) {
        let reader = TableFunctionBuilder::new("lines_reader")
            // Exactly one VARCHAR — the file path. `CCopyFromBind` supplies it.
            .param(TypeId::Varchar)
            // COPY options arrive as named parameters, and an option the
            // function never declared is a bind error naming the format.
            .named_param("skip_rows", TypeId::BigInt)
            .with_state::<Reader, _>(|bind| {
                // A COPY ... FROM reader must NOT call add_result_column: the
                // destination table already fixed the schema. Read it instead,
                // and adapt — or refuse.
                let schema: Vec<(String, Option<TypeId>)> = (0..bind.result_column_count())
                    .map(|i| {
                        let name = bind.result_column_name(i).unwrap_or_default();
                        // SAFETY: called during bind, with `i` in range.
                        let ty = unsafe { bind.result_column_type(i) };
                        // SAFETY: the handle is live for as long as `ty` is.
                        let id = ty.map(|t| unsafe { t.get_type_id() });
                        (name, id)
                    })
                    .collect();
                if schema.len() != 2 {
                    return Err(ExtensionError::new(format!(
                        "lines: expected a two-column target table, got {}",
                        schema.len()
                    )));
                }
                if schema[0].1 != Some(TypeId::Integer) || schema[1].1 != Some(TypeId::Varchar) {
                    let named = |(name, id): &(String, Option<TypeId>)| {
                        format!("{name} {}", id.map_or("<unknown>", TypeId::sql_name))
                    };
                    return Err(ExtensionError::new(format!(
                        "lines: expected (INTEGER, VARCHAR), got ({}, {})",
                        named(&schema[0]),
                        named(&schema[1]),
                    )));
                }
                // SAFETY: parameter 0 is declared above, and DuckDB always
                // supplies the file path there for a COPY ... FROM.
                let path = unsafe { bind.get_parameter_value(0) }
                    .as_str()
                    .map_err(|e| ExtensionError::new(format!("lines: bad path: {e}")))?;
                // SAFETY: `skip_rows` is declared above; an absent option yields
                // a null handle, which `as_i64_or` reports as the default.
                let skip = usize::try_from(
                    unsafe { bind.get_named_parameter_value("skip_rows") }.as_i64_or(0),
                )
                .unwrap_or(0);

                let text = std::fs::read_to_string(&path)
                    .map_err(|e| ExtensionError::new(format!("lines: {path}: {e}")))?;
                let rows = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .skip(skip)
                    .map(|line| {
                        let (id, label) = line.split_once(',').ok_or_else(|| {
                            ExtensionError::new(format!("lines: no comma: {line}"))
                        })?;
                        let id: i32 = id
                            .trim()
                            .parse()
                            .map_err(|e| ExtensionError::new(format!("lines: bad id: {e}")))?;
                        Ok((id, label.trim().to_owned()))
                    })
                    .collect::<Result<Vec<_>, ExtensionError>>()?;

                Ok(Reader {
                    rows,
                    next: 0,
                    schema,
                })
            })
            .scan(|state, chunk| {
                assert_eq!(
                    state.schema.len(),
                    chunk.column_count(),
                    "the output chunk has one vector per target column"
                );
                let take = (state.rows.len() - state.next).min(1024);
                // SAFETY: two columns, exactly as the target table has.
                let (mut ids, mut labels) = unsafe { (chunk.writer(0), chunk.writer(1)) };
                for row in 0..take {
                    let (id, label) = &state.rows[state.next + row];
                    // SAFETY: `row` is below the chunk's capacity.
                    unsafe {
                        ids.write_i32(row, *id);
                        labels.write_varchar(row, label);
                    }
                }
                state.next += take;
                // SAFETY: `take` is within the chunk's capacity.
                unsafe { chunk.set_size(take) };
                Ok(())
            })
            .build()
            .expect("build lines_reader");

        // SAFETY: every callback matches its declared signature.
        let handle = unsafe { reader.build_handle() }.expect("build_handle");
        assert_eq!(handle.param_types(), &[Some(TypeId::Varchar)]);

        let copy = CopyFunctionBuilder::try_new("lines")
            .expect("copy function name")
            .copy_from(handle)
            .expect("attach the reader");
        // SAFETY: `con` is open.
        unsafe { copy.register(con) }.expect("register the lines format");
    }

    fn write_fixture_file(name: &str, contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join(name), contents).expect("write fixture");
        dir
    }

    #[test]
    fn a_read_only_copy_function_loads_a_table() {
        let fx = Fixture::open();
        register_lines_format(fx.con());
        let dir = write_fixture_file("data.lines", "1,alpha\n2,beta\n3,gamma\n");
        let path = dir.path().join("data.lines");
        let path = path.to_str().expect("utf-8 path");

        fx.query("CREATE TABLE t (id INTEGER, label VARCHAR)");
        fx.query(&format!("COPY t FROM '{path}' (FORMAT lines)"));

        assert_eq!(
            fx.scalar("SELECT count(*) FROM t", |r, i| unsafe { r.read_i64(i) }),
            Some(3)
        );
        assert_eq!(
            fx.scalar("SELECT sum(id)::BIGINT FROM t", |r, i| unsafe {
                r.read_i64(i)
            }),
            Some(6)
        );
        assert_eq!(
            fx.scalar(
                "SELECT string_agg(label, '|' ORDER BY id) FROM t",
                |r, i| { unsafe { r.read_str(i).to_owned() } }
            )
            .as_deref(),
            Some("alpha|beta|gamma")
        );
    }

    #[test]
    fn a_copy_option_reaches_the_reader_as_a_named_parameter() {
        let fx = Fixture::open();
        register_lines_format(fx.con());
        let dir = write_fixture_file("data.lines", "0,header\n1,alpha\n2,beta\n");
        let path = dir.path().join("data.lines");
        let path = path.to_str().expect("utf-8 path");

        fx.query("CREATE TABLE t (id INTEGER, label VARCHAR)");
        fx.query(&format!("COPY t FROM '{path}' (FORMAT lines, SKIP_ROWS 1)"));

        assert_eq!(
            fx.scalar("SELECT count(*) FROM t", |r, i| unsafe { r.read_i64(i) }),
            Some(2)
        );
        assert_eq!(
            fx.scalar("SELECT min(label) FROM t", |r, i| unsafe {
                r.read_str(i).to_owned()
            })
            .as_deref(),
            Some("alpha")
        );
    }

    #[test]
    fn an_undeclared_copy_option_is_a_binder_error_naming_the_format() {
        let fx = Fixture::open();
        register_lines_format(fx.con());
        let dir = write_fixture_file("data.lines", "1,alpha\n");
        let path = dir.path().join("data.lines");
        let path = path.to_str().expect("utf-8 path");

        fx.query("CREATE TABLE t (id INTEGER, label VARCHAR)");
        // SAFETY: `con` is open.
        let err = unsafe {
            quack_rs::query::query(
                fx.con(),
                &format!("COPY t FROM '{path}' (FORMAT lines, NONSENSE 1)"),
            )
        }
        .expect_err("an option the reader never declared must not be accepted");
        // Two things worth pinning down, because neither is obvious:
        //   * the option name comes back exactly as the user typed it, even
        //     though the lookup against the declared named parameters is
        //     case-insensitive (SKIP_ROWS matches `skip_rows` above);
        //   * the format named in the message is the *table function's* name,
        //     not the copy function's. `CCopyFromBind` reports `info.tf.name`,
        //     and `duckdb_copy_function_set_copy_from_function` only borrows the
        //     copy function's name when the table function has none.
        assert!(err.as_str().contains("NONSENSE"), "{err}");
        assert!(err.as_str().contains("lines_reader"), "{err}");
    }

    #[test]
    fn the_reader_sees_the_target_tables_schema_rather_than_declaring_one() {
        let fx = Fixture::open();
        register_lines_format(fx.con());
        let dir = write_fixture_file("data.lines", "1,alpha\n");
        let path = dir.path().join("data.lines");
        let path = path.to_str().expect("utf-8 path");

        // The bind callback rejects anything but two columns, and it only knows
        // the column count because DuckDB told it — nothing in the reader
        // declares a schema.
        fx.query("CREATE TABLE three (a INTEGER, b VARCHAR, c INTEGER)");
        // SAFETY: `con` is open.
        let err = unsafe {
            quack_rs::query::query(
                fx.con(),
                &format!("COPY three FROM '{path}' (FORMAT lines)"),
            )
        }
        .expect_err("the reader refuses a three-column target");
        assert!(err.as_str().contains("two-column target table"), "{err}");
        assert!(err.as_str().contains('3'), "{err}");
    }

    #[test]
    fn the_reader_sees_the_target_tables_column_types_too() {
        let fx = Fixture::open();
        register_lines_format(fx.con());
        let dir = write_fixture_file("data.lines", "1,alpha\n");
        let path = dir.path().join("data.lines");
        let path = path.to_str().expect("utf-8 path");

        fx.query("CREATE TABLE mistyped (id VARCHAR, label VARCHAR)");
        // SAFETY: `con` is open.
        let err = unsafe {
            quack_rs::query::query(
                fx.con(),
                &format!("COPY mistyped FROM '{path}' (FORMAT lines)"),
            )
        }
        .expect_err("the reader writes INTEGER into column 0");
        assert!(
            err.as_str().contains("expected (INTEGER, VARCHAR)"),
            "{err}"
        );
        assert!(err.as_str().contains("id VARCHAR"), "{err}");
    }

    #[test]
    fn copy_from_rejects_a_reader_whose_parameters_do_not_match_the_contract() {
        let fx = Fixture::open();

        for (label, build) in [
            (
                "no positional parameter",
                (|| TableFunctionBuilder::new("no_params")) as fn() -> TableFunctionBuilder,
            ),
            ("a non-VARCHAR parameter", || {
                TableFunctionBuilder::new("wrong_type").param(TypeId::BigInt)
            }),
            ("two parameters", || {
                TableFunctionBuilder::new("two_params")
                    .param(TypeId::Varchar)
                    .param(TypeId::Varchar)
            }),
        ] {
            let reader = build()
                .with_state::<u8, _>(|_| Ok(0))
                .scan(|_, chunk| {
                    // SAFETY: end of stream.
                    unsafe { chunk.set_size(0) };
                    Ok(())
                })
                .build()
                .expect("build");
            // SAFETY: every callback matches its declared signature.
            let handle = unsafe { reader.build_handle() }.expect("build_handle");
            let err = CopyFunctionBuilder::try_new("rejected")
                .expect("name")
                .copy_from(handle)
                .err()
                .unwrap_or_else(|| panic!("{label} must be rejected"));
            assert!(
                err.as_str().contains("exactly one VARCHAR"),
                "{label}: {err}"
            );
        }

        drop(fx);
    }

    #[test]
    fn a_copy_function_that_implements_neither_direction_is_refused() {
        let fx = Fixture::open();
        let builder = CopyFunctionBuilder::try_new("empty_format").expect("name");
        // SAFETY: `con` is open.
        let err = unsafe { builder.register(fx.con()) }
            .expect_err("DuckDB refuses this with no message of its own");
        assert!(err.as_str().contains("implements nothing"), "{err}");
    }
}
