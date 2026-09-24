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

use super::escape::{doc_comment_lines, yaml_quoted};
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
    // A C_STRUCT build only uses the frozen stable prefix of the C API, so any
    // 1.x bindings work. A C_STRUCT_UNSTABLE build is only correct against the
    // exact release it declares, so its bindings are pinned to that release.
    let libduckdb_sys = if config.use_unstable_c_api {
        super::libduckdb_sys_requirement(&config.target_duckdb_version)
            .unwrap_or_else(|_| String::from(">=1.4.4, <2"))
    } else {
        String::from(">=1.4.4, <2")
    };
    let libduckdb_sys_note = if config.use_unstable_c_api {
        format!(
            "# Pinned to the bindings for DuckDB {target} (TARGET_DUCKDB_VERSION in the\n\
             # Makefile): a C_STRUCT_UNSTABLE binary must be compiled against exactly the\n\
             # release it is stamped for. Change both together; `make release` refuses a\n\
             # mismatch.\n",
            target = config.target_duckdb_version
        )
    } else {
        String::new()
    };
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
{libduckdb_sys_note}libduckdb-sys = {{ version = "{libduckdb_sys}", features = ["loadable-extension"] }}

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
        libduckdb_sys = libduckdb_sys,
        libduckdb_sys_note = libduckdb_sys_note,
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
         # quack-rs's `duckdb-1-5` / `-3` / `-4` features, because those wrap C API\n\
         # functions whose slot indices move between DuckDB releases."
    } else {
        "# USE_UNSTABLE_C_API=0: the binary is stamped C_STRUCT and loads into any DuckDB\n\
         # whose C extension API version is >= TARGET_DUCKDB_VERSION. Only valid while you\n\
         # stay off quack-rs's `duckdb-1-5` / `-3` / `-4` features; TARGET_DUCKDB_VERSION\n\
         # is then the *C API* version (v1.2.0), not a DuckDB release. See LESSONS.md P2."
    };
    // Unstable builds only: tell quack-rs's ABI check which release the bindings
    // were built against (so it accepts a release its layout table predates,
    // which is what happens every time DuckDB ships and the community
    // repository rebuilds this extension from unchanged source), test against
    // that same release, and refuse to build when the resolved libduckdb-sys
    // is for a different one. Without that last guard, bumping
    // TARGET_DUCKDB_VERSION alone makes the extension *declare* a release its
    // bindings do not match — and for a release quack-rs has no layout entry
    // for, the declaration is trusted.
    let (declare_block, check_prereq, check_target) = if config.use_unstable_c_api {
        (
            "\n# pip version of DuckDB that `make test` runs against: the pinned release.\n\
             DUCKDB_TEST_VERSION=$(patsubst v%,%,$(TARGET_DUCKDB_VERSION))\n\
             \n\
             # Lets quack-rs's ABI check accept a DuckDB release newer than its layout table.\n\
             export QUACK_RS_TARGET_DUCKDB_VERSION = $(TARGET_DUCKDB_VERSION)\n",
            "check_duckdb_pin ",
            CHECK_DUCKDB_PIN_TARGET,
        )
    } else {
        ("", "", "")
    };
    format!(
        r"# DuckDB Rust extension Makefile.
# Delegates to cargo for building and to extension-ci-tools for metadata.

.PHONY: all configure debug release test test_debug test_release clean clean_all{phony_check}

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME={name}
EXT_CONFIG=$(PROJ_DIR)extension_config.cmake

{abi_note}
USE_UNSTABLE_C_API={unstable}
TARGET_DUCKDB_VERSION={target_version}
{declare_block}
all: configure release

# Include extension-ci-tools build rules. A freshly generated project has a
# .gitmodules but no submodule yet, and `git submodule update --init` does
# nothing until the submodule has been added once (LESSONS.md P4).
ifeq ($(wildcard extension-ci-tools/makefiles/c_api_extensions/base.Makefile),)
$(error extension-ci-tools is missing. In a new repository run: git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools -- in a clone of an existing one: git submodule update --init --recursive)
endif
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: venv platform extension_version

debug: {check_prereq}build_extension_library_debug build_extension_with_metadata_debug
release: {check_prereq}build_extension_library_release build_extension_with_metadata_release
{check_target}
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
        declare_block = declare_block,
        check_prereq = check_prereq,
        check_target = check_target,
        phony_check = if config.use_unstable_c_api {
            " check_duckdb_pin"
        } else {
            ""
        },
    )
}

/// The `check_duckdb_pin` rule of an unstable-ABI `Makefile`.
///
/// Asks cargo which `libduckdb-sys` the build resolves to, decodes the
/// `DuckDB` release it ships (the same mapping as quack-rs's
/// `scripts/duckdb-version-from-lock.sh`) and fails unless it is
/// `TARGET_DUCKDB_VERSION`. A plain string so the recipe's tab and `$$` reach
/// the file exactly.
const CHECK_DUCKDB_PIN_TARGET: &str = "
# The bindings compiled in come from the libduckdb-sys pin in Cargo.toml, not
# from TARGET_DUCKDB_VERSION. Refuse to build when they name different releases.
check_duckdb_pin:
\t@v=$$(cargo tree -e normal -i libduckdb-sys --depth 0 --prefix none | sed -n 's/^libduckdb-sys v//p' | head -n 1); \\
\tm=$$(echo \"$$v\" | cut -d. -f2); \\
\tif [ \"$${m:-0}\" -ge 10000 ] 2>/dev/null; then got=\"v$$((m / 10000)).$$((m / 100 % 100)).$$((m % 100))\"; else got=\"v$$v\"; fi; \\
\tif [ \"$$got\" != \"$(TARGET_DUCKDB_VERSION)\" ]; then \\
\t\techo \"error: libduckdb-sys $$v is DuckDB $$got, but TARGET_DUCKDB_VERSION=$(TARGET_DUCKDB_VERSION); update the libduckdb-sys pin in Cargo.toml\" >&2; \\
\t\texit 1; \\
\tfi
";

pub(super) fn generate_lib_rs(config: &ScaffoldConfig) -> String {
    format!(
        r##"{description}
//!
//! A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).

use quack_rs::prelude::*;

// ---------------------------------------------------------------------------
// Example: a simple SQL macro. Replace with your own functions.
// ---------------------------------------------------------------------------

/// The example function: `{name}_hello(name)` greets `name`.
fn hello_macro() -> Result<SqlMacro, ExtensionError> {{
    SqlMacro::scalar(
        "{name}_hello",
        &["name"],
        "concat('Hello from {name}! ', name)",
    )
}}

/// Registers all extension functions on the given connection.
fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {{
    // Example: register a scalar SQL macro (no unsafe callbacks needed).
    // Replace this with your own aggregate, scalar, or table functions.
    //
    // SAFETY: `con` is the connection quack-rs opened for this entry point and
    // is valid for the duration of this function.
    unsafe {{
        hello_macro()?.register(con)?;
    }}
    Ok(())
}}

// ---------------------------------------------------------------------------
// Entry point — the C Extension API handles everything, no C++ glue needed.
// ---------------------------------------------------------------------------

quack_rs::entry_point!({name}_init_c_api, register);

// Unit tests run under plain `cargo test`: building a function definition
// needs no DuckDB. Calling into DuckDB does, so behaviour against a real engine
// is tested in test/sql/{name}.test (SQLLogicTest, run by `make test`).
#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn hello_macro_renders_the_expected_sql() -> Result<(), ExtensionError> {{
        assert_eq!(
            hello_macro()?.to_sql(),
            r#"CREATE OR REPLACE MACRO "{name}_hello"("name") AS (concat('Hello from {name}! ', name))"#
        );
        Ok(())
    }}
}}
"##,
        description = doc_comment_lines(&config.description),
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
        // Quoted: `description` is free text, and a version such as
        // `2025120401` or `1.0` would otherwise be read as a number.
        description = yaml_quoted(&config.description),
        version = yaml_quoted(&config.version),
        license = config.license,
    );

    if !config.excluded_platforms.is_empty() {
        let platforms = config.excluded_platforms.join(";");
        let _ = writeln!(yml, "  excluded_platforms: \"{platforms}\"");
    }

    let _ = writeln!(yml, "  maintainers:");
    let _ = writeln!(yml, "    - {}", yaml_quoted(&config.maintainer));

    let _ = writeln!(yml);
    let _ = writeln!(yml, "repo:");
    let _ = writeln!(yml, "  github: {}", config.github_repo);
    // DuckDB's documentation: "Provide the hash of the latest commit on the
    // branch targeting stable as `ref`". The community repository builds
    // exactly this revision and signs the result, so a branch name would make
    // the build unreproducible.
    let _ = writeln!(yml, "  # Must be a commit hash, not a branch.");
    let _ = writeln!(yml, "  ref: {}", config.git_ref);
    let _ = writeln!(
        yml,
        "  # ref_next: <hash>   # optional: a revision compatible with DuckDB main,"
    );
    let _ = writeln!(
        yml,
        "  #                    # used while a new DuckDB release is being prepared."
    );

    // 332 of the 346 published descriptors (community-extensions `5ae7df8`)
    // have a `docs:` section; it is what renders on the community-extensions
    // documentation site.
    let _ = writeln!(yml);
    let _ = writeln!(yml, "docs:");
    let _ = writeln!(yml, "  hello_world: |");
    // Must call something `generate_lib_rs` registers: this is the example the
    // community-extensions site shows users to copy.
    let _ = writeln!(yml, "    SELECT {}_hello('world');", config.name);
    let _ = writeln!(
        yml,
        "  extended_description: {}",
        yaml_quoted(&config.description)
    );

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
         # The example function registered in src/lib.rs.\n\
         query T\n\
         SELECT {name}_hello('world');\n\
         ----\n\
         Hello from {name}! world\n\
         \n\
         # One row per input row.\n\
         query T\n\
         SELECT {name}_hello(x) FROM (VALUES ('a'), ('b')) t(x) ORDER BY x;\n\
         ----\n\
         Hello from {name}! a\n\
         Hello from {name}! b\n\
         \n\
         # ---- Add a test like these for each function you add. ----\n\
         # For example, an aggregate you register as {name}_count:\n\
         # query I\n\
         # SELECT {name}_count(col) FROM (VALUES (1), (2), (3)) t(col);\n\
         # ----\n\
         # 3\n\
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
      # Every action is SHA-pinned. A tag or branch is a moving target that the
      # action's owner can repoint at any time, and a workflow step runs
      # arbitrary code in your CI.
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
        with:
          submodules: recursive

      # Pinned to a commit on the action's `master` branch (2026-09-03), whose
      # `toolchain:` input is required. Pinning the action does not pin your
      # Rust version: `toolchain: stable` installs the current stable release
      # on every run.
      - uses: dtolnay/rust-toolchain@d1031067263f94b142dd6c0ce24c5eb9d02d52a0 # master 2026-09-03
        with:
          toolchain: stable
          components: clippy, rustfmt

      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1

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
