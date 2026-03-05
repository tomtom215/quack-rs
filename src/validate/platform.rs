//! `DuckDB` build platform validation.
//!
//! `DuckDB` community extensions must build for a standard set of platforms.
//! Extensions that cannot support a platform must declare it in
//! `extension.excluded_platforms`. This module validates those declarations.
//!
//! # Reference
//!
//! <https://duckdb.org/community_extensions/development>

use crate::error::ExtensionError;

/// The set of valid `DuckDB` build platform identifiers.
///
/// These are the platforms that the `DuckDB` community extension CI builds for.
/// Extensions must either build for all of them or declare exclusions.
pub const DUCKDB_PLATFORMS: &[&str] = &[
    "linux_amd64",
    "linux_amd64_gcc4",
    "linux_arm64",
    "osx_amd64",
    "osx_arm64",
    "windows_amd64",
    "windows_arm64",
    "wasm_mvp",
    "wasm_eh",
    "wasm_threads",
];

/// Validates that a platform identifier is a known `DuckDB` build target.
///
/// # Errors
///
/// Returns `ExtensionError` if the platform is empty or not in [`DUCKDB_PLATFORMS`].
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_platform;
///
/// assert!(validate_platform("linux_amd64").is_ok());
/// assert!(validate_platform("osx_arm64").is_ok());
/// assert!(validate_platform("wasm_eh").is_ok());
/// assert!(validate_platform("windows_arm32").is_err());
/// assert!(validate_platform("").is_err());
/// ```
pub fn validate_platform(platform: &str) -> Result<(), ExtensionError> {
    if platform.is_empty() {
        return Err(ExtensionError::new("platform identifier must not be empty"));
    }

    if DUCKDB_PLATFORMS.contains(&platform) {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "platform '{platform}' is not a recognized DuckDB build target; valid targets: {}",
            DUCKDB_PLATFORMS.join(", ")
        )))
    }
}

/// Validates a list of excluded platform identifiers.
///
/// Each platform must be a known `DuckDB` build target. Duplicates are flagged
/// as an error.
///
/// # Errors
///
/// Returns `ExtensionError` on the first invalid or duplicate platform.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::platform::validate_excluded_platforms;
///
/// assert!(validate_excluded_platforms(&["wasm_mvp", "wasm_eh"]).is_ok());
/// assert!(validate_excluded_platforms(&["invalid_platform"]).is_err());
/// assert!(validate_excluded_platforms(&["linux_amd64", "linux_amd64"]).is_err());
/// ```
pub fn validate_excluded_platforms(platforms: &[&str]) -> Result<(), ExtensionError> {
    let mut seen = std::collections::HashSet::new();
    for &platform in platforms {
        validate_platform(platform)?;
        if !seen.insert(platform) {
            return Err(ExtensionError::new(format!(
                "duplicate excluded platform: '{platform}'"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_platforms_valid() {
        for &platform in DUCKDB_PLATFORMS {
            assert!(
                validate_platform(platform).is_ok(),
                "expected '{platform}' to be valid"
            );
        }
    }

    #[test]
    fn linux_amd64_valid() {
        assert!(validate_platform("linux_amd64").is_ok());
    }

    #[test]
    fn osx_arm64_valid() {
        assert!(validate_platform("osx_arm64").is_ok());
    }

    #[test]
    fn wasm_valid() {
        assert!(validate_platform("wasm_mvp").is_ok());
        assert!(validate_platform("wasm_eh").is_ok());
        assert!(validate_platform("wasm_threads").is_ok());
    }

    #[test]
    fn empty_rejected() {
        assert!(validate_platform("").is_err());
    }

    #[test]
    fn unknown_platform_rejected() {
        let err = validate_platform("freebsd_amd64").unwrap_err();
        assert!(err.as_str().contains("not a recognized"));
    }

    #[test]
    fn validate_excluded_platforms_valid() {
        assert!(validate_excluded_platforms(&["wasm_mvp", "wasm_eh"]).is_ok());
    }

    #[test]
    fn validate_excluded_platforms_empty() {
        assert!(validate_excluded_platforms(&[]).is_ok());
    }

    #[test]
    fn validate_excluded_platforms_invalid() {
        assert!(validate_excluded_platforms(&["invalid"]).is_err());
    }

    #[test]
    fn validate_excluded_platforms_duplicate() {
        let err = validate_excluded_platforms(&["linux_amd64", "linux_amd64"]).unwrap_err();
        assert!(err.as_str().contains("duplicate"));
    }
}
