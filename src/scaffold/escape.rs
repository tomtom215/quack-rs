// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Writing configured free text into generated files without changing its
//! meaning: YAML quoting for `description.yml`, `//!` lines for `src/lib.rs`.

/// `text` as a YAML double-quoted scalar.
///
/// Quoting is what makes an ordinary description safe: unquoted, `: ` starts a
/// mapping, ` #` a comment and a leading quote a quoted scalar. Inside double
/// quotes only `\` and `"` are special, and everything outside YAML's
/// printable set — plus the characters a YAML 1.1 reader such as `PyYAML` takes
/// for line breaks (U+0085, U+2028, U+2029) and the byte-order mark — is
/// written as an escape, so the value decodes to exactly `text`.
pub(super) fn yaml_quoted(text: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' '..='~' => out.push(c),
            '\u{a0}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..
                if !matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}') =>
            {
                out.push(c);
            }
            _ => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
        }
    }
    out.push('"');
    out
}

/// `text` as `//!` doc-comment lines: every line of a multi-line description
/// gets its own prefix, so none of it escapes the comment into Rust source.
pub(super) fn doc_comment_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                "//!".to_string()
            } else {
                format!("//! {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
