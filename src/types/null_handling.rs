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
/// **Aggregates are different.** `DuckDB`'s aggregate executor really does
/// filter NULL rows out before `update` under `DefaultNullHandling`; the
/// caveat above is specific to scalar functions.
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
    /// For an **aggregate**, `DuckDB` enforces it by filtering NULL rows before
    /// `update`.
    #[default]
    DefaultNullHandling,
    /// The function means to see NULLs and may return non-NULL for them.
    ///
    /// Registers `FunctionNullHandling::SPECIAL_HANDLING`, which suppresses
    /// `DuckDB`'s debug-build assertion that NULL in implies NULL out, and stops
    /// the aggregate executor from filtering NULL rows. The callback must check
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
