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

/// `DuckDB` names an extension after its file and calls `<name>_init_c_api`.
/// A library exporting another name stamps fine and then fails at `LOAD` with
/// "undefined symbol", so the tool says so at stamp time — and still writes the
/// file, because stamping to a temporary name and renaming is ordinary.
#[test]
fn an_entry_point_that_does_not_match_the_file_name_is_warned_about() {
    let dir = scratch("entry");
    let input = dir.join("libother.so");
    let mut library = b"\x7fELF strtab\0".to_vec();
    library.extend_from_slice(b"other_init_c_api\0");
    std::fs::write(&input, &library).expect("seed");
    let stamp = |name: &str| {
        let output = dir.join(name);
        let out = run(&[
            input.to_str().expect("utf-8"),
            output.to_str().expect("utf-8"),
            "--platform",
            "linux_amd64",
        ]);
        assert!(out.status.success(), "{out:?}");
        assert!(output.exists(), "the file is written either way");
        text(&out.stderr)
    };

    // The name matches: nothing to say.
    let quiet = stamp("other.duckdb_extension");
    assert!(!quiet.contains("warning"), "{quiet}");

    // The name does not: the warning names both symbols and the right file.
    let loud = stamp("mine.duckdb_extension");
    assert!(loud.starts_with("warning: "), "{loud}");
    assert!(loud.contains("`other_init_c_api`"), "{loud}");
    assert!(loud.contains("`mine_init_c_api`"), "{loud}");
    assert!(loud.contains("other.duckdb_extension"), "{loud}");

    // No such symbol anywhere: say what to check.
    std::fs::write(&input, b"\x7fELF nothing exported").expect("seed");
    let none = stamp("none.duckdb_extension");
    assert!(none.contains("exports no `<name>_init_c_api`"), "{none}");

    // A C++ extension has other entry points, and is not told about this one.
    let output = dir.join("cpp.duckdb_extension");
    let out = run(&[
        input.to_str().expect("utf-8"),
        output.to_str().expect("utf-8"),
        "--abi-type",
        "CPP",
        "--duckdb-version",
        "v1.5.5",
        "--platform",
        "linux_amd64",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert!(!text(&out.stderr).contains("warning"), "{out:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}

/// Re-stamping to a path a host has loaded must not rewrite that file in
/// place: the host maps it, and its next call into the extension crashes.
#[cfg(unix)]
#[test]
fn restamping_replaces_the_output_instead_of_rewriting_it() {
    use std::io::Read as _;
    let dir = scratch("replace");
    let input = dir.join("lib.so");
    let output = dir.join("out.duckdb_extension");
    std::fs::write(&input, b"first build").expect("seed");
    let args = |extra: &[&'static str]| {
        let mut v = vec![
            input.to_str().expect("utf-8").to_owned(),
            output.to_str().expect("utf-8").to_owned(),
            "--platform".to_owned(),
            "linux_amd64".to_owned(),
        ];
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v
    };
    let stamp = |a: Vec<String>| {
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        let out = run(&a);
        assert!(out.status.success(), "{out:?}");
    };
    stamp(args(&[]));
    let mut held = std::fs::File::open(&output).expect("the host's handle");
    let before = std::fs::metadata(&output).expect("meta").len();

    std::fs::write(&input, b"second, longer build").expect("rebuild");
    stamp(args(&[]));

    let mut seen = Vec::new();
    held.read_to_end(&mut seen).expect("read the old file");
    assert_eq!(
        u64::try_from(seen.len()).expect("fits"),
        before,
        "the file the host holds is the old one, whole"
    );
    assert!(seen.starts_with(b"first build"));
    let now = std::fs::read(&output).expect("new output");
    assert!(now.starts_with(b"second, longer build"));
    let stray: Vec<_> = std::fs::read_dir(&dir)
        .expect("dir")
        .map(|e| e.expect("entry").file_name())
        .filter(|n| n.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
    std::fs::remove_dir_all(&dir).expect("clean up");
}
