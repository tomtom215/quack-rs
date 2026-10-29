// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

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
    assert_eq!(desc.version.as_deref(), Some("0.1.0"));
    assert_eq!(desc.language, "Rust");
    assert_eq!(desc.build, "cargo");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.requires_toolchains, "rust;python3");
    assert_eq!(desc.excluded_platforms, "");
    assert_eq!(desc.opt_in_platforms, "");
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

/// An unlisted licence is reported, but as a warning: the community build never
/// reads the field, and published extensions declare `BSL 1.1`, `GPL-3.0`
/// and free text, so a hard error rejected files `DuckDB` accepts. An empty
/// licence is still an error, checked at the end.
#[test]
fn unlisted_license_is_a_warning() {
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
               \x20\x20ref: 4f1c2e9\n";
    let desc = parse_description_yml(yml).unwrap();
    assert_eq!(desc.license, "FAKE-LICENSE");
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(desc.warnings[0].contains("license"), "{:?}", desc.warnings);
    assert!(
        desc.warnings[0].contains("FAKE-LICENSE"),
        "{:?}",
        desc.warnings
    );
    let missing = yml.replace("  license: FAKE-LICENSE\n", "");
    let err = parse_description_yml(&missing).unwrap_err();
    assert!(err.as_str().contains("extension.license"), "{err}");
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
    assert_eq!(desc.version.as_deref(), Some("690bfc5"));
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
    assert_eq!(desc.version.as_deref(), Some("1.2.3"));
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
    assert_eq!(desc.version.as_deref(), Some("0.1.0"));
    assert_eq!(desc.language, "Rust");
    assert_eq!(desc.build, "cargo");
    assert_eq!(desc.license, "MIT");
    assert_eq!(desc.requires_toolchains, "rust;python3");
    assert_eq!(desc.github, "j/r");
    assert_eq!(desc.git_ref, "main");
}

/// One-field variant of [`valid_yml`] for scalar-decoding checks.
fn with_description(line: &str) -> String {
    valid_yml().replace("description: Fast analytics for DuckDB.", line)
}

#[test]
fn quoted_values_are_unquoted_and_keep_a_hash_inside_the_quotes() {
    // Inside quotes a '#' is data, not a comment — and the quotes themselves
    // are syntax, so they come off.
    let desc =
        parse_description_yml(&with_description("description: \"value # not a comment\"")).unwrap();
    assert_eq!(desc.description, "value # not a comment");
    // Real description.yml files single-quote the version.
    let yml = valid_yml().replace("version: 0.1.0", "version: '2025120401'");
    assert_eq!(
        parse_description_yml(&yml).unwrap().version.as_deref(),
        Some("2025120401")
    );
}

/// Unbalanced or repeated quotes used to be passed through verbatim
/// (`"unbalanced` became the value `"unbalanced`). They are YAML syntax
/// errors — `yaml.safe_load` rejects all three — so they are errors here too,
/// rather than values with stray quote characters in them.
#[test]
fn malformed_quoting_is_an_error() {
    for line in [
        "description: \"unbalanced",
        "description: \"\"doubled\"\"",
        "description: \"",
    ] {
        assert!(
            parse_description_yml(&with_description(line)).is_err(),
            "{line}"
        );
    }
}

#[test]
fn an_inline_comment_is_stripped_from_an_unquoted_value() {
    let desc = parse_description_yml(&with_description("description: value # a comment")).unwrap();
    assert_eq!(desc.description, "value");
}

#[test]
fn an_unquoted_value_without_a_comment_is_kept_whole() {
    let desc = parse_description_yml(&with_description("description: plain_value")).unwrap();
    assert_eq!(desc.description, "plain_value");
}

// ─── Regression fixtures modelled on real published extensions ──────────────
//
// quack-rs's validator once rejected 36 of 43 published community extensions.
// Each fixture below reproduces one structural feature that caused a false
// rejection or a silently wrong parse, with the extension it was observed in
// named so the claim can be re-checked.

/// `docs.extended_description` is free-form prose in 319 of the 346 published
/// descriptors (community-extensions `5ae7df8`). A flat line scan treats a
/// `version:` or `license:` line inside that prose as a real field and silently
/// overwrites the extension's own metadata.
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
    assert_eq!(desc.version.as_deref(), Some("1.2.3"));
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
    assert_eq!(
        desc.version.as_deref(),
        Some("2025120401"),
        "quotes must be stripped"
    );
    assert_eq!(desc.name, "shellfs");
    // The `docs:` block comes first here, and maintainers are indented only two
    // spaces — both shapes real files use.
    assert_eq!(desc.maintainers, vec!["rustyconover".to_string()]);
}

/// Observed in `chsql`, `dns`, `sshfs` and ten others (13 descriptors at
/// community-extensions `5ae7df8`): a quoted `excluded_platforms` with a
/// trailing semicolon, and `windows_amd64_rtools`,
/// which is not in the distribution matrix but is excluded by 84 of the 346
/// published descriptors (community-extensions `5ae7df8`).
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
    // No `requires_toolchains`: only 143 of 346 published descriptors set it.
    assert_eq!(desc.requires_toolchains, "");
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
    assert_eq!(
        desc.version.as_deref(),
        Some("1.2.3"),
        "the body's version must not win"
    );
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
    assert_eq!(desc.git_ref_next, "");
}

/// Setting both spellings is ambiguous, so it is an error rather than one of
/// them silently winning.
#[test]
fn license_and_licence_together_are_rejected() {
    let yml = valid_yml().replace(
        "  license: MIT\n",
        "  license: MIT\n  licence: Apache-2.0\n",
    );
    let err = parse_description_yml(&yml).unwrap_err();
    assert!(err.as_str().contains("both"), "{}", err.as_str());
    assert!(
        err.as_str().contains("extension.licence"),
        "{}",
        err.as_str()
    );
}

/// An empty list entry (`-`, `- ""`, `- '  '`) is skipped, not an error and
/// not an empty name.
#[test]
fn empty_list_entries_are_skipped() {
    let yml = valid_yml()
        .replace(
            "    - Jane Doe\n",
            "    - Jane Doe\n    -\n    - \"\"\n    - '  '\n",
        )
        .replace(
            "  maintainers:\n",
            "  excluded_platforms:\n    - osx_amd64\n    -\n  maintainers:\n",
        );
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Jane Doe"]);
    assert_eq!(desc.excluded_platforms, "osx_amd64");
}

/// `repo.ref` is what the community build checks out: anything but a commit
/// hash is accepted, with a warning naming it.
#[test]
fn a_ref_that_is_not_a_commit_hash_is_a_warning() {
    let yml = |git_ref: &str| {
        format!(
            "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
             license: MIT\n  requires_toolchains: rust\n  maintainers:\n    - Jane\n\
             repo:\n  github: j/r\n  ref: {git_ref}\n"
        )
    };
    for not_a_hash in ["main", "v0.1.0", "4f1c2e", "4f1c2e9g"] {
        let desc = parse_description_yml(&yml(not_a_hash)).unwrap();
        assert_eq!(desc.warnings.len(), 1, "{not_a_hash}: {:?}", desc.warnings);
        assert!(
            desc.warnings[0].contains("not a commit hash") && desc.warnings[0].contains(not_a_hash),
            "{:?}",
            desc.warnings
        );
    }
    for hash in [
        "4f1c2e9",
        "4F1C2E9A7B3D5C8E1F0A2B4C6D8E0F1A3B5C7D9E",
        &"a".repeat(64),
    ] {
        let desc = parse_description_yml(&yml(hash)).unwrap();
        assert!(desc.warnings.is_empty(), "{hash}: {:?}", desc.warnings);
    }
    let too_long = "a".repeat(65);
    let desc = parse_description_yml(&yml(&too_long)).unwrap();
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
}

/// extension-ci-tools matches `;rust;` as an exact token, so `python3; rust`
/// installs no Rust toolchain; the check used to trim and accept it.
#[test]
fn a_padded_rust_toolchain_token_is_refused() {
    let yml = |toolchains: &str| {
        format!(
            "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
             license: MIT\n  requires_toolchains: \"{toolchains}\"\n  maintainers:\n    - Jane\n\
             repo:\n  github: j/r\n  ref: 4f1c2e9\n"
        )
    };
    let err = validate_rust_extension(&parse_description_yml(&yml("python3; rust")).unwrap())
        .expect_err("a space before rust");
    assert!(
        err.to_string()
            .contains("matches ';'-separated tokens exactly"),
        "{err}"
    );
    assert!(
        validate_rust_extension(&parse_description_yml(&yml("rust ;python3")).unwrap()).is_err()
    );
    assert!(validate_rust_extension(&parse_description_yml(&yml("python3;rust")).unwrap()).is_ok());
    let err = validate_rust_extension(&parse_description_yml(&yml("python3")).unwrap())
        .expect_err("no rust");
    assert!(err.to_string().contains("must include 'rust'"), "{err}");
}

/// `PyYAML` keeps the spaces inside a quoted scalar, so the community build sees
/// `" my_ext"`; the parser used to trim it to a valid name.
#[test]
fn a_quoted_value_keeps_its_spaces() {
    let yml = "extension:\n  name: \" my_ext\"\n  description: d\n  language: Rust\n  \
               build: cargo\n  license: MIT\n  maintainers:\n    - Jane\n\
               repo:\n  github: j/r\n  ref: 4f1c2e9\n";
    match parse_description_yml(yml) {
        Ok(desc) => assert_eq!(desc.name, " my_ext"),
        Err(e) => assert!(e.to_string().contains("extension name must start"), "{e}"),
    }
    assert!(crate::validate::description_yml::validate_description_yml_str(yml).is_err());
}

/// The community build removes a platform only when its exact name is listed
/// (`arch in excluded_arch_values` in `extension-ci-tools`), so a group name is
/// accepted, as published files write it, but excludes nothing: the warning
/// names the platforms that would.
#[test]
fn a_platform_group_name_is_accepted_with_a_warning_naming_its_members() {
    // A commit hash, so the only warnings are about platforms.
    let with = |field: &str, value: &str| {
        valid_yml().replace("ref: main", "ref: 4f1c2e9").replace(
            "  maintainers:\n",
            &format!("  {field}: \"{value}\"\n  maintainers:\n"),
        )
    };
    for (group, members) in [
        ("wasm", "wasm_eh;wasm_mvp;wasm_threads"),
        ("windows", "windows_amd64;windows_amd64_mingw;windows_arm64"),
        (
            "linux",
            "linux_amd64;linux_amd64_musl;linux_arm64;linux_arm64_musl",
        ),
        ("osx", "osx_amd64;osx_arm64"),
    ] {
        let desc = parse_description_yml(&with("excluded_platforms", &format!("{group};wasm_mvp")))
            .unwrap_or_else(|e| panic!("{group}: {e}"));
        let group_warnings: Vec<_> = desc
            .warnings
            .iter()
            .filter(|w| w.contains(&format!("'{group}' is a platform group")))
            .collect();
        assert_eq!(group_warnings.len(), 1, "{group}: {:?}", desc.warnings);
        assert!(
            group_warnings[0].contains(&format!("\"{members}\"")),
            "{:?}",
            group_warnings[0]
        );
    }
    // Exact names draw no warning, and a group is no more meaningful in
    // `opt_in_platforms`.
    let desc = parse_description_yml(&with("excluded_platforms", "wasm_mvp;wasm_eh")).unwrap();
    assert!(desc.warnings.is_empty(), "{:?}", desc.warnings);
    let desc = parse_description_yml(&with("opt_in_platforms", "wasm")).unwrap();
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(desc.warnings[0].contains("opt_in_platforms: 'wasm' is a platform group"));
}

/// `opt_in_platforms` is read, and each name the build would ignore is named:
/// the build consults the field only for platforms that are opt-in, and a
/// misspelt one is a silent omission.
#[test]
fn opt_in_platforms_are_read_and_the_names_the_build_ignores_are_warned_about() {
    let with = |value: &str| {
        valid_yml().replace("ref: main", "ref: 4f1c2e9").replace(
            "  maintainers:\n",
            &format!("  opt_in_platforms: \"{value}\"\n  maintainers:\n"),
        )
    };
    let desc = parse_description_yml(&with("osx_amd64;linux_arm64_musl")).unwrap();
    assert_eq!(desc.opt_in_platforms, "osx_amd64;linux_arm64_musl");
    assert!(desc.warnings.is_empty(), "{:?}", desc.warnings);

    let desc = parse_description_yml(&with("linux_amd64")).unwrap();
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(
        desc.warnings[0].contains("'linux_amd64' is not an opt-in platform"),
        "{:?}",
        desc.warnings
    );

    let desc = parse_description_yml(&with("osx_amd65;windows_arm64")).unwrap();
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(
        desc.warnings[0].contains("'osx_amd65' is not a recognized DuckDB build target"),
        "{:?}",
        desc.warnings
    );

    // A trailing ';' is an empty token and means nothing.
    let desc = parse_description_yml(&with("osx_amd64;")).unwrap();
    assert!(desc.warnings.is_empty(), "{:?}", desc.warnings);

    // The list form is joined, as for `excluded_platforms`.
    let yml = valid_yml().replace("ref: main", "ref: 4f1c2e9").replace(
        "  maintainers:\n",
        "  opt_in_platforms:\n    - osx_amd64\n    - windows_arm64\n  maintainers:\n",
    );
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.opt_in_platforms, "osx_amd64;windows_arm64");
    assert_eq!(desc.warnings.len(), 1, "{:?}", desc.warnings);
    assert!(desc.warnings[0].contains("opt_in_platforms is a YAML list"));
}
