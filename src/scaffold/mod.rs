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
}

/// A generated file with its relative path and content.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    /// Relative path from the project root (e.g., `"src/lib.rs"`).
    pub path: String,
    /// File content as a string.
    pub content: String,
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
