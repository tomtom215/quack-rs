// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! `DuckDB` type system wrappers.
//!
//! This module provides ergonomic wrappers for `DuckDB`'s type system:
//!
//! - [`TypeId`]: An enum of all supported `DuckDB` column types
//! - [`LogicalType`]: An RAII wrapper around `duckdb_logical_type` that ensures
//!   the type is properly destroyed when it goes out of scope
//!
//! # Pitfall L7: `LogicalType` memory leak
//!
//! `duckdb_create_logical_type` allocates memory that must be freed with
//! `duckdb_destroy_logical_type`. Failing to call the destructor leaks memory.
//! [`LogicalType`] solves this by implementing `Drop`.

pub mod logical_type;
pub mod type_id;

pub use logical_type::LogicalType;
pub use type_id::TypeId;
