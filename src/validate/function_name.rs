// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SQL function name validation for `DuckDB` extensions.
//!
//! A function name that needs quoting in SQL, or that cannot survive the trip
//! through a C string, will fail at registration or produce a function nobody
//! can call. This validator rejects those, and nothing else.
//!
//! # What it deliberately does not do
//!
//! It does not impose a naming *style*. `DuckDB` identifiers are
//! case-insensitive and `DuckDB` itself ships mixed-case functions
//! (`formatReadableSize`, `formatReadableDecimalSize`), so `myFunc` registers
//! fine and is callable as `myfunc`, `MYFUNC` or `myFunc` — verified against
//! `DuckDB` 1.5.5. `snake_case` is the overwhelming convention in `DuckDB`'s own
//! catalog and is worth following, but it is a convention, not a rule, and this
//! validator gates [`ScalarFunctionBuilder::try_new`][crate::scalar::ScalarFunctionBuilder::try_new]
//! — so enforcing it here would make a legitimate function name
//! *unregisterable*.

use crate::error::ExtensionError;

/// Maximum length for a function name.
///
/// `DuckDB` does not publicly document a hard limit, but names beyond 256
/// characters are unreasonable and may cause issues with catalog storage.
const MAX_FUNCTION_NAME_LEN: usize = 256;

/// Validates a `DuckDB` function name.
///
/// # Rules
///
/// - Must not be empty
/// - Must not exceed 256 characters
/// - Must start with an ASCII letter or underscore
/// - Must contain only ASCII letters, digits, or underscores
/// - Must not contain interior null bytes
///
/// Every one of these is something that would actually break: a name needing
/// quotes in SQL, a name starting with a digit that the parser reads as a
/// number, or a name a C string truncates. Casing is **not** checked — see the
/// [module docs][crate::validate::function_name] for why enforcing `snake_case`
/// here would make a name `DuckDB` accepts unregisterable.
///
/// # Errors
///
/// Returns `ExtensionError` describing the first rule violation found.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_function_name;
///
/// assert!(validate_function_name("word_count").is_ok());
/// assert!(validate_function_name("my_func_v2").is_ok());
/// assert!(validate_function_name("_internal").is_ok());
/// // DuckDB ships `formatReadableSize`; mixed case is legal.
/// assert!(validate_function_name("formatReadableSize").is_ok());
///
/// assert!(validate_function_name("").is_err());        // empty
/// assert!(validate_function_name("my-func").is_err());  // needs quoting in SQL
/// assert!(validate_function_name("1func").is_err());    // parsed as a number
/// assert!(validate_function_name("my func").is_err());  // needs quoting in SQL
/// ```
pub fn validate_function_name(name: &str) -> Result<(), ExtensionError> {
    if name.is_empty() {
        return Err(ExtensionError::new("function name must not be empty"));
    }

    if name.len() > MAX_FUNCTION_NAME_LEN {
        return Err(ExtensionError::new(format!(
            "function name must not exceed {MAX_FUNCTION_NAME_LEN} characters, got {}",
            name.len()
        )));
    }

    // Check for interior null bytes (would truncate the CString)
    if name.bytes().any(|b| b == 0) {
        return Err(ExtensionError::new(
            "function name must not contain null bytes",
        ));
    }

    let first = name.as_bytes()[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return Err(ExtensionError::new(format!(
            "function name must start with a letter or underscore, got '{}'",
            name.chars().next().unwrap_or('?')
        )));
    }

    for (i, ch) in name.chars().enumerate() {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err(ExtensionError::new(format!(
                "function name contains invalid character '{ch}' at position {i}; \
                 only letters, digits, and underscores are allowed (a name needing \
                 quotes in SQL is not worth the trouble it causes callers)"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple() {
        assert!(validate_function_name("word_count").is_ok());
    }

    #[test]
    fn valid_with_digits() {
        assert!(validate_function_name("my_func_v2").is_ok());
    }

    #[test]
    fn valid_underscore_prefix() {
        assert!(validate_function_name("_internal").is_ok());
    }

    #[test]
    fn valid_single_char() {
        assert!(validate_function_name("f").is_ok());
    }

    #[test]
    fn mixed_case_is_accepted_because_duckdb_accepts_it() {
        // DuckDB ships these two, and registering a camelCase name through the
        // C API succeeds — verified against DuckDB 1.5.5 in
        // tests/ffi_roundtrip.rs. Since this validator gates
        // `ScalarFunctionBuilder::try_new`, rejecting them made a name DuckDB
        // accepts impossible to register at all.
        assert!(validate_function_name("formatReadableSize").is_ok());
        assert!(validate_function_name("formatReadableDecimalSize").is_ok());
        assert!(validate_function_name("MyFunc").is_ok());
        assert!(validate_function_name("_Internal2").is_ok());
    }

    #[test]
    fn names_needing_quotes_are_still_rejected() {
        for name in [
            "my-func", "my func", "my.func", "my\"func", "my'func", "1func", "+",
        ] {
            assert!(
                validate_function_name(name).is_err(),
                "{name} should be rejected"
            );
        }
    }

    #[test]
    fn empty_rejected() {
        let err = validate_function_name("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn hyphen_rejected() {
        let err = validate_function_name("my-func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn starts_with_digit_rejected() {
        let err = validate_function_name("1func").unwrap_err();
        assert!(err.as_str().contains("letter or underscore"));
    }

    #[test]
    fn space_rejected() {
        let err = validate_function_name("my func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn special_char_rejected() {
        let err = validate_function_name("my@func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn null_byte_rejected() {
        let err = validate_function_name("my\0func").unwrap_err();
        assert!(err.as_str().contains("null bytes"));
    }

    #[test]
    fn too_long_rejected() {
        let long_name: String = "a".repeat(257);
        let err = validate_function_name(&long_name).unwrap_err();
        assert!(err.as_str().contains("256 characters"));
    }

    #[test]
    fn max_length_accepted() {
        let max_name: String = "a".repeat(256);
        assert!(validate_function_name(&max_name).is_ok());
    }

    #[test]
    fn semicolon_rejected() {
        let err = validate_function_name("func;drop").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn quote_rejected() {
        let err = validate_function_name("func'name").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }
}
