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

// ─── The reader itself ──────────────────────────────────────────────────────
//
// These call the YAML reader directly, so each can pin down one value — its
// exact decoding, or that it is a mapping, a sequence or null — without the
// field checks of `parse_description_yml` in between.

/// The value of `key` in an `extension:` section whose body is `body`.
fn read(body: &str, key: &str) -> Result<yaml::Value, String> {
    let sections = yaml::read_sections(&format!("extension:\n{body}"), &["extension"])?;
    sections
        .into_iter()
        .flat_map(|s| s.entries)
        .find(|e| e.key == key)
        .map(|e| e.value)
        .ok_or_else(|| format!("no '{key}' in {body:?}"))
}

fn scalar(text: &str) -> yaml::Value {
    yaml::Value::Scalar(text.to_string())
}

#[test]
fn a_leading_document_marker_is_accepted_but_a_second_document_is_not() {
    let sections = yaml::read_sections("---\nextension:\n  a: 1\n", &["extension"]).unwrap();
    assert_eq!(sections[0].entries[0].value, scalar("1"));
    for yml in [
        "extension:\n  a: 1\n---\nrepo:\n  b: 2\n",
        "extension:\n  a: 1\n...\n",
    ] {
        let err = yaml::read_sections(yml, &["extension"]).unwrap_err();
        assert!(err.contains("single YAML document"), "{yml:?}: {err}");
    }
}

#[test]
fn errors_name_the_line_they_are_on() {
    let err = yaml::read_sections("extension:\n  a: 1\n  a: 2\n", &["extension"]).unwrap_err();
    assert!(err.starts_with("line 3:"), "{err}");
    assert!(err.contains("first on line 2"), "{err}");
}

#[test]
fn a_tab_used_for_indentation_is_rejected() {
    let err = yaml::read_sections("extension:\n\tname: x\n", &["extension"]).unwrap_err();
    assert!(err.contains("tab"), "{err}");
}

#[test]
fn indented_text_before_the_first_key_is_rejected_but_an_indented_comment_is_not() {
    let err = yaml::read_sections("  stray: x\nextension:\n  a: 1\n", &["extension"]).unwrap_err();
    assert!(err.contains("before any top-level key"), "{err}");
    assert!(yaml::read_sections("  # note\nextension:\n  a: 1\n", &["extension"]).is_ok());
}

/// A `#` line indented past the key is inside the block scalar, so it is text.
#[test]
fn a_comment_indented_inside_a_block_scalar_is_part_of_it() {
    let body = "  description: |\n    text\n    # kept\n  name: x\n";
    assert_eq!(read(body, "description").unwrap(), scalar("text\n# kept"));
    assert_eq!(read(body, "name").unwrap(), scalar("x"));
}

/// A comment (and a blank line) level with the keys, followed by the next
/// key, ends the block scalar rather than being read as badly indented content.
#[test]
fn a_comment_at_key_level_after_a_block_scalar_ends_it() {
    let body = "  description: |\n    text\n  # about name\n\n  name: x\n";
    assert_eq!(read(body, "description").unwrap(), scalar("text"));
    assert_eq!(read(body, "name").unwrap(), scalar("x"));
}

#[test]
fn a_plain_value_starting_on_the_next_line_is_a_scalar() {
    let body = "  description:\n    on the next line\n";
    assert_eq!(
        read(body, "description").unwrap(),
        scalar("on the next line")
    );
}

#[test]
fn a_flow_mapping_value_is_a_mapping() {
    assert_eq!(
        read("  license: {id: MIT}\n", "license").unwrap(),
        yaml::Value::Mapping
    );
}

#[test]
fn anchors_aliases_and_tags_are_rejected() {
    for value in ["&a x", "*a", "!!str x"] {
        let err = read(&format!("  name: {value}\n"), "name").unwrap_err();
        assert!(err.contains("anchors, aliases and tags"), "{value}: {err}");
    }
}

#[test]
fn a_sequence_line_without_a_dash_is_rejected() {
    let err = read("  maintainers:\n    - Jane\n    Bob\n", "maintainers").unwrap_err();
    assert!(err.contains("expected a `- item`"), "{err}");
}

/// A sequence item continues on deeper lines — a folded plain scalar, or a
/// block scalar with a blank line in it — up to the next `- ` marker.
#[test]
fn a_sequence_item_continues_on_deeper_lines() {
    let body = "  k:\n    - first\n      second\n    - |\n      one\n\n      two\n    - x\n";
    assert_eq!(
        read(body, "k").unwrap(),
        yaml::Value::Seq(vec![
            scalar("first second"),
            scalar("one\n\ntwo"),
            scalar("x")
        ])
    );
}

/// `- key: value` is a mapping item; a quoted item containing `: ` is not.
#[test]
fn a_mapping_item_in_a_sequence_is_a_mapping() {
    let body = "  k:\n    - name: Jane\n    - \"Bob: Jr.\"\n";
    assert_eq!(
        read(body, "k").unwrap(),
        yaml::Value::Seq(vec![yaml::Value::Mapping, scalar("Bob: Jr.")])
    );
}

/// A line break in a double-quoted scalar folds to a space, and a blank line
/// to a newline.
#[test]
fn a_multi_line_double_quoted_value_folds_its_line_breaks() {
    let body = "  description: \"first\n    second\n\n    third\"\n";
    assert_eq!(
        read(body, "description").unwrap(),
        scalar("first second\nthird")
    );
}

#[test]
fn a_comment_ends_a_plain_value() {
    for body in [
        "  description: value # c\n    more\n",
        "  description: a\n    b # c\n    d\n",
    ] {
        let err = read(body, "description").unwrap_err();
        assert!(err.contains("text after a comment"), "{body:?}: {err}");
    }
    // Without a comment, every continuation line is folded in.
    let body = "  description: a\n    b\n    c\n";
    assert_eq!(read(body, "description").unwrap(), scalar("a b c"));
}

#[test]
fn null_spellings_read_as_null() {
    for value in ["~", "null", "Null", "NULL"] {
        let body = format!("  version: {value}\n");
        assert_eq!(
            read(&body, "version").unwrap(),
            yaml::Value::Null,
            "{value}"
        );
    }
    assert_eq!(
        read("  version: nulls\n", "version").unwrap(),
        scalar("nulls")
    );
}

#[test]
fn an_empty_quoted_flow_item_is_kept() {
    assert_eq!(
        read("  k: [\"\", a]\n", "k").unwrap(),
        yaml::Value::Seq(vec![scalar(""), scalar("a")])
    );
}

#[test]
fn text_between_flow_items_without_a_comma_is_rejected() {
    let err = read("  k: [\"a\" b]\n", "k").unwrap_err();
    assert!(err.contains("expected ','"), "{err}");
}

#[test]
fn hex_escapes_are_decoded() {
    let body = "  description: \"\\x41\\u00e9\\U0001F600\"\n";
    assert_eq!(
        read(body, "description").unwrap(),
        scalar("A\u{e9}\u{1f600}")
    );
}

/// A quoted or empty key is reported as unsupported rather than read with its
/// quotes, or as an empty key.
#[test]
fn a_quoted_or_empty_key_is_rejected() {
    for body in ["  \"name\": x\n", "  : x\n"] {
        let err = read(body, "name").unwrap_err();
        assert!(err.contains("expected `key: value`"), "{body:?}: {err}");
    }
}

/// Only a colon followed by a space or the end of the line separates a key.
#[test]
fn a_colon_inside_a_key_or_a_url_is_not_the_key_separator() {
    let body = "  a:b: c\n  description: see\n    https://example.com/x for docs\n";
    assert_eq!(read(body, "a:b").unwrap(), scalar("c"));
    assert_eq!(
        read(body, "description").unwrap(),
        scalar("see https://example.com/x for docs")
    );
}

#[test]
fn a_tab_before_a_hash_starts_a_comment() {
    let body = "  description: value\t# note\n";
    assert_eq!(read(body, "description").unwrap(), scalar("value"));
}

/// `PyYAML` reads `name:\n  "my_ext"` as `my_ext`; the reader kept the quotes
/// (and so refused the name) because a value starting on the next line was
/// always taken as plain text. Flow and block values there were read as text
/// too.
#[test]
fn a_quoted_or_flow_value_on_the_line_after_its_key_is_parsed_as_such() {
    let yml = doc("").replace("name: my_ext", "name:\n    \"my_ext\"");
    assert_eq!(parse_description_yml(&yml).unwrap().name, "my_ext");
    let yml = doc("").replace("ref: main", "ref:\n    'abc123' # pinned");
    assert_eq!(parse_description_yml(&yml).unwrap().git_ref, "abc123");
    let yml = doc("").replace(
        "  maintainers:\n    - Jane\n",
        "  maintainers:\n    [Jane, Bob]\n",
    );
    assert_eq!(
        parse_description_yml(&yml).unwrap().maintainers,
        ["Jane", "Bob"]
    );
    let yml = doc("").replace(
        "description: d",
        "description:\n    |\n      two\n      lines",
    );
    assert_eq!(
        parse_description_yml(&yml).unwrap().description,
        "two\nlines"
    );
}

/// Values this parser reads as text but `PyYAML` (YAML 1.1) reads as something
/// else draw a warning naming the field; quoting them, or a field this
/// parser does not read, draws none. The published corpus (346 files at
/// community-extensions `5ae7df8`) has three such values: two unquoted
/// all-digit versions and `custom_toolchain_script: true`, which is meant as
/// a boolean.
#[test]
fn unquoted_values_yaml_1_1_reads_as_non_text_are_warned_about() {
    let warned = |yml: &str| -> Vec<String> {
        parse_description_yml(yml)
            .unwrap()
            .warnings
            .into_iter()
            .filter(|w| w.contains("YAML 1.1"))
            .collect()
    };
    for (from, to, field, kind) in [
        ("name: my_ext", "name: yes", "extension.name", "a boolean"),
        (
            "version: 1.0.0",
            "version: 0.10",
            "extension.version",
            "a float",
        ),
        (
            "version: 1.0.0",
            "version: 2026012700",
            "extension.version",
            "an integer",
        ),
        ("ref: main", "ref: 0123456", "repo.ref", "an integer"),
        ("ref: main", "ref: 0x1f", "repo.ref", "an integer"),
        ("ref: main", "ref: 1.10", "repo.ref", "a float"),
        (
            "ref: main",
            "ref:\n    2024-01-01",
            "repo.ref",
            "a timestamp",
        ),
    ] {
        let yml = doc("").replace(from, to);
        let w = warned(&yml);
        assert_eq!(w.len(), 1, "{to}: {w:?}");
        assert!(
            w[0].starts_with(field) && w[0].contains(kind),
            "{to}: {w:?}"
        );
    }
    for (from, to) in [
        ("name: my_ext", "name: \"yes\""),
        ("ref: main", "ref: '0123456'"),
        ("ref: main", "ref: a1b2c3d"),
        ("version: 1.0.0", "version: v1.0.0"),
    ] {
        let yml = doc("").replace(from, to);
        assert_eq!(warned(&yml), Vec::<String>::new(), "{to}");
    }
    let yml = doc("  custom_toolchain_script: true\n");
    assert_eq!(warned(&yml), Vec::<String>::new());
}

/// Whether the YAML 1.1 warning may apply to a value that starts on the line
/// after its key: only a plain scalar there is one `PyYAML` may retype, not a
/// quoted scalar, a sequence or a mapping.
#[test]
fn only_a_plain_scalar_on_the_next_line_counts_as_plain() {
    let plain = |body: &str| {
        yaml::read_sections(&format!("extension:\n{body}"), &["extension"])
            .expect("parses")
            .into_iter()
            .flat_map(|s| s.entries)
            .find(|e| e.key == "version")
            .map(|e| e.plain)
            .expect("has a version")
    };
    assert!(plain("  version:\n    0.10\n"));
    assert!(!plain("  version:\n    \"0.10\"\n"));
    assert!(!plain("  version:\n    '0.10'\n"));
    assert!(!plain("  version:\n    - 0.10\n"));
    assert!(!plain("  version:\n    major: 0.10\n"));
    assert!(!plain("  version:\n    \"a: b\"\n"));
}

/// The error for text after a comment names the line the comment is on, not
/// the line the value started on — including a value that starts on the
/// line after its key.
#[test]
fn text_after_a_comment_names_the_comments_line() {
    for (body, comment_line) in [
        ("  version: a\n    b # c\n    d\n", 3),
        ("  version:\n    a # c\n    d\n", 3),
        ("  version:\n    a\n    b # c\n    d\n", 4),
    ] {
        let err = read(body, "version").expect_err("text after the comment");
        assert!(
            err.contains(&format!("ended the value on line {comment_line}")),
            "{body:?}: {err}"
        );
    }
}
