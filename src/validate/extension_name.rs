// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! Extension name validation per `DuckDB` community extension rules.
//!
//! `DuckDB` community extensions must have names matching `^[a-z0-9_-]+$` —
//! lowercase letters, digits, hyphens, and underscores only. No uppercase,
//! no spaces, no special characters.
//!
//! # Reference
//!
//! <https://duckdb.org/community_extensions/development>

use crate::error::ExtensionError;

/// Validates a `DuckDB` community extension name.
///
/// # Rules
///
/// - Must not be empty
/// - Must contain only lowercase ASCII letters (`a-z`), digits (`0-9`),
///   hyphens (`-`), or underscores (`_`)
/// - Must start with a letter
/// - Must not exceed 64 characters
///
/// # Errors
///
/// Returns `ExtensionError` describing the first rule violation found.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_extension_name;
///
/// assert!(validate_extension_name("my_extension").is_ok());
/// assert!(validate_extension_name("my-ext-2").is_ok());
/// assert!(validate_extension_name("").is_err());
/// assert!(validate_extension_name("MyExtension").is_err());
/// assert!(validate_extension_name("my extension").is_err());
/// assert!(validate_extension_name("123start").is_err());
/// ```
pub fn validate_extension_name(name: &str) -> Result<(), ExtensionError> {
    if name.is_empty() {
        return Err(ExtensionError::new("extension name must not be empty"));
    }

    if name.len() > 64 {
        return Err(ExtensionError::new(format!(
            "extension name must not exceed 64 characters, got {}",
            name.len()
        )));
    }

    if !name.as_bytes()[0].is_ascii_lowercase() {
        return Err(ExtensionError::new(format!(
            "extension name must start with a lowercase letter, got '{}'",
            name.chars().next().unwrap_or('?')
        )));
    }

    for (i, ch) in name.chars().enumerate() {
        if !matches!(ch, 'a'..='z' | '0'..='9' | '-' | '_') {
            return Err(ExtensionError::new(format!(
                "extension name contains invalid character '{ch}' at position {i}; \
                 only lowercase letters, digits, hyphens, and underscores are allowed"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple_name() {
        assert!(validate_extension_name("myext").is_ok());
    }

    #[test]
    fn valid_with_underscore() {
        assert!(validate_extension_name("my_extension").is_ok());
    }

    #[test]
    fn valid_with_hyphen() {
        assert!(validate_extension_name("my-extension").is_ok());
    }

    #[test]
    fn valid_with_digits() {
        assert!(validate_extension_name("ext2go").is_ok());
    }

    #[test]
    fn valid_mixed() {
        assert!(validate_extension_name("my-ext_v2").is_ok());
    }

    #[test]
    fn empty_name_rejected() {
        let err = validate_extension_name("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn uppercase_start_rejected() {
        let err = validate_extension_name("MyExtension").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn uppercase_mid_rejected() {
        let err = validate_extension_name("myExtension").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn space_rejected() {
        let err = validate_extension_name("my extension").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn starts_with_digit_rejected() {
        let err = validate_extension_name("123ext").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn starts_with_hyphen_rejected() {
        let err = validate_extension_name("-ext").unwrap_err();
        assert!(err.as_str().contains("start with a lowercase letter"));
    }

    #[test]
    fn special_chars_rejected() {
        let err = validate_extension_name("my@ext").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn too_long_rejected() {
        let long_name: String = "a".repeat(65);
        let err = validate_extension_name(&long_name).unwrap_err();
        assert!(err.as_str().contains("64 characters"));
    }

    #[test]
    fn max_length_accepted() {
        let max_name: String = "a".repeat(64);
        assert!(validate_extension_name(&max_name).is_ok());
    }
}
