// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

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

    fn con(&self) -> duckdb_connection {
        self.con
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
