// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! NULL propagation behaviour for `DuckDB` functions.

/// Controls how `DuckDB` handles NULL arguments for a function.
///
/// By default, `DuckDB` automatically returns NULL if any input is NULL
/// (the `DefaultNullHandling` behaviour). Setting `SpecialNullHandling`
/// tells `DuckDB` to pass NULLs through to your callback, so your function
/// can handle them explicitly.
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
    /// `DuckDB` automatically returns NULL if any argument is NULL.
    /// This is the default behaviour — no FFI call is needed.
    #[default]
    DefaultNullHandling,
    /// NULLs are passed through to the function callback.
    /// The function must check validity itself.
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
