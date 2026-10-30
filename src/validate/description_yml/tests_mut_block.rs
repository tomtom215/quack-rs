// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Inputs for the block-mapping, block-sequence and value readers
//! (`yaml/block.rs`, `yaml/value.rs`) that only a mutation-testing run asked
//! for: each row below separates the reader from a mutant of it that every
//! other test let through. The comment on a row names the mutation it kills.
//!
//! Where a row's verdict comes from `PyYAML` 6.0.3's `yaml.safe_load` it says
//! so. The rows on the nesting bound are the exception: `PyYAML` reads 64
//! levels (it fails only near Python's recursion limit, a few hundred levels
//! down), so those rows pin the bound the reader sets itself, `MAX_DEPTH`, at
//! 63 and 64 levels.

use super::parse_description_yml;

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

/// The line `docs(body)` puts the first line of `body` on.
fn first_docs_line() -> usize {
    desc("description: x").lines().count() + 2
}

/// Each row's text is accepted exactly when `PyYAML` accepts it (the bool).
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

/// Each row's text is refused, and the message holds `needle`. Which of two
/// errors the reader reports for text that has both is what separates a row
/// from a mutant that moves a check.
fn check_refusals(rows: &[(&str, String, String)]) {
    for (what, text, needle) in rows {
        let err = parse_description_yml(text).expect_err(what);
        assert!(
            err.as_str().contains(needle.as_str()),
            "{what}: expected '{needle}' in the error, got: {err}\n{text}"
        );
    }
}

/// `n` block mappings nested in one another, each key one space deeper than
/// its parent (`k:`, ` k:`, `  k:` ...), the innermost one's line ending in
/// `last`.
fn mapping_chain(n: usize, last: &str) -> String {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&" ".repeat(i));
        out.push_str("k:");
        if i + 1 == n {
            out.push_str(last);
        }
        out.push('\n');
    }
    out
}

/// `s:` holding `n` sequences nested in one another on one line (`- - x`),
/// the innermost holding `last`.
fn seq_chain(n: usize, last: &str) -> String {
    format!("s:\n  {}{last}\n", "- ".repeat(n))
}

/// `s:` holding `n` levels of `- a:`: a sequence item that is a mapping whose
/// one value is the next level's sequence. The innermost value is `x`.
fn hop_chain(n: usize) -> String {
    let mut out = String::from("s:\n");
    for i in 0..n {
        out.push_str(&" ".repeat(2 + 3 * i));
        out.push_str(if i + 1 == n { "- a: x\n" } else { "- a:\n" });
    }
    out
}

/// Kills `block.rs:48:20`, `||` -> `&&` in `mapping_key`. With `&&` the test
/// for an explicit key can never hold, and an `? ` entry is still refused, but
/// by the plain-scalar rules ("cannot start with '? '") instead of as the
/// explicit key it is.
///
/// `PyYAML` refuses both rows too (`? b: c` makes the key a mapping: "found
/// unhashable key"). It reads `? b` plus `: c` as a key `b`, which this reader
/// refuses as well, as its module documentation says.
#[test]
fn an_explicit_key_is_refused_as_one() {
    let explicit = "explicit `? key`".to_string();
    check_refusals(&[
        ("`? ` first in docs", docs("? b: c"), explicit.clone()),
        ("`? ` after another key", docs("a: 1\n? b: c"), explicit),
    ]);
}

/// Kills `block.rs:149:28`, `>` -> `<` in `parse_mapping`: a key indented more
/// than the mapping it follows but less than the nested one before it belongs
/// to neither. (A deeper line right after a plain value is refused by the
/// plain scalar, so no earlier row reached this check.)
#[test]
fn a_key_between_two_indentations_is_refused() {
    check(&[
        // PyYAML: while parsing a block mapping ... expected <block end>
        (
            "between a nested mapping and its parent",
            docs("a:\n    b: 1\n  c: 2"),
            false,
        ),
        ("back at the parent", docs("a:\n    b: 1\nc: 2"), true),
    ]);
    let err = parse_description_yml(&docs("a:\n    b: 1\n  c: 2")).expect_err("between");
    assert!(err.as_str().contains("inconsistent indentation"), "{err}");
}

/// Kills `block.rs:81:26` (`-` -> `/`) and `block.rs:81:57` (`>` -> `>=`,
/// `>` -> `==`) in `quoted_key`: `PyYAML` allows a simple key 1024 characters
/// from its opening quote to its colon, and refuses one more ("mapping values
/// are not allowed here"). The quotes count.
///
/// `block.rs:80:37` (`-` -> `/`) is equivalent: the text `check_code` is given
/// there is the run of spaces between the closing quote and the colon, or a
/// prefix of it and the colon, so it never holds the tab it looks for. A tab
/// there is refused anyway, for the colon is then not found (the last two
/// rows).
#[test]
fn a_quoted_key_may_have_1024_characters_with_its_quotes() {
    let key = |quote: char, inner: usize| format!("{quote}{}{quote}: v", "k".repeat(inner));
    check(&[
        (
            "a 1024-character double-quoted key",
            docs(&key('"', 1022)),
            true,
        ),
        (
            "a 1025-character double-quoted key",
            docs(&key('"', 1023)),
            false,
        ),
        (
            "a 1024-character single-quoted key",
            docs(&key('\'', 1022)),
            true,
        ),
        (
            "a 1025-character single-quoted key",
            docs(&key('\'', 1023)),
            false,
        ),
        // found character '\t' that cannot start any token
        ("a tab before the colon", docs("\"a\"\t: 1"), false),
        ("a space, a tab, the colon", docs("\"a\" \t: 1"), false),
    ]);
}

/// Kills `value.rs:39:14` (`>` -> `>=`, `>` -> `==`) in `parse_value`: a value
/// at depth 64 is read and one at depth 65 is not. The value of the 63rd
/// nested mapping (the `docs:` value is depth 1) is at depth 64.
///
/// Also kills `block.rs:158:18` (`>` -> `>=`, `>` -> `==`) in `parse_mapping`:
/// the 64th mapping is within the bound, so a bad key in it is reported as a
/// bad key (`PyYAML`: found character '@' that cannot start any token), where
/// both mutants call it too deep.
#[test]
fn mappings_nest_64_levels_deep_and_no_deeper() {
    check(&[(
        "63 nested mappings, a value at depth 64",
        docs(&mapping_chain(63, " v")),
        true,
    )]);
    check_refusals(&[
        (
            "64 nested mappings, a value at depth 65",
            docs(&mapping_chain(64, " v")),
            "nested more than 64".to_string(),
        ),
        (
            "a bad key in the 64th mapping",
            docs(&format!("{}{}@k: v", mapping_chain(63, ""), " ".repeat(63))),
            "cannot start with '@'".to_string(),
        ),
    ]);
}

/// Kills `block.rs:267:56` (`+` -> `-`, `+` -> `*`) in `parse_seq`: the value
/// of a sequence item is one level deeper than the sequence. The item of the
/// 64th nested sequence is at depth 65.
#[test]
fn sequences_nest_64_levels_deep_and_no_deeper() {
    check(&[(
        "63 nested sequences, the item at depth 64",
        top(&seq_chain(63, "x")),
        true,
    )]);
    check_refusals(&[(
        "64 nested sequences, the item at depth 65",
        top(&seq_chain(64, "x")),
        "nested more than 64".to_string(),
    )]);
}

/// Kills `block.rs:239:18` (`>` -> `>=`, `>` -> `==`) in `parse_seq`: the 64th
/// nested sequence is within the bound, so the tab after its marker
/// (`PyYAML`: found character '\t' that cannot start any token) is the error
/// reported, where both mutants call it too deep.
#[test]
fn the_64th_nested_sequence_is_read() {
    check_refusals(&[(
        "a tab after the marker of the 64th sequence",
        top(&seq_chain(63, "- \tx")),
        "a tab is used".to_string(),
    )]);
}

/// Kills `block.rs:257:81` (`+` -> `-`, `+` -> `*`) in `parse_seq`: the mapping
/// on a `- a:` item is one level deeper than the sequence, so each level of
/// `- a:` is two levels in all.
#[test]
fn a_mapping_on_a_sequence_item_is_one_level_deeper() {
    check(&[(
        "31 levels of `- a:`, the value at depth 63",
        top(&hop_chain(31)),
        true,
    )]);
    check_refusals(&[(
        "32 levels of `- a:`, the value at depth 65",
        top(&hop_chain(32)),
        "nested more than 64".to_string(),
    )]);
}

/// Kills `value.rs:67:55` and `value.rs:68:44` (`+` -> `-`) for a quoted
/// value, and `value.rs:73:55` and `value.rs:74:29` (`+` -> `-`) for a flow
/// collection: the lines below the value that are indented past its key are
/// checked and counted. No other test had any, and `used - 0` is `used`.
#[test]
fn lines_below_a_quoted_or_flow_value_are_checked_and_counted() {
    check(&[
        // PyYAML: while parsing a block mapping ... expected <block end>,
        // but found '<scalar>'
        ("text below a quoted value", docs("a: \"x\"\n  junk"), false),
        ("text below a flow sequence", docs("a: [x]\n  junk"), false),
        (
            "text below a flow mapping",
            docs("a: {x: 1}\n  junk"),
            false,
        ),
        // Accepted: comments only.
        (
            "a comment below a quoted value",
            docs("a: \"x\"\n  # c\nb: 1"),
            true,
        ),
        (
            "a comment below a flow sequence",
            docs("a: [x]\n  # c\nb: 1"),
            true,
        ),
        (
            "a comment below a flow mapping",
            docs("a: {x: 1}\n  # c\nb: 1"),
            true,
        ),
        // The key after the comments is read: PyYAML refuses '@' there.
        (
            "two comments below a quoted value over four lines",
            docs("a: \"x\n  y\n  z\n  w\"\n  # c1\n  # c2\nb: @bad"),
            false,
        ),
        (
            "two comments below a flow sequence over four lines",
            docs("a: [x,\n  y,\n  z,\n  w]\n  # c1\n  # c2\nb: @bad"),
            false,
        ),
    ]);
}

/// Kills `value.rs:63:31`, `-` -> `/` in `parse_value`: the text after a
/// multi-line quoted value is on the line the value ends on, and the error
/// names that line (`PyYAML` marks the junk there too), not the one after.
#[test]
fn text_after_a_multi_line_quoted_value_is_reported_where_it_is() {
    let second = first_docs_line() + 1;
    let said = |line: usize| format!("line {line}: unexpected text after a quoted value: 'junk'");
    check_refusals(&[
        (
            "junk after a quoted value that ends on its second line",
            docs("a: \"x\n  y\" junk\nb: 1"),
            said(second),
        ),
        (
            "junk after a quoted value that ends on its third line",
            docs("a: 'x\n  y\n  z' junk\nb: 1"),
            said(second + 1),
        ),
    ]);
}
