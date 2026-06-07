// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Build script for quack-rs.
//!
//! When the `bundled-test` (or `bundled-test-prebuilt`) feature is active this
//! compiles a tiny C++ shim (`src/testing/bundled_api_init.cpp`) that exposes
//! `DuckDB`'s internal `CreateAPIv1()` function as a C-linkage symbol.  The Rust
//! side calls this at test startup to populate the `loadable-extension` dispatch
//! table from the linked `DuckDB` symbols, enabling `InMemoryDb` to work in
//! `cargo test`.
//!
//! Two modes are supported:
//!
//! 1. **`bundled-test` (source)**: `libduckdb-sys` extracts and compiles
//!    `DuckDB` from C++ source.  Headers come from the extraction, linking is
//!    handled by `libduckdb-sys` itself.
//! 2. **`bundled-test-prebuilt`**: `libduckdb-sys` runs its `build_linked` path,
//!    which can either download a pre-built libduckdb (`DUCKDB_DOWNLOAD_LIB=1`,
//!    recommended) or use a user-supplied tree (`DUCKDB_LIB_DIR`).  In
//!    `loadable-extension` mode `libduckdb-sys` suppresses its
//!    `cargo:rustc-link-lib` emission, so quack-rs emits it here.
//!
//! Header resolution prefers `DEP_DUCKDB_INCLUDE` (emitted via `cargo:include=`
//! by `libduckdb-sys` >= 1.10503) and falls back to mode-specific probes for
//! older versions, preserving the MSRV and dep-range of pre-1.10503 consumers.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    // On Windows, bundled DuckDB uses the Restart Manager API (RmStartSession,
    // RmEndSession, RmRegisterResources, RmGetList) in its AdditionalLockInfo()
    // function.  libduckdb-sys's build script does not emit a link directive for
    // rstrtmgr.lib, so we add it here whenever we're building for Windows.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=rstrtmgr");
    }

    // Only needed when DuckDB testing support is enabled (either feature).
    let source_build = env::var("CARGO_FEATURE_BUNDLED_TEST").is_ok();
    let prebuilt_feature = env::var("CARGO_FEATURE_BUNDLED_TEST_PREBUILT").is_ok();
    if !source_build && !prebuilt_feature {
        return;
    }
    // Prebuilt mode applies only when the prebuilt feature is on and the source
    // (`bundled`) build is not — if both are enabled, `duckdb/bundled` wins and
    // libduckdb-sys links the compiled-from-source library itself.
    let prebuilt = prebuilt_feature && !source_build;

    if prebuilt {
        emit_prebuilt_link_lib();
    }

    let duckdb_include = resolve_duckdb_include(!prebuilt);

    cc::Build::new()
        .cpp(true)
        .file("src/testing/bundled_api_init.cpp")
        .include(&duckdb_include)
        // DuckDB headers use C++11 features; keep it minimal.
        .flag_if_supported("-std=c++11")
        // Suppress warnings from DuckDB headers that we don't own.
        .flag_if_supported("-w")
        // On Windows/MSVC the DuckDB headers declare all public symbols with
        // __declspec(dllimport) unless DUCKDB_STATIC_BUILD is defined.  Without
        // this flag the compiler emits __imp_duckdb_* references, but the
        // bundled static library exports plain duckdb_* symbols, causing
        // LNK2019 "unresolved external symbol __imp_duckdb_*" errors at link.
        .define("DUCKDB_STATIC_BUILD", None)
        .compile("quack_rs_bundled_init");

    println!("cargo:rerun-if-changed=src/testing/bundled_api_init.cpp");
}

/// In `bundled-test-prebuilt` mode, `libduckdb-sys` resolves the library
/// location (downloaded or `DUCKDB_LIB_DIR`) and emits
/// `cargo:rustc-link-search=...`, but its `loadable-extension` feature
/// intentionally skips `cargo:rustc-link-lib=...` (the loaded extension's
/// host process is normally responsible for providing libduckdb).  In
/// `cargo test` we *are* the host, so we emit the link-lib directive here.
fn emit_prebuilt_link_lib() {
    println!("cargo:rerun-if-env-changed=DUCKDB_DOWNLOAD_LIB");
    println!("cargo:rerun-if-env-changed=DUCKDB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=DUCKDB_INCLUDE_DIR");
    println!("cargo:rustc-link-lib=dylib=duckdb");
}

/// Resolves the include directory containing `duckdb.hpp`.  Tries (in order):
///
/// 1. `DEP_DUCKDB_INCLUDE` — emitted by `libduckdb-sys >= 1.10503`.  Works in
///    both bundled and `build_linked` (download / `DUCKDB_LIB_DIR`) paths.
/// 2. Mode-specific fallbacks for older `libduckdb-sys`:
///    - **source build (`bundled`)**: scan the Cargo build directory for the
///      extracted source tree (the historical path used by quack-rs <= 0.12).
///    - **prebuilt**: `DUCKDB_INCLUDE_DIR`, then `DUCKDB_LIB_DIR` if it has
///      `duckdb.hpp` alongside the library.  `DUCKDB_DOWNLOAD_LIB` is not
///      probeable on pre-1.10503 (the download dir isn't exposed), so users
///      on that mode must upgrade `libduckdb-sys` or set the dirs explicitly.
fn resolve_duckdb_include(bundled: bool) -> PathBuf {
    if let Some(dir) = env::var_os("DEP_DUCKDB_INCLUDE") {
        let p = PathBuf::from(dir);
        if p.join("duckdb.hpp").exists() {
            return p;
        }
    }

    if bundled {
        find_duckdb_include_via_scan()
    } else {
        resolve_prebuilt_include_from_env()
    }
}

/// Bundle-less, pre-1.10503 fallback: derive the include directory from
/// the env vars the user is required to set anyway.
fn resolve_prebuilt_include_from_env() -> PathBuf {
    let mut inspected: Vec<(&str, PathBuf)> = Vec::new();
    if let Some(inc) = env::var_os("DUCKDB_INCLUDE_DIR") {
        let p = PathBuf::from(inc);
        if p.join("duckdb.hpp").exists() {
            return p;
        }
        inspected.push(("DUCKDB_INCLUDE_DIR", p));
    }
    if let Some(lib) = env::var_os("DUCKDB_LIB_DIR") {
        let p = PathBuf::from(lib);
        if p.join("duckdb.hpp").exists() {
            return p;
        }
        inspected.push(("DUCKDB_LIB_DIR", p));
    }
    let inspected_report = if inspected.is_empty() {
        "  (neither DUCKDB_INCLUDE_DIR nor DUCKDB_LIB_DIR was set)".to_string()
    } else {
        inspected
            .iter()
            .map(|(name, p)| format!("  {name}={} (no duckdb.hpp here)", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    panic!(
        "quack-rs: bundled-test without bundled could not locate duckdb.hpp.\n\
         Inspected:\n{inspected_report}\n\
         Either upgrade libduckdb-sys to >= 1.10503 (which publishes the include\n\
         directory via DEP_DUCKDB_INCLUDE), or point DUCKDB_INCLUDE_DIR /\n\
         DUCKDB_LIB_DIR at a directory containing duckdb.hpp."
    );
}

/// Legacy fallback for older `libduckdb-sys` versions that don't emit
/// `cargo:include=`.  Navigates from `OUT_DIR` to the shared Cargo build
/// directory and scans for `libduckdb-sys-*/out/duckdb/src/include`.
///
/// **Build-order caveat**: Cargo runs build scripts as soon as their
/// `[build-dependencies]` are ready — *before* regular `[dependencies]` are
/// compiled.  This means `libduckdb-sys` (a regular dependency) may still be
/// compiling when this function executes.  We therefore poll with a timeout
/// to wait for the headers to appear.
fn find_duckdb_include_via_scan() -> PathBuf {
    // OUT_DIR  = .../target/{profile}/build/quack-rs-{hash}/out
    // We want  = .../target/{profile}/build/
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let build_dir = out_dir
        .parent() // .../build/quack-rs-{hash}
        .and_then(Path::parent) // .../build
        .expect("could not navigate to Cargo build directory from OUT_DIR");

    // Poll for up to ~10 minutes (libduckdb-sys compiles the full DuckDB C++
    // source when the `bundled` feature is active, which can take several
    // minutes on CI runners).
    for attempt in 0..120 {
        if let Some(path) = scan_for_duckdb_headers(build_dir) {
            return path;
        }
        if attempt < 119 {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }

    panic!(
        "Could not find DuckDB headers from libduckdb-sys build output.\n\
         Ensure that the `duckdb` dependency is resolved with `features = [\"bundled\"]`\n\
         and that `libduckdb-sys` has been built before this crate."
    );
}

/// Scans the Cargo build directory for `libduckdb-sys-*` subdirectories
/// that contain the extracted `DuckDB` include tree.
fn scan_for_duckdb_headers(build_dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(build_dir).ok()?.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("libduckdb-sys-")
        {
            continue;
        }

        let candidate = entry.path().join("out/duckdb/src/include");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}
