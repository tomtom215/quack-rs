// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `description.yml` text is accepted exactly when `PyYAML`'s `yaml.safe_load`
//! — the first thing the community-extensions build runs on the file —
//! accepts it, and a hostile file cannot make the reader take quadratic time.
//!
//! Every row below was run through `yaml.safe_load` (`PyYAML` 6.0.3); the
//! comment on a refused row is the first line of its error. The table is the
//! reviewers' probes (`conform`, `harden`) plus one probe per rule the reader
//! implements, each with an accepted neighbour where one exists. A
//! differential run of the same reader over the 395 published descriptors of
//! `duckdb/community-extensions` (`845779e`), 6,665 targeted and 34,758
//! random mutants of them found no file `PyYAML` refuses that this reader
//! accepts, and no published file it refuses.

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

/// The table in the `conform` reviewer's report (CONFORM-1): each was accepted.
#[test]
fn the_conform_probes_are_refused() {
    check(&[
        // mapping values are not allowed here
        ("plain `: `", desc("description: Fast: analytics"), false),
        // found character '@' that cannot start any token
        ("plain @", desc("description: @x"), false),
        // found character '`' that cannot start any token
        ("plain backtick", desc("description: `x`"), false),
        // mapping values are not allowed here
        ("docs `: `", docs("extended_description: Foo: bar"), false),
        // while parsing a block mapping ... expected <block end>
        (
            "docs dedented after a block scalar",
            top("docs:\n  hello_world: |\n    SELECT 1;\n dedented\n"),
            false,
        ),
        // while scanning a quoted scalar ... found unexpected end of stream
        (
            "docs unterminated quote",
            docs("extended_description: \"foo"),
            false,
        ),
        // found character '%' that cannot start any token
        ("docs %", docs("hello_world: %foo"), false),
        ("docs @", docs("hello_world: @foo"), false),
        ("docs backtick", docs("hello_world: `x`"), false),
        // found character '\t' that cannot start any token
        ("docs tab indent", top("docs:\n\thello_world: x\n"), false),
        // found undefined alias 'alias'
        ("docs undefined alias", docs("hello_world: *alias"), false),
        // while parsing a flow mapping ... expected ',' or '}'
        (
            "docs unclosed flow mapping",
            docs("hello_world: {a: 1"),
            false,
        ),
        // found character '\t' that cannot start any token
        (
            "tab after the name colon",
            desc("description: x").replace("name: my_ext", "name:\tmy_ext"),
            false,
        ),
        // sequence entries are not allowed here
        ("plain `- `", desc("description: - x"), false),
        // mapping keys are not allowed here
        ("plain `? `", desc("description: ? x"), false),
    ]);
}

#[test]
fn plain_scalars_follow_pyyamls_rules() {
    check(&[
        // mapping values are not allowed here
        ("final `:`", desc("description: trailing colon:"), false),
        ("`:` then a tab", desc("description: a:\tb"), false),
        ("`: ` leading", desc("description: : x"), false),
        // found character '%' that cannot start any token
        ("leading %", desc("description: %d things"), false),
        // while parsing a block node ... expected the node content, but found ','
        ("leading ,", desc("description: , x"), false),
        ("leading ]", desc("description: ] x"), false),
        ("leading }", desc("description: } x"), false),
        // found character '\t' that cannot start any token
        ("tab inside", desc("description: a\tb"), false),
        ("tab at the end", desc("description: ab\t"), false),
        ("tab before a comment", desc("description: ab\t# c"), false),
        // while parsing a block mapping (a comment line ends a plain scalar)
        (
            "text after a comment line",
            desc("description: first\n    # c\n    more"),
            false,
        ),
        // mapping values are not allowed here
        (
            "continued `key: value`",
            desc("description: first\n    key: second"),
            false,
        ),
        (
            "continued final `:`",
            desc("description: first\n    second:"),
            false,
        ),
        // found character '\t' that cannot start any token
        (
            "continued with a tab",
            desc("description: first\n    sec\tond"),
            false,
        ),
        // month must be in 1..12 / day is out of range for month
        (
            "a date that cannot exist",
            desc("description: 2024-13-01"),
            false,
        ),
        ("a date in docs", docs("hello_world: 2024-02-30"), false),
        // Accepted, and so not refused here:
        ("`?x`", desc("description: ?maybe"), true),
        ("`-x`", desc("description: -dash"), true),
        ("`:x`", desc("description: :x"), true),
        ("` | `", desc("description: a | b"), true),
        ("`C#`", desc("description: C# and F#1"), true),
        ("a URL", desc("description: see http://example.com/x"), true),
        (
            "a time",
            desc("description: opens at 10:30 - closes later"),
            true,
        ),
        ("`a:b`", desc("description: a:b"), true),
        (
            "continued with `- `",
            desc("description: first\n    - second"),
            true,
        ),
        (
            "continued with @",
            desc("description: first\n    @second"),
            true,
        ),
        (
            "continued with a backtick",
            desc("description: first\n    `second`"),
            true,
        ),
        (
            "a blank line inside",
            desc("description: first\n\n    second"),
            true,
        ),
        ("a leap day", docs("hello_world: 2024-02-29"), true),
        ("a tab in a comment", desc("description: x # a\tb"), true),
    ]);
}

/// `PyYAML` skips only spaces between tokens: a tab outside quotes, block
/// scalar content and comments is refused wherever it is.
#[test]
fn tabs_follow_pyyamls_rules() {
    check(&[
        // found character '\t' that cannot start any token, every one
        ("after the colon", desc("description:\tvalue"), false),
        (
            "space then tab after the colon",
            desc("description: \tvalue"),
            false,
        ),
        ("after a quoted value", desc("description: \"ab\"\t"), false),
        ("after a section key", top("docs:\t\n  a: b\n"), false),
        ("a tab-only line", desc("description: x\n\t"), false),
        ("spaces then a tab", desc("description: x\n  \t"), false),
        (
            "a tab-only line in docs",
            top("docs:\n  a: b\n\t\n  c: d\n"),
            false,
        ),
        (
            "a tab-only line in a block scalar",
            top("docs:\n  a: |\n    l1\n\t\n    l2\n"),
            false,
        ),
        ("before the colon", desc("description\t: x"), false),
        ("after a `- `", top("docs:\n  a:\n    - \tx\n"), false),
        // Accepted: past a block scalar's indentation, and inside quotes.
        (
            "past the block indentation",
            top("docs:\n  a: |\n    l1\n    \tl2\n"),
            true,
        ),
        (
            "continuing a quoted value",
            top("docs:\n  a: \"first\n\tsecond\"\n"),
            true,
        ),
    ]);
}

#[test]
fn quoted_scalars_follow_pyyamls_rules() {
    check(&[
        // while scanning a double-quoted scalar ... expected escape sequence of
        // 2 hexadecimal numbers, but found '+'
        ("`\\x+1`", desc("description: \"\\x+1\""), false),
        ("`\\u+041`", desc("description: \"\\u+041\""), false),
        // found unknown escape character 'q'
        ("unknown escape in docs", docs("a: \"\\q\""), false),
        // while parsing a block mapping ... expected <block end>
        ("text after the quote", desc("description: \"a\" b"), false),
        // while scanning a quoted scalar ... found unexpected document separator
        (
            "a separator inside",
            top("docs:\n  a: \"x\n---\n  y\"\n"),
            false,
        ),
        // found unexpected end of stream
        ("unterminated, single", docs("a: 'one"), false),
        // Accepted:
        ("hex escapes", desc("description: \"\\x41\\u00e9\""), true),
        (
            "a comment right after the quote",
            desc("description: \"a\"#c"),
            true,
        ),
        ("continued at column 0", top("docs:\n  a: \"x\ny\"\n"), true),
        ("single-quoted over lines", docs("a: 'one\n  two'"), true),
    ]);
}

#[test]
fn block_scalars_follow_pyyamls_rules() {
    check(&[
        // while scanning a block scalar ... expected chomping or indentation
        // indicators, but found 'x' / '#' / '-' / '0' / '\t'
        ("`|x`", docs("a: |x\n  y"), false),
        ("`|#c`", docs("a: |#c\n  x"), false),
        ("`|--`", docs("a: |--\n  x"), false),
        ("`|0`", docs("a: |0\n  x"), false),
        ("`|` then a tab", docs("a: |\t\n  x"), false),
        // while parsing a block mapping: a leading blank line wider than the
        // first content line sets the indentation, which that line then lacks
        (
            "a wide leading blank line",
            docs("a: |\n      \n  x"),
            false,
        ),
        (
            "a line indented less than the first",
            docs("a: |\n      deep\n    shallow"),
            false,
        ),
        // Accepted:
        ("`|2-`", docs("a: |2-\n    x"), true),
        (
            "a shallower comment, then a key",
            docs("a: |\n      deep\n     # c\nc: d"),
            true,
        ),
    ]);
}

#[test]
fn structure_follows_pyyamls_rules() {
    check(&[
        // mapping values are not allowed here
        ("a deeper line after a value", docs("a: 1\n  b: 2"), false),
        // while parsing a block mapping ... expected <block end>
        (
            "a line between two indentations",
            top("docs:\n    a: 1\n  b: 2\n"),
            false,
        ),
        // while parsing a block collection ... expected <block end>
        (
            "a misaligned key under `- `",
            docs("a:\n  - b: 1\n   c: 2"),
            false,
        ),
        // while scanning a block scalar / for the next token
        ("a key starting with |", docs("|hello: x"), false),
        ("a key starting with @", docs("@hello: x"), false),
        // mapping values are not allowed here (simple keys stop at 1024)
        (
            "a 1025-character key",
            docs(&format!("{}: v", "k".repeat(1025))),
            false,
        ),
        // while constructing a mapping ... expected a mapping or list of mappings
        ("`<<` merging a scalar", docs("<<: 1"), false),
        // expected a single document in the stream
        ("a second document", top("---\nfoo: 1\n"), false),
        ("text on a `---` line", top("---  hello: x\n"), false),
        // while scanning a simple key ... could not find expected ':'
        ("a top-level scalar", top("just text\n"), false),
        // Accepted:
        ("a duplicate key in docs", docs("a: 1\na: 2"), true),
        ("a quoted key in docs", docs("\"a\": 1"), true),
        (
            "a mapping on a `- ` line",
            docs("a:\n  - b: 1\n    c: 2"),
            true,
        ),
        (
            "a sequence on a `- ` line",
            docs("a:\n  - - x\n    - y"),
            true,
        ),
        ("a key starting with `-x`", docs("-test_config: x"), true),
        (
            "a 1024-character key",
            docs(&format!("{}: v", "k".repeat(1024))),
            true,
        ),
        ("`<<` merging a mapping", docs("<<: {a: 1}"), true),
        (
            "`<<` merging an alias",
            top("x: &d {a: 1}\ny:\n  <<: *d\n"),
            true,
        ),
        ("a document end", top("...\n# done\n"), true),
    ]);
}

#[test]
fn anchors_aliases_and_flow_collections_follow_pyyamls_rules() {
    check(&[
        // found duplicate anchor 'x'
        ("a duplicate anchor", docs("a: &x 1\nb: &x 2"), false),
        // while scanning an anchor ... expected alphabetic or numeric character
        ("an anchor with a dot", docs("a: &x.y 1"), false),
        // while parsing a block mapping ... expected <block end>
        ("text after an alias", docs("a: &x 1\nb: *x y"), false),
        // could not determine a constructor for the tag '!foo'
        ("an unknown tag", docs("a: !foo 1"), false),
        // while parsing a flow node ... expected the node content
        ("an empty flow item", docs("a: [x,,y]"), false),
        ("a leading comma", docs("a: [,x]"), false),
        ("`- ` in a flow sequence", docs("a: [- x]"), false),
        ("`:x` in a flow sequence", docs("a: [:x]"), false),
        // while parsing a flow sequence ... expected ',' or ']'
        ("a quoted then a plain item", docs("a: [\"x\" y]"), false),
        ("an unclosed flow sequence", docs("a: [x, y"), false),
        // found character '|' / '\t' / '@' that cannot start any token
        ("`|` in a flow sequence", docs("a: [|]"), false),
        ("a tab in a flow sequence", docs("a: [x,\ty]"), false),
        ("`@` in a flow sequence", docs("a: [@x]"), false),
        // while constructing a mapping ... found unhashable key
        ("a flow sequence as a key", docs("a: {[x]: 1}"), false),
        // while parsing a block mapping ... expected <block end>
        ("text after a flow collection", docs("a: [x] y"), false),
        // Accepted:
        (
            "an alias of an earlier anchor",
            docs("a: &x 1\nb: *x"),
            true,
        ),
        ("an anchored section", top("docs: &d\n  a: 1\n"), true),
        ("a trailing comma", docs("a: [x, y,]"), true),
        ("two words in one item", docs("a: [x y]"), true),
        ("a plain then a quoted word", docs("a: [x \"y\"]"), true),
        ("a one-pair mapping item", docs("a: [x: 1]"), true),
        ("flow keys without values", docs("a: {x, y}"), true),
        ("a comment after a comma", docs("a: [x,#c\n  y]"), true),
        ("a comment after the close", docs("a: [x] # c"), true),
        ("continued at column 0", top("docs:\n  a: [x,\ny]\n"), true),
    ]);
}

/// `PyYAML` refuses a stream holding any character outside YAML's printable
/// set ("unacceptable character #x001b: special characters are not
/// allowed"), in `docs:` as much as in the fields read here.
#[test]
fn a_control_character_anywhere_is_refused() {
    for (what, c) in [
        ("ESC", '\u{1b}'),
        ("DEL", '\u{7f}'),
        ("NUL", '\0'),
        ("BEL", '\u{7}'),
    ] {
        let err =
            parse_description_yml(&desc(&format!("description: Fast {c} thing"))).expect_err(what);
        assert!(err.as_str().contains("control character"), "{what}: {err}");
        assert!(
            parse_description_yml(&docs(&format!("hello_world: |\n  a{c}b"))).is_err(),
            "{what} in docs"
        );
    }
    // A BOM in the middle is an ordinary character (PyYAML: 'a\ufeffb'), and
    // so are U+00A0 and anything above it.
    let d = parse_description_yml(&desc("description: a\u{feff}b \u{a0}é日本😀")).unwrap();
    assert_eq!(d.description, "a\u{feff}b \u{a0}é日本😀");
}

/// `\r` (alone or before `\n`), NEL, LINE SEPARATOR and PARAGRAPH SEPARATOR
/// end a line for the community build — Python's universal newlines and
/// `PyYAML` itself — so they end one here.
#[test]
fn every_line_break_the_community_build_knows_ends_a_line() {
    check(&[
        // while scanning a simple key ... could not find expected ':' (the
        // text after the break is a new line at column 0)
        (
            "NEL inside a plain value",
            desc("description: Fast \u{85}\u{a0}x thing"),
            false,
        ),
        (
            "LS inside a plain value",
            desc("description: a\u{2028}b"),
            false,
        ),
        (
            "a lone CR inside a plain value",
            desc("description: a\rb"),
            false,
        ),
        // Accepted: a break that ends the value, and one inside a block.
        (
            "NEL at the end of a value",
            desc("description: Fast\u{85}"),
            true,
        ),
        (
            "NEL starting a block line",
            top("docs:\n  a: |\n    l1\n\u{85}    l2\n"),
            true,
        ),
    ]);
    let d =
        parse_description_yml(&desc("description: \"a\u{85}b\"").replace('\n', "\r\n")).unwrap();
    assert_eq!(
        d.description, "a b",
        "a break inside quotes folds to a space"
    );
}

/// Runs `f`, failing if it takes more than five seconds. The inputs below took
/// 3.8 to 21.5 s each on 734bb15 (release build) and take milliseconds now,
/// so the bound is far from both.
fn within_five_seconds(what: &str, f: impl FnOnce() -> bool) {
    let start = std::time::Instant::now();
    let ok = f();
    let elapsed = start.elapsed();
    assert!(ok, "{what}: wrong result");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "{what}: took {elapsed:?}; the reader is quadratic again"
    );
}

#[test]
#[cfg_attr(miri, ignore = "100,000-line inputs; the bound is about native speed")]
fn a_long_run_of_blank_lines_inside_a_value_is_linear() {
    // Each blank line used to rescan the rest of the run for the next content
    // line: k blank lines cost k² steps.
    let blanks = "\n".repeat(100_000);
    let text = desc(&format!("description: first\n{blanks}    second"));
    within_five_seconds("blank lines", || {
        parse_description_yml(&text)
            .is_ok_and(|d| d.description == format!("first{}second", "\n".repeat(100_000)))
    });
}

#[test]
#[cfg_attr(miri, ignore = "100,000-line inputs; the bound is about native speed")]
fn a_long_run_of_comment_lines_inside_a_value_is_linear() {
    // A comment at the key's level belongs to the value if more of it
    // follows, and each one used to rescan the rest of the run to find out.
    // (The text is refused, as PyYAML refuses it — "while parsing a block
    // mapping": the comment ends the block scalar — but only after the run.)
    let comments = "  # c\n".repeat(100_000);
    let text = desc(&format!("description: |\n    text\n{comments}    more"));
    within_five_seconds("comment lines", || parse_description_yml(&text).is_err());
    // Between keys the same run is just comments.
    let text = desc(&format!("description: first\n{comments}  version: 1.0.0"));
    within_five_seconds("comment lines between keys", || {
        parse_description_yml(&text).is_ok()
    });
}

/// `count` lines `<indent>k<i>: v`.
fn many_keys(indent: &str, count: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for i in 0..count {
        let _ = writeln!(out, "{indent}k{i}: v");
    }
    out
}

#[test]
#[cfg_attr(miri, ignore = "100,000-line inputs; the bound is about native speed")]
fn a_file_of_many_keys_is_linear() {
    // Duplicate-key detection scanned every earlier key: 100,000 keys took
    // 12.6 s at the top level and 21.5 s inside `extension:`.
    let top_level = many_keys("", 100_000);
    within_five_seconds("top-level keys", || {
        parse_description_yml(&format!("{top_level}{}", desc("description: Fast"))).is_ok()
    });
    let inner = many_keys("  ", 100_000);
    let text =
        desc("description: Fast").replacen("extension:\n", &format!("extension:\n{inner}"), 1);
    within_five_seconds("keys inside extension", || {
        parse_description_yml(&text).is_ok()
    });
    within_five_seconds("keys inside docs", || {
        parse_description_yml(&docs(&many_keys("", 100_000))).is_ok()
    });
}

/// Nesting is bounded, so a hostile file cannot exhaust the stack: 64 levels
/// (no published descriptor nests more than three).
#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    // 100,000 levels natively. Miri interprets about a thousand times slower
    // and this parser has no `unsafe`, so under Miri a thousand levels, sixteen
    // times the limit, takes the same path: at 100,000 the test alone took 18
    // minutes of the Miri job's 60.
    let depth = if cfg!(miri) { 1_000 } else { 100_000 };
    for (what, body) in [
        ("sequence markers", format!("s:\n  {}x", "- ".repeat(depth))),
        (
            "flow sequences",
            format!("f: {}{}", "[".repeat(depth), "]".repeat(depth)),
        ),
        (
            "mappings",
            (0..200).fold(String::new(), |mut out, i| {
                out.push_str(&" ".repeat(i));
                out.push_str("k:\n");
                out
            }),
        ),
    ] {
        let err = parse_description_yml(&docs(&body)).expect_err(what);
        assert!(
            err.as_str().contains("nested more than 64"),
            "{what}: {err}"
        );
    }
    let sixty_deep = format!("f: {}{}", "[".repeat(60), "]".repeat(60));
    assert!(parse_description_yml(&docs(&sixty_deep)).is_ok());
}
