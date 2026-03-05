//! Builders for registering `DuckDB` aggregate functions.
//!
//! This module provides two builder types:
//!
//! - [`AggregateFunctionBuilder`]: Register a single aggregate function with
//!   one fixed signature.
//! - [`AggregateFunctionSetBuilder`]: Register a function set (multiple overloads
//!   under one name) for functions with variadic signatures.
//!
//! # Pitfalls solved
//!
//! - **L6** ([Problem 2][super]): The builder enforces that every function in a
//!   function set has its name set individually. Without this, `DuckDB` silently
//!   returns `DuckDBError` when you call `duckdb_register_aggregate_function_set`,
//!   and the function is never registered.
//! - **L7**: [`LogicalType`][crate::types::LogicalType] handles memory management
//!   for all type handles.
//!
//! # Example: Single function
//!
//! ```rust,no_run
//! use quack_rs::aggregate::AggregateFunctionBuilder;
//! use quack_rs::types::TypeId;
//! use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_aggregate_state,
//!                     duckdb_data_chunk, duckdb_vector, idx_t};
//!
//! // Define your callbacks
//! unsafe extern "C" fn state_size(_: duckdb_function_info) -> idx_t { 8 }
//! unsafe extern "C" fn state_init(_: duckdb_function_info, _: duckdb_aggregate_state) {}
//! unsafe extern "C" fn update(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_aggregate_state) {}
//! unsafe extern "C" fn combine(_: duckdb_function_info, _: duckdb_aggregate_state, _: duckdb_aggregate_state, _: idx_t) {}
//! unsafe extern "C" fn finalize(_: duckdb_function_info, _: duckdb_aggregate_state, _: duckdb_vector, _: idx_t, _: idx_t) {}
//!
//! // fn register(con: duckdb_connection) {
//! //     AggregateFunctionBuilder::new("my_count")
//! //         .param(TypeId::BigInt)
//! //         .returns(TypeId::BigInt)
//! //         .state_size(state_size)
//! //         .init(state_init)
//! //         .update(update)
//! //         .combine(combine)
//! //         .finalize(finalize)
//! //         .register(con)
//! //         .expect("registration failed");
//! // }
//! ```

pub mod builder;
pub mod callbacks;
pub mod state;

pub use builder::{AggregateFunctionBuilder, AggregateFunctionSetBuilder};
pub use state::{AggregateState, FfiState};
