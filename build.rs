// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Build script for quack-rs.
//!
//! When the `bundled-test` feature is active this compiles a tiny C++ shim
//! (`src/testing/bundled_api_init.cpp`) that exposes `DuckDB`'s internal
//! `CreateAPIv1()` function as a C-linkage symbol.  The Rust side calls this
//! at test startup to populate the `loadable-extension` dispatch table from
//! the bundled `DuckDB` symbols, enabling `InMemoryDb` to work in `cargo test`.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    // Only needed when bundled-test is enabled.
    if env::var("CARGO_FEATURE_BUNDLED_TEST").is_err() {
        return;
    }

    let duckdb_include = find_duckdb_include();

    cc::Build::new()
        .cpp(true)
        .file("src/testing/bundled_api_init.cpp")
        .include(&duckdb_include)
        // DuckDB headers use C++11 features; keep it minimal.
        .flag_if_supported("-std=c++11")
        // Suppress warnings from DuckDB headers that we don't own.
        .flag_if_supported("-w")
        .compile("quack_rs_bundled_init");

    println!("cargo:rerun-if-changed=src/testing/bundled_api_init.cpp");
}

/// Locates the `DuckDB` include directory from `libduckdb-sys`'s build output.
///
/// Cargo places all crate build outputs under `target/{profile}/build/`.  We
/// navigate up from our own `OUT_DIR` to that shared `build/` directory and
/// search for a `libduckdb-sys-*` subdirectory that contains the extracted
/// `DuckDB` source tree (present only when the `bundled` feature is active).
fn find_duckdb_include() -> PathBuf {
    // OUT_DIR  = .../target/{profile}/build/quack-rs-{hash}/out
    // We want  = .../target/{profile}/build/
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let build_dir = out_dir
        .parent() // .../build/quack-rs-{hash}
        .and_then(Path::parent) // .../build
        .expect("could not navigate to Cargo build directory from OUT_DIR");

    for entry in std::fs::read_dir(build_dir)
        .expect("could not read Cargo build directory")
        .flatten()
    {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("libduckdb-sys-")
        {
            continue;
        }

        let candidate = entry.path().join("out/duckdb/src/include");
        if candidate.is_dir() {
            return candidate;
        }
    }

    panic!(
        "Could not find DuckDB headers from libduckdb-sys build output.\n\
         Ensure that the `duckdb` dependency is resolved with `features = [\"bundled\"]`\n\
         and that `libduckdb-sys` has been built before this crate."
    );
}
