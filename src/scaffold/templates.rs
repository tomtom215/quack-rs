// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Template generators for scaffold file content.
//!
//! Each function here produces the string content for one generated file.
//! All functions are `pub(super)` — they are implementation details of
//! [`generate_scaffold`][super::generate_scaffold] and are not part of
//! the public API.

use super::ScaffoldConfig;

pub(super) fn generate_cargo_toml(config: &ScaffoldConfig) -> String {
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
quack-rs = {{ version = "0.3" }}
libduckdb-sys = {{ version = ">=1.4.4, <2", features = ["loadable-extension"] }}

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

pub(super) fn generate_makefile(config: &ScaffoldConfig) -> String {
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
DUCKDB_PLATFORM_VERSION=v1.5.0

# Include extension-ci-tools build rules
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile
",
        name = config.name,
    )
}

pub(super) fn generate_lib_rs(config: &ScaffoldConfig) -> String {
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

pub(super) fn generate_description_yml(config: &ScaffoldConfig) -> String {
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

pub(super) fn generate_gitmodules() -> String {
    "[submodule \"extension-ci-tools\"]\n\tpath = extension-ci-tools\n\turl = https://github.com/duckdb/extension-ci-tools\n".to_string()
}

pub(super) fn generate_gitignore() -> String {
    "/target\n*.duckdb\n*.wal\nbuild/\n.env\n__pycache__/\n".to_string()
}

pub(super) fn generate_cargo_config() -> String {
    "# Statically link the C runtime on Windows MSVC targets.\n\
     # This avoids requiring vcredist on end-user machines.\n\
     [target.x86_64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n\
     \n\
     [target.aarch64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n"
        .to_string()
}

pub(super) fn generate_wasm_lib() -> String {
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
pub(super) fn generate_extension_config_cmake(config: &ScaffoldConfig) -> String {
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
pub(super) fn generate_sqllogictest(config: &ScaffoldConfig) -> String {
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
pub(super) fn generate_extension_ci(config: &ScaffoldConfig) -> String {
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
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20version: v1.5.0\n\
         \n\
         \x20\x20\x20\x20\x20\x20- name: SQLLogicTest (E2E)\n\
         \x20\x20\x20\x20\x20\x20\x20\x20run: make test\n\
         \x20\x20\x20\x20\x20\x20\x20\x20env:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20DUCKDB_PLATFORM: ${{{{ matrix.platform }}}}\n\
         "
    )
}
