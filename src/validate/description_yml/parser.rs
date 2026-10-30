// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use crate::error::ExtensionError;
use crate::validate::platform::{
    is_opt_in_platform, DUCKDB_CI_PLATFORMS, DUCKDB_OPT_IN_PLATFORMS, DUCKDB_PLATFORM_GROUPS,
    DUCKDB_RETIRED_PLATFORMS,
};
use crate::validate::{
    validate_community_extension_name, validate_excluded_platforms_str, validate_extension_version,
    validate_platform, validate_spdx_license,
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
/// - `extension.name` — must pass [`validate_community_extension_name`]: well
///   formed, and not a `DuckDB` alias (`s3`) or core extension name (`json`)
/// - `extension.description` — non-empty
/// - `extension.version` — optional; if present, must pass [`validate_extension_version`]
/// - `extension.language`, `extension.build` — non-empty
/// - `extension.license` — non-empty (`licence:` is accepted, with a warning).
///   A value [`validate_spdx_license`] does not accept is a **warning**, not an
///   error: the community build does not read the field, and published
///   extensions use `BSL 1.1`, `GPL-3.0` and free text
/// - `extension.excluded_platforms` — if present, must pass
///   [`validate_excluded_platforms_str`]; a YAML list is validated too, and warned about.
///   A platform *group* name (`wasm`, `windows`, …) passes, with a warning: the
///   community build compares exact platform names, so a group excludes nothing
/// - `extension.opt_in_platforms` — if present, a warning for each name the
///   build would ignore: a group, an unknown name, or a platform that is built
///   without opting in
/// - `extension.maintainers` — at least one name
/// - `repo.github` — `owner/repo`, in the characters GitHub allows (not a URL)
/// - `repo.ref` — non-empty; anything but a commit hash is a warning
/// - `repo.variegata`, `repo.andium` — if present, non-empty (the revisions the
///   community build checks out for `DuckDB` v1.5.6 and v1.4.5); anything but a
///   commit hash is a warning
/// - any other `repo:` key but `ref_next` and `canonical_name` is a warning:
///   the community build ignores it
/// - no key may appear twice in a section
///
/// # Errors
///
/// Returns [`ExtensionError`] on the first validation failure with a descriptive message.
///
/// # Note on parsing
///
/// quack-rs has no YAML dependency, so this reads `description.yml` with a
/// small hand-written reader, held to what `PyYAML`'s `yaml.safe_load` — the
/// first thing the community build runs on the file — accepts: a file it
/// refuses is refused here too, `docs:` and every other section included.
/// That covers the plain-scalar rules (no `: `, no leading `@`, `` ` ``, `%`
/// or `- `, no tab), tabs, control characters, block-scalar headers and
/// indentation, quoted and flow scalars across lines, anchors and aliases,
/// and dates `PyYAML` cannot construct. Checked against the 395 published
/// descriptors (all accepted, every field as `PyYAML` reads it but block
/// scalars, which are trimmed) and 41,423 mutants of them (none accepted that
/// `PyYAML` refuses).
///
/// It is stricter than `PyYAML` in a few places no published file needs:
/// quoted or repeated keys in `extension:` and `repo:`, and anywhere tags,
/// explicit `? key` entries, directives, anchors on keys and a second
/// document.
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
///   ref: 4f1c2e9a7b3d5c8e1f0a2b4c6d8e0f1a3b5c7d9e
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
/// [`validate_community_extension_name`]: crate::validate::validate_community_extension_name
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
    validate_community_extension_name(&name)
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

    let (excluded_platforms, opt_in_platforms) = extension.platforms(&mut warnings)?;

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
    if !crate::validate::is_github_repo(&github) {
        return Err(ExtensionError::new(format!(
            "description.yml: 'repo.github' must be in 'owner/repo' format — the repository, \
             not its URL or a path inside it, in the characters GitHub allows (the community \
             build passes it to actions/checkout as `repository`), got '{github}'"
        )));
    }

    let git_ref = repo.scalar("ref")?;
    if git_ref.is_empty() {
        return Err(missing("repo.ref"));
    }
    warn_unless_commit_hash("repo.ref", &git_ref, &mut warnings);
    let git_ref_next = repo.scalar("ref_next")?;
    let git_ref_variegata = repo.release_ref("variegata", "v1.5.6", &mut warnings)?;
    let git_ref_andium = repo.release_ref("andium", "v1.4.5", &mut warnings)?;
    warn_about_unknown_repo_keys(&repo, &mut warnings);

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
        opt_in_platforms,
        maintainers,
        github,
        git_ref,
        git_ref_next,
        git_ref_variegata,
        git_ref_andium,
        warnings,
    })
}

/// The `repo:` keys `scripts/build.py` in `duckdb/community-extensions`
/// reads.
const REPO_KEYS: [&str; 6] = [
    "github",
    "ref",
    "ref_next",
    "variegata",
    "andium",
    "canonical_name",
];

/// Warns about each `repo:` key the community build does not read: a typo
/// such as `next:` (one published file has it) silently does nothing.
fn warn_about_unknown_repo_keys(repo: &Fields<'_>, warnings: &mut Vec<String>) {
    for entry in repo.entries {
        if !REPO_KEYS.contains(&entry.key.as_str()) {
            warnings.push(format!(
                "repo.{}: the community build does not read this key (it reads {}), so it has \
                 no effect",
                entry.key,
                REPO_KEYS.join(", ")
            ));
        }
    }
}

/// The platforms `group` stands for, as the community CI builds them:
/// `wasm` is `wasm_eh`, `wasm_mvp` and `wasm_threads`.
fn platforms_in_group(group: &str) -> Vec<&'static str> {
    DUCKDB_CI_PLATFORMS
        .iter()
        .copied()
        .filter(|p| {
            p.strip_prefix(group)
                .is_some_and(|rest| rest.starts_with('_'))
        })
        .collect()
}

/// Warns about each platform group name in `list`, the value of `field`.
///
/// `DUCKDB_PLATFORM_GROUPS` are the top-level keys of the distribution matrix,
/// and published files do write them in `excluded_platforms`. The script that
/// applies the field, `scripts/modify_distribution_matrix.py` in
/// `extension-ci-tools`, tests `arch in excluded_arch_values` on every branch
/// the community build uses (`v1.4-andium`, `v1.5-variegata`, `main`,
/// `v2.0-cyanoptera`), so a group name matches no platform and the build
/// carries on building all of them.
fn warn_about_platform_groups(field: &str, list: &str, warnings: &mut Vec<String>) {
    for group in list
        .split(';')
        .filter(|p| DUCKDB_PLATFORM_GROUPS.contains(p))
    {
        warnings.push(format!(
            "extension.{field}: '{group}' is a platform group, not a platform. The community \
             build compares exact platform names, so it matches nothing here; list the \
             platforms instead: \"{}\"",
            platforms_in_group(group).join(";")
        ));
    }
}

/// Warns about each name in `opt_in_platforms` the community build would ignore.
///
/// The build only looks the field up for platforms that are opt-in
/// ([`DUCKDB_OPT_IN_PLATFORMS`]), so anything else in it does nothing, and a
/// misspelt platform is a silent omission.
fn warn_about_ignored_opt_ins(list: &str, warnings: &mut Vec<String>) {
    warn_about_platform_groups("opt_in_platforms", list, warnings);
    for platform in list.split(';').filter(|p| !p.is_empty()) {
        if DUCKDB_PLATFORM_GROUPS.contains(&platform) || is_opt_in_platform(platform) {
            continue;
        }
        let why = if validate_platform(platform).is_ok() {
            "is not an opt-in platform, so the community build ignores it here"
        } else {
            "is not a recognized DuckDB build target, so the community build ignores it"
        };
        warnings.push(format!(
            "extension.opt_in_platforms: '{platform}' {why} (the opt-in platforms are {})",
            DUCKDB_OPT_IN_PLATFORMS.join(", ")
        ));
    }
}

/// Warns about each field this parser reads as text whose unquoted value
/// `PyYAML` — which the community build reads the file with, following YAML
/// 1.1 — reads as something else: an unquoted `yes` is a boolean there and
/// `0.10` the float 0.1. Fields this parser does not read
/// (`custom_toolchain_script: true`, a real boolean) are left alone.
fn warn_about_yaml_1_1_retyping(sections: &[&Fields<'_>], warnings: &mut Vec<String>) {
    const READ_AS_TEXT: [&str; 15] = [
        "name",
        "description",
        "version",
        "language",
        "build",
        "license",
        "licence",
        "requires_toolchains",
        "excluded_platforms",
        "opt_in_platforms",
        "github",
        "ref",
        "ref_next",
        "variegata",
        "andium",
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
            // Not trimmed: the reader already strips a plain scalar, so trimming
            // here would only alter a quoted one — `name: " my_ext"` is
            // " my_ext" to PyYAML and the community build.
            Some((Value::Scalar(s), _)) => Ok(s.clone()),
            Some((_, line)) => Err(ExtensionError::new(format!(
                "description.yml: line {line}: '{}.{key}' must be a single value, not a list \
                 or mapping",
                self.section
            ))),
        }
    }

    /// `repo.variegata` / `repo.andium`: the revision the community build
    /// checks out for the `DuckDB` release `release` (`scripts/build.py`: `if
    /// 'andium' in desc['repo']`) instead of `ref`. Empty when absent; present but empty is
    /// an error, because the build then gets no revision at all for that
    /// release. Anything but a commit hash is a warning, as for `ref`.
    fn release_ref(
        &self,
        key: &str,
        release: &str,
        warnings: &mut Vec<String>,
    ) -> Result<String, ExtensionError> {
        let Some(entry) = self.get(key) else {
            return Ok(String::new());
        };
        let value = self.scalar(key)?;
        if value.is_empty() {
            return Err(ExtensionError::new(format!(
                "description.yml: line {}: 'repo.{key}' is present but empty: the community \
                 build then checks out no revision when it builds for DuckDB {release}; give \
                 a commit hash or remove the key",
                entry.line
            )));
        }
        warn_unless_commit_hash(&format!("repo.{key}"), &value, warnings);
        Ok(value)
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

    /// `excluded_platforms` and `opt_in_platforms`, in that order: the first
    /// validated, both warned about where the community build would ignore them.
    fn platforms(&self, warnings: &mut Vec<String>) -> Result<(String, String), ExtensionError> {
        let excluded = self.semicolon_list("excluded_platforms", warnings)?;
        validate_excluded_platforms_str(&excluded).map_err(|e| {
            ExtensionError::new(format!(
                "description.yml: extension.excluded_platforms: {e}"
            ))
        })?;
        for retired in excluded
            .split(';')
            .filter(|p| DUCKDB_RETIRED_PLATFORMS.contains(p))
        {
            warnings.push(format!(
                "extension.excluded_platforms: '{retired}' is no longer built by DuckDB, so \
                 excluding it has no effect"
            ));
        }
        warn_about_platform_groups("excluded_platforms", &excluded, warnings);

        let opt_in = self.semicolon_list("opt_in_platforms", warnings)?;
        warn_about_ignored_opt_ins(&opt_in, warnings);
        Ok((excluded, opt_in))
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

/// Warns when `field` (`repo.ref` and its per-release variants) is not a
/// commit hash: the community build checks out exactly that revision, so a
/// branch builds whatever it points at on the day.
fn warn_unless_commit_hash(field: &str, git_ref: &str, warnings: &mut Vec<String>) {
    if !is_commit_hash(git_ref) {
        warnings.push(format!(
            "{field} '{git_ref}' is not a commit hash: the community build checks out \
             exactly this revision, so a branch builds whatever it points at on the day, and \
             DuckDB's documentation asks for the hash of the commit to build"
        ));
    }
}

/// Whether `git_ref` looks like a commit hash: 7 to 64 hexadecimal digits
/// (an abbreviated or full SHA-1, or a SHA-256 object name).
fn is_commit_hash(git_ref: &str) -> bool {
    (7..=64).contains(&git_ref.len()) && git_ref.bytes().all(|b| b.is_ascii_hexdigit())
}
