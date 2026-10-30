// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The `quack_scaffold` binary run as a user runs it: its exit status, what it
//! prints and the files it writes (or refuses to overwrite).

use std::path::Path;
use std::process::{Command, Output};

const REQUIRED: [&str; 6] = [
    "--description",
    "Demo functions",
    "--maintainer",
    "Jane Doe",
    "--github-repo",
    "janedoe/duckdb-demo",
];

fn run(dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quack_scaffold"))
        .arg("demo_ext")
        .args(REQUIRED)
        .arg("--dir")
        .arg(dir)
        .args(extra)
        .output()
        .expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_succeeds_and_describes_every_option() {
    let out = Command::new(env!("CARGO_BIN_EXE_quack_scaffold"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    assert!(out.status.success(), "{out:?}");
    let help = text(&out.stdout);
    for option in [
        "--description",
        "--maintainer",
        "--github-repo",
        "--version",
        "--license",
        "--excluded-platforms",
        "--target-duckdb-version",
        "--unstable-c-api",
        "--git-ref",
        "--dir",
        "--force",
    ] {
        assert!(help.contains(option), "{option}: {help}");
    }
}

#[test]
fn a_bad_argument_fails_with_the_reason_and_a_pointer_to_help() {
    let out = Command::new(env!("CARGO_BIN_EXE_quack_scaffold"))
        .args(["demo_ext", "--no-such-flag"])
        .output()
        .expect("the binary runs");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = text(&out.stderr);
    assert!(
        err.starts_with("error: unknown flag: --no-such-flag"),
        "{err}"
    );
    assert!(err.contains("--help"), "{err}");
}

#[test]
fn it_writes_the_project_and_prints_the_next_steps() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("demo_ext");
    let out = run(&dir, &[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for path in [
        "Cargo.toml",
        "Makefile",
        "src/lib.rs",
        "description.yml",
        "test/sql/demo_ext.test",
        "README.md",
        "LICENSE",
        "deny.toml",
        "docs/function_descriptions.csv",
        ".github/workflows/extension-ci.yml",
    ] {
        assert!(dir.join(path).is_file(), "{path} missing");
    }
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).expect("lib.rs");
    assert!(lib.contains("\"demo_ext_hello\""), "{lib}");

    let stdout = text(&out.stdout);
    for needle in [
        "git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git extension-ci-tools",
        "make configure release test",
        "append_metadata target/release/",
        "build/release/demo_ext.duckdb_extension --abi-type C_STRUCT --extension-version v0.1.0 --duckdb-version v1.2.0",
        "repo.ref",
    ] {
        assert!(stdout.contains(needle), "{needle}: {stdout}");
    }
    assert!(text(&out.stderr).is_empty(), "{}", text(&out.stderr));
}

#[test]
fn an_existing_file_is_not_overwritten_without_force() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("demo_ext");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("README.md"), "mine\n").expect("readme");
    std::fs::write(dir.join("notes.txt"), "unrelated\n").expect("notes");

    let out = run(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = text(&out.stderr);
    assert!(
        err.contains("README.md") && err.contains("--force"),
        "{err}"
    );
    // Nothing was written: not even the files that did not exist.
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "mine\n"
    );
    assert!(!dir.join("Cargo.toml").exists());

    let out = run(&dir, &["--force"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
    assert!(readme.starts_with("# demo_ext\n"), "{readme}");
    assert!(dir.join("Cargo.toml").is_file());
    // A file the scaffold does not generate is left alone.
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).unwrap(),
        "unrelated\n"
    );
}

#[test]
fn an_invalid_config_writes_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("out");
    let out = Command::new(env!("CARGO_BIN_EXE_quack_scaffold"))
        .arg("s3")
        .args(REQUIRED)
        .arg("--dir")
        .arg(&dir)
        .output()
        .expect("the binary runs");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(text(&out.stderr).contains("alias"), "{}", text(&out.stderr));
    assert!(!dir.exists());
}

#[test]
fn warnings_go_to_stderr_and_do_not_fail_the_run() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("demo_ext");
    let out = run(&dir, &["--license", "BUSL-1.1"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    assert!(err.starts_with("warning: license 'BUSL-1.1'"), "{err}");
    assert!(dir.join("LICENSE.TODO").is_file());
}

/// A symbolic link inside the project would make the write land outside it.
#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_way_is_refused_even_with_force() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("demo_ext");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::create_dir_all(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, dir.join("src")).expect("symlink");

    let out = run(&dir, &["--force"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        text(&out.stderr).contains("symbolic link"),
        "{}",
        text(&out.stderr)
    );
    assert!(!outside.join("lib.rs").exists());
    assert!(!dir.join("Cargo.toml").exists());
}

/// `std::env::args` panics on an argument that is not UTF-8 (HARDEN-4).
#[cfg(unix)]
#[test]
fn a_non_utf8_argument_is_an_error_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let out = Command::new(env!("CARGO_BIN_EXE_quack_scaffold"))
        .arg(std::ffi::OsStr::from_bytes(b"demo\xff"))
        .output()
        .expect("the binary runs");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        text(&out.stderr).contains("not valid UTF-8"),
        "{}",
        text(&out.stderr)
    );
}
