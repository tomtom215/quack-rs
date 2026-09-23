// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! NULL propagation behaviour for `DuckDB` functions.

/// Declares whether a function is prepared to see NULL arguments.
///
/// # This does not make `DuckDB` propagate NULLs for you
///
/// The name invites the reading "`DefaultNullHandling` means `DuckDB` returns
/// NULL when an argument is NULL, without calling me". For a **scalar function
/// registered through the C API that is false at run time**, and the difference
/// is silent wrong answers rather than an error:
///
/// - `CAPIScalarFunction` calls the extension's callback for the whole
///   flattened chunk, NULL rows included, and never inspects the result's
///   validity.
/// - `ExpressionExecutor::Execute` then calls `VerifyNullHandling`, which is
///   compiled **only under `#ifdef DEBUG`** and merely *asserts* that the
///   function already produced NULL wherever an input was NULL.
///
/// A quick `SELECT my_func(NULL)` does not reveal this, because a literal NULL
/// is constant-folded before the function is ever called. The wrong answers
/// start once the argument is a column. Verified against `DuckDB` 1.5.4: a
/// scalar function writing `999` unconditionally, registered with
/// `DefaultNullHandling`, returned `999` — valid, not NULL — for the NULL row
/// of a real table.
///
/// So: under `DefaultNullHandling`, propagate NULLs yourself. The one-line way
/// is [`DataChunk::propagate_nulls`][crate::data_chunk::DataChunk::propagate_nulls]
/// at the end of the callback.
///
/// What the setting *does* control is `DuckDB`'s own expectations —
/// `SpecialNullHandling` tells the planner and the debug assertion that this
/// function means to see NULLs and will not necessarily return NULL for them,
/// which is what a `coalesce`-like or `is_null`-like function needs.
///
/// # Aggregates see NULL rows too
///
/// The same is true of aggregates: under **either** setting, `update` receives
/// every row of the chunk, NULL rows included. `CAPIAggregateUpdate`
/// (`src/main/capi/aggregate_function-c.cpp`) flattens each input vector and
/// hands the whole chunk to the callback without looking at validity, and the
/// aggregate executor applies no NULL filter of its own. An aggregate that must
/// ignore NULLs checks
/// [`VectorReader::is_valid`][crate::vector::VectorReader::is_valid] per row
/// and skips the invalid ones; reading an invalid row's value reads whatever
/// happens to be in the data buffer.
///
/// For an aggregate the setting is read in exactly one place:
/// `BoundAggregateExpression::PropagatesNullValues`, which the correlated
/// subquery decorrelator (`flatten_dependent_join.cpp`) consults to choose an
/// `INNER` or `LEFT` join. No query tried against `DuckDB` 1.5.5 — correlated
/// scalar subqueries with and without arithmetic or `coalesce` over the
/// aggregate, `LATERAL`, and a correlated subquery in `WHERE` — gave a
/// different answer under the two settings.
///
/// What *does* differ from an uncorrelated query, under both settings, is an
/// outer row with no matching inner rows: the correlated subquery yields NULL
/// without ever finalizing an empty state. `DuckDB` special-cases only its own
/// `count` and `count(*)` to return 0 there, so a count-like aggregate that
/// returns 0 for empty input returns NULL for such a row. Wrap the subquery in
/// `coalesce(..., 0)` if that matters.
///
/// # Example
///
/// ```rust,no_run
/// use quack_rs::scalar::ScalarFunctionBuilder;
/// use quack_rs::types::{TypeId, NullHandling};
///
/// // fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
/// //     unsafe {
/// //         ScalarFunctionBuilder::new("coalesce_custom")
/// //             .param(TypeId::BigInt)
/// //             .returns(TypeId::BigInt)
/// //             .null_handling(NullHandling::SpecialNullHandling)
/// //             .function(my_func)
/// //             .register(con)
/// //     }
/// // }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NullHandling {
    /// The function is expected to return NULL wherever an argument is NULL.
    ///
    /// This is `DuckDB`'s default, so quack-rs makes no FFI call for it. For a
    /// **scalar** function it is a promise the callback must keep itself — see
    /// the type-level documentation, and
    /// [`DataChunk::propagate_nulls`][crate::data_chunk::DataChunk::propagate_nulls].
    /// An **aggregate**'s `update` also receives NULL rows under this setting
    /// and must skip them itself — see the type-level documentation.
    #[default]
    DefaultNullHandling,
    /// The function means to see NULLs and may return non-NULL for them.
    ///
    /// Registers `FunctionNullHandling::SPECIAL_HANDLING`, which suppresses
    /// `DuckDB`'s debug-build assertion that NULL in implies NULL out. It does
    /// not change which rows a callback receives: scalar and aggregate
    /// callbacks see NULL rows under either setting. The callback must check
    /// [`VectorReader::is_valid`][crate::vector::VectorReader::is_valid] itself.
    SpecialNullHandling,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_default_handling() {
        assert_eq!(NullHandling::default(), NullHandling::DefaultNullHandling);
    }

    #[test]
    fn debug_display() {
        let s = format!("{:?}", NullHandling::SpecialNullHandling);
        assert!(s.contains("SpecialNullHandling"));
    }
}
