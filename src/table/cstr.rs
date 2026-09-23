// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Panic-free `&str` → `CString` conversion for the callback info wrappers.
//!
//! Every `set_error`, column name and parameter name the table, cast, copy and
//! replacement-scan wrappers hand to `DuckDB` goes through here. It lives in
//! `table` because that is where it was first needed; it has no table-specific
//! behaviour.

use std::ffi::CString;

/// Converts a `&str` to a `CString` without panicking.
///
/// If the string contains an interior NUL byte it is truncated at the first
/// one. This runs inside `extern "C"` callbacks, where a panic would abort the
/// process, so `CString::new(..).expect(..)` is not an option.
pub fn str_to_cstring(s: &str) -> CString {
    let end = s.bytes().position(|b| b == 0).unwrap_or(s.len());
    // `s[..end]` holds no NUL by construction, so this cannot fail; the
    // fallback only exists to avoid an `unwrap` in library code.
    CString::new(&s.as_bytes()[..end]).unwrap_or_default()
}

/// Converts an error message, substituting `placeholder` when the message is
/// empty — or empty after truncation at an interior NUL.
///
/// `DuckDB` reports an empty message verbatim (`Binder Error: ` followed by
/// nothing), and some error channels ignore it altogether, so an error the
/// author forgot to describe would otherwise reach the user as no information
/// at all.
pub fn error_cstring(message: &str, placeholder: &str) -> CString {
    let c_msg = str_to_cstring(message);
    if c_msg.as_bytes().is_empty() {
        str_to_cstring(placeholder)
    } else {
        c_msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_to_cstring_keeps_ordinary_text() {
        assert_eq!(str_to_cstring("plain").as_bytes(), b"plain");
        assert_eq!(str_to_cstring("").as_bytes(), b"");
    }

    #[test]
    fn str_to_cstring_truncates_at_the_first_nul() {
        assert_eq!(str_to_cstring("bad\0message\0more").as_bytes(), b"bad");
        assert_eq!(str_to_cstring("\0hidden").as_bytes(), b"");
    }

    #[test]
    fn error_cstring_replaces_only_an_empty_message() {
        assert_eq!(error_cstring("boom", "ph").as_bytes(), b"boom");
        assert_eq!(error_cstring("", "ph").as_bytes(), b"ph");
        assert_eq!(error_cstring("\0hidden", "ph").as_bytes(), b"ph");
        assert_eq!(error_cstring("x\0y", "ph").as_bytes(), b"x");
    }
}
