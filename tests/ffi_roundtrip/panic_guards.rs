// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The panic-guard macros and `set_error` methods, against a live `DuckDB`:
//! the right error channel for each callback kind, and a readable message
//! even when the panic or error has none.

use quack_rs::callback::EMPTY_PANIC_PLACEHOLDER;
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionInfo};
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::Fixture;

/// Runs `sql`, which must fail, and returns the error text.
fn error_of(fx: &Fixture, sql: &str) -> String {
    // SAFETY: the fixture's connection is open.
    let err = unsafe { quack_rs::query::query(fx.con(), sql) }.expect_err(sql);
    format!("{err}")
}

/// The connection still answers after a failed query.
fn still_answers(fx: &Fixture) {
    assert_eq!(
        fx.scalar("SELECT 42::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(42)
    );
}

quack_rs::scalar_callback!(copy_through, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row)) };
    }
});

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_bind_callback!(panicking_scalar_bind, |_info| {
    panic!("scalar bind deliberately panicked");
});

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_init_callback!(panicking_scalar_init, |_info| {
    panic!("scalar init deliberately panicked");
});

/// The fourth audit's F1. quack-rs had no panic guard for a scalar
/// function's `bind` / `init`, and the table macros compile in their place
/// because the C signatures match. Their `duckdb_bind_set_error` /
/// `duckdb_init_set_error` then wrote into a table function's info struct
/// laid over the smaller scalar one: the process died with SIGSEGV. The
/// scalar macros report through the scalar setters.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_panicking_scalar_bind_or_init_becomes_a_sql_error() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::new("bind_bomb")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(copy_through)
            .bind(panicking_scalar_bind)
            .register(fx.con())
            .expect("register bind_bomb");
        ScalarFunctionBuilder::new("init_bomb")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(copy_through)
            .init(panicking_scalar_init)
            .register(fx.con())
            .expect("register init_bomb");
    }
    let message = error_of(&fx, "SELECT bind_bomb(i) FROM range(3) t(i)");
    assert!(
        message.contains("scalar bind deliberately panicked"),
        "{message}"
    );
    still_answers(&fx);
    let message = error_of(&fx, "SELECT init_bomb(i) FROM range(3) t(i)");
    assert!(
        message.contains("scalar init deliberately panicked"),
        "{message}"
    );
    still_answers(&fx);
}

quack_rs::scalar_callback!(silent_panic, |_info, _input, _output| {
    panic!("");
});

quack_rs::scalar_callback!(silent_error, |info, _input, _output| {
    unsafe { ScalarFunctionInfo::new(info) }.set_error("");
});

/// The fourth audit's F12: `panic!("")` in a callback, or `set_error("")`,
/// produced `Invalid Input Error: ` and nothing else. Both now carry a
/// placeholder that says what happened.
#[test]
fn an_empty_panic_or_error_message_is_replaced_by_a_placeholder() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::new("silent_panic")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(silent_panic)
            .register(fx.con())
            .expect("register silent_panic");
        ScalarFunctionBuilder::new("silent_error")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(silent_error)
            .register(fx.con())
            .expect("register silent_error");
    }
    let message = error_of(&fx, "SELECT silent_panic(i) FROM range(3) t(i)");
    assert!(message.contains(EMPTY_PANIC_PLACEHOLDER), "{message}");
    let message = error_of(&fx, "SELECT silent_error(i) FROM range(3) t(i)");
    assert!(
        message.contains(quack_rs::scalar::info::EMPTY_ERROR_PLACEHOLDER),
        "{message}"
    );
    still_answers(&fx);
}

quack_rs::replacement_scan_callback!(
    half_configured_then_silent_panic,
    |info, table_name, _data| {
        // SAFETY: DuckDB passes a valid NUL-terminated identifier.
        let name = unsafe { std::ffi::CStr::from_ptr(table_name) }.to_string_lossy();
        if name.ends_with(".quiet") {
            // SAFETY: `info` is the pointer DuckDB passed in.
            unsafe {
                quack_rs::replacement_scan::ReplacementScanInfo::new(info)
                    .set_function("range")
                    .add_i64_parameter(3);
            }
            panic!("");
        }
    }
);

/// The fourth audit's T3. `DuckDB` raises a replacement-scan error only when
/// its message is non-empty (`replacement_scan-c.cpp`), so `panic!("")`
/// after `set_function` was swallowed and the half-configured redirect ran:
/// `SELECT * FROM 'x.quiet'` returned `range(3)`'s three rows.
#[test]
fn a_replacement_scan_panicking_without_a_message_fails_the_query() {
    use quack_rs::replacement_scan::ReplacementScanBuilder;

    let fx = Fixture::open();
    // SAFETY: `db` is open; no extra data to clean up.
    unsafe {
        ReplacementScanBuilder::register(
            fx.db(),
            half_configured_then_silent_panic,
            std::ptr::null_mut(),
            None,
        );
    }
    let message = error_of(&fx, "SELECT * FROM 'x.quiet'");
    assert!(message.contains(EMPTY_PANIC_PLACEHOLDER), "{message}");
    still_answers(&fx);
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_bind_callback!(inspect_first_argument, |info| {
    let bind = unsafe { quack_rs::scalar::ScalarBindInfo::new(info) };
    // SAFETY: the function declares one parameter.
    let _ = unsafe { bind.argument(0) };
});

/// The fourth audit's T5. Asking for a scalar subquery argument at bind time
/// fails the query inside `DuckDB` (its copy throws), and the error it left
/// was a raw JSON exception. The message now says what happened.
///
/// That is v1.5.5 and later. Before it, `DuckDB` copies the argument outside
/// any `try`, so the same query aborted the process; there `argument` asks
/// for nothing and fails the bind instead, for any argument. Checked against
/// libduckdb 1.5.0 with `libduckdb-sys` pinned to its bindings: the ungated
/// call aborts, the gated one fails the query and the connection answers.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn inspecting_a_subquery_argument_fails_with_a_readable_message() {
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callbacks match the declared signatures.
    unsafe {
        ScalarFunctionBuilder::new("inspects_arg")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(copy_through)
            .bind(inspect_first_argument)
            .register(fx.con())
            .expect("register inspects_arg");
    }
    let version = fx
        .scalar("SELECT version()", |r, i| unsafe {
            r.read_str(i).to_owned()
        })
        .expect("version() is never NULL");
    let catches = quack_rs::abi::parse_version(&version).is_some_and(|v| v >= (1, 5, 5));
    if catches {
        assert_eq!(
            fx.scalar("SELECT inspects_arg(7::BIGINT)", |r, i| unsafe {
                r.read_i64(i)
            }),
            Some(7),
            "an ordinary argument can be inspected"
        );
    } else {
        let message = error_of(&fx, "SELECT inspects_arg(7::BIGINT)");
        assert!(message.contains("before v1.5.5"), "{version}: {message}");
    }
    let message = error_of(&fx, "SELECT inspects_arg((SELECT 7::BIGINT))");
    assert!(
        message.contains("cannot be inspected at bind time"),
        "{message}"
    );
    assert!(!message.contains("exception_type"), "{message}");
    still_answers(&fx);
}

/// The fourth audit's F14. A return type of `ANY` is refused by `DuckDB`
/// with a bare error, which surfaced with a hint about name collisions. It is
/// now refused first, naming the slot, including when nested.
#[test]
fn an_any_return_type_is_refused_by_name() {
    use quack_rs::types::LogicalType;
    let fx = Fixture::open();
    // SAFETY: `con` is open; the callback matches the declared signature.
    let err = unsafe {
        ScalarFunctionBuilder::new("returns_any")
            .param(TypeId::BigInt)
            .returns(TypeId::Any)
            .function(copy_through)
            .register(fx.con())
    }
    .expect_err("ANY return");
    assert!(
        err.as_str()
            .contains("scalar function return type must not be"),
        "{err}"
    );
    // SAFETY: as above.
    let err = unsafe {
        ScalarFunctionBuilder::new("returns_any_list")
            .param(TypeId::BigInt)
            .returns_logical(LogicalType::list(TypeId::Any))
            .function(copy_through)
            .register(fx.con())
    }
    .expect_err("LIST(ANY) return");
    assert!(err.as_str().contains("must not be or contain ANY"), "{err}");
}

/// A literal type as a return or parameter type registered, and the first
/// query returning it failed with an internal error that invalidated the
/// database: every later query, `SELECT 42` included, failed. Both slots are
/// now refused before `DuckDB` sees them, and the database stays usable.
#[test]
fn a_literal_type_is_refused_in_every_slot() {
    let fx = Fixture::open();
    for ty in [TypeId::IntegerLiteral, TypeId::StringLiteral] {
        // SAFETY: `con` is open; the callback matches the declared signature.
        let err = unsafe {
            ScalarFunctionBuilder::new("returns_literal")
                .param(TypeId::BigInt)
                .returns(ty)
                .function(copy_through)
                .register(fx.con())
        }
        .expect_err("literal return");
        assert!(err.as_str().contains("unbound literal"), "{ty}: {err}");
        // SAFETY: as above.
        let err = unsafe {
            ScalarFunctionBuilder::new("takes_literal")
                .param(ty)
                .returns(TypeId::BigInt)
                .function(copy_through)
                .register(fx.con())
        }
        .expect_err("literal parameter");
        assert!(err.as_str().contains("unbound literal"), "{ty}: {err}");
    }
    assert_eq!(
        fx.scalar("SELECT 42::BIGINT", |r, i| unsafe { r.read_i64(i) }),
        Some(42)
    );
}
