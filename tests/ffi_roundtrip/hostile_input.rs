// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Hostile names, SQL bodies, nesting and hints against a real engine.
//!
//! A `canary` table sits in every database; a name or body that reached the
//! SQL parser as SQL would have a chance to drop or create a table, and the
//! assertions are that it never does. Nothing here is a claim about what
//! `DuckDB` does with the name beyond "registers it as a name or refuses it".

use quack_rs::connection::{Connection, Registrar};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::sql_macro::SqlMacro;
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::vector::VectorWriter;

use super::Fixture;

quack_rs::scalar_callback!(zero, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 0) };
    }
});

#[cfg(feature = "duckdb-1-5")]
mod copy_noop {
    quack_rs::copy_bind_callback!(bind, |_info| {});
    quack_rs::copy_global_init_callback!(init, |_info| {});
    quack_rs::copy_sink_callback!(sink, |_info, _chunk| {});
    quack_rs::copy_finalize_callback!(finalize, |_info| {});
}

/// Names that need quoting, carry SQL, or are otherwise hostile. (A NUL is
/// refused by every constructor and covered by their unit tests.)
fn hostile_names() -> Vec<String> {
    let mut names: Vec<String> = [
        "",
        " ",
        "a b",
        "a\"b",
        "a'b",
        "a;b",
        "a--b",
        "a/*b*/c",
        "a\nb",
        "a\tb",
        "ü",
        "日本",
        "😀",
        "ORDER",
        "select",
        "a.b",
        "main.x",
        "*",
        "?",
        "$1",
        "../x",
        "a%b",
        "\\",
        "'; DROP TABLE canary; --",
        "\"; drop table canary; --",
        "x\u{202e}y",
        "\u{feff}x",
        "1abc",
        "e\u{301}",
        "\r\n",
        "\u{7f}",
        "%s%n",
        "{}",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    names.push("x".repeat(300));
    names.push("x".repeat(70_000));
    names
}

fn fixture_with_canary() -> Fixture {
    let fx = Fixture::open();
    fx.query("CREATE TABLE canary(i INTEGER)");
    fx.query("INSERT INTO canary VALUES (1), (2), (3)");
    fx
}

fn count(fx: &Fixture, sql: &str) -> i64 {
    // SAFETY: one BIGINT column, row 0.
    fx.scalar(sql, |r, i| unsafe { r.read_i64(i) })
        .expect("a count")
}

/// Nothing a name does can reach the SQL parser: the canary survives, and no
/// table appears, whatever each constructor and registration makes of it.
fn assert_untouched(fx: &Fixture, what: &str) {
    assert_eq!(count(fx, "SELECT count(*) FROM canary"), 3, "{what}");
    assert_eq!(
        count(fx, "SELECT count(*) FROM duckdb_tables()"),
        1,
        "{what}: a table was created or dropped"
    );
}

#[test]
fn a_hostile_function_name_is_refused_or_registered_as_a_name_never_run() {
    for name in hostile_names() {
        let fx = fixture_with_canary();
        // SAFETY: the fixture's handles outlive `con`.
        let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };

        // The validated constructors refuse all of them.
        assert!(ScalarFunctionBuilder::try_new(&name).is_err(), "{name:?}");
        assert!(TableFunctionBuilder::try_new(&name).is_err(), "{name:?}");
        assert!(
            quack_rs::aggregate::AggregateFunctionBuilder::try_new(&name).is_err(),
            "{name:?}"
        );
        assert!(SqlMacro::scalar(&name, &["x"], "x").is_err(), "{name:?}");
        assert!(
            SqlMacro::table(&name, &["x"], "SELECT 1").is_err(),
            "{name:?}"
        );
        assert!(
            SqlMacro::scalar("m", &[name.as_str()], "1").is_err(),
            "{name:?}"
        );

        // The unvalidated `new` constructors reach the C API with the name
        // as given. It is a name there, never SQL: the registration either
        // fails (an empty name) or succeeds under that literal name.
        // SAFETY: the connection is valid for these calls.
        let registered = unsafe {
            con.register_scalar(
                ScalarFunctionBuilder::new(&name)
                    .param(TypeId::BigInt)
                    .returns(TypeId::BigInt)
                    .function(zero),
            )
        };
        assert_eq!(
            registered.is_ok(),
            !name.is_empty(),
            "{name:?}: {registered:?}"
        );
        let table = TableFunctionBuilder::new(&name)
            .with_state::<(), _>(|bind| {
                bind.add_result_column("x", TypeId::BigInt);
                Ok(())
            })
            .scan(|(), chunk| {
                // SAFETY: ending the scan.
                unsafe { chunk.set_size(0) };
                Ok(())
            })
            .build()
            .expect("build");
        // SAFETY: the connection is valid for this call.
        let registered = unsafe { con.register_table(table) };
        assert_eq!(
            registered.is_ok(),
            !name.is_empty(),
            "{name:?}: {registered:?}"
        );

        assert_untouched(&fx, &format!("function {name:.40?}"));
    }
}

#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_hostile_setting_or_format_name_is_registered_as_a_name_never_run() {
    use quack_rs::config_option::ConfigOptionBuilder;
    use quack_rs::copy_function::CopyFunctionBuilder;

    for name in hostile_names() {
        let fx = fixture_with_canary();
        // The name, the description and the default are all hostile text.
        let option = ConfigOptionBuilder::try_new(&name)
            .and_then(|b| b.option_type(TypeId::Varchar).description(&name))
            .and_then(|b| b.default_value(&name));
        // SAFETY: `con` is open.
        let registered = option.and_then(|b| unsafe { b.register(fx.con()) });
        assert_eq!(
            registered.is_ok(),
            !name.is_empty(),
            "{name:.40?}: {registered:?}"
        );

        let format = CopyFunctionBuilder::try_new(&name).map(|b| {
            b.bind(copy_noop::bind)
                .global_init(copy_noop::init)
                .sink(copy_noop::sink)
                .finalize(copy_noop::finalize)
        });
        // SAFETY: `con` is open; all four callbacks have their signatures.
        let registered = format.and_then(|b| unsafe { b.register(fx.con()) });
        assert_eq!(
            registered.is_ok(),
            !name.is_empty(),
            "{name:.40?}: {registered:?}"
        );

        assert_untouched(&fx, &format!("setting or format {name:.40?}"));
    }
}

#[test]
fn a_macro_body_cannot_turn_one_statement_into_several() {
    for (label, body) in [
        ("stacked", "1); DROP TABLE canary; SELECT (1"),
        ("stacked create", "1); CREATE TABLE evil AS SELECT (1"),
        ("comment then stacked", "1); DROP TABLE canary; --"),
        ("cr ends a comment", "1 --\r); DROP TABLE canary; SELECT (1"),
        ("nested comments", "1 /* /* ; */ DROP TABLE canary; */"),
    ] {
        for table in [false, true] {
            let fx = fixture_with_canary();
            // SAFETY: the fixture's handles outlive `con`.
            let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };
            let m = if table {
                SqlMacro::table("m_t", &["x"], body)
            } else {
                SqlMacro::scalar("m_s", &["x"], body)
            }
            .expect("valid names");
            // Registering may succeed (a comment swallowing the rest is still
            // one statement) or fail; either way nothing else runs.
            // SAFETY: the connection is valid for this call.
            let _ = unsafe { con.register_sql_macro(m) };
            assert_untouched(&fx, label);
        }
    }
    // Text that merely looks like a second statement is one statement.
    let fx = fixture_with_canary();
    // SAFETY: the fixture's handles outlive `con`.
    let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };
    for (name, body) in [
        ("m_lit", "';'"),
        ("m_dollar", "$$;$$"),
        ("m_block", "1 /* ; */"),
    ] {
        let m = SqlMacro::scalar(name, &["x"], body).expect("names");
        // SAFETY: the connection is valid for this call.
        unsafe { con.register_sql_macro(m) }.unwrap_or_else(|e| panic!("{body}: {e}"));
    }
}

/// A type nested a few thousand levels deep, built in a loop (as a reader of
/// an untrusted schema would), used to overflow the stack in
/// `scalar::builder::collision::type_string`, which aborts the process.
#[test]
fn a_deeply_nested_type_is_refused_not_recursed_into() {
    const DEPTH: usize = 2_000;
    // `signature::describe` (a set's duplicate check) has a smaller frame
    // than `collision::type_string`: on this 2 MiB thread in a debug build it
    // first overflowed between 2,000 and 3,500 levels. `DuckDB`'s own
    // `LogicalType::ToString` overflows the same stack at 6,000 (a later
    // catalog scan renders the registered type), so deeper is the engine's
    // limit, not quack-rs's.
    const SET_DEPTH: usize = 3_500;
    let nest_to = |depth: usize| {
        let mut t = LogicalType::new(TypeId::BigInt);
        for _ in 0..depth {
            t = LogicalType::try_list_from_logical(&t).expect("nest");
        }
        t
    };
    let nest = move || nest_to(DEPTH);
    // 2 MiB is the default stack of a test thread; the registration runs on it.
    let worker = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            let fx = fixture_with_canary();
            // SAFETY: the fixture's handles outlive `con`.
            let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };
            // SAFETY: the connection is valid for these calls.
            let scalar = unsafe {
                con.register_scalar(
                    ScalarFunctionBuilder::new("deep_scalar")
                        .param_logical(nest())
                        .returns_logical(nest())
                        .function(zero),
                )
            };
            // A set renders every overload for the duplicate check
            // (`signature::describe`), which recursed the same way.
            // SAFETY: the connection is valid for this call.
            let set = unsafe {
                con.register_scalar_set(
                    ScalarFunctionSetBuilder::new("deep_set")
                        .overload(
                            ScalarOverloadBuilder::new()
                                .param_logical(nest_to(SET_DEPTH))
                                .returns(TypeId::BigInt)
                                .function(zero),
                        )
                        .overload(
                            ScalarOverloadBuilder::new()
                                .param_logical(nest_to(SET_DEPTH))
                                .param(TypeId::BigInt)
                                .returns(TypeId::BigInt)
                                .function(zero),
                        ),
                )
            };
            let table = TableFunctionBuilder::new("deep_table")
                .with_state::<(), _>(move |bind| {
                    let mut t = LogicalType::new(TypeId::BigInt);
                    for _ in 0..DEPTH {
                        t = LogicalType::try_list_from_logical(&t).expect("nest");
                    }
                    bind.add_result_column_with_type("x", &t);
                    Ok(())
                })
                .scan(|(), chunk| {
                    // SAFETY: ending the scan.
                    unsafe { chunk.set_size(0) };
                    Ok(())
                })
                .build()
                .expect("build");
            // SAFETY: the connection is valid for this call.
            let table = unsafe { con.register_table(table) };
            assert_untouched(&fx, "deep types");
            (
                scalar.map_err(|e| e.to_string()),
                set.map_err(|e| e.to_string()),
                table.map_err(|e| e.to_string()),
            )
        })
        .expect("spawn");
    // Reaching here at all is the test: the stack did not overflow. Whether a
    // 2,000-deep type registers is the engine's business.
    let (scalar, set, table) = worker.join().expect("the worker did not panic");
    let _ = (scalar, set, table);
}

/// `u64::MAX` is what a careless "unknown" looks like. `DuckDB` converts the
/// hint to a signed count when it estimates a join and failed any query that
/// joined the function with `INTERNAL Error: Information loss on integer cast`.
#[test]
fn an_enormous_cardinality_hint_does_not_break_a_join() {
    let fx = Fixture::open();
    // SAFETY: the fixture's handles outlive `con`.
    let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };
    for exact in [true, false] {
        let name = format!("huge_card_{exact}");
        let table = TableFunctionBuilder::new(&name)
            .with_state::<(), _>(move |bind| {
                bind.add_result_column("x", TypeId::BigInt);
                bind.set_cardinality(u64::MAX, exact);
                Ok(())
            })
            .scan(|(), chunk| {
                // SAFETY: ending the scan.
                unsafe { chunk.set_size(0) };
                Ok(())
            })
            .build()
            .expect("build");
        // SAFETY: the connection is valid for this call.
        unsafe { con.register_table(table) }.expect("register");
        for sql in [
            format!("SELECT count(*) FROM {name}() a, {name}() b"),
            format!("SELECT count(*) FROM {name}() a JOIN range(10) r ON a.x = r.range"),
            format!("EXPLAIN SELECT * FROM {name}() a, {name}() b"),
        ] {
            // `fx.query` panics on an error, with the message.
            let _ = fx.query(&sql);
        }
    }
}
