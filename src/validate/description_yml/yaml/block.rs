// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Block mappings (`key: value` lines) and block sequences (`- item` lines),
//! and their keys.

use std::collections::HashMap;

use super::scalar::{
    check_code, check_plain_start, check_timestamp, quoted_scalar, split_key, starts_quoted,
    strip_comment,
};
use super::{
    after_spaces, check_skipped, is_plain, next_content, parse_value, tab_error, too_deep, Doc,
    Entry, Line, Mode, Value, ValueAt, MAX_DEPTH, MAX_KEY_LEN,
};

/// Checks the plain key `key` of the entry `text`, whose value text is `rest`.
pub(super) fn check_plain_key(
    text: &str,
    key: &str,
    rest: &str,
    line: usize,
) -> Result<(), String> {
    let before_colon = &text[..text.len() - rest.len() - 1];
    check_code(before_colon, line)?;
    // Everything up to the `:` counts towards PyYAML's limit, spaces included.
    if before_colon.chars().count() > MAX_KEY_LEN {
        return Err(format!(
            "line {line}: a key longer than {MAX_KEY_LEN} characters, which PyYAML refuses"
        ));
    }
    if key.starts_with(['&', '*', '!']) {
        return Err(format!(
            "line {line}: anchors, aliases and tags are not supported on mapping keys in \
             description.yml"
        ));
    }
    check_plain_start(key, line)?;
    check_timestamp(key, line)
}

/// Splits a mapping entry into its key and the text after the `:`.
fn mapping_key<'a>(line: &Line<'a>, path: &str, mode: Mode) -> Result<(String, &'a str), String> {
    let text = line.text;
    if text == "?" || text.starts_with("? ") {
        return Err(format!(
            "line {}: explicit `? key` entries are not supported in description.yml",
            line.number
        ));
    }
    if starts_quoted(text) && mode == Mode::Check {
        if let Some((key, rest)) = quoted_key(text, line.number)? {
            return Ok((key, rest));
        }
    }
    let (key, rest) = split_key(text).ok_or_else(|| {
        format!(
            "line {}: expected `key: value` in '{path}', got '{text}'",
            line.number
        )
    })?;
    check_plain_key(text, key, rest, line.number)?;
    Ok((key.to_string(), rest))
}

/// A quoted key — `"a b": value` — decoded, with the text after its `:`, or
/// `None` when the quoted scalar is a value, not a key.
fn quoted_key(text: &str, line: usize) -> Result<Option<(String, &str)>, String> {
    let (key, _, after) = quoted_scalar(text, line, &[])?;
    let after_spaces = after.trim_start_matches(' ');
    let Some(rest) = after_spaces.strip_prefix(':') else {
        return Ok(None);
    };
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return Ok(None);
    }
    // PyYAML measures a simple key from its first character to the `:`, the
    // spaces before the colon included (as it does for a plain key).
    if text[..text.len() - after_spaces.len()].chars().count() > MAX_KEY_LEN {
        return Err(format!(
            "line {line}: a key longer than {MAX_KEY_LEN} characters, which PyYAML refuses"
        ));
    }
    Ok(Some((key, rest)))
}

/// Whether `text` starts a mapping entry (`key: …` or `"key": …`).
pub(super) fn is_mapping_text(text: &str, line: usize) -> bool {
    if starts_quoted(text) {
        return matches!(quoted_key(text, line), Ok(Some(_)));
    }
    split_key(text).is_some()
}

/// Where a block mapping or sequence starts: `head`, a first entry or item
/// written on an outer sequence marker's line (`- key: value`, `- - x`), when
/// there is one, then the lines after.
#[derive(Debug, Clone, Copy)]
pub(super) struct Block<'s, 'a> {
    head: Option<Line<'a>>,
    lines: &'s [Line<'a>],
}

impl<'s, 'a> Block<'s, 'a> {
    pub(super) const fn new(head: Option<Line<'a>>, lines: &'s [Line<'a>]) -> Self {
        Self { head, lines }
    }
}

/// A block mapping: its entries and how many of `block.lines` it used. Its
/// keys sit at the indentation of the head line, or else of its first content
/// line, and it ends at the first content line indented less.
pub(super) fn parse_mapping(
    block: Block<'_, '_>,
    path: &str,
    mode: Mode,
    doc: &mut Doc,
    depth: usize,
) -> Result<(Vec<Entry>, usize), String> {
    let lines = block.lines;
    let mut entries: Vec<Entry> = Vec::new();
    // Key -> first line, so duplicate detection is a lookup, not a scan of
    // every entry before it.
    let mut first_lines: HashMap<String, usize> = HashMap::new();
    let field_indent = match block.head {
        Some(head) => head.indent,
        None => match lines.get(next_content(lines, 0)) {
            Some(first) => first.indent,
            None => return check_skipped(lines).map(|()| (entries, lines.len())),
        },
    };
    let mut pending = block.head;
    let mut i = 0;
    loop {
        let (turn_start, had_head) = (i, pending.is_some());
        let (line, after) = if let Some(head) = pending.take() {
            (head, 0)
        } else {
            let j = next_content(lines, i);
            let Some(line) = lines.get(j) else {
                check_skipped(&lines[i..])?;
                return Ok((entries, lines.len()));
            };
            if line.indent < field_indent {
                return Ok((entries, i));
            }
            check_skipped(&lines[i..j])?;
            if line.indent > field_indent {
                return Err(format!(
                    "line {}: inconsistent indentation in '{path}' (expected {field_indent} \
                     spaces, found {})",
                    line.number, line.indent
                ));
            }
            (*line, j + 1)
        };
        if depth > MAX_DEPTH {
            return Err(too_deep(line.number));
        }
        if line.text.starts_with('\t') {
            return Err(tab_error(line.number));
        }
        let (key, rest) = mapping_key(&line, path, mode)?;
        if mode == Mode::Read {
            if let Some(&first) = first_lines.get(&key) {
                return Err(format!(
                    "line {}: duplicate key '{path}.{key}' (first on line {first})",
                    line.number
                ));
            }
            first_lines.insert(key.clone(), line.number);
        }
        let value = ValueAt {
            rest,
            lines: &lines[after..],
            parent_indent: field_indent,
            line: line.number,
            seq_at_key_level: strip_comment(rest).is_empty(),
        };
        let child_path = format!("{path}.{key}");
        let (parsed, used) = parse_value(&value, &child_path, mode, doc, depth + 1)?;
        if key == "<<" && matches!(parsed, Value::Scalar(_) | Value::Null) {
            return Err(format!(
                "line {}: a `<<` merge key needs a mapping (or a list of them); PyYAML refuses \
                 a scalar",
                line.number
            ));
        }
        entries.push(Entry {
            key,
            line: line.number,
            plain: is_plain(rest, &lines[after..after + used]),
            value: parsed,
        });
        i = after + used;
        // A turn that did not start from the head entry took a line of its own
        // (`after > i`), so `i` moved on and the loop ends. Debug builds check
        // it: a turn that stood still would spin for ever.
        debug_assert!(
            had_head || i > turn_start,
            "a mapping entry made no progress"
        );
    }
}

/// A block sequence whose `- ` markers sit at the indentation of the head line
/// or else of its first content line, and how many of `block.lines` it used.
/// One written level with its key (`key:\n- item`, `at_key_level`) ends at
/// the next line at that level that is not an item.
pub(super) fn parse_seq(
    block: Block<'_, '_>,
    at_key_level: bool,
    path: &str,
    mode: Mode,
    doc: &mut Doc,
    depth: usize,
) -> Result<(Value, usize), String> {
    let lines = block.lines;
    let first_indent = || lines.get(next_content(lines, 0)).map_or(0, |l| l.indent);
    let seq_indent = block.head.map_or_else(first_indent, |head| head.indent);
    let mut items = Vec::new();
    let mut pending = block.head;
    let mut i = 0;
    loop {
        let (turn_start, had_head) = (i, pending.is_some());
        let (line, after) = if let Some(head) = pending.take() {
            (head, 0)
        } else {
            let j = next_content(lines, i);
            let Some(line) = lines.get(j) else {
                check_skipped(&lines[i..])?;
                return Ok((Value::Seq(items), lines.len()));
            };
            if line.indent < seq_indent || (at_key_level && !line.is_seq_item()) {
                return Ok((Value::Seq(items), i));
            }
            check_skipped(&lines[i..j])?;
            (*line, j + 1)
        };
        if line.indent != seq_indent || !line.is_seq_item() {
            return Err(format!(
                "line {}: expected a `- item` at indentation {seq_indent}, got '{}'",
                line.number, line.text
            ));
        }
        if depth > MAX_DEPTH {
            return Err(too_deep(line.number));
        }
        let after_dash = &line.text[1..];
        let item = after_spaces(after_dash, line.number)?;
        // The item's own text, at the column it starts at, for a collection
        // that begins on the marker's line (`- - x`, `- key: value`).
        let item_line = Line {
            number: line.number,
            indent: seq_indent + 1 + (after_dash.len() - item.len()),
            text: item,
            raw: line.raw,
        };
        let rest = &lines[after..];
        let nested = Block::new(Some(item_line), rest);
        let (value, used) = if item_line.is_seq_item() {
            parse_seq(nested, false, path, mode, doc, depth + 1)?
        } else if !item.is_empty() && is_mapping_text(item, line.number) {
            let (_, used) = parse_mapping(nested, path, Mode::Check, doc, depth + 1)?;
            (Value::Mapping, used)
        } else {
            let value = ValueAt {
                rest: item,
                lines: rest,
                parent_indent: seq_indent,
                line: line.number,
                seq_at_key_level: false,
            };
            parse_value(&value, path, mode, doc, depth + 1)?
        };
        items.push(value);
        i = after + used;
        // As in `parse_mapping`: a turn after the head item always moves `i` on.
        debug_assert!(
            had_head || i > turn_start,
            "a sequence item made no progress"
        );
    }
}
