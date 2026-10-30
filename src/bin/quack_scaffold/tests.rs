// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use std::ffi::OsString;
use std::path::PathBuf;

use super::*;
use cli::Args;

const REQUIRED: &str = "--description Demo --maintainer Jane --github-repo jane/demo";

fn argv(s: &str) -> Vec<OsString> {
    s.split_whitespace().map(OsString::from).collect()
}

fn parse_ok(s: &str) -> Args {
    match cli::parse(&argv(s)) {
        Ok(Command::Run(args)) => *args,
        other => panic!("{s}: {other:?}"),
    }
}

fn parse_err(s: &str) -> String {
    match cli::parse(&argv(s)) {
        Err(e) => e,
        other => panic!("{s} must be rejected, got {other:?}"),
    }
}

#[test]
fn defaults_are_the_library_defaults() {
    let args = parse_ok(&format!("demo {REQUIRED}"));
    let defaults = ScaffoldConfig::default();
    let config = &args.config;
    assert_eq!(config.name, "demo");
    assert_eq!(config.description, "Demo");
    assert_eq!(config.maintainer, "Jane");
    assert_eq!(config.github_repo, "jane/demo");
    assert_eq!(config.version, defaults.version);
    assert_eq!(config.license, defaults.license);
    assert_eq!(config.excluded_platforms, defaults.excluded_platforms);
    assert_eq!(config.target_duckdb_version, defaults.target_duckdb_version);
    assert!(!config.use_unstable_c_api);
    assert_eq!(config.git_ref, defaults.git_ref);
    assert_eq!(args.dir, PathBuf::from("demo"));
    assert!(!args.force);
}

#[test]
fn every_config_field_has_a_flag() {
    let args = parse_ok(&format!(
        "demo {REQUIRED} --version 1.2.3 --license=Apache-2.0 \
         --excluded-platforms wasm_mvp,windows_amd64_mingw --target-duckdb-version v1.5.5 \
         --unstable-c-api --git-ref 0123abc --dir out/x --force"
    ));
    let config = &args.config;
    assert_eq!(config.version, "1.2.3");
    assert_eq!(config.license, "Apache-2.0");
    assert_eq!(
        config.excluded_platforms,
        ["wasm_mvp", "windows_amd64_mingw"]
    );
    assert_eq!(config.target_duckdb_version, "v1.5.5");
    assert!(config.use_unstable_c_api);
    assert_eq!(config.git_ref, "0123abc");
    assert_eq!(args.dir, PathBuf::from("out/x"));
    assert!(args.force);
}

#[test]
fn excluded_platforms_take_either_separator_and_an_empty_list() {
    let args = parse_ok(&format!(
        "demo {REQUIRED} --excluded-platforms=wasm_mvp;;wasm_eh,"
    ));
    assert_eq!(args.config.excluded_platforms, ["wasm_mvp", "wasm_eh"]);
    let args = parse_ok(&format!("demo {REQUIRED} --excluded-platforms="));
    assert!(args.config.excluded_platforms.is_empty());
}

#[test]
fn values_with_spaces_and_dashes_survive() {
    let mut raw = argv("demo --maintainer Jane --github-repo jane/demo");
    raw.push("--description".into());
    raw.push("Fast analytics: sums, counts".into());
    raw.push("--license=-weird".into());
    let Ok(Command::Run(args)) = cli::parse(&raw) else {
        panic!("parse failed");
    };
    assert_eq!(args.config.description, "Fast analytics: sums, counts");
    assert_eq!(args.config.license, "-weird");
}

#[test]
fn help_wins() {
    assert!(matches!(cli::parse(&argv("--help")), Ok(Command::Help)));
    assert!(matches!(cli::parse(&argv("demo -h")), Ok(Command::Help)));
}

#[test]
fn bad_command_lines_are_refused_with_the_reason() {
    for (line, reason) in [
        ("", "<name>"),
        (
            "demo --maintainer Jane --github-repo jane/demo",
            "--description is required",
        ),
        (
            "demo --description D --github-repo jane/demo",
            "--maintainer is required",
        ),
        (
            "demo --description D --maintainer Jane",
            "--github-repo is required",
        ),
        ("demo other --description D", "expected one <name>"),
        ("demo --frobnicate", "unknown flag: --frobnicate"),
        ("demo --license", "--license requires a value"),
        ("demo --license --force", "requires a value, got the flag"),
        ("demo --version 1 --version 2", "given more than once"),
        ("demo --force=yes", "takes no value"),
        ("demo --dir", "--dir requires a value"),
        ("demo --dir a --dir b", "--dir given more than once"),
    ] {
        let err = parse_err(line);
        assert!(err.contains(reason), "{line:?}: {err}");
    }
}

/// HARDEN-4's lesson for this binary: `std::env::args` panics on a non-UTF-8
/// argument. Only `--dir PATH` can be one.
#[cfg(unix)]
#[test]
fn a_non_utf8_argument_is_an_error_and_a_non_utf8_dir_is_a_path() {
    use std::os::unix::ffi::OsStringExt;
    let bad = OsString::from_vec(vec![b'd', 0xff]);
    let err = match cli::parse(std::slice::from_ref(&bad)) {
        Err(e) => e,
        other => panic!("{other:?}"),
    };
    assert!(err.contains("not valid UTF-8"), "{err}");

    let mut raw = argv(&format!("demo {REQUIRED} --dir"));
    raw.push(bad.clone());
    let Ok(Command::Run(args)) = cli::parse(&raw) else {
        panic!("parse failed");
    };
    assert_eq!(args.dir, PathBuf::from(bad));
}

#[test]
fn shell_words_are_quoted_only_when_needed() {
    assert_eq!(shell_word("my_ext"), "my_ext");
    assert_eq!(shell_word("../out/my-ext"), "../out/my-ext");
    assert_eq!(shell_word("my ext"), "'my ext'");
    assert_eq!(shell_word("it's"), "'it'\\''s'");
    assert_eq!(shell_word(""), "''");
}

#[test]
fn next_steps_name_the_submodule_branch_and_the_stamp() {
    let args = parse_ok(&format!("demo {REQUIRED} --version 0.3.0"));
    let scaffold = generate_scaffold_with_warnings(&args.config).unwrap();
    let text = next_steps(&args.config, &args.dir, &scaffold.files);
    assert!(text.contains(
        "git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git \
         extension-ci-tools"
    ), "{text}");
    let library = format!(
        "target/release/{}demo{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    assert!(
        text.contains(&format!(
            "append_metadata {library} build/release/demo.duckdb_extension --abi-type C_STRUCT \
         --extension-version v0.3.0 --duckdb-version v1.2.0"
        )),
        "{text}"
    );

    let args = parse_ok(&format!(
        "demo {REQUIRED} --unstable-c-api --target-duckdb-version v1.5.5"
    ));
    let scaffold = generate_scaffold_with_warnings(&args.config).unwrap();
    let text = next_steps(&args.config, &args.dir, &scaffold.files);
    assert!(text.contains("git submodule add -b v1.5.5 "), "{text}");
    assert!(
        text.contains(
            "--abi-type C_STRUCT_UNSTABLE --extension-version v0.1.0 --duckdb-version v1.5.5"
        ),
        "{text}"
    );
}
