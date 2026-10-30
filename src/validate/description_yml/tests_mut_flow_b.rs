// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Flow collections (`yaml/flow.rs`, from `optional_value` on): the paths that
//! only a particular spelling reaches. Each test names the `cargo mutants`
//! mutants it exists for, as `flow.rs:line`.
//!
//! Every accepted-or-refused row was run through `yaml.safe_load` (`PyYAML`
//! 6.0.3), and every value is what `yaml.compose` reads for the text; the
//! wording of an error is this reader's own. The one verdict that is not
//! `PyYAML`'s is the nesting bound (64 levels, to keep a hostile file from
//! exhausting the stack): `PyYAML` reads the same texts at 100 levels.

use super::{parse_description_yml, yaml};

const EXTENSION: &str = "extension:\n  name: my_ext\n  {d}\n  language: Rust\n  build: cargo\n  \
                         license: MIT\n  maintainers:\n    - Jane Doe\n\nrepo:\n  github: \
                         janedoe/duckdb-my-ext\n  ref: 4f1c2e9a7b3d5c8e1f0a2b4c6d8e0f1a3b5c7d9e\n";

/// A descriptor whose `extension.description` line is `line`.
fn desc(line: &str) -> String {
    EXTENSION.replace("{d}", line)
}

/// A descriptor with `body` as its `docs:` section, indented two spaces.
fn docs(body: &str) -> String {
    let mut text = desc("description: x") + "docs:\n";
    for line in body.split('\n') {
        if !line.is_empty() {
            text.push_str("  ");
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn check(rows: &[(&str, String, bool)]) {
    for (what, text, pyyaml_accepts) in rows {
        let result = parse_description_yml(text);
        assert_eq!(
            result.is_ok(),
            *pyyaml_accepts,
            "{what}: PyYAML {} it, quack-rs says {result:?}\n{text}",
            if *pyyaml_accepts {
                "accepts"
            } else {
                "refuses"
            }
        );
    }
}

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

/// `: ` followed by nothing before the closing bracket or a comma is a pair
/// with a null value, and nothing more is read for it.
#[test]
fn a_flow_pair_with_no_value_follows_pyyamls_rules() {
    check(&[
        // flow.rs:195:53 `&&` -> `||`: the closing character, and a comma,
        // would each be read as the start of a value.
        ("`[x: ]`", docs("a: [x: ]"), true),
        ("a mapping, `: ` then `}`", docs("a: { x: }"), true),
        ("`[x:, y]`", docs("a: [x:, y]"), true),
        ("`{x:, y: 1}`", docs("a: {x:, y: 1}"), true),
        ("`{x: , y}`", docs("a: {x: , y}"), true),
        // while parsing a flow node ... expected the node content, but found ','
        ("`{,}`", docs("a: {,}"), false),
        ("`{x,,}`", docs("a: {x,,}"), false),
    ]);
}

/// A mapping that closes right where a key was expected: `{}` and a trailing
/// comma.
#[test]
fn an_empty_or_comma_ended_flow_mapping_follows_pyyamls_rules() {
    check(&[
        // flow.rs:262:30 `+=` -> `-=` / `*=`: the reader is left on or before
        // the `}`, so the collection seems not to end where it does.
        ("`{}`", docs("a: {}"), true),
        ("`[{}]`", docs("a: [{}]"), true),
        ("`{x: {}}`", docs("a: {x: {}}"), true),
        ("`{x: 1,}`", docs("a: {x: 1,}"), true),
        ("`{x,}`", docs("a: {x,}"), true),
    ]);
    assert_eq!(read("  k: {}\n", "k"), Ok(yaml::Value::Mapping));
    assert_eq!(read("  k: {x: 1,}\n", "k"), Ok(yaml::Value::Mapping));
}

/// The value of a pair is a node of its own: a collection, not text up to the
/// next space.
#[test]
fn a_flow_collection_can_be_the_value_of_a_pair() {
    check(&[
        // flow.rs:316:21 delete match arm ':', and 316:35 `+` -> `-` / `*`: the
        // `:` of `x: ` no longer ends the key, which then runs on to `[`.
        ("`{x: [1]}`", docs("a: {x: [1]}"), true),
        ("`[x: [1]]`", docs("a: [x: [1]]"), true),
        ("`{x: {y: 1}}`", docs("a: {x: {y: 1}}"), true),
        (
            "two entries",
            docs("a: {x: [1], y: {z: 2}, w: [v: [u]]}"),
            true,
        ),
        // A `:` not followed by a space or a flow indicator is part of the word.
        ("`[x:y]`", docs("a: [x:y]"), true),
        ("`[a :b]`", docs("a: [a :b]"), true),
    ]);
}

/// Nesting is bounded (see the module comment). The value of a `[k: v]` pair,
/// the key of a `{k}` mapping and the value of a `{k: v}` pair are one level
/// below the collection that holds them, as an item of a sequence is. The first
/// bracket under `docs:` is level 2, and 64 is the deepest level read.
#[test]
fn the_nesting_bound_holds_inside_flow_pairs_and_mappings() {
    let pair_in_seq = |m: usize| docs(&format!("a: [x: {}{}]", "[".repeat(m), "]".repeat(m)));
    let key_in_map = |k: usize| docs(&format!("a: {}{{x}}{}", "[".repeat(k), "]".repeat(k)));
    let pair_in_map = |m: usize| docs(&format!("a: {{x: {}{}}}", "[".repeat(m), "]".repeat(m)));
    for (what, deepest, too_deep) in [
        // flow.rs:228:39 `depth + 1` -> `depth - 1` / `depth * 1`
        (
            "the value of a `[k: v]` pair",
            pair_in_seq(62),
            pair_in_seq(63),
        ),
        // flow.rs:279:39
        ("the key of a `{k}` mapping", key_in_map(61), key_in_map(62)),
        // flow.rs:284:39
        (
            "the value of a `{k: v}` pair",
            pair_in_map(62),
            pair_in_map(63),
        ),
    ] {
        assert!(
            parse_description_yml(&deepest).is_ok(),
            "{what}: refused at the deepest level\n{deepest}"
        );
        let err = parse_description_yml(&too_deep).expect_err(what);
        assert!(
            err.as_str().contains("nested more than 64"),
            "{what}: {err}"
        );
    }
}

/// A plain item may run over several lines: a break between words is a space
/// and each blank line one newline (`yaml.compose`: `'Jane Doe'`,
/// `'Jane\nDoe'`, `'Jane\n\nDoe'`).
#[test]
fn a_plain_flow_item_continued_on_later_lines_is_folded() {
    for (what, body, folded) in [
        // flow.rs:341:43 `-` -> `+`, 342:24 delete `!`, 347:33 `==` -> `!=`
        ("one break", "  k: [Jane\n    Doe]\n", "Jane Doe"),
        (
            "three lines",
            "  k: [Jane\n    Q.\n    Doe]\n",
            "Jane Q. Doe",
        ),
        ("trailing spaces", "  k: [Jane  \n    Doe]\n", "Jane Doe"),
        ("at column 0", "  k: [Jane\nDoe]\n", "Jane Doe"),
        // flow.rs:345:28 `+=` -> `*=`: the blank lines are not counted
        ("one blank line", "  k: [Jane\n\n    Doe]\n", "Jane\nDoe"),
        (
            "a blank line of spaces",
            "  k: [Jane\n   \n    Doe]\n",
            "Jane\nDoe",
        ),
        (
            "two blank lines",
            "  k: [Jane\n\n\n    Doe]\n",
            "Jane\n\nDoe",
        ),
    ] {
        assert_eq!(
            read(body, "k"),
            Ok(yaml::Value::Seq(vec![scalar(folded)])),
            "{what}"
        );
    }
    // The same through the public entry point: a maintainer's name over lines.
    let yml = desc("description: x").replace(
        "  maintainers:\n    - Jane Doe\n",
        "  maintainers: [Jane\n    Doe, Bob\n\n    Roe]\n",
    );
    assert_eq!(
        parse_description_yml(&yml).unwrap().maintainers,
        ["Jane Doe", "Bob\nRoe"]
    );
    check(&[
        // while parsing a flow sequence: a comment ends the plain item, and
        // more text after it is not a separator
        (
            "text after a comment line",
            docs("a: [first\n # c\n second]"),
            false,
        ),
    ]);
}

/// `~`, `null`, `Null` and `NULL` are null in a flow collection as elsewhere;
/// quoted, or longer, they are text.
#[test]
fn null_spellings_in_a_flow_sequence_read_as_null() {
    // flow.rs:363:13 delete match arm "~" | "null" | "Null" | "NULL"
    assert_eq!(
        read("  k: [~, null, Null, NULL, nulls, \"~\"]\n", "k"),
        Ok(yaml::Value::Seq(vec![
            yaml::Value::Null,
            yaml::Value::Null,
            yaml::Value::Null,
            yaml::Value::Null,
            scalar("nulls"),
            scalar("~"),
        ]))
    );
    // So `[~, Jane]` names one maintainer, as `- ~` does in a block list.
    let yml = desc("description: x").replace(
        "  maintainers:\n    - Jane Doe\n",
        "  maintainers: [~, Jane, null]\n",
    );
    assert_eq!(parse_description_yml(&yml).unwrap().maintainers, ["Jane"]);
}

/// The error for a flow collection that never closes names the line it opened
/// on, and says which kind it is.
#[test]
fn an_unterminated_flow_collection_names_the_line_it_opened_on() {
    // while parsing a flow sequence / mapping, or (after a comma) a flow node:
    // PyYAML refuses each. flow.rs:370:5 `unterminated` -> `String::new()` /
    // "xyzzy", from each of its four call sites.
    for (what, body, error) in [
        (
            "a sequence after an item",
            "  k: [x\n",
            "line 2: unterminated flow sequence",
        ),
        (
            "a sequence after a comma",
            "  k: [x,\n",
            "line 2: unterminated flow sequence",
        ),
        (
            "a mapping after an entry",
            "  k: {x: 1\n",
            "line 2: unterminated flow mapping",
        ),
        (
            "a mapping after a comma",
            "  k: {x: 1,\n",
            "line 2: unterminated flow mapping",
        ),
        (
            "a sequence opened lines earlier",
            "  k: [x,\n    y,\n    z\n",
            "line 2: unterminated flow sequence",
        ),
    ] {
        assert_eq!(read(body, "k"), Err(error.to_string()), "{what}");
    }
}

/// A sequence or mapping cannot be a mapping key (`PyYAML`: "found unhashable
/// key"), and the error says so.
#[test]
fn a_collection_used_as_a_flow_mapping_key_is_refused_with_its_line() {
    for (what, body) in [
        ("a sequence key", "  k: {[x]: 1}\n"),
        ("a mapping key", "  k: {{x: 1}: 2}\n"),
        ("a sequence key in a pair", "  k: [[x]: 1]\n"),
        ("a mapping key in a pair", "  k: [{x: 1}: 2]\n"),
    ] {
        // flow.rs:374:5 `collection_key` -> `String::new()` / "xyzzy"
        assert_eq!(
            read(body, "k"),
            Err("line 2: a collection used as a mapping key, which PyYAML cannot load".to_string()),
            "{what}"
        );
    }
    check(&[
        ("a sequence key", docs("a: {[x]: 1}"), false),
        ("a mapping key in a pair", docs("a: [{x: 1}: 2]"), false),
    ]);
}
