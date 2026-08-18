// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::parser::parse_kv;
use super::*;

fn valid_yml() -> &'static str {
    "extension:\n\
     \x20\x20name: my_ext\n\
     \x20\x20description: Fast analytics for DuckDB.\n\
     \x20\x20version: 0.1.0\n\
     \x20\x20language: Rust\n\
     \x20\x20build: cargo\n\
     \x20\x20license: MIT\n\
     \x20\x20requires_toolchains: rust;python3\n\
     \x20\x20maintainers:\n\
     \x20\x20\x20\x20- Jane Doe\n\
     \n\
     repo:\n\
     \x20\x20github: janedoe/duckdb-my-ext\n\
     \x20\x20ref: main\n"
}

#[test]
fn valid_yml_parses_correctly() {
    let desc = parse_description_yml(valid_yml()).unwrap();
    assert_eq!(desc.name, "my_ext");
    assert_eq!(desc.description, "Fast analytics for DuckDB.");
    assert_eq!(desc.version, "0.1.0");
    assert_eq!(desc.language, "Rust");
    assert_eq!(desc.build, "cargo");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.requires_toolchains, "rust;python3");
    assert_eq!(desc.excluded_platforms, "");
    assert_eq!(desc.maintainers, vec!["Jane Doe"]);
    assert_eq!(desc.github, "janedoe/duckdb-my-ext");
    assert_eq!(desc.git_ref, "main");
}

#[test]
fn validate_description_yml_str_valid() {
    assert!(validate_description_yml_str(valid_yml()).is_ok());
}

#[test]
fn missing_name_rejected() {
    let yml = "extension:\n\
               \x20\x20description: Fast analytics.\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(
        err.as_str().contains("name"),
        "expected 'name' in: {}",
        err.as_str()
    );
}

#[test]
fn invalid_name_rejected() {
    let yml = "extension:\n\
               \x20\x20name: Bad Name!\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    assert!(parse_description_yml(yml).is_err());
}

#[test]
fn missing_description_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("description"));
}

#[test]
fn invalid_version_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 1.0.0 with a space\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("version"));
}

#[test]
fn invalid_license_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: FAKE-LICENSE\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("license"));
}

#[test]
fn invalid_excluded_platforms_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20excluded_platforms: \"invalid_platform\"\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("excluded_platforms"));
}

#[test]
fn excluded_platforms_wasm_accepted() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20excluded_platforms: \"wasm_mvp;wasm_eh;wasm_threads\"\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.excluded_platforms, "wasm_mvp;wasm_eh;wasm_threads");
}

#[test]
fn missing_maintainers_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("maintainer"));
}

#[test]
fn multiple_maintainers_parsed() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Alice\n\
               \x20\x20\x20\x20- Bob\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Alice", "Bob"]);
}

#[test]
fn missing_github_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("github"));
}

#[test]
fn invalid_github_no_slash_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: noslash\n\
               \x20\x20ref: main\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("owner/repo"));
}

#[test]
fn missing_ref_rejected() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n";
    let err = parse_description_yml(yml).unwrap_err();
    assert!(err.as_str().contains("ref"));
}

#[test]
fn validate_rust_extension_valid() {
    let desc = parse_description_yml(valid_yml()).unwrap();
    assert!(validate_rust_extension(&desc).is_ok());
}

#[test]
fn validate_rust_extension_wrong_language() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Go\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    let err = validate_rust_extension(&desc).unwrap_err();
    assert!(err.as_str().contains("language"));
}

#[test]
fn validate_rust_extension_wrong_build() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cmake\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    let err = validate_rust_extension(&desc).unwrap_err();
    assert!(err.as_str().contains("build"));
}

#[test]
fn validate_rust_extension_missing_rust_toolchain() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    let err = validate_rust_extension(&desc).unwrap_err();
    assert!(err.as_str().contains("requires_toolchains"));
}

#[test]
fn unstable_git_hash_version_accepted() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 690bfc5\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.version, "690bfc5");
}

#[test]
fn stable_semver_version_accepted() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 1.2.3\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.version, "1.2.3");
}

#[test]
fn excluded_platforms_quoted_stripped() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20excluded_platforms: \"wasm_mvp;wasm_eh\"\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).unwrap();
    // Quotes must be stripped from the value
    assert_eq!(desc.excluded_platforms, "wasm_mvp;wasm_eh");
    assert!(!desc.excluded_platforms.starts_with('"'));
}

#[test]
fn comments_are_ignored() {
    let yml = "# This is a full description.yml with comments\n\
               extension:\n\
               \x20\x20# Extension metadata\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: d\n\
               \x20\x20version: 0.1.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20requires_toolchains: rust;python3\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane # primary maintainer\n\
               \n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main # default branch\n";
    // Should parse without error despite full-line and inline comments
    let desc = parse_description_yml(yml).expect("parsing failed");
    // Inline comments must be stripped from values
    assert_eq!(desc.git_ref, "main", "inline comment not stripped from ref");
    // Maintainer inline comments must also be stripped
    assert_eq!(
        desc.maintainers,
        vec!["Jane"],
        "inline comment not stripped from maintainer"
    );
}

#[test]
fn inline_comments_stripped_from_values() {
    let yml = "extension:\n\
               \x20\x20name: my_ext\n\
               \x20\x20description: Fast analytics # for DuckDB\n\
               \x20\x20version: 0.1.0 # initial release\n\
               \x20\x20language: Rust # not C++\n\
               \x20\x20build: cargo # build system\n\
               \x20\x20license: MIT # open source\n\
               \x20\x20requires_toolchains: rust;python3 # both needed\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r # github repo\n\
               \x20\x20ref: main # default branch\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.description, "Fast analytics");
    assert_eq!(desc.version, "0.1.0");
    assert_eq!(desc.language, "Rust");
    assert_eq!(desc.build, "cargo");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.requires_toolchains, "rust;python3");
    assert_eq!(desc.github, "j/r");
    assert_eq!(desc.git_ref, "main");
}

#[test]
fn parse_kv_unquotes_and_keeps_hash_inside_quotes() {
    // Inside quotes a '#' is data, not a comment — and the quotes themselves
    // are syntax, so they come off.
    assert_eq!(
        parse_kv("key: \"value # not a comment\"", "key:"),
        Some("value # not a comment")
    );
    // Real description.yml files single-quote the version.
    assert_eq!(
        parse_kv("  version: '2025120401'", "  version:"),
        Some("2025120401")
    );
    // Unbalanced or repeated quotes are left alone rather than chewed through
    // the way `trim_matches` would.
    assert_eq!(parse_kv("key: \"unbalanced", "key:"), Some("\"unbalanced"));
    assert_eq!(
        parse_kv("key: \"\"doubled\"\"", "key:"),
        Some("\"doubled\"")
    );
    assert_eq!(parse_kv("key: \"", "key:"), Some("\""));
}

#[test]
fn parse_kv_strips_inline_comment_from_unquoted() {
    let result = parse_kv("key: value # a comment", "key:");
    assert_eq!(result, Some("value"));
}

#[test]
fn parse_kv_no_comment() {
    let result = parse_kv("key: plain_value", "key:");
    assert_eq!(result, Some("plain_value"));
}

// ─── Regression fixtures modelled on real published extensions ──────────────
//
// quack-rs's validator once rejected 36 of 43 published community extensions.
// Each fixture below reproduces one structural feature that caused a false
// rejection or a silently wrong parse, with the extension it was observed in
// named so the claim can be re-checked.

/// `docs.extended_description` is free-form prose in 42 of 43 published
/// extensions. A flat line scan treats a `version:` or `license:` line inside
/// that prose as a real field and silently overwrites the extension's own
/// metadata.
#[test]
fn prose_in_the_docs_section_is_not_metadata() {
    let yml = "extension:\n\
               \x20\x20name: probe\n\
               \x20\x20description: A real extension\n\
               \x20\x20version: 1.2.3\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: abc1234\n\
               docs:\n\
               \x20\x20extended_description: |\n\
               \x20\x20\x20\x20Usage notes.\n\
               \n\
               \x20\x20\x20\x20name: not_the_extension_name\n\
               \x20\x20\x20\x20version: 9.9.9\n\
               \x20\x20\x20\x20license: FAKE-LICENSE\n\
               \x20\x20\x20\x20ref: not_the_ref\n";
    let desc = parse_description_yml(yml).expect("prose must not invalidate the file");
    assert_eq!(desc.name, "probe");
    assert_eq!(desc.version, "1.2.3");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.git_ref, "abc1234");
}

/// Observed in `shellfs`, `crypto`, `lindel` and eight others: the version is a
/// single-quoted date-based build id. Quotes must come off, and the format must
/// not be second-guessed — `DuckDB` documents no version format.
#[test]
fn single_quoted_date_versions_parse() {
    let yml = "docs:\n\
               \x20\x20extended_description: For more information, see the docs.\n\
               extension:\n\
               \x20\x20build: cmake\n\
               \x20\x20description: Allow shell commands to be used for input and output\n\
               \x20\x20excluded_platforms: wasm_mvp;wasm_eh;wasm_threads\n\
               \x20\x20language: C++\n\
               \x20\x20license: MIT\n\
               \x20\x20maintainers:\n\
               \x20\x20- rustyconover\n\
               \x20\x20name: shellfs\n\
               \x20\x20requires_toolchains: python3\n\
               \x20\x20version: '2025120401'\n\
               repo:\n\
               \x20\x20github: rustyconover/duckdb-shellfs-extension\n\
               \x20\x20ref: 6e2eb0f\n";
    let desc = parse_description_yml(yml).expect("a real published file must parse");
    assert_eq!(desc.version, "2025120401", "quotes must be stripped");
    assert_eq!(desc.name, "shellfs");
    // The `docs:` block comes first here, and maintainers are indented only two
    // spaces — both shapes real files use.
    assert_eq!(desc.maintainers, vec!["rustyconover".to_string()]);
}

/// Observed in `chsql`, `substrait`, `magic` and eleven others: a quoted
/// `excluded_platforms` with a trailing semicolon, and `windows_amd64_rtools`,
/// which is not in the distribution matrix but is excluded by 14 of 43
/// published extensions.
#[test]
fn quoted_excluded_platforms_with_trailing_semicolon_parse() {
    let yml = "extension:\n\
               \x20\x20name: chsql\n\
               \x20\x20description: ClickHouse SQL macros\n\
               \x20\x20version: 1.4.0\n\
               \x20\x20language: SQL & C++\n\
               \x20\x20build: cmake\n\
               \x20\x20license: MIT\n\
               \x20\x20excluded_platforms: \"windows_amd64_rtools;windows_amd64_mingw;windows_amd64;\"\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- lmangani\n\
               repo:\n\
               \x20\x20github: quackscience/duckdb-extension-clickhouse-sql\n\
               \x20\x20ref: 60f5e13\n";
    let desc = parse_description_yml(yml).expect("a real published file must parse");
    assert_eq!(
        desc.excluded_platforms,
        "windows_amd64_rtools;windows_amd64_mingw;windows_amd64;"
    );
    // No `requires_toolchains`: only 14 of 43 published extensions set it.
    assert!(desc.requires_toolchains.is_empty());
}

/// A `key: |` body indented under `extension:` is the field's *value*, not a
/// sequence of mapping lines — so it must be captured, and the mapping-looking
/// lines inside it must not be read as fields.
#[test]
fn block_scalars_inside_the_extension_section_are_captured_not_scanned() {
    let yml = "extension:\n\
               \x20\x20name: probe\n\
               \x20\x20description: |\n\
               \x20\x20\x20\x20A multi-line description.\n\
               \x20\x20\x20\x20version: 9.9.9\n\
               \x20\x20version: 1.2.3\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n";
    let desc = parse_description_yml(yml).expect("parse");
    assert_eq!(desc.version, "1.2.3", "the body's version must not win");
    assert_eq!(
        desc.description, "A multi-line description.\nversion: 9.9.9",
        "a literal block keeps its line breaks and is the field's value"
    );
}

/// Folded blocks (`>`) join onto one line; literal blocks (`|`) keep breaks.
#[test]
fn folded_and_literal_block_scalars_join_differently() {
    let make = |indicator: &str| {
        format!(
            "extension:\n\
             \x20\x20name: probe\n\
             \x20\x20description: {indicator}\n\
             \x20\x20\x20\x20one\n\
             \x20\x20\x20\x20two\n\
             \x20\x20version: 1.0.0\n\
             \x20\x20language: Rust\n\
             \x20\x20build: cargo\n\
             \x20\x20license: MIT\n\
             \x20\x20maintainers:\n\
             \x20\x20\x20\x20- Jane\n\
             repo:\n\
             \x20\x20github: j/r\n\
             \x20\x20ref: main\n"
        )
    };
    assert_eq!(
        parse_description_yml(&make("|"))
            .expect("literal")
            .description,
        "one\ntwo"
    );
    assert_eq!(
        parse_description_yml(&make(">"))
            .expect("folded")
            .description,
        "one two"
    );
    // A chomping indicator does not change the trimmed result.
    assert_eq!(
        parse_description_yml(&make("|-"))
            .expect("chomped")
            .description,
        "one\ntwo"
    );
}

/// A block scalar that runs to the end of the file still has to be assigned.
#[test]
fn a_block_scalar_at_end_of_file_is_captured() {
    let yml = "repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: main\n\
               extension:\n\
               \x20\x20name: probe\n\
               \x20\x20version: 1.0.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               \x20\x20description: |\n\
               \x20\x20\x20\x20trailing block\n";
    let desc = parse_description_yml(yml).expect("parse");
    assert_eq!(desc.description, "trailing block");
}

/// `ref_next` is a documented field — `DuckDB`'s development docs describe using
/// it while a new release is being prepared — and was silently dropped.
#[test]
fn ref_next_is_parsed_and_does_not_shadow_ref() {
    let yml = "extension:\n\
               \x20\x20name: probe\n\
               \x20\x20description: d\n\
               \x20\x20version: 1.0.0\n\
               \x20\x20language: Rust\n\
               \x20\x20build: cargo\n\
               \x20\x20license: MIT\n\
               \x20\x20maintainers:\n\
               \x20\x20\x20\x20- Jane\n\
               repo:\n\
               \x20\x20github: j/r\n\
               \x20\x20ref: e5ed59b6ccf915c65e17eb6286b9a64f3ab09f59\n\
               \x20\x20ref_next: c8941c92ec103f7825eb88207c04512f8a714b23\n";
    let desc = parse_description_yml(yml).expect("parse");
    assert_eq!(desc.git_ref, "e5ed59b6ccf915c65e17eb6286b9a64f3ab09f59");
    assert_eq!(
        desc.git_ref_next,
        "c8941c92ec103f7825eb88207c04512f8a714b23"
    );

    // Absent is empty, not an error — it is optional.
    let without = yml
        .lines()
        .filter(|l| !l.contains("ref_next"))
        .collect::<Vec<_>>()
        .join("\n");
    let desc = parse_description_yml(&without).expect("parse");
    assert_eq!(desc.git_ref, "e5ed59b6ccf915c65e17eb6286b9a64f3ab09f59");
    assert!(desc.git_ref_next.is_empty());
}
