// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Behaviour the rustdoc of `query`, `PreparedStatement`, `DbConfig` and
//! `Value::struct_field_names` describes, pinned against a live `DuckDB` so
//! the docs cannot drift from it again (audit F-V9 c–f).

use quack_rs::config::DbConfig;
use quack_rs::query::OwnedConnection;
#[cfg(feature = "duckdb-1-5")]
use quack_rs::types::{LogicalType, TypeId};
#[cfg(feature = "duckdb-1-5")]
use quack_rs::value::Value;

use super::Fixture;

fn connect(fx: &Fixture) -> OwnedConnection {
    // SAFETY: the fixture's database outlives the connection.
    unsafe { OwnedConnection::open(fx.db()) }.expect("connect")
}

/// F-V9e: a string with several statements runs every one of them in order.
/// The result is the first statement that produces rows — or, if none does,
/// the last statement's; `DuckDB` chains any later row-producing results where
/// the C API cannot reach them. An empty string is not an error.
#[test]
fn multi_statement_strings_run_every_statement_and_return_the_last() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let mut result = con
        .query("CREATE TABLE ms (i INTEGER); INSERT INTO ms VALUES (1), (2); SELECT count(*) AS n FROM ms")
        .expect("three statements");
    assert_eq!(result.column_name(0).as_deref(), Some("n"));
    let chunk = result.next_chunk().expect("fetch").expect("a row");
    // SAFETY: one BIGINT column, one row.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 2);

    let result = con
        .query("SELECT 1 AS first_col; SELECT 2 AS second_col")
        .expect("two selects");
    assert_eq!(result.column_name(0).as_deref(), Some("first_col"));

    // A later statement that fails fails the whole call, but the statements
    // before it have already run.
    assert!(con
        .query("INSERT INTO ms VALUES (3); SELECT error('second fails')")
        .is_err());
    // `execute` reports the rows changed by the statement whose result it
    // got: here the SELECT's (0), although the INSERT after it ran.
    assert_eq!(
        con.execute("SELECT 1; INSERT INTO ms VALUES (4)").ok(),
        Some(0)
    );
    assert_eq!(
        con.execute("INSERT INTO ms VALUES (5); INSERT INTO ms VALUES (6), (7)")
            .ok(),
        Some(2),
        "no statement produces rows, so the last one's result"
    );
    let mut count = con.query("SELECT count(*) FROM ms").expect("count");
    let chunk = count.next_chunk().expect("fetch").expect("a row");
    // SAFETY: one BIGINT column, one row.
    assert_eq!(
        unsafe { chunk.reader(0).read_i64(0) },
        7,
        "rows 1-7: every statement before the failing one ran"
    );

    // No statement at all is not an error.
    for sql in ["", ";", "  ;  "] {
        let result = con.query(sql).unwrap_or_else(|e| panic!("{sql:?}: {e}"));
        assert_eq!(result.column_count(), 0, "{sql:?}");
    }
    // A prepared statement takes exactly one statement.
    assert!(con.prepare("SELECT 1; SELECT 2").is_err());
}

/// F-V9f: `parameter_name` of a positional `?` is its 1-based position as
/// text, not `None`.
#[test]
fn positional_parameters_are_named_by_their_position() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let statement = con
        .prepare("SELECT ?::BIGINT + ?::BIGINT")
        .expect("prepare");
    assert_eq!(statement.parameter_name(1).as_deref(), Some("1"));
    assert_eq!(statement.parameter_name(2).as_deref(), Some("2"));
    assert_eq!(statement.parameter_name(0), None);
    assert_eq!(statement.parameter_name(3), None);
    let named = con.prepare("SELECT $foo || $Bar").expect("prepare");
    assert_eq!(named.parameter_name(1).as_deref(), Some("foo"));
    // Names keep their case; `parameter_index` matches them case-insensitively.
    assert_eq!(named.parameter_name(2).as_deref(), Some("Bar"));
    assert_eq!(named.parameter_index("BAR"), Some(2));
}

/// F-V9c: for an extension's setting, `get_flag`'s second string is the
/// extension's name, not a description (`duckdb_get_config_flag`'s third
/// branch, `config-c.cpp`).
#[test]
fn extension_settings_report_their_extension_instead_of_a_description() {
    let _fx = Fixture::open();
    let flags: Vec<(String, String)> = (0..DbConfig::flag_count())
        .map(|i| DbConfig::get_flag(i).expect("in range"))
        .collect();
    let threads = flags
        .iter()
        .find(|(name, _)| name == "threads")
        .expect("threads");
    assert!(
        threads.1.len() > 10,
        "a core option has a description: {threads:?}"
    );
    let s3 = flags
        .iter()
        .find(|(name, _)| name == "s3_region")
        .expect("httpfs's s3_region is listed");
    assert_eq!(s3.1, "httpfs");
    assert!(DbConfig::get_flag(DbConfig::flag_count()).is_err());
}

/// F-V9b: the docs said `DuckDB`'s UTF-8 check was stricter than Rust's. It
/// agrees with `std::str::from_utf8` on every Unicode scalar value and on
/// each class of malformed input.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn duckdb_utf8_validation_matches_rust() {
    use quack_rs::error_data::check_valid_utf8;

    let _fx = Fixture::open();
    let mut checked = 0_u32;
    let mut buffer = [0_u8; 4];
    for scalar in (0..=0x0010_FFFF_u32).filter_map(char::from_u32) {
        let bytes = scalar.encode_utf8(&mut buffer).as_bytes();
        assert!(
            check_valid_utf8(bytes).is_ok(),
            "U+{:04X}",
            u32::from(scalar)
        );
        checked += 1;
    }
    assert_eq!(checked, 1_112_064);
    let malformed: [&[u8]; 8] = [
        b"\xED\xA0\x80",     // a surrogate
        b"\xC0\x80",         // overlong, two bytes
        b"\xE0\x80\x80",     // overlong, three bytes
        b"\xF0\x80\x80\x80", // overlong, four bytes
        b"\xF4\x90\x80\x80", // above U+10FFFF
        b"\xF5\x80\x80\x80", // an invalid lead byte
        b"\xE2\x82",         // truncated
        b"\x80",             // a lone continuation byte
    ];
    for bytes in malformed {
        assert!(std::str::from_utf8(bytes).is_err());
        assert!(check_valid_utf8(bytes).is_err(), "{bytes:?}");
    }
}

/// F-V9d: a `UNION` value is stored as a struct of its tag and members, so
/// `struct_field_names` returns `""` for the tag, then the member names —
/// positionally aligned with `struct_child`.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn struct_field_names_of_a_union_include_its_tag() {
    let _fx = Fixture::open();
    let union_type = LogicalType::union_type(&[("a", TypeId::Integer), ("b", TypeId::Varchar)]);
    let value = Value::union_value(&union_type, 1, &Value::varchar("x")).expect("union");
    assert_eq!(value.struct_field_names(), ["", "a", "b"]);
    assert!(Value::bigint(1).struct_field_names().is_empty());
    let list = Value::list_value(&LogicalType::new(TypeId::BigInt), &[]).expect("list");
    assert!(list.struct_field_names().is_empty());
}
