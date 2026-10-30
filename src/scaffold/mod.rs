// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Project scaffolding for `DuckDB` Rust extensions.
//!
//! Generates the complete file set needed to build and submit a `DuckDB` extension
//! to the community extensions repository — **without any C++ glue**.
//!
//! # Background
//!
//! The `DuckDB` C Extension API (used by the official
//! [`extension-template-rs`](https://github.com/duckdb/extension-template-rs))
//! allows pure-Rust extensions that need only:
//!
//! - `Cargo.toml` (cdylib, `quack-rs` + `libduckdb-sys` deps)
//! - `Makefile` (delegates to `cargo build` + metadata scripts from `extension-ci-tools`)
//! - `src/lib.rs` (entry point + function registration)
//! - `extension-ci-tools/` (git submodule for CI/CD)
//! - `description.yml` (extension metadata for community submission)
//!
//! This module generates all of these from a [`ScaffoldConfig`], plus a
//! `SQLLogicTest`, two GitHub Actions workflows (fast native checks, and the
//! extension-ci-tools distribution build for every community platform) and
//! the supporting dotfiles.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};
//!
//! let config = ScaffoldConfig {
//!     name: "my_analytics".to_string(),
//!     description: "Fast analytics functions for DuckDB".to_string(),
//!     version: "0.1.0".to_string(),
//!     license: "MIT".to_string(),
//!     maintainer: "Jane Doe".to_string(),
//!     github_repo: "janedoe/duckdb-my-analytics".to_string(),
//!     // `excluded_platforms` defaults to DEFAULT_EXCLUDED_PLATFORMS.
//!     ..ScaffoldConfig::default()
//! };
//!
//! let files = generate_scaffold(&config).unwrap();
//! assert!(files.iter().any(|f| f.path == "Cargo.toml"));
//! assert!(files.iter().any(|f| f.path == "Makefile"));
//! assert!(files.iter().any(|f| f.path == "src/lib.rs"));
//! assert!(files.iter().any(|f| f.path == "description.yml"));
//! ```

mod escape;
mod templates;
mod templates_ci;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_escaping;
#[cfg(test)]
mod tests_generated;
#[cfg(test)]
mod tests_workflows;

use crate::error::ExtensionError;
use crate::validate::{validate_community_extension_name, validate_semver, validate_spdx_license};

/// Configuration for generating a new extension project.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Extension name (must pass [`validate_community_extension_name`]: well
    /// formed, and not a `DuckDB` alias such as `s3` or a core extension name
    /// such as `json`).
    pub name: String,
    /// Short description of the extension.
    ///
    /// Free text: YAML punctuation such as `: ` or ` #`, quotes, backslashes
    /// and non-ASCII text are quoted and escaped in `description.yml`, and each
    /// line becomes a `//!` line in `src/lib.rs`. It must not be empty, start
    /// or end with whitespace, or contain control characters other than
    /// newline and tab (or Unicode bidirectional controls).
    pub description: String,
    /// Initial version: semver without a leading `v` (e.g. `"0.1.0"`),
    /// because it is the `[package] version` of the generated `Cargo.toml`
    /// (and `cargo` refuses anything else) as well as the version in
    /// `description.yml`. Must pass [`validate_semver`].
    pub version: String,
    /// SPDX license identifier (must pass [`validate_spdx_license`]).
    pub license: String,
    /// Primary maintainer name: a single non-empty line, quoted and escaped in
    /// `description.yml` like [`description`][Self::description].
    pub maintainer: String,
    /// GitHub repository path (e.g., `"myorg/duckdb-my-ext"`): `owner/repo`,
    /// in the characters GitHub allows.
    pub github_repo: String,
    /// Platforms the community build skips: written as `excluded_platforms`
    /// in `description.yml` and as `exclude_archs` in the generated
    /// distribution workflow, so the two always agree.
    ///
    /// Defaults to [`DEFAULT_EXCLUDED_PLATFORMS`]; an empty list builds for
    /// every platform.
    pub excluded_platforms: Vec<String>,
    /// Value written as `TARGET_DUCKDB_VERSION` in the generated `Makefile`.
    ///
    /// `extension-ci-tools` passes this to `append_extension_metadata.py -dv`,
    /// and its meaning depends on [`use_unstable_c_api`][Self::use_unstable_c_api]:
    ///
    /// - `false` → the **C extension API** version. The extension is stamped
    ///   `C_STRUCT` and loads into any `DuckDB` whose C API version is at least
    ///   this. One of:
    ///   - [`DUCKDB_API_VERSION`][crate::DUCKDB_API_VERSION] (`"v1.2.0"`), for
    ///     an extension that stays off quack-rs's `duckdb-1-5*` features. It
    ///     loads into every `DuckDB` from v1.2.0, 2.0.0 included.
    ///   - [`DUCKDB_API_VERSION_1_5_6`][crate::DUCKDB_API_VERSION_1_5_6]
    ///     (`"v1.5.6"`), for one that uses them. `DuckDB` v1.5.6 and v2.0.0
    ///     load it and older releases refuse it. The generated `Cargo.toml`
    ///     pins `libduckdb-sys` to the v1.5.6 bindings (`~1.10506.0`), whose
    ///     layout both releases share, and the `Makefile` refuses to build when
    ///     the resolved bindings are for another release.
    /// - `true` → an exact **`DuckDB` 1.x release**, e.g. `"v1.5.5"`. The
    ///   extension is stamped `C_STRUCT_UNSTABLE` and `DuckDB` refuses to load
    ///   it into any other release. The generated `Cargo.toml` then pins
    ///   `libduckdb-sys` to that release's bindings (`~1.10505.0` for v1.5.5),
    ///   the `Makefile` tests against it (`DUCKDB_TEST_VERSION`) and refuses to
    ///   build when the resolved bindings and `TARGET_DUCKDB_VERSION` disagree.
    ///   A 2.x release is refused: `DuckDB` 2.0 loads a `C_STRUCT_UNSTABLE`
    ///   binary only through the C API v2 entry point, which quack-rs does not
    ///   generate.
    ///
    /// Defaults to [`DUCKDB_API_VERSION`][crate::DUCKDB_API_VERSION].
    pub target_duckdb_version: String,
    /// Whether the generated `Makefile` sets `USE_UNSTABLE_C_API=1`, pinning
    /// the binary to one `DuckDB` 1.x release.
    ///
    /// An extension that enables quack-rs's `duckdb-1-5`, `duckdb-1-5-3` or
    /// `duckdb-1-5-4` features calls C API functions past the stable prefix of
    /// `duckdb_ext_api_v1`, so `DuckDB` must not load it into a release with a
    /// different layout (see [`crate::abi`]). The way to ensure that which
    /// also works on `DuckDB` 2.0 is to leave this `false` and set
    /// [`target_duckdb_version`][Self::target_duckdb_version] to
    /// [`DUCKDB_API_VERSION_1_5_6`][crate::DUCKDB_API_VERSION_1_5_6]. Set this
    /// only to target one release older than v1.5.6.
    ///
    /// Defaults to `false`.
    pub use_unstable_c_api: bool,
    /// Value written as `repo.ref` in the generated `description.yml`.
    ///
    /// **Must be a commit hash**, not a branch. `DuckDB`'s community-extension
    /// documentation is explicit — "Provide the hash of the latest commit on
    /// the branch targeting stable as `ref`" — because the repository builds
    /// exactly this revision and signs the result, so a moving reference would
    /// make the build unreproducible. Of the 346 `description.yml` files in
    /// `duckdb/community-extensions` at commit `5ae7df8`, 327 pin a full
    /// 40-character hash and 19 pin a tag (15 as `vX.Y.Z`, four as
    /// `refs/tags/vX.Y.Z`); none uses a branch. Letters, digits, `-`, `_`, `.`
    /// and `/` only.
    ///
    /// Defaults to [`REF_PLACEHOLDER`], which is deliberately not a valid
    /// revision so it cannot be submitted by accident.
    pub git_ref: String,
}

/// Placeholder written as `repo.ref` when [`ScaffoldConfig::git_ref`] is left
/// at its default.
///
/// Deliberately not a valid git revision: `ref` must be pinned before the
/// `description.yml` is submitted, and a scaffold that emitted `main` would
/// look correct while producing an unreproducible build.
pub const REF_PLACEHOLDER: &str = "REPLACE_WITH_COMMIT_HASH";

/// The default [`ScaffoldConfig::excluded_platforms`]: the three `DuckDB`-Wasm
/// targets and `windows_amd64_mingw`.
///
/// The community build compiles those with toolchains a Rust extension rarely
/// needs and often does not build with — emscripten, where the cdylib becomes
/// a staticlib through `src/wasm_lib.rs`, and the `MinGW` (`windows-gnu`)
/// target — and most `build: cargo` extensions in `duckdb/community-extensions`
/// exclude them. Excluding them by default keeps a new extension's first
/// community build green. To opt a platform in, drop it from
/// `excluded_platforms` (or from both `description.yml` and the workflow's
/// `exclude_archs` in a generated project) and let the distribution workflow
/// prove it builds.
pub const DEFAULT_EXCLUDED_PLATFORMS: &[&str] =
    &["wasm_mvp", "wasm_eh", "wasm_threads", "windows_amd64_mingw"];

/// The `DuckDB` release a stable-ABI (`C_STRUCT`) scaffold's distribution
/// workflow builds and tests against: its `duckdb_version`.
///
/// This is what `duckdb/community-extensions` builds an extension's `ref`
/// against (`DUCKDB_LATEST_STABLE` in its `build.yml`).
///
/// An unstable-ABI scaffold uses its
/// [`target_duckdb_version`][ScaffoldConfig::target_duckdb_version] instead,
/// since that binary loads into no other release.
pub const DISTRIBUTION_DUCKDB_VERSION: &str = "v2.0.0";

/// The `extension-ci-tools` branch a stable-ABI (`C_STRUCT`) scaffold builds
/// with, for [`DISTRIBUTION_DUCKDB_VERSION`].
///
/// The generated distribution workflow calls `_extension_distribution.yml` at
/// it and passes it as `ci_tools_version`, `.gitmodules` tracks it, and the
/// `Makefile`'s setup hint names it. `extension-ci-tools` names its 1.x
/// branches after `DuckDB` releases (`v1.5.5`, `v1.5.6`, ...) but has no
/// `v2.0.0` branch: `duckdb/community-extensions` builds `DuckDB` v2.0.0 with
/// this one.
pub const DISTRIBUTION_CI_TOOLS_REF: &str = "v2.0-cyanoptera";

/// The `extension-ci-tools` branch the generated project builds with: see
/// [`DISTRIBUTION_CI_TOOLS_REF`].
fn extension_ci_tools_ref(config: &ScaffoldConfig) -> &str {
    if config.use_unstable_c_api {
        &config.target_duckdb_version
    } else {
        DISTRIBUTION_CI_TOOLS_REF
    }
}

/// The `DuckDB` release the generated distribution workflow builds against:
/// see [`DISTRIBUTION_DUCKDB_VERSION`].
fn distribution_duckdb_version(config: &ScaffoldConfig) -> &str {
    if config.use_unstable_c_api {
        &config.target_duckdb_version
    } else {
        DISTRIBUTION_DUCKDB_VERSION
    }
}

/// Whether the generated project pins `libduckdb-sys` to the bindings of
/// [`target_duckdb_version`][ScaffoldConfig::target_duckdb_version]: every
/// build that may use quack-rs's `duckdb-1-5*` features, i.e. one stamped
/// `C_STRUCT_UNSTABLE`, or `C_STRUCT` with C API v1.5.6.
fn pins_bindings(config: &ScaffoldConfig) -> bool {
    config.use_unstable_c_api || config.target_duckdb_version == crate::DUCKDB_API_VERSION_1_5_6
}

impl Default for ScaffoldConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            version: String::from("0.1.0"),
            license: String::from("MIT"),
            maintainer: String::new(),
            github_repo: String::new(),
            excluded_platforms: DEFAULT_EXCLUDED_PLATFORMS
                .iter()
                .map(|p| (*p).to_string())
                .collect(),
            target_duckdb_version: String::from(crate::DUCKDB_API_VERSION),
            use_unstable_c_api: false,
            git_ref: String::from(REF_PLACEHOLDER),
        }
    }
}

/// A generated file with its relative path and content.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    /// Relative path from the project root (e.g., `"src/lib.rs"`).
    pub path: String,
    /// File content as a string.
    pub content: String,
}

/// Checks that `target_duckdb_version` is a `vX.Y.Z` string and is consistent
/// with `use_unstable_c_api`.
///
/// The two settings are coupled: `extension-ci-tools` feeds
/// `TARGET_DUCKDB_VERSION` to `append_extension_metadata.py -dv`, where it means
/// the C API version for a `C_STRUCT` build and an exact `DuckDB` release for a
/// `C_STRUCT_UNSTABLE` one. Getting this wrong yields a binary `DuckDB` silently
/// refuses to load ("built specifically for `DuckDB` version …").
fn validate_target_duckdb_version(config: &ScaffoldConfig) -> Result<(), ExtensionError> {
    let version = config.target_duckdb_version.as_str();

    let numeric = version.strip_prefix('v').ok_or_else(|| {
        ExtensionError::new(format!(
            "target_duckdb_version must start with 'v' (e.g. \"v1.5.5\"), got {version:?}"
        ))
    })?;
    let parts: Vec<&str> = numeric.split('.').collect();
    let well_formed = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        return Err(ExtensionError::new(format!(
            "target_duckdb_version must be 'vMAJOR.MINOR.PATCH' (e.g. \"v1.5.5\"), got {version:?}"
        )));
    }

    let stable_api = crate::DUCKDB_API_VERSION;
    let api_1_5_6 = crate::DUCKDB_API_VERSION_1_5_6;
    if !config.use_unstable_c_api && version != stable_api && version != api_1_5_6 {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = false the extension is stamped C_STRUCT, so \
             target_duckdb_version is the C extension API version and must be {stable_api:?} \
             (stable API only) or {api_1_5_6:?} (quack-rs's duckdb-1-5 features; loads into \
             DuckDB v1.5.6 and v2.0.0); got {version:?}. Set use_unstable_c_api = true to pin \
             the binary to DuckDB {version} instead."
        )));
    }

    if config.use_unstable_c_api && parts[0].parse::<u64>().is_ok_and(|major| major >= 2) {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = true the extension is stamped C_STRUCT_UNSTABLE, and \
             DuckDB {version} loads such a binary only through the C API v2 entry point \
             (<name>_init_c_api_v2), which quack-rs does not generate. Set \
             use_unstable_c_api = false and target_duckdb_version = {api_1_5_6:?}: DuckDB \
             v1.5.6 and v2.0.0 load that, duckdb-1-5 features included."
        )));
    }

    if config.use_unstable_c_api && version == crate::DUCKDB_API_VERSION {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = true the extension is stamped C_STRUCT_UNSTABLE and \
             DuckDB requires target_duckdb_version to be the exact DuckDB release it will be \
             loaded into (e.g. \"v1.5.5\"); {:?} is the C extension API version, not a \
             DuckDB release to pin to.",
            crate::DUCKDB_API_VERSION
        )));
    }

    if pins_bindings(config) {
        libduckdb_sys_requirement(version)?;
    }

    Ok(())
}

/// Checks a free-text field (`description`, `maintainer`) before it is written
/// into `description.yml` and, for the description, a `//!` doc comment.
///
/// Punctuation that means something to YAML (`: `, ` #`, quotes, backslashes)
/// is fine — the templates quote and escape it. What is refused is what no
/// escaping makes right: an empty value; leading or trailing whitespace, which
/// `description.yml` readers trim; control characters (a bare carriage return
/// is a hard error in a Rust doc comment); Unicode bidirectional controls,
/// which rustc denies in comments (`text_direction_codepoint_in_comment`); and,
/// in a single-line field, any line break.
fn validate_free_text(field: &str, text: &str, multi_line: bool) -> Result<(), ExtensionError> {
    if text.trim().is_empty() {
        return Err(ExtensionError::new(format!("{field} must not be empty")));
    }
    if text.trim() != text {
        return Err(ExtensionError::new(format!(
            "{field} must not start or end with whitespace: {text:?}"
        )));
    }
    let line_break = |c: char| matches!(c, '\n' | '\u{2028}' | '\u{2029}');
    if let Some(bad) = text.chars().find(|&c| {
        (c.is_control() && c != '\t' && !(multi_line && c == '\n'))
            || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            || (!multi_line && line_break(c))
    }) {
        return Err(ExtensionError::new(format!(
            "{field} contains the character {bad:?}, which cannot be written into the \
             generated files{}",
            if multi_line {
                ""
            } else {
                "; it must be a single line"
            }
        )));
    }
    Ok(())
}

/// `owner/repo`, in the characters GitHub allows: the value is written into
/// `description.yml`.
fn validate_github_repo(repo: &str) -> Result<(), ExtensionError> {
    if crate::validate::is_github_repo(repo) {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "github_repo must be 'owner/repo' (letters, digits, '-', and '_' / '.' in the \
             repository name), got {repo:?}"
        )))
    }
}

/// A commit hash or tag — anything else is not a revision `repo.ref` can name.
fn validate_git_ref(git_ref: &str) -> Result<(), ExtensionError> {
    let valid = !git_ref.is_empty()
        && !git_ref.starts_with('-')
        && git_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if valid {
        Ok(())
    } else {
        Err(ExtensionError::new(format!(
            "git_ref must be a commit hash or tag (letters, digits, '-', '_', '.', '/'), \
             got {git_ref:?}"
        )))
    }
}

/// The `libduckdb-sys` version requirement that compiles against exactly the
/// `DuckDB` release `target` (`vX.Y.Z`), for a build that [pins its
/// bindings](pins_bindings). Only 1.x targets reach it: a 2.x one is refused
/// before (`C_STRUCT_UNSTABLE` cannot load into 2.x, and C API v1.5.6 maps to
/// the v1.5.6 bindings).
///
/// `libduckdb-sys` numbers its releases after the `DuckDB` release whose
/// headers it ships. From `DuckDB` 1.5.0 the scheme is
/// `1.(10000·major + 100·minor + patch).N` — 1.10505.0 is `DuckDB` v1.5.5,
/// 1.20000.0 is v2.0.0 — where `N`
/// is the bindings crate's own patch release, so `~1.10505.0` admits binding
/// fixes but never another `DuckDB` release. On the 1.4 line the crate version
/// *is* the `DuckDB` version, so the pin is exact. This is the inverse of the
/// mapping in `scripts/duckdb-version-from-lock.sh`.
///
/// # Errors
///
/// A release neither scheme can express, or one older than the 1.4.4 floor
/// quack-rs itself requires.
fn libduckdb_sys_requirement(target: &str) -> Result<String, ExtensionError> {
    let numbers: Vec<u32> = target
        .trim_start_matches('v')
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| ExtensionError::new(format!("malformed DuckDB version {target:?}")))?;
    match numbers.as_slice() {
        [1, minor @ 5..=99, patch @ 0..=99] => Ok(format!("~1.{}.0", 10_000 + minor * 100 + patch)),
        [1, 4, patch] if *patch >= 4 => Ok(format!("=1.4.{patch}")),
        _ => Err(ExtensionError::new(format!(
            "cannot pin libduckdb-sys to DuckDB {target}: libduckdb-sys encodes DuckDB \
             1.Y.Z (Y >= 5) as 1.(10000 + 100*Y + Z) and ships 1.4.x as-is, and quack-rs \
             needs at least 1.4.4; no libduckdb-sys release is known to match {target}"
        ))),
    }
}

/// Generates the complete set of project files for a new `DuckDB` Rust extension.
///
/// Validates the configuration and returns a list of [`GeneratedFile`]s that can be
/// written to disk. Does NOT write files — callers decide how to persist them.
///
/// # Errors
///
/// Returns [`ExtensionError`] if the extension name, license, version (which
/// must be semver, as it is also the Cargo package version),
/// description, maintainer, GitHub repository, git ref, excluded platforms or
/// target `DuckDB` version is invalid — including a hyphenated name, which
/// `DuckDB` could never load, and a name `DuckDB` itself uses (see
/// [`validate_community_extension_name`]).
pub fn generate_scaffold(config: &ScaffoldConfig) -> Result<Vec<GeneratedFile>, ExtensionError> {
    validate_community_extension_name(&config.name)?;
    // Semver, not the looser `validate_extension_version` that `description.yml`
    // alone would allow: the version is also `[package] version` in Cargo.toml.
    validate_semver(&config.version).map_err(|e| {
        ExtensionError::new(format!(
            "version {:?} is not usable as the Cargo package version: {e}",
            config.version
        ))
    })?;
    validate_spdx_license(&config.license)?;
    validate_free_text("description", &config.description, true)?;
    validate_free_text("maintainer", &config.maintainer, false)?;
    validate_github_repo(&config.github_repo)?;
    validate_git_ref(&config.git_ref)?;

    let excluded: Vec<&str> = config
        .excluded_platforms
        .iter()
        .map(String::as_str)
        .collect();
    crate::validate::validate_excluded_platforms(&excluded)?;

    validate_target_duckdb_version(config)?;

    let files = vec![
        GeneratedFile {
            path: "Cargo.toml".to_string(),
            content: templates::generate_cargo_toml(config),
        },
        GeneratedFile {
            path: "Makefile".to_string(),
            content: templates::generate_makefile(config),
        },
        GeneratedFile {
            path: "src/lib.rs".to_string(),
            content: templates::generate_lib_rs(config),
        },
        GeneratedFile {
            path: "src/wasm_lib.rs".to_string(),
            content: templates::generate_wasm_lib(),
        },
        GeneratedFile {
            path: "description.yml".to_string(),
            content: templates::generate_description_yml(config),
        },
        GeneratedFile {
            path: format!("test/sql/{}.test", config.name),
            content: templates::generate_sqllogictest(config),
        },
        GeneratedFile {
            path: ".github/workflows/extension-ci.yml".to_string(),
            content: templates_ci::generate_extension_ci(config),
        },
        GeneratedFile {
            path: ".github/workflows/MainDistributionPipeline.yml".to_string(),
            content: templates_ci::generate_distribution_workflow(config),
        },
        GeneratedFile {
            path: ".gitmodules".to_string(),
            content: templates::generate_gitmodules(config),
        },
        GeneratedFile {
            path: ".gitignore".to_string(),
            content: templates::generate_gitignore(),
        },
        GeneratedFile {
            path: ".cargo/config.toml".to_string(),
            content: templates::generate_cargo_config(),
        },
    ];

    Ok(files)
}
