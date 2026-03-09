// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use crate::error::ExtensionError;

use super::model::DescriptionYml;
use super::parser::parse_description_yml;

/// Validates a `description.yml` string and returns `Ok(())` if it passes all checks.
///
/// This is a convenience wrapper around [`parse_description_yml`] for callers that
/// only need a pass/fail result.
///
/// # Errors
///
/// Returns [`ExtensionError`] on the first validation failure.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::description_yml::validate_description_yml_str;
///
/// let valid_yml = "\
/// extension:\n\
///   name: my_ext\n\
///   description: My extension.\n\
///   version: 0.1.0\n\
///   language: Rust\n\
///   build: cargo\n\
///   license: MIT\n\
///   requires_toolchains: rust;python3\n\
///   maintainers:\n\
///     - Jane Doe\n\
/// \n\
/// repo:\n\
///   github: janedoe/duckdb-my-ext\n\
///   ref: main\n";
///
/// assert!(validate_description_yml_str(valid_yml).is_ok());
///
/// // Missing required field
/// assert!(validate_description_yml_str("extension:\n  name: bad!Name\n").is_err());
/// ```
pub fn validate_description_yml_str(content: &str) -> Result<(), ExtensionError> {
    parse_description_yml(content)?;
    Ok(())
}

/// Validates that a Rust extension's `description.yml` follows pure-Rust best practices.
///
/// This function checks:
/// - `language` is `"Rust"`
/// - `build` is `"cargo"`
/// - `requires_toolchains` includes `"rust"`
///
/// These are the required values for a pure-Rust extension built with `quack-rs`.
///
/// # Errors
///
/// Returns [`ExtensionError`] if any Rust-specific field is missing or wrong.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::description_yml::{parse_description_yml, validate_rust_extension};
///
/// let yml = "\
/// extension:\n\
///   name: my_ext\n\
///   description: My extension.\n\
///   version: 0.1.0\n\
///   language: Rust\n\
///   build: cargo\n\
///   license: MIT\n\
///   requires_toolchains: rust;python3\n\
///   maintainers:\n\
///     - Jane Doe\n\
/// \n\
/// repo:\n\
///   github: janedoe/duckdb-my-ext\n\
///   ref: main\n";
///
/// let desc = parse_description_yml(yml).unwrap();
/// assert!(validate_rust_extension(&desc).is_ok());
/// ```
pub fn validate_rust_extension(desc: &DescriptionYml) -> Result<(), ExtensionError> {
    if desc.language != "Rust" {
        return Err(ExtensionError::new(format!(
            "description.yml: extension.language must be 'Rust' for a Rust extension, got '{}'",
            desc.language
        )));
    }

    if desc.build != "cargo" {
        return Err(ExtensionError::new(format!(
            "description.yml: extension.build must be 'cargo' for a Rust extension, got '{}'",
            desc.build
        )));
    }

    let toolchains: Vec<&str> = desc.requires_toolchains.split(';').collect();
    if !toolchains.iter().any(|t| t.trim() == "rust") {
        return Err(ExtensionError::new(format!(
            "description.yml: extension.requires_toolchains must include 'rust' for a Rust extension, got '{}'",
            desc.requires_toolchains
        )));
    }

    Ok(())
}
