// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use crate::error::ExtensionError;
use crate::validate::platform::DUCKDB_RETIRED_PLATFORMS;
use crate::validate::{
    validate_excluded_platforms_str, validate_extension_name, validate_extension_version,
    validate_spdx_license,
};

use super::model::DescriptionYml;
use super::yaml::{read_sections, Entry, Section, Value};
use super::yaml11;

/// Parses and validates a `description.yml` string.
///
/// Returns a validated [`DescriptionYml`] if all required fields are present and correct.
///
/// # What is validated
///
/// - `extension.name` — must pass [`validate_extension_name`]
/// - `extension.description` — non-empty
/// - `extension.version` — optional; if present, must pass [`validate_extension_version`]
/// - `extension.language`, `extension.build` — non-empty
/// - `extension.license` — non-empty (`licence:` is accepted, with a warning).
///   A value [`validate_spdx_license`] does not accept is a **warning**, not an
///   error: the community build does not read the field, and published
///   extensions use `BSL 1.1`, `GPL-3.0` and free text
/// - `extension.excluded_platforms` — if present, must pass
///   [`validate_excluded_platforms_str`]; a YAML list is validated too, and warned about
/// - `extension.maintainers` — at least one name
/// - `repo.github` — non-empty and must contain `/`
/// - `repo.ref` — non-empty
/// - no key may appear twice in a section
///
/// # Errors
///
/// Returns [`ExtensionError`] on the first validation failure with a descriptive message.
///
/// # Note on parsing
///
/// quack-rs has no YAML dependency, so this reads the subset of YAML that
/// `description.yml` files use with a small hand-written reader. It respects
/// indentation (a nested mapping's keys are not the section's), quotes (a `#`
/// inside them is not a comment), block and flow sequences, block scalars and
/// multi-line plain scalars, and a leading byte-order mark. Anchors, aliases,
/// tags and nested flow collections are reported as unsupported rather than
/// misread. Only `extension:` and `repo:` are parsed; `docs:` and any other
/// section is free-form and skipped.
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
/// assert_eq!(desc.version.as_deref(), Some("0.1.0"));
/// assert_eq!(desc.license, "MIT");
/// assert_eq!(desc.github, "janedoe/duckdb-my-ext");
/// assert_eq!(desc.maintainers, vec!["Jane Doe"]);
/// assert!(desc.warnings.is_empty());
/// ```
///
/// [`validate_extension_name`]: crate::validate::validate_extension_name
/// [`validate_extension_version`]: crate::validate::validate_extension_version
/// [`validate_spdx_license`]: crate::validate::validate_spdx_license
/// [`validate_excluded_platforms_str`]: crate::validate::validate_excluded_platforms_str
pub fn parse_description_yml(content: &str) -> Result<DescriptionYml, ExtensionError> {
    let sections = read_sections(content, &["extension", "repo"])
        .map_err(|e| ExtensionError::new(format!("description.yml: {e}")))?;
    let extension = Fields::new(&sections, "extension");
    let repo = Fields::new(&sections, "repo");
    let mut warnings = Vec::new();

    let name = extension.scalar("name")?;
    if name.is_empty() {
        return Err(missing("extension.name"));
    }
    validate_extension_name(&name)
        .map_err(|e| ExtensionError::new(format!("description.yml: extension.name: {e}")))?;

    let description = extension.required("description")?;

    let version = extension.scalar("version")?;
    let version = if version.is_empty() {
        None
    } else {
        validate_extension_version(&version)
            .map_err(|e| ExtensionError::new(format!("description.yml: extension.version: {e}")))?;
        Some(version)
    };

    let language = extension.required("language")?;
    let build = extension.required("build")?;

    let license = match (extension.get("license"), extension.get("licence")) {
        (Some(first), Some(second)) => {
            return Err(ExtensionError::new(format!(
                "description.yml: both 'extension.license' (line {}) and \
                 'extension.licence' (line {}) are set",
                first.line, second.line
            )))
        }
        (None, Some(_)) => {
            warnings.push(
                "'extension.licence' is the British spelling; DuckDB's documentation and \
                 tooling use 'license'"
                    .to_string(),
            );
            extension.scalar("licence")?
        }
        _ => extension.scalar("license")?,
    };
    if license.is_empty() {
        return Err(missing("extension.license"));
    }
    if let Err(e) = validate_spdx_license(&license) {
        warnings.push(format!("extension.license: {e}"));
    }

    // `requires_toolchains` is optional. Only 143 of the 346 published
    // descriptors (community-extensions `5ae7df8`) set it, and the
    // community-extensions documentation does not list it as required.
    let requires_toolchains = extension.semicolon_list("requires_toolchains", &mut warnings)?;

    let excluded_platforms = extension.semicolon_list("excluded_platforms", &mut warnings)?;
    validate_excluded_platforms_str(&excluded_platforms).map_err(|e| {
        ExtensionError::new(format!(
            "description.yml: extension.excluded_platforms: {e}"
        ))
    })?;
    for retired in excluded_platforms
        .split(';')
        .filter(|p| DUCKDB_RETIRED_PLATFORMS.contains(p))
    {
        warnings.push(format!(
            "extension.excluded_platforms: '{retired}' is no longer built by DuckDB, so \
             excluding it has no effect"
        ));
    }

    let maintainers = extension.maintainers(&mut warnings)?;
    if maintainers.is_empty() {
        return Err(ExtensionError::new(
            "description.yml: 'extension.maintainers' must list at least one maintainer",
        ));
    }

    let github = repo.scalar("github")?;
    if github.is_empty() {
        return Err(missing("repo.github"));
    }
    if !github.contains('/') {
        return Err(ExtensionError::new(format!(
            "description.yml: 'repo.github' must be in 'owner/repo' format, got '{github}'"
        )));
    }

    let git_ref = repo.scalar("ref")?;
    if git_ref.is_empty() {
        return Err(missing("repo.ref"));
    }
    let git_ref_next = repo.scalar("ref_next")?;

    warn_about_yaml_1_1_retyping(&[&extension, &repo], &mut warnings);

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
        git_ref_next,
        warnings,
    })
}

/// Warns about each field this parser reads as text whose unquoted value
/// `PyYAML` — which the community build reads the file with, following YAML
/// 1.1 — reads as something else: an unquoted `yes` is a boolean there and
/// `0.10` the float 0.1. Fields this parser does not read
/// (`custom_toolchain_script: true`, a real boolean) are left alone.
fn warn_about_yaml_1_1_retyping(sections: &[&Fields<'_>], warnings: &mut Vec<String>) {
    const READ_AS_TEXT: [&str; 12] = [
        "name",
        "description",
        "version",
        "language",
        "build",
        "license",
        "licence",
        "requires_toolchains",
        "excluded_platforms",
        "github",
        "ref",
        "ref_next",
    ];
    for fields in sections {
        for entry in fields.entries {
            let Value::Scalar(ref text) = entry.value else {
                continue;
            };
            if !entry.plain || !READ_AS_TEXT.contains(&entry.key.as_str()) {
                continue;
            }
            if let Some(kind) = yaml11::implicit_type(text) {
                warnings.push(format!(
                    "{}.{}: the unquoted value '{text}' is {kind}, not text, to YAML 1.1 \
                     readers such as PyYAML, which the community-extensions build uses; quote \
                     it",
                    fields.section, entry.key
                ));
            }
        }
    }
}

fn missing(field: &str) -> ExtensionError {
    ExtensionError::new(format!("description.yml: missing required field '{field}'"))
}

/// The entries of one section, with typed accessors that name the field in
/// every error.
struct Fields<'a> {
    section: &'static str,
    entries: &'a [Entry],
}

impl<'a> Fields<'a> {
    fn new(sections: &'a [Section], section: &'static str) -> Self {
        let entries = sections
            .iter()
            .find(|s| s.name == section)
            .map_or(&[][..], |s| s.entries.as_slice());
        Self { section, entries }
    }

    fn get(&self, key: &str) -> Option<&'a Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// A single-valued field; `""` when absent or empty.
    fn scalar(&self, key: &str) -> Result<String, ExtensionError> {
        match self.get(key).map(|e| (&e.value, e.line)) {
            None | Some((Value::Null, _)) => Ok(String::new()),
            Some((Value::Scalar(s), _)) => Ok(s.trim().to_string()),
            Some((_, line)) => Err(ExtensionError::new(format!(
                "description.yml: line {line}: '{}.{key}' must be a single value, not a list \
                 or mapping",
                self.section
            ))),
        }
    }

    /// A single-valued field that must be present and non-empty.
    fn required(&self, key: &str) -> Result<String, ExtensionError> {
        let value = self.scalar(key)?;
        if value.is_empty() {
            return Err(missing(&format!("{}.{key}", self.section)));
        }
        Ok(value)
    }

    /// A `;`-separated field. A YAML list is accepted and joined with `;`, with
    /// a warning: `scripts/build.py` in `duckdb/community-extensions` writes the
    /// value into the build environment as-is, so a list arrives as its Python
    /// `repr` (`['a', 'b']`), which `extension-ci-tools` then splits on `;` —
    /// and matches nothing.
    fn semicolon_list(
        &self,
        key: &str,
        warnings: &mut Vec<String>,
    ) -> Result<String, ExtensionError> {
        let Some(Entry {
            value: Value::Seq(items),
            line,
            ..
        }) = self.get(key)
        else {
            return self.scalar(key);
        };
        let mut joined = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Scalar(s) => joined.push(s.trim().to_string()),
                Value::Null => {}
                _ => {
                    return Err(ExtensionError::new(format!(
                        "description.yml: line {line}: '{}.{key}' entries must be plain values",
                        self.section
                    )))
                }
            }
        }
        let joined = joined.join(";");
        warnings.push(format!(
            "{}.{key} is a YAML list; the community build copies the value into its \
             environment as-is, so a list arrives as Python's ['a', 'b'] rather than the \
             ';'-separated string it splits — write \"{joined}\"",
            self.section
        ));
        Ok(joined)
    }

    /// `maintainers`: a list of names. A lone name is accepted with a warning.
    fn maintainers(&self, warnings: &mut Vec<String>) -> Result<Vec<String>, ExtensionError> {
        let Some(entry) = self.get("maintainers") else {
            return Ok(Vec::new());
        };
        match &entry.value {
            Value::Null => Ok(Vec::new()),
            Value::Scalar(name) => {
                warnings.push(format!(
                    "extension.maintainers should be a list; read '{name}' as its only entry"
                ));
                Ok(vec![name.trim().to_string()])
            }
            Value::Seq(items) => {
                let mut names = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::Scalar(name) if !name.trim().is_empty() => {
                            names.push(name.trim().to_string());
                        }
                        Value::Scalar(_) | Value::Null => {}
                        _ => {
                            return Err(ExtensionError::new(format!(
                                "description.yml: line {}: 'extension.maintainers' entries must \
                                 be names, not lists or mappings",
                                entry.line
                            )))
                        }
                    }
                }
                Ok(names)
            }
            Value::Mapping => Err(ExtensionError::new(format!(
                "description.yml: line {}: 'extension.maintainers' must be a list of names",
                entry.line
            ))),
        }
    }
}
