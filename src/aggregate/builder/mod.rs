// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder types for registering `DuckDB` aggregate functions.
//!
//! # Pitfall L6: Function set name must be set on EACH member
//!
//! When using `duckdb_register_aggregate_function_set`, the function name must be
//! set on **each individual function** added to the set, not just on the set itself.
//! If you call `duckdb_aggregate_function_set_name` only on the set (or forget to
//! call it on an individual function), `DuckDB` silently returns `DuckDBError` at
//! registration time, and the function is never registered.
//!
//! [`AggregateFunctionSetBuilder`] enforces this by calling the name-setter
//! internally for every function added to the set.

mod set;
mod single;

#[cfg(test)]
mod tests;

pub use set::{AggregateFunctionSetBuilder, OverloadBuilder};
pub use single::AggregateFunctionBuilder;
