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
               \x20\x20version: not-a-version\n\
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
fn parse_kv_preserves_hash_in_quoted_values() {
    // Quoted values should not have inline comments stripped
    let result = parse_kv("key: \"value # not a comment\"", "key:");
    assert_eq!(result, Some("\"value # not a comment\""));
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
