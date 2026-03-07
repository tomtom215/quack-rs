// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SQL function name validation for `DuckDB` extensions.
//!
//! Function names registered with `DuckDB` should follow safe naming conventions
//! to avoid registration failures or unexpected behavior. This validator enforces
//! conservative rules that are compatible with `DuckDB`'s internal function catalog.

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
/// - Must start with a lowercase ASCII letter or underscore
/// - Must contain only lowercase ASCII letters, digits, or underscores
/// - Must not contain interior null bytes
///
/// These rules are intentionally conservative. `DuckDB` may accept a wider range
/// of names, but restricting to this set avoids catalog issues and makes function
/// names unambiguous in SQL queries.
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
/// assert!(validate_function_name("").is_err());        // empty
/// assert!(validate_function_name("MyFunc").is_err());   // uppercase
/// assert!(validate_function_name("my-func").is_err());  // hyphen
/// assert!(validate_function_name("1func").is_err());    // starts with digit
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
    if !first.is_ascii_lowercase() && first != b'_' {
        return Err(ExtensionError::new(format!(
            "function name must start with a lowercase letter or underscore, got '{}'",
            name.chars().next().unwrap_or('?')
        )));
    }

    for (i, ch) in name.chars().enumerate() {
        if !matches!(ch, 'a'..='z' | '0'..='9' | '_') {
            return Err(ExtensionError::new(format!(
                "function name contains invalid character '{ch}' at position {i}; \
                 only lowercase letters, digits, and underscores are allowed"
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
    fn empty_rejected() {
        let err = validate_function_name("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn uppercase_rejected() {
        let err = validate_function_name("MyFunc").unwrap_err();
        assert!(err.as_str().contains("lowercase letter or underscore"));
    }

    #[test]
    fn uppercase_mid_rejected() {
        let err = validate_function_name("myFunc").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn hyphen_rejected() {
        let err = validate_function_name("my-func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn starts_with_digit_rejected() {
        let err = validate_function_name("1func").unwrap_err();
        assert!(err.as_str().contains("lowercase letter or underscore"));
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
