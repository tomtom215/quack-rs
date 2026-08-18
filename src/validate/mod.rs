// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Validation utilities for `DuckDB` community extension compliance.
//!
//! This module provides compile-time and runtime validators that enforce the
//! [`DuckDB` Community Extension](https://duckdb.org/community_extensions/development)
//! requirements. Extensions that pass these checks are ready for submission to the
//! `DuckDB` community extensions repository.
//!
//! # What is validated
//!
//! - **Extension name**: Must be lowercase alphanumeric with hyphens/underscores only
//! - **Function name**: Letters, digits and underscores — anything needing quotes in SQL is rejected
//! - **Semantic versioning**: `validate_semver` for strict semver; extension
//!   versions are checked far more loosely, because `DuckDB` specifies no format
//! - **SPDX license**: Must be a recognized SPDX license identifier
//! - **Platform targets**: Exclusions must name real `DuckDB` build targets,
//!   kept in sync with `extension-ci-tools`' distribution matrix by CI
//! - **Platform exclusion list**: Semicolon-separated string from `description.yml`
//! - **Cargo.toml**: Release profile settings for loadable extensions
//!
//! # Example
//!
//! ```rust
//! use quack_rs::validate::{
//!     validate_extension_name, validate_extension_version,
//!     validate_function_name, validate_semver, validate_spdx_license,
//!     validate_platform, validate_excluded_platforms_str,
//! };
//!
//! assert!(validate_extension_name("my_extension").is_ok());
//! assert!(validate_extension_name("MyExt").is_err()); // uppercase not allowed
//!
//! assert!(validate_function_name("word_count").is_ok());
//! // Mixed case is fine — DuckDB ships `formatReadableSize`.
//! assert!(validate_function_name("wordCount").is_ok());
//! assert!(validate_function_name("word-count").is_err()); // would need quoting in SQL
//!
//! assert!(validate_semver("1.0.0").is_ok());
//! assert!(validate_semver("not-a-version").is_err());
//!
//! // Extension versions accept both semver and git hashes (unstable)
//! assert!(validate_extension_version("0.1.0").is_ok());
//! assert!(validate_extension_version("690bfc5").is_ok());
//!
//! assert!(validate_spdx_license("MIT").is_ok());
//! assert!(validate_spdx_license("FAKE-LICENSE").is_err());
//!
//! assert!(validate_platform("linux_amd64").is_ok());
//! assert!(validate_platform("freebsd_amd64").is_err());
//!
//! // Validate the semicolon-separated excluded_platforms field from description.yml
//! assert!(validate_excluded_platforms_str("wasm_mvp;wasm_eh;wasm_threads").is_ok());
//! assert!(validate_excluded_platforms_str("invalid_platform").is_err());
//! assert!(validate_excluded_platforms_str("").is_ok()); // empty means no exclusions
//! ```

pub mod description_yml;
pub mod extension_name;
pub mod function_name;
pub mod platform;
pub mod release_profile;
pub mod semver;
pub mod spdx;

pub use extension_name::validate_extension_name;
pub use function_name::validate_function_name;
pub use platform::{
    is_opt_in_platform, validate_excluded_platforms, validate_platform, DUCKDB_CI_PLATFORMS,
    DUCKDB_OPT_IN_PLATFORMS, DUCKDB_PLATFORMS, DUCKDB_PLATFORM_GROUPS,
};
pub use release_profile::{validate_release_profile, ReleaseProfileCheck};
pub use semver::{validate_extension_version, validate_semver};
pub use spdx::validate_spdx_license;

/// Validates the `excluded_platforms` field from `description.yml` as a semicolon-separated string.
///
/// `DuckDB`'s `description.yml` stores excluded platforms as a semicolon-delimited string:
/// `"wasm_mvp;wasm_eh;wasm_threads"`. This function splits on `;` and validates each token.
///
/// An empty string is valid and means no platforms are excluded.
///
/// # Errors
///
/// Returns [`crate::error::ExtensionError`] if any token is not a known `DuckDB` build target,
/// or if the same platform appears more than once.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_excluded_platforms_str;
///
/// assert!(validate_excluded_platforms_str("").is_ok());
/// assert!(validate_excluded_platforms_str("wasm_mvp;wasm_eh").is_ok());
/// assert!(validate_excluded_platforms_str("wasm_mvp;wasm_threads;wasm_eh").is_ok());
/// assert!(validate_excluded_platforms_str("invalid_platform").is_err());
/// assert!(validate_excluded_platforms_str("linux_amd64;linux_amd64").is_err()); // duplicate
/// ```
pub fn validate_excluded_platforms_str(s: &str) -> Result<(), crate::error::ExtensionError> {
    if s.is_empty() {
        return Ok(());
    }
    let tokens: Vec<&str> = s.split(';').collect();
    validate_excluded_platforms(&tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_excluded_platforms_str_is_ok() {
        assert!(validate_excluded_platforms_str("").is_ok());
    }

    #[test]
    fn single_platform_str_valid() {
        assert!(validate_excluded_platforms_str("wasm_mvp").is_ok());
    }

    #[test]
    fn multiple_platforms_str_valid() {
        assert!(validate_excluded_platforms_str("wasm_mvp;wasm_eh;wasm_threads").is_ok());
    }

    #[test]
    fn invalid_platform_str_rejected() {
        let err = validate_excluded_platforms_str("invalid_platform").unwrap_err();
        assert!(err.as_str().contains("not a recognized"));
    }

    #[test]
    fn duplicate_platform_str_rejected() {
        let err = validate_excluded_platforms_str("linux_amd64;linux_amd64").unwrap_err();
        assert!(err.as_str().contains("duplicate"));
    }

    #[test]
    fn all_wasm_platforms_excluded_is_ok() {
        assert!(validate_excluded_platforms_str("wasm_mvp;wasm_eh;wasm_threads").is_ok());
    }
}
