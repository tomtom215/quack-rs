// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Semantic versioning validation for `DuckDB` community extensions.
//!
//! Extensions submitted to the `DuckDB` community repository must use
//! valid semantic versioning for the `extension.version` field.
//!
//! # `DuckDB` Extension Versioning Scheme
//!
//! `DuckDB` core extensions use a three-tier versioning scheme:
//!
//! | Level | Format | Example | Meaning |
//! |-------|--------|---------|---------|
//! | **Unstable** | Short git hash | `690bfc5` | No stability guarantees |
//! | **Pre-release** | `0.y.z` | `0.1.0` | Working toward stability, semver applies |
//! | **Stable** | `x.y.z` (x>0) | `1.0.0` | Full semver, backwards-compatible API |
//!
//! Use [`classify_extension_version`] to determine which tier a version falls into,
//! or [`validate_extension_version`] to accept both semver and git-hash formats.
//!
//! # Reference
//!
//! - <https://semver.org/>
//! - <https://duckdb.org/docs/extensions/versioning>

use crate::error::ExtensionError;

/// Validates that a version string is valid semantic versioning.
///
/// Accepts versions in the form `MAJOR.MINOR.PATCH` with optional
/// pre-release (`-alpha.1`) and build metadata (`+build.123`) suffixes.
///
/// # Rules
///
/// - Must have exactly three numeric components separated by dots
/// - Components must not have leading zeros (except `0` itself)
/// - Pre-release identifiers are alphanumeric with dots/hyphens
/// - Build metadata follows a `+` and is alphanumeric with dots/hyphens
///
/// # Errors
///
/// Returns `ExtensionError` if the version is not valid semver.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_semver;
///
/// assert!(validate_semver("1.0.0").is_ok());
/// assert!(validate_semver("0.1.0").is_ok());
/// assert!(validate_semver("1.2.3-alpha.1").is_ok());
/// assert!(validate_semver("1.2.3+build.456").is_ok());
/// assert!(validate_semver("1.2.3-rc.1+build.456").is_ok());
/// assert!(validate_semver("1.2").is_err());
/// assert!(validate_semver("v1.0.0").is_err());
/// assert!(validate_semver("01.0.0").is_err());
/// ```
pub fn validate_semver(version: &str) -> Result<(), ExtensionError> {
    if version.is_empty() {
        return Err(ExtensionError::new("version must not be empty"));
    }

    // Split off build metadata first (after +)
    let (version_pre, _build) = match version.split_once('+') {
        Some((v, b)) => {
            validate_identifiers(b, "build metadata")?;
            (v, Some(b))
        }
        None => (version, None),
    };

    // Split off pre-release (after -)
    let (core, _pre) = match version_pre.split_once('-') {
        Some((c, p)) => {
            validate_identifiers(p, "pre-release")?;
            (c, Some(p))
        }
        None => (version_pre, None),
    };

    // Parse core version: MAJOR.MINOR.PATCH
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(ExtensionError::new(format!(
            "version '{version}' must have exactly three components (MAJOR.MINOR.PATCH), got {}",
            parts.len()
        )));
    }

    for (i, &part) in parts.iter().enumerate() {
        let label = ["major", "minor", "patch"][i];
        validate_numeric_component(part, label, version)?;
    }

    Ok(())
}

/// The stability level of a `DuckDB` extension version.
///
/// `DuckDB` core extensions use three tiers of stability, each with different
/// expectations for API stability, release cadence, and semver semantics.
///
/// # Reference
///
/// See the [`DuckDB` extension versioning docs](https://duckdb.org/docs/extensions/versioning)
/// for the full specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionStability {
    /// Version is a short git hash (e.g., `690bfc5`).
    ///
    /// No stability guarantees. Functionality may change or be removed
    /// completely with every release. No structured release cycle.
    Unstable,
    /// Version is semver `0.y.z` (e.g., `0.1.0`).
    ///
    /// Working toward stability. Semver semantics apply, but the API is
    /// not yet considered stable. Breaking changes may occur in minor versions.
    PreRelease,
    /// Version is semver `x.y.z` where `x > 0` (e.g., `1.0.0`).
    ///
    /// Full semver semantics apply. The API is stable and will only change
    /// in backwards-incompatible ways when the major version is bumped.
    Stable,
}

impl std::fmt::Display for ExtensionStability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unstable => f.write_str("unstable"),
            Self::PreRelease => f.write_str("pre-release"),
            Self::Stable => f.write_str("stable"),
        }
    }
}

/// Classifies an extension version into its stability level.
///
/// This follows the `DuckDB` core extension versioning scheme:
/// - **Unstable**: a short git hash (7+ lowercase hex characters)
/// - **Pre-release**: semver `0.y.z`
/// - **Stable**: semver `x.y.z` where `x > 0`
///
/// # Errors
///
/// Returns `ExtensionError` if the version string is empty or does not match
/// any recognized format.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::semver::{classify_extension_version, ExtensionStability};
///
/// let (stability, _) = classify_extension_version("1.0.0").unwrap();
/// assert_eq!(stability, ExtensionStability::Stable);
///
/// let (stability, _) = classify_extension_version("0.1.0").unwrap();
/// assert_eq!(stability, ExtensionStability::PreRelease);
///
/// let (stability, _) = classify_extension_version("690bfc5").unwrap();
/// assert_eq!(stability, ExtensionStability::Unstable);
/// ```
pub fn classify_extension_version(
    version: &str,
) -> Result<(ExtensionStability, &str), ExtensionError> {
    if version.is_empty() {
        return Err(ExtensionError::new("extension version must not be empty"));
    }

    // Try semver first
    if version.contains('.') {
        validate_semver(version)?;
        let major = version.split('.').next().unwrap_or("0");
        let stability = if major == "0" {
            ExtensionStability::PreRelease
        } else {
            ExtensionStability::Stable
        };
        return Ok((stability, version));
    }

    // Try git hash: 7+ lowercase hex characters
    if version.len() >= 7
        && version.len() <= 40
        && version
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Ok((ExtensionStability::Unstable, version));
    }

    Err(ExtensionError::new(format!(
        "extension version '{version}' is not a valid semver version or git hash; \
         expected MAJOR.MINOR.PATCH or a 7-40 character lowercase hex hash"
    )))
}

/// Validates an extension version string in any of `DuckDB`'s recognized formats.
///
/// Accepts both semver versions (`1.0.0`, `0.1.0-alpha`) and unstable git
/// hashes (`690bfc5`). Use [`classify_extension_version`] if you also need
/// the stability level.
///
/// # Errors
///
/// Returns `ExtensionError` if the version is not valid.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_extension_version;
///
/// assert!(validate_extension_version("1.0.0").is_ok());
/// assert!(validate_extension_version("0.1.0").is_ok());
/// assert!(validate_extension_version("690bfc5").is_ok());
/// assert!(validate_extension_version("").is_err());
/// assert!(validate_extension_version("not-valid").is_err());
/// ```
pub fn validate_extension_version(version: &str) -> Result<(), ExtensionError> {
    classify_extension_version(version)?;
    Ok(())
}

/// Validates a single numeric version component (no leading zeros).
fn validate_numeric_component(
    s: &str,
    label: &str,
    full_version: &str,
) -> Result<(), ExtensionError> {
    if s.is_empty() {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component is empty"
        )));
    }

    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component '{s}' is not a valid number"
        )));
    }

    // No leading zeros (except "0" itself)
    if s.len() > 1 && s.starts_with('0') {
        return Err(ExtensionError::new(format!(
            "version '{full_version}': {label} component '{s}' has a leading zero"
        )));
    }

    Ok(())
}

/// Validates dot-separated identifiers (pre-release or build metadata).
fn validate_identifiers(s: &str, label: &str) -> Result<(), ExtensionError> {
    if s.is_empty() {
        return Err(ExtensionError::new(format!(
            "{label} identifier must not be empty"
        )));
    }

    for ident in s.split('.') {
        if ident.is_empty() {
            return Err(ExtensionError::new(format!(
                "{label} contains an empty identifier"
            )));
        }
        if !ident
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(ExtensionError::new(format!(
                "{label} identifier '{ident}' contains invalid characters"
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
        assert!(validate_semver("1.0.0").is_ok());
        assert!(validate_semver("0.1.0").is_ok());
        assert!(validate_semver("0.0.1").is_ok());
        assert!(validate_semver("123.456.789").is_ok());
    }

    #[test]
    fn valid_prerelease() {
        assert!(validate_semver("1.0.0-alpha").is_ok());
        assert!(validate_semver("1.0.0-alpha.1").is_ok());
        assert!(validate_semver("1.0.0-0.3.7").is_ok());
        assert!(validate_semver("1.0.0-x.7.z.92").is_ok());
        assert!(validate_semver("1.0.0-rc-1").is_ok());
    }

    #[test]
    fn valid_build_metadata() {
        assert!(validate_semver("1.0.0+build").is_ok());
        assert!(validate_semver("1.0.0+build.456").is_ok());
        assert!(validate_semver("1.0.0+20130313144700").is_ok());
    }

    #[test]
    fn valid_prerelease_and_build() {
        assert!(validate_semver("1.0.0-alpha+001").is_ok());
        assert!(validate_semver("1.0.0-rc.1+build.456").is_ok());
    }

    #[test]
    fn empty_rejected() {
        assert!(validate_semver("").is_err());
    }

    #[test]
    fn two_components_rejected() {
        let err = validate_semver("1.2").unwrap_err();
        assert!(err.as_str().contains("three components"));
    }

    #[test]
    fn four_components_rejected() {
        let err = validate_semver("1.2.3.4").unwrap_err();
        assert!(err.as_str().contains("three components"));
    }

    #[test]
    fn leading_v_rejected() {
        let err = validate_semver("v1.0.0").unwrap_err();
        assert!(err.as_str().contains("not a valid number"));
    }

    #[test]
    fn leading_zero_rejected() {
        assert!(validate_semver("01.0.0").is_err());
        assert!(validate_semver("1.01.0").is_err());
        assert!(validate_semver("1.0.01").is_err());
    }

    #[test]
    fn leading_zero_on_zero_itself_accepted() {
        assert!(validate_semver("0.0.0").is_ok());
    }

    #[test]
    fn non_numeric_rejected() {
        assert!(validate_semver("a.b.c").is_err());
        assert!(validate_semver("1.0.x").is_err());
    }

    #[test]
    fn empty_component_rejected() {
        assert!(validate_semver("1..0").is_err());
        assert!(validate_semver(".1.0").is_err());
    }

    #[test]
    fn single_number_rejected() {
        assert!(validate_semver("1").is_err());
    }

    // --- Extension version classification tests ---

    #[test]
    fn classify_stable() {
        let (stability, _) = classify_extension_version("1.0.0").unwrap();
        assert_eq!(stability, ExtensionStability::Stable);
    }

    #[test]
    fn classify_stable_high_major() {
        let (stability, _) = classify_extension_version("13.11.0").unwrap();
        assert_eq!(stability, ExtensionStability::Stable);
    }

    #[test]
    fn classify_pre_release() {
        let (stability, _) = classify_extension_version("0.1.0").unwrap();
        assert_eq!(stability, ExtensionStability::PreRelease);
    }

    #[test]
    fn classify_pre_release_with_suffix() {
        let (stability, _) = classify_extension_version("0.1.0-alpha.1").unwrap();
        assert_eq!(stability, ExtensionStability::PreRelease);
    }

    #[test]
    fn classify_unstable_git_hash() {
        let (stability, _) = classify_extension_version("690bfc5").unwrap();
        assert_eq!(stability, ExtensionStability::Unstable);
    }

    #[test]
    fn classify_unstable_long_hash() {
        let (stability, _) =
            classify_extension_version("d9e5cc104c61e4a2b3f8a9c7d1e5f0a2b4c6d8e0").unwrap();
        assert_eq!(stability, ExtensionStability::Unstable);
    }

    #[test]
    fn classify_empty_rejected() {
        assert!(classify_extension_version("").is_err());
    }

    #[test]
    fn classify_uppercase_hash_rejected() {
        assert!(classify_extension_version("690BFC5").is_err());
    }

    #[test]
    fn classify_too_short_hash_rejected() {
        assert!(classify_extension_version("abc12").is_err());
    }

    #[test]
    fn classify_not_hex_rejected() {
        assert!(classify_extension_version("not-valid").is_err());
    }

    #[test]
    fn validate_extension_version_semver() {
        assert!(validate_extension_version("1.0.0").is_ok());
        assert!(validate_extension_version("0.1.0").is_ok());
    }

    #[test]
    fn validate_extension_version_hash() {
        assert!(validate_extension_version("690bfc5").is_ok());
    }

    #[test]
    fn validate_extension_version_invalid() {
        assert!(validate_extension_version("").is_err());
        assert!(validate_extension_version("xyz").is_err());
    }

    #[test]
    fn stability_display() {
        assert_eq!(ExtensionStability::Unstable.to_string(), "unstable");
        assert_eq!(ExtensionStability::PreRelease.to_string(), "pre-release");
        assert_eq!(ExtensionStability::Stable.to_string(), "stable");
    }
}
