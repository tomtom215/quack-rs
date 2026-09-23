// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Scalar-level pieces of the `description.yml` YAML reader: decoding plain,
//! quoted, block and flow values, and recognising keys and comments.

use std::fmt::Write as _;

use super::{Line, Value};

/// Joins the physical lines of a multi-line quoted scalar the way YAML folds
/// them: a line break becomes a space, a blank line becomes a newline, and in a
/// double-quoted scalar a line ending in an unescaped `\` continues directly
/// onto the next line (the escaped line break of `bitfilters`' description).
pub(super) fn join_quoted_lines(first: &str, children: &[Line<'_>]) -> String {
    let double = first.starts_with('"');
    let mut text = first.to_string();
    for child in children {
        if child.is_blank() {
            text.push('\n');
            continue;
        }
        let trailing_backslashes = text.bytes().rev().take_while(|&b| b == b'\\').count();
        if double && trailing_backslashes % 2 == 1 {
            text.pop();
        } else if !text.ends_with('\n') {
            text.push(' ');
        }
        text.push_str(child.text);
    }
    text
}

/// A `|` (literal) or `>` (folded) block scalar. The result is trimmed, as a
/// `description.yml` field never wants leading or trailing whitespace.
pub(super) fn block_scalar(
    header: &str,
    children: &[Line<'_>],
    parent_indent: usize,
    line: usize,
) -> Result<Value, String> {
    let literal = header.starts_with('|');
    let indicators = strip_comment(&header[1..]);
    if !indicators
        .chars()
        .all(|c| matches!(c, '-' | '+' | '1'..='9'))
    {
        return Err(format!(
            "line {line}: invalid block scalar header '{header}'"
        ));
    }
    let Some(indent) = children
        .iter()
        .filter(|l| !l.is_blank())
        .map(|l| l.indent)
        .min()
    else {
        return Ok(Value::Scalar(String::new()));
    };
    if indent <= parent_indent {
        return Err(format!(
            "line {line}: block scalar content must be indented past its key"
        ));
    }
    // Literal blocks keep every line break. Folded blocks join adjacent
    // non-blank lines with a space and turn each blank line into a break.
    let mut out = String::new();
    let mut previous_blank = true;
    for (n, child) in children.iter().enumerate() {
        if literal && n > 0 {
            out.push('\n');
        }
        if child.is_blank() {
            if !literal {
                out.push('\n');
            }
            previous_blank = true;
            continue;
        }
        if !literal && !previous_blank {
            out.push(' ');
        }
        // Keep indentation beyond the block's own, as YAML does.
        let extra = child.indent - indent;
        let _ = write!(out, "{:extra$}{}", "", child.text);
        previous_blank = false;
    }
    Ok(Value::Scalar(out.trim().to_string()))
}

/// A plain (unquoted) scalar, possibly continued on deeper lines, which YAML
/// folds into one line. A comment ends it.
pub(super) fn plain_scalar(
    first: &str,
    children: &[Line<'_>],
    line: usize,
) -> Result<Value, String> {
    let head = strip_comment(first);
    let mut parts: Vec<&str> = Vec::new();
    if !head.is_empty() {
        parts.push(head);
    }
    let mut ended = head.len() < first.trim_end().len();
    for child in children.iter().filter(|l| l.is_content()) {
        if ended {
            return Err(format!(
                "line {}: text after a comment that ended the value on line {line}",
                child.number
            ));
        }
        if split_key(child.text).is_some() {
            return Err(format!(
                "line {}: a `key: value` cannot continue a plain value; indent it under its \
                 own key or quote the text",
                child.number
            ));
        }
        let text = strip_comment(child.text);
        ended = text.len() < child.text.len();
        parts.push(text);
    }
    let joined = parts.join(" ");
    Ok(match joined.as_str() {
        "~" | "null" | "Null" | "NULL" => Value::Null,
        _ => Value::Scalar(joined),
    })
}

/// A flow sequence such as `[Jane, "Bob, Jr."]`, already joined onto one line
/// with comments removed.
pub(super) fn flow_seq(text: &str, line: usize) -> Result<Value, String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.trim_end().strip_suffix(']'))
        .ok_or_else(|| format!("line {line}: unterminated flow sequence '{text}'"))?;
    let mut items = Vec::new();
    let mut rest = inner.trim_start();
    while !rest.is_empty() {
        let (item, after) = if rest.starts_with(['"', '\'']) {
            let (value, after) = quoted_scalar(rest, line)?;
            (value, after.trim_start())
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let item = rest[..end].trim();
            if item.contains(['[', ']', '{', '}']) {
                return Err(format!(
                    "line {line}: nested flow collections are not supported"
                ));
            }
            (item.to_string(), &rest[end..])
        };
        if !item.is_empty() || !after.is_empty() {
            items.push(Value::Scalar(item));
        }
        rest = match after.strip_prefix(',') {
            Some(next) => next.trim_start(),
            None if after.is_empty() => after,
            None => {
                return Err(format!(
                    "line {line}: expected ',' between flow sequence items, got '{after}'"
                ))
            }
        };
    }
    Ok(Value::Seq(items))
}

/// Decodes the quoted scalar at the start of `text`, returning it and the text
/// after the closing quote.
pub(super) fn quoted_scalar(text: &str, line: usize) -> Result<(String, &str), String> {
    let quote = text.chars().next().unwrap_or('"');
    let mut out = String::new();
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        if c == quote {
            if quote == '\'' && chars.peek().map(|&(_, n)| n) == Some('\'') {
                chars.next();
                out.push('\'');
                continue;
            }
            return Ok((out, &text[i + 1..]));
        }
        if c == '\\' && quote == '"' {
            let Some((_, escape)) = chars.next() else {
                break;
            };
            let decoded = match escape {
                '0' => '\0',
                'a' => '\u{07}',
                'b' => '\u{08}',
                't' | '\t' => '\t',
                'n' => '\n',
                'v' => '\u{0b}',
                'f' => '\u{0c}',
                'r' => '\r',
                'e' => '\u{1b}',
                ' ' => ' ',
                '"' => '"',
                '/' => '/',
                '\\' => '\\',
                'N' => '\u{85}',
                '_' => '\u{a0}',
                'L' => '\u{2028}',
                'P' => '\u{2029}',
                'x' | 'u' | 'U' => {
                    let width = match escape {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let hex: String = (0..width)
                        .filter_map(|_| chars.next().map(|(_, h)| h))
                        .collect();
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .filter(|_| hex.len() == width)
                        .and_then(char::from_u32)
                        .ok_or_else(|| format!("line {line}: invalid escape '\\{escape}{hex}'"))?
                }
                other => return Err(format!("line {line}: invalid escape '\\{other}'")),
            };
            out.push(decoded);
            continue;
        }
        out.push(c);
    }
    Err(format!("line {line}: unterminated {quote}-quoted value"))
}

/// Splits `key: rest` (or `key:` at end of line). `None` when the line is not
/// a mapping entry — in YAML the colon must be followed by a space or the end
/// of the line, so `http://x` and `a:b` are not keys.
pub(super) fn split_key(text: &str) -> Option<(&str, &str)> {
    if starts_quoted(text) || text.starts_with(['-', '#', '[', '{']) {
        return None;
    }
    let mut search = 0;
    while let Some(pos) = text[search..].find(':').map(|p| p + search) {
        let after = &text[pos + 1..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            let key = text[..pos].trim_end();
            // A ` #` before the colon means the colon is inside a comment.
            if key.is_empty() || key.contains(" #") {
                return None;
            }
            return Some((key, after));
        }
        search = pos + 1;
    }
    None
}

pub(super) fn starts_quoted(text: &str) -> bool {
    text.starts_with(['"', '\''])
}

/// Removes a trailing comment from unquoted text: a `#` at the start or after
/// whitespace begins one, while `C#` or `#1` inside a word does not.
pub(super) fn strip_comment(text: &str) -> &str {
    let bytes = text.as_bytes();
    let cut = (0..bytes.len())
        .find(|&i| bytes[i] == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t'))
        .unwrap_or(bytes.len());
    text[..cut].trim()
}
