// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Every `Registrar` method of `Connection`, the facade `entry_point_v2!` hands
//! to an extension's registration closure, reaches `DuckDB`.
//!
//! Line coverage of the live suite shows seven of the ten methods never ran:
//! `register_typed_scalar`, `register_aggregate`, `register_aggregate_set`,
//! `register_sql_macro`, `register_cast`, `register_copy_function` and
//! `register_config_option`. Each is a one-line delegation, which is exactly
//! the code a wrong-builder or wrong-connection slip hides in.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateOverloadBuilder,
};
use quack_rs::connection::{Connection, Registrar};
use quack_rs::data_chunk::DataChunk;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::sql_macro::SqlMacro;
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::vector_support::count;
use super::Fixture;

const unsafe extern "C" fn size(_info: duckdb_function_info) -> idx_t {
    8
}

const unsafe extern "C" fn init(_info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { state.cast::<i64>().write(0) };
}

/// Sums the first column into the state.
unsafe extern "C" fn update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if unsafe { reader.is_valid(row) } {
            unsafe { *(*states.add(row)).cast::<i64>() += reader.read_i64(row) };
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
        unsafe { *(*target.add(i)).cast::<i64>() += *(*source.add(i)).cast::<i64>() };
    }
}

unsafe extern "C" fn finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        unsafe { writer.write_i64(offset as usize + i, *(*source.add(i)).cast::<i64>()) };
    }
}

fn sum_builder(name: &str) -> AggregateFunctionBuilder {
    AggregateFunctionBuilder::try_new(name)
        .expect("name")
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt)
        .state_size(size)
        .init(init)
        .update(update)
        .combine(combine)
        .finalize(finalize)
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_bind_callback!(copy_bind, |_info| {});
#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_global_init_callback!(copy_init, |_info| {});
#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_sink_callback!(copy_sink, |_info, _chunk| {});
#[cfg(feature = "duckdb-1-5")]
quack_rs::copy_finalize_callback!(copy_finalize, |_info| {});

#[test]
fn every_registrar_method_of_connection_registers_what_it_is_given() {
    let fx = Fixture::open();
    // SAFETY: the fixture's handles outlive `con`.
    let con = unsafe { Connection::from_raw(fx.con(), fx.db()) };

    // SAFETY: the connection is valid; every callback matches its signature.
    unsafe {
        con.register_typed_scalar(
            ScalarFunctionBuilder::map1("cr_typed", |x: i64| x + 1).expect("typed builder"),
        )
        .expect("register_typed_scalar");
        con.register_aggregate(sum_builder("cr_sum"))
            .expect("register_aggregate");
        con.register_aggregate_set(
            AggregateFunctionSetBuilder::try_new("cr_sum_set")
                .expect("name")
                .returns(TypeId::BigInt)
                .overloads(1..=1, |_, builder: AggregateOverloadBuilder| {
                    builder
                        .param(TypeId::BigInt)
                        .state_size(size)
                        .init(init)
                        .update(update)
                        .combine(combine)
                        .finalize(finalize)
                }),
        )
        .expect("register_aggregate_set");
        con.register_sql_macro(SqlMacro::scalar("cr_double", &["a"], "a * 2").expect("macro"))
            .expect("register_sql_macro");
    }

    assert_eq!(
        count(&fx, "SELECT cr_typed(41)"),
        42,
        "register_typed_scalar"
    );
    assert_eq!(
        count(&fx, "SELECT cr_sum(i) FROM range(10) t(i)"),
        45,
        "register_aggregate"
    );
    assert_eq!(
        count(&fx, "SELECT cr_sum_set(i) FROM range(10) t(i)"),
        45,
        "register_aggregate_set"
    );
    assert_eq!(
        count(&fx, "SELECT cr_double(21::BIGINT)"),
        42,
        "register_sql_macro"
    );

    #[cfg(feature = "duckdb-1-5")]
    {
        use quack_rs::cast::CastFunctionBuilder;
        use quack_rs::config_option::ConfigOptionBuilder;
        use quack_rs::copy_function::CopyFunctionBuilder;

        // SAFETY: as above.
        unsafe {
            con.register_cast(
                CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
                    .function(super::panicking_cast),
            )
            .expect("register_cast");
            con.register_copy_function(
                CopyFunctionBuilder::try_new("cr_copy")
                    .expect("name")
                    .bind(copy_bind)
                    .global_init(copy_init)
                    .sink(copy_sink)
                    .finalize(copy_finalize),
            )
            .expect("register_copy_function");
            con.register_config_option(
                ConfigOptionBuilder::try_new("cr_registrar_option")
                    .expect("name")
                    .description("registered through Connection")
                    .expect("description")
                    .option_type(TypeId::BigInt)
                    .default_value("7")
                    .expect("default"),
            )
            .expect("register_config_option");
        }
        // The cast is ours: the stand-in panics, and the panic reaches SQL.
        let err = unsafe { quack_rs::query::query(fx.con(), "SELECT CAST('7' AS INTEGER)") }
            .expect_err("the registered cast must run");
        assert!(
            err.as_str().contains("cast deliberately exploded"),
            "register_cast: {err}"
        );
        assert_eq!(
            count(&fx, "SELECT current_setting('cr_registrar_option')::BIGINT"),
            7,
            "register_config_option"
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.cr");
        fx.query(&format!(
            "COPY (SELECT 1 AS n) TO '{}' (FORMAT cr_copy)",
            path.display()
        ));
    }
}
