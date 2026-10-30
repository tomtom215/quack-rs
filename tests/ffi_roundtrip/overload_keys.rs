// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Overloads `DuckDB`'s overload resolution cannot tell apart although their
//! types differ as written (AUTHT2-1, reviewer `author-types`): it compares a
//! `DECIMAL`, `MAP` or `ENUM` parameter by type id alone, also inside a
//! `LIST`, `ARRAY` or `STRUCT`, and named `STRUCT` fields by name in any
//! order and case. `DuckDB` registers such a set and then fails every call to
//! it with "Could not choose a best candidate function"; quack-rs refuses it
//! at registration, while overloads `DuckDB` does resolve still register and
//! answer each call with the right overload.
//!
//! Also: closure-built functions as overloads of one set (AUTHT2-2), which
//! is the only way to overload them on `DuckDB` before v1.5.0.

use libduckdb_sys::{duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, idx_t};
use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
use quack_rs::data_chunk::DataChunk;
use quack_rs::query::query;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

/// The part of the refusal that explains why the types collide.
const RULE: &str = "ignores a DECIMAL's width and scale";

macro_rules! answer {
    ($name:ident, $value:expr) => {
        quack_rs::scalar_callback!($name, |_info, input, output| {
            let chunk = unsafe { DataChunk::from_raw(input) };
            let mut writer = unsafe { VectorWriter::from_vector(output) };
            for row in 0..chunk.size() {
                unsafe { writer.write_i64(row, $value) };
            }
        });
    };
}
answer!(answer0, 0);
answer!(answer1, 1);

fn overload(param: LogicalType, answer: quack_rs::scalar::ScalarFn) -> ScalarOverloadBuilder {
    ScalarOverloadBuilder::new()
        .param_logical(param)
        .returns(TypeId::BigInt)
        .function(answer)
}

/// What a call `DuckDB` fails says, for a test that expected a refusal and
/// got a registration instead.
fn call_error(fx: &Fixture, sql: &str) -> String {
    // SAFETY: the fixture's connection is open.
    match unsafe { query(fx.con(), sql) } {
        Ok(_) => format!("{sql} answered"),
        Err(e) => format!("{sql} failed: {e}"),
    }
}

/// Each pair is refused with the reason. Registered (as before the fix), a
/// call with exactly the first overload's argument type fails as ambiguous,
/// which the panic message reports.
#[test]
fn scalar_sets_duckdb_resolves_by_type_id_are_refused() {
    let fx = Fixture::open();
    let struct_of = |fields: &[(&str, LogicalType)]| LogicalType::struct_type_from_logical(fields);
    let pairs: Vec<(&str, LogicalType, LogicalType, &str)> = vec![
        (
            "ok_dec",
            LogicalType::decimal(4, 1),
            LogicalType::decimal(9, 3),
            "1.5::DECIMAL(4,1)",
        ),
        (
            "ok_map",
            LogicalType::map(TypeId::Varchar, TypeId::BigInt),
            LogicalType::map(TypeId::BigInt, TypeId::Varchar),
            "MAP {'k': 1::BIGINT}",
        ),
        (
            "ok_enum",
            LogicalType::enum_type(&["a", "b"]),
            LogicalType::enum_type(&["x"]),
            "'a'::ENUM('a', 'b')",
        ),
        (
            "ok_list_dec",
            LogicalType::list_from_logical(&LogicalType::decimal(4, 1)),
            LogicalType::list_from_logical(&LogicalType::decimal(18, 3)),
            "[1.5::DECIMAL(4,1)]",
        ),
        (
            "ok_array_dec",
            LogicalType::array_from_logical(&LogicalType::decimal(4, 1), 2),
            LogicalType::array_from_logical(&LogicalType::decimal(18, 3), 2),
            "[1.5, 2.5]::DECIMAL(4,1)[2]",
        ),
        (
            "ok_struct_dec",
            struct_of(&[("a", LogicalType::decimal(4, 1))]),
            struct_of(&[("a", LogicalType::decimal(18, 3))]),
            "{'a': 1.5::DECIMAL(4,1)}",
        ),
        (
            "ok_list_map",
            LogicalType::list_from_logical(&LogicalType::map(TypeId::Varchar, TypeId::BigInt)),
            LogicalType::list_from_logical(&LogicalType::map(TypeId::BigInt, TypeId::BigInt)),
            "[MAP {'k': 1::BIGINT}]",
        ),
        (
            "ok_struct_order",
            LogicalType::struct_type(&[("a", TypeId::Integer), ("b", TypeId::Varchar)]),
            LogicalType::struct_type(&[("B", TypeId::Varchar), ("A", TypeId::Integer)]),
            "{'a': 1::INTEGER, 'b': 'x'}",
        ),
    ];
    for (name, first, second, argument) in pairs {
        let set = ScalarFunctionSetBuilder::try_new(name)
            .expect("name")
            .overload(overload(first, answer0))
            .overload(overload(second, answer1));
        // SAFETY: `con` is open; the callbacks match the declared signatures.
        match unsafe { set.register(fx.con()) } {
            Err(e) => {
                assert!(
                    e.as_str().contains("overload 0 (")
                        && e.as_str().contains("accept the same arguments")
                        && e.as_str().contains(RULE),
                    "{name}: {e}"
                );
            }
            Ok(()) => panic!(
                "{name}: registered; then {}",
                call_error(&fx, &format!("SELECT {name}({argument})"))
            ),
        }
    }
}

/// Overloads `DuckDB` does tell apart still register, and each call reaches
/// the overload for its argument type.
#[test]
fn overloads_duckdb_tells_apart_still_register_and_resolve() {
    let fx = Fixture::open();
    let cases: Vec<(&str, LogicalType, LogicalType, &str, &str)> = vec![
        (
            "ok_int",
            LogicalType::new(TypeId::Integer),
            LogicalType::new(TypeId::BigInt),
            "1::INTEGER",
            "1::BIGINT",
        ),
        (
            "ok_list",
            LogicalType::list(TypeId::Integer),
            LogicalType::list(TypeId::Varchar),
            "[1]::INTEGER[]",
            "['a']",
        ),
        (
            "ok_array_size",
            LogicalType::array(TypeId::BigInt, 3),
            LogicalType::array(TypeId::BigInt, 4),
            "[1, 2, 3]::BIGINT[3]",
            "[1, 2, 3, 4]::BIGINT[4]",
        ),
        (
            "ok_struct_names",
            LogicalType::struct_type(&[("a", TypeId::BigInt)]),
            LogicalType::struct_type(&[("b", TypeId::Varchar)]),
            "{'a': 1::BIGINT}",
            "{'b': 'x'}",
        ),
        (
            "ok_decimal_bigint",
            LogicalType::decimal(4, 1),
            LogicalType::new(TypeId::BigInt),
            "1.5::DECIMAL(4,1)",
            "1::BIGINT",
        ),
    ];
    for (name, first, second, first_arg, second_arg) in cases {
        let set = ScalarFunctionSetBuilder::try_new(name)
            .expect("name")
            .overload(overload(first, answer0))
            .overload(overload(second, answer1));
        // SAFETY: `con` is open; the callbacks match the declared signatures.
        unsafe { set.register(fx.con()) }.unwrap_or_else(|e| panic!("{name}: {e}"));
        for (argument, expected) in [(first_arg, 0), (second_arg, 1)] {
            let got = fx.scalar(&format!("SELECT {name}({argument})"), |r, i| unsafe {
                r.read_i64(i)
            });
            assert_eq!(got, Some(expected), "{name}({argument})");
        }
    }
}

mod dummy_aggregate {
    use super::{duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, idx_t};
    use libduckdb_sys::duckdb_vector;

    pub const unsafe extern "C" fn size(_: duckdb_function_info) -> idx_t {
        8
    }
    pub const unsafe extern "C" fn init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
    pub const unsafe extern "C" fn update(
        _: duckdb_function_info,
        _: duckdb_data_chunk,
        _: *mut duckdb_aggregate_state,
    ) {
    }
    pub const unsafe extern "C" fn combine(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: *mut duckdb_aggregate_state,
        _: idx_t,
    ) {
    }
    pub const unsafe extern "C" fn finalize(
        _: duckdb_function_info,
        _: *mut duckdb_aggregate_state,
        _: duckdb_vector,
        _: idx_t,
        _: idx_t,
    ) {
    }
}

/// The aggregate set builder shares the check: the reviewer's
/// `DECIMAL(4,1)` / `DECIMAL(18,3)` aggregate failed every call on all
/// three engines.
#[test]
fn an_aggregate_set_duckdb_resolves_by_type_id_is_refused() {
    let fx = Fixture::open();
    let agg = |param: LogicalType| {
        AggregateOverloadBuilder::new()
            .param_logical(param)
            .state_size(dummy_aggregate::size)
            .init(dummy_aggregate::init)
            .update(dummy_aggregate::update)
            .combine(dummy_aggregate::combine)
            .finalize(dummy_aggregate::finalize)
    };
    let set = AggregateFunctionSetBuilder::try_new("ok_agg_dec")
        .expect("name")
        .returns(TypeId::BigInt)
        .overload(agg(LogicalType::decimal(4, 1)))
        .overload(agg(LogicalType::decimal(18, 3)));
    // SAFETY: `con` is open.
    match unsafe { set.register(fx.con()) } {
        Err(e) => assert!(
            e.as_str()
                .contains("overload 0 (DECIMAL(4,1)) and overload 1 (DECIMAL(18,3))")
                && e.as_str().contains(RULE),
            "{e}"
        ),
        Ok(()) => panic!(
            "registered; then {}",
            call_error(&fx, "SELECT ok_agg_dec(1.5::DECIMAL(4,1))")
        ),
    }
}

/// The catalog check compares the same way: `round(DECIMAL(18,3))` beside
/// the built-in `round(DECIMAL)` would make every `round` of a `DECIMAL` in
/// the database ambiguous, and an extension's own two registrations of
/// `MAP`s collide the same way. Before v1.5.0 any existing name is refused.
#[test]
fn a_registration_duckdb_would_resolve_by_type_id_against_the_catalog_is_refused() {
    let fx = Fixture::open();
    let merges = super::engine_merges_scalar_overloads(&fx);
    let single = |name: &str, param: LogicalType| {
        ScalarFunctionBuilder::try_new(name)
            .expect("name")
            .param_logical(param)
            .returns(TypeId::BigInt)
            .function(answer0)
    };
    let expect_refused =
        |result: Result<(), quack_rs::error::ExtensionError>, call: &str| match result {
            Err(e) if merges => assert!(
                e.as_str()
                    .contains("accepts the same arguments as the existing")
                    && e.as_str().contains(RULE),
                "{e}"
            ),
            Err(e) => assert!(
                e.as_str()
                    .contains("cannot add an overload to an existing name"),
                "{e}"
            ),
            Ok(()) => panic!("registered; then {}", call_error(&fx, call)),
        };
    // SAFETY: `con` is open; the callback matches the declared signature.
    let result = unsafe { single("round", LogicalType::decimal(18, 3)).register(fx.con()) };
    expect_refused(result, "SELECT round(1.5::DECIMAL(18,3))");
    // The built-in is untouched.
    let rounded = fx.scalar("SELECT round(1.5::DECIMAL(4,1))::VARCHAR", |r, i| unsafe {
        r.read_str(i).to_owned()
    });
    assert_eq!(rounded.as_deref(), Some("2"));

    // SAFETY: as above.
    unsafe {
        single(
            "ok_cat_map",
            LogicalType::map(TypeId::Varchar, TypeId::BigInt),
        )
        .register(fx.con())
    }
    .expect("a fresh name");
    // SAFETY: as above.
    let result = unsafe {
        single(
            "ok_cat_map",
            LogicalType::map(TypeId::BigInt, TypeId::Varchar),
        )
        .register(fx.con())
    };
    expect_refused(result, "SELECT ok_cat_map(MAP {'k': 1::BIGINT})");
}

/// Three closures, one name: each call reaches the closure for its argument
/// type and returns that type, on every engine (a set is one registration).
/// One closure-built function per name, registered one after another, is
/// refused before v1.5.0, with a message naming `overload_typed`.
#[test]
fn closure_built_functions_overload_one_name_through_a_set() {
    let fx = Fixture::open();
    let set = ScalarFunctionSetBuilder::try_new("typed_twice")
        .expect("name")
        .overload_typed(
            ScalarFunctionBuilder::map1("typed_twice", |x: i32| x.wrapping_mul(2)).expect("i32"),
        )
        .overload_typed(
            ScalarFunctionBuilder::map1("typed_twice", |x: i64| x.wrapping_mul(2)).expect("i64"),
        )
        .overload_typed(ScalarFunctionBuilder::map1("typed_twice", |x: f64| x * 2.0).expect("f64"));
    // SAFETY: `con` is open.
    unsafe { set.register(fx.con()) }.expect("register the set");
    let text = |sql: &str| {
        fx.scalar(sql, |r, i| unsafe { r.read_str(i).to_owned() })
            .expect("not NULL")
    };
    for (argument, expected) in [
        ("21::INTEGER", "42 INTEGER"),
        ("21::BIGINT", "42 BIGINT"),
        ("1.25::DOUBLE", "2.5 DOUBLE"),
    ] {
        assert_eq!(
            text(&format!(
                "SELECT typed_twice({argument})::VARCHAR || ' ' || typeof(typed_twice({argument}))"
            )),
            expected,
            "{argument}"
        );
    }

    // One by one: a second registration under the name is an overload from
    // v1.5.0, refused (with the way out) before it.
    let one = |name| ScalarFunctionBuilder::map1(name, |x: i32| x).expect("i32");
    let other = |name| ScalarFunctionBuilder::map1(name, |x: i64| x).expect("i64");
    // SAFETY: `con` is open.
    unsafe { one("typed_once").register(fx.con()) }.expect("a fresh name");
    // SAFETY: as above.
    let second = unsafe { other("typed_once").register(fx.con()) };
    if super::engine_merges_scalar_overloads(&fx) {
        second.expect("an overload of an existing name");
    } else {
        let err = second.expect_err("an existing name before v1.5.0");
        assert!(err.as_str().contains("`overload_typed`"), "{err}");
    }
}

/// An aggregate cannot take a name any scalar function, aggregate or macro
/// already has, and the C API says only `DuckDBError` (FFI-8). The failure
/// now names the entry that holds the name, for the single builder and the
/// set builder, built-ins and the extension's own earlier registration alike.
#[test]
fn an_aggregate_under_a_taken_name_is_refused_naming_the_entry() {
    let fx = Fixture::open();
    let single = |name: &str| {
        quack_rs::aggregate::AggregateFunctionBuilder::try_new(name)
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(dummy_aggregate::size)
            .init(dummy_aggregate::init)
            .update(dummy_aggregate::update)
            .combine(dummy_aggregate::combine)
            .finalize(dummy_aggregate::finalize)
    };
    let set = |name: &str| {
        AggregateFunctionSetBuilder::try_new(name)
            .expect("name")
            .returns(TypeId::BigInt)
            .overload(
                AggregateOverloadBuilder::new()
                    .param(TypeId::BigInt)
                    .state_size(dummy_aggregate::size)
                    .init(dummy_aggregate::init)
                    .update(dummy_aggregate::update)
                    .combine(dummy_aggregate::combine)
                    .finalize(dummy_aggregate::finalize),
            )
    };
    for (name, holder) in [
        ("sum", "an aggregate function named 'sum' already exists ("),
        ("lower", "a scalar function named 'lower' already exists ("),
        ("list_sum", "a macro named 'list_sum' already exists ("),
    ] {
        // SAFETY: `con` is open; the callbacks match their signatures.
        let err = unsafe { single(name).register(fx.con()) }.expect_err("a taken name");
        assert!(err.as_str().contains(holder), "{name}: {err}");
        // SAFETY: as above.
        let err = unsafe { set(name).register(fx.con()) }.expect_err("a taken name");
        assert!(err.as_str().contains(holder), "{name} (set): {err}");
    }
    // SAFETY: as above.
    unsafe { single("agg_once").register(fx.con()) }.expect("a fresh name");
    // SAFETY: as above.
    let err = unsafe { set("AGG_ONCE").register(fx.con()) }.expect_err("taken by itself");
    assert!(
        err.as_str()
            .contains("an aggregate function named 'agg_once' already exists (")
            && err
                .as_str()
                .contains("cannot register an aggregate under a taken name"),
        "{err}"
    );
}
