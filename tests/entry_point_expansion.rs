// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Compiles the text of the entry-point macros, as written in
//! `src/entry_point.rs`, in settings this crate's own build never uses.
//!
//! - **Edition 2024.** The edition that applies to an exported macro's tokens
//!   is the defining crate's, so building quack-rs (edition 2021) cannot show
//!   what its macros do once it moves to 2024 — and a bare `#[no_mangle]` in
//!   them would then fail every extension that expands them.
//! - **`panic = "abort"`.** The macros refuse to compile there, because every
//!   panic guard in quack-rs is `catch_unwind`. The `cfg` is evaluated in the
//!   crate that expands the macro, and nothing in a test profile sets it.
//!
//! The macros are copied into a one-file crate whose stand-ins for
//! `libduckdb_sys` and quack-rs's own items have the paths the macros name,
//! and `rustc` compiles it (`--emit=metadata`, no dependencies, well under a
//! second). If a macro starts naming an item the stand-ins lack, the compile
//! fails and the error names it: add a stand-in to `STAND_INS`.
//!
//! The last two tests scan every file of `src/` for the edition-2024 unsafe
//! attributes and `extern` blocks written the old way.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `#[macro_export] macro_rules! name { … }` block for `name`, from its
/// `#[macro_export]` line to the `}` that closes it at column 0.
fn macro_text(source: &str, name: &str) -> String {
    let header = format!("#[macro_export]\nmacro_rules! {name} {{\n");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("src/entry_point.rs defines no exported macro `{name}`"));
    let body = &source[start..];
    let end = body
        .find("\n}\n")
        .unwrap_or_else(|| panic!("macro `{name}` has no closing brace at column 0"));
    body[..end + 3].to_owned()
}

/// What the macros reach through `$crate::` and `::libduckdb_sys::`, with the
/// same paths. `extern crate self as libduckdb_sys` makes the fixture its own
/// `libduckdb_sys`.
const STAND_INS: &str = r#"
#![allow(non_camel_case_types, dead_code, unused_unsafe)]
extern crate self as libduckdb_sys;

pub type duckdb_extension_info = *mut ::core::ffi::c_void;
pub struct duckdb_extension_access;
pub const DUCKDB_API_VERSION: &str = "v1.2.0";

pub mod abi {
    pub enum AbiPolicy { Strict, Trust }
}

pub mod entry_point {
    pub struct EntryPointOptions;
    impl EntryPointOptions {
        pub fn new() -> Self {
            Self
        }
    }
    macro_rules! stand_in {
        ($($name:ident),*) => {$(
            pub unsafe fn $name<P, R>(
                _: crate::duckdb_extension_info,
                _: *const crate::duckdb_extension_access,
                _: &str,
                _: P,
                _: R,
            ) -> bool {
                false
            }
        )*};
    }
    stand_in!(
        __entry_point,
        __entry_point_v2,
        __entry_point_with_options,
        __entry_point_v2_with_options
    );
}

fn register(_: u8) -> Result<(), ()> {
    Ok(())
}
"#;

/// Every arm of both macros, as an extension would write them.
const INVOCATIONS: &str = r"
entry_point!(two_arg_init_c_api, register);
entry_point!(three_arg_init_c_api, crate::abi::AbiPolicy::Trust, register);
entry_point_v2!(two_arg_v2_init_c_api, register);
entry_point_v2!(three_arg_v2_init_c_api, crate::abi::AbiPolicy::Trust, register);
entry_point!(options_init_c_api, options = crate::entry_point::EntryPointOptions::new(), register);
entry_point_v2!(options_v2_init_c_api, options = crate::entry_point::EntryPointOptions::new(), register);
";

/// Writes the fixture and compiles it with `rustc`, returning its exit status
/// and stderr.
fn compile(edition: &str, panic: &str) -> (bool, String) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(manifest_dir.join("src/entry_point.rs"))
        .expect("read src/entry_point.rs");
    let mut fixture = String::from(STAND_INS);
    for name in [
        "entry_point",
        "entry_point_v2",
        "__quack_rs_entry_point_fn",
        "__quack_rs_refuse_panic_abort",
    ] {
        fixture.push_str(&macro_text(&source, name));
    }
    fixture.push_str(INVOCATIONS);

    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("entry_point_expansion-{edition}-{panic}"));
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    let file = dir.join("fixture.rs");
    std::fs::write(&file, fixture).expect("write the fixture");

    // The `rustc` next to the `cargo` running this test: the same toolchain.
    let rustc =
        Path::new(env!("CARGO")).with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new(&rustc)
        .args([
            "--edition",
            edition,
            "--crate-type",
            "lib",
            "--emit=metadata",
        ])
        .args(["-C", &format!("panic={panic}")])
        .arg("--crate-name")
        .arg("fixture")
        .arg("--out-dir")
        .arg(&dir)
        .arg(&file)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", rustc.display()));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_entry_point_macros_compile_when_defined_in_an_edition_2024_crate() {
    for edition in ["2021", "2024"] {
        let (ok, stderr) = compile(edition, "unwind");
        assert!(ok, "edition {edition}:\n{stderr}");
        assert!(!stderr.contains("warning"), "edition {edition}:\n{stderr}");
    }
}

#[test]
fn the_entry_point_macros_refuse_to_compile_under_panic_abort() {
    let (ok, stderr) = compile("2021", "abort");
    assert!(!ok, "a panic = \"abort\" build compiled:\n{stderr}");
    let message = "quack-rs: this DuckDB extension is being compiled with `panic = \"abort\"`";
    // Once per expansion: six invocations, every arm of both macros.
    assert_eq!(stderr.matches(message).count(), 6, "{stderr}");
    assert!(
        stderr.contains("set it under `[profile.release]`"),
        "the error names the fix:\n{stderr}"
    );
    assert!(
        stderr.contains("Build it with `panic = \"unwind\"`"),
        "{stderr}"
    );
}

// ─── The rest of `src/` ─────────────────────────────────────────────────────
//
// Edition 2024 also makes `export_name` and `link_section` unsafe attributes
// and requires `unsafe extern` blocks. `cargo fix --edition` rewrites those
// outside macro bodies, but writing them that way now (valid from Rust 1.82;
// the MSRV is 1.86) keeps any future macro that copies them safe too.

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// `(file:line, text)` for every line of `src/` that `check` flags.
fn offending_lines(check: impl Fn(&str) -> bool) -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(files.len() > 50, "found only {} files in src/", files.len());
    let mut hits = Vec::new();
    for file in files {
        let text =
            std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            // Comments and doc comments may name the old spelling.
            if code.starts_with("//") {
                continue;
            }
            if check(code) {
                hits.push(format!("{}:{}: {}", file.display(), i + 1, code));
            }
        }
    }
    hits
}

#[test]
fn no_unsafe_attribute_is_written_without_unsafe() {
    // Matches `#[no_mangle]`, `#[export_name = "…"]` and
    // `#[link_section = "…"]`, alone or inside `cfg_attr`, but not their
    // `unsafe(...)` forms.
    let hits = offending_lines(|code| {
        ["no_mangle", "export_name", "link_section"]
            .iter()
            .any(|attr| {
                code.starts_with("#[")
                    && code.contains(attr)
                    && !code.contains(&format!("unsafe({attr}"))
            })
    });
    assert!(
        hits.is_empty(),
        "write these as #[unsafe(no_mangle)] (and so on): a bare one in a macro body \
         breaks every crate that expands the macro once quack-rs is on edition 2024:\n{}",
        hits.join("\n")
    );
}

#[test]
fn every_extern_block_is_an_unsafe_extern_block() {
    // An `extern "ABI" {` block opener, not an `extern "C" fn` definition.
    let is_block = |code: &str| {
        code.strip_prefix("extern \"")
            .and_then(|rest| rest.split_once('"'))
            .is_some_and(|(_abi, rest)| rest.trim() == "{")
    };
    let hits = offending_lines(is_block);
    assert!(
        hits.is_empty(),
        "write these as `unsafe extern \"C\" {{ … }}` (edition 2024 requires it):\n{}",
        hits.join("\n")
    );
}
