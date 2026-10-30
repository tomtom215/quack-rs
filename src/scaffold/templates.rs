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

/// The `libduckdb-sys` requirement of a stable-ABI (`C_STRUCT` v1.2.0) project:
/// every bindings release quack-rs supports, from the 1.4.4 floor up to and
/// not including the `DuckDB` 2.1 bindings (1.20100.x), which quack-rs has not
/// been tested with.
pub(super) const LIBDUCKDB_SYS_RANGE: &str = ">=1.4.4, <1.20100";

/// The newest of quack-rs's `duckdb-1-5*` features a project stamped for
/// `config` can use, if any: the point of stamping C API v1.5.6, or an exact
/// `DuckDB` 1.5 release, is to use them.
pub(super) fn quack_rs_feature(config: &ScaffoldConfig) -> Option<&'static str> {
    if !super::pins_bindings(config) {
        return None;
    }
    if !config.use_unstable_c_api {
        return Some("duckdb-1-5-4");
    }
    let mut parts = config
        .target_duckdb_version
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    match (parts.next(), parts.next(), parts.next()) {
        (Some(1), Some(5), Some(patch)) if patch >= 4 => Some("duckdb-1-5-4"),
        (Some(1), Some(5), Some(3)) => Some("duckdb-1-5-3"),
        (Some(1), Some(5), _) => Some("duckdb-1-5"),
        _ => None,
    }
}

pub(super) fn generate_cargo_toml(config: &ScaffoldConfig) -> String {
    // A C_STRUCT v1.2.0 build only uses the frozen stable prefix of the C API,
    // so any bindings work. A build that may use the `duckdb-1-5*` features is
    // only correct against the layout it is stamped for, so its bindings are
    // pinned: to the exact release for C_STRUCT_UNSTABLE, to the v1.5.6 ones
    // (the layout DuckDB v1.5.6 and v2.0.0 share) for C_STRUCT v1.5.6.
    let libduckdb_sys = if super::pins_bindings(config) {
        super::checks::libduckdb_sys_requirement(&config.target_duckdb_version)
            .unwrap_or_else(|_| String::from(LIBDUCKDB_SYS_RANGE))
    } else {
        String::from(LIBDUCKDB_SYS_RANGE)
    };
    let (quack_rs_note, quack_rs_features) = quack_rs_feature(config).map_or_else(
        || (String::new(), String::new()),
        |feature| {
            (
                format!(
                    "# `{feature}`: quack-rs's API past the stable prefix of the C extension API,\n\
                     # which is what this project's stamping (TARGET_DUCKDB_VERSION in the\n\
                     # Makefile) is for. Without it the project builds for the stable API only.\n"
                ),
                format!(", features = [\"{feature}\"]"),
            )
        },
    );
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
        String::from(
            "# Any bindings release quack-rs supports, below the DuckDB 2.1 bindings\n\
             # (1.20100), which this quack-rs has not been tested with: a C_STRUCT v1.2.0\n\
             # binary uses only the stable prefix of the C API, which all of them have.\n",
        )
    };
    // Cargo unifies features: with the defaults left on here they would be on
    // for quack-rs's copy of libduckdb-sys as well.
    let libduckdb_sys_note = libduckdb_sys_note
        + "# `default-features = false`: the defaults (from 1.20000, an HTTP/TLS client\n\
           # that downloads libduckdb, and vcpkg / pkg-config probing) are for linking a\n\
           # whole DuckDB, not for a loadable extension; Cargo unifies features, so\n\
           # enabling them here would enable them for quack-rs's copy too.\n";
    format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"
license = "{license}""#,
        name = config.name,
        version = config.version,
        // Normalised to single spaces, as in `description.yml`: the SPDX check
        // splits an expression on any Unicode whitespace, so `MIT\nOR
        // Apache-2.0` passes it, but a line break inside this TOML basic string
        // is not TOML. (Identifiers hold only letters, digits, `.`, `-`, `+`.)
        license = config
            .license
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    ) + &format!(
        r#"
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
{quack_rs_note}quack-rs = {{ version = "{quack_rs}"{quack_rs_features} }}
{libduckdb_sys_note}libduckdb-sys = {{ version = "{libduckdb_sys}", default-features = false, features = ["loadable-extension"] }}

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
        msrv = env!("CARGO_PKG_RUST_VERSION"),
        quack_rs = quack_rs_dependency_version(),
    )
}

pub(super) fn generate_lib_rs(config: &ScaffoldConfig) -> String {
    format!(
        r#"{description}
//!
//! A DuckDB extension built with [quack-rs](https://github.com/tomtom215/quack-rs).

use quack_rs::prelude::*;

// ---------------------------------------------------------------------------
// Example: a scalar function. Replace it with your own functions.
// ---------------------------------------------------------------------------

/// The extension's name, as DuckDB knows it.
const EXTENSION: &str = "{name}";

/// The SQL name of the example function.
const HELLO: &str = "{name}_hello";

/// What `{name}_hello(name)` returns. Plain Rust, so it is unit-tested below
/// without DuckDB.
fn greet(name: &str) -> String {{
    format!("Hello from {{EXTENSION}}! {{name}}")
}}

/// Registers all extension functions on the given connection.
fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {{
    // A native scalar function from a closure: quack-rs derives the SQL types
    // from the closure's signature (VARCHAR -> VARCHAR), returns NULL for a
    // NULL argument without calling it, and turns a panic into a query error.
    // A native function lives in the extension, so the user's database is not
    // changed and LOAD works on a read-only one too (an SQL macro would be
    // written into the database).
    //
    // SAFETY: `con` is the connection quack-rs opened for this entry point and
    // is valid for the duration of this function.
    unsafe {{
        ScalarFunctionBuilder::map1_str(HELLO, |name: &str| greet(name))?.register(con)?;
    }}
    Ok(())
}}

// ---------------------------------------------------------------------------
// Entry point — the C Extension API handles everything, no C++ glue needed.
// ---------------------------------------------------------------------------

{entry_point_call}

// Unit tests run under plain `cargo test` and need no DuckDB. Behaviour
// against a real engine is tested in test/sql/{name}.test (SQLLogicTest, run
// by `make test`).
#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn greets_by_name() {{
        assert_eq!(greet("world"), format!("Hello from {{EXTENSION}}! world"));
    }}
}}
"#,
        description = doc_comment_lines(&config.description),
        name = config.name,
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
    // scripts/build.py builds `ref` for the newest release, `variegata` for
    // v1.5.6, `andium` for v1.4.5 and `ref_next` for a release in preparation.
    let latest = super::DISTRIBUTION_DUCKDB_VERSION;
    let release_lines: &[&str] = if config.use_unstable_c_api {
        &[
            "  # This project is stamped C_STRUCT_UNSTABLE: the binary loads into one DuckDB",
            "  # release only, and the community build builds `ref` for the newest release",
            "  # (@@LATEST@@), which refuses it. Point `ref` at a revision stamped C_STRUCT, and",
            "  # give this one under the key for its release:",
            "  # variegata: <hash>  # built for DuckDB v1.5.6",
            "  # andium: <hash>     # built for DuckDB v1.4.5",
        ]
    } else {
        &[
            "  # Optional, each a revision to build instead of `ref` (which is built for",
            "  # the newest release, @@LATEST@@):",
            "  # variegata: <hash>  # for DuckDB v1.5.6",
            "  # andium: <hash>     # for DuckDB v1.4.5",
            "  # ref_next: <hash>   # for a DuckDB release still in preparation",
        ]
    };
    for line in release_lines {
        let _ = writeln!(yml, "{}", line.replace("@@LATEST@@", latest));
    }

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
/// The header lines are the ones `DuckDB`'s own test files and
/// `duckdb/extension-template-rs` carry. The runner executes the file top to
/// bottom and loads the extension at `require`, so the `statement error`
/// before it proves the function comes from this extension. The NULL is read
/// from a column: a literal `NULL` argument is constant-folded, so it never
/// reaches the function's per-row NULL handling.
///
/// Pitfall P5: Expected values must match `DuckDB`'s exact output format.
/// Generate expected values by running queries in the `DuckDB` CLI and copying the output.
pub(super) fn generate_sqllogictest(config: &ScaffoldConfig) -> String {
    let name = &config.name;
    format!(
        "# name: test/sql/{name}.test\n\
         # description: test the {name} extension\n\
         # group: [{name}]\n\
         #\n\
         # Run with `make test`. Format: https://duckdb.org/docs/stable/dev/sqllogictest/intro\n\
         # - query T = VARCHAR result, query I = INTEGER, query R = REAL, query B = BOOLEAN\n\
         # - Expected output must match DuckDB's exact format:\n\
         #   {P5_URL}\n\
         \n\
         # Before the extension is loaded, the function does not exist.\n\
         statement error\n\
         SELECT {name}_hello('world');\n\
         ----\n\
         Catalog Error: Scalar Function with name {name}_hello does not exist!\n\
         \n\
         # Loads the extension. A file whose `require` cannot be met is skipped,\n\
         # which the Makefile's test target turns into a failure.\n\
         require {name}\n\
         \n\
         # The example function registered in src/lib.rs.\n\
         query T\n\
         SELECT {name}_hello('world');\n\
         ----\n\
         Hello from {name}! world\n\
         \n\
         # One row per input row, and NULL in, NULL out. The NULL comes from a\n\
         # column: a literal NULL argument would be folded away before the call.\n\
         query T\n\
         SELECT {name}_hello(x) FROM (VALUES ('a'), (NULL), ('b')) t(x) ORDER BY x NULLS LAST;\n\
         ----\n\
         Hello from {name}! a\n\
         Hello from {name}! b\n\
         NULL\n\
         \n\
         # ---- Add a test like these for each function you add. ----\n\
         # For example, an aggregate you register as {name}_count:\n\
         # query I\n\
         # SELECT {name}_count(col) FROM (VALUES (1), (2), (3)) t(col);\n\
         # ----\n\
         # 3\n\
         ",
    )
}

/// The book's pitfall P5 (`SQLLogicTest` expected values).
const P5_URL: &str =
    "https://quack-rs.com/reference/pitfalls.html#p5-sqllogictest-expected-values-must-match-exactly";
