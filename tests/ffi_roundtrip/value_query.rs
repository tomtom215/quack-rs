// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `Value` getters, DECIMAL construction/binding, `Expression::fold`,
//! `DuckDbErrorType`, `DbConfig::set`, and the Arrow-options lifetime — each
//! against a live `DuckDB`.
//!
//! Several of these guard paths that used to abort the process (a C++
//! exception thrown through the C API) or dereference a null handle. Those
//! can only be tested after the fix: before it, the test binary died.

use quack_rs::query::OwnedConnection;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;

use super::Fixture;

fn owned_connection(fx: &Fixture) -> OwnedConnection {
    // SAFETY: the fixture's database is open for its whole lifetime, and every
    // caller drops the connection before the fixture.
    unsafe { OwnedConnection::open(fx.db()) }.expect("connect")
}

#[cfg(feature = "duckdb-1-5-4")]
fn first_varchar(con: &OwnedConnection, sql: &str) -> String {
    let mut result = con.query(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: column 0 is VARCHAR and row 0 exists.
    unsafe { chunk.reader(0).read_str(0).to_owned() }
}

/// Every scalar getter on `value`, as `Some(true)` if it produced a value.
/// Calling each one is the point: none may abort or crash.
fn all_scalar_getters(value: &Value) -> Vec<(&'static str, bool)> {
    // `mut` is used only by the `duckdb-1-5` additions below.
    #[cfg_attr(not(feature = "duckdb-1-5"), allow(unused_mut))]
    let mut out = vec![
        ("i8", value.as_i8().is_some()),
        ("i16", value.as_i16().is_some()),
        ("i32", value.as_i32().is_some()),
        ("i64", value.as_i64().is_some()),
        ("i128", value.as_i128().is_some()),
        ("u8", value.as_u8().is_some()),
        ("u16", value.as_u16().is_some()),
        ("u32", value.as_u32().is_some()),
        ("u64", value.as_u64().is_some()),
        ("u128", value.as_u128().is_some()),
        ("f32", value.as_f32().is_some()),
        ("f64", value.as_f64().is_some()),
        ("bool", value.as_bool().is_some()),
        ("date", value.as_date().is_some()),
        ("time", value.as_time().is_some()),
        ("time_tz", value.as_time_tz().is_some()),
        ("timestamp", value.as_timestamp().is_some()),
        ("timestamp_tz", value.as_timestamp_tz().is_some()),
        ("timestamp_s", value.as_timestamp_s().is_some()),
        ("timestamp_ms", value.as_timestamp_ms().is_some()),
        ("timestamp_ns", value.as_timestamp_ns().is_some()),
        ("interval", value.as_interval().is_some()),
        ("uuid", value.as_uuid().is_some()),
        ("decimal", value.as_decimal().is_some()),
        ("enum", value.as_enum_index().is_some()),
    ];
    #[cfg(feature = "duckdb-1-5")]
    out.push(("time_ns", value.as_time_ns().is_some()));
    out
}

// ── Findings 1 and 2: SQL NULL and null handles ─────────────────────────────

/// `duckdb_get_*` on a SQL NULL threw `InternalException` from
/// `Value::GetValue`, aborting the process — `_or` included, since it only
/// checked the handle.
#[test]
fn every_getter_returns_none_for_a_sql_null() {
    let _fx = Fixture::open();
    let null = Value::null_value();
    assert!(null.is_sql_null());
    for (name, some) in all_scalar_getters(&null) {
        assert!(!some, "as_{name} on SQL NULL must be None");
    }
    assert_eq!(null.as_i64_or(7), 7);
    assert_eq!(null.as_u16_or(5432), 5432);
    assert!(null.as_bool_or(true));
    assert!(null.as_str().is_err());
    assert_eq!(null.as_str_or("dflt"), "dflt");
    assert!(null.as_blob().is_err());

    // A *typed* NULL, as a bound parameter or folded constant would be.
    let ty = LogicalType::new(TypeId::BigInt);
    let list = Value::list_value(&ty, &[Value::null_value()]).expect("LIST(BIGINT)");
    let typed_null = list.list_child(0).expect("child 0");
    assert_eq!(typed_null.type_id(), Some(TypeId::BigInt));
    assert!(typed_null.is_sql_null());
    for (name, some) in all_scalar_getters(&typed_null) {
        assert!(!some, "as_{name} on BIGINT NULL must be None");
    }
    assert_eq!(typed_null.as_i64_or(-3), -3);
    assert!(typed_null.as_str().is_err());
}

/// `CAPIGetValue` dereferenced a null handle without a check.
#[test]
fn every_getter_returns_none_for_a_null_handle() {
    let _fx = Fixture::open();
    // SAFETY: a null handle is part of `Value`'s contract.
    let value = unsafe { Value::from_raw(std::ptr::null_mut()) };
    for (name, some) in all_scalar_getters(&value) {
        assert!(!some, "as_{name} on a null handle must be None");
    }
    assert_eq!(value.as_i128_or(-9), -9);
    assert!(value.as_blob().is_err());
}

/// The realistic trigger: `f(n := NULL)` in SQL, read with the defaulting
/// getter. Also an absent named parameter, which hands back a null handle.
#[test]
fn a_null_named_parameter_reads_as_the_default() {
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();
    let table_fn = TableFunctionBuilder::new("vq_named")
        .named_param("n", TypeId::BigInt)
        .with_state::<Option<i64>, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: "n" was declared above.
            let n = unsafe { bind.get_named_parameter_value("n") }.as_i64_or(5);
            Ok(Some(n))
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
        .expect("build vq_named");
    // SAFETY: `con` is open.
    unsafe { table_fn.register(fx.con()) }.expect("register vq_named");

    let read = |sql: &str| fx.scalar(sql, |r, i| unsafe { r.read_i64(i) });
    assert_eq!(read("SELECT n FROM vq_named(n := NULL)"), Some(5));
    assert_eq!(read("SELECT n FROM vq_named(n := 3)"), Some(3));
    assert_eq!(read("SELECT n FROM vq_named()"), Some(5));
}

// ── Finding 7: casting, failures and mutation ───────────────────────────────

/// A failed cast returned `NullValue<T>` (`i64::MIN`, `NaN`) — the doc said 0
/// — which is also a legitimate value. Now it is `None`, and the legitimate
/// value still reads back.
#[test]
fn failed_casts_are_none_and_sentinel_values_still_read() {
    let _fx = Fixture::open();
    let abc = Value::varchar("abc");
    assert_eq!(abc.as_i64(), None);
    assert_eq!(abc.as_i32(), None);
    assert_eq!(abc.as_f64(), None);
    assert_eq!(abc.as_bool(), None);
    assert_eq!(abc.as_date(), None);
    assert_eq!(abc.as_i64_or(11), 11);

    assert_eq!(Value::bigint(i64::MIN).as_i64(), Some(i64::MIN));
    assert_eq!(Value::integer(i32::MIN).as_i32(), Some(i32::MIN));
    assert!(Value::double(f64::NAN).as_f64().is_some_and(f64::is_nan));
    assert_eq!(Value::boolean(false).as_bool(), Some(false));

    // Casting, TRY_CAST style.
    assert_eq!(Value::varchar("42").as_i64(), Some(42));
    assert_eq!(Value::varchar("true").as_bool(), Some(true));
    assert_eq!(Value::integer(7).as_i64(), Some(7));
    assert_eq!(Value::bigint(300).as_i8(), None, "out of range for TINYINT");
    assert_eq!(
        Value::double(1e20).as_i32(),
        None,
        "out of range for INTEGER"
    );
    assert_eq!(Value::bigint(-1).as_u64(), None);
    assert_eq!(Value::varchar("2024-01-02").as_date(), Some(19_724));
}

/// `CAPIGetValue` cast the caller's value *in place*:
/// `Value::double(1.5).as_i32()` left it a `DOUBLE` holding `2.0`.
#[test]
fn getters_do_not_modify_the_value() {
    let _fx = Fixture::open();
    let v = Value::double(1.5);
    assert_eq!(v.as_i32(), Some(2), "DuckDB rounds DOUBLE -> INTEGER");
    assert_eq!(v.as_f64(), Some(1.5));
    assert_eq!(v.type_id(), Some(TypeId::Double));
    #[cfg(feature = "duckdb-1-5")]
    assert_eq!(v.display_string().as_deref(), Some("1.5"));

    let s = Value::varchar("12");
    assert_eq!(s.as_i64(), Some(12));
    assert_eq!(s.type_id(), Some(TypeId::Varchar));
    assert_eq!(s.as_str().expect("utf8"), "12");
}

/// Every scalar getter against a value of every scalar type, plus nested and
/// binary types: nothing may abort, and nested/BLOB/ENUM sources are refused.
#[test]
fn every_getter_is_safe_on_every_value_type() {
    let _fx = Fixture::open();
    let enum_ty = LogicalType::enum_type(&["a", "b"]);
    let list_ty = LogicalType::new(TypeId::BigInt);
    let struct_ty = LogicalType::struct_type(&[("x", TypeId::BigInt)]);
    // `mut` is used only by the `duckdb-1-5` additions below.
    #[cfg_attr(not(feature = "duckdb-1-5"), allow(unused_mut))]
    let mut values = vec![
        Value::boolean(true),
        Value::tinyint(-1),
        Value::smallint(-300),
        Value::integer(70_000),
        Value::bigint(1 << 40),
        Value::utinyint(200),
        Value::usmallint(60_000),
        Value::uinteger(4_000_000_000),
        Value::ubigint(u64::MAX),
        Value::hugeint(i128::MIN),
        Value::uhugeint(u128::MAX),
        Value::float(1.25),
        Value::double(-2.5e300),
        Value::varchar("not a number"),
        Value::varchar("1"),
        Value::date(19_000),
        Value::time(1_000_000),
        Value::time_tz(0),
        Value::timestamp(1_700_000_000_000_000),
        Value::timestamp_tz(0),
        Value::timestamp_s(0),
        Value::timestamp_ms(0),
        Value::timestamp_ns(0),
        Value::interval(quack_rs::interval::DuckInterval {
            months: 1,
            days: 2,
            micros: 3,
        }),
        Value::uuid(u128::MAX),
        Value::blob(&[0xff, 0x00]),
        Value::decimal(38, 2, 10_i128.pow(37)).expect("DECIMAL(38,2)"),
        Value::enum_value(&enum_ty, 1).expect("ENUM"),
        Value::list_value(&list_ty, &[Value::bigint(1)]).expect("LIST"),
        Value::struct_value(&struct_ty, &[Value::bigint(1)]).expect("STRUCT"),
    ];
    #[cfg(feature = "duckdb-1-5")]
    values.push(Value::time_ns(5));
    for value in &values {
        let before = value.type_id();
        let results = all_scalar_getters(value);
        assert_eq!(value.type_id(), before, "a getter changed {value:?}");
        let refused = matches!(
            before,
            Some(TypeId::Blob | TypeId::Enum | TypeId::List | TypeId::Struct)
        );
        for (name, some) in results {
            if refused && name != "enum" {
                assert!(!some, "as_{name} must refuse {value:?}");
            }
        }
    }
    let blob = Value::blob(&[0xff, 0x00]);
    assert_eq!(blob.as_blob().expect("blob"), vec![0xff, 0x00]);
    assert_eq!(
        Value::enum_value(&enum_ty, 1)
            .expect("ENUM")
            .as_enum_index(),
        Some(1)
    );
    assert_eq!(Value::bigint(1).as_enum_index(), None);
    assert_eq!(Value::bigint(1).as_decimal(), None);
}

/// `duckdb_get_blob` casts to BLOB with a *throwing* cast: an `INTEGER`
/// aborted the process. Non-BLOB values are now an error.
#[test]
fn as_blob_refuses_values_that_are_not_blobs() {
    let _fx = Fixture::open();
    assert!(Value::integer(1).as_blob().is_err());
    assert!(Value::varchar("\\xZZ").as_blob().is_err());
    assert_eq!(
        Value::blob(&[]).as_blob().expect("empty blob"),
        Vec::<u8>::new()
    );
}

// ── Findings 5 and 6: DECIMAL range ─────────────────────────────────────────

/// `Value::decimal(4, 0, 100_000)` aborted (`NumericCast` to `int16`);
/// `(4, 0, 10_000)` stored a five-digit `DECIMAL(4,0)`; `(4, 0, 2^70 + 7)`
/// stored `7`.
#[test]
fn value_decimal_rejects_an_unscaled_value_wider_than_the_width() {
    let _fx = Fixture::open();
    assert!(Value::decimal(4, 0, 100_000).is_err());
    assert!(Value::decimal(4, 0, 10_000).is_err());
    assert!(Value::decimal(2, 1, 12_345).is_err());
    assert!(Value::decimal(4, 0, (1_i128 << 70) + 7).is_err());
    assert!(Value::decimal(0, 0, 0).is_err());
    assert!(Value::decimal(39, 0, 0).is_err());
    assert!(Value::decimal(4, 5, 0).is_err());

    for (width, scale, unscaled) in [
        (4_u8, 0_u8, 9_999_i128),
        (4, 2, -9_999),
        (18, 3, 999_999_999_999_999_999),
        (19, 0, 10_i128.pow(18)),
        (38, 10, 10_i128.pow(38) - 1),
        (38, 0, -(10_i128.pow(38) - 1)),
    ] {
        let value = Value::decimal(width, scale, unscaled).expect("in range");
        let read = value.as_decimal().expect("a DECIMAL");
        assert_eq!(
            (read.width, read.scale, read.value),
            (width, scale, unscaled)
        );
    }
}

/// `bind_decimal` aborted for width 50 or an `unscaled` too wide for the
/// physical type, and for `width <= 18` silently dropped the high 64 bits:
/// `(18, 0, 2^64)` bound `0`.
#[test]
fn bind_decimal_validates_width_scale_and_range() {
    let fx = Fixture::open();
    let con = owned_connection(&fx);
    let st = con.prepare("SELECT CAST($1 AS VARCHAR)").expect("prepare");

    assert!(st.bind_decimal(1, 50, 60, 1).is_err());
    assert!(st.bind_decimal(1, 0, 0, 1).is_err());
    assert!(st.bind_decimal(1, 4, 5, 1).is_err());
    assert!(st.bind_decimal(1, 4, 0, 100_000).is_err());
    assert!(st.bind_decimal(1, 18, 0, 1_i128 << 64).is_err());

    st.bind_decimal(1, 18, 2, -12_345).expect("in range");
    let mut result = st.execute().expect("execute");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: column 0 is VARCHAR and row 0 exists.
    assert_eq!(unsafe { chunk.reader(0).read_str(0) }, "-123.45");
    drop(result);

    st.bind_decimal(1, 38, 0, 10_i128.pow(37))
        .expect("in range");
    let mut result = st.execute().expect("execute");
    let chunk = result.next_chunk().expect("one chunk");
    // SAFETY: column 0 is VARCHAR and row 0 exists.
    assert_eq!(
        unsafe { chunk.reader(0).read_str(0) },
        format!("1{}", "0".repeat(37))
    );
}

// ── Finding 8: error types 40–42 ────────────────────────────────────────────

#[cfg(feature = "duckdb-1-5")]
#[test]
fn error_data_reports_sequence_and_autoload_types() {
    use quack_rs::error_data::{DuckDbErrorType, ErrorData};

    let _fx = Fixture::open();
    for (raw, expected) in [
        (40, DuckDbErrorType::Autoload),
        (41, DuckDbErrorType::Sequence),
        (42, DuckDbErrorType::InvalidConfiguration),
    ] {
        // SAFETY: creates an owned error-data handle that `ErrorData` frees.
        let err = unsafe {
            ErrorData::from_raw(libduckdb_sys::duckdb_create_error_data(
                raw,
                c"probe".as_ptr(),
            ))
        };
        assert_eq!(err.error_type(), expected, "raw type {raw}");
        let made = ErrorData::new(expected, "probe");
        assert_eq!(made.error_type(), expected);
    }
}

// ── Finding 9: DbConfig::set accepts unknown names ──────────────────────────

#[cfg(feature = "duckdb-1-5")]
#[test]
fn db_config_set_accepts_an_unknown_name_and_opening_rejects_it() {
    use quack_rs::config::DbConfig;
    use quack_rs::instance_cache::InstanceCache;

    let _fx = Fixture::open();
    // Pinned: DuckDB stores unknown names in `unrecognized_options`.
    let config = DbConfig::new()
        .expect("config")
        .set("vq_definitely_not_an_option", "1")
        .expect("duckdb_set_config accepts an unknown name");
    // The open is what fails.
    let cache = InstanceCache::new();
    let err = cache
        .get_or_create(c":memory:vq_unknown_option", Some(&config))
        .expect_err("opening with an unknown option must fail");
    assert!(
        err.as_str().contains("vq_definitely_not_an_option"),
        "{err}"
    );

    // A recognised option with an invalid value is rejected by `set`.
    let bad = DbConfig::new().expect("config").set("threads", "lots");
    assert!(bad.is_err(), "a non-numeric thread count must be rejected");
}

// ── Finding 3: Expression::fold of a non-foldable expression ───────────────

#[cfg(feature = "duckdb-1-5")]
mod fold_probe {
    use std::sync::Mutex;

    /// What the bind callback saw, one entry per bind.
    pub static OUTCOMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
}

#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn vq_fold_bind(info: libduckdb_sys::duckdb_bind_info) {
    use quack_rs::scalar::ScalarBindInfo;

    // SAFETY: DuckDB passes a valid bind info.
    let bind = unsafe { ScalarBindInfo::new(info) };
    // SAFETY: two parameters were declared, so argument 1 exists.
    let Some(expr) = (unsafe { bind.argument(1) }) else {
        return;
    };
    // SAFETY: inside a bind callback, so the context is live.
    let ctx = unsafe { bind.get_client_context() };
    let outcome = match expr.fold(&ctx) {
        Err(e) => format!("err: {e}"),
        Ok(v) if v.is_null() => "ok: null handle".to_owned(),
        Ok(v) => format!("ok: {:?}", v.as_i64()),
    };
    if let Ok(mut outcomes) = fold_probe::OUTCOMES.lock() {
        outcomes.push(outcome);
    }
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(vq_fold_exec, |_info, input, output| {
    use quack_rs::data_chunk::DataChunk;
    use quack_rs::vector::VectorWriter;

    // SAFETY: DuckDB passes a valid chunk and a BIGINT output vector.
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // SAFETY: `row` is within the chunk.
        unsafe { writer.write_i64(row, 0) };
    }
});

/// `duckdb_expression_fold` returns no error *and* no value for an expression
/// that is not foldable; `fold` used to wrap that as `Ok` with a null handle.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn fold_of_a_non_foldable_expression_is_an_error() {
    use quack_rs::scalar::ScalarFunctionBuilder;

    let fx = Fixture::open();
    // SAFETY: `con` is open; every callback matches its declared signature.
    unsafe {
        ScalarFunctionBuilder::try_new("vq_foldprobe")
            .expect("name")
            .param(TypeId::BigInt)
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .null_handling(quack_rs::types::NullHandling::SpecialNullHandling)
            .bind(vq_fold_bind)
            .function(vq_fold_exec)
            .register(fx.con())
            .expect("register vq_foldprobe");
    }
    let run = |sql: &str| -> String {
        fold_probe::OUTCOMES.lock().expect("lock").clear();
        drop(fx.query(sql));
        fold_probe::OUTCOMES
            .lock()
            .expect("lock")
            .first()
            .cloned()
            .expect("bind ran")
    };

    let column = run("SELECT vq_foldprobe(i, i) FROM range(3) t(i)");
    assert!(column.starts_with("err: "), "{column}");
    assert!(column.contains("not foldable"), "{column}");

    assert_eq!(run("SELECT vq_foldprobe(1, 20 + 1)"), "ok: Some(21)");
    // A folded NULL is a real value holding SQL NULL, read as `None`.
    assert_eq!(run("SELECT vq_foldprobe(1, NULL::BIGINT)"), "ok: None");
}

// ── Finding 4: ArrowOptions borrow their connection ────────────────────────

/// The positive half of the lifetime fix (the negative half is the
/// `compile_fail` doctest on `ArrowOptions`): options borrowed from an
/// `OwnedConnection` convert a chunk, and are dropped before it. Clean under
/// valgrind and `AddressSanitizer`.
#[cfg(feature = "duckdb-1-5-4")]
#[test]
fn arrow_options_from_an_owned_connection_convert_a_chunk() {
    use quack_rs::arrow::{data_chunk_to_arrow, ArrowOptions};

    let fx = Fixture::open();
    let con = owned_connection(&fx);
    let options = ArrowOptions::from_connection(&con).expect("arrow options");
    let mut result = con.query("SELECT 42::BIGINT AS a").expect("query");
    let chunk = result.next_chunk().expect("one chunk");
    let mut array = data_chunk_to_arrow(&options, &chunk).expect("to arrow");
    assert_eq!(array.len(), 1);
    array.release();
    drop(chunk);
    drop(result);
    drop(options);
    drop(con);
    assert_eq!(first_varchar(&owned_connection(&fx), "SELECT 'x'"), "x");
}
