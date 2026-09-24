// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Tests that free-text configuration reaches the generated files intact:
//! `description.yml` must decode back to exactly what was configured, and the
//! generated `src/lib.rs` must keep every description line inside its `//!`
//! doc comment.

use super::*;
use crate::validate::description_yml::parse_description_yml;

fn config(description: &str, maintainer: &str) -> ScaffoldConfig {
    ScaffoldConfig {
        name: "my_ext".to_string(),
        description: description.to_string(),
        maintainer: maintainer.to_string(),
        github_repo: "janedoe/duckdb-my-ext".to_string(),
        ..ScaffoldConfig::default()
    }
}

fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not generated"))
        .content
}

/// Descriptions a real author could write that are not plain YAML scalars.
/// Written unquoted, each one used to corrupt `description.yml`: `: ` starts a
/// mapping, ` #` starts a comment, a leading quote starts a quoted scalar, a
/// newline escapes the field, and so on.
const ADVERSARIAL: &[&str] = &[
    "Fast: analytics for DuckDB",
    "Analytics #1 choice",
    "\"quoted\" start",
    "'single' start",
    "line one\nline two",
    "C:\\path\\to\\thing and a trailing backslash\\",
    "Unicode: caf\u{e9}, \u{65e5}\u{672c}\u{8a9e}, \u{1f986}",
    "- looks like a list item",
    "[looks, like, a flow sequence]",
    "{looks: like a mapping}",
    "&anchor *alias !tag %directive @reserved `backtick`",
    "null",
    "true",
    "~",
    "0.1",
    "tab\tinside",
    "a\u{2028}line separator and a\u{2029}paragraph separator",
];

/// Regression: `description`, the maintainer and the extended description
/// were written into `description.yml` unquoted. The fixed output must decode,
/// through quack-rs's own parser, to exactly the configured text.
#[test]
fn adversarial_text_round_trips_through_description_yml() {
    for &text in ADVERSARIAL {
        // A maintainer is one line: exercise the single-line inputs there too.
        let maintainer = if text.contains(['\n', '\u{2028}', '\u{2029}']) {
            "Jane: Doe #1"
        } else {
            text
        };
        let files = generate_scaffold(&config(text, maintainer))
            .unwrap_or_else(|e| panic!("{text:?} refused: {e}"));
        let yml = file(&files, "description.yml");
        let parsed = parse_description_yml(yml)
            .unwrap_or_else(|e| panic!("{text:?} produced an unparseable file: {e}\n{yml}"));
        assert_eq!(parsed.description, text, "{yml}");
        assert_eq!(parsed.maintainers, vec![maintainer.to_string()], "{yml}");
        assert_eq!(parsed.github, "janedoe/duckdb-my-ext", "{yml}");
        assert_eq!(parsed.version.as_deref(), Some("0.1.0"), "{yml}");
    }
}

/// The same files, decoded by a real YAML implementation (`PyYAML`, which the
/// community-extensions tooling uses). Runs only where `python3` with `PyYAML`
/// is available; quack-rs's own parser above always runs.
#[test]
#[cfg_attr(miri, ignore = "spawns python3; Miri cannot run a subprocess")]
fn adversarial_text_round_trips_through_pyyaml() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let has_pyyaml = Command::new("python3")
        .args(["-c", "import yaml"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !has_pyyaml {
        eprintln!("python3 with PyYAML not found; skipping the PyYAML cross-check");
        return;
    }

    // Prints each field as JSON, so the comparison sees exact code points.
    let script = "import json, sys, yaml\n\
                  d = yaml.safe_load(sys.stdin.read())\n\
                  e = d['extension']\n\
                  print(json.dumps([e['description'], e['maintainers'], e['version'], \
                  d['repo']['github'], d['repo']['ref'], d['docs']['extended_description']]))\n";
    for &text in ADVERSARIAL {
        let maintainer = if text.contains(['\n', '\u{2028}', '\u{2029}']) {
            "Jane: Doe #1"
        } else {
            text
        };
        let files = generate_scaffold(&config(text, maintainer)).unwrap();
        let yml = file(&files, "description.yml");

        let mut child = Command::new("python3")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn python3");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(yml.as_bytes())
            .expect("write yml");
        let out = child.wait_with_output().expect("python3 output");
        assert!(
            out.status.success(),
            "PyYAML rejected the file for {text:?}: {}\n{yml}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got = String::from_utf8(out.stdout).expect("utf-8");
        let expected = format!(
            "[{}, [{}], \"0.1.0\", \"janedoe/duckdb-my-ext\", \"{}\", {}]",
            json_string(text),
            json_string(maintainer),
            REF_PLACEHOLDER,
            json_string(text),
        );
        assert_eq!(got.trim_end(), expected, "{yml}");
    }
}

/// `json.dumps`'s default (`ensure_ascii=True`) encoding of a string.
fn json_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

/// Regression: `//! {description}` put every line after the first outside the
/// doc comment, where it was compiled as Rust.
#[test]
fn every_description_line_stays_inside_the_lib_rs_doc_comment() {
    for &text in ADVERSARIAL {
        let files = generate_scaffold(&config(text, "Jane Doe")).unwrap();
        let lib = file(&files, "src/lib.rs");
        let header: Vec<&str> = lib
            .split('\n')
            .take_while(|line| !line.starts_with("use "))
            .filter(|line| !line.is_empty())
            .collect();
        assert!(
            header.iter().all(|line| line.starts_with("//!")),
            "{text:?}:\n{lib}"
        );
        // A bare CR is a hard error in a Rust doc comment.
        assert!(!lib.contains('\r'), "{text:?}");
    }
}

#[test]
fn empty_or_padded_free_text_is_rejected() {
    for (description, maintainer) in [
        ("", "Jane"),
        ("   ", "Jane"),
        ("d", ""),
        ("d", "  "),
        (" leading space", "Jane"),
        ("trailing space ", "Jane"),
        ("d", "Jane\nDoe"),
        ("nul \0 inside", "Jane"),
        ("escape \u{1b}[31m", "Jane"),
        ("carriage\rreturn", "Jane"),
        ("next line \u{85} (C1 control)", "Jane"),
        ("bidi \u{202e} override", "Jane"),
        ("d", "Jane\u{2028}Doe"),
    ] {
        assert!(
            generate_scaffold(&config(description, maintainer)).is_err(),
            "{description:?} / {maintainer:?}"
        );
    }
}

#[test]
fn github_repo_must_be_owner_slash_repo() {
    for repo in [
        "",
        "noslash",
        "a/b/c",
        "/repo",
        "owner/",
        "own er/repo",
        "o/r#x",
        "o/r\"x",
    ] {
        let cfg = ScaffoldConfig {
            github_repo: repo.to_string(),
            ..config("d", "Jane")
        };
        assert!(generate_scaffold(&cfg).is_err(), "{repo:?}");
    }
    for repo in ["janedoe/duckdb-my-ext", "Org-1/repo.name_2"] {
        let cfg = ScaffoldConfig {
            github_repo: repo.to_string(),
            ..config("d", "Jane")
        };
        assert!(generate_scaffold(&cfg).is_ok(), "{repo:?}");
    }
}
