// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use crate::error::ExtensionError;
use crate::validate::{
    validate_excluded_platforms_str, validate_extension_name, validate_extension_version,
    validate_spdx_license,
};

use super::model::DescriptionYml;

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
///
/// [`validate_extension_name`]: crate::validate::validate_extension_name
/// [`validate_extension_version`]: crate::validate::validate_extension_version
/// [`validate_spdx_license`]: crate::validate::validate_spdx_license
/// [`validate_excluded_platforms_str`]: crate::validate::validate_excluded_platforms_str
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
                let m = strip_inline_comment(name_val.trim()).to_string();
                if !m.is_empty() {
                    maintainers.push(m);
                }
            } else if trimmed.starts_with('-') {
                // bare "- " with no content
                let m = strip_inline_comment(trimmed.trim_start_matches('-').trim()).to_string();
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

/// Parses a `key: value` line. Returns the trimmed value if the key matches.
///
/// Inline comments (e.g., `key: value # comment`) are stripped unless the value
/// is surrounded by quotes, in which case the quoted content is returned as-is
/// (quotes included — the caller is responsible for stripping them if needed).
pub(super) fn parse_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key).map(|v| {
        let v = v.trim();
        // If the value is quoted, return it as-is (preserve content inside quotes).
        if (v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')) {
            return v;
        }
        // Strip inline comment: "value # comment" → "value"
        v.find(" #").map_or(v, |pos| v[..pos].trim_end())
    })
}

/// Strips an inline YAML comment from a value string.
///
/// Returns the portion before ` #` (space-hash), trimmed. If no inline comment
/// is found, returns the input unchanged.
pub(super) fn strip_inline_comment(value: &str) -> &str {
    value
        .find(" #")
        .map_or(value, |pos| value[..pos].trim_end())
}
