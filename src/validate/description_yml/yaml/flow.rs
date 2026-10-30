// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Flow collections — `[a, "b"]`, `{k: v}` — nested and spread over lines,
//! read as `PyYAML` scans and parses them (`scan_plain`, `scan_flow_scalar`
//! and `parse_flow_*` in `yaml/scanner.py` and `yaml/parser.py`, 6.0).

use super::scalar::{anchor_name, check_timestamp, decode_quoted, Segs};
use super::{nothing_but_a_comment, tab_error, too_deep, Doc, Line, Value, MAX_DEPTH, MAX_KEY_LEN};

/// The flow collection that `rest` (starting with `[` or `{`) opens on line
/// `line`, continued over the following `lines` until it closes — at any
/// indentation, as in `PyYAML`, which ignores indentation in a flow
/// collection. Returns it and how many of `lines` it used.
///
/// With `read` set it is the value of a field the caller reads: only a flat
/// sequence of scalars is accepted, and its items are returned; a mapping is
/// checked and returned as [`Value::Mapping`]. Otherwise anything `PyYAML`
/// accepts is accepted (anchors and aliases included; tags and explicit `?`
/// keys are refused as unsupported).
pub(super) fn parse(
    rest: &str,
    line: usize,
    lines: &[Line<'_>],
    read: bool,
    doc: &mut Doc,
    depth: usize,
) -> Result<(Value, usize), String> {
    let segs = Segs {
        first: (line, rest),
        more: lines,
    };
    let mut cursor = Cursor {
        segs,
        seg: 0,
        pos: 0,
        doc,
        read,
    };
    let value = cursor.node(depth)?;
    nothing_but_a_comment(cursor.rest(), cursor.line(), "a flow collection")?;
    // Segment `n` past the first is `lines[n - 1]`.
    Ok((value, cursor.seg))
}

/// A position in `segs`, and what the reader needs while it moves through
/// them. `seg` is always a segment that exists.
struct Cursor<'s, 'a, 'd> {
    segs: Segs<'s, 'a>,
    seg: usize,
    pos: usize,
    doc: &'d mut Doc,
    read: bool,
}

impl<'a> Cursor<'_, 'a, '_> {
    fn line(&self) -> usize {
        self.segs.get(self.seg).map_or(0, |(line, _)| line)
    }

    fn text(&self) -> &'a str {
        self.segs.get(self.seg).map_or("", |(_, text)| text)
    }

    fn rest(&self) -> &'a str {
        &self.text()[self.pos..]
    }

    /// Moves to the start of the next segment, if there is one.
    fn next_seg(&mut self) -> Result<bool, String> {
        if self.segs.get(self.seg + 1).is_none() {
            return Ok(false);
        }
        self.seg += 1;
        self.pos = 0;
        self.segs.check_separator(self.seg)?;
        Ok(true)
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }

    /// Skips spaces, comments and line ends between tokens, refusing a tab
    /// there as `PyYAML` does (it skips only spaces).
    fn skip(&mut self) -> Result<(), String> {
        loop {
            let round_start = (self.seg, self.pos);
            match self.peek() {
                Some(' ') => self.pos += 1,
                Some('\t') => return Err(tab_error(self.line())),
                Some('#') => self.pos = self.text().len(),
                Some(_) => return Ok(()),
                None => {
                    if !self.next_seg()? {
                        return Ok(());
                    }
                }
            }
            // Every round that does not return moves the cursor on, so the loop
            // ends. (Debug builds check it; a round that stood still would
            // otherwise spin for ever.)
            debug_assert!((self.seg, self.pos) > round_start, "skip made no progress");
        }
    }

    /// One node: a nested collection, a quoted or plain scalar, or (outside
    /// a field the caller reads) an anchored node or an alias.
    fn node(&mut self, depth: usize) -> Result<Value, String> {
        let line = self.line();
        if depth > MAX_DEPTH {
            return Err(too_deep(line));
        }
        match self.peek() {
            Some('[') => {
                self.pos += 1;
                self.seq(line, depth)
            }
            Some('{') => {
                self.pos += 1;
                self.map(line, depth)
            }
            Some('"' | '\'') => {
                let (value, seg, pos) = decode_quoted(&self.segs, self.seg, self.pos)?;
                self.seg = seg;
                self.pos = pos;
                Ok(Value::Scalar(value))
            }
            Some('&' | '*' | '!') if self.read => Err(format!(
                "line {line}: YAML anchors, aliases and tags are not supported in description.yml"
            )),
            Some('!') => Err(format!(
                "line {line}: YAML tags are not supported in description.yml"
            )),
            Some('&') => {
                let (name, _) = anchor_name(&self.rest()[1..], line)?;
                if !self.doc.define(name) {
                    return Err(format!(
                        "line {line}: the anchor '&{name}' is defined twice, which PyYAML refuses"
                    ));
                }
                self.pos += 1 + name.len();
                self.skip()?;
                let value = match self.peek() {
                    // An anchored empty node.
                    None | Some(',' | ']' | '}' | ':') => Value::Null,
                    Some('&' | '*') => {
                        return Err(format!(
                            "line {line}: an anchored node cannot be another anchor or an alias"
                        ))
                    }
                    Some(_) => self.node(depth)?,
                };
                self.doc.resolve(name, &value);
                Ok(value)
            }
            Some('*') => {
                let (name, _) = anchor_name(&self.rest()[1..], line)?;
                let value = self.doc.alias(name).ok_or_else(|| {
                    format!(
                        "line {line}: the alias '*{name}' names no anchor defined before it, which \
                         PyYAML refuses"
                    )
                })?;
                self.pos += 1 + name.len();
                Ok(value)
            }
            Some('?') => Err(format!(
                "line {line}: explicit `? key` entries are not supported in description.yml"
            )),
            Some('-') if matches!(self.peek_second(), None | Some(' ' | '\t')) => Err(format!(
                "line {line}: a `- ` sequence entry inside a flow collection, which PyYAML \
                 refuses; quote the value"
            )),
            Some(c @ ('|' | '>' | '@' | '`' | '%' | ':' | ',' | ']' | '}')) => Err(format!(
                "line {line}: '{c}' cannot start a value inside a flow collection (PyYAML \
                 refuses it); quote the value"
            )),
            Some(_) => self.plain(),
            None => Err(format!(
                "line {line}: a flow collection ends where a value was expected"
            )),
        }
    }

    /// After a key or item that began at `key_start`: an optional `: value`.
    /// Returns whether one was there.
    fn optional_value(
        &mut self,
        key_start: (usize, usize),
        depth: usize,
        closing: char,
    ) -> Result<bool, String> {
        if self.peek() != Some(':') {
            return Ok(false);
        }
        self.check_key_extent(key_start)?;
        self.pos += 1;
        self.skip()?;
        if !matches!(self.peek(), Some(',') | None) && self.peek() != Some(closing) {
            self.node(depth)?;
            self.skip()?;
        }
        Ok(true)
    }

    /// `PyYAML` takes a node for a mapping key only if the `:` after it is on
    /// the key's own line and at most 1024 characters from where the key
    /// began (`stale_possible_simple_keys` in `yaml/scanner.py`). In a flow
    /// collection a key that is too far from its `:` is not an error of the
    /// scanner's; the `:` is left with no key, and the parser stops at it. The
    /// key began at `key_start`, which includes an anchor in front of it.
    fn check_key_extent(&self, key_start: (usize, usize)) -> Result<(), String> {
        let (seg, pos) = key_start;
        if seg != self.seg {
            return Err(format!(
                "line {}: a flow mapping key and its ':' on different lines, which PyYAML refuses",
                self.line()
            ));
        }
        if self.text()[pos..self.pos].chars().count() > MAX_KEY_LEN {
            return Err(format!(
                "line {}: a key longer than {MAX_KEY_LEN} characters, which PyYAML refuses",
                self.line()
            ));
        }
        Ok(())
    }

    /// The rest of a `[ … ]` whose `[` was on `open_line`.
    fn seq(&mut self, open_line: usize, depth: usize) -> Result<Value, String> {
        let mut items = Vec::new();
        loop {
            let round_start = (self.seg, self.pos);
            self.skip()?;
            match self.peek() {
                None => return Err(unterminated(open_line, "sequence")),
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Seq(items));
                }
                Some(',') => {
                    return Err(format!(
                        "line {}: an empty entry in a flow sequence, which PyYAML refuses",
                        self.line()
                    ))
                }
                Some(_) => {}
            }
            let key_start = (self.seg, self.pos);
            let item = self.node(depth + 1)?;
            self.skip()?;
            // `[key: value]` is a one-entry mapping.
            let pair = self.peek() == Some(':');
            if pair && matches!(item, Value::Seq(_) | Value::Mapping) {
                return Err(collection_key(self.line()));
            }
            self.optional_value(key_start, depth + 1, ']')?;
            let item = if pair { Value::Mapping } else { item };
            if self.read && matches!(item, Value::Seq(_) | Value::Mapping) {
                return Err(format!(
                    "line {}: nested flow collections are not supported",
                    self.line()
                ));
            }
            items.push(item);
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Seq(items));
                }
                None => return Err(unterminated(open_line, "sequence")),
                Some(_) => {
                    return Err(format!(
                        "line {}: expected ',' or ']' between flow sequence items, got '{}'",
                        self.line(),
                        self.rest()
                    ))
                }
            }
            debug_assert!(
                (self.seg, self.pos) > round_start,
                "a round made no progress"
            );
        }
    }

    /// The rest of a `{ … }` whose `{` was on `open_line`.
    fn map(&mut self, open_line: usize, depth: usize) -> Result<Value, String> {
        loop {
            let round_start = (self.seg, self.pos);
            self.skip()?;
            match self.peek() {
                None => return Err(unterminated(open_line, "mapping")),
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Mapping);
                }
                Some(',') => {
                    return Err(format!(
                        "line {}: an empty entry in a flow mapping, which PyYAML refuses",
                        self.line()
                    ))
                }
                Some(':') => {
                    return Err(format!(
                        "line {}: a flow mapping entry with no key is not supported",
                        self.line()
                    ))
                }
                Some(_) => {}
            }
            let key_start = (self.seg, self.pos);
            let key = self.node(depth + 1)?;
            if matches!(key, Value::Seq(_) | Value::Mapping) {
                return Err(collection_key(self.line()));
            }
            self.skip()?;
            self.optional_value(key_start, depth + 1, '}')?;
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Mapping);
                }
                None => return Err(unterminated(open_line, "mapping")),
                Some(_) => {
                    return Err(format!(
                        "line {}: expected ',' or '}}' between flow mapping entries, got '{}'",
                        self.line(),
                        self.rest()
                    ))
                }
            }
            debug_assert!(
                (self.seg, self.pos) > round_start,
                "a round made no progress"
            );
        }
    }

    /// A plain scalar inside a flow collection: words separated by spaces or
    /// line breaks, ended by `,`, `?`, a bracket or brace, `: `, a tab or a
    /// comment.
    fn plain(&mut self) -> Result<Value, String> {
        let line = self.line();
        let mut value = String::new();
        let mut gap = String::new();
        loop {
            let round_start = (self.seg, self.pos);
            let rest = self.rest();
            let mut end = 0;
            for (i, c) in rest.char_indices() {
                let stop = match c {
                    ' ' | '\t' | ',' | '?' | '[' | ']' | '{' | '}' => true,
                    ':' => rest[i + 1..]
                        .chars()
                        .next()
                        .is_none_or(|n| matches!(n, ' ' | '\t' | ',' | '[' | ']' | '{' | '}')),
                    _ => false,
                };
                if stop {
                    break;
                }
                end = i + c.len_utf8();
            }
            if end == 0 {
                break;
            }
            value.push_str(&gap);
            value.push_str(&rest[..end]);
            self.pos += end;
            let spaces = self.rest().len() - self.rest().trim_start_matches(' ').len();
            self.pos += spaces;
            if self.peek().is_none() {
                // The line ended: fold over the line breaks to the next text.
                let mut breaks = 0;
                while self.next_seg()? {
                    let text = self.text();
                    let content = text.trim_start_matches(' ');
                    self.pos = text.len() - content.len();
                    if !content.is_empty() {
                        break;
                    }
                    breaks += 1;
                }
                gap = if breaks == 0 {
                    " ".to_string()
                } else {
                    "\n".repeat(breaks)
                };
            } else if spaces == 0 {
                break;
            } else {
                gap = " ".repeat(spaces);
            }
            if self.peek() == Some('#') {
                break;
            }
            debug_assert!(
                (self.seg, self.pos) > round_start,
                "a round made no progress"
            );
        }
        check_timestamp(&value, line)?;
        Ok(match value.as_str() {
            "~" | "null" | "Null" | "NULL" => Value::Null,
            _ => Value::Scalar(value),
        })
    }
}

fn unterminated(open_line: usize, what: &str) -> String {
    format!("line {open_line}: unterminated flow {what}")
}

fn collection_key(line: usize) -> String {
    format!("line {line}: a collection used as a mapping key, which PyYAML cannot load")
}
