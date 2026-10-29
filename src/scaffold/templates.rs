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
    // A C_STRUCT v1.2.0 build only uses the frozen stable prefix of the C API,
    // so any bindings work. A build that may use the `duckdb-1-5*` features is
    // only correct against the layout it is stamped for, so its bindings are
    // pinned: to the exact release for C_STRUCT_UNSTABLE, to the v1.5.6 ones
    // (the layout DuckDB v1.5.6 and v2.0.0 share) for C_STRUCT v1.5.6.
    let libduckdb_sys = if super::pins_bindings(config) {
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
    } else if super::pins_bindings(config) {
        String::from(
            "# Pinned to the DuckDB v1.5.6 bindings: the binary is stamped C API v1.5.6\n\
             # (TARGET_DUCKDB_VERSION in the Makefile), whose layout DuckDB v1.5.6 and v2.0.0\n\
             # share. Newer bindings would add slots DuckDB v1.5.6 does not have. `make\n\
             # release` refuses a mismatch.\n",
        )
    } else {
        String::new()
    };
    format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"
license = "{license}"
# The oldest Rust this project builds with: quack-rs's own MSRV. (For DuckDB v2.0,
# extension-ci-tools pins a newer one, 1.97.1, on Windows and WebAssembly and uses
# the current stable on Linux and macOS.) Raise it only together with
# the `msrv` job in .github/workflows/extension-ci.yml.
rust-version = "{msrv}"
# Rust-version-aware dependency resolution: `cargo update` and a fresh
# `generate-lockfile` pick the newest versions that still support
# `rust-version`, instead of versions that need a newer compiler.
resolver = "3"

[lib]
name = "{name}"
# cdylib: the .so/.dylib/.dll DuckDB loads. rlib: lets tests/, examples and
# doctests link the crate as a library.
crate-type = ["cdylib", "rlib"]

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
        license = config.license,
        msrv = env!("CARGO_PKG_RUST_VERSION"),
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
         # only load it into exactly TARGET_DUCKDB_VERSION, a DuckDB 1.x release. (DuckDB\n\
         # 2.0 loads C_STRUCT_UNSTABLE only through the C API v2 entry point, which\n\
         # quack-rs does not generate: for 2.0 use USE_UNSTABLE_C_API=0 with\n\
         # TARGET_DUCKDB_VERSION=v1.5.6.)"
    } else if super::pins_bindings(config) {
        "# USE_UNSTABLE_C_API=0 with TARGET_DUCKDB_VERSION=v1.5.6: the binary is stamped\n\
         # C_STRUCT for C API v1.5.6, which DuckDB v1.5.6 and v2.0.0 load and older\n\
         # releases refuse. This is the stamping for quack-rs's `duckdb-1-5` / `-3` / `-4`\n\
         # features: DuckDB declared that whole layout stable from v1.5.6. TARGET_DUCKDB_VERSION\n\
         # is the *C API* version here, not a DuckDB release. See LESSONS.md P2."
    } else {
        "# USE_UNSTABLE_C_API=0: the binary is stamped C_STRUCT and loads into any DuckDB\n\
         # whose C extension API version is >= TARGET_DUCKDB_VERSION (every release from\n\
         # v1.2.0, 2.0 included). Only valid while you stay off quack-rs's `duckdb-1-5` /\n\
         # `-3` / `-4` features (for those, TARGET_DUCKDB_VERSION=v1.5.6);\n\
         # TARGET_DUCKDB_VERSION is the *C API* version (v1.2.0), not a DuckDB release.\n\
         # See LESSONS.md P2."
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
    } else if super::pins_bindings(config) {
        // Tested against whatever DuckDB extension-ci-tools installs (the
        // newest release), which is the point: the binary is meant for v2.0.
        (
            "\n# The bindings are DuckDB v1.5.6's (see Cargo.toml): tells quack-rs's ABI check so.\n\
             export QUACK_RS_TARGET_DUCKDB_VERSION = $(TARGET_DUCKDB_VERSION)\n",
            "check_duckdb_pin ",
            CHECK_DUCKDB_PIN_TARGET,
        )
    } else {
        ("", "", "")
    };
    let ci_tools_ref = super::extension_ci_tools_ref(config);
    format!(
        r#"# DuckDB Rust extension Makefile.
# Delegates to cargo for building and to extension-ci-tools for metadata.

.PHONY: all configure debug release test test_debug test_release clean clean_all{phony_check}

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME={name}

{abi_note}
USE_UNSTABLE_C_API={unstable}
TARGET_DUCKDB_VERSION={target_version}
{declare_block}
# Extension version written into the metadata footer. extension-ci-tools reads
# it from git (the tag on HEAD, else the short commit hash) -- and in a
# repository with no commit yet it records git's error message instead, which
# overflows the footer's 32-byte version field and leaves a file DuckDB refuses
# to load ('Unknown ABI type for extension'). Until HEAD exists, fall back to
# the package version in Cargo.toml. Set EXTENSION_VERSION to override both;
# after the first commit, run `make clean_all configure` to switch to git.
ifeq ($(EXTENSION_VERSION),)
ifeq ($(shell git rev-parse --verify --quiet HEAD 2>/dev/null),)
EXTENSION_VERSION := v$(shell sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
endif
endif

all: configure release

# Include extension-ci-tools build rules. A freshly generated project has a
# .gitmodules but no submodule yet, and `git submodule update --init` does
# nothing until the submodule has been added once (LESSONS.md P4). Use the
# branch the distribution workflow names as `ci_tools_version`.
ifeq ($(wildcard extension-ci-tools/makefiles/c_api_extensions/base.Makefile),)
$(error extension-ci-tools is missing. In a new repository run: git submodule add -b {ci_tools_ref} https://github.com/duckdb/extension-ci-tools.git extension-ci-tools -- in a clone of an existing one: git submodule update --init --recursive)
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
"#,
        name = config.name,
        ci_tools_ref = ci_tools_ref,
        abi_note = abi_note,
        unstable = unstable,
        target_version = config.target_duckdb_version,
        declare_block = declare_block,
        check_prereq = check_prereq,
        check_target = check_target,
        phony_check = if super::pins_bindings(config) {
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

/// The example function: `{name}_hello(name)` greets `name`. `||` returns NULL
/// when `name` is NULL, as SQL functions conventionally do (`concat` would
/// skip the NULL and still return a greeting).
fn hello_macro() -> Result<SqlMacro, ExtensionError> {{
    {hello_call}
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

{entry_point_call}

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
            r#"CREATE OR REPLACE MACRO "{name}_hello"("name") AS ('Hello from {name}! ' || name)"#
        );
        Ok(())
    }}
}}
"##,
        description = doc_comment_lines(&config.description),
        name = config.name,
        hello_call = hello_macro_call(&config.name),
        entry_point_call = entry_point_call(&config.name),
    )
}

/// The `entry_point!` invocation, laid out as `rustfmt` lays it out: on one
/// line while its arguments fit in `fn_call_width` (60 columns), otherwise one
/// argument per line, without a trailing comma.
fn entry_point_call(name: &str) -> String {
    let args = format!("{name}_init_c_api, register");
    if args.len() <= 60 {
        format!("quack_rs::entry_point!({args});")
    } else {
        format!("quack_rs::entry_point!(\n    {name}_init_c_api,\n    register\n);")
    }
}

/// The `SqlMacro::scalar(...)` call in `hello_macro`, laid out the way
/// `rustfmt` lays it out for this name, so the generated project's
/// `cargo fmt --check` passes. `rustfmt` keeps a call's arguments on one line
/// while they fit in `fn_call_width` (60 columns by default); a short name
/// fits, a longer one does not.
fn hello_macro_call(name: &str) -> String {
    let args = [
        format!("\"{name}_hello\""),
        String::from("&[\"name\"]"),
        format!("\"'Hello from {name}! ' || name\""),
    ];
    let one_line = args.join(", ");
    if one_line.len() <= 60 {
        format!("SqlMacro::scalar({one_line})")
    } else {
        format!(
            "SqlMacro::scalar(\n        {},\n    )",
            args.join(",\n        ")
        )
    }
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
        // Quoted, like every text field here: the community build reads this
        // file with PyYAML (YAML 1.1), where an unquoted `yes` or `null` is
        // not a name, a version such as `2025120401` or `1.0` is a number,
        // and a ref such as `0123456` is the octal integer 42798.
        name = yaml_quoted(&config.name),
        description = yaml_quoted(&config.description),
        version = yaml_quoted(&config.version),
        // Normalised to single spaces: the SPDX check splits an expression on
        // any whitespace, so `MIT\nOR Apache-2.0` passes it, but a raw line
        // break here would end the YAML scalar.
        license = yaml_quoted(
            &config
                .license
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );

    if !config.excluded_platforms.is_empty() {
        let platforms = config.excluded_platforms.join(";");
        let _ = writeln!(yml, "  excluded_platforms: \"{platforms}\"");
    }

    let _ = writeln!(yml, "  maintainers:");
    let _ = writeln!(yml, "    - {}", yaml_quoted(&config.maintainer));

    let _ = writeln!(yml);
    let _ = writeln!(yml, "repo:");
    let _ = writeln!(yml, "  github: {}", yaml_quoted(&config.github_repo));
    // DuckDB's documentation: "Provide the hash of the latest commit on the
    // branch targeting stable as `ref`". The community repository builds
    // exactly this revision and signs the result, so a branch name would make
    // the build unreproducible.
    let _ = writeln!(yml, "  # Must be a commit hash, not a branch.");
    let _ = writeln!(yml, "  ref: {}", yaml_quoted(&config.git_ref));
    let _ = writeln!(
        yml,
        "  # ref_next: <hash>   # optional: a revision compatible with DuckDB main,"
    );
    let _ = writeln!(
        yml,
        "  #                    # used while a new DuckDB release is being prepared."
    );
    let _ = writeln!(
        yml,
        "  # variegata: <hash>  # optional: a revision to build for DuckDB v1.5.6 while"
    );
    let _ = writeln!(
        yml,
        "  #                    # `ref` is built for the newest release (v2.0.0)."
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

/// `.gitmodules`, tracking the extension-ci-tools branch the distribution
/// workflow builds with, so `git submodule update --remote` stays on it.
pub(super) fn generate_gitmodules(config: &ScaffoldConfig) -> String {
    format!(
        "[submodule \"extension-ci-tools\"]\n\
         \tpath = extension-ci-tools\n\
         \turl = https://github.com/duckdb/extension-ci-tools\n\
         \tbranch = {}\n",
        super::extension_ci_tools_ref(config)
    )
}

/// `.gitignore`: build output, the `make configure` state (venv, platform and
/// version files) and what `make test` leaves behind, as in
/// `duckdb/extension-template-rs`. `Cargo.lock` is deliberately tracked: the
/// extension is a binary, and its lock file is what makes builds repeatable.
pub(super) fn generate_gitignore() -> String {
    "/target\n\
     /build/\n\
     /configure/\n\
     /duckdb_unittest_tempdir/\n\
     /test/bin/\n\
     venv/\n\
     .idea/\n\
     *.duckdb\n\
     *.wal\n\
     .env\n\
     __pycache__/\n"
        .to_string()
}

pub(super) fn generate_cargo_config() -> String {
    "# Statically link the C runtime on Windows MSVC targets.\n\
     # This avoids requiring vcredist on end-user machines.\n\
     [target.x86_64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n\
     \n\
     [target.aarch64-pc-windows-msvc]\n\
     rustflags = [\"-Ctarget-feature=+crt-static\"]\n\
     \n\
     # The opposite on musl: a `cdylib` cannot be produced with the C runtime linked\n\
     # statically, which is musl's default (\"target does not support these crate\n\
     # types\"). Only needed for the opt-in `linux_*_musl` platforms; extension-ci-tools'\n\
     # Rust makefile passes nothing for them. Checked with `cargo build --release`\n\
     # on x86_64-unknown-linux-musl, not inside the Alpine build container.\n\
     [target.x86_64-unknown-linux-musl]\n\
     rustflags = [\"-Ctarget-feature=-crt-static\"]\n\
     \n\
     [target.aarch64-unknown-linux-musl]\n\
     rustflags = [\"-Ctarget-feature=-crt-static\"]\n"
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
         # NULL in, NULL out.\n\
         query T\n\
         SELECT {name}_hello(NULL);\n\
         ----\n\
         NULL\n\
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
