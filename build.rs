// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! Build script for quack-rs.
//!
//! When the `bundled-test` (or `bundled-test-prebuilt`) feature is active this
//! compiles a tiny shim that fills a `duckdb_ext_api_v1` struct from the linked
//! `DuckDB` symbols, which is what `DuckDB`'s internal `CreateAPIv1()` does. The
//! Rust side calls it at test startup to populate the `loadable-extension`
//! dispatch table, enabling `InMemoryDb` to work in `cargo test`.
//!
//! The shim is C generated from `duckdb_extension.h` whenever the headers
//! include it (every `DuckDB` 1.5 and 2.0 release archive, and the bundled
//! source tree): one `slots[i] = (void *)duckdb_x;` per field, in the
//! header's order, against `duckdb.h` alone. `DuckDB` 2.0's release archives
//! ship no `duckdb.hpp`, so this is the only way to build there; and it needs no
//! C++ standard at all (2.0's C++ headers need C++17). Where only `duckdb.hpp`
//! is available (the `DuckDB` 1.4 release zips), the C++ shim
//! `src/testing/bundled_api_init.cpp` calls `CreateAPIv1()` itself.
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
    export_built_against_duckdb_version();

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

    let duckdb_include = resolve_duckdb_include(!prebuilt);

    let mut build = cc::Build::new();
    let extension_header = duckdb_include.join("duckdb_extension.h");
    if extension_header.exists() {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let shim = out_dir.join("quack_rs_api_v1_fill.c");
        let header = std::fs::read_to_string(&extension_header)
            .unwrap_or_else(|e| panic!("reading {}: {e}", extension_header.display()));
        let fields = ext_api_v1_fields(&header).unwrap_or_else(|e| {
            panic!("quack-rs: {}: {e}", extension_header.display());
        });
        std::fs::write(&shim, generated_c_shim(&fields, &duckdb_include))
            .unwrap_or_else(|e| panic!("writing {}: {e}", shim.display()));
        build.file(&shim);
        println!("cargo:rerun-if-changed={}", extension_header.display());
    } else {
        build
            .cpp(true)
            .file("src/testing/bundled_api_init.cpp")
            // The DuckDB 1.4 headers need C++11; C++17 is a superset, and what
            // DuckDB 2.0's C++ headers require, should a tree without
            // `duckdb_extension.h` but with 2.0's `duckdb.hpp` ever turn up.
            .flag_if_supported("-std=c++17")
            .flag_if_supported("/std:c++17");
        println!("cargo:rerun-if-changed=src/testing/bundled_api_init.cpp");
    }
    build
        .include(&duckdb_include)
        // Suppress warnings from DuckDB headers that we don't own.
        .flag_if_supported("-w")
        // On Windows/MSVC the DuckDB headers declare all public symbols with
        // __declspec(dllimport) unless DUCKDB_STATIC_BUILD is defined.  Without
        // this flag the compiler emits __imp_duckdb_* references, but the
        // bundled static library exports plain duckdb_* symbols, causing
        // LNK2019 "unresolved external symbol __imp_duckdb_*" errors at link.
        .define("DUCKDB_STATIC_BUILD", None)
        .compile("quack_rs_bundled_init");

    // After `compile`, which emits the shim's own `rustc-link-lib`, so the
    // library the shim calls into is named after it. Emitted before, the test
    // binary failed to link on Rust 1.86.0 (MSRV) with "undefined reference to
    // `duckdb_open`" from the shim, while 1.97 linked it.
    if prebuilt {
        emit_prebuilt_link_lib();
    }
}

/// The function-pointer field names of `duckdb_ext_api_v1`, in declaration
/// order, with every conditional band included.
///
/// Every band is what `libduckdb-sys` compiles in (it defines
/// `DUCKDB_EXTENSION_API_VERSION_UNSTABLE` and targets the newest C API) and
/// what `CreateAPIv1()` fills, so the generated shim matches both. The only
/// conditionals the struct has ever contained are `#if`/`#ifdef` bands with no
/// alternative; an `#else` or `#elif` would mean a field list this parser
/// cannot evaluate, so it is refused rather than guessed at.
fn ext_api_v1_fields(header: &str) -> Result<Vec<String>, String> {
    let start = header
        .find("typedef struct {")
        .ok_or("no `typedef struct {` (is this duckdb_extension.h?)")?;
    let end = header[start..]
        .find("} duckdb_ext_api_v1;")
        .ok_or("no `} duckdb_ext_api_v1;`")?
        + start;
    let body = &header[start..end];
    let mut fields = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#else") || trimmed.starts_with("#elif") {
            return Err(format!(
                "duckdb_ext_api_v1 contains `{}`, which the test shim generator does not handle",
                trimmed.trim_end()
            ));
        }
    }
    let mut rest = body;
    while let Some(at) = rest.find("(*") {
        rest = &rest[at + 2..];
        let name: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.starts_with("duckdb_") {
            return Err(format!("unexpected function-pointer field name {name:?}"));
        }
        fields.push(name);
    }
    if fields.is_empty() {
        return Err("duckdb_ext_api_v1 has no function-pointer fields".into());
    }
    Ok(fields)
}

/// C source that fills a caller's `duckdb_ext_api_v1`-sized buffer field by
/// field, as `CreateAPIv1()` does, using only `duckdb.h`'s declarations.
///
/// `duckdb_extension.h` itself is not included: from v1.5.6 it `#define`s every
/// `duckdb_x` to `duckdb_ext_api.duckdb_x`, which would rewrite these
/// assignments.
fn generated_c_shim(fields: &[String], include: &Path) -> String {
    use std::fmt::Write as _;
    // A C string literal: Rust's escaping is C's for printable ASCII.
    let origin = include.display().to_string();
    let origin = if origin
        .chars()
        .all(|c| c.is_ascii() && !c.is_ascii_control())
    {
        format!("{:?}", format!("DuckDB C headers in {origin}"))
    } else {
        "\"DuckDB C headers (non-ASCII path)\"".to_string()
    };
    let mut c = String::new();
    c.push_str(
        "/* Generated by quack-rs's build.rs from duckdb_extension.h. Do not edit. */\n\
         #include \"duckdb.h\"\n#include <stddef.h>\n\n",
    );
    let _ = writeln!(
        c,
        "size_t quack_rs_api_v1_size(void) {{ return {} * sizeof(void *); }}\n",
        fields.len()
    );
    let _ = writeln!(
        c,
        "const char *quack_rs_header_duckdb_version(void) {{ return {origin}; }}\n"
    );
    c.push_str("void quack_rs_fill_api_v1(void *out) {\n    void **slots = (void **)out;\n");
    for (i, field) in fields.iter().enumerate() {
        let _ = writeln!(c, "    slots[{i}] = (void *){field};");
    }
    c.push_str("}\n");
    c
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

/// Whether `dir` holds headers the test shim can be built from:
/// `duckdb_extension.h` (the generated C shim) or `duckdb.hpp` (the C++ one).
fn has_shim_headers(dir: &Path) -> bool {
    dir.join("duckdb_extension.h").exists() || dir.join("duckdb.hpp").exists()
}

/// Resolves the include directory holding `duckdb_extension.h` or
/// `duckdb.hpp` (see [`has_shim_headers`]).  Tries (in order):
///
/// 1. `DEP_DUCKDB_INCLUDE` — emitted by `libduckdb-sys >= 1.10503`.  Works in
///    both bundled and `build_linked` (download / `DUCKDB_LIB_DIR`) paths.
/// 2. Mode-specific fallbacks for older `libduckdb-sys`:
///    - **source build (`bundled`)**: scan the Cargo build directory for the
///      extracted source tree (the historical path used by quack-rs <= 0.12).
///    - **prebuilt**: `DUCKDB_INCLUDE_DIR`, then `DUCKDB_LIB_DIR` if it has
///      the headers alongside the library.  `DUCKDB_DOWNLOAD_LIB` is not
///      probeable on pre-1.10503 (the download dir isn't exposed), so users
///      on that mode must upgrade `libduckdb-sys` or set the dirs explicitly.
fn resolve_duckdb_include(bundled: bool) -> PathBuf {
    if let Some(dir) = env::var_os("DEP_DUCKDB_INCLUDE") {
        let p = PathBuf::from(dir);
        if has_shim_headers(&p) {
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
        if has_shim_headers(&p) {
            return p;
        }
        inspected.push(("DUCKDB_INCLUDE_DIR", p));
    }
    if let Some(lib) = env::var_os("DUCKDB_LIB_DIR") {
        let p = PathBuf::from(lib);
        if has_shim_headers(&p) {
            return p;
        }
        inspected.push(("DUCKDB_LIB_DIR", p));
    }
    let inspected_report = if inspected.is_empty() {
        "  (neither DUCKDB_INCLUDE_DIR nor DUCKDB_LIB_DIR was set)".to_string()
    } else {
        inspected
            .iter()
            .map(|(name, p)| {
                format!(
                    "  {name}={} (neither duckdb_extension.h nor duckdb.hpp here)",
                    p.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    panic!(
        "quack-rs: bundled-test without bundled could not locate the DuckDB headers\n\
         (duckdb_extension.h or duckdb.hpp).\n\
         Inspected:\n{inspected_report}\n\
         Either upgrade libduckdb-sys to >= 1.10503 (which publishes the include\n\
         directory via DEP_DUCKDB_INCLUDE), or point DUCKDB_INCLUDE_DIR /\n\
         DUCKDB_LIB_DIR at the extracted DuckDB release archive."
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

/// Forwards `QUACK_RS_TARGET_DUCKDB_VERSION` into the crate as
/// `QUACK_RS_BUILT_AGAINST_DUCKDB`, so `abi::built_against_version()` can read
/// it with `option_env!`.
///
/// Doing this through the build script rather than reading the variable with
/// `option_env!` directly is what makes it *correct*: Cargo does not track
/// environment variables read by `option_env!` in library code, so changing the
/// variable would not invalidate a cached build and a stale value would be baked
/// in. `cargo:rerun-if-env-changed` fixes that.
///
/// The variable names the exact `DuckDB` release whose headers `libduckdb-sys`
/// was resolved against. It is deliberately **not** read from
/// `DUCKDB_EXTENSION_MIN_DUCKDB_VERSION`, which `extension-ci-tools` also sets:
/// that one carries `TARGET_DUCKDB_VERSION`, which means the *C API* version
/// (`v1.2.0`) for a `C_STRUCT` build and a `DuckDB` release only for a
/// `C_STRUCT_UNSTABLE` one. Treating those two meanings as interchangeable would
/// let a `C_STRUCT` build claim it was compiled against `DuckDB` v1.2.0.
fn export_built_against_duckdb_version() {
    println!("cargo:rerun-if-env-changed=QUACK_RS_TARGET_DUCKDB_VERSION");
    if let Ok(version) = env::var("QUACK_RS_TARGET_DUCKDB_VERSION") {
        let version = version.trim();
        if !version.is_empty() {
            // Anything but `vX.Y.Z` is dropped here, not passed on: this value
            // is printed into a `cargo:` directive, and cargo reads those a
            // line at a time, so a value holding a newline (`v1.5.6\ncargo:
            // rustc-cfg=...`) would add directives of its own (HARDEN-12).
            // `abi::built_against_version` ignores such a value anyway (a
            // development build's `v1.5.6-dev123` included); say so here,
            // where the declaration is made, rather than only in a LOAD error.
            if !is_release_version(version) {
                println!(
                    "cargo:warning=QUACK_RS_TARGET_DUCKDB_VERSION={version:?} is not a release \
                     version (vX.Y.Z) and is ignored: the ABI check treats this build as \
                     undeclared"
                );
                return;
            }
            println!("cargo:rustc-env=QUACK_RS_BUILT_AGAINST_DUCKDB={version}");
        }
    }
}

/// `vX.Y.Z` (the `v` optional) with three parts of ASCII digits, each a
/// `u64`: what `abi::parse_version` accepts, less the leading `+` that
/// `u64::from_str` also takes. Only such a value reaches a `cargo:` directive.
fn is_release_version(version: &str) -> bool {
    let parts: Vec<&str> = version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u64>().is_ok()
        })
}
