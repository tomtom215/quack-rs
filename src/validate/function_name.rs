// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SQL function name validation for `DuckDB` extensions.
//!
//! A function name that needs quoting in SQL, or that cannot survive the trip
//! through a C string, will fail at registration or produce a function nobody
//! can call. This validator rejects those, and nothing else.
//!
//! # What it deliberately does not do
//!
//! It does not impose a naming *style*. `DuckDB` identifiers are
//! case-insensitive and `DuckDB` itself ships mixed-case functions
//! (`formatReadableSize`, `formatReadableDecimalSize`), so `myFunc` registers
//! fine and is callable as `myfunc`, `MYFUNC` or `myFunc` — verified against
//! `DuckDB` 1.5.5. `snake_case` is the overwhelming convention in `DuckDB`'s own
//! catalog and is worth following, but it is a convention, not a rule, and this
//! validator gates [`ScalarFunctionBuilder::try_new`][crate::scalar::ScalarFunctionBuilder::try_new]
//! — so enforcing it here would make a legitimate function name
//! *unregisterable*.

use crate::error::ExtensionError;

/// Maximum length for a function name.
///
/// `DuckDB` does not publicly document a hard limit, but names beyond 256
/// characters are unreasonable and may cause issues with catalog storage.
const MAX_FUNCTION_NAME_LEN: usize = 256;

/// `DuckDB`'s **reserved** SQL keywords, lowercase and sorted.
///
/// A function with one of these names cannot be called without quoting it —
/// `SELECT order(1)` is a parser error, only `"order"(1)` works — so
/// [`validate_function_name`] rejects them (case-insensitively). The other
/// keyword categories (`unreserved`, `column_name`, `type_function`) are
/// callable unquoted and stay allowed: `DuckDB` itself ships `left`, `similar`
/// and `year`.
///
/// Taken from `SELECT keyword_name FROM duckdb_keywords() WHERE
/// keyword_category = 'reserved'`, which returns this identical list on
/// `DuckDB` 1.4.4, 1.5.0 and 1.5.5. An end-to-end test compares it against the
/// linked engine, so a `DuckDB` release that changes the set fails CI.
pub const DUCKDB_RESERVED_KEYWORDS: [&str; 75] = [
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "both",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "constraint",
    "create",
    "default",
    "deferrable",
    "desc",
    "describe",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "false",
    "fetch",
    "for",
    "foreign",
    "from",
    "group",
    "having",
    "in",
    "initially",
    "intersect",
    "into",
    "lambda",
    "lateral",
    "leading",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "pivot",
    "pivot_longer",
    "pivot_wider",
    "placing",
    "primary",
    "qualify",
    "references",
    "returning",
    "select",
    "show",
    "some",
    "summarize",
    "symmetric",
    "table",
    "then",
    "to",
    "trailing",
    "true",
    "union",
    "unique",
    "unpivot",
    "using",
    "variadic",
    "when",
    "where",
    "window",
    "with",
];

/// Validates a `DuckDB` function name.
///
/// # Rules
///
/// - Must not be empty
/// - Must not exceed 256 characters
/// - Must start with an ASCII letter or underscore
/// - Must contain only ASCII letters, digits, or underscores
/// - Must not contain interior null bytes
/// - Must not be one of [`DUCKDB_RESERVED_KEYWORDS`] (compared
///   case-insensitively)
///
/// Every one of these is something that would actually break: a name needing
/// quotes in SQL (including a reserved keyword such as `order`), a name
/// starting with a digit that the parser reads as a number, or a name a C
/// string truncates. Casing is **not** checked — see the
/// [module docs][crate::validate::function_name] for why enforcing `snake_case`
/// here would make a name `DuckDB` accepts unregisterable.
///
/// # Errors
///
/// Returns `ExtensionError` describing the first rule violation found.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_function_name;
///
/// assert!(validate_function_name("word_count").is_ok());
/// assert!(validate_function_name("my_func_v2").is_ok());
/// assert!(validate_function_name("_internal").is_ok());
/// // DuckDB ships `formatReadableSize`; mixed case is legal.
/// assert!(validate_function_name("formatReadableSize").is_ok());
///
/// assert!(validate_function_name("").is_err());        // empty
/// assert!(validate_function_name("my-func").is_err());  // needs quoting in SQL
/// assert!(validate_function_name("1func").is_err());    // parsed as a number
/// assert!(validate_function_name("my func").is_err());  // needs quoting in SQL
/// assert!(validate_function_name("order").is_err());    // reserved keyword
/// assert!(validate_function_name("left").is_ok());      // keyword, but not reserved
/// ```
pub fn validate_function_name(name: &str) -> Result<(), ExtensionError> {
    if name.is_empty() {
        return Err(ExtensionError::new("function name must not be empty"));
    }

    if name.len() > MAX_FUNCTION_NAME_LEN {
        return Err(ExtensionError::new(format!(
            "function name must not exceed {MAX_FUNCTION_NAME_LEN} characters, got {}",
            name.len()
        )));
    }

    // Check for interior null bytes (would truncate the CString)
    if name.bytes().any(|b| b == 0) {
        return Err(ExtensionError::new(
            "function name must not contain null bytes",
        ));
    }

    let first = name.as_bytes()[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return Err(ExtensionError::new(format!(
            "function name must start with a letter or underscore, got '{}'",
            name.chars().next().unwrap_or('?')
        )));
    }

    for (i, ch) in name.chars().enumerate() {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err(ExtensionError::new(format!(
                "function name contains invalid character '{ch}' at position {i}; \
                 only letters, digits, and underscores are allowed (a name needing \
                 quotes in SQL is not worth the trouble it causes callers)"
            )));
        }
    }

    // Only reached for an all-ASCII name, so ASCII case folding is exact.
    let lower = name.to_ascii_lowercase();
    if DUCKDB_RESERVED_KEYWORDS
        .binary_search(&lower.as_str())
        .is_ok()
    {
        return Err(ExtensionError::new(format!(
            "function name '{name}' is a reserved keyword in DuckDB's SQL: `SELECT {lower}(...)` \
             is a parser error, so callers would have to write `\"{lower}\"(...)`; \
             choose another name"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple() {
        assert!(validate_function_name("word_count").is_ok());
    }

    #[test]
    fn valid_with_digits() {
        assert!(validate_function_name("my_func_v2").is_ok());
    }

    #[test]
    fn valid_underscore_prefix() {
        assert!(validate_function_name("_internal").is_ok());
    }

    #[test]
    fn valid_single_char() {
        assert!(validate_function_name("f").is_ok());
    }

    #[test]
    fn mixed_case_is_accepted_because_duckdb_accepts_it() {
        // DuckDB ships these two, and registering a camelCase name through the
        // C API succeeds — verified against DuckDB 1.5.5 in
        // tests/ffi_roundtrip.rs. Since this validator gates
        // `ScalarFunctionBuilder::try_new`, rejecting them made a name DuckDB
        // accepts impossible to register at all.
        assert!(validate_function_name("formatReadableSize").is_ok());
        assert!(validate_function_name("formatReadableDecimalSize").is_ok());
        assert!(validate_function_name("MyFunc").is_ok());
        assert!(validate_function_name("_Internal2").is_ok());
    }

    #[test]
    fn names_needing_quotes_are_still_rejected() {
        for name in [
            "my-func", "my func", "my.func", "my\"func", "my'func", "1func", "+",
        ] {
            assert!(
                validate_function_name(name).is_err(),
                "{name} should be rejected"
            );
        }
    }

    /// `SELECT order(1)` is a parser error in `DuckDB` 1.4.4–1.5.5; only
    /// `"order"(1)` works. The module promises names needing quotes are
    /// rejected, and reserved keywords are exactly such names.
    #[test]
    fn reserved_keywords_are_rejected_in_any_case() {
        for name in [
            "order",
            "select",
            "from",
            "group",
            "table",
            "case",
            "ORDER",
            "Select",
            "window",
            "pivot_longer",
        ] {
            let err = validate_function_name(name)
                .expect_err(&format!("reserved keyword {name} must be rejected"));
            assert!(err.as_str().contains("reserved keyword"), "{name}: {err}");
        }
    }

    /// Unreserved, column-name and type/function keywords are callable
    /// unquoted (`SELECT similar(1)`, `SELECT left('ab', 1)` work), so they
    /// must stay registerable.
    #[test]
    fn non_reserved_keywords_are_accepted() {
        for name in [
            "left", "similar", "overlaps", "year", "filter", "count", "orders",
        ] {
            assert!(validate_function_name(name).is_ok(), "{name}");
        }
    }

    /// Pins the list's shape: sorted, unique, lowercase, and the size `DuckDB`
    /// 1.4.4, 1.5.0 and 1.5.5 all report. `tests/ffi_roundtrip/tooling.rs`
    /// compares it against the linked engine's `duckdb_keywords()`.
    #[test]
    fn reserved_keyword_list_is_sorted_unique_lowercase() {
        assert_eq!(DUCKDB_RESERVED_KEYWORDS.len(), 75);
        assert!(DUCKDB_RESERVED_KEYWORDS.windows(2).all(|w| w[0] < w[1]));
        assert!(DUCKDB_RESERVED_KEYWORDS
            .iter()
            .all(|k| k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')));
    }

    #[test]
    fn empty_rejected() {
        let err = validate_function_name("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn hyphen_rejected() {
        let err = validate_function_name("my-func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn starts_with_digit_rejected() {
        let err = validate_function_name("1func").unwrap_err();
        assert!(err.as_str().contains("letter or underscore"));
    }

    #[test]
    fn space_rejected() {
        let err = validate_function_name("my func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn special_char_rejected() {
        let err = validate_function_name("my@func").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn null_byte_rejected() {
        let err = validate_function_name("my\0func").unwrap_err();
        assert!(err.as_str().contains("null bytes"));
    }

    #[test]
    fn too_long_rejected() {
        let long_name: String = "a".repeat(257);
        let err = validate_function_name(&long_name).unwrap_err();
        assert!(err.as_str().contains("256 characters"));
    }

    #[test]
    fn max_length_accepted() {
        let max_name: String = "a".repeat(256);
        assert!(validate_function_name(&max_name).is_ok());
    }

    #[test]
    fn semicolon_rejected() {
        let err = validate_function_name("func;drop").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }

    #[test]
    fn quote_rejected() {
        let err = validate_function_name("func'name").unwrap_err();
        assert!(err.as_str().contains("invalid character"));
    }
}
