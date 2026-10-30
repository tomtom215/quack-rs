// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A reader for the subset of YAML that `description.yml` files use.
//!
//! quack-rs deliberately has no YAML dependency, so this is hand-rolled — but
//! it follows YAML's structure rather than scanning for `key:` prefixes: a
//! value belongs to the key whose indentation encloses it, comments are only
//! recognised outside quotes, and quoted scalars are decoded.
//!
//! The yardstick is `PyYAML`'s `yaml.safe_load`, which the community build
//! (`scripts/build.py` in `duckdb/community-extensions`) runs first: a file it
//! refuses never builds, so this reader refuses it too. That is why **every**
//! section is read, not only the ones whose values the caller wants: a syntax
//! error in `docs:` fails the community build as surely as one in
//! `extension:`. Other sections are checked and their values discarded
//! ([`Mode::Check`]); there, quoted and duplicate keys (which `PyYAML` accepts,
//! keeping the last) and anchors with their aliases are accepted. The sections
//! the caller reads ([`Mode::Read`]) take plain keys only, once each, and no
//! anchors, aliases or nested flow collections.
//!
//! Refused everywhere although `PyYAML` reads them, as no descriptor uses them:
//! tags (`!!str`), explicit `? key` entries, directives, a document end
//! (`...`) or a second document, and a top-level key in quotes.
//!
//! Each value consumes its own lines: a plain or block scalar and a nested
//! collection the lines indented past its key, a quoted scalar or flow
//! collection everything up to where it closes, since `PyYAML` does not
//! check the indentation of those.

mod block;
mod flow;
mod scalar;
mod value;

use std::collections::HashMap;

use block::{check_plain_key, is_mapping_text, parse_mapping, parse_seq, Block};
use scalar::{check_code, split_comment, split_key, starts_quoted, strip_comment};
use value::{parse_value, ValueAt};

/// How deeply mappings, sequences and flow collections may nest. `PyYAML`'s
/// recursive parser stops near Python's recursion limit; no published
/// descriptor nests more than three levels, and the bound keeps a hostile file
/// from exhausting the stack.
const MAX_DEPTH: usize = 64;

/// `PyYAML` refuses a block-mapping key longer than this ("simple keys" are
/// limited to 1024 characters before the `:`).
const MAX_KEY_LEN: usize = 1024;

/// A parsed YAML value, reduced to what the `description.yml` fields need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Value {
    /// An empty value, `~` or `null`.
    Null,
    /// A plain, quoted or block scalar, decoded.
    Scalar(String),
    /// A block (`- item`) or flow (`[a, b]`) sequence.
    Seq(Vec<Self>),
    /// A nested mapping. Its contents are checked but not kept, since no field
    /// needs them — and its keys never leak into the enclosing mapping.
    Mapping,
}

/// One `key: value` pair of a section.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) key: String,
    /// 1-based line of the key, for error messages.
    pub(super) line: usize,
    pub(super) value: Value,
    /// Whether the value is a plain (unquoted, non-block) scalar, which a
    /// YAML 1.1 reader may resolve to a boolean, number, date or null.
    pub(super) plain: bool,
}

/// A top-level section such as `extension:`, with its entries in order.
#[derive(Debug)]
pub(super) struct Section {
    pub(super) name: String,
    pub(super) entries: Vec<Entry>,
}

/// What a mapping is read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A section whose fields the caller reads.
    Read,
    /// Anything else: checked as `PyYAML` reads it, values discarded.
    Check,
}

/// What the reader carries through the whole document.
#[derive(Default)]
struct Doc {
    /// Anchors defined so far, and whether each names a collection: an alias
    /// may only name one of these, and stands for a node of its kind.
    anchors: HashMap<String, bool>,
}

impl Doc {
    /// Defines the anchor `name`; `false` if it already exists, which
    /// `PyYAML` refuses ("found duplicate anchor").
    fn define(&mut self, name: &str) -> bool {
        if self.anchors.contains_key(name) {
            return false;
        }
        self.anchors.insert(name.to_string(), false);
        true
    }

    /// Records what the anchor `name` turned out to name.
    fn resolve(&mut self, name: &str, value: &Value) {
        if let Some(collection) = self.anchors.get_mut(name) {
            *collection = matches!(value, Value::Seq(_) | Value::Mapping);
        }
    }

    /// The node the alias `name` stands for, as far as the reader needs it
    /// (a collection or not), or `None` for an undefined anchor.
    fn alias(&self, name: &str) -> Option<Value> {
        self.anchors.get(name).map(|&collection| {
            if collection {
                Value::Mapping
            } else {
                Value::Null
            }
        })
    }
}

/// One physical line of the document.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    /// 1-based line number.
    number: usize,
    /// Leading spaces (for a whitespace-only line, all of its spaces).
    indent: usize,
    /// The line after its indentation, with trailing spaces trimmed. Tabs are
    /// kept: where one may appear depends on what the line holds.
    text: &'a str,
    /// The whole line, without its line break.
    raw: &'a str,
}

impl Line<'_> {
    /// Empty once the indentation is gone, or tabs only.
    fn is_blank(&self) -> bool {
        self.text.bytes().all(|b| b == b'\t')
    }

    fn is_comment(&self) -> bool {
        self.text.starts_with('#')
    }

    fn is_content(&self) -> bool {
        !self.is_blank() && !self.is_comment()
    }

    fn is_seq_item(&self) -> bool {
        self.text == "-" || self.text.starts_with("- ")
    }
}

/// The characters that end a line for the community build: `\r\n`, a lone
/// `\r` and `\n` (Python's universal newlines, in `build.py`'s `open()`), and
/// NEL, LINE SEPARATOR and PARAGRAPH SEPARATOR, which `PyYAML` reads as line
/// breaks itself.
const LINE_BREAKS: [char; 5] = ['\r', '\n', '\u{85}', '\u{2028}', '\u{2029}'];

/// Splits `content` into lines where the community build does (see
/// [`LINE_BREAKS`]) and refuses any character outside YAML's printable set,
/// which `PyYAML` refuses anywhere in a file — `docs:` included.
fn split_lines(content: &str) -> Result<Vec<Line<'_>>, String> {
    let mut lines = Vec::new();
    let mut rest = content;
    let mut number = 0;
    loop {
        number += 1;
        let (raw, next) = rest.find(LINE_BREAKS).map_or((rest, None), |at| {
            let len = if rest[at..].starts_with("\r\n") {
                2
            } else {
                rest[at..].chars().next().map_or(1, char::len_utf8)
            };
            (&rest[..at], Some(&rest[at + len..]))
        });
        if let Some(bad) = raw.chars().find(|&c| !is_yaml_printable(c)) {
            return Err(format!(
                "line {number}: the control character U+{:04X} is not allowed in YAML (PyYAML, \
                 which the community build reads this file with, refuses the whole file)",
                u32::from(bad)
            ));
        }
        let indent = raw.len() - raw.trim_start_matches(' ').len();
        lines.push(Line {
            number,
            indent,
            text: raw[indent..].trim_end_matches(' '),
            raw,
        });
        match next {
            Some(next) => rest = next,
            None => return Ok(lines),
        }
    }
}

/// Checks lines that hold no value: a blank line may hold no tab (a comment
/// may hold anything).
fn check_skipped(lines: &[Line<'_>]) -> Result<(), String> {
    lines.iter().try_for_each(check_blank_or_comment)
}

fn check_blank_or_comment(line: &Line<'_>) -> Result<(), String> {
    if !line.is_comment() && line.text.contains('\t') {
        return Err(tab_error(line.number));
    }
    Ok(())
}

/// The index of the first content line of `lines` at or after `from`, or
/// `lines.len()`.
fn next_content(lines: &[Line<'_>], from: usize) -> usize {
    lines[from..]
        .iter()
        .position(Line::is_content)
        .map_or(lines.len(), |p| from + p)
}

fn tab_error(line: usize) -> String {
    format!(
        "line {line}: a tab is used for indentation or between tokens, which YAML forbids \
         (PyYAML accepts a tab only inside quotes, block scalars and comments); use spaces"
    )
}

fn too_deep(line: usize) -> String {
    format!("line {line}: nested more than {MAX_DEPTH} levels deep")
}

/// Reads `content` and returns the bodies of the top-level sections named in
/// `wanted`; every other section is checked and discarded. A UTF-8
/// byte-order mark is ignored.
///
/// # Errors
///
/// A message naming the offending line when the document is malformed, uses
/// YAML outside the supported subset, or would be refused by `PyYAML`.
pub(super) fn read_sections(content: &str, wanted: &[&str]) -> Result<Vec<Section>, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let lines = split_lines(content)?;
    let mut doc = Doc::default();
    let mut sections = Vec::new();
    // Top-level keys by first line: a map, so a file of many keys is linear.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut seen_content = false;

    let mut i = 0;
    loop {
        let j = next_content(&lines, i);
        check_skipped(&lines[i..j])?;
        let Some(line) = lines.get(j) else {
            return Ok(sections);
        };
        if line.text.starts_with('\t') {
            return Err(tab_error(line.number));
        }
        if line.indent > 0 {
            return Err(format!(
                "line {}: indented text before any top-level key",
                line.number
            ));
        }
        let text = line.text;
        // `---` or `...` then a space or the line end, at column 0, is a
        // document marker to PyYAML (`---x` is a plain scalar).
        if let Some(after) = text
            .strip_prefix("---")
            .or_else(|| text.strip_prefix("..."))
        {
            if after.is_empty() || after.starts_with([' ', '\t']) {
                let only_a_comment = strip_comment(after).is_empty() && !after.contains('\t');
                if text.starts_with("---") && !seen_content && only_a_comment {
                    seen_content = true;
                    i = j + 1;
                    continue;
                }
                // The end of the document: only comments may follow it.
                let rest = &lines[j + 1..];
                if text.starts_with("...") && only_a_comment && next_content(rest, 0) == rest.len()
                {
                    check_skipped(rest)?;
                    return Ok(sections);
                }
                return Err(format!(
                    "line {}: only a single YAML document, without text on its `---` line, is \
                     supported",
                    line.number
                ));
            }
        }
        seen_content = true;

        let (key, rest) = split_key(text).ok_or_else(|| {
            format!(
                "line {}: expected a top-level `key:`, got '{text}'",
                line.number
            )
        })?;
        check_plain_key(text, key, rest, line.number)?;
        if let Some(first) = seen.insert(key, line.number) {
            return Err(format!(
                "line {}: duplicate top-level key '{key}' (first on line {first})",
                line.number
            ));
        }
        let after = &lines[j + 1..];
        let used = if wanted.contains(&key) {
            nothing_but_a_comment(rest, line.number, &format!("'{key}:'")).map_err(|_| {
                format!(
                    "line {}: '{key}' must be a block mapping of `key: value` lines",
                    line.number
                )
            })?;
            let at = next_content(after, 0);
            let (entries, used) = match after.get(at) {
                Some(first) if first.indent > 0 => {
                    parse_mapping(Block::new(None, after), key, Mode::Read, &mut doc, 1)?
                }
                Some(first) if first.is_seq_item() => {
                    return Err(format!(
                        "line {}: '{key}' must be a block mapping of `key: value` lines",
                        line.number
                    ))
                }
                _ => (Vec::new(), 0),
            };
            sections.push(Section {
                name: key.to_string(),
                entries,
            });
            used
        } else {
            let value = ValueAt {
                rest,
                lines: after,
                parent_indent: 0,
                line: line.number,
                seq_at_key_level: strip_comment(rest).is_empty(),
            };
            parse_value(&value, key, Mode::Check, &mut doc, 1)?.1
        };
        i = j + 1 + used;
    }
}

/// How many of `lines`, the lines after a key at `field_indent`, are the
/// value's own by indentation: everything indented deeper, plus a sequence
/// written level with its key (`key:\n- item`), which YAML allows.
fn value_len(lines: &[Line<'_>], field_indent: usize, seq_at_key_level: bool) -> usize {
    // A blank line, or a comment at the key's level or shallower, belongs to
    // the value only if more of the value follows it. Every line of one such
    // run gets the same answer (it depends only on the first content line
    // after the run), so it is worked out once per run: asking again for
    // each line made a run of k blank lines cost k² steps.
    let mut run_answer: Option<(usize, bool)> = None; // (index of the content line, belongs)
    for (n, next) in lines.iter().enumerate() {
        // A comment indented past the key may be block-scalar content, so
        // it is kept.
        let take = if next.is_blank() || (next.is_comment() && next.indent <= field_indent) {
            match run_answer {
                Some((until, answer)) if n < until => answer,
                _ => {
                    let until = next_content(lines, n);
                    let answer = lines
                        .get(until)
                        .is_some_and(|l| belongs_to_value(l, field_indent, seq_at_key_level));
                    run_answer = Some((until, answer));
                    answer
                }
            }
        } else {
            belongs_to_value(next, field_indent, seq_at_key_level)
        };
        if !take {
            return n;
        }
    }
    lines.len()
}

/// Whether a line sits inside the value of a key at `field_indent`: indented
/// deeper, or a sequence item level with the key when the key's own line
/// carried no value.
fn belongs_to_value(line: &Line<'_>, field_indent: usize, seq_at_key_level: bool) -> bool {
    line.indent > field_indent
        || (seq_at_key_level && line.indent == field_indent && line.is_seq_item())
}

/// Whether a value's text opens a quoted, flow or block scalar, or a
/// construct this reader refuses.
fn starts_structured(text: &str) -> bool {
    text.starts_with(['"', '\'', '[', '{', '|', '>', '&', '*', '!'])
}

/// Whether the value after `key:` — `rest` on the key's line, then the
/// `used` lines — is a plain scalar, deciding exactly as `parse_value` does.
fn is_plain(rest: &str, used: &[Line<'_>]) -> bool {
    let rest = strip_comment(rest);
    if !rest.is_empty() {
        return !starts_structured(rest);
    }
    used.iter().find(|l| l.is_content()).is_some_and(|child| {
        let mapping = split_key(child.text).is_some() && !starts_quoted(child.text);
        !(child.is_seq_item() || mapping || starts_structured(child.text))
    })
}

/// The text of a value after its indicator (`:` or `-`) and the spaces that
/// follow it; a tab there is refused.
fn after_spaces(rest: &str, line: usize) -> Result<&str, String> {
    let value = rest.trim_start_matches(' ');
    if value.starts_with('\t') {
        return Err(tab_error(line));
    }
    Ok(value)
}

/// `text`, what follows a complete value on its last line, may hold only
/// spaces and a comment.
fn nothing_but_a_comment(text: &str, line: usize, what: &str) -> Result<(), String> {
    let code = split_comment(text).0;
    check_code(code, line)?;
    if !code.trim_matches(' ').is_empty() {
        return Err(format!(
            "line {line}: unexpected text after {what}: '{}'",
            code.trim_matches(' ')
        ));
    }
    Ok(())
}

/// The lines after a complete value that still lie inside it may only be
/// blank or comments.
fn nothing_but_comments(lines: &[Line<'_>], what: &str) -> Result<(), String> {
    for line in lines {
        if line.is_content() {
            return Err(format!(
                "line {}: unexpected text after {what}: '{}'",
                line.number, line.text
            ));
        }
        check_blank_or_comment(line)?;
    }
    Ok(())
}

/// Whether `c` is in YAML 1.1's printable set (`c-printable`), which is also
/// the set `PyYAML` accepts anywhere in a stream: tab, line feed, carriage
/// return, `U+0020`–`U+007E`, `U+0085`, `U+00A0`–`U+D7FF`, `U+E000`–`U+FFFD`
/// and everything above `U+FFFF`. NUL, the other C0 controls, DEL and the C1
/// controls other than `U+0085` are not.
const fn is_yaml_printable(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\r'
            | ' '..='~'
            | '\u{85}'
            | '\u{a0}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..
    )
}
