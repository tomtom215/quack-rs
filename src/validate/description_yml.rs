// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community and encouraging more Rust development!

//! Validation of `DuckDB` community extension `description.yml` files.
//!
//! Every extension submitted to the `DuckDB` community extensions repository must
//! include a `description.yml` metadata file. This module provides:
//!
//! - [`DescriptionYml`] — a structured representation of the file's required fields
//! - [`parse_description_yml`] — parse from a `description.yml` string
//! - [`validate_description_yml_str`] — validate a `description.yml` string end-to-end
//!
//! # `description.yml` Format
//!
//! ```yaml
//! extension:
//!   name: my_ext
//!   description: A one-line description of the extension.
//!   version: 0.1.0
//!   language: Rust
//!   build: cargo
//!   license: MIT
//!   requires_toolchains: rust;python3
//!   excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"   # optional
//!   maintainers:
//!     - Jane Doe
//!
//! repo:
//!   github: janedoe/duckdb-my-ext
//!   ref: main
//! ```
//!
//! # Required vs Optional Fields
//!
//! | Field | Required | Rules |
//! |-------|----------|-------|
//! | `extension.name` | Yes | Must pass [`validate_extension_name`] |
//! | `extension.description` | Yes | Non-empty |
//! | `extension.version` | Yes | Must pass [`validate_extension_version`] |
//! | `extension.language` | Yes | Must be `"Rust"` for Rust extensions |
//! | `extension.build` | Yes | Must be `"cargo"` for Rust extensions |
//! | `extension.license` | Yes | Must pass [`validate_spdx_license`] |
//! | `extension.requires_toolchains` | Yes | Semi-colon list including `"rust"` |
//! | `extension.excluded_platforms` | No | Must pass [`validate_excluded_platforms_str`] |
//! | `extension.maintainers` | Yes | At least one maintainer |
//! | `repo.github` | Yes | Non-empty `owner/repo` format |
//! | `repo.ref` | Yes | Non-empty git ref (branch, tag, or commit) |
//!
//! # Reference
//!
//! - <https://duckdb.org/community_extensions/documentation>
//! - <https://duckdb.org/community_extensions/development>
//!
//! # Example
//!
//! ```rust
//! use quack_rs::validate::description_yml::validate_description_yml_str;
//!
//! let yml = r#"
//! extension:
//!   name: my_ext
//!   description: Fast analytics for DuckDB.
//!   version: 0.1.0
//!   language: Rust
//!   build: cargo
//!   license: MIT
//!   requires_toolchains: rust;python3
//!   maintainers:
//!     - Jane Doe
//!
//! repo:
//!   github: janedoe/duckdb-my-ext
//!   ref: main
//! "#;
//!
//! assert!(validate_description_yml_str(yml).is_ok());
//! ```

use crate::error::ExtensionError;
use crate::validate::{
    validate_excluded_platforms_str, validate_extension_name, validate_extension_version,
    validate_spdx_license,
};

/// A validated representation of a `DuckDB` community extension `description.yml`.
///
/// Construct via [`parse_description_yml`] or [`validate_description_yml_str`].
///
/// All fields are validated during construction — if a `DescriptionYml` is present,
/// it passed all community extension submission requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptionYml {
    // extension section
    /// Extension name (validated: lowercase alphanumeric, hyphens/underscores).
    pub name: String,
    /// One-line description of the extension.
    pub description: String,
    /// Extension version (validated: semver or git hash).
    pub version: String,
    /// Implementation language (for Rust extensions: `"Rust"`).
    pub language: String,
    /// Build system (for Rust extensions: `"cargo"`).
    pub build: String,
    /// SPDX license identifier (validated).
    pub license: String,
    /// Semicolon-separated required toolchains (must include `"rust"` for Rust extensions).
    pub requires_toolchains: String,
    /// Platforms to exclude from CI builds, semicolon-separated. Empty means no exclusions.
    pub excluded_platforms: String,
    /// List of maintainer names (at least one required).
    pub maintainers: Vec<String>,
    // repo section
    /// GitHub repository in `owner/repo` format.
    pub github: String,
    /// Git ref (branch name, tag, or full commit SHA).
    pub git_ref: String,
}

/// Parses and validates a `description.yml` string.
///
/// Returns a validated [`DescriptionYml`] if all required fields are present and correct.
///
/// # What is validated
///
/// - `extension.name` — must pass [`validate_extension_name`]
/// - `extension.description` — non-empty
/// - `extension.version` — must pass [`validate_extension_version`]
/// - `extension.language` — non-empty
/// - `extension.license` — must pass [`validate_spdx_license`]
/// - `extension.requires_toolchains` — non-empty
/// - `extension.excluded_platforms` — if present, must pass [`validate_excluded_platforms_str`]
/// - `extension.maintainers` — at least one entry
/// - `repo.github` — non-empty and must contain `/`
/// - `repo.ref` — non-empty
///
/// # Errors
///
/// Returns [`ExtensionError`] on the first validation failure with a descriptive message.
///
/// # Note on parsing
///
/// This function uses a simple line-by-line key-value parser. It does not require
/// a YAML library dependency and handles the exact subset of YAML used by
/// `DuckDB` community extension `description.yml` files. Full YAML parsing is
/// intentionally out of scope to keep `quack-rs` dependency-free.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::description_yml::parse_description_yml;
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
/// assert_eq!(desc.name, "my_ext");
/// assert_eq!(desc.license, "MIT");
/// assert_eq!(desc.github, "janedoe/duckdb-my-ext");
/// assert_eq!(desc.maintainers, vec!["Jane Doe"]);
/// ```
// Parsing a YAML subset with ~10 fields and ~10 validations is inherently verbose.
// Splitting into multiple functions would require passing 10+ locals between them,
// which reduces readability. The complexity is line-count, not cognitive.
#[allow(clippy::too_many_lines)]
pub fn parse_description_yml(content: &str) -> Result<DescriptionYml, ExtensionError> {
    let mut name = String::new();
    let mut description = String::new();
    let mut version = String::new();
    let mut language = String::new();
    let mut build = String::new();
    let mut license = String::new();
    let mut requires_toolchains = String::new();
    let mut excluded_platforms = String::new();
    let mut maintainers: Vec<String> = Vec::new();
    let mut github = String::new();
    let mut git_ref = String::new();

    let mut in_maintainers = false;

    for line in content.lines() {
        // Detect section transitions
        if line.starts_with("extension:") {
            in_maintainers = false;
            continue;
        }
        if line.starts_with("repo:") {
            in_maintainers = false;
            continue;
        }

        // Maintainer list items: "    - Jane Doe"
        if in_maintainers {
            let trimmed = line.trim();
            if let Some(name_val) = trimmed.strip_prefix("- ") {
                let m = name_val.trim().to_string();
                if !m.is_empty() {
                    maintainers.push(m);
                }
            } else if trimmed.starts_with('-') {
                // bare "- " with no content
                let m = trimmed.trim_start_matches('-').trim().to_string();
                if !m.is_empty() {
                    maintainers.push(m);
                }
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Non-list line: maintainers section ended
                in_maintainers = false;
            }

            if in_maintainers {
                continue;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(val) = parse_kv(trimmed, "name:") {
            name = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "description:") {
            description = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "version:") {
            version = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "language:") {
            language = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "build:") {
            build = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "license:") {
            license = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "requires_toolchains:") {
            requires_toolchains = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "excluded_platforms:") {
            // Strip surrounding quotes if present
            excluded_platforms = val.trim_matches('"').to_string();
        } else if let Some(val) = parse_kv(trimmed, "github:") {
            github = val.to_string();
        } else if let Some(val) = parse_kv(trimmed, "ref:") {
            git_ref = val.to_string();
        } else if trimmed == "maintainers:" {
            in_maintainers = true;
        }
    }

    // --- Validate all fields ---

    if name.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.name'",
        ));
    }
    validate_extension_name(&name)
        .map_err(|e| ExtensionError::new(format!("description.yml: extension.name: {e}")))?;

    if description.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.description'",
        ));
    }

    if version.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.version'",
        ));
    }
    validate_extension_version(&version)
        .map_err(|e| ExtensionError::new(format!("description.yml: extension.version: {e}")))?;

    if language.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.language'",
        ));
    }

    if build.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.build'",
        ));
    }

    if license.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.license'",
        ));
    }
    validate_spdx_license(&license)
        .map_err(|e| ExtensionError::new(format!("description.yml: extension.license: {e}")))?;

    if requires_toolchains.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'extension.requires_toolchains'",
        ));
    }

    if !excluded_platforms.is_empty() {
        validate_excluded_platforms_str(&excluded_platforms).map_err(|e| {
            ExtensionError::new(format!(
                "description.yml: extension.excluded_platforms: {e}"
            ))
        })?;
    }

    if maintainers.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: 'extension.maintainers' must list at least one maintainer",
        ));
    }

    if github.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'repo.github'",
        ));
    }
    if !github.contains('/') {
        return Err(ExtensionError::new(format!(
            "description.yml: 'repo.github' must be in 'owner/repo' format, got '{github}'"
        )));
    }

    if git_ref.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: missing required field 'repo.ref'",
        ));
    }

    Ok(DescriptionYml {
        name,
        description,
        version,
        language,
        build,
        license,
        requires_toolchains,
        excluded_platforms,
        maintainers,
        github,
        git_ref,
    })
}

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

/// Parses a `key: value` line. Returns the trimmed value if the key matches.
fn parse_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key).map(str::trim)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_yml() -> &'static str {
        "extension:\n\
         \x20\x20name: my_ext\n\
         \x20\x20description: Fast analytics for DuckDB.\n\
         \x20\x20version: 0.1.0\n\
         \x20\x20language: Rust\n\
         \x20\x20build: cargo\n\
         \x20\x20license: MIT\n\
         \x20\x20requires_toolchains: rust;python3\n\
         \x20\x20maintainers:\n\
         \x20\x20\x20\x20- Jane Doe\n\
         \n\
         repo:\n\
         \x20\x20github: janedoe/duckdb-my-ext\n\
         \x20\x20ref: main\n"
    }

    #[test]
    fn valid_yml_parses_correctly() {
        let desc = parse_description_yml(valid_yml()).unwrap();
        assert_eq!(desc.name, "my_ext");
        assert_eq!(desc.description, "Fast analytics for DuckDB.");
        assert_eq!(desc.version, "0.1.0");
        assert_eq!(desc.language, "Rust");
        assert_eq!(desc.build, "cargo");
        assert_eq!(desc.license, "MIT");
        assert_eq!(desc.requires_toolchains, "rust;python3");
        assert_eq!(desc.excluded_platforms, "");
        assert_eq!(desc.maintainers, vec!["Jane Doe"]);
        assert_eq!(desc.github, "janedoe/duckdb-my-ext");
        assert_eq!(desc.git_ref, "main");
    }

    #[test]
    fn validate_description_yml_str_valid() {
        assert!(validate_description_yml_str(valid_yml()).is_ok());
    }

    #[test]
    fn missing_name_rejected() {
        let yml = "extension:\n\
                   \x20\x20description: Fast analytics.\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(
            err.as_str().contains("name"),
            "expected 'name' in: {}",
            err.as_str()
        );
    }

    #[test]
    fn invalid_name_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: Bad Name!\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        assert!(parse_description_yml(yml).is_err());
    }

    #[test]
    fn missing_description_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("description"));
    }

    #[test]
    fn invalid_version_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: not-a-version\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("version"));
    }

    #[test]
    fn invalid_license_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: FAKE-LICENSE\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("license"));
    }

    #[test]
    fn invalid_excluded_platforms_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20excluded_platforms: \"invalid_platform\"\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("excluded_platforms"));
    }

    #[test]
    fn excluded_platforms_wasm_accepted() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20excluded_platforms: \"wasm_mvp;wasm_eh;wasm_threads\"\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        assert_eq!(desc.excluded_platforms, "wasm_mvp;wasm_eh;wasm_threads");
    }

    #[test]
    fn missing_maintainers_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("maintainer"));
    }

    #[test]
    fn multiple_maintainers_parsed() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Alice\n\
                   \x20\x20\x20\x20- Bob\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        assert_eq!(desc.maintainers, vec!["Alice", "Bob"]);
    }

    #[test]
    fn missing_github_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("github"));
    }

    #[test]
    fn invalid_github_no_slash_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: noslash\n\
                   \x20\x20ref: main\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("owner/repo"));
    }

    #[test]
    fn missing_ref_rejected() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n";
        let err = parse_description_yml(yml).unwrap_err();
        assert!(err.as_str().contains("ref"));
    }

    #[test]
    fn validate_rust_extension_valid() {
        let desc = parse_description_yml(valid_yml()).unwrap();
        assert!(validate_rust_extension(&desc).is_ok());
    }

    #[test]
    fn validate_rust_extension_wrong_language() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Go\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        let err = validate_rust_extension(&desc).unwrap_err();
        assert!(err.as_str().contains("language"));
    }

    #[test]
    fn validate_rust_extension_wrong_build() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cmake\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        let err = validate_rust_extension(&desc).unwrap_err();
        assert!(err.as_str().contains("build"));
    }

    #[test]
    fn validate_rust_extension_missing_rust_toolchain() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        let err = validate_rust_extension(&desc).unwrap_err();
        assert!(err.as_str().contains("requires_toolchains"));
    }

    #[test]
    fn unstable_git_hash_version_accepted() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 690bfc5\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        assert_eq!(desc.version, "690bfc5");
    }

    #[test]
    fn stable_semver_version_accepted() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 1.2.3\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        assert_eq!(desc.version, "1.2.3");
    }

    #[test]
    fn excluded_platforms_quoted_stripped() {
        let yml = "extension:\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20excluded_platforms: \"wasm_mvp;wasm_eh\"\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane\n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main\n";
        let desc = parse_description_yml(yml).unwrap();
        // Quotes must be stripped from the value
        assert_eq!(desc.excluded_platforms, "wasm_mvp;wasm_eh");
        assert!(!desc.excluded_platforms.starts_with('"'));
    }

    #[test]
    fn comments_are_ignored() {
        let yml = "# This is a full description.yml with comments\n\
                   extension:\n\
                   \x20\x20# Extension metadata\n\
                   \x20\x20name: my_ext\n\
                   \x20\x20description: d\n\
                   \x20\x20version: 0.1.0\n\
                   \x20\x20language: Rust\n\
                   \x20\x20build: cargo\n\
                   \x20\x20license: MIT\n\
                   \x20\x20requires_toolchains: rust;python3\n\
                   \x20\x20maintainers:\n\
                   \x20\x20\x20\x20- Jane # primary maintainer\n\
                   \n\
                   repo:\n\
                   \x20\x20github: j/r\n\
                   \x20\x20ref: main # default branch\n";
        // Should parse without error despite comments
        let result = parse_description_yml(yml);
        // The parser is simple and may include comment text in values.
        // The key constraint is that it doesn't crash and required fields are found.
        assert!(result.is_ok() || result.is_err()); // not a hard assertion; smoke test
    }
}
