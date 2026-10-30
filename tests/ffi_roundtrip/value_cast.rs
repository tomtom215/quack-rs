// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `Value::try_cast`, `array_items` and `union_tag`: reading the `ARRAY` and
//! `UNION` values the C API has no reader for (eighth review, APIGAPS-2).
//!
//! Before these existed an `ARRAY` value read as empty (`list_len() == 0`)
//! and a `UNION` as having no fields (`struct_child(i)` is `None`), so a
//! table-function, `COPY` or scalar-bind argument of either type could not
//! be read at all.

use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;

use super::Fixture;

fn ints(values: &[Value]) -> Vec<Option<i32>> {
    values.iter().map(Value::as_i32).collect()
}

/// The reviewer's reproduction: `list_len` of an `ARRAY` is 0 (the C API
/// reads a `LIST` only, which is still true and pinned here), and the
/// elements are now readable through `array_items` and `try_cast`.
#[test]
fn an_array_value_is_readable() {
    let _fx = Fixture::open();
    let int = LogicalType::new(TypeId::Integer);
    let array = Value::array_value(
        &int,
        &[Value::integer(1), Value::integer(2), Value::integer(3)],
    )
    .expect("INTEGER[3]");
    assert_eq!(array.type_id(), Some(TypeId::Array));
    assert_eq!(array.list_len(), 0, "the C API's LIST reader sees nothing");

    let items = array.array_items().expect("an ARRAY has items");
    assert_eq!(ints(&items), [Some(1), Some(2), Some(3)]);

    let as_list = array
        .try_cast(&LogicalType::list(TypeId::Integer))
        .expect("ARRAY(INTEGER, 3) casts to INTEGER[]");
    assert_eq!(as_list.type_id(), Some(TypeId::List));
    assert_eq!(ints(&as_list.list_items()), [Some(1), Some(2), Some(3)]);
    // The element cast happens too.
    let texts: Vec<String> = array
        .try_cast(&LogicalType::list(TypeId::Varchar))
        .expect("INTEGER[3] casts to VARCHAR[]")
        .list_items()
        .iter()
        .map(|v| v.as_str().expect("text"))
        .collect();
    assert_eq!(texts, ["1", "2", "3"]);
    // The array itself is untouched.
    assert_eq!(array.type_id(), Some(TypeId::Array));
    assert_eq!(
        ints(&array.array_items().expect("still an ARRAY")),
        [Some(1), Some(2), Some(3)]
    );
}

/// Every refusal is `None`, not an abort.
#[test]
fn failed_casts_and_non_arrays_are_none() {
    let _fx = Fixture::open();
    let int = LogicalType::new(TypeId::Integer);
    let array = Value::array_value(&int, &[Value::integer(1), Value::integer(2)]).expect("[1, 2]");
    assert!(array.try_cast(&LogicalType::new(TypeId::Integer)).is_none());
    assert!(
        array
            .try_cast(&LogicalType::array(TypeId::Integer, 3))
            .is_none(),
        "an ARRAY of the wrong size"
    );
    assert!(Value::varchar("abc").try_cast(&int).is_none());
    assert!(Value::varchar("[1, x]")
        .try_cast(&LogicalType::list(TypeId::Integer))
        .is_none());
    assert!(Value::varchar("1")
        .try_cast(&LogicalType::new(TypeId::Any))
        .is_none());

    // A LIST is read like an ARRAY; a scalar, a SQL NULL and a null handle
    // have no items.
    let list = Value::list_value(&int, &[Value::integer(9)]).expect("[9]");
    assert_eq!(
        ints(&list.array_items().expect("a LIST has items")),
        [Some(9)]
    );
    assert!(Value::integer(1).array_items().is_none());
    assert!(Value::null_value().array_items().is_none());
    // SAFETY: a null handle is part of `Value`'s contract.
    let null_handle = unsafe { Value::from_raw(std::ptr::null_mut()) };
    assert!(null_handle.array_items().is_none());
    assert!(null_handle.try_cast(&int).is_none());
}

/// `try_cast` is SQL's `TRY_CAST`: scalars, parsing, and a `NULL` that
/// stays `NULL` at the target type.
#[test]
fn try_cast_casts_scalars_and_parses_text_as_sql_does() {
    let _fx = Fixture::open();
    let date = Value::varchar("2024-02-29")
        .try_cast(&LogicalType::new(TypeId::Date))
        .expect("a valid date");
    assert_eq!(date.type_id(), Some(TypeId::Date));
    assert_eq!(date.as_date(), Some(19_782));

    let list = Value::varchar("[1, 2, 3]")
        .try_cast(&LogicalType::list(TypeId::BigInt))
        .expect("parses as a list");
    assert_eq!(
        list.list_items()
            .iter()
            .map(Value::as_i64)
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3)]
    );

    let null = Value::null_value()
        .try_cast(&LogicalType::new(TypeId::Integer))
        .expect("NULL casts to a typed NULL");
    assert!(null.is_sql_null());
    assert_eq!(null.type_id(), Some(TypeId::Integer));
}

/// A timestamp payload outside `DuckDB`'s range is refused before any cast
/// of it — inside an `ARRAY` too, whose elements are checked after an
/// element-preserving cast to `LIST` — while `array_items` still hands the
/// element out for the guarded getters to refuse.
#[test]
fn an_out_of_range_payload_inside_an_array_is_not_cast() {
    let _fx = Fixture::open();
    // SAFETY: DuckDB returns an owned value for any payload.
    let bad = unsafe {
        Value::from_raw(libduckdb_sys::duckdb_create_timestamp(
            libduckdb_sys::duckdb_timestamp { micros: i64::MIN },
        ))
    };
    let ts = LogicalType::new(TypeId::Timestamp);
    let array = Value::array_value(&ts, &[bad]).expect("TIMESTAMP[1]");
    assert!(array
        .try_cast(&LogicalType::list(TypeId::Varchar))
        .is_none());
    assert!(array.try_cast(&LogicalType::new(TypeId::Varchar)).is_none());
    let items = array
        .array_items()
        .expect("the element is handed out unchanged");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].type_id(), Some(TypeId::Timestamp));
    assert_eq!(items[0].as_timestamp(), None);
    assert!(items[0].as_str().is_err());

    // An in-range one casts.
    let good = Value::timestamp(0).expect("epoch");
    let array = Value::array_value(&ts, &[good]).expect("TIMESTAMP[1]");
    let texts = array
        .try_cast(&LogicalType::list(TypeId::Varchar))
        .expect("casts");
    assert_eq!(
        texts.list_items()[0].as_str().expect("text"),
        "1970-01-01 00:00:00"
    );
}

/// The motivating case, on every engine: a table function whose named
/// parameters are an `ARRAY` and a `UNION` reads both in its bind callback.
#[test]
fn array_and_union_parameters_of_a_table_function_are_readable() {
    let fx = Fixture::open();
    let union_type = LogicalType::union_type(&[("i", TypeId::Integer), ("s", TypeId::Varchar)]);
    let table_fn = TableFunctionBuilder::new("vc_params")
        .named_param_logical("arr", LogicalType::array(TypeId::Integer, 3))
        .named_param_logical("u", union_type)
        .with_state::<Option<(i64, String)>, _>(|bind| {
            bind.add_result_column("total", TypeId::BigInt);
            bind.add_result_column("member", TypeId::Varchar);
            // SAFETY: both names were declared above.
            let arr = unsafe { bind.get_named_parameter_value("arr") };
            // SAFETY: as above.
            let u = unsafe { bind.get_named_parameter_value("u") };
            let total = arr
                .array_items()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_i64)
                .sum();
            let member = u
                .try_cast(&LogicalType::new(TypeId::Varchar))
                .and_then(|v| v.as_str().ok())
                .unwrap_or_else(|| "<unreadable>".to_owned());
            Ok(Some((total, member)))
        })
        .scan(|state, chunk| {
            match state.take() {
                // SAFETY: two columns as declared; row 0 is in range.
                Some((total, member)) => unsafe {
                    chunk.writer(0).write_i64(0, total);
                    chunk.writer(1).write_varchar(0, &member);
                    chunk.set_size(1);
                },
                // SAFETY: ending the scan.
                None => unsafe { chunk.set_size(0) },
            }
            Ok(())
        })
        .build()
        .expect("build vc_params");
    // SAFETY: `con` is open.
    unsafe { table_fn.register(fx.con()) }.expect("register vc_params");

    let total = fx.scalar(
        "SELECT total FROM vc_params(arr := [4, 5, 6]::INTEGER[3], \
         u := union_value(s := 'hello')::UNION(i INTEGER, s VARCHAR))",
        |r, i| unsafe { r.read_i64(i) },
    );
    assert_eq!(total, Some(15));
    let member = fx.scalar(
        "SELECT member FROM vc_params(arr := [1, 1, 1]::INTEGER[3], \
         u := union_value(i := 42)::UNION(i INTEGER, s VARCHAR))",
        |r, i| unsafe { r.read_str(i).to_owned() },
    );
    assert_eq!(member.as_deref(), Some("42"));
    let member = fx.scalar(
        "SELECT member FROM vc_params(arr := [1, 1, 1]::INTEGER[3], \
         u := union_value(s := 'hello')::UNION(i INTEGER, s VARCHAR))",
        |r, i| unsafe { r.read_str(i).to_owned() },
    );
    assert_eq!(member.as_deref(), Some("hello"));
}

/// `union_tag` names the active member; `try_cast` to `VARCHAR` reads it.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_union_value_reports_its_tag_and_reads_as_text() {
    let _fx = Fixture::open();
    let union_type = LogicalType::union_type(&[
        ("i", TypeId::Integer),
        ("s", TypeId::Varchar),
        ("l", TypeId::BigInt),
    ]);
    let varchar = LogicalType::new(TypeId::Varchar);
    let text = |v: &Value| v.try_cast(&varchar).and_then(|t| t.as_str().ok());

    let s = Value::union_value(&union_type, 1, &Value::varchar("hello")).expect("s := 'hello'");
    // Still true: the C API reads no field of a UNION.
    assert!(s.struct_child(0).is_none() && s.struct_child(2).is_none());
    assert_eq!(s.union_tag(), Some(1));
    assert_eq!(s.struct_field_names(), ["", "i", "s", "l"]);
    assert_eq!(text(&s).as_deref(), Some("hello"));

    let i = Value::union_value(&union_type, 0, &Value::integer(-7)).expect("i := -7");
    assert_eq!(i.union_tag(), Some(0));
    assert_eq!(text(&i).as_deref(), Some("-7"));
    assert_eq!(
        i.try_cast(&varchar).and_then(|t| t.as_i32()),
        Some(-7),
        "and back through the text to the member's type"
    );

    let l = Value::union_value(&union_type, 2, &Value::bigint(1 << 40)).expect("l := 2^40");
    assert_eq!(l.union_tag(), Some(2));
    assert_eq!(text(&l).as_deref(), Some("1099511627776"));

    // A NULL member: the tag is still there, and the text is "NULL".
    let null_member = Value::union_value(
        &union_type,
        1,
        &Value::null_value()
            .try_cast(&varchar)
            .expect("NULL::VARCHAR"),
    )
    .expect("s := NULL");
    assert!(!null_member.is_sql_null());
    assert_eq!(null_member.union_tag(), Some(1));
    assert_eq!(text(&null_member).as_deref(), Some("NULL"));

    // No tag for anything else.
    assert_eq!(Value::integer(1).union_tag(), None);
    assert_eq!(Value::null_value().union_tag(), None);
    // SAFETY: a null handle is part of `Value`'s contract.
    assert_eq!(
        unsafe { Value::from_raw(std::ptr::null_mut()) }.union_tag(),
        None
    );
    // A UNION cast to another UNION keeps its member (and its tag by name).
    let wider = LogicalType::union_type(&[
        ("x", TypeId::Double),
        ("i", TypeId::Integer),
        ("s", TypeId::Varchar),
        ("l", TypeId::BigInt),
    ]);
    let moved = s.try_cast(&wider).expect("UNION to a wider UNION");
    assert_eq!(
        moved.union_tag(),
        Some(2),
        "member `s` is the third one there"
    );
    assert_eq!(text(&moved).as_deref(), Some("hello"));
}

/// A `UNION` read through SQL — the shape a parameter has — reports the same.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_union_from_sql_reports_its_tag() {
    let fx = Fixture::open();
    let table_fn = TableFunctionBuilder::new("vc_union_tag")
        .param_logical(LogicalType::union_type(&[
            ("a", TypeId::Integer),
            ("b", TypeId::Varchar),
        ]))
        .with_state::<Option<i64>, _>(|bind| {
            bind.add_result_column("tag", TypeId::BigInt);
            // SAFETY: parameter 0 was declared above.
            let u = unsafe { bind.get_parameter_value(0) };
            Ok(Some(u.union_tag().map_or(-1, i64::from)))
        })
        .scan(|state, chunk| {
            match state.take() {
                // SAFETY: one BIGINT column; row 0 is in range.
                Some(tag) => unsafe {
                    chunk.writer(0).write_i64(0, tag);
                    chunk.set_size(1);
                },
                // SAFETY: ending the scan.
                None => unsafe { chunk.set_size(0) },
            }
            Ok(())
        })
        .build()
        .expect("build vc_union_tag");
    // SAFETY: `con` is open.
    unsafe { table_fn.register(fx.con()) }.expect("register vc_union_tag");
    let tag = |sql: &str| fx.scalar(sql, |r, i| unsafe { r.read_i64(i) });
    assert_eq!(
        tag("SELECT * FROM vc_union_tag(union_value(b := 'x')::UNION(a INTEGER, b VARCHAR))"),
        Some(1)
    );
    assert_eq!(
        tag("SELECT * FROM vc_union_tag(union_value(a := 3)::UNION(a INTEGER, b VARCHAR))"),
        Some(0)
    );
    assert_eq!(
        tag("SELECT * FROM vc_union_tag(NULL::UNION(a INTEGER, b VARCHAR))"),
        Some(-1),
        "a NULL UNION has no tag"
    );
}
