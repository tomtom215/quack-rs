// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `Value::as_str`, `display_string` and `Debug` on values that plain SQL
//! builds and `DuckDB` cannot render (`docs/upstream-duckdb-reports.md`, items
//! 13 and 22). Before the render guard became an allow-list, each case below
//! could abort the process — `DuckDB`'s cast to text throws through the C
//! API — and one returned corrupt text when it did not.
//!
//! - A `VARIANT` holding an out-of-range timestamp: the guard did not look
//!   inside `VARIANT` (and the C API cannot).
//! - A `DECIMAL(38, 0)` holding `i128::MIN`, from `sum` over two in-range
//!   values: "Negation of HUGEINT is out of range".
//! - A `DECIMAL(38, 38)` holding 1.2, also from `sum`: `DuckDB` never writes
//!   the first character of its text, so the result is whatever byte was
//!   there — a stray character, a NUL (an empty string), or, when the byte is
//!   not valid UTF-8, an exception ("Invalid unicode") that aborts.
//! - A `GEOMETRY` built from malformed WKB.
//!
//! Two tests are not abort cases: one checks which `ARRAY` and `UNION`
//! values are still rendered, and the last pins how a `UNION` renders.

use std::cell::RefCell;

use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;
use quack_rs::value::UNRENDERABLE;

use super::Fixture;

type Seen = (Result<String, String>, Option<bool>, String);
thread_local! {
    // Per thread: the tests run in parallel, and a table function binds on
    // the thread that runs the query.
    static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
}

/// Registers `render_probe(ANY)`, which renders its argument every way a
/// `Value` can be rendered and records the results.
fn register_probe(fx: &Fixture) {
    let table_fn = TableFunctionBuilder::new("render_probe")
        .param(TypeId::Any)
        .with_state::<bool, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            // SAFETY: parameter 0 exists: the function declares one.
            let value = unsafe { bind.get_parameter_value(0) };
            let as_str = value.as_str().map_err(|e| e.to_string());
            #[cfg(feature = "duckdb-1-5")]
            let display = Some(value.display_string().is_some());
            #[cfg(not(feature = "duckdb-1-5"))]
            let display = None;
            let debug = format!("{value:?}");
            SEEN.with(|seen| seen.borrow_mut().push((as_str, display, debug)));
            Ok(true)
        })
        .scan(|pending, chunk| {
            // SAFETY: column 0 is BIGINT; row 0 is in range; a zero size ends
            // the scan.
            unsafe {
                if std::mem::take(pending) {
                    chunk.writer(0).write_i64(0, 1);
                    chunk.set_size(1);
                } else {
                    chunk.set_size(0);
                }
            }
            Ok(())
        })
        .build()
        .expect("build render_probe");
    // SAFETY: `con` is open for the fixture's lifetime.
    unsafe { table_fn.register(fx.con()) }.expect("register render_probe");
}

fn render(fx: &Fixture, arg: &str) -> Seen {
    SEEN.with(|seen| seen.borrow_mut().clear());
    let sql = format!("SELECT n FROM render_probe({arg})");
    assert_eq!(
        fx.scalar(&sql, |r, i| unsafe { r.read_i64(i) }),
        Some(1),
        "{sql}"
    );
    let mut seen = SEEN.with(|seen| std::mem::take(&mut *seen.borrow_mut()));
    assert_eq!(seen.len(), 1, "{arg}");
    seen.remove(0)
}

/// Every renderer refuses `arg`, and `Debug` still returns.
fn assert_refused(fx: &Fixture, arg: &str) {
    let (text, display, debug) = render(fx, arg);
    assert_eq!(text, Err(UNRENDERABLE.to_owned()), "{arg}");
    assert_ne!(display, Some(true), "{arg}");
    assert!(debug.starts_with("Value"), "{arg}: {debug}");
}

/// `arg` renders as `text`.
fn assert_rendered(fx: &Fixture, arg: &str, text: &str) {
    let (got, display, _) = render(fx, arg);
    assert_eq!(got.as_deref(), Ok(text), "{arg}");
    assert_ne!(display, Some(false), "{arg}");
}

/// `VARIANT` holds any type, and the C API cannot see which: a `VARIANT` is
/// never rendered, whatever it holds.
#[test]
fn a_variant_is_not_rendered() {
    let fx = Fixture::open();
    register_probe(&fx);
    assert_refused(&fx, "make_timestamp(-9223372036854775808)::VARIANT");
    assert_refused(&fx, "42::VARIANT");
    assert_refused(&fx, "[42::VARIANT]");
}

/// `sum` over a `DECIMAL(38, s)` column checks for `HUGEINT` overflow, not
/// for the width, so plain SQL builds `DECIMAL`s wider than their type.
#[test]
fn a_decimal_wider_than_its_type_is_not_rendered() {
    let fx = Fixture::open();
    register_probe(&fx);
    for (name, values) in [
        (
            "dec_min",
            "(-99999999999999999999999999999999999999)::DECIMAL(38,0)), \
             ((-70141183460469231731687303715884105729)::DECIMAL(38,0)",
        ),
        (
            "dec_scale",
            "'0.6'::DECIMAL(38,38)), ('0.6'::DECIMAL(38,38)",
        ),
    ] {
        fx.query(&format!(
            "SET VARIABLE {name} = (SELECT sum(x) FROM (VALUES ({values})) t(x))"
        ));
        assert_refused(&fx, &format!("getvariable('{name}')"));
        assert_refused(&fx, &format!("[getvariable('{name}')]"));
    }
    assert_rendered(&fx, "1.25::DECIMAL(4,2)", "1.25");
    assert_rendered(&fx, "(-0.5)::DECIMAL(4,4)", "-.5000");
    assert_rendered(&fx, "[1.25::DECIMAL(4,2)]", "[1.25]");
}

/// An `ARRAY` or `UNION` is refused only when it could hold something that
/// needs checking, since the C API cannot read its elements.
#[test]
fn opaque_containers_are_refused_only_when_their_type_needs_a_check() {
    let fx = Fixture::open();
    register_probe(&fx);
    assert_rendered(&fx, "[1, 2]::INTEGER[2]", "[1, 2]");
    assert_rendered(
        &fx,
        "union_value(k := 'x')::UNION(n INTEGER, k VARCHAR)",
        "x",
    );
    for arg in [
        "[1.5, 2.5]::DECIMAL(4,1)[2]",
        "union_value(d := 1.5::DECIMAL(4,1))::UNION(n INTEGER, d DECIMAL(4,1))",
        "[42::VARIANT]::VARIANT[1]",
    ] {
        assert_refused(&fx, arg);
    }
}

/// `ST_GeomFromWKB` (built in from 1.5.0) accepts a `MULTIPOINT` whose part is
/// a `LINESTRING`, and `Geometry::ToString` throws on it ("Expected POINT in
/// MULTIPOINT"). A well-formed one is refused too: the C API cannot read the
/// WKB to tell them apart.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn a_geometry_is_not_rendered() {
    let fx = Fixture::open();
    register_probe(&fx);
    let malformed = "ST_GeomFromWKB('\\x01\\x04\\x00\\x00\\x00\\x01\\x00\\x00\\x00\\x01\\x02\\x00\\x00\\x00\\x00\\x00\\x00\\x00'::BLOB)";
    assert_refused(&fx, malformed);
    assert_refused(&fx, &format!("[{malformed}]"));
    assert_refused(&fx, "'POINT(1 2)'::GEOMETRY");
}

/// `as_str` casts to VARCHAR as SQL does, so a UNION renders as its active
/// member alone: the tag is lost, and a member that is NULL renders as the
/// text `NULL` although the value itself is not SQL NULL. `Debug` keeps both
/// where it shows contents at all (`duckdb-1-5`, which `display_string`
/// needs).
#[test]
fn a_union_renders_as_its_member_alone() {
    let fx = Fixture::open();
    register_probe(&fx);
    let debug_shows = |debug: &str, want: &str| {
        if cfg!(feature = "duckdb-1-5") {
            assert!(debug.contains(want), "{debug}");
        } else {
            assert_eq!(debug, "Value { type: Union }");
        }
    };
    let (text, _, debug) = render(&fx, "union_value(a := 5)");
    assert_eq!(text.as_deref(), Ok("5"));
    debug_shows(&debug, "union_value(a := 5)");
    let (text, _, debug) = render(&fx, "union_value(a := NULL::INTEGER)");
    assert_eq!(text.as_deref(), Ok("NULL"));
    debug_shows(&debug, "union_value(a := NULL)");
    let (text, _, _) = render(&fx, "NULL::UNION(a INTEGER, b VARCHAR)");
    assert_eq!(text, Err("Value is SQL NULL".to_owned()));
}
