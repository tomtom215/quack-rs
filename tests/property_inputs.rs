// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Property tests for the text parsers and generators that take input from
//! outside the extension: `abi::parse_version`, `validate_spdx_license`,
//! `parse_description_yml`, and `generate_scaffold`'s escaping.
//!
//! The fuzz targets in `fuzz/` only assert "never panics" (and, for
//! `duck_string`, one acceptance rule). These assert what the output must
//! *be*: a model for `parse_version`, a grammar for SPDX, a round trip for
//! the scaffold and the double-quoted scalar reader. Runs in the default
//! `cargo test`, so the Windows and macOS jobs get it too.

use proptest::prelude::*;
use quack_rs::abi::parse_version;
use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};
use quack_rs::validate::description_yml::parse_description_yml;
use quack_rs::validate::validate_spdx_license;

// ─── abi::parse_version ─────────────────────────────────────────────────────

/// What `parse_version` documents: an optional `v`, then three decimal parts.
fn version_model(s: &str) -> Option<(u64, u64, u64)> {
    let body = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<&str> = body.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut nums = [0_u64; 3];
    for (slot, part) in nums.iter_mut().zip(&parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(nums.into())
}

proptest! {
    #[test]
    fn parse_version_round_trips_every_release_triple(a: u64, b: u64, c: u64, v: bool) {
        let text = format!("{}{a}.{b}.{c}", if v { "v" } else { "" });
        prop_assert_eq!(parse_version(&text), Some((a, b, c)));
    }

    /// Strings over the alphabet a version could plausibly be mangled into.
    #[test]
    fn parse_version_agrees_with_the_documented_grammar(s in "[v0-9.+\\-_ eE]{0,14}") {
        prop_assert_eq!(parse_version(&s), version_model(&s), "{:?}", s);
    }

    /// The same, aimed at the three-part shape so that near-misses are common:
    /// each part may carry a sign or be empty (`u64::from_str` accepts a
    /// leading `+`, which no DuckDB version string has).
    #[test]
    fn parse_version_refuses_a_part_that_is_not_plain_decimal(
        v: bool,
        a in "[+\\-]?[0-9]{0,3}",
        b in "[+\\-]?[0-9]{0,3}",
        c in "[+\\-]?[0-9]{0,3}",
    ) {
        let s = format!("{}{a}.{b}.{c}", if v { "v" } else { "" });
        prop_assert_eq!(parse_version(&s), version_model(&s), "{:?}", s);
    }

    #[test]
    fn parse_version_never_panics(s in "\\PC{0,24}") {
        let _ = parse_version(&s);
    }
}

// ─── validate_spdx_license ──────────────────────────────────────────────────

const IDS: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-3-Clause",
    "GPL-3.0-only",
    "LGPL-2.1-or-later",
    "MPL-2.0",
    "ISC",
    "Unlicense",
    "LicenseRef-Internal",
];
const EXCEPTIONS: &[&str] = &["LLVM-exception", "Classpath-exception-2.0"];

#[derive(Debug, Clone)]
enum Expr {
    Id(&'static str, Option<&'static str>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Paren(Box<Self>),
}

fn render(e: &Expr, out: &mut String) {
    match e {
        Expr::Id(id, exc) => {
            out.push_str(id);
            if let Some(x) = exc {
                out.push_str(" WITH ");
                out.push_str(x);
            }
        }
        Expr::And(a, b) | Expr::Or(a, b) => {
            // AND binds tighter than OR, so an OR under an AND needs parentheses;
            // everything else may be written flat.
            let is_and = matches!(e, Expr::And(..));
            for (i, side) in [a, b].into_iter().enumerate() {
                if i == 1 {
                    out.push_str(if is_and { " AND " } else { " OR " });
                }
                let wrap = is_and && matches!(**side, Expr::Or(..));
                if wrap {
                    out.push('(');
                }
                render(side, out);
                if wrap {
                    out.push(')');
                }
            }
        }
        Expr::Paren(inner) => {
            out.push('(');
            render(inner, out);
            out.push(')');
        }
    }
}

fn expr() -> impl Strategy<Value = Expr> {
    let leaf = (
        prop::sample::select(IDS),
        prop::option::of(prop::sample::select(EXCEPTIONS)),
    )
        .prop_map(|(id, exc)| Expr::Id(id, exc));
    leaf.prop_recursive(5, 24, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::And(Box::new(a), Box::new(b))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Expr::Or(Box::new(a), Box::new(b))),
            inner.prop_map(|e| Expr::Paren(Box::new(e))),
        ]
    })
}

proptest! {
    #[test]
    fn every_expression_built_from_the_grammar_is_accepted(e in expr()) {
        let mut text = String::new();
        render(&e, &mut text);
        prop_assert!(validate_spdx_license(&text).is_ok(), "{}: {:?}", text, validate_spdx_license(&text));
    }

    /// One byte that SPDX has no use for, anywhere in an accepted expression,
    /// makes it unacceptable: the only identifiers that pass are listed ones.
    #[test]
    fn a_stray_byte_makes_an_accepted_expression_unacceptable(
        e in expr(),
        at in 0_usize..200,
        junk in prop::sample::select(vec!['!', '"', '$', '*', ',', ';', '<', '=', '>', '?', '@', '\\', '^', '`', '{', '|', '}', '~']),
    ) {
        let mut text = String::new();
        render(&e, &mut text);
        let at = text.char_indices().nth(at % (text.chars().count() + 1)).map_or(text.len(), |(i, _)| i);
        text.insert(at, junk);
        prop_assert!(validate_spdx_license(&text).is_err(), "{:?} was accepted", text);
    }

    #[test]
    fn a_dangling_operator_is_refused(e in expr(), op in prop::sample::select(vec!["AND", "OR", "WITH"]), lead: bool) {
        let mut body = String::new();
        render(&e, &mut body);
        let text = if lead { format!("{op} {body}") } else { format!("{body} {op}") };
        prop_assert!(validate_spdx_license(&text).is_err(), "{:?} was accepted", text);
    }

    #[test]
    fn unbalanced_parentheses_are_refused(e in expr(), extra in 1_usize..4, open: bool) {
        let mut body = String::new();
        render(&e, &mut body);
        let text = if open {
            format!("{}{body}", "(".repeat(extra))
        } else {
            format!("{body}{}", ")".repeat(extra))
        };
        prop_assert!(validate_spdx_license(&text).is_err(), "{:?} was accepted", text);
    }
}

// ─── description.yml: double-quoted scalars and the scaffold ────────────────

/// `s` as a YAML double-quoted scalar (JSON's escapes are a subset of YAML's).
fn yaml_dq(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}') => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Printable text with the characters that break YAML and Rust comments mixed in.
fn awkward_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        8 => "[a-zA-Z0-9 ]{1,6}",
        3 => prop::sample::select(vec![
            ": ", " #", "#", "'", "\"", "\\", "- ", "[", "]", "{", "}", "&", "*", "!", "|", ">",
            "%", "@", "`", ",", "?", "~", "\t", "\n", "\u{2028}", "\u{2029}", "\u{85}", "\u{e9}",
            "\u{65e5}", "\u{1f986}", "\u{200d}", "\u{feff}", "---", "...", "null", "true", "0.1",
        ]).prop_map(str::to_owned),
    ];
    prop::collection::vec(piece, 1..8).prop_map(|v| v.concat())
}

proptest! {
    /// The hand-written reader decodes any double-quoted scalar a writer can
    /// produce back to the text it encodes.
    #[test]
    fn double_quoted_scalars_decode_to_what_was_encoded(text in awkward_text(), who in awkward_text()) {
        prop_assume!(!text.trim().is_empty() && !who.trim().is_empty());
        let yml = format!(
            "extension:\n  name: my_ext\n  description: {}\n  version: 0.1.0\n  language: Rust\n  \
             build: cargo\n  license: MIT\n  maintainers:\n    - {}\nrepo:\n  github: a/b\n  \
             ref: 4f1c2e9a7b3d5c8e1f0a2b4c6d8e0f1a3b5c7d9e\n",
            yaml_dq(&text),
            yaml_dq(&who),
        );
        let parsed = parse_description_yml(&yml);
        prop_assert!(parsed.is_ok(), "{:?}\n{}", parsed.err().map(|e| e.to_string()), yml);
        let parsed = parsed.unwrap();
        prop_assert_eq!(parsed.description, text);
        // Maintainer names are trimmed by the reader (deliberately: a name is
        // not meant to carry padding); the description is kept as written.
        prop_assert_eq!(parsed.maintainers, vec![who.trim().to_string()]);
    }

    /// Whatever free text `generate_scaffold` accepts comes back out of the
    /// generated `description.yml` unchanged, and every description line stays
    /// inside the `//!` header of `src/lib.rs`.
    #[test]
    fn scaffold_free_text_survives_into_the_generated_files(text in awkward_text(), who in awkward_text()) {
        let config = ScaffoldConfig {
            name: "my_ext".to_string(),
            description: text.clone(),
            maintainer: who.clone(),
            github_repo: "janedoe/duckdb-my-ext".to_string(),
            ..ScaffoldConfig::default()
        };
        let Ok(files) = generate_scaffold(&config) else {
            return Ok(()); // refused: nothing was written, nothing to check
        };
        let file = |path: &str| files.iter().find(|f| f.path == path).expect(path).content.clone();
        let yml = file("description.yml");
        let parsed = parse_description_yml(&yml);
        prop_assert!(parsed.is_ok(), "{:?}\n{}", parsed.err().map(|e| e.to_string()), yml);
        let parsed = parsed.unwrap();
        prop_assert_eq!(&parsed.description, &text, "{}", yml);
        prop_assert_eq!(&parsed.maintainers, &[who], "{}", yml);

        let lib = file("src/lib.rs");
        let header: Vec<&str> = lib
            .split('\n')
            .take_while(|line| !line.starts_with("use "))
            .filter(|line| !line.is_empty())
            .collect();
        prop_assert!(header.iter().all(|l| l.starts_with("//!")), "{:?}\n{}", text, lib);
        prop_assert!(!lib.contains('\r'), "{:?}", text);
    }
}
