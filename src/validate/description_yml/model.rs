// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

/// A validated representation of a `DuckDB` community extension `description.yml`.
///
/// Construct via [`parse_description_yml`] or [`validate_description_yml_str`].
///
/// All fields are validated during construction — if a `DescriptionYml` is present,
/// it passed all community extension submission requirements.
///
/// [`parse_description_yml`]: super::parse_description_yml
/// [`validate_description_yml_str`]: super::validate_description_yml_str
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
