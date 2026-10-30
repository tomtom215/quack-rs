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
//! extension-ci-tools distribution build for every community platform), a
//! `README.md`, the license text, a `cargo-deny` configuration, the
//! `docs/function_descriptions.csv` the community documentation site reads,
//! and the supporting dotfiles. The `quack_scaffold` binary
//! (`cargo install quack-rs --bin quack_scaffold`) writes them to disk.
//!
//! # Example
//!
//! ```rust
//! use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold_with_warnings};
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
//! let scaffold = generate_scaffold_with_warnings(&config).unwrap();
//! let files = &scaffold.files;
//! assert!(files.iter().any(|f| f.path == "Cargo.toml"));
//! assert!(files.iter().any(|f| f.path == "Makefile"));
//! assert!(files.iter().any(|f| f.path == "src/lib.rs"));
//! assert!(files.iter().any(|f| f.path == "description.yml"));
//! assert!(scaffold.warnings.is_empty());
//! ```

mod checks;
mod escape;
mod licenses;
mod templates;
mod templates_ci;
mod templates_docs;
mod templates_make;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_content;
#[cfg(test)]
mod tests_escaping;
#[cfg(test)]
mod tests_generated;
// Runs `make`, `git` and a shell script: Unix only. Two attributes rather than
// `cfg(all(test, unix))`: cargo-mutants skips a module only when it sees a plain
// `#[cfg(test)]`, and otherwise mutates this file's helpers.
#[cfg(test)]
#[cfg(unix)]
mod tests_make;
#[cfg(test)]
mod tests_workflows;

use crate::error::ExtensionError;
#[cfg(test)]
use checks::libduckdb_sys_requirement;

/// Configuration for generating a new extension project.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Extension name (must pass
    /// [`validate_community_extension_name`][crate::validate::validate_community_extension_name]:
    /// well formed, and not a `DuckDB` alias such as `s3` or a core extension
    /// name such as `json`).
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
    /// `description.yml`. Must pass [`validate_semver`][crate::validate::validate_semver],
    /// with every numeric part at most `u64::MAX` (what Cargo can read).
    pub version: String,
    /// The license: an SPDX identifier or expression such as `MIT OR
    /// Apache-2.0`. One that [`validate_spdx_license`][crate::validate::validate_spdx_license]
    /// accepts is used as is; a well-formed one naming an identifier outside
    /// its shortlist (`BUSL-1.1`, `CC0-1.0`) is accepted with a warning, since
    /// the shortlist is not the SPDX registry. Line breaks and other
    /// whitespace are written as single spaces.
    ///
    /// The scaffold writes the license text for `MIT`, `Apache-2.0`,
    /// `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `0BSD`, `MIT-0` and `Unlicense`
    /// (`LICENSE`, or `LICENSE-<ID>` for each in an expression), and a
    /// `LICENSE.TODO` naming the text to add for anything else.
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
    /// make the build unreproducible. Of the 395 `description.yml` files in
    /// `duckdb/community-extensions` at commit `845779e`, 384 pin a full
    /// 40-character hash and 11 a tag; none uses a branch. Letters, digits,
    /// `-`, `_`, `.` and `/` only.
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
/// community build green. (`windows_amd64_mingw` in particular: on
/// `v2.0-cyanoptera`, extension-ci-tools' `rust.Makefile` passes Cargo no
/// `--target` for it, so the build would produce the runner's MSVC-ABI DLL and
/// stamp it `windows_amd64_mingw`.) To opt a platform in, drop it from
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

/// What [`generate_scaffold_with_warnings`] produces: the files to write, and
/// what about the configuration deserves a second look although it was
/// accepted.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Scaffold {
    /// The generated files, paths relative to the project root.
    pub files: Vec<GeneratedFile>,
    /// One sentence each: a license identifier outside quack-rs's shortlist,
    /// an exclusion that excludes nothing or everything, an unstable-ABI
    /// project the community build cannot ship as generated.
    pub warnings: Vec<String>,
}

/// Generates the complete set of project files for a new `DuckDB` Rust extension.
///
/// Validates the configuration and returns a list of [`GeneratedFile`]s that can be
/// written to disk. Does NOT write files — callers decide how to persist them.
/// Warnings about values that were accepted are dropped; use
/// [`generate_scaffold_with_warnings`] to see them.
///
/// # Errors
///
/// As [`generate_scaffold_with_warnings`].
pub fn generate_scaffold(config: &ScaffoldConfig) -> Result<Vec<GeneratedFile>, ExtensionError> {
    generate_scaffold_with_warnings(config).map(|scaffold| scaffold.files)
}

/// Generates the project files, and the warnings about the configuration.
///
/// # Errors
///
/// Returns [`ExtensionError`] if the extension name, license, version (which
/// must be semver, as it is also the Cargo package version),
/// description, maintainer, GitHub repository, git ref, excluded platforms or
/// target `DuckDB` version is invalid — including a hyphenated name, which
/// `DuckDB` could never load, a name `DuckDB` itself uses (see
/// [`validate_community_extension_name`]), a license that is not an SPDX
/// expression (one that is, but names an identifier outside
/// [`COMMON_SPDX_LICENSES`][crate::validate::spdx::COMMON_SPDX_LICENSES], is a
/// warning), a version component Cargo cannot read, and an excluded platform
/// the community build does not build (a group such as `wasm` included).
///
/// [`validate_community_extension_name`]: crate::validate::validate_community_extension_name
pub fn generate_scaffold_with_warnings(
    config: &ScaffoldConfig,
) -> Result<Scaffold, ExtensionError> {
    let mut warnings = Vec::new();
    checks::check_config(config, &mut warnings)?;

    let mut files = vec![
        GeneratedFile {
            path: "Cargo.toml".to_string(),
            content: templates::generate_cargo_toml(config),
        },
        GeneratedFile {
            path: "Makefile".to_string(),
            content: templates_make::generate_makefile(config),
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
        GeneratedFile {
            path: "README.md".to_string(),
            content: templates_docs::generate_readme(config),
        },
        GeneratedFile {
            path: "docs/function_descriptions.csv".to_string(),
            content: templates_docs::generate_function_descriptions(config),
        },
        GeneratedFile {
            path: "deny.toml".to_string(),
            content: templates_docs::generate_deny_toml(config),
        },
    ];
    files.extend(licenses::license_files(config));

    Ok(Scaffold { files, warnings })
}
