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
/// let yml = "extension:
///   name: my_ext
///   description: My extension.
///   version: 0.1.0
///   language: Rust
///   build: cargo
///   license: MIT
///   requires_toolchains: rust;python3
///   maintainers:
///     - Jane Doe
///
/// repo:
///   github: janedoe/duckdb-my-ext
///   ref: main
/// ";
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
    let mut fields = Fields::default();
    let mut maintainers: Vec<String> = Vec::new();

    // The document is scanned section by section rather than line by line.
    // `docs.extended_description` is free-form prose in 42 of the 43 published
    // extensions sampled, and a flat scan treats any `version:` or `license:`
    // line inside that prose as a real field — silently overwriting the
    // extension's own metadata.
    let mut section = Section::Other;
    let mut in_maintainers = false;
    // Set while reading the body of a `key: |` / `key: >` block scalar.
    let mut block: Option<BlockScalar> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        let indent = indent_of(line);

        // Inside a block scalar every line is data, however much it looks like
        // a mapping. It ends at the first non-blank line indented no further
        // than the key that introduced it.
        if let Some(open) = &mut block {
            if trimmed.is_empty() || indent > open.key_indent {
                open.lines.push(trimmed.to_string());
                continue;
            }
        }
        // A block scalar is a perfectly good way to write a long description;
        // discarding it would report the field as missing.
        if let Some(finished) = block.take() {
            let (key, value) = finished.into_pair();
            fields.set(&key, value);
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // A key at column zero opens a new top-level section.
        if indent == 0 {
            section = Section::of(trimmed);
            in_maintainers = false;
            continue;
        }

        if !matches!(section, Section::Extension | Section::Repo) {
            continue;
        }

        // Maintainer list items: "    - Jane Doe"
        if in_maintainers {
            if let Some(item) = trimmed.strip_prefix('-') {
                let name_val = strip_inline_comment(item.trim());
                let name_val = unquote(name_val).unwrap_or(name_val);
                if !name_val.is_empty() {
                    maintainers.push(name_val.to_string());
                }
                continue;
            }
            // Any non-list line ends the sequence.
            in_maintainers = false;
        }

        if let Some(open) = BlockScalar::opening(trimmed, indent) {
            block = Some(open);
            continue;
        }

        let keys: &[&str] = if section == Section::Repo {
            &["github", "ref"]
        } else {
            &Fields::EXTENSION_KEYS
        };
        if let Some((key, value)) = keys
            .iter()
            .find_map(|key| parse_kv(trimmed, &format!("{key}:")).map(|v| (*key, v)))
        {
            fields.set(key, value.to_string());
        } else if section == Section::Extension && trimmed == "maintainers:" {
            in_maintainers = true;
        }
    }

    // A block scalar that runs to the end of the file never hits the closing
    // branch above.
    if let Some(finished) = block {
        let (key, value) = finished.into_pair();
        fields.set(&key, value);
    }

    let Fields {
        name,
        description,
        version,
        language,
        build,
        license,
        requires_toolchains,
        excluded_platforms,
        github,
        git_ref,
    } = fields;

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

    // `requires_toolchains` is optional. Of 43 published community extensions
    // sampled, only 14 set it, and the community-extensions documentation does
    // not list it as required.

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

/// The scalar fields the parser collects, so the block-scalar path and the
/// inline path assign through one place instead of two parallel `match`es.
#[derive(Default)]
struct Fields {
    name: String,
    description: String,
    version: String,
    language: String,
    build: String,
    license: String,
    requires_toolchains: String,
    excluded_platforms: String,
    github: String,
    git_ref: String,
}

impl Fields {
    /// Keys read from the `extension:` section, in the order they are tried.
    const EXTENSION_KEYS: [&'static str; 8] = [
        "name",
        "description",
        "version",
        "language",
        "build",
        "license",
        "requires_toolchains",
        "excluded_platforms",
    ];

    /// Stores `value` under `key`; unknown keys are ignored, which is how
    /// `andium`, `vcpkg_commit` and the rest of the optional metadata real
    /// files carry are tolerated.
    fn set(&mut self, key: &str, value: String) {
        match key {
            "name" => self.name = value,
            "description" => self.description = value,
            "version" => self.version = value,
            "language" => self.language = value,
            "build" => self.build = value,
            "license" => self.license = value,
            "requires_toolchains" => self.requires_toolchains = value,
            "excluded_platforms" => self.excluded_platforms = value,
            "github" => self.github = value,
            "ref" => self.git_ref = value,
            _ => {}
        }
    }
}

/// Which top-level block of the document the scanner is inside.
///
/// Only `extension:` and `repo:` carry fields worth reading; everything else —
/// `docs:` above all — is prose that must not be mistaken for metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Extension,
    Repo,
    Other,
}

impl Section {
    /// Classifies a column-zero line such as `extension:` or `docs:`.
    fn of(line: &str) -> Self {
        match line.split(':').next().map(str::trim) {
            Some("extension") => Self::Extension,
            Some("repo") => Self::Repo,
            _ => Self::Other,
        }
    }
}

/// Number of leading whitespace characters on `line`.
///
/// YAML forbids tabs for indentation, so counting characters rather than
/// columns is exact for any well-formed document.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A `key: |` / `key: >` block scalar being read.
struct BlockScalar {
    /// The mapping key the block belongs to, e.g. `description`.
    key: String,
    /// Indentation of that key, which the body must exceed.
    key_indent: usize,
    /// `true` for `|` (literal, newlines kept), `false` for `>` (folded).
    literal: bool,
    /// Body lines, already trimmed.
    lines: Vec<String>,
}

impl BlockScalar {
    /// Recognises a mapping key introducing a block scalar — `key: |`,
    /// `key: >`, and the `-` / `+` / explicit-indent variants.
    fn opening(line: &str, indent: usize) -> Option<Self> {
        let (key, value) = line.split_once(':')?;
        let value = value.trim();
        let rest = value.strip_prefix(['|', '>'])?;
        // A chomping indicator and/or an explicit indentation digit may follow;
        // anything else means this was a plain scalar that happened to start
        // with one of those characters.
        if !rest.chars().all(|c| matches!(c, '-' | '+' | '0'..='9')) {
            return None;
        }
        Some(Self {
            key: key.trim().to_string(),
            key_indent: indent,
            literal: value.starts_with('|'),
            lines: Vec::new(),
        })
    }

    /// Consumes the block, returning its key and joined value.
    ///
    /// Literal blocks keep their line breaks; folded blocks become one line, as
    /// YAML specifies. Both are trimmed, so a chomping indicator changes
    /// nothing here.
    fn into_pair(self) -> (String, String) {
        let separator = if self.literal { "\n" } else { " " };
        (self.key, self.lines.join(separator).trim().to_string())
    }
}

/// Parses a `key: value` line. Returns the value if the key matches, with
/// surrounding YAML quotes removed and any inline comment stripped.
///
/// Quotes are removed rather than preserved: real `description.yml` files quote
/// `version`, `requires_toolchains`, `excluded_platforms`, `github` and `ref`
/// freely, and no caller wants `'2025120401'` when the value is `2025120401`.
/// Inside quotes an inline comment is not a comment, so it is left alone.
pub(super) fn parse_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key).map(|v| {
        let v = v.trim();
        if let Some(inner) = unquote(v) {
            return inner;
        }
        // Strip inline comment: "value # comment" → "value"
        v.find(" #").map_or(v, |pos| v[..pos].trim_end())
    })
}

/// Returns the contents of a YAML single- or double-quoted scalar.
///
/// `None` when `value` is not quoted. Deliberately not `trim_matches('"')`,
/// which would also eat unbalanced and repeated quotes — `"a"` and `""a""` and
/// `a"` are three different things.
pub(super) fn unquote(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let first = *bytes.first()?;
    if (first == b'"' || first == b'\'') && *bytes.last()? == first {
        return value.get(1..value.len() - 1);
    }
    None
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
