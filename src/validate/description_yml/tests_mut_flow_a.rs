// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests aimed at the mutants `cargo mutants` found alive in the first half of
//! `yaml/flow.rs` (`Cursor::line` through `Cursor::node`'s anchor and alias
//! arms): the flow-collection cursor, its position arithmetic and the
//! `node` dispatch.
//!
//! Accept/refuse rows were run through `yaml.safe_load` (`PyYAML` 6.0.3); the
//! line number on an error row is the line of `PyYAML`'s own error mark. Two
//! tests pin documented limits of the reader that `PyYAML` does not share, and
//! say so.

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

/// A descriptor with `text` after it, at the top level.
fn top(text: &str) -> String {
    desc("description: x") + text
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

/// The reader's message for `text`, read with `extension:` as the section whose
/// values the caller wants (so `f:` and `docs:` are only checked).
fn error_of(text: &str) -> String {
    yaml::read_sections(text, &["extension"]).expect_err(text)
}

/// Every error inside a flow collection names the line the cursor is on, and
/// `PyYAML` marks the same line. Kills `Cursor::line -> 0` and `-> 1`.
#[test]
fn a_flow_error_names_the_line_it_is_on() {
    for (what, text, line) in [
        // expected the node content, but found ','
        ("an empty item", "f: [a,\n,b]\n", 2),
        // expected <block end>, but found '<scalar>'
        ("text after the close", "f: [a,\n b] z\n", 2),
        // found character '\t' that cannot start any token
        ("a tab", "f: [a,\n\tb]\n", 2),
        // expected ',' or ']', but got '<scalar>'
        ("two quoted items", "f: [\"a\"\n \"b\"]\n", 2),
        // expected the node content, but found ','
        ("an empty mapping entry", "f: {a: 1,\n ,b}\n", 2),
        // expected ',' or '}', but got '<scalar>'
        ("no comma in a mapping", "f: {a: 1,\n b: \"2\"\n c}\n", 3),
        // found unhashable key
        ("a sequence as a key", "f: {a: 1,\n[x]: 1}\n", 2),
        // found character '@' that cannot start any token
        ("a leading @", "f: [a,\n @b]\n", 2),
    ] {
        let err = error_of(text);
        assert!(
            err.starts_with(&format!("line {line}:")),
            "{what}: PyYAML marks line {line}, quack-rs says {err:?}\n{text}"
        );
    }
}

/// `PyYAML` refuses an unterminated flow sequence (its error is at the end of
/// the stream: line 3 of the first text, line 2 of the second). This reader,
/// reading `maintainers:`, stops earlier, at the nested collection the open
/// sequence holds, and names the line its cursor is on: the last one. After
/// the last segment the cursor must stay on it. Kills `Cursor::next_seg`'s
/// `self.seg + 1` mutated to `self.seg * 1`, which moves the cursor one past
/// the last segment, where `line` is 0.
#[test]
fn the_cursor_stays_on_the_last_line_at_the_end_of_the_text() {
    for (what, text, line) in [
        (
            "ending in a line break",
            "extension:\n  maintainers: [[x]\n",
            3,
        ),
        ("ending in text", "extension:\n  maintainers: [[x]", 2),
    ] {
        let err = error_of(text);
        assert!(
            err.starts_with(&format!("line {line}:")),
            "{what}: PyYAML marks line {line}, quack-rs says {err:?}\n{text}"
        );
    }
}

/// `-` starts a plain scalar unless a space, a tab or the line end follows it
/// (`PyYAML`: `[-x]` is `['-x']`, `[-]` is `['-']`, `[- x]` is an error).
/// Kills `Cursor::peek_second -> None` and the `- ` guard of `node` mutated
/// to `true`, which refuse every flow item that starts with `-`.
#[test]
fn a_dash_starts_a_flow_scalar_unless_a_space_follows() {
    check(&[
        ("`-x`", docs("a: [-x]"), true),
        ("`-1` and `-a`", docs("a: [-1, -a]"), true),
        ("a lone `-` before `]`", docs("a: [-]"), true),
        ("a lone `-` before `,`", docs("a: [-, x]"), true),
        ("a mapping key `-x`", docs("a: {-x: 1}"), true),
        // expected the node content, but found '-'
        ("`- ` in a flow sequence", docs("a: [- x]"), false),
        ("`-` at the end of a line", docs("a: [-\n x]"), false),
    ]);
}

/// A flow collection nested exactly as deep as the reader allows is accepted
/// (`PyYAML` accepts this and much deeper ones; the reader's bound is the
/// documented `MAX_DEPTH` of 64, whose refusal side
/// `deep_nesting_is_an_error_not_a_stack_overflow` covers). A key's value at
/// depth 1, so the innermost of 64 brackets is at depth 64. Kills `node`'s
/// `depth > MAX_DEPTH` mutated to `>=` and to `==`.
#[test]
fn a_flow_collection_at_the_depth_limit_is_accepted() {
    let text = top(&format!("f: {}{}\n", "[".repeat(64), "]".repeat(64)));
    assert!(parse_description_yml(&text).is_ok(), "{text}");
}

/// Anchors in a flow collection of an ordinary section are read as `PyYAML`
/// reads them, wherever on the line they stand: the cursor must land exactly
/// after `&name`. Kills `node`'s `self.read` guard mutated to `true`, the `!` of
/// `define` deleted, and the `self.pos += 1 + name.len()` of the anchor arm
/// mutated to `-=`, `*=`, `-` and `*`.
#[test]
fn anchors_inside_flow_collections_are_read() {
    check(&[
        // An anchor, then a nested collection: if the cursor stops short, `x`
        // is read as a scalar and `[1]` is stray text; if it stays, `&` is.
        ("an anchored sequence", docs("a: [&x [1], 2]"), true),
        (
            "an anchor with a long name",
            docs("a: [&anchor_1 [1]]"),
            true,
        ),
        ("an anchored mapping", docs("a: [&m {k: v}, 2]"), true),
        ("two different anchors", docs("a: [&x 1, &y 2]"), true),
        // At column 0 of a continuation line.
        ("an anchor at column 0", top("f: [a,\n&x [1]]\n"), true),
        // found duplicate anchor 'x'
        ("a repeated anchor", docs("a: [&x 1, &x 2]"), false),
        // expected ',' or ']', but got '<anchor>'
        ("an anchor on an anchor", docs("a: [&x &y 1]"), false),
    ]);
}

/// Aliases in a flow collection of an ordinary section are read as `PyYAML`
/// reads them. Kills the `self.pos += 1 + name.len()` of the alias arm of
/// `node` mutated to `-=`, `*=`, `-` and `*`. Every alias here is the first
/// thing on its line or in its collection, where `-=` underflows and panics:
/// further along it would move the cursor back before the alias and read it
/// again for ever, which no test can report.
#[test]
fn aliases_inside_flow_collections_are_read() {
    check(&[
        (
            "an alias first in a sequence",
            docs("a: &x 1\nb: [*x]"),
            true,
        ),
        (
            "an alias at column 0",
            top("anc: &x 1\nf: [a,\n*x]\n"),
            true,
        ),
        (
            "an alias of an anchor in the same collection",
            top("f: [&x 1,\n*x]\n"),
            true,
        ),
        ("an alias of a mapping", top("f: [&m {k: v},\n*m]\n"), true),
        (
            "an alias as a mapping key",
            top("f: [&x 1,\n{*x : 1}]\n"),
            true,
        ),
        // found undefined alias 'nope'
        ("an undefined alias", docs("a: [*nope]"), false),
        // found unhashable key
        (
            "an alias of a sequence as a key",
            top("f: [&x [1],\n{*x : 1}]\n"),
            false,
        ),
    ]);
}

/// In a field the caller reads, the reader refuses anchors, aliases and tags
/// (yaml.rs: "no anchors, aliases or nested flow collections" in
/// `Mode::Read`), as it does for a scalar value. `PyYAML` accepts the anchor
/// row: this is the documented subset, not `PyYAML`'s verdict. Kills `node`'s
/// `self.read` guard mutated to `false`.
#[test]
fn anchors_aliases_and_tags_in_a_read_flow_sequence_are_refused() {
    for (what, value) in [
        ("an anchor", "[&a x]"),
        ("an alias", "[*a]"),
        ("a tag", "[!t x]"),
    ] {
        let err = error_of(&format!("extension:\n  maintainers: {value}\n"));
        assert!(
            err.contains("anchors, aliases and tags are not supported"),
            "{what}: {err}"
        );
    }
}
