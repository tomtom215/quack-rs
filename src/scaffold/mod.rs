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
//! - `Cargo.toml` (cdylib, pinned `duckdb` + `libduckdb-sys` deps)
//! - `Makefile` (delegates to `cargo build` + metadata scripts from `extension-ci-tools`)
//! - `src/lib.rs` (entry point + function registration)
//! - `extension-ci-tools/` (git submodule for CI/CD)
//! - `description.yml` (extension metadata for community submission)
//!
//! This module generates all of these from a [`ScaffoldConfig`].
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
//!     excluded_platforms: vec![],
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

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_escaping;
#[cfg(test)]
mod tests_generated;

use crate::error::ExtensionError;
use crate::validate::{validate_extension_name, validate_spdx_license};

/// Configuration for generating a new extension project.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Extension name (must pass [`validate_extension_name`]).
    pub name: String,
    /// Short description of the extension.
    ///
    /// Free text: YAML punctuation such as `: ` or ` #`, quotes, backslashes
    /// and non-ASCII text are quoted and escaped in `description.yml`, and each
    /// line becomes a `//!` line in `src/lib.rs`. It must not be empty, start
    /// or end with whitespace, or contain control characters other than
    /// newline and tab (or Unicode bidirectional controls).
    pub description: String,
    /// Initial version (semver, e.g., `"0.1.0"`).
    pub version: String,
    /// SPDX license identifier (must pass [`validate_spdx_license`]).
    pub license: String,
    /// Primary maintainer name: a single non-empty line, quoted and escaped in
    /// `description.yml` like [`description`][Self::description].
    pub maintainer: String,
    /// GitHub repository path (e.g., `"myorg/duckdb-my-ext"`): `owner/repo`,
    /// in the characters GitHub allows.
    pub github_repo: String,
    /// Platforms to exclude from CI builds (e.g., `["wasm_mvp", "wasm_eh"]`).
    pub excluded_platforms: Vec<String>,
    /// Value written as `TARGET_DUCKDB_VERSION` in the generated `Makefile`.
    ///
    /// `extension-ci-tools` passes this to `append_extension_metadata.py -dv`,
    /// and its meaning depends on [`use_unstable_c_api`][Self::use_unstable_c_api]:
    ///
    /// - `false` → the **C extension API** version, i.e.
    ///   [`DUCKDB_API_VERSION`][crate::DUCKDB_API_VERSION] (`"v1.2.0"`). The
    ///   extension is stamped `C_STRUCT` and loads into any `DuckDB` whose C API
    ///   version is at least this.
    /// - `true` → an exact **`DuckDB` release**, e.g. `"v1.5.5"`. The extension
    ///   is stamped `C_STRUCT_UNSTABLE` and `DuckDB` refuses to load it into any
    ///   other release. The generated `Cargo.toml` then pins `libduckdb-sys`
    ///   to that release's bindings (`~1.10505.0` for v1.5.5), the `Makefile`
    ///   tests against it (`DUCKDB_TEST_VERSION`) and refuses to build when the
    ///   resolved bindings and `TARGET_DUCKDB_VERSION` disagree.
    ///
    /// Defaults to [`DUCKDB_API_VERSION`][crate::DUCKDB_API_VERSION].
    pub target_duckdb_version: String,
    /// Whether the generated `Makefile` sets `USE_UNSTABLE_C_API=1`.
    ///
    /// Set this when the extension enables quack-rs's `duckdb-1-5`,
    /// `duckdb-1-5-3` or `duckdb-1-5-4` features. Those wrap C API functions
    /// that live past the
    /// stable prefix of `duckdb_ext_api_v1`, where `DuckDB` inserts new entries
    /// between releases — so the binary must be pinned to one release. See
    /// [`crate::abi`].
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

impl Default for ScaffoldConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            version: String::from("0.1.0"),
            license: String::from("MIT"),
            maintainer: String::new(),
            github_repo: String::new(),
            excluded_platforms: Vec::new(),
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

    if !config.use_unstable_c_api && version != crate::DUCKDB_API_VERSION {
        return Err(ExtensionError::new(format!(
            "with use_unstable_c_api = false the extension is stamped C_STRUCT, so \
             target_duckdb_version is the C extension API version and must be {:?}; got \
             {version:?}. Set use_unstable_c_api = true to pin the binary to DuckDB \
             {version} instead.",
            crate::DUCKDB_API_VERSION
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

    if config.use_unstable_c_api {
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
/// `description.yml` and into a URL in `extension_config.cmake`.
fn validate_github_repo(repo: &str) -> Result<(), ExtensionError> {
    let valid = repo.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty()
            && !owner.starts_with('-')
            && owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !name.is_empty()
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    });
    if valid {
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
/// `DuckDB` release `target` (`vX.Y.Z`), for a `C_STRUCT_UNSTABLE` build.
///
/// `libduckdb-sys` numbers its releases after the `DuckDB` release whose
/// headers it ships. From `DuckDB` 1.5.0 the scheme is
/// `1.(10000 + 100·minor + patch).N` — 1.10505.0 is `DuckDB` v1.5.5 — where `N`
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
/// Returns [`ExtensionError`] if the extension name, license, version,
/// description, maintainer, GitHub repository, git ref, excluded platforms or
/// target `DuckDB` version is invalid — including a hyphenated name, which
/// `DuckDB` could never load (see [`validate_extension_name`]).
pub fn generate_scaffold(config: &ScaffoldConfig) -> Result<Vec<GeneratedFile>, ExtensionError> {
    validate_extension_name(&config.name)?;
    crate::validate::validate_extension_version(&config.version)?;
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
            path: "extension_config.cmake".to_string(),
            content: templates::generate_extension_config_cmake(config),
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
            content: templates::generate_extension_ci(config),
        },
        GeneratedFile {
            path: ".gitmodules".to_string(),
            content: templates::generate_gitmodules(),
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
