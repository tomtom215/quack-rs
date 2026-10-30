// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Scalar-level pieces of the `description.yml` YAML reader: decoding plain,
//! quoted and block values, and recognising keys, comments and anchors — each
//! held to the rules `PyYAML`'s scanner applies (`yaml/scanner.py`, 6.0).

use super::super::yaml11;
use super::{Line, Value};

/// A piece of text with the line it is on: one physical line, or the part of
/// it after a key. Quoted scalars and flow collections are read across these.
pub(super) type Seg<'a> = (usize, &'a str);

/// The text a quoted scalar or flow collection is read from: the rest of the
/// line it starts on, then the lines after it. Nothing is copied: segment
/// `i > 0` is `more[i - 1]`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Segs<'s, 'a> {
    pub(super) first: Seg<'a>,
    pub(super) more: &'s [Line<'a>],
}

impl<'a> Segs<'_, 'a> {
    pub(super) fn get(&self, i: usize) -> Option<Seg<'a>> {
        if i == 0 {
            Some(self.first)
        } else {
            self.more.get(i - 1).map(|l| (l.number, l.text))
        }
    }

    /// Refuses segment `i` if it is a document separator, `---` or `...` at
    /// column 0, which `PyYAML` refuses inside a quoted scalar or flow
    /// collection ("found unexpected document separator").
    pub(super) fn check_separator(&self, i: usize) -> Result<(), String> {
        let Some(line) = i.checked_sub(1).and_then(|i| self.more.get(i)) else {
            return Ok(());
        };
        // `get(..3)` is `None` unless byte 3 is a character boundary, so
        // slicing there afterwards cannot split a character.
        let separator = matches!(line.text.get(..3), Some("---" | "..."))
            && matches!(line.text[3..].chars().next(), None | Some(' ' | '\t'));
        if line.indent == 0 && separator {
            return Err(format!(
                "line {}: a document separator inside a quoted value or flow collection",
                line.number
            ));
        }
        Ok(())
    }
}

/// Refuses a tab in `code`, text outside quotes, block scalars and comments,
/// where `PyYAML` does not allow one: it skips only spaces between tokens and
/// ends a plain scalar at a tab.
pub(super) fn check_code(code: &str, line: usize) -> Result<(), String> {
    if code.contains('\t') {
        return Err(format!(
            "line {line}: a tab outside quotes, block scalars and comments, which PyYAML (what \
             the community build reads this file with) refuses; use spaces, or quote the value"
        ));
    }
    Ok(())
}

/// Splits unquoted text at its comment: a `#` at the start or after a space
/// begins one, while `C#` or `#1` inside a word does not. (After a tab, `#`
/// is not a comment to `PyYAML` either: the tab itself is the error.)
pub(super) fn split_comment(text: &str) -> (&str, Option<&str>) {
    let bytes = text.as_bytes();
    (0..bytes.len())
        .find(|&i| bytes[i] == b'#' && (i == 0 || bytes[i - 1] == b' '))
        .map_or((text, None), |i| (&text[..i], Some(&text[i + 1..])))
}

/// `text` without its comment and without surrounding spaces.
pub(super) fn strip_comment(text: &str) -> &str {
    split_comment(text).0.trim_matches(' ')
}

/// Splits `key: rest` (or `key:` at end of line). `None` when the line is not
/// a mapping entry — in YAML the colon must be followed by a space or the end
/// of the line, so `http://x` and `a:b` are not keys. (A tab after the colon
/// also splits, so that the tab can be reported as the error it is.)
pub(super) fn split_key(text: &str) -> Option<(&str, &str)> {
    // `- ` is a sequence item; `-x` is an ordinary plain scalar.
    let item = text == "-" || text.starts_with("- ");
    if item || starts_quoted(text) || text.starts_with(['#', '[', '{']) {
        return None;
    }
    for (pos, _) in text.match_indices(':') {
        let after = &text[pos + 1..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            let key = text[..pos].trim_end_matches(' ');
            // A ` #` before the colon means the colon is inside a comment.
            if key.is_empty() || key.contains(" #") {
                return None;
            }
            return Some((key, after));
        }
    }
    None
}

pub(super) fn starts_quoted(text: &str) -> bool {
    text.starts_with(['"', '\''])
}

/// Refuses an unquoted value or key that `PyYAML` cannot start a plain scalar
/// with: `@`, `` ` `` and `%` are reserved or directive indicators, `,` `]`
/// `}` flow indicators, `|` and `>` block-scalar headers, and `-`, `?` and
/// `:` followed by a space start a sequence entry, an explicit key and a
/// value. (`[`, `{`, quotes, `&`, `*`, `!` and `#` start other things and are
/// dispatched before this.)
pub(super) fn check_plain_start(text: &str, line: usize) -> Result<(), String> {
    let mut chars = text.chars();
    let first = chars.next();
    let spaced = matches!(chars.next(), None | Some(' ' | '\t'));
    match first {
        Some(c @ ('@' | '`' | '%' | ',' | ']' | '}' | '|' | '>')) => Err(format!(
            "line {line}: an unquoted value cannot start with '{c}', which PyYAML (what the \
             community build reads this file with) refuses; quote the value"
        )),
        Some(c @ ('-' | '?' | ':')) if spaced => Err(format!(
            "line {line}: an unquoted value cannot start with '{c} ', which PyYAML reads as \
             YAML syntax and refuses here; quote the value"
        )),
        _ => Ok(()),
    }
}

/// One line of a plain scalar, without its comment: no tab, no `: ` and no
/// final `:`, which `PyYAML` reads as a nested mapping ("mapping values are not
/// allowed here").
fn check_plain_text(code: &str, line: usize) -> Result<(), String> {
    check_code(code, line)?;
    let code = code.trim_end_matches(' ');
    if code.contains(": ") || code.ends_with(':') {
        return Err(format!(
            "line {line}: an unquoted value cannot contain ': ' or end in ':' (PyYAML, which the \
             community build reads this file with, reads it as a nested mapping and fails with \
             \"mapping values are not allowed here\"); quote the value"
        ));
    }
    Ok(())
}

/// Refuses a plain scalar `PyYAML` resolves to a date or time it then cannot
/// construct (`2024-13-01`: "month must be in 1..12").
pub(super) fn check_timestamp(text: &str, line: usize) -> Result<(), String> {
    yaml11::unconstructible_timestamp(text).map_or(Ok(()), |why| {
        Err(format!(
            "line {line}: the unquoted value '{text}' reads as a date to PyYAML, which then \
             fails ({why}); quote it"
        ))
    })
}

/// A plain (unquoted) scalar starting with `first`, possibly continued on the
/// deeper `children` lines, folded as YAML folds them: a line break becomes a
/// space and each blank line a newline. A comment — at the end of a line or on
/// a line of its own — ends it.
pub(super) fn plain_scalar(
    first: &str,
    children: &[Line<'_>],
    line: usize,
) -> Result<Value, String> {
    check_plain_start(first, line)?;
    let (head, comment) = split_comment(first);
    check_plain_text(head, line)?;
    let mut value = head.trim_end_matches(' ').to_string();
    // The line of the comment that ended the value, once one has.
    let mut ended_on = comment.map(|_| line);
    let mut breaks = 0usize;
    for child in children {
        if child.is_blank() {
            if child.text.contains('\t') {
                return Err(super::tab_error(child.number));
            }
            breaks += 1;
            continue;
        }
        if child.is_comment() {
            ended_on.get_or_insert(child.number);
            continue;
        }
        if let Some(comment_line) = ended_on {
            return Err(format!(
                "line {}: text after a comment that ended the value on line {comment_line}",
                child.number
            ));
        }
        if child.text.starts_with('\t') {
            return Err(super::tab_error(child.number));
        }
        if split_key(child.text).is_some() {
            return Err(format!(
                "line {}: a `key: value` cannot continue a plain value; indent it under its \
                 own key or quote the text",
                child.number
            ));
        }
        let (text, comment) = split_comment(child.text);
        check_plain_text(text, child.number)?;
        if comment.is_some() {
            ended_on = Some(child.number);
        }
        if breaks == 0 {
            value.push(' ');
        }
        for _ in 0..breaks {
            value.push('\n');
        }
        breaks = 0;
        value.push_str(text.trim_end_matches(' '));
    }
    check_timestamp(&value, line)?;
    Ok(match value.as_str() {
        "~" | "null" | "Null" | "NULL" => Value::Null,
        _ => Value::Scalar(value),
    })
}

/// The quoted scalar at the start of `first` (on `line`), continued over the
/// `more` lines when it does not close on its own: its value, how many of
/// `more` it used, and the text after its closing quote.
pub(super) fn quoted_scalar<'a>(
    first: &'a str,
    line: usize,
    more: &[Line<'a>],
) -> Result<(String, usize, &'a str), String> {
    let segs = Segs {
        first: (line, first),
        more,
    };
    let (value, seg, end) = decode_quoted(&segs, 0, 0)?;
    let text = segs.get(seg).map_or("", |(_, text)| text);
    Ok((value, seg, &text[end..]))
}

/// Decodes the quoted scalar whose opening quote is at byte `pos` of
/// `segs[seg]`, as `PyYAML` scans one (`scan_flow_scalar`): escapes decoded in
/// a double-quoted scalar, `''` in a single-quoted one, and at each line end
/// the trailing spaces and tabs dropped and the break folded — to a space, or
/// to one newline per blank line that follows. A double-quoted line ending in
/// `\` continues without the space. Returns the value and the position just
/// past the closing quote.
pub(super) fn decode_quoted(
    segs: &Segs<'_, '_>,
    seg: usize,
    pos: usize,
) -> Result<(String, usize, usize), String> {
    let (start_line, first) = segs.get(seg).unwrap_or((0, ""));
    let quote = first[pos..].chars().next().unwrap_or('"');
    let double = quote == '"';
    let mut out = String::new();
    let (mut seg, mut at) = (seg, pos + 1);
    let mut text = first;
    loop {
        let line = segs.get(seg).map_or(start_line, |(line, _)| line);
        let mut chars = text[at..].char_indices().peekable();
        // Where an unbroken run of literal spaces and tabs began: kept if text
        // follows it on the line, dropped if the line ends.
        let mut space_run: Option<usize> = None;
        let mut escaped_break = false;
        while let Some((offset, c)) = chars.next() {
            let i = at + offset;
            if c == ' ' || c == '\t' {
                space_run.get_or_insert(i);
                continue;
            }
            if let Some(start) = space_run.take() {
                out.push_str(&text[start..i]);
            }
            if c == quote {
                if !double && chars.peek().map(|&(_, n)| n) == Some('\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                return Ok((out, seg, i + 1));
            }
            if c == '\\' && double {
                let Some((_, escape)) = chars.next() else {
                    escaped_break = true;
                    break;
                };
                out.push(decode_escape(escape, &mut chars, line)?);
                continue;
            }
            out.push(c);
        }
        // The line ended inside the scalar.
        let mut breaks = 0;
        loop {
            seg += 1;
            let Some((_, next)) = segs.get(seg) else {
                return Err(format!(
                    "line {start_line}: unterminated {quote}-quoted value"
                ));
            };
            segs.check_separator(seg)?;
            let content = next.trim_start_matches([' ', '\t']);
            if content.is_empty() {
                breaks += 1;
                continue;
            }
            at = next.len() - content.len();
            text = next;
            break;
        }
        if breaks == 0 && !escaped_break {
            out.push(' ');
        }
        for _ in 0..breaks {
            out.push('\n');
        }
    }
}

/// The character a double-quoted escape `\<escape>` stands for (`PyYAML`'s
/// `ESCAPE_REPLACEMENTS` and `ESCAPE_CODES`).
fn decode_escape(
    escape: char,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    line: usize,
) -> Result<char, String> {
    Ok(match escape {
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
            // `from_str_radix` accepts a leading `+`; YAML does not.
            u32::from_str_radix(&hex, 16)
                .ok()
                .filter(|_| hex.len() == width && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(char::from_u32)
                .ok_or_else(|| format!("line {line}: invalid escape '\\{escape}{hex}'"))?
        }
        other => return Err(format!("line {line}: invalid escape '\\{other}'")),
    })
}

/// The header of a block scalar after its `|` or `>`, as `PyYAML` reads it:
/// at most one chomping indicator (`+`/`-`) and one indentation digit (1-9),
/// in either order, then the end of the line or spaces and a comment. Returns
/// the indentation digit, if any.
fn block_header(header: &str, line: usize) -> Result<Option<usize>, String> {
    let bytes = header.as_bytes();
    let invalid = || format!("line {line}: invalid block scalar header '{header}'");
    let mut i = 1;
    let mut increment = None;
    let chomp = |b: Option<&u8>| matches!(b, Some(b'+' | b'-'));
    let digit = |b: Option<&u8>| {
        b.filter(|b| b.is_ascii_digit())
            .map(|b| usize::from(b - b'0'))
    };
    if chomp(bytes.get(i)) {
        i += 1;
        if let Some(d) = digit(bytes.get(i)) {
            increment = Some(d);
            i += 1;
        }
    } else if let Some(d) = digit(bytes.get(i)) {
        increment = Some(d);
        i += 1;
        if chomp(bytes.get(i)) {
            i += 1;
        }
    }
    if increment == Some(0) {
        return Err(format!(
            "line {line}: a block scalar's indentation indicator must be 1-9, not 0"
        ));
    }
    let after = &header[i..];
    if !(after.is_empty() || after.starts_with(' ')) {
        return Err(invalid());
    }
    let after = after.trim_start_matches(' ');
    if !(after.is_empty() || after.starts_with('#')) {
        return Err(invalid());
    }
    Ok(increment)
}

/// A `|` (literal) or `>` (folded) block scalar whose header is `header`.
/// The result is trimmed, as a `description.yml` field never wants leading or
/// trailing whitespace.
///
/// As in `PyYAML`, the content's indentation is that of its first non-blank
/// line (or the header's indentation digit): a later line indented less ends
/// the block, and only blank lines and comments may follow it.
pub(super) fn block_scalar(
    header: &str,
    children: &[Line<'_>],
    parent_indent: usize,
    line: usize,
) -> Result<Value, String> {
    let literal = header.starts_with('|');
    // Without an indentation digit, PyYAML takes the indentation of the first
    // non-blank line, counting the spaces of the blank lines before it too.
    let detected = || {
        let mut widest = 0;
        for child in children {
            widest = widest.max(child.indent);
            if !child.text.is_empty() {
                break;
            }
        }
        widest.max(parent_indent + 1)
    };
    let indent =
        block_header(header, line)?.map_or_else(detected, |increment| parent_indent + increment);

    let mut out = String::new();
    // Whether the previous content line started with a non-space, which
    // decides how a folded block joins it to the next.
    let mut previous: Option<bool> = None;
    let mut breaks = 0usize;
    let mut ended_on: Option<usize> = None;
    for child in children {
        if let Some(end) = ended_on {
            if child.is_content() {
                return Err(format!(
                    "line {}: text after the block scalar that the comment on line {end} \
                     ended (it is indented less than the content, {indent} spaces); PyYAML \
                     refuses this",
                    child.number
                ));
            }
            super::check_blank_or_comment(child)?;
            continue;
        }
        if child.indent >= indent && child.raw.len() > indent {
            let content = &child.raw[indent..];
            let leading = !content.starts_with([' ', '\t']);
            if let Some(previous) = previous {
                if !literal && previous && leading {
                    if breaks == 0 {
                        out.push(' ');
                    }
                } else {
                    out.push('\n');
                }
            }
            for _ in 0..breaks {
                out.push('\n');
            }
            breaks = 0;
            out.push_str(content);
            previous = Some(leading);
        } else if child.text.is_empty() {
            breaks += 1;
        } else if child.text.starts_with('\t') {
            return Err(super::tab_error(child.number));
        } else if child.is_comment() {
            // A comment indented less than the content ends the block.
            ended_on = Some(child.number);
        } else {
            return Err(format!(
                "line {}: indented less than the block scalar's first line ({indent} spaces); \
                 PyYAML ends the block there and then refuses the line",
                child.number
            ));
        }
    }
    Ok(Value::Scalar(out.trim().to_string()))
}

/// The name of an anchor or alias after its `&` or `*`: letters, digits, `-`
/// and `_`, ended as `PyYAML` requires by a space, the line end or one of
/// `?:,]}%@` and `` ` ``. Returns the name and the text after it.
pub(super) fn anchor_name(text: &str, line: usize) -> Result<(&str, &str), String> {
    let len = text
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        .count();
    let (name, after) = text.split_at(len);
    let ends_well = after
        .chars()
        .next()
        .is_none_or(|c| c == ' ' || c == '\t' || "?:,]}%@`".contains(c));
    if name.is_empty() || !ends_well {
        return Err(format!(
            "line {line}: an anchor or alias name must be letters, digits, '-' and '_' (PyYAML \
             refuses '{}')",
            text.split(' ').next().unwrap_or(text)
        ));
    }
    Ok((name, after))
}
