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
            content: generate_cargo_toml(config),
        },
        GeneratedFile {
            path: "Makefile".to_string(),
            content: generate_makefile(config),
        },
        GeneratedFile {
            path: "extension_config.cmake".to_string(),
            content: generate_extension_config_cmake(config),
        },
        GeneratedFile {
            path: "src/lib.rs".to_string(),
            content: generate_lib_rs(config),
        },
        GeneratedFile {
            path: "src/wasm_lib.rs".to_string(),
            content: generate_wasm_lib(),
        },
        GeneratedFile {
            path: "description.yml".to_string(),
            content: generate_description_yml(config),
        },
        GeneratedFile {
            path: format!("test/sql/{}.test", config.name),
            content: generate_sqllogictest(config),
        },
        GeneratedFile {
            path: ".github/workflows/extension-ci.yml".to_string(),
            content: generate_extension_ci(config),
        },
        GeneratedFile {
            path: ".gitmodules".to_string(),
            content: generate_gitmodules(),
        },
        GeneratedFile {
            path: ".gitignore".to_string(),
            content: generate_gitignore(),
        },
        GeneratedFile {
            path: ".cargo/config.toml".to_string(),
            content: generate_cargo_config(),
        },
    ];

    Ok(files)
}

fn generate_cargo_toml(config: &ScaffoldConfig) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"

[lib]
name = "{name}"
crate-type = ["cdylib"]

# WASM support: staticlib target for emcc compilation.
# See extension-ci-tools for details.
[[example]]
name = "{name}"
crate-type = ["staticlib"]
path = "src/wasm_lib.rs"

[dependencies]
quack-rs = {{ version = "0.2" }}
libduckdb-sys = {{ version = "=1.4.4", features = ["loadable-extension"] }}

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
"#,
        name = config.name,
        version = config.version,
    )
}

fn generate_makefile(config: &ScaffoldConfig) -> String {
    // Matches the structure from duckdb/extension-template-rs
    format!(
        r"# DuckDB Rust Extension Makefile
# Delegates to cargo for building and extension-ci-tools for metadata.

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

# Extension configuration
EXT_NAME={name}
EXT_CONFIG=$(PROJ_DIR)extension_config.cmake

# DuckDB C API version (NOT the DuckDB release version)
# See: https://github.com/tomtom215/quack-rs/blob/main/LESSONS.md (Pitfall P2)
USE_UNSTABLE_C_API=1
DUCKDB_PLATFORM_VERSION=v1.4.4

# Include extension-ci-tools build rules
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile
",
        name = config.name,
    )
}

fn generate_lib_rs(config: &ScaffoldConfig) -> String {
    format!(
        r#"//! {description}
//!
//! A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).

use quack_rs::prelude::*;

// ---------------------------------------------------------------------------
// Example: a simple SQL macro. Replace with your own functions.
// ---------------------------------------------------------------------------

/// Registers all extension functions on the given connection.
fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {{
    // Example: register a scalar SQL macro (no unsafe callbacks needed).
    // Replace this with your own aggregate, scalar, or table functions.
    unsafe {{
        SqlMacro::scalar(
            "{name}_hello",
            &["name"],
            "concat('Hello from {name}! ', name)",
        )?
        .register(con)?;
    }}
    Ok(())
}}

// ---------------------------------------------------------------------------
// Entry point — the C Extension API handles everything, no C++ glue needed.
// ---------------------------------------------------------------------------

quack_rs::entry_point!({name}_init_c_api, |con| register(con));
"#,
        description = config.description,
        name = config.name,
    )
}

fn generate_description_yml(config: &ScaffoldConfig) -> String {
    use std::fmt::Write;

    let mut yml = format!(
        r"extension:
  name: {name}
  description: {description}
  version: {version}
  language: Rust
  build: cargo
  license: {license}
  requires_toolchains: rust;python3
",
        name = config.name,
        description = config.description,
        version = config.version,
        license = config.license,
    );

    if !config.excluded_platforms.is_empty() {
        let platforms = config.excluded_platforms.join(";");
        let _ = writeln!(yml, "  excluded_platforms: \"{platforms}\"");
    }

    let _ = writeln!(yml, "  maintainers:");
    let _ = writeln!(yml, "    - {}", config.maintainer);

    let _ = writeln!(yml);
    let _ = writeln!(yml, "repo:");
    let _ = writeln!(yml, "  github: {}", config.github_repo);
    let _ = writeln!(yml, "  ref: main");

    yml
}

fn generate_gitmodules() -> String {
    "[submodule \"extension-ci-tools\"]\n\tpath = extension-ci-tools\n\turl = https://github.com/duckdb/extension-ci-tools\n".to_string()
}

fn generate_gitignore() -> String {
    "/target\n*.duckdb\n*.wal\nbuild/\n.env\n__pycache__/\n".to_string()
}

fn generate_cargo_config() -> String {
    "# Statically link the C runtime on Windows MSVC targets.\n\
     # This avoids requiring vcredist on end-user machines.\n\
     [target.x86_64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n\
     \n\
     [target.aarch64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n"
        .to_string()
}

fn generate_wasm_lib() -> String {
    "// WASM shim: re-exports lib.rs as a staticlib for emscripten compilation.\n\
     // The [[example]] target in Cargo.toml points here with crate-type = [\"staticlib\"].\n\
     // See extension-ci-tools/makefiles/c_api_extensions/rust.Makefile for details.\n\
     mod lib;\n"
        .to_string()
}

/// Generates `extension_config.cmake`, required by the `EXT_CONFIG` reference in the Makefile.
///
/// This file tells `DuckDB`'s CMake-based build system about the extension. Even though
/// the extension itself is built with `cargo`, `extension-ci-tools` expects this file
/// to exist for metadata and CI integration purposes.
fn generate_extension_config_cmake(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    let github_repo = &config.github_repo;
    format!(
        "# Extension configuration for `DuckDB`'s build system.\n\
         # Required by extension-ci-tools even for pure-Rust (cargo) extensions.\n\
         # See: https://github.com/duckdb/extension-ci-tools\n\
         \n\
         duckdb_extension_load({name}\n\
         \tLOAD_TESTS\n\
         \tGIT_URL https://github.com/{github_repo}\n\
         \tGIT_TAG main\n\
         )\n"
    )
}

/// Generates a `SQLLogicTest` skeleton for `test/sql/{name}.test`.
///
/// `SQLLogicTest` is `DuckDB`'s integration test format. Tests in this file run via
/// `make test` against a real `DuckDB` process with the extension loaded.
///
/// Pitfall P5: Expected values must match `DuckDB`'s exact output format.
/// Generate expected values by running queries in the `DuckDB` CLI and copying the output.
fn generate_sqllogictest(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    format!(
        "# Integration tests for the {name} extension.\n\
         # Run via: make test\n\
         #\n\
         # Format reference: https://duckdb.org/dev/sqllogictest/intro.html\n\
         # - query T = VARCHAR result, query I = INTEGER, query R = REAL, query B = BOOLEAN\n\
         # - Expected output must match DuckDB's exact format (see LESSONS.md Pitfall P5)\n\
         \n\
         # Verify the extension loads without error\n\
         require {name}\n\
         \n\
         # ---- Replace the examples below with your actual function tests ----\n\
         \n\
         # Example: test a scalar function that returns a VARCHAR\n\
         # query T\n\
         # SELECT {name}_hello('world');\n\
         # ----\n\
         # Hello from {name}! world\n\
         \n\
         # Example: test an aggregate function\n\
         # query I\n\
         # SELECT {name}_count(col) FROM (VALUES (1), (2), (3)) t(col);\n\
         # ----\n\
         # 3\n\
         \n\
         # Example: NULL handling\n\
         # query I\n\
         # SELECT {name}_count(col) FROM (VALUES (NULL), (1)) t(col);\n\
         # ----\n\
         # 1\n\
         "
    )
}

/// Generates a GitHub Actions CI workflow for the extension repository.
///
/// This workflow builds and tests the extension on all `DuckDB` community extension
/// platforms using `extension-ci-tools`. It is separate from quack-rs's own CI.
fn generate_extension_ci(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    format!(
        "# GitHub Actions CI for the {name} DuckDB extension.\n\
         # Builds and tests on all community-extension platforms.\n\
         # Generated by quack-rs scaffold — customize as needed.\n\
         \n\
         name: Extension CI\n\
         \n\
         on:\n\
         \x20\x20push:\n\
         \x20\x20\x20\x20branches: [main]\n\
         \x20\x20pull_request:\n\
         \x20\x20\x20\x20branches: [main]\n\
         \n\
         env:\n\
         \x20\x20CARGO_TERM_COLOR: always\n\
         \n\
         jobs:\n\
         \x20\x20build:\n\
         \x20\x20\x20\x20name: Build and Test\n\
         \x20\x20\x20\x20strategy:\n\
         \x20\x20\x20\x20\x20\x20fail-fast: false\n\
         \x20\x20\x20\x20\x20\x20matrix:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20include:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20- os: ubuntu-latest\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20platform: linux_amd64\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20- os: macos-latest\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20platform: osx_arm64\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20- os: windows-latest\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20platform: windows_amd64\n\
         \x20\x20\x20\x20runs-on: ${{{{ matrix.os }}}}\n\
         \x20\x20\x20\x20steps:\n\
         \x20\x20\x20\x20\x20\x20- uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2\n\
         \x20\x20\x20\x20\x20\x20\x20\x20with:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20submodules: recursive\n\
         \n\
         \x20\x20\x20\x20\x20\x20# dtolnay/rust-toolchain is intentionally ref-pinned (not SHA-pinned)\n\
         \x20\x20\x20\x20\x20\x20# because its SHA changes with each Rust release.\n\
         \x20\x20\x20\x20\x20\x20- uses: dtolnay/rust-toolchain@stable\n\
         \n\
         \x20\x20\x20\x20\x20\x20- uses: Swatinem/rust-cache@82a92a6e8fbeee089604da2575dc567ae9ddeaab # v2.7.5\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: Lint\n\
         \x20\x20\x20\x20\x20\x20\x20\x20if: matrix.os == 'ubuntu-latest'\n\
         \x20\x20\x20\x20\x20\x20\x20\x20run: |\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20cargo clippy --all-targets -- -D warnings\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20cargo fmt -- --check\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: Build (release)\n\
         \x20\x20\x20\x20\x20\x20\x20\x20run: cargo build --release\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: Unit tests\n\
         \x20\x20\x20\x20\x20\x20\x20\x20run: cargo test\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: Install DuckDB CLI\n\
         \x20\x20\x20\x20\x20\x20\x20\x20uses: duckdb/duckdb-build@v1\n\
         \x20\x20\x20\x20\x20\x20\x20\x20with:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20version: v1.4.4\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: SQLLogicTest (E2E)\n\
         \x20\x20\x20\x20\x20\x20\x20\x20run: make test\n\
         \x20\x20\x20\x20\x20\x20\x20\x20env:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20DUCKDB_PLATFORM: ${{{{ matrix.platform }}}}\n\
         "
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> ScaffoldConfig {
        ScaffoldConfig {
            name: "my_analytics".to_string(),
            description: "Fast analytics functions".to_string(),
            version: "0.1.0".to_string(),
            license: "MIT".to_string(),
            maintainer: "Jane Doe".to_string(),
            github_repo: "janedoe/duckdb-my-analytics".to_string(),
            excluded_platforms: vec![],
        }
    }

    #[test]
    fn generates_all_required_files() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"Makefile"));
        assert!(paths.contains(&"extension_config.cmake"));
        assert!(paths.contains(&"src/lib.rs"));
        assert!(paths.contains(&"src/wasm_lib.rs"));
        assert!(paths.contains(&"description.yml"));
        assert!(paths.contains(&"test/sql/my_analytics.test"));
        assert!(paths.contains(&".github/workflows/extension-ci.yml"));
        assert!(paths.contains(&".gitmodules"));
        assert!(paths.contains(&".gitignore"));
        assert!(paths.contains(&".cargo/config.toml"));
    }

    #[test]
    fn cargo_toml_has_correct_name() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("name = \"my_analytics\""));
    }

    #[test]
    fn cargo_toml_has_cdylib() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("crate-type = [\"cdylib\"]"));
    }

    #[test]
    fn cargo_toml_has_release_profile() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("panic = \"abort\""));
        assert!(cargo.content.contains("lto = true"));
        assert!(cargo.content.contains("strip = true"));
    }

    #[test]
    fn makefile_has_extension_name() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let makefile = files.iter().find(|f| f.path == "Makefile").unwrap();
        assert!(makefile.content.contains("EXT_NAME=my_analytics"));
    }

    #[test]
    fn makefile_includes_rust_build_rules() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let makefile = files.iter().find(|f| f.path == "Makefile").unwrap();
        assert!(makefile.content.contains("rust.Makefile"));
    }

    #[test]
    fn lib_rs_has_entry_point() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        assert!(lib.content.contains("entry_point!"));
    }

    #[test]
    fn lib_rs_uses_quack_rs_api() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        assert!(lib.content.contains("quack_rs::prelude"));
        // Must not use the duckdb crate VTab API
        assert!(!lib.content.contains("use duckdb::"));
        // Must not contain .expect() or .unwrap() (no panics in FFI paths)
        assert!(!lib.content.contains(".expect("));
        assert!(!lib.content.contains(".unwrap()"));
    }

    #[test]
    fn lib_rs_no_cpp_glue() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let lib = files.iter().find(|f| f.path == "src/lib.rs").unwrap();
        // Must not contain any C++ references
        assert!(!lib.content.contains("CMake"));
    }

    #[test]
    fn description_yml_has_fields() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
        assert!(desc.content.contains("name: my_analytics"));
        assert!(desc.content.contains("license: MIT"));
        assert!(desc.content.contains("janedoe/duckdb-my-analytics"));
    }

    #[test]
    fn description_yml_uses_rust_language() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
        assert!(desc.content.contains("language: Rust"));
        assert!(desc.content.contains("build: cargo"));
        assert!(desc.content.contains("requires_toolchains: rust;python3"));
    }

    #[test]
    fn invalid_name_rejected() {
        let mut config = valid_config();
        config.name = "Invalid Name".to_string();
        assert!(generate_scaffold(&config).is_err());
    }

    #[test]
    fn invalid_license_rejected() {
        let mut config = valid_config();
        config.license = "FAKE-LICENSE".to_string();
        assert!(generate_scaffold(&config).is_err());
    }

    #[test]
    fn invalid_version_rejected() {
        let mut config = valid_config();
        config.version = "not-a-version".to_string();
        assert!(generate_scaffold(&config).is_err());
    }

    #[test]
    fn invalid_platform_rejected() {
        let mut config = valid_config();
        config.excluded_platforms = vec!["invalid_platform".to_string()];
        assert!(generate_scaffold(&config).is_err());
    }

    #[test]
    fn excluded_platforms_in_description() {
        let mut config = valid_config();
        config.excluded_platforms = vec!["wasm_mvp".to_string(), "wasm_eh".to_string()];
        let files = generate_scaffold(&config).unwrap();
        let desc = files.iter().find(|f| f.path == "description.yml").unwrap();
        // Platforms are semicolon-separated per DuckDB convention
        assert!(desc
            .content
            .contains("excluded_platforms: \"wasm_mvp;wasm_eh\""));
    }

    #[test]
    fn gitmodules_references_ci_tools() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let gitmod = files.iter().find(|f| f.path == ".gitmodules").unwrap();
        assert!(gitmod
            .content
            .contains("https://github.com/duckdb/extension-ci-tools"));
    }

    #[test]
    fn unstable_version_accepted() {
        let mut config = valid_config();
        config.version = "690bfc5".to_string();
        assert!(generate_scaffold(&config).is_ok());
    }

    #[test]
    fn wasm_staticlib_example_present() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("staticlib"));
        assert!(cargo.content.contains("wasm_lib.rs"));
    }

    #[test]
    fn cargo_config_has_windows_crt_static() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cfg = files
            .iter()
            .find(|f| f.path == ".cargo/config.toml")
            .unwrap();
        assert!(cfg.content.contains("crt-static"));
        assert!(cfg.content.contains("x86_64-pc-windows-msvc"));
        assert!(cfg.content.contains("aarch64-pc-windows-msvc"));
    }

    #[test]
    fn wasm_lib_shim_exists() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let wasm = files.iter().find(|f| f.path == "src/wasm_lib.rs").unwrap();
        assert!(wasm.content.contains("mod lib"));
    }

    // --- extension_config.cmake ---

    #[test]
    fn extension_config_cmake_exists() {
        let files = generate_scaffold(&valid_config()).unwrap();
        assert!(files.iter().any(|f| f.path == "extension_config.cmake"));
    }

    #[test]
    fn extension_config_cmake_references_name() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cmake = files
            .iter()
            .find(|f| f.path == "extension_config.cmake")
            .unwrap();
        assert!(cmake.content.contains("duckdb_extension_load(my_analytics"));
    }

    #[test]
    fn extension_config_cmake_references_github_repo() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cmake = files
            .iter()
            .find(|f| f.path == "extension_config.cmake")
            .unwrap();
        assert!(cmake.content.contains("janedoe/duckdb-my-analytics"));
    }

    #[test]
    fn extension_config_cmake_has_load_tests() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let cmake = files
            .iter()
            .find(|f| f.path == "extension_config.cmake")
            .unwrap();
        assert!(cmake.content.contains("LOAD_TESTS"));
    }

    // --- SQLLogicTest ---

    #[test]
    fn sqllogictest_file_exists() {
        let files = generate_scaffold(&valid_config()).unwrap();
        assert!(files.iter().any(|f| f.path == "test/sql/my_analytics.test"));
    }

    #[test]
    fn sqllogictest_has_require_directive() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let test = files
            .iter()
            .find(|f| f.path == "test/sql/my_analytics.test")
            .unwrap();
        assert!(test.content.contains("require my_analytics"));
    }

    #[test]
    fn sqllogictest_name_matches_extension() {
        let mut config = valid_config();
        config.name = "custom_ext".to_string();
        let files = generate_scaffold(&config).unwrap();
        let test_path = "test/sql/custom_ext.test";
        assert!(files.iter().any(|f| f.path == test_path));
        let test = files.iter().find(|f| f.path == test_path).unwrap();
        assert!(test.content.contains("require custom_ext"));
    }

    // --- GitHub Actions CI ---

    #[test]
    fn extension_ci_yml_exists() {
        let files = generate_scaffold(&valid_config()).unwrap();
        assert!(files
            .iter()
            .any(|f| f.path == ".github/workflows/extension-ci.yml"));
    }

    #[test]
    fn extension_ci_yml_has_linux_matrix() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let ci = files
            .iter()
            .find(|f| f.path == ".github/workflows/extension-ci.yml")
            .unwrap();
        assert!(ci.content.contains("ubuntu-latest"));
        assert!(ci.content.contains("linux_amd64"));
    }

    #[test]
    fn extension_ci_yml_has_macos_matrix() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let ci = files
            .iter()
            .find(|f| f.path == ".github/workflows/extension-ci.yml")
            .unwrap();
        assert!(ci.content.contains("macos-latest"));
        assert!(ci.content.contains("osx_arm64"));
    }

    #[test]
    fn extension_ci_yml_has_windows_matrix() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let ci = files
            .iter()
            .find(|f| f.path == ".github/workflows/extension-ci.yml")
            .unwrap();
        assert!(ci.content.contains("windows-latest"));
        assert!(ci.content.contains("windows_amd64"));
    }

    #[test]
    fn extension_ci_yml_runs_sqllogictest() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let ci = files
            .iter()
            .find(|f| f.path == ".github/workflows/extension-ci.yml")
            .unwrap();
        assert!(ci.content.contains("make test"));
    }

    #[test]
    fn extension_ci_yml_checks_out_submodules() {
        let files = generate_scaffold(&valid_config()).unwrap();
        let ci = files
            .iter()
            .find(|f| f.path == ".github/workflows/extension-ci.yml")
            .unwrap();
        assert!(ci.content.contains("submodules: recursive"));
    }
}
