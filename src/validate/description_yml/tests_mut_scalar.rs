// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests for the scalar-level pieces of the reader (`yaml/scalar.rs`), the
//! line and value bookkeeping of `yaml.rs`, and the date check of
//! `yaml11.rs`, written to kill the mutants `cargo mutants` reported as
//! surviving there. Each row names the mutant it kills.
//!
//! Every accepted or refused verdict below was taken from `yaml.safe_load`
//! (`PyYAML` 6.0.3), not from this reader. Where `PyYAML` refuses and so does
//! the reader, and only the message tells a mutant from the original, the
//! test pins the message.

use super::parse_description_yml;

const EXTENSION: &str = "extension:\n  name: my_ext\n  {d}\n  language: Rust\n  build: cargo\n  \
                         license: MIT\n  maintainers:\n    - Jane Doe\n\nrepo:\n  github: \
                         janedoe/duckdb-my-ext\n  ref: 4f1c2e9a7b3d5c8e1f0a2b4c6d8e0f1a3b5c7d9e\n";

/// A descriptor whose `extension.description` line is `line` (more lines of
/// the value may follow it, written with their full indentation).
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

/// `extension.description` of `text`, which must be accepted.
fn description(text: &str) -> String {
    parse_description_yml(text)
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
        .description
}

/// The message of the error `text` is refused with.
fn refusal(text: &str) -> String {
    parse_description_yml(text)
        .expect_err(text)
        .as_str()
        .to_string()
}

/// `yaml.rs` `read_sections`, guards `first.indent > 0` (`true`, `>=`) and
/// `first.is_seq_item()` (`false`).
///
/// `PyYAML` accepts all of these as YAML (an empty section is null, and a
/// section may be a sequence); the reader refuses them because the section
/// must be a mapping with fields. What is pinned is *why*: an empty section
/// does not swallow the top-level key after it (that key is read as a
/// top-level key, where an anchor is fine), and a sequence is named as such.
#[test]
fn a_wanted_section_is_empty_or_a_mapping_and_nothing_else() {
    // `>= 0` / `true`: the next top-level key was read as the section's own
    // entry, in a mode that refuses its anchor.
    let ext = "extension:\n  name: my_ext\n  description: x\n  language: Rust\n  build: cargo\n  \
               license: MIT\n  maintainers:\n    - Jane Doe\n";
    let err = refusal(&format!("{ext}repo:\nx: &a 1\n"));
    assert!(err.contains("repo.github"), "{err}");
    let err = refusal("extension:\nx: &a 1\nrepo:\n  github: a/b\n  ref: main\n");
    assert!(err.contains("extension.name"), "{err}");

    // `is_seq_item()` -> false: the sequence was left to be read as a
    // top-level line, which is a different error.
    for (what, text) in [
        (
            "extension",
            "extension:\n- name\nrepo:\n  github: a/b\n  ref: main\n".to_string(),
        ),
        ("repo", format!("{ext}repo:\n- a/b\n")),
    ] {
        let err = refusal(&text);
        assert!(
            err.contains(&format!("'{what}' must be a block mapping")),
            "{what}: {err}"
        );
    }
}

/// `yaml.rs` `nothing_but_comments` -> `Ok(())`: a quoted value, a flow
/// collection or an alias may be followed by comments and blank lines, but
/// not by text on a deeper line.
#[test]
fn text_on_a_line_after_a_complete_value_is_refused() {
    check(&[
        // while parsing a block mapping ... expected <block end>
        ("after a quoted value", docs("a: \"x\"\n  more"), false),
        (
            "after a quoted value, in a field",
            desc("description: \"a\"\n    b"),
            false,
        ),
        ("after a flow sequence", docs("a: [x]\n  more"), false),
        ("after a flow mapping", docs("a: {x: 1}\n  more"), false),
        ("after an alias", docs("a: &x 1\nb: *x\n  more"), false),
        // Accepted: only comments and blank lines follow.
        (
            "a comment after a flow sequence",
            docs("a: [x]\n  # c\nb: 1"),
            true,
        ),
        (
            "a comment after a quoted value",
            docs("a: \"x\"\n  # c\n\nb: 1"),
            true,
        ),
    ]);
}

/// `scalar.rs` `split_key`, `||` -> `&&` in `text == "-" || text.starts_with("- ")`:
/// a `- item` line is not a `key: value` entry even when it holds a `: `. Both
/// readings are refused (`PyYAML`: "expected <block end>"); the mutant refuses
/// them with the key's own message instead.
#[test]
fn a_sequence_item_line_is_not_a_mapping_entry() {
    let err = refusal(&docs("a: 1\n- b: 2"));
    assert!(err.contains("expected `key: value`"), "{err}");
    let err = refusal(&top("- a: b\n"));
    assert!(err.contains("expected a top-level `key:`"), "{err}");
}

/// `scalar.rs` `decode_quoted`, `c == ' ' || c == '\t'` -> `&&`: spaces and
/// tabs before a line break inside quotes are dropped (`PyYAML`: 'foo bar').
#[test]
fn trailing_whitespace_before_a_break_inside_quotes_is_dropped() {
    for (what, text) in [
        ("double, spaces", "description: \"foo  \n    bar\""),
        ("double, tab and space", "description: \"foo\t \n    bar\""),
        ("single, spaces", "description: 'foo  \n    bar'"),
    ] {
        assert_eq!(description(&desc(text)), "foo bar", "{what}");
    }
    // Whitespace that is followed by text on its line stays.
    assert_eq!(
        description(&desc("description: \"a  b\t c \n    d\"")),
        "a  b\t c d"
    );
}

/// `scalar.rs` `block_header`, `i += 1` -> `-=` / `*=` after `[-+]` then a
/// digit: the digit is part of the header, not text after it.
#[test]
fn a_chomping_indicator_then_an_indentation_digit_is_a_block_header() {
    check(&[
        ("`|-2`", docs("a: |-2\n    x\n     y"), true),
        ("`|+1`", docs("a: |+1\n   x\n    y"), true),
        // while scanning a block scalar ... expected chomping or indentation
        // indicators, but found '2' (a second digit)
        ("`|-22`", docs("a: |-22\n    x"), false),
    ]);
    assert_eq!(
        description(&desc("description: |-2\n    x\n     y")),
        "x\n y"
    );
}

/// `scalar.rs` `block_scalar`: the indentation of a block scalar without a
/// digit is the widest of its leading blank lines and `parent + 1`
/// (`parent_indent + 1` -> `-` / `*`), and with a digit it is `parent + digit`
/// (`parent_indent + increment` -> `-` / `*`).
#[test]
fn a_block_scalars_indentation_is_the_parents_plus_one_or_its_digit() {
    check(&[
        // At the top level the parent's indentation is 0 (`0 - 1` underflows).
        ("top level, detected", top("x: |\n  text\n  more\n"), true),
        // `0 - 2` underflows, `0 * 2` is 0.
        ("top level, `|2`", top("x: |2\n    x\n     y\n"), true),
        // while parsing a block mapping: a comment less indented than the
        // content ends the block, and the text after it is refused. With the
        // indentation `parent` (not `parent + 1`) the comment was content.
        (
            "a comment at the parent's level, top level",
            top("x: |\n# c\n  text\n"),
            false,
        ),
        (
            "a comment at the parent's level, in docs",
            docs("a: |\n# c\n  text"),
            false,
        ),
    ]);
    // `|1` under a key at column 2: content starts at column 3. With 2 (`*`)
    // or 1 (`-`) the lines keep more of their indentation.
    assert_eq!(description(&desc("description: |1\n   x\n    y")), "x\n y");
    // `|4` under a key at column 2: column 6; `2 - 4` underflows.
    assert_eq!(
        description(&desc("description: |4\n      x\n       y")),
        "x\n y"
    );
}

/// `scalar.rs` `block_scalar`, `child.raw.len() > indent` -> `>=`: a line of
/// exactly the block's indentation in spaces is a blank line, not an empty
/// content line. In a folded scalar it is a line break (`PyYAML`: 'a\nb\n');
/// as an empty content line it folded to a space.
#[test]
fn a_line_of_only_the_indentation_is_a_blank_line() {
    assert_eq!(
        description(&desc("description: >\n    a\n    \n    b")),
        "a\nb"
    );
    assert_eq!(
        description(&desc("description: >\n    a\n    \n    \n    b")),
        "a\n\nb"
    );
    assert_eq!(
        description(&desc("description: |\n    a\n    \n    b")),
        "a\n\nb"
    );
}

/// `scalar.rs` `anchor_name`: the name ends at a space, a tab, or one of
/// `?:,]}%@` and a backtick; it is not empty.
#[test]
fn an_anchor_name_is_not_empty_and_ends_where_pyyaml_ends_it() {
    check(&[
        // while scanning an anchor ... expected alphabetic or numeric
        // character, but found ' ' (`name.is_empty() || ..` -> `&&`)
        ("an empty anchor name", docs("a: & x"), false),
        ("an anchor with no name", docs("a: &"), false),
        ("an empty anchor name in a flow", docs("a: [& x]"), false),
        // while scanning an anchor ... expected alphabetic or numeric
        // character, but found '.' / '"' / '[' (`c == '\t'` -> `!=`): in a
        // flow collection nothing but the name check refuses them
        ("a dot after an anchor", docs("a: [&x.y]"), false),
        ("a quote after an anchor", docs("a: [&x\"y\"]"), false),
        ("a bracket after an anchor", docs("a: [&x[1]]"), false),
        (
            "a dot after an anchor in a map",
            docs("a: {&x.y: 1}"),
            false,
        ),
        // Accepted: names end at a space, `,` and `]`.
        ("an anchor then a space", docs("a: &x 1\nb: *x"), true),
        (
            "an alias then `,` and `]`",
            docs("a: &x 1\nb: [*x, *x]"),
            true,
        ),
        ("an anchor then `,`", docs("a: [&x,&y]"), true),
    ]);
    // A tab ends the name too, so it is the tab that `PyYAML` refuses
    // ("found character '\t' that cannot start any token"), not the name. All
    // four are refused either way; `|| "?:,]}%@`".contains(c)` -> `&&` makes
    // the name check say something else.
    for (what, text, said) in [
        ("block anchor", docs("a: &x\t1"), "a space after the anchor"),
        ("flow anchor", docs("a: [&x\t1]"), "a tab"),
        ("alias", docs("a: &x 1\nb: *x\t"), "a tab"),
        ("flow alias", docs("a: &x 1\nb: [*x\t]"), "a tab"),
    ] {
        let err = refusal(&text);
        assert!(err.contains(said), "{what}: {err}");
    }
}

/// `yaml11.rs` `unconstructible_timestamp`, `hour > 23`, `minute > 59` and
/// `second > 59` -> `>=`: the last valid value of each field is accepted and
/// the first invalid one is refused ("hour must be in 0..23", ...).
#[test]
fn a_timestamp_may_have_the_last_hour_minute_and_second() {
    check(&[
        ("hour 23", desc("description: 2001-12-14t23:00:00"), true),
        ("minute 59", desc("description: 2001-12-14t22:59:00"), true),
        ("second 59", desc("description: 2001-12-14t22:00:59"), true),
        ("all three", desc("description: 2001-12-14 23:59:59"), true),
        ("hour 24", desc("description: 2001-12-14t24:00:00"), false),
        ("minute 60", desc("description: 2001-12-14t23:60:00"), false),
        ("second 60", desc("description: 2001-12-14t23:59:60"), false),
    ]);
}
