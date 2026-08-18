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

/// The `quack-rs` version requirement written into generated `Cargo.toml`
/// files: the major.minor of the crate doing the generating.
fn quack_rs_dependency_version() -> String {
    let full = env!("CARGO_PKG_VERSION");
    let mut parts = full.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        _ => full.to_string(),
    }
}

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
quack-rs = {{ version = "{quack_rs}" }}
libduckdb-sys = {{ version = ">=1.4.4, <2", features = ["loadable-extension"] }}

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
# Keep this as `unwind`. quack-rs's callback wrappers (`scalar_callback!`,
# `table_scan_callback!`) and its extension entry point use `catch_unwind` to
# turn a panic in your code into a DuckDB error. Switching this to abort-on-panic
# makes that machinery inert, so any panic kills the whole DuckDB process —
# including an embedding application and the user's session.
panic = "unwind"
strip = true
"#,
        name = config.name,
        version = config.version,
        quack_rs = quack_rs_dependency_version(),
    )
}

pub(super) fn generate_makefile(config: &ScaffoldConfig) -> String {
    // Mirrors duckdb/extension-template-rs: `EXTENSION_NAME`,
    // `TARGET_DUCKDB_VERSION` and `USE_UNSTABLE_C_API` are the variables
    // `extension-ci-tools/makefiles/c_api_extensions/base.Makefile` actually
    // reads, and the aggregate targets below are what `make` users invoke.
    let unstable = u8::from(config.use_unstable_c_api);
    let abi_note = if config.use_unstable_c_api {
        "# USE_UNSTABLE_C_API=1: the binary is stamped C_STRUCT_UNSTABLE and DuckDB will\n\
         # only load it into exactly TARGET_DUCKDB_VERSION. Required if you enable\n\
         # quack-rs's `duckdb-1-5` / `duckdb-1-5-3` features, because those wrap C API\n\
         # functions whose slot indices move between DuckDB releases."
    } else {
        "# USE_UNSTABLE_C_API=0: the binary is stamped C_STRUCT and loads into any DuckDB\n\
         # whose C extension API version is >= TARGET_DUCKDB_VERSION. Only valid while you\n\
         # stay off quack-rs's `duckdb-1-5` / `duckdb-1-5-3` features; TARGET_DUCKDB_VERSION\n\
         # is then the *C API* version (v1.2.0), not a DuckDB release. See LESSONS.md P2."
    };
    format!(
        r"# DuckDB Rust extension Makefile.
# Delegates to cargo for building and to extension-ci-tools for metadata.

.PHONY: all configure debug release test test_debug test_release clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME={name}
EXT_CONFIG=$(PROJ_DIR)extension_config.cmake

{abi_note}
USE_UNSTABLE_C_API={unstable}
TARGET_DUCKDB_VERSION={target_version}

all: configure release

# Include extension-ci-tools build rules
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: venv platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_release
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean
",
        name = config.name,
        abi_note = abi_note,
        unstable = unstable,
        target_version = config.target_duckdb_version,
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

quack_rs::entry_point!({name}_init_c_api, register);
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
    // `#[path = "lib.rs"]` is required: for the example root `src/wasm_lib.rs`,
    // a bare `mod lib;` resolves to `src/wasm_lib/lib.rs`, which does not exist.
    "// WASM shim: re-exports lib.rs as a staticlib for emscripten compilation.\n\
     // The [[example]] target in Cargo.toml points here with crate-type = [\"staticlib\"].\n\
     // See extension-ci-tools/makefiles/c_api_extensions/rust.Makefile for details.\n\
     #[path = \"lib.rs\"]\n\
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
        r"# GitHub Actions CI for the {name} DuckDB extension.
# Generated by the quack-rs scaffold — customize as needed.
#
# The build goes through extension-ci-tools' Makefiles (see ./Makefile), which
# need the git submodule and a Python 3 virtualenv. `make configure` creates the
# venv, writes configure/platform.txt and resolves the extension version;
# `make release` builds the cdylib and appends the DuckDB extension metadata
# footer; `make test` runs the SQLLogicTests in test/sql against a real DuckDB
# with the extension loaded.

name: Extension CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: Build and test (${{{{ matrix.platform }}}})
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            platform: linux_amd64
          - os: macos-latest
            platform: osx_arm64
          - os: windows-latest
            platform: windows_amd64
    runs-on: ${{{{ matrix.os }}}}
    env:
      DUCKDB_PLATFORM: ${{{{ matrix.platform }}}}
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
        with:
          submodules: recursive

      # dtolnay/rust-toolchain is intentionally ref-pinned (not SHA-pinned)
      # because its SHA changes with each Rust release.
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - uses: Swatinem/rust-cache@82a92a6e8fbeee089604da2575dc567ae9ddeaab # v2.7.5

      - uses: actions/setup-python@42375524e23c412d93fb67b49958b491fce71c38 # v5.4.0
        with:
          python-version: '3.12'

      - name: Lint
        if: matrix.os == 'ubuntu-latest'
        run: |
          cargo fmt -- --check
          cargo clippy --all-targets -- -D warnings

      - name: Unit tests
        run: cargo test

      - name: Configure (venv, platform, version)
        run: make configure

      - name: Build extension with metadata
        run: make release

      - name: SQLLogicTest (end to end)
        run: make test
"
    )
}
