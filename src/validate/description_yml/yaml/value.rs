// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The value after a mapping key's `:` or a sequence marker's `- `.

use super::scalar::{anchor_name, block_scalar, plain_scalar, quoted_scalar};
use super::{
    after_spaces, check_skipped, flow, is_mapping_text, next_content, nothing_but_a_comment,
    nothing_but_comments, parse_mapping, parse_seq, tab_error, too_deep, value_len, Block, Doc,
    Line, Mode, Value, MAX_DEPTH,
};

/// Where a value is: the text after its key's `:` (or its `- `) on line
/// `line`, and the lines after that one.
#[derive(Debug, Clone, Copy)]
pub(super) struct ValueAt<'s, 'a> {
    pub(super) rest: &'a str,
    pub(super) lines: &'s [Line<'a>],
    /// The indentation of the mapping or sequence the value belongs to: the
    /// value's own lines are indented deeper.
    pub(super) parent_indent: usize,
    pub(super) line: usize,
    /// Whether a sequence written level with the key (`key:\n- item`) may be
    /// the value: true for a mapping key whose line holds nothing else.
    pub(super) seq_at_key_level: bool,
}

/// Parses the value at `at`, returning it and how many of `at.lines` it used.
pub(super) fn parse_value(
    at: &ValueAt<'_, '_>,
    path: &str,
    mode: Mode,
    doc: &mut Doc,
    depth: usize,
) -> Result<(Value, usize), String> {
    let line = at.line;
    if depth > MAX_DEPTH {
        return Err(too_deep(line));
    }
    let rest = after_spaces(at.rest, line)?;
    if rest.is_empty() || rest.starts_with('#') {
        return on_following_lines(at, path, mode, doc, depth);
    }
    // The lines that are the value's by indentation: all a plain or block
    // scalar may use.
    let own = || value_len(at.lines, at.parent_indent, false);

    match rest.as_bytes()[0] {
        b'|' | b'>' => {
            let used = own();
            let value = block_scalar(rest, &at.lines[..used], at.parent_indent, line)?;
            Ok((value, used))
        }
        b'"' | b'\'' => {
            // Read to the closing quote, however the lines are indented: PyYAML
            // does not check the indentation inside a quoted scalar.
            let (value, used, remainder) = quoted_scalar(rest, line, at.lines)?;
            let end_line = if used == 0 {
                line
            } else {
                at.lines[used - 1].number
            };
            nothing_but_a_comment(remainder, end_line, "a quoted value")?;
            let tail = value_len(&at.lines[used..], at.parent_indent, false);
            nothing_but_comments(&at.lines[used..used + tail], "a quoted value")?;
            Ok((Value::Scalar(value), used + tail))
        }
        b'[' | b'{' => {
            let (value, used) = flow::parse(rest, line, at.lines, mode == Mode::Read, doc, depth)?;
            let tail = value_len(&at.lines[used..], at.parent_indent, false);
            nothing_but_comments(&at.lines[used..used + tail], "a flow collection")?;
            Ok((value, used + tail))
        }
        b'&' | b'*' | b'!' if mode == Mode::Read => Err(format!(
            "line {line}: YAML anchors, aliases and tags are not supported in description.yml"
        )),
        b'!' => Err(format!(
            "line {line}: YAML tags are not supported in description.yml"
        )),
        b'&' => {
            let (name, after) = anchor_name(&rest[1..], line)?;
            if !doc.define(name) {
                return Err(format!(
                    "line {line}: the anchor '&{name}' is defined twice, which PyYAML refuses"
                ));
            }
            if !after.is_empty() && !after.starts_with(' ') {
                return Err(format!(
                    "line {line}: expected a space after the anchor '&{name}'"
                ));
            }
            if after.trim_start_matches(' ').starts_with(['&', '*']) {
                return Err(format!(
                    "line {line}: an anchored node cannot be another anchor or an alias"
                ));
            }
            let parsed = parse_value(&ValueAt { rest: after, ..*at }, path, mode, doc, depth)?;
            doc.resolve(name, &parsed.0);
            Ok(parsed)
        }
        b'*' => {
            let (name, after) = anchor_name(&rest[1..], line)?;
            let value = doc.alias(name).ok_or_else(|| {
                format!(
                    "line {line}: the alias '*{name}' names no anchor defined before it, which \
                     PyYAML refuses"
                )
            })?;
            nothing_but_a_comment(after, line, "an alias")?;
            let used = own();
            nothing_but_comments(&at.lines[..used], "an alias")?;
            Ok((value, used))
        }
        _ => {
            let used = own();
            Ok((plain_scalar(rest, &at.lines[..used], line)?, used))
        }
    }
}

/// A value that starts on the line after its key: a block sequence or
/// mapping, a scalar, or nothing (null).
fn on_following_lines(
    at: &ValueAt<'_, '_>,
    path: &str,
    mode: Mode,
    doc: &mut Doc,
    depth: usize,
) -> Result<(Value, usize), String> {
    let first = next_content(at.lines, 0);
    let Some(child) = at.lines.get(first) else {
        // Nothing follows: null. Trailing blank lines are the caller's.
        return Ok((Value::Null, 0));
    };
    let at_key_level =
        at.seq_at_key_level && child.indent == at.parent_indent && child.is_seq_item();
    if child.indent <= at.parent_indent && !at_key_level {
        return Ok((Value::Null, 0));
    }
    check_skipped(&at.lines[..first])?;
    if child.text.starts_with('\t') {
        return Err(tab_error(child.number));
    }
    let from_child = Block::new(None, &at.lines[first..]);
    let (value, used) = if child.is_seq_item() {
        parse_seq(from_child, at_key_level, path, mode, doc, depth)?
    } else if is_mapping_text(child.text, child.number) {
        let (_, used) = parse_mapping(from_child, path, Mode::Check, doc, depth)?;
        (Value::Mapping, used)
    } else {
        // Any other value that starts on the next line reads as if it started
        // on the key's own: `name:\n  "x"` is `x`, not `"x"` with its quotes,
        // and a plain one folds the lines below it.
        let next = ValueAt {
            rest: child.text,
            lines: &at.lines[first + 1..],
            line: child.number,
            seq_at_key_level: false,
            ..*at
        };
        let (value, used) = parse_value(&next, path, mode, doc, depth)?;
        (value, used + 1)
    };
    Ok((value, first + used))
}
