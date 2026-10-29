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

mod overload;
mod set;
mod single;

#[cfg(test)]
mod tests;

pub use overload::AggregateOverloadBuilder;
pub use set::AggregateFunctionSetBuilder;
pub use single::AggregateFunctionBuilder;

use crate::error::ExtensionError;

/// The refusal of an aggregate that takes no parameter, for `what` (the
/// function, or one overload of a set).
///
/// The C API gives `update` a chunk and the state array and no row count: the
/// rows are `chunk.size()`. `DuckDB` 2.0 stopped setting the chunk's cardinality
/// (`CAPIAggregateUpdate`, `aggregate_function-c.cpp`), and a chunk with no
/// column reports a size of 0, so `SELECT f() FROM t` over 100 rows answers
/// 100 on `DuckDB` 1.5.6 and 0 on 2.0.0 (checked against both). A windowed call
/// of such an aggregate never reaches `update` on either. A silent wrong
/// answer is worse than a refusal.
pub(super) fn zero_parameter_error(what: &str, name: &str) -> ExtensionError {
    ExtensionError::new(format!(
        "{what} takes no parameter: DuckDB 2.0 passes `update` a chunk with no columns, \
         which reports a size of 0 and has no row count, so the aggregate would count \
         no rows (and a windowed call never reaches `update` on any version). Give it \
         one parameter, and for a zero-argument SQL spelling register a macro in front \
         of it: `CREATE MACRO {name}() AS {name}_1(1)`"
    ))
}

/// Former name of [`AggregateOverloadBuilder`].
///
/// Renamed for symmetry with [`ScalarOverloadBuilder`][crate::scalar::ScalarOverloadBuilder];
/// the two now read the same way at a call site.
#[deprecated(
    since = "0.18.0",
    note = "renamed to `AggregateOverloadBuilder` for symmetry with `ScalarOverloadBuilder`"
)]
pub type OverloadBuilder = AggregateOverloadBuilder;
