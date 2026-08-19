// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `DuckDB` build platform validation.
//!
//! `DuckDB` community extensions must build for a standard set of platforms.
//! Extensions that cannot support a platform must declare it in
//! `extension.excluded_platforms`. This module validates those declarations.
//!
//! # Reference
//!
//! The list below is derived from `config/distribution_matrix.json` in
//! [`duckdb/extension-ci-tools`], which is the file the community-extensions
//! build actually reads. `scripts/check-platform-table.py` re-derives it and
//! fails CI when the two diverge, because a stale list is worse than none: it
//! rejects platforms that exist and accepts ones that do not.
//!
//! [`duckdb/extension-ci-tools`]: https://github.com/duckdb/extension-ci-tools/blob/main/config/distribution_matrix.json

use crate::error::ExtensionError;

/// Every platform identifier [`validate_platform`] recognises.
///
/// This is [`DUCKDB_CI_PLATFORMS`] — the architectures the community-extension
/// CI builds — plus two kinds of name that appear in real, accepted
/// `description.yml` files and are therefore not errors:
///
/// - **`windows_amd64_rtools`**, the R-tools Windows build (`DuckDBPlatform()`
///   emits it under `DUCKDB_PLATFORM_RTOOLS`). It is not in the distribution
///   matrix, but 14 of the 43 published extensions sampled exclude it.
/// - **The group names** in [`DUCKDB_PLATFORM_GROUPS`] — `linux`, `osx`,
///   `wasm`, `windows` — which are the top-level keys of
///   `distribution_matrix.json` and appear in at least one published
///   extension's exclusion list.
///
/// An extension must either build for all of the non-opt-in CI platforms or
/// declare the ones it cannot in `extension.excluded_platforms`.
///
/// `linux_amd64_gcc4` is **not** here. `DuckDB` retired the legacy CXX ABI
/// target: `DuckDBPlatform()` in `duckdb/common/platform.hpp` now raises a
/// compile error for it rather than emitting a `_gcc4` suffix, and it is absent
/// from the distribution matrix. Excluding it is a no-op.
pub const DUCKDB_PLATFORMS: &[&str] = &[
    "linux",
    "linux_amd64",
    "linux_amd64_musl",
    "linux_arm64",
    "linux_arm64_musl",
    "osx",
    "osx_amd64",
    "osx_arm64",
    "wasm",
    "wasm_eh",
    "wasm_mvp",
    "wasm_threads",
    "windows",
    "windows_amd64",
    "windows_amd64_mingw",
    "windows_amd64_rtools",
    "windows_arm64",
];

/// The platforms the community-extension CI actually builds.
///
/// Exactly the `duckdb_arch` values in `distribution_matrix.json`, kept in sync
/// by `scripts/check-platform-table.py`. [`DUCKDB_PLATFORMS`] is a superset:
/// see its documentation for what else is a legal name.
pub const DUCKDB_CI_PLATFORMS: &[&str] = &[
    "linux_amd64",
    "linux_amd64_musl",
    "linux_arm64",
    "linux_arm64_musl",
    "osx_amd64",
    "osx_arm64",
    "wasm_eh",
    "wasm_mvp",
    "wasm_threads",
    "windows_amd64",
    "windows_amd64_mingw",
    "windows_arm64",
];

/// The group names `distribution_matrix.json` organises its architectures
/// under, which appear in real `excluded_platforms` fields as a shorthand for
/// "all of this group".
pub const DUCKDB_PLATFORM_GROUPS: &[&str] = &["linux", "osx", "wasm", "windows"];

/// The [`DUCKDB_CI_PLATFORMS`] that are **opt-in**: the community CI does not
/// build them unless an extension asks for them.
///
/// Listing one of these in `excluded_platforms` has no effect — it was never
/// going to be built. [`validate_excluded_platforms`] accepts them anyway,
/// since a redundant exclusion is not an error, but
/// [`is_opt_in_platform`] lets a caller warn about it.
pub const DUCKDB_OPT_IN_PLATFORMS: &[&str] =
    &["linux_amd64_musl", "linux_arm64_musl", "windows_arm64"];

/// Returns `true` if `platform` is only built when an extension opts into it.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::platform::is_opt_in_platform;
///
/// assert!(is_opt_in_platform("linux_arm64_musl"));
/// assert!(!is_opt_in_platform("linux_amd64"));
/// ```
#[must_use]
pub fn is_opt_in_platform(platform: &str) -> bool {
    DUCKDB_OPT_IN_PLATFORMS.contains(&platform)
}

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
/// assert!(validate_platform("linux_arm64_musl").is_ok());
/// assert!(validate_platform("windows_arm32").is_err());
/// // Retired upstream — DuckDB no longer builds it.
/// assert!(validate_platform("linux_amd64_gcc4").is_err());
/// assert!(validate_platform("").is_err());
/// ```
pub fn validate_platform(platform: &str) -> Result<(), ExtensionError> {
    if platform.is_empty() {
        return Err(ExtensionError::new("platform identifier must not be empty"));
    }

    if DUCKDB_PLATFORMS.contains(&platform) {
        return Ok(());
    }
    // The one stale name worth naming: it appears in older DuckDB
    // documentation and in extensions written against it.
    if platform == "linux_amd64_gcc4" {
        return Err(ExtensionError::new(
            "platform 'linux_amd64_gcc4' no longer exists; DuckDB retired the legacy \
             CXX ABI target and no longer builds it. Remove it from excluded_platforms.",
        ));
    }
    Err(ExtensionError::new(format!(
        "platform '{platform}' is not a recognized DuckDB build target; valid targets: {}",
        DUCKDB_PLATFORMS.join(", ")
    )))
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
        // A trailing ';' is common in real files and yields an empty segment;
        // it means nothing, so it is skipped rather than reported as a bogus
        // platform named "".
        if platform.is_empty() {
            continue;
        }
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
        // `DuckDBPlatform()` can emit "freebsd_amd64", but the community CI
        // never builds it, so it cannot be excluded from a build that does not
        // happen.
        let err = validate_platform("freebsd_amd64").unwrap_err();
        assert!(err.as_str().contains("not a recognized"));
    }

    #[test]
    fn retired_gcc4_platform_says_why() {
        let err = validate_platform("linux_amd64_gcc4").unwrap_err();
        assert!(
            err.as_str().contains("no longer exists"),
            "a stale name deserves a better message than \"not recognized\": {err}"
        );
    }

    #[test]
    fn musl_platforms_are_recognised() {
        // Present in extension-ci-tools' distribution_matrix.json as opt-in
        // targets; rejecting them made a legitimate exclusion un-declarable.
        assert!(validate_platform("linux_amd64_musl").is_ok());
        assert!(validate_platform("linux_arm64_musl").is_ok());
        assert!(validate_excluded_platforms(&["linux_arm64_musl"]).is_ok());
    }

    #[test]
    fn opt_in_platforms_are_a_subset() {
        for &platform in DUCKDB_OPT_IN_PLATFORMS {
            assert!(
                DUCKDB_PLATFORMS.contains(&platform),
                "{platform} is opt-in but not a known platform"
            );
            assert!(is_opt_in_platform(platform));
        }
        assert!(!is_opt_in_platform("linux_amd64"));
    }

    #[test]
    fn platform_list_has_no_duplicates() {
        let unique: std::collections::HashSet<_> = DUCKDB_PLATFORMS.iter().collect();
        assert_eq!(unique.len(), DUCKDB_PLATFORMS.len());
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
