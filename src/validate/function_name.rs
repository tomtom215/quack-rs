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
/// [`validate_function_name`] rejects them (case-insensitively). Most of the
/// other keywords (`unreserved`, `column_name`, `type_function`) are callable
/// unquoted and stay allowed — `DuckDB` itself ships `left`, `similar` and
/// `year` — but not all: see [`DUCKDB_UNCALLABLE_KEYWORDS`].
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

/// Non-reserved keywords that still cannot be a function's name, lowercase
/// and sorted.
///
/// `SELECT between(1)`, `SELECT values(1)` and `SELECT time(1)` are parser
/// errors whatever function is registered under the name, and some calls are
/// taken by the grammar before any function lookup: `coalesce(x)` is the
/// `COALESCE` operator, so a function called `coalesce` is never called.
///
/// Measured, not taken from a keyword category: for each keyword `kw` in
/// `duckdb_keywords()` and each argument count from 0 to 3, a macro `"kw"`
/// taking that many arguments is created and called as `kw(41, …)`. These
/// are the non-reserved keywords for which no such call reaches the macro —
/// the same 53 on `DuckDB` 1.4.4, 1.5.0 and 1.5.5 (`duckdb_keywords()` lists
/// 489 keywords on each). `nullif` is the one keyword that fails with one
/// argument but works with two, so it is not refused. `position` is listed
/// although `DuckDB` ships a function of that name: it is reachable only
/// through the `position(a IN b)` syntax, never as `position(a, b)`. The
/// sweep calls the name as a scalar; a table function of the same name is
/// refused too. An end-to-end test repeats the sweep against the linked
/// engine.
pub const DUCKDB_UNCALLABLE_KEYWORDS: [&str; 53] = [
    "anti",
    "between",
    "bigint",
    "bit",
    "boolean",
    "by",
    "char",
    "character",
    "coalesce",
    "columns",
    "dec",
    "decimal",
    "exists",
    "extract",
    "float",
    "grouping",
    "grouping_id",
    "if",
    "inout",
    "int",
    "integer",
    "interval",
    "national",
    "nchar",
    "none",
    "numeric",
    "operator",
    "out",
    "overlay",
    "position",
    "precision",
    "real",
    "semi",
    "setof",
    "smallint",
    "time",
    "timestamp",
    "treat",
    "try_cast",
    "unpack",
    "values",
    "varchar",
    "xmlattributes",
    "xmlconcat",
    "xmlelement",
    "xmlexists",
    "xmlforest",
    "xmlnamespaces",
    "xmlparse",
    "xmlpi",
    "xmlroot",
    "xmlserialize",
    "xmltable",
];

/// Non-reserved keywords that cannot be a macro parameter's name, lowercase
/// and sorted.
///
/// A parameter is referred to by name in the macro's body, and
/// `CREATE MACRO m(left) AS left + 1` fails to parse, as do `join`, `like`,
/// `row` and the rest of this list. Measured the same way as
/// [`DUCKDB_UNCALLABLE_KEYWORDS`] — `CREATE MACRO m(kw) AS kw + 1` then
/// `SELECT m(41)` — and the same 79 on `DuckDB` 1.4.4, 1.5.0 and 1.5.5.
pub const DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS: [&str; 79] = [
    "anti",
    "asof",
    "at",
    "authorization",
    "between",
    "bigint",
    "binary",
    "bit",
    "boolean",
    "by",
    "char",
    "character",
    "coalesce",
    "collation",
    "concurrently",
    "cross",
    "dec",
    "decimal",
    "exists",
    "extract",
    "float",
    "freeze",
    "full",
    "glob",
    "grouping",
    "grouping_id",
    "ilike",
    "inner",
    "inout",
    "int",
    "integer",
    "interval",
    "is",
    "isnull",
    "join",
    "left",
    "like",
    "national",
    "natural",
    "nchar",
    "none",
    "notnull",
    "nullif",
    "numeric",
    "out",
    "outer",
    "overlaps",
    "overlay",
    "position",
    "positional",
    "precision",
    "real",
    "right",
    "row",
    "semi",
    "setof",
    "similar",
    "smallint",
    "substring",
    "tablesample",
    "time",
    "timestamp",
    "treat",
    "trim",
    "unpack",
    "values",
    "varchar",
    "verbose",
    "xmlattributes",
    "xmlconcat",
    "xmlelement",
    "xmlexists",
    "xmlforest",
    "xmlnamespaces",
    "xmlparse",
    "xmlpi",
    "xmlroot",
    "xmlserialize",
    "xmltable",
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
/// - Must not be one of [`DUCKDB_RESERVED_KEYWORDS`] or
///   [`DUCKDB_UNCALLABLE_KEYWORDS`] (compared case-insensitively)
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
/// assert!(validate_function_name("coalesce").is_err()); // the COALESCE operator takes the call
/// assert!(validate_function_name("left").is_ok());      // keyword, but callable
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
    if DUCKDB_UNCALLABLE_KEYWORDS
        .binary_search(&lower.as_str())
        .is_ok()
    {
        return Err(ExtensionError::new(format!(
            "function name '{name}' is a DuckDB keyword that cannot be called as a function: \
             `SELECT {lower}(...)` either fails to parse or is taken by the grammar before any \
             function is looked up; choose another name"
        )));
    }

    Ok(())
}

/// Validates a macro parameter name.
///
/// The rules of [`validate_function_name`], except that the keywords refused
/// are [`DUCKDB_RESERVED_KEYWORDS`] and
/// [`DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS`] — the names a macro body
/// cannot refer to unquoted — rather than [`DUCKDB_UNCALLABLE_KEYWORDS`]. A
/// parameter named `columns` or `if` is fine; one named `left` is not.
///
/// # Errors
///
/// Returns `ExtensionError` describing the first rule violation found.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_parameter_name;
///
/// assert!(validate_parameter_name("threshold").is_ok());
/// assert!(validate_parameter_name("columns").is_ok());
/// assert!(validate_parameter_name("left").is_err());
/// assert!(validate_parameter_name("order").is_err());
/// ```
pub fn validate_parameter_name(name: &str) -> Result<(), ExtensionError> {
    match validate_function_name(name) {
        Ok(()) => {}
        Err(e) => {
            let lower = name.to_ascii_lowercase();
            // Only the name-specific keyword rule is lifted for a parameter.
            if DUCKDB_UNCALLABLE_KEYWORDS
                .binary_search(&lower.as_str())
                .is_err()
            {
                return Err(e);
            }
        }
    }
    let lower = name.to_ascii_lowercase();
    if DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS
        .binary_search(&lower.as_str())
        .is_ok()
    {
        return Err(ExtensionError::new(format!(
            "parameter name '{name}' is a DuckDB keyword a macro body cannot refer to unquoted \
             (`CREATE MACRO m({lower}) AS {lower} + 1` fails to parse); choose another name"
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

    #[test]
    fn uncallable_keywords_are_refused_as_function_names_only() {
        for name in ["coalesce", "Between", "VALUES", "time", "if", "try_cast"] {
            let err = validate_function_name(name).expect_err(name);
            assert!(err.as_str().contains("cannot be called"), "{name}: {err}");
        }
        // `if` and `columns` cannot name a function but can name a parameter.
        assert!(validate_parameter_name("if").is_ok());
        assert!(validate_parameter_name("columns").is_ok());
    }

    #[test]
    fn unreferenceable_keywords_are_refused_as_parameter_names_only() {
        for name in ["left", "JOIN", "like", "row", "between"] {
            let err = validate_parameter_name(name).expect_err(name);
            assert!(err.as_str().contains("cannot refer to"), "{name}: {err}");
        }
        // `left` and `like` stay valid function names: both are callable.
        assert!(validate_function_name("left").is_ok());
        assert!(validate_function_name("like").is_ok());
        // A parameter name gets every other rule of a function name.
        assert!(validate_parameter_name("order").is_err());
        assert!(validate_parameter_name("my-param").is_err());
        assert!(validate_parameter_name("").is_err());
        assert!(validate_parameter_name("threshold").is_ok());
    }

    #[test]
    fn keyword_lists_are_sorted_unique_lowercase_and_non_reserved() {
        for list in [
            &DUCKDB_UNCALLABLE_KEYWORDS[..],
            &DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS[..],
        ] {
            assert!(list.windows(2).all(|w| w[0] < w[1]));
            assert!(list
                .iter()
                .all(|k| k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')));
            assert!(list
                .iter()
                .all(|k| DUCKDB_RESERVED_KEYWORDS.binary_search(k).is_err()));
        }
        assert_eq!(DUCKDB_UNCALLABLE_KEYWORDS.len(), 53);
        assert_eq!(DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS.len(), 79);
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
