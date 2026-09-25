// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `MockRegistrar` refuses a builder with the same message as the real
//! registration, for every type check the real one makes before its first
//! function handle: a composite or literal `TypeId` in any slot, an `ANY`
//! return type. The mock runs these checks without `DuckDB`; this compares
//! its message with the one a live connection gives.

use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_bind_info, duckdb_data_chunk, duckdb_function_info,
    duckdb_init_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{
    AggregateFunctionBuilder, AggregateFunctionSetBuilder, AggregateOverloadBuilder,
};
use quack_rs::cast::CastFunctionBuilder;
use quack_rs::connection::Registrar;
use quack_rs::error::ExtensionError;
use quack_rs::scalar::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::table::TableFunctionBuilder;
use quack_rs::testing::MockRegistrar;
use quack_rs::types::TypeId;

use super::Fixture;

// Never invoked: every builder here is refused before registration.
const unsafe extern "C" fn scalar_fn(
    _: duckdb_function_info,
    _: duckdb_data_chunk,
    _: duckdb_vector,
) {
}
const unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t {
    0
}
const unsafe extern "C" fn state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
const unsafe extern "C" fn update(
    _: duckdb_function_info,
    _: duckdb_data_chunk,
    _: *mut duckdb_aggregate_state,
) {
}
const unsafe extern "C" fn combine(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: *mut duckdb_aggregate_state,
    _: idx_t,
) {
}
const unsafe extern "C" fn finalize(
    _: duckdb_function_info,
    _: *mut duckdb_aggregate_state,
    _: duckdb_vector,
    _: idx_t,
    _: idx_t,
) {
}
const unsafe extern "C" fn table_bind(_: duckdb_bind_info) {}
const unsafe extern "C" fn table_init(_: duckdb_init_info) {}
const unsafe extern "C" fn table_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}
const unsafe extern "C" fn cast_fn(
    _: duckdb_function_info,
    _: idx_t,
    _: duckdb_vector,
    _: duckdb_vector,
) -> bool {
    true
}

fn scalar() -> ScalarFunctionBuilder {
    ScalarFunctionBuilder::new("parity_scalar")
        .returns(TypeId::BigInt)
        .function(scalar_fn)
}

fn aggregate() -> AggregateFunctionBuilder {
    AggregateFunctionBuilder::new("parity_agg")
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt)
        .state_size(state_size)
        .init(state_init)
        .update(update)
        .combine(combine)
        .finalize(finalize)
}

fn overload() -> AggregateOverloadBuilder {
    AggregateOverloadBuilder::new()
        .param(TypeId::BigInt)
        .state_size(state_size)
        .init(state_init)
        .update(update)
        .combine(combine)
        .finalize(finalize)
}

fn table() -> TableFunctionBuilder {
    TableFunctionBuilder::new("parity_table")
        .bind(table_bind)
        .init(table_init)
        .scan(table_scan)
}

/// Both refuse, with the same message; returns it.
fn same(mock: Result<(), ExtensionError>, real: Result<(), ExtensionError>) -> String {
    let mock = mock.expect_err("the mock refuses").as_str().to_owned();
    let real = real.expect_err("registration refuses").as_str().to_owned();
    assert_eq!(mock, real);
    mock
}

#[test]
fn the_mock_refuses_every_type_with_the_real_message() {
    let fx = Fixture::open();
    let con = fx.con();
    let mock = MockRegistrar::new();
    let mut seen = Vec::new();
    // SAFETY (every call): `con` is open; the mock ignores it.
    unsafe {
        seen.push(same(
            mock.register_scalar(scalar().param(TypeId::Struct)),
            scalar().param(TypeId::Struct).register(con),
        ));
        seen.push(same(
            mock.register_scalar(scalar().varargs(TypeId::IntegerLiteral)),
            scalar().varargs(TypeId::IntegerLiteral).register(con),
        ));
        seen.push(same(
            mock.register_scalar(scalar().returns(TypeId::Any)),
            scalar().returns(TypeId::Any).register(con),
        ));
        let set = || {
            ScalarFunctionSetBuilder::new("parity_set").overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::List)
                    .returns(TypeId::BigInt)
                    .function(scalar_fn),
            )
        };
        seen.push(same(mock.register_scalar_set(set()), set().register(con)));
        seen.push(same(
            mock.register_aggregate(aggregate().returns(TypeId::StringLiteral)),
            aggregate().returns(TypeId::StringLiteral).register(con),
        ));
        let agg_set = || {
            AggregateFunctionSetBuilder::new("parity_agg_set")
                .returns(TypeId::Any)
                .overload(overload())
        };
        seen.push(same(
            mock.register_aggregate_set(agg_set()),
            agg_set().register(con),
        ));
        seen.push(same(
            mock.register_table(table().param(TypeId::Map)),
            table().param(TypeId::Map).register(con),
        ));
        seen.push(same(
            mock.register_table(table().named_param("n", TypeId::Union)),
            table().named_param("n", TypeId::Union).register(con),
        ));
        let cast = || CastFunctionBuilder::new(TypeId::Struct, TypeId::Integer).function(cast_fn);
        seen.push(same(mock.register_cast(cast()), cast().register(con)));
        #[cfg(feature = "duckdb-1-5")]
        {
            let option = || {
                quack_rs::config_option::ConfigOptionBuilder::try_new("parity_opt")
                    .expect("name")
                    .option_type(TypeId::Struct)
                    .default_value("x")
                    .expect("default")
            };
            seen.push(same(
                mock.register_config_option(option()),
                option().register(con),
            ));
        }
    }
    let prefixes = [
        "scalar function parameter 0: ",
        "scalar function varargs: ",
        "scalar function return type must not be or contain ANY",
        "overload 0 parameter 0: ",
        "aggregate function return type: ",
        "overload 0 return type must not be or contain ANY",
        "table function parameter 0: ",
        "table function named parameter n: ",
        "cast function source type: ",
        "config option type: ",
    ];
    for (message, prefix) in seen.iter().zip(prefixes) {
        assert!(message.starts_with(prefix), "{prefix}: {message}");
    }
    assert_eq!(mock.total_registrations(), 0);
}
