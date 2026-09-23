// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Regression tests for YAML the hand-rolled reader used to mis-read: each
//! input here is valid YAML that `yaml.safe_load` (what the community build
//! uses) reads differently from how the old line scanner did.

use super::*;

/// A complete, valid document with `{extra}` spliced into the `extension:`
/// section just before `maintainers:`.
fn doc(extra: &str) -> String {
    format!(
        "extension:\n  name: my_ext\n  description: d\n  version: 1.0.0\n  language: Rust\n  \
         build: cargo\n  license: MIT\n{extra}  maintainers:\n    - Jane\n\nrepo:\n  \
         github: a/b\n  ref: main\n"
    )
}

#[test]
fn a_quoted_value_followed_by_a_comment_is_unquoted() {
    let yml = doc("")
        .replace("description: d", "description: \"Fast\" # note")
        .replace("ref: main", "ref: 'abc123'   # pinned");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.description, "Fast");
    assert_eq!(desc.git_ref, "abc123");
}

#[test]
fn a_hash_inside_a_quoted_maintainer_is_not_a_comment() {
    let yml = doc("").replace("    - Jane\n", "    - Jane # lead\n    - \"Bob # x\"\n");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Jane", "Bob # x"]);
}

#[test]
fn a_hash_without_preceding_space_is_part_of_a_plain_value() {
    let yml = doc("").replace("description: d", "description: C# and F#1 # trailing");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.description, "C# and F#1");
}

/// The published `mssql` extension writes its exclusions as a block sequence.
/// It used to be dropped without a word — and never validated.
#[test]
fn a_block_sequence_of_excluded_platforms_is_read_and_validated() {
    let yml = doc("  excluded_platforms:\n    - \"osx_amd64\"\n    - wasm_mvp # no wasm\n");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.excluded_platforms, "osx_amd64;wasm_mvp");

    let yml = doc("  excluded_platforms:\n    - osx_amd64\n    - freebsd_amd64\n");
    let err = parse_description_yml(&yml).unwrap_err();
    assert!(err.as_str().contains("freebsd_amd64"), "{err}");
}

#[test]
fn a_flow_sequence_of_maintainers_is_read() {
    let yml = doc("").replace(
        "  maintainers:\n    - Jane\n",
        "  maintainers: [Jane, \"Bob, Jr.\"]\n",
    );
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Jane", "Bob, Jr."]);
}

#[test]
fn a_comment_after_the_maintainers_key_is_ignored() {
    let yml = doc("").replace("  maintainers:\n", "  maintainers: # people\n");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Jane"]);
}

#[test]
fn a_sequence_indented_level_with_its_key_is_read() {
    // `key:\n- item` at the key's own indentation is ordinary YAML, and 24 of
    // the published files write their maintainers that way.
    let yml = doc("").replace("    - Jane\n", "  - Jane\n  - Bob\n");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.maintainers, vec!["Jane", "Bob"]);
}

/// A nested mapping's keys belong to the nested mapping; they used to
/// overwrite the extension's own `name` and `version`.
#[test]
fn keys_of_a_nested_mapping_do_not_overwrite_the_sections_own_keys() {
    let yml = doc("  extra:\n    name: Other_Name\n    version: 9 9\n");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.name, "my_ext");
    let yml = doc("").replace(
        "  ref: main\n",
        "  ref: main\n  nested:\n    github: evil/repo\n",
    );
    assert_eq!(parse_description_yml(&yml).unwrap().github, "a/b");
}

#[test]
fn a_utf8_byte_order_mark_is_ignored() {
    let yml = format!("\u{feff}{}", doc(""));
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.name, "my_ext");
}

#[test]
fn a_multi_line_plain_scalar_is_folded() {
    let yml = doc("").replace("description: d", "description: first part\n    second part");
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.description, "first part second part");
}

/// YAML requires unique keys; `yaml.safe_load` silently keeps the last one,
/// so a duplicate is almost always a mistake worth reporting.
#[test]
fn a_duplicate_key_is_rejected() {
    let yml = doc("  name: my_other_ext\n");
    let err = parse_description_yml(&yml).unwrap_err();
    assert!(err.as_str().contains("duplicate"), "{err}");
    assert!(err.as_str().contains("extension.name"), "{err}");
}

#[test]
fn a_known_field_given_a_mapping_is_rejected() {
    let yml = doc("").replace("  license: MIT\n", "  license:\n    id: MIT\n");
    let err = parse_description_yml(&yml).unwrap_err();
    assert!(err.as_str().contains("extension.license"), "{err}");
}

#[test]
fn double_quoted_escapes_are_decoded() {
    let yml = doc("").replace("description: d", r#"description: "tab\there \"q\" \u00e9""#);
    let desc = parse_description_yml(&yml).unwrap();
    assert_eq!(desc.description, "tab\there \"q\" \u{e9}");
    let yml = doc("").replace("description: d", "description: 'it''s'");
    assert_eq!(parse_description_yml(&yml).unwrap().description, "it's");
}

#[test]
fn an_unterminated_quote_is_an_error_not_a_value() {
    let yml = doc("").replace("description: d", "description: \"open");
    let err = parse_description_yml(&yml).unwrap_err();
    assert!(err.as_str().contains("quote"), "{err}");
}

#[test]
fn text_after_a_closing_quote_is_an_error() {
    let yml = doc("").replace("description: d", "description: \"a\" b");
    assert!(parse_description_yml(&yml).is_err());
}

#[test]
fn an_extension_key_at_column_zero_in_prose_does_not_start_a_section() {
    // A literal block in `docs:` ends at the first line indented no further
    // than its key, so a following `extension:` really is the section.
    let yml = format!(
        "docs:\n  hello_world: |\n    extension:\n      name: x\n{}",
        doc("")
    );
    assert_eq!(parse_description_yml(&yml).unwrap().name, "my_ext");
}
