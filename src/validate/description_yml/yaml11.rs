// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Which plain scalars `PyYAML` does not read as strings.
//!
//! `duckdb/community-extensions` reads `description.yml` with `PyYAML`'s
//! `yaml.safe_load`, which follows YAML 1.1: an unquoted `yes` is a boolean,
//! `010` is the octal integer 8, `0.10` is the float 0.1 and `2024-01-01` is a
//! date. This module reproduces the implicit resolvers of `PyYAML`'s
//! `yaml/resolver.py` (6.0) for those five types, so a field this crate reads as
//! text can be flagged when the community build would read something else.

/// The type `PyYAML` gives the plain scalar `s`, with its article (for
/// example "an integer"), or `None` for a string.
pub(super) fn implicit_type(s: &str) -> Option<&'static str> {
    if is_null(s) {
        Some("null")
    } else if is_bool(s) {
        Some("a boolean")
    } else if is_int(s) {
        Some("an integer")
    } else if is_float(s) {
        Some("a float")
    } else if is_timestamp(s) {
        Some("a timestamp")
    } else {
        None
    }
}

fn is_null(s: &str) -> bool {
    matches!(s, "" | "~" | "null" | "Null" | "NULL")
}

fn is_bool(s: &str) -> bool {
    matches!(
        s,
        "yes"
            | "Yes"
            | "YES"
            | "no"
            | "No"
            | "NO"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | "on"
            | "On"
            | "ON"
            | "off"
            | "Off"
            | "OFF"
    )
}

/// Strips one leading `-` or `+`.
fn unsigned(s: &str) -> &str {
    s.strip_prefix(['-', '+']).unwrap_or(s)
}

/// Non-empty and every byte accepted by `ok`.
fn all(s: &str, ok: impl Fn(u8) -> bool) -> bool {
    !s.is_empty() && s.bytes().all(ok)
}

const fn digit_or_underscore(b: u8) -> bool {
    b.is_ascii_digit() || b == b'_'
}

/// `(?::[0-5]?[0-9])+` — one or more sexagesimal groups.
fn sexagesimal_groups(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(':') else {
        return false;
    };
    rest.split(':').all(|group| match group.as_bytes() {
        [d] => d.is_ascii_digit(),
        [a, b] => (b'0'..=b'5').contains(a) && b.is_ascii_digit(),
        _ => false,
    })
}

/// `[1-9][0-9_]*` followed by sexagesimal groups (`base` is sign-stripped).
fn sexagesimal(base: &str, first_nonzero: bool) -> bool {
    let end = base.find(':').unwrap_or(base.len());
    let (head, groups) = base.split_at(end);
    let head_ok = match head.as_bytes().first() {
        Some(b'1'..=b'9') => true,
        Some(b'0') => !first_nonzero,
        _ => false,
    } && head.bytes().all(digit_or_underscore);
    head_ok && sexagesimal_groups(groups)
}

fn is_int(s: &str) -> bool {
    let u = unsigned(s);
    if let Some(bits) = u.strip_prefix("0b") {
        return all(bits, |b| matches!(b, b'0' | b'1' | b'_'));
    }
    if let Some(hex) = u.strip_prefix("0x") {
        return all(hex, |b| b.is_ascii_hexdigit() || b == b'_');
    }
    if u == "0" {
        return true;
    }
    if let Some(octal) = u.strip_prefix('0') {
        return all(octal, |b| matches!(b, b'0'..=b'7' | b'_'));
    }
    if matches!(u.as_bytes().first(), Some(b'1'..=b'9')) && u.bytes().all(digit_or_underscore) {
        return true;
    }
    sexagesimal(u, true)
}

/// `(?:[eE][-+][0-9]+)?` — an optional exponent, whose sign is required.
fn optional_exponent(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let Some(rest) = s.strip_prefix(['e', 'E']) else {
        return false;
    };
    let Some(digits) = rest.strip_prefix(['-', '+']) else {
        return false;
    };
    all(digits, |b| b.is_ascii_digit())
}

fn is_float(s: &str) -> bool {
    let u = unsigned(s);
    if matches!(u, ".inf" | ".Inf" | ".INF") {
        return true;
    }
    if matches!(s, ".nan" | ".NaN" | ".NAN") {
        return true;
    }
    let Some(dot) = u.find('.') else {
        return false;
    };
    let (int_part, frac_and_exp) = u.split_at(dot);
    let frac_and_exp = &frac_and_exp[1..];
    // `[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*`: sexagesimal, no exponent.
    if int_part.contains(':') {
        return sexagesimal(int_part, false) && frac_and_exp.bytes().all(digit_or_underscore);
    }
    let exp_at = frac_and_exp.find(['e', 'E']).unwrap_or(frac_and_exp.len());
    let (frac, exp) = frac_and_exp.split_at(exp_at);
    if int_part.is_empty() {
        // `\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?`, with no sign allowed.
        return s.starts_with('.')
            && frac.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && frac.bytes().all(digit_or_underscore)
            && optional_exponent(exp);
    }
    // `[-+]?[0-9][0-9_]*\.[0-9_]*(?:[eE][-+][0-9]+)?`
    int_part.as_bytes()[0].is_ascii_digit()
        && int_part.bytes().all(digit_or_underscore)
        && frac.bytes().all(digit_or_underscore)
        && optional_exponent(exp)
}

/// `n` ASCII digits.
fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// One or two ASCII digits.
fn one_or_two_digits(s: &str) -> bool {
    digits(s, 1) || digits(s, 2)
}

fn is_timestamp(s: &str) -> bool {
    // `[0-9]{4}-[0-9]{2}-[0-9]{2}`: a date.
    let bytes = s.as_bytes();
    if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return digits(&s[..4], 4) && digits(&s[5..7], 2) && digits(&s[8..], 2);
    }
    // `YYYY-M?M-D?D([Tt]|[ \t]+)H?H:MM:SS(\.[0-9]*)?([ \t]*(Z|[-+]H?H(:MM)?))?`
    let Some((date, time)) = s.split_once(['T', 't', ' ', '\t']) else {
        return false;
    };
    let mut date_parts = date.split('-');
    let date_ok = matches!(
        (date_parts.next(), date_parts.next(), date_parts.next(), date_parts.next()),
        (Some(year), Some(month), Some(day), None)
            if digits(year, 4) && one_or_two_digits(month) && one_or_two_digits(day)
    );
    if !date_ok {
        return false;
    }
    let time = time.trim_start_matches([' ', '\t']);
    // Split off a zone: `Z`, or `[-+]` and an hour after the seconds.
    let (clock, zone) = time
        .find(['Z', '+', '-'])
        .map_or((time, ""), |at| time.split_at(at));
    let clock = clock.trim_end_matches([' ', '\t']);
    let zone_ok = zone.is_empty()
        || zone == "Z"
        || zone
            .get(1..)
            .is_some_and(|rest| match rest.split_once(':') {
                Some((hours, minutes)) => one_or_two_digits(hours) && digits(minutes, 2),
                None => one_or_two_digits(rest),
            });
    let (hms, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut hms_parts = hms.split(':');
    let clock_ok = matches!(
        (hms_parts.next(), hms_parts.next(), hms_parts.next(), hms_parts.next()),
        (Some(hour), Some(minute), Some(second), None)
            if one_or_two_digits(hour) && digits(minute, 2) && digits(second, 2)
    ) && fraction.bytes().all(|b| b.is_ascii_digit());
    zone_ok && clock_ok
}

#[cfg(test)]
mod tests {
    use super::implicit_type;

    /// Pinned against `yaml.safe_load` (`PyYAML` 6.0.1): each value, written
    /// unquoted after `v: `, loads as the named type.
    #[test]
    fn matches_pyyaml_on_the_values_that_change_meaning() {
        for (value, want) in [
            ("yes", Some("a boolean")),
            ("on", Some("a boolean")),
            ("Off", Some("a boolean")),
            ("y", None),
            ("null", Some("null")),
            ("~", Some("null")),
            ("010", Some("an integer")),
            ("0123456", Some("an integer")),
            ("0x1f", Some("an integer")),
            ("0b101", Some("an integer")),
            ("1_000", Some("an integer")),
            ("190:20:30", Some("an integer")),
            ("08", None),
            ("0.10", Some("a float")),
            ("1.10", Some("a float")),
            ("1e5", None),
            ("1.0e+5", Some("a float")),
            (".5", Some("a float")),
            ("-.inf", Some("a float")),
            ("2024-01-01", Some("a timestamp")),
            ("2001-12-14t21:59:43.10-05:00", Some("a timestamp")),
            ("1.0.0", None),
            ("v1.0.0", None),
            ("a1b2c3d", None),
            ("my_ext", None),
            ("owner/repo", None),
        ] {
            assert_eq!(implicit_type(value), want, "{value:?}");
        }
    }
}
