// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The `append_metadata` binary run as a user runs it: its exit status, what
//! it prints and the file it writes. The unit tests in
//! `src/bin/append_metadata/tests.rs` call its functions directly and never
//! reach `main`, `run` or the `--dump` output.

use std::path::PathBuf;
use std::process::{Command, Output};

/// The footer `DuckDB` reads from the end of an extension file.
const FOOTER: usize = 512;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quack_rs_append_metadata_cli_{}_{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_append_metadata"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_succeeds_and_describes_every_option() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "{out:?}");
    let help = text(&out.stderr);
    for option in ["--abi-type", "--platform", "--wasm", "--replace", "--dump"] {
        assert!(help.contains(option), "{option}: {help}");
    }
}

#[test]
fn a_bad_argument_fails_with_the_reason_and_a_pointer_to_help() {
    let out = run(&["--no-such-flag"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = text(&out.stderr);
    assert!(err.starts_with("error: "), "{err}");
    assert!(err.contains("--no-such-flag"), "{err}");
    assert!(err.contains("--help"), "{err}");
}

#[test]
fn a_missing_input_fails_without_writing_the_output() {
    let dir = scratch("missing");
    let output = dir.join("out.duckdb_extension");
    let input = dir.join("absent.so");
    let out = run(&[
        input.to_str().expect("utf-8"),
        output.to_str().expect("utf-8"),
        "--platform",
        "linux_amd64",
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(text(&out.stderr).contains("failed to read"), "{out:?}");
    assert!(!output.exists());
    std::fs::remove_dir_all(&dir).expect("clean up");
}

#[test]
fn stamping_appends_one_footer_and_dump_prints_its_fields() {
    let dir = scratch("stamp");
    let input = dir.join("lib.so");
    let output = dir.join("out.duckdb_extension");
    let library = b"\x7fELF not really a library".to_vec();
    std::fs::write(&input, &library).expect("seed");
    let out = run(&[
        input.to_str().expect("utf-8"),
        output.to_str().expect("utf-8"),
        "--platform",
        "osx_arm64",
        "--extension-version",
        "v9.8.7",
        "--dump",
    ]);
    assert!(out.status.success(), "{out:?}");
    let stdout = text(&out.stdout);
    assert!(stdout.contains("C_STRUCT for osx_arm64"), "{stdout}");
    for field in ["\"osx_arm64\"", "\"v9.8.7\"", "\"C_STRUCT\""] {
        assert!(stdout.contains(field), "{field}: {stdout}");
    }

    let written = std::fs::read(&output).expect("output");
    assert_eq!(written.len(), library.len() + FOOTER);
    assert_eq!(&written[..library.len()], library.as_slice());
    let footer = &written[library.len()..];
    assert!(
        footer.windows(9).any(|w| w == b"osx_arm64"),
        "platform in the footer"
    );
    assert!(
        footer.windows(6).any(|w| w == b"v9.8.7"),
        "version in the footer"
    );

    // Stamping the stamped file again is refused without `--replace` ...
    let again = dir.join("again.duckdb_extension");
    let out = run(&[
        output.to_str().expect("utf-8"),
        again.to_str().expect("utf-8"),
        "--platform",
        "osx_arm64",
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!again.exists());
    // ... and with it, the footer is replaced rather than appended.
    let out = run(&[
        output.to_str().expect("utf-8"),
        again.to_str().expect("utf-8"),
        "--platform",
        "linux_arm64",
        "--replace",
    ]);
    assert!(out.status.success(), "{out:?}");
    let replaced = std::fs::read(&again).expect("replaced");
    assert_eq!(replaced.len(), library.len() + FOOTER);
    assert_eq!(&replaced[..library.len()], library.as_slice());
    std::fs::remove_dir_all(&dir).expect("clean up");
}
