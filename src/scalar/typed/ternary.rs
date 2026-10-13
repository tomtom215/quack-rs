// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Three-argument closure-based scalar constructors.
//!
//! [`map3`][ScalarFunctionBuilder::map3] and
//! [`try_map3`][ScalarFunctionBuilder::try_map3] extend
//! [`map2`][ScalarFunctionBuilder::map2] / [`try_map2`][ScalarFunctionBuilder::try_map2]
//! by one [`ScalarValue`] column — enough for `clamp(x, lo, hi)`,
//! `lerp(a, b, t)` and similar functions without dropping to
//! [`scalar_callback!`][crate::scalar_callback].

use std::fmt::Display;

use super::fallible::user_error;
use crate::error::ExtensionError;
use crate::scalar::builder::ScalarFunctionBuilder;
use crate::scalar::typed::{ScalarOut, ScalarValue};
use crate::scalar::typed_builder::TypedScalarFunctionBuilder;
use crate::types::NullHandling;

impl ScalarFunctionBuilder {
    /// A ternary scalar function from a safe closure, with SQL NULL
    /// propagation.
    ///
    /// A row with any argument NULL yields NULL without calling the closure.
    /// Return `Option<R>` to produce NULL for other rows too.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::scalar::ScalarFunctionBuilder;
    ///
    /// # fn demo(con: libduckdb_sys::duckdb_connection)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// // SAFETY: `con` is the connection DuckDB handed the entry point.
    /// unsafe {
    ///     ScalarFunctionBuilder::map3("lerp", |a: f64, b: f64, t: f64| a + (b - a) * t)?
    ///         .register(con)
    /// }
    /// # }
    /// ```
    pub fn map3<A, B, C, R, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        B: ScalarValue,
        C: ScalarValue,
        R: ScalarOut,
        F: Fn(A, B, C) -> R + Send + Sync + 'static,
    {
        Self::try_map3(name, move |a: A, b: B, c: C| {
            Ok::<R, std::convert::Infallible>(f(a, b, c))
        })
    }

    /// A ternary scalar function from a fallible closure, with SQL NULL
    /// propagation.
    ///
    /// Like [`map3`][Self::map3], with `Err` handled as in
    /// [`try_map1`][Self::try_map1]: the first `Err` stops the chunk and fails
    /// the query with its `Display` text.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is not a valid SQL identifier.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use quack_rs::scalar::ScalarFunctionBuilder;
    ///
    /// # fn demo(con: libduckdb_sys::duckdb_connection)
    /// # -> Result<(), quack_rs::error::ExtensionError> {
    /// // SAFETY: `con` is the connection DuckDB handed the entry point.
    /// unsafe {
    ///     ScalarFunctionBuilder::try_map3("strict_clamp", |x: i64, lo: i64, hi: i64| {
    ///         if lo > hi {
    ///             Err(format!("strict_clamp: empty range [{lo}, {hi}]"))
    ///         } else {
    ///             Ok(x.clamp(lo, hi))
    ///         }
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    // As for `map2`: the `&& -> ||` mutants of the per-row NULL test live in a
    // closure only DuckDB's executor runs. They are killed end to end by
    // `map3_propagates_a_null_in_any_argument` in
    // `tests/ffi_roundtrip/typed_scalar_opt.rs`, which NULLs each argument in
    // turn from a real column.
    #[cfg_attr(test, mutants::skip)]
    pub fn try_map3<A, B, C, R, E, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        B: ScalarValue,
        C: ScalarValue,
        R: ScalarOut,
        E: Display,
        F: Fn(A, B, C) -> Result<R, E> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id(), B::type_id(), C::type_id()],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: three parameters were declared, so all three columns
                // exist with the types named at registration.
                let (a, b, c) = unsafe { (chunk.reader(0), chunk.reader(1), chunk.reader(2)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) && b.is_valid(row) && c.is_valid(row) } {
                        // SAFETY: the columns' types were declared as `A`, `B`
                        // and `C`.
                        let value =
                            unsafe { f(A::read(&a, row), B::read(&b, row), C::read(&c, row)) }
                                .map_err(|e| user_error(&e))?;
                        // SAFETY: the result vector's type was declared as `R`.
                        unsafe { R::write(writer, row, value) };
                    } else {
                        // SAFETY: `row` is within the output vector.
                        unsafe { writer.set_null(row) };
                    }
                }
                Ok(())
            }),
        )
    }
}
