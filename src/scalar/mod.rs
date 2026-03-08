// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` scalar (table) functions.
//!
//! Scalar functions process one row at a time and return a single value per row.
//! They are the most common type of function in `DuckDB` extensions.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::scalar::ScalarFunctionBuilder;
//! use quack_rs::types::TypeId;
//! use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_data_chunk,
//!                     duckdb_vector};
//!
//! unsafe extern "C" fn my_func(
//!     _info: duckdb_function_info,
//!     _input: duckdb_data_chunk,
//!     _output: duckdb_vector,
//! ) {}
//!
//! // fn register(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
//! //     unsafe {
//! //         ScalarFunctionBuilder::new("double_it")
//! //             .param(TypeId::BigInt)
//! //             .returns(TypeId::BigInt)
//! //             .function(my_func)
//! //             .register(con)
//! //     }
//! // }
//! ```

pub mod builder;

pub use builder::{ScalarFunctionBuilder, ScalarFunctionSetBuilder, ScalarOverloadBuilder};
