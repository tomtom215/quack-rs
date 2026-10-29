// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Function signatures `DuckDB` accepts at registration and then fails on at
//! query time. A refusal at registration is the only place quack-rs can stop
//! them: once registered, the first query that reaches the function takes the
//! process (or the database) down.

#[cfg(feature = "duckdb-1-5-3")]
use quack_rs::scalar::ScalarFunctionBuilder;
#[cfg(feature = "duckdb-1-5-3")]
use quack_rs::table::TableFunctionBuilder;
#[cfg(feature = "duckdb-1-5-3")]
use quack_rs::types::{LogicalType, TypeId};

#[cfg(feature = "duckdb-1-5-3")]
use super::Fixture;

#[cfg(feature = "duckdb-1-5-3")]
quack_rs::scalar_callback!(never_called, |_info, _input, _output| {});

/// `duckdb_create_logical_type(VARIANT)` returns a type with no `STRUCT`
/// layout. A scalar function declared with it registers, and then
/// `SELECT v_out(1)` is a segmentation fault on `DuckDB` 2.0.0
/// (`VectorStructBuffer` while initialising the result vector) and
/// `SELECT v_in({'a':1}::VARIANT)` an internal error that invalidates the
/// database on 1.5.x. So the id is refused wherever a type is made from it.
#[cfg(feature = "duckdb-1-5-3")]
#[test]
fn a_variant_in_a_function_signature_is_refused() {
    let fx = Fixture::open();

    let err = LogicalType::try_new(TypeId::Variant).expect_err("VARIANT from its id is refused");
    assert!(err.to_string().contains("VARIANT"), "{err}");
    assert!(
        err.to_string().contains("ANY"),
        "points at the way out: {err}"
    );

    // Scalar: result slot and argument slot.
    // SAFETY: `con` is open; the callback matches the declared signature.
    let result = unsafe {
        ScalarFunctionBuilder::try_new("v_out")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::Variant)
            .function(never_called)
            .register(fx.con())
    };
    let err = result.expect_err("a VARIANT result is refused");
    assert!(err.as_str().contains("VARIANT"), "{err}");
    // SAFETY: as above.
    let result = unsafe {
        ScalarFunctionBuilder::try_new("v_in")
            .expect("name")
            .param(TypeId::Variant)
            .returns(TypeId::BigInt)
            .function(never_called)
            .register(fx.con())
    };
    let err = result.expect_err("a VARIANT argument is refused");
    assert!(err.as_str().contains("VARIANT"), "{err}");

    // Neither registration left a function behind.
    assert_eq!(
        fx.scalar(
            "SELECT count(*) FROM duckdb_functions() WHERE function_name IN ('v_in', 'v_out')",
            // SAFETY: one BIGINT column, row `i`.
            |r, i| unsafe { r.read_i64(i) }
        ),
        Some(0)
    );

    // Table function: a named parameter. Complete callbacks, so it is the
    // VARIANT that is refused.
    let built = TableFunctionBuilder::new("v_table")
        .named_param("v", TypeId::Variant)
        .with_state::<bool, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            Ok(true)
        })
        .scan(|_state, chunk| {
            // SAFETY: ending the scan.
            unsafe { chunk.set_size(0) };
            Ok(())
        })
        .build();
    let err = match built {
        Err(e) => e,
        // SAFETY: as above.
        Ok(table) => {
            unsafe { table.register(fx.con()) }.expect_err("a VARIANT parameter is refused")
        }
    };
    assert!(err.as_str().contains("VARIANT"), "{err}");

    // ANY, the documented way to accept one, still registers.
    // SAFETY: as above.
    unsafe {
        ScalarFunctionBuilder::try_new("v_any")
            .expect("name")
            .param(TypeId::Any)
            .returns(TypeId::BigInt)
            .function(never_called)
            .register(fx.con())
    }
    .expect("ANY registers");
}
