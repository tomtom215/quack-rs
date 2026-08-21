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

mod templates;

#[cfg(test)]
mod tests;

use crate::error::ExtensionError;
use crate::validate::{validate_extension_name, validate_spdx_license};

/// Configuration for generating a new extension project.
#[derive(Debug, Clone)]
pub struct ScaffoldConfig {
    /// Extension name (must pass [`validate_extension_name`]).
    pub name: String,
    /// One-line description of the extension.
    pub description: String,
    /// Initial version (semver, e.g., `"0.1.0"`).
    pub version: String,
    /// SPDX license identifier (must pass [`validate_spdx_license`]).
    pub license: String,
    /// Primary maintainer name.
    pub maintainer: String,
    /// GitHub repository path (e.g., `"myorg/duckdb-my-ext"`).
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
    ///   other release.
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
    /// make the build unreproducible. Of 43 published extensions sampled, 41
    /// pin a full 40-character hash and the remaining two pin a tag; none uses
    /// a branch.
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

    Ok(())
}

/// Generates the complete set of project files for a new `DuckDB` Rust extension.
///
/// Validates the configuration and returns a list of [`GeneratedFile`]s that can be
/// written to disk. Does NOT write files — callers decide how to persist them.
///
/// # Errors
///
/// Returns [`ExtensionError`] if the extension name, license, or version is invalid.
pub fn generate_scaffold(config: &ScaffoldConfig) -> Result<Vec<GeneratedFile>, ExtensionError> {
    validate_extension_name(&config.name)?;
    crate::validate::validate_extension_version(&config.version)?;
    validate_spdx_license(&config.license)?;

    for platform in &config.excluded_platforms {
        crate::validate::validate_platform(platform)?;
    }

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
