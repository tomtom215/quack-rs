// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The scalar signature-collision check, held to `DuckDB`'s own binder.

use libduckdb_sys as ffi;
use quack_rs::connection::{Connection, Registrar};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
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

/// One overload: fixed parameter types and an optional varargs type.
type Sig = (&'static [TypeId], Option<TypeId>);

fn overload(sig: Sig) -> ScalarOverloadBuilder {
    let mut b = ScalarOverloadBuilder::new();
    for id in sig.0 {
        b = b.param(*id);
    }
    if let Some(v) = sig.1 {
        b = b.varargs(v);
    }
    b.returns(TypeId::BigInt).function(zero)
}

/// Registers `sigs` as one set named `name` through the raw C API, with no
/// check of any kind, as another extension could.
unsafe fn register_raw_set(con: ffi::duckdb_connection, name: &str, sigs: &[Sig]) {
    let c_name = std::ffi::CString::new(name).unwrap();
    unsafe {
        let mut set = ffi::duckdb_create_scalar_function_set(c_name.as_ptr());
        for sig in sigs {
            let mut f = ffi::duckdb_create_scalar_function();
            ffi::duckdb_scalar_function_set_name(f, c_name.as_ptr());
            for id in sig.0 {
                let t = LogicalType::new(*id);
                ffi::duckdb_scalar_function_add_parameter(f, t.as_raw());
            }
            if let Some(v) = sig.1 {
                let t = LogicalType::new(v);
                ffi::duckdb_scalar_function_set_varargs(f, t.as_raw());
            }
            let ret = LogicalType::new(TypeId::BigInt);
            ffi::duckdb_scalar_function_set_return_type(f, ret.as_raw());
            ffi::duckdb_scalar_function_set_function(f, Some(zero));
            assert_eq!(
                ffi::duckdb_add_scalar_function_to_set(set, f),
                ffi::DuckDBSuccess
            );
            ffi::duckdb_destroy_scalar_function(&raw mut f);
        }
        assert_eq!(
            ffi::duckdb_register_scalar_function_set(con, set),
            ffi::DuckDBSuccess
        );
        ffi::duckdb_destroy_scalar_function_set(&raw mut set);
    }
}

fn literal(id: TypeId) -> &'static str {
    match id {
        TypeId::BigInt | TypeId::Any => "1::BIGINT",
        TypeId::Integer => "1::INTEGER",
        TypeId::Double => "1::DOUBLE",
        TypeId::Varchar => "'x'",
        other => panic!("no literal for {other}"),
    }
}

/// Whether some call `sig` accepts (up to two varargs arguments) is refused
/// by `DuckDB`'s binder as ambiguous for the set `name`.
fn some_call_is_ambiguous(fx: &Fixture, name: &str, sigs: &[Sig]) -> bool {
    sigs.iter().any(|sig| {
        let extra = if sig.1.is_some() { 0..=2 } else { 0..=0 };
        extra.into_iter().any(|n| {
            let args: Vec<&str> = sig
                .0
                .iter()
                .copied()
                .chain(std::iter::repeat_n(sig.1.unwrap_or(TypeId::BigInt), n))
                .map(literal)
                .collect();
            let sql = format!("SELECT {name}({})", args.join(", "));
            match unsafe { quack_rs::query::query(fx.con(), &sql) } {
                Ok(_) => false,
                Err(e) => {
                    let message = format!("{e}");
                    assert!(
                        message.contains("Could not choose a best candidate"),
                        "{sql}: unexpected error {message}"
                    );
                    true
                }
            }
        })
    })
}

/// The fourth audit's F6. The duplicate-overload check compared whole
/// signatures, so a set whose overloads were not identical but accepted the
/// same call registered and then failed that call: `{f(BIGINT),
/// f(BIGINT, BIGINT...)}` fails `f(1)`, `{f(), f(ANY...)}` fails `f()`, and
/// `{f(BIGINT...), f(BIGINT, BIGINT...)}` fails every call. For each pair
/// below, the builder must refuse the set exactly when `DuckDB`'s binder,
/// given the same set through the raw C API, finds a call ambiguous.
#[test]
fn the_overlap_check_agrees_with_duckdbs_binder() {
    use TypeId::{Any, BigInt, Double, Integer, Varchar};
    let fx = Fixture::open();
    let cases: &[(Sig, Sig)] = &[
        ((&[BigInt], None), (&[BigInt], Some(BigInt))),
        ((&[], None), (&[], Some(Any))),
        ((&[], Some(BigInt)), (&[BigInt], Some(BigInt))),
        ((&[BigInt], None), (&[], Some(BigInt))),
        ((&[BigInt], None), (&[], Some(Double))),
        ((&[BigInt], None), (&[], Some(Any))),
        ((&[Varchar], Some(BigInt)), (&[Varchar], Some(Double))),
        ((&[Varchar], Some(BigInt)), (&[Double], Some(BigInt))),
        ((&[BigInt, Double], None), (&[BigInt], Some(BigInt))),
        ((&[], None), (&[Varchar], Some(BigInt))),
        ((&[BigInt], None), (&[Integer], None)),
        ((&[BigInt], None), (&[BigInt, BigInt], None)),
    ];
    let mut disagreements = Vec::new();
    let mut ambiguous_cases = 0;
    for (k, (a, b)) in cases.iter().enumerate() {
        let raw_name = format!("amb_raw_{k}");
        // SAFETY: `con` is open.
        unsafe { register_raw_set(fx.con(), &raw_name, &[*a, *b]) };
        let ambiguous = some_call_is_ambiguous(&fx, &raw_name, &[*a, *b]);
        // SAFETY: `con` is open; the callback matches the declared signature.
        let refused = unsafe {
            ScalarFunctionSetBuilder::new(&format!("amb_checked_{k}"))
                .overload(overload(*a))
                .overload(overload(*b))
                .register(fx.con())
        }
        .is_err();
        ambiguous_cases += usize::from(ambiguous);
        if ambiguous != refused {
            disagreements.push(format!(
                "case {k} {a:?} / {b:?}: DuckDB ambiguous = {ambiguous}, refused = {refused}"
            ));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
    // Both outcomes occur, so the agreement is not vacuous: DuckDB 1.5.5
    // finds cases 0, 1, 2, 3 and 6 ambiguous.
    assert_eq!(ambiguous_cases, 5, "cases DuckDB found ambiguous");
}

/// The same rule against the catalog: a single function overlapping an
/// existing registration is refused, a non-overlapping one is not.
#[test]
fn an_overload_overlapping_an_earlier_registration_is_refused() {
    let fx = Fixture::open();
    // SAFETY: `con` is open.
    unsafe {
        register_raw_set(
            fx.con(),
            "later_amb",
            &[(&[TypeId::BigInt], Some(TypeId::BigInt))],
        );
    }
    let register = |params: &[TypeId]| {
        let mut b = ScalarFunctionBuilder::new("later_amb");
        for id in params {
            b = b.param(*id);
        }
        // SAFETY: `con` is open; the callback matches the declared signature.
        unsafe { b.returns(TypeId::BigInt).function(zero).register(fx.con()) }
    };
    let err = register(&[TypeId::BigInt]).expect_err("f(BIGINT) beside f(BIGINT, BIGINT...)");
    if !super::engine_merges_scalar_overloads(&fx) {
        // Before DuckDB 1.5.0 no existing name takes another overload.
        assert!(err.as_str().contains("before v1.5.0"), "{err}");
        return;
    }
    assert!(err.as_str().contains("accepts the same arguments"), "{err}");
    register(&[TypeId::Varchar]).expect("f(VARCHAR) overlaps nothing");
}

/// The fourth audit's F2. The check rendered types without their alias, but
/// `duckdb_functions()` lists an aliased type by its alias and `DuckDB`
/// compares aliases: re-registering `cf(myint)` silently replaced the first
/// registration, while `abs(myint)` — a distinct overload `DuckDB` accepts —
/// was refused as `abs(INTEGER)`.
#[test]
fn aliased_parameter_types_are_compared_by_alias() {
    let fx = Fixture::open();
    let myint = || {
        let t = LogicalType::new(TypeId::Integer);
        // SAFETY: `t` is a live handle.
        unsafe { t.set_alias("myint") };
        t
    };
    let register = |name: &str, param: LogicalType| {
        // SAFETY: `con` is open; the callback matches the declared signature.
        unsafe {
            ScalarFunctionBuilder::new(name)
                .param_logical(param)
                .returns(TypeId::BigInt)
                .function(zero)
                .register(fx.con())
        }
    };
    register("cf", myint()).expect("first cf(myint)");
    register("cf", myint()).expect_err("second cf(myint) would replace the first");
    if !super::engine_merges_scalar_overloads(&fx) {
        // Before DuckDB 1.5.0 no existing name takes another overload.
        let err = register("abs", myint()).expect_err("abs exists");
        assert!(err.as_str().contains("before v1.5.0"), "{err}");
        return;
    }
    register("abs", myint()).expect("abs(myint) is its own overload");
    assert_eq!(
        fx.scalar("SELECT abs(-5::INTEGER)::BIGINT", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(5),
        "the built-in abs(INTEGER) is untouched"
    );
    let list_of_myint = || LogicalType::list_from_logical(&myint());
    register("lf", list_of_myint()).expect("first lf(myint[])");
    register("lf", list_of_myint()).expect_err("second lf(myint[])");
}

/// The fourth audit's F7. The check's SQL called `array_to_string`, `lower`
/// and `chr` unqualified, so a macro of the same name in the default schema
/// changed what it computed, and `abs(BIGINT)` was then silently replaced.
#[test]
fn a_user_macro_cannot_defeat_the_check() {
    let fx = Fixture::open();
    for sql in [
        "CREATE MACRO array_to_string(a, b) AS 'shadowed'",
        "CREATE MACRO lower(a) AS 'shadowed'",
        "CREATE MACRO chr(a) AS 'shadowed'",
        "CREATE MACRO duckdb_functions() AS TABLE SELECT 1 AS x",
    ] {
        // SAFETY: `con` is open.
        unsafe { quack_rs::query::query(fx.con(), sql) }.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let builder = ScalarFunctionBuilder::map1("abs", |x: i64| x + 1000).expect("valid name");
    // SAFETY: `con` is open.
    let err = unsafe { builder.register(fx.con()) }.expect_err("abs(BIGINT) exists");
    assert!(err.as_str().contains("already exists"), "{err}");
    assert_eq!(
        fx.scalar("SELECT abs(-5::BIGINT)", |r, i| unsafe { r.read_i64(i) }),
        Some(5)
    );
}

/// The fourth audit's F5. Registration through the entry point's
/// `Connection` lists the catalog once and records each registration, so a
/// signature it registered itself is refused the second time without
/// listing the catalog again, and sets are covered too.
#[test]
fn a_connection_remembers_what_it_registered() {
    let fx = Fixture::open();
    // SAFETY: the fixture's handles outlive `con`.
    let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };
    let make = || {
        ScalarFunctionBuilder::new("remembered")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(zero)
    };
    // SAFETY: the connection is valid for these calls.
    unsafe {
        con.register_scalar(make()).expect("first");
        con.register_scalar(make())
            .expect_err("second, from the snapshot");
        con.register_scalar_set(
            ScalarFunctionSetBuilder::new("remembered_set")
                .overload(overload((&[TypeId::Varchar], None)))
                .overload(overload((&[TypeId::Double], None))),
        )
        .expect("set");
        con.register_scalar(
            ScalarFunctionBuilder::new("remembered_set")
                .param(TypeId::Double)
                .returns(TypeId::BigInt)
                .function(zero),
        )
        .expect_err("overload of the set");
        con.register_scalar(
            ScalarFunctionBuilder::new("abs")
                .param(TypeId::BigInt)
                .returns(TypeId::BigInt)
                .function(zero),
        )
        .expect_err("built-in, from the snapshot");
    }
}

/// The same hazard in the table-function and config-option name checks: a
/// `lower` macro made `lower(function_name) = lower($1)` true for every row,
/// so every table function name counted as taken. Both queries now qualify
/// every function they call.
#[test]
fn the_table_and_setting_name_checks_ignore_user_macros() {
    use quack_rs::table::TableFunctionBuilder;

    let fx = Fixture::open();
    for sql in [
        "CREATE MACRO lower(a) AS 'same'",
        "CREATE MACRO duckdb_functions() AS TABLE SELECT 1 AS x",
        "CREATE MACRO duckdb_settings() AS TABLE SELECT 1 AS x",
    ] {
        // SAFETY: `con` is open.
        unsafe { quack_rs::query::query(fx.con(), sql) }.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let table = |name: &str| {
        TableFunctionBuilder::new(name)
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
            .expect("build")
    };
    // SAFETY: `con` is open.
    unsafe { table("fresh_table_fn").register(fx.con()) }.expect("a fresh name is free");
    // SAFETY: `con` is open.
    unsafe { table("range").register(fx.con()) }.expect_err("the built-in range is taken");

    #[cfg(feature = "duckdb-1-5")]
    {
        use quack_rs::config_option::ConfigOptionBuilder;
        let option = |name: &str| {
            ConfigOptionBuilder::try_new(name)
                .expect("valid name")
                .option_type(TypeId::BigInt)
                .default_value("1")
                .expect("valid default")
        };
        // SAFETY: `con` is open.
        unsafe { option("fresh_setting").register(fx.con()) }.expect("a fresh name is free");
        // SAFETY: `con` is open.
        unsafe { option("threads").register(fx.con()) }.expect_err("threads is taken");
    }
}
