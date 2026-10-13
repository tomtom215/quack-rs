// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Closure-based scalar constructors whose closure can fail.
//!
//! Each `try_*` constructor mirrors its infallible namesake — same argument
//! types, same NULL propagation, same registered signature — except that the
//! closure returns `Result<R, E>`. An `Err` stops the chunk and is reported
//! through `duckdb_scalar_function_set_error`, the same channel a panic and
//! [`scalar_callback!`][crate::scalar_callback] use, so the query fails with
//! `E`'s `Display` text.

use std::fmt::Display;

use crate::error::ExtensionError;
use crate::scalar::builder::ScalarFunctionBuilder;
use crate::scalar::typed::{ScalarOut, ScalarValue};
use crate::scalar::typed_builder::TypedScalarFunctionBuilder;
use crate::types::{NullHandling, TypeId};

/// Turns a closure's error into the error the trampoline reports, verbatim.
///
/// An empty `Display` is replaced by a placeholder when it is reported; see
/// [`ScalarFunctionInfo::set_error`][crate::scalar::ScalarFunctionInfo::set_error].
pub(super) fn user_error<E: Display>(err: &E) -> ExtensionError {
    ExtensionError::new(err.to_string())
}

impl ScalarFunctionBuilder {
    /// A unary scalar function from a fallible closure, with SQL NULL
    /// propagation.
    ///
    /// Like [`map1`][Self::map1], but the closure returns `Result<R, E>`.
    /// `Ok(v)` writes `v`; return `Ok(None)` (with `R = Option<_>`) for SQL
    /// NULL. A row whose argument is NULL yields NULL without calling the
    /// closure, exactly as in `map1`.
    ///
    /// # What an `Err` does
    ///
    /// The first `Err` in a chunk stops processing that chunk: no later row is
    /// passed to the closure. Its `Display` text is reported through the
    /// scalar function's `set_error`, and `DuckDB` fails the whole query with
    /// that message. Rows already written into the result vector are
    /// discarded with the rest of the query — the statement returns an error,
    /// never a partial result. (A client that is *streaming* a result may
    /// already have fetched earlier chunks before the failing one is computed;
    /// the fetch that reaches the failing chunk returns the error.)
    ///
    /// A panic in the closure is reported the same way.
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
    ///     ScalarFunctionBuilder::try_map1("checked_square", |x: i64| {
    ///         x.checked_mul(x).ok_or_else(|| format!("{x} squared overflows BIGINT"))
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    pub fn try_map1<A, R, E, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        R: ScalarOut,
        E: Display,
        F: Fn(A) -> Result<R, E> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id()],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: DuckDB declared one parameter, so column 0 exists and
                // has the type `A::type_id()` named at registration.
                let a = unsafe { chunk.reader(0) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) } {
                        // SAFETY: the column's type was declared as `A`.
                        let value = f(unsafe { A::read(&a, row) }).map_err(|e| user_error(&e))?;
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

    /// A binary scalar function from a fallible closure, with SQL NULL
    /// propagation.
    ///
    /// Like [`map2`][Self::map2], with `Err` handled as in
    /// [`try_map1`][Self::try_map1]. A row with either argument NULL yields
    /// NULL without calling the closure.
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
    ///     ScalarFunctionBuilder::try_map2("strict_div", |a: i64, b: i64| {
    ///         a.checked_div(b).ok_or("strict_div: division by zero")
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    // As for `map2`: the `&& -> ||` mutant of the per-row NULL test lives in a
    // closure only DuckDB's executor runs. It is killed end to end by
    // `try_closures_propagate_null_arguments` in
    // `tests/ffi_roundtrip/typed_scalar_opt.rs`, which NULLs one argument at a
    // time from a real column.
    #[cfg_attr(test, mutants::skip)]
    pub fn try_map2<A, B, R, E, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        A: ScalarValue,
        B: ScalarValue,
        R: ScalarOut,
        E: Display,
        F: Fn(A, B) -> Result<R, E> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[A::type_id(), B::type_id()],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: two parameters were declared, so both columns exist
                // with the types named at registration.
                let (a, b) = unsafe { (chunk.reader(0), chunk.reader(1)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) && b.is_valid(row) } {
                        // SAFETY: the columns' types were declared as `A` and `B`.
                        let value = unsafe { f(A::read(&a, row), B::read(&b, row)) }
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

    /// A `VARCHAR`-argument scalar function from a fallible closure, with SQL
    /// NULL propagation.
    ///
    /// Like [`map1_str`][Self::map1_str] — the closure borrows the string
    /// straight out of the vector — with `Err` handled as in
    /// [`try_map1`][Self::try_map1].
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
    ///     // '' is NULL, 'x@y' is 'y', anything else fails the query.
    ///     ScalarFunctionBuilder::try_map1_str("email_domain", |s: &str| {
    ///         if s.is_empty() {
    ///             return Ok(None);
    ///         }
    ///         match s.split_once('@') {
    ///             Some((_, domain)) => Ok(Some(domain.to_owned())),
    ///             None => Err(format!("not an email address: {s:?}")),
    ///         }
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    pub fn try_map1_str<R, E, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        R: ScalarOut,
        E: Display,
        F: for<'a> Fn(&'a str) -> Result<R, E> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[TypeId::Varchar],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: one VARCHAR parameter was declared, so column 0 exists
                // and holds VARCHAR.
                let a = unsafe { chunk.reader(0) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) } {
                        // SAFETY: the column was declared VARCHAR; the borrow
                        // ends before the chunk does.
                        let value = f(unsafe { a.read_str(row) }).map_err(|e| user_error(&e))?;
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

    /// A two-`VARCHAR` scalar function from a fallible closure, with SQL NULL
    /// propagation.
    ///
    /// Like [`map2_str`][Self::map2_str], with `Err` handled as in
    /// [`try_map1`][Self::try_map1].
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
    ///     ScalarFunctionBuilder::try_map2_str("field", |line: &str, key: &str| {
    ///         line.split(',')
    ///             .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    ///             .map(str::to_owned)
    ///             .ok_or_else(|| format!("no field {key:?}"))
    ///     })?
    ///     .register(con)
    /// }
    /// # }
    /// ```
    // As for `map2_str`: the `&& -> ||` mutant of the per-row NULL test lives
    // in a closure only DuckDB's executor runs. It is killed end to end by
    // `try_closures_propagate_null_arguments` in
    // `tests/ffi_roundtrip/typed_scalar_opt.rs`.
    #[cfg_attr(test, mutants::skip)]
    pub fn try_map2_str<R, E, F>(
        name: &str,
        f: F,
    ) -> Result<TypedScalarFunctionBuilder, ExtensionError>
    where
        R: ScalarOut,
        E: Display,
        F: for<'a, 'b> Fn(&'a str, &'b str) -> Result<R, E> + Send + Sync + 'static,
    {
        TypedScalarFunctionBuilder::from_exec(
            name,
            &[TypeId::Varchar, TypeId::Varchar],
            R::type_id(),
            NullHandling::DefaultNullHandling,
            Box::new(move |chunk, writer| {
                // SAFETY: two VARCHAR parameters were declared.
                let (a, b) = unsafe { (chunk.reader(0), chunk.reader(1)) };
                for row in 0..chunk.size() {
                    // SAFETY: `row` is within the chunk.
                    if unsafe { a.is_valid(row) && b.is_valid(row) } {
                        // SAFETY: both columns were declared VARCHAR.
                        let value = unsafe { f(a.read_str(row), b.read_str(row)) }
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
