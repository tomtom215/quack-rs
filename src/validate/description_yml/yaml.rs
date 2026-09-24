// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A reader for the subset of YAML that `description.yml` files use.
//!
//! quack-rs deliberately has no YAML dependency, so this is hand-rolled — but
//! it follows YAML's structure rather than scanning for `key:` prefixes: a
//! value belongs to the key whose indentation encloses it, comments are only
//! recognised outside quotes, and quoted scalars are decoded. Anything outside
//! the subset (anchors, aliases, tags, nested flow collections) is reported as
//! unsupported instead of being misread.
//!
//! Only the top-level sections the caller asks for are parsed. Everything else
//! — `docs:` above all, which is free-form prose — is skipped by indentation
//! alone, so text in it can never be mistaken for metadata.

mod scalar;

use scalar::{
    block_scalar, flow_seq, join_quoted_lines, plain_scalar, quoted_scalar, split_key,
    starts_quoted, strip_comment,
};

/// A parsed YAML value, reduced to what the `description.yml` fields need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Value {
    /// An empty value, `~` or `null`.
    Null,
    /// A plain, quoted or block scalar, decoded.
    Scalar(String),
    /// A block (`- item`) or flow (`[a, b]`) sequence.
    Seq(Vec<Self>),
    /// A nested mapping. Its contents are not needed by any field, so they are
    /// not kept — but its keys never leak into the enclosing mapping.
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

/// Whether a value's text opens a quoted, flow or block scalar, or a
/// construct this reader refuses.
fn starts_structured(text: &str) -> bool {
    text.starts_with(['"', '\'', '[', '{', '|', '>', '&', '*', '!'])
}

/// Whether the value after `key:` — `rest` on the key's line, then
/// `children` — is a plain scalar, deciding exactly as `parse_value` does.
fn is_plain(rest: &str, children: &[Line<'_>]) -> bool {
    let rest = strip_comment(rest.trim_start());
    if !rest.is_empty() {
        return !starts_structured(rest);
    }
    children
        .iter()
        .find(|l| l.is_content())
        .is_some_and(|child| {
            let mapping = split_key(child.text).is_some() && !starts_quoted(child.text);
            !(child.is_seq_item() || mapping || starts_structured(child.text))
        })
}

/// A top-level section such as `extension:`, with its entries in order.
#[derive(Debug)]
pub(super) struct Section {
    pub(super) name: String,
    pub(super) entries: Vec<Entry>,
}

/// One physical line of the document.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    /// 1-based line number.
    number: usize,
    /// Leading spaces.
    indent: usize,
    /// The line with its indentation removed and trailing whitespace trimmed.
    text: &'a str,
}

impl Line<'_> {
    const fn is_blank(&self) -> bool {
        self.text.is_empty()
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

/// Reads `content` and parses the bodies of the top-level sections named in
/// `wanted`; other sections are skipped. A UTF-8 byte-order mark is ignored.
///
/// # Errors
///
/// A message naming the offending line when the document is malformed or
/// uses YAML outside the supported subset.
pub(super) fn read_sections(content: &str, wanted: &[&str]) -> Result<Vec<Section>, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut sections: Vec<(Section, Vec<Line<'_>>)> = Vec::new();
    let mut seen: Vec<(&str, usize)> = Vec::new();
    // `current` is the wanted section being collected, if any; `seen` is
    // non-empty once any top-level key (wanted or not) has opened a section.
    let mut current: Option<usize> = None;
    let mut seen_content = false;

    for (index, raw) in content.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_end();
        let starts_indented = line.starts_with([' ', '\t']);
        let text = line.trim_start();

        if starts_indented || text.is_empty() || text.starts_with('#') {
            if let Some(i) = current {
                let indent = line.len() - line.trim_start_matches(' ').len();
                if line[indent..].starts_with('\t') && !text.is_empty() {
                    return Err(format!(
                        "line {number}: a tab is used for indentation, which YAML forbids"
                    ));
                }
                sections[i].1.push(Line {
                    number,
                    indent,
                    text,
                });
            } else if seen.is_empty()
                && starts_indented
                && !text.is_empty()
                && !text.starts_with('#')
            {
                return Err(format!(
                    "line {number}: indented text before any top-level key"
                ));
            }
            continue;
        }

        if text == "---" && !seen_content {
            seen_content = true;
            continue;
        }
        if text == "---" || text == "..." {
            return Err(format!(
                "line {number}: only a single YAML document is supported"
            ));
        }
        seen_content = true;

        let (key, rest) = split_key(text)
            .ok_or_else(|| format!("line {number}: expected a top-level `key:`, got '{text}'"))?;
        if let Some(&(_, first)) = seen.iter().find(|(k, _)| *k == key) {
            return Err(format!(
                "line {number}: duplicate top-level key '{key}' (first on line {first})"
            ));
        }
        seen.push((key, number));

        if wanted.contains(&key) {
            if !strip_comment(rest).is_empty() {
                return Err(format!(
                    "line {number}: '{key}' must be a block mapping of `key: value` lines"
                ));
            }
            sections.push((
                Section {
                    name: key.to_string(),
                    entries: Vec::new(),
                },
                Vec::new(),
            ));
            current = Some(sections.len() - 1);
        } else {
            current = None;
        }
    }

    sections
        .into_iter()
        .map(|(mut section, body)| {
            section.entries = parse_mapping(&body, &section.name)?;
            Ok(section)
        })
        .collect()
}

/// Parses the body of a block mapping whose entries all sit at one indent.
fn parse_mapping(lines: &[Line<'_>], path: &str) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let Some(field_indent) = lines.iter().find(|l| l.is_content()).map(|l| l.indent) else {
        return Ok(entries);
    };

    // Each pass consumes the key's line and then its value's lines, so the
    // loop always advances and ends when the slice is used up.
    let mut remaining = lines;
    while let Some((line, after)) = remaining.split_first() {
        remaining = after;
        if !line.is_content() {
            continue;
        }
        if line.indent != field_indent {
            return Err(format!(
                "line {}: inconsistent indentation in '{path}' (expected {field_indent} spaces, \
                 found {})",
                line.number, line.indent
            ));
        }
        let (key, rest) = split_key(line.text).ok_or_else(|| {
            format!(
                "line {}: expected `key: value` in '{path}', got '{}'",
                line.number, line.text
            )
        })?;
        if let Some(first) = entries.iter().find(|e| e.key == key) {
            return Err(format!(
                "line {}: duplicate key '{path}.{key}' (first on line {})",
                line.number, first.line
            ));
        }

        let seq_at_key_level = strip_comment(rest).is_empty();
        let (children, after_value) =
            remaining.split_at(value_len(remaining, field_indent, seq_at_key_level));
        remaining = after_value;
        let value = parse_value(rest, children, field_indent, line.number)?;
        entries.push(Entry {
            key: key.to_string(),
            line: line.number,
            value,
            plain: is_plain(rest, children),
        });
    }
    Ok(entries)
}

/// How many of `lines`, the lines after a key at `field_indent`, are the
/// value's own: everything indented deeper, plus a sequence written level with
/// its key (`key:\n- item`), which YAML allows.
fn value_len(lines: &[Line<'_>], field_indent: usize, seq_at_key_level: bool) -> usize {
    (0..lines.len())
        .find(|&n| {
            let next = &lines[n];
            // A comment indented past the key may be block-scalar content, so
            // it is kept; a blank line, or a comment at the key's level or
            // shallower, belongs to the value only if more of the value
            // follows it.
            let take = if next.is_blank() || (next.is_comment() && next.indent <= field_indent) {
                continues_after(&lines[n..], field_indent, seq_at_key_level)
            } else {
                belongs_to_value(next, field_indent, seq_at_key_level)
            };
            !take
        })
        .unwrap_or(lines.len())
}

/// Whether a line sits inside the value of a key at `field_indent`: indented
/// deeper, or a sequence item level with the key when the key's own line
/// carried no value.
fn belongs_to_value(line: &Line<'_>, field_indent: usize, seq_at_key_level: bool) -> bool {
    line.indent > field_indent
        || (seq_at_key_level && line.indent == field_indent && line.is_seq_item())
}

/// Whether the blank/comment lines at the head of `rest` are followed by more
/// of the current value (so they belong to it) rather than by the next key.
fn continues_after(rest: &[Line<'_>], field_indent: usize, seq_at_key_level: bool) -> bool {
    rest.iter()
        .find(|l| l.is_content())
        .is_some_and(|l| belongs_to_value(l, field_indent, seq_at_key_level))
}

/// Parses the value that follows `key:` — `rest` on the key's own line, and
/// `children`, the lines that belong to it.
fn parse_value(
    rest: &str,
    children: &[Line<'_>],
    parent_indent: usize,
    line: usize,
) -> Result<Value, String> {
    let rest = rest.trim_start();
    let first_child = children.iter().find(|l| l.is_content());

    if strip_comment(rest).is_empty() {
        return match first_child {
            None => Ok(Value::Null),
            Some(child) if child.is_seq_item() => parse_seq(children, child.indent),
            Some(child) if split_key(child.text).is_some() && !starts_quoted(child.text) => {
                Ok(Value::Mapping)
            }
            // Any other value that starts on the next line reads as if it
            // started on the key's own: `name:\n  "x"` is `x`, not `"x"` with
            // its quotes, and a plain one folds the lines below it.
            Some(child) => {
                let at = children
                    .iter()
                    .position(Line::is_content)
                    .map_or(children.len(), |i| i + 1);
                parse_value(child.text, &children[at..], parent_indent, child.number)
            }
        };
    }

    match rest.as_bytes()[0] {
        b'|' | b'>' => block_scalar(rest, children, parent_indent, line),
        b'"' | b'\'' => {
            let text = join_quoted_lines(rest, children);
            let (value, remainder) = quoted_scalar(&text, line)?;
            if !strip_comment(remainder).is_empty() {
                return Err(format!(
                    "line {line}: unexpected text after a quoted value: '{}'",
                    remainder.trim()
                ));
            }
            Ok(Value::Scalar(value))
        }
        b'[' => {
            let mut text = strip_comment(rest).to_string();
            for child in children.iter().filter(|l| l.is_content()) {
                text.push(' ');
                text.push_str(strip_comment(child.text));
            }
            flow_seq(&text, line)
        }
        b'{' => Ok(Value::Mapping),
        b'&' | b'*' | b'!' => Err(format!(
            "line {line}: YAML anchors, aliases and tags are not supported in description.yml"
        )),
        _ => plain_scalar(rest, children, line),
    }
}

/// A block sequence whose `- ` markers sit at `seq_indent`.
fn parse_seq(lines: &[Line<'_>], seq_indent: usize) -> Result<Value, String> {
    let mut items = Vec::new();
    // As in `parse_mapping`, each pass consumes at least the item's own line.
    let mut remaining = lines;
    while let Some((line, after)) = remaining.split_first() {
        remaining = after;
        if !line.is_content() {
            continue;
        }
        if line.indent != seq_indent || !line.is_seq_item() {
            return Err(format!(
                "line {}: expected a `- item` at indentation {seq_indent}, got '{}'",
                line.number, line.text
            ));
        }
        // The item runs up to the next content line at the marker's
        // indentation or shallower; blank lines and comments never end it.
        let len = remaining
            .iter()
            .position(|l| l.is_content() && l.indent <= seq_indent)
            .unwrap_or(remaining.len());
        let (children, after_item) = remaining.split_at(len);
        remaining = after_item;
        let item = line.text[1..].trim_start();
        let value = if split_key(item).is_some() && !starts_quoted(item) {
            Value::Mapping
        } else {
            parse_value(item, children, seq_indent, line.number)?
        };
        items.push(value);
    }
    Ok(Value::Seq(items))
}
