// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` scalar functions.
//!
//! Scalar functions take a data chunk of input rows and produce one output value
//! per row. This is the most common function type in `DuckDB` extensions.

mod overload;
mod set;
pub(crate) mod signature;
mod single;

#[cfg(test)]
mod tests;

pub use overload::ScalarOverloadBuilder;
pub use set::ScalarFunctionSetBuilder;
#[cfg(feature = "duckdb-1-5")]
pub use single::{ScalarBindFn, ScalarInitFn};
pub use single::{ScalarFn, ScalarFunctionBuilder};
