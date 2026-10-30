// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! quack-rs's build script run with a crafted `QUACK_RS_TARGET_DUCKDB_VERSION`
//! (HARDEN-12): the value is printed into a `cargo:` directive, which cargo
//! reads a line at a time, so a value holding a newline used to add
//! directives of its own.
//!
//! The test runs the build-script binary Cargo compiled for this test run, with
//! no other environment, which takes it down the path every build takes
//! (no `bundled-test*` feature set: nothing is compiled).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The newest build-script binary of this package under a `build` directory
/// above the test binary. Cargo's directory layout has two forms:
/// `build/quack-rs-<hash>/build-script-build` (stable today) and
/// `build/quack-rs/<hash>/out/build_script_build` (the new layout, on nightly,
/// where the test binary sits in `out/` too).
fn build_script() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut consider = |dir: &Path| {
        for name in [
            "build-script-build",
            "build-script-build.exe",
            "build_script_build",
            "build_script_build.exe",
        ] {
            let path = dir.join(name);
            let Ok(modified) = path.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    };
    for dir in exe.ancestors().skip(1).take(6) {
        let Ok(entries) = std::fs::read_dir(dir.join("build")) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "quack-rs" {
                for hashed in std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    consider(&hashed.path().join("out"));
                }
            } else if name.starts_with("quack-rs-") {
                consider(&entry.path());
            }
        }
    }
    newest
        .map(|(_, path)| path)
        .expect("Cargo compiled quack-rs's build script for this test run")
}

/// The `cargo:` directives the build script prints for `value`.
fn directives(script: &Path, value: &str) -> Vec<String> {
    let out = Command::new(script)
        .env_clear()
        .env("QUACK_RS_TARGET_DUCKDB_VERSION", value)
        .output()
        .expect("the build script runs");
    assert!(out.status.success(), "{value:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "spawns the build script")]
fn only_a_release_version_reaches_a_cargo_directive() {
    let script = build_script();
    let forwarded = |lines: &[String]| {
        lines
            .iter()
            .filter(|l| l.starts_with("cargo:rustc-env=QUACK_RS_BUILT_AGAINST_DUCKDB="))
            .cloned()
            .collect::<Vec<_>>()
    };

    let lines = directives(&script, "v1.5.6");
    assert_eq!(
        forwarded(&lines),
        ["cargo:rustc-env=QUACK_RS_BUILT_AGAINST_DUCKDB=v1.5.6"],
        "{lines:?}"
    );

    // A newline in the value must not start a directive of its own.
    let lines = directives(&script, "v1.5.6\ncargo:rustc-cfg=injected_by_env");
    assert!(
        !lines.iter().any(|l| l.starts_with("cargo:rustc-cfg")),
        "{lines:?}"
    );
    assert!(forwarded(&lines).is_empty(), "{lines:?}");
    let warnings: Vec<_> = lines
        .iter()
        .filter(|l| l.starts_with("cargo:warning="))
        .collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    assert!(
        warnings[0].contains("\\n"),
        "the value is quoted: {lines:?}"
    );

    // Not a release, so not declared: `u64::from_str` alone takes the `+`.
    for value in ["v+1.5.6", "v1.5.6-dev123", "1.5", "v1..6"] {
        let lines = directives(&script, value);
        assert!(forwarded(&lines).is_empty(), "{value:?}: {lines:?}");
        assert!(
            lines.iter().any(|l| l.starts_with("cargo:warning=")),
            "{value:?}: {lines:?}"
        );
    }
    // Every line is one of the build script's own directives.
    for value in [
        "v1.5.6",
        "v1.5.6\ncargo:rustc-cfg=x",
        "x\ncargo:rustc-link-lib=evil",
    ] {
        for line in directives(&script, value) {
            assert!(
                line.starts_with("cargo:rerun-if-env-changed=")
                    || line.starts_with("cargo:rustc-env=QUACK_RS_BUILT_AGAINST_DUCKDB=")
                    || line.starts_with("cargo:warning="),
                "{value:?}: {line:?}"
            );
        }
    }
}
