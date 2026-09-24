// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SPDX license identifier validation for `DuckDB` community extensions.
//!
//! Extensions must declare a recognized open-source license. This module
//! validates that the `extension.license` field is a commonly used SPDX
//! identifier, or an SPDX `AND` / `OR` expression over such identifiers.
//!
//! # Reference
//!
//! <https://spdx.org/licenses/>

use crate::error::ExtensionError;

pub use super::spdx_exceptions::SPDX_LICENSE_EXCEPTIONS;

/// How deeply parentheses may nest in a license expression.
const MAX_NESTING: usize = 64;

/// Commonly used SPDX license identifiers.
///
/// This is a **curated shortlist, not the SPDX registry** — the registry has
/// over 700 entries, and a license absent from this list is very often still
/// perfectly valid. [`validate_spdx_license`] says so rather than claiming the
/// identifier does not exist.
///
/// Every entry is checked against the official registry by
/// `scripts/check-spdx-list.py`, which fails CI on a typo or on an identifier
/// SPDX has deprecated.
///
/// Sorted, and kept sorted, so additions are easy to review.
///
/// # A note on `SSPL-1.0`
///
/// It is a real SPDX identifier and is listed here, but it is **not
/// OSI-approved** — it is source-available rather than open source. If your
/// extension is bound by a policy that requires an OSI-approved license, this
/// list is not the thing that will tell you.
pub const COMMON_SPDX_LICENSES: &[&str] = &[
    "0BSD",
    "AAL",
    "AFL-3.0",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    "Apache-2.0",
    "Artistic-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "BlueOak-1.0.0",
    "CAL-1.0",
    "CAL-1.0-Combined-Work-Exception",
    "CECILL-2.1",
    "CERN-OHL-P-2.0",
    "CERN-OHL-S-2.0",
    "CERN-OHL-W-2.0",
    "ECL-2.0",
    "EFL-2.0",
    "EPL-2.0",
    "EUPL-1.2",
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "ISC",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "MulanPSL-2.0",
    "NCSA",
    "OSL-3.0",
    "PostgreSQL",
    "RPL-1.5",
    "SSPL-1.0",
    "UPL-1.0",
    "Unlicense",
    "Zlib",
];

/// Validates that a license string is a recognized SPDX identifier, or an
/// SPDX license expression built from them.
///
/// Accepted:
///
/// - any identifier in [`COMMON_SPDX_LICENSES`] (case-sensitive, per the SPDX
///   specification);
/// - a user-defined `LicenseRef-<idstring>` reference, which SPDX allows in
///   any expression;
/// - either of those followed by `WITH <exception>`, e.g.
///   `Apache-2.0 WITH LLVM-exception`, where the exception is one of
///   [`SPDX_LICENSE_EXCEPTIONS`] (the whole SPDX exception registry) or a
///   user-defined `AdditionRef-<idstring>` (SPDX 3.0). As in the SPDX grammar,
///   `WITH` binds tightest and applies to a single license, not to a
///   parenthesised expression;
/// - a compound expression joining those with `AND` / `OR` and parentheses,
///   e.g. `MIT OR Apache-2.0` — the Rust ecosystem's default and what several
///   published community extensions declare. Parentheses may nest up to 64
///   deep.
///
/// Not accepted: lowercase operators, and identifiers outside the shortlist —
/// the error for those says they may still be valid rather than claiming they
/// do not exist.
///
/// # Errors
///
/// Returns `ExtensionError` if the license is empty, is not a well-formed
/// expression, nests parentheses more than 64 deep, or names a license or
/// exception identifier not in the recognized lists.
///
/// # Example
///
/// ```rust
/// use quack_rs::validate::validate_spdx_license;
///
/// assert!(validate_spdx_license("MIT").is_ok());
/// assert!(validate_spdx_license("Apache-2.0").is_ok());
/// assert!(validate_spdx_license("BSD-3-Clause").is_ok());
/// assert!(validate_spdx_license("MIT OR Apache-2.0").is_ok());
/// assert!(validate_spdx_license("Apache-2.0 WITH LLVM-exception").is_ok());
/// assert!(validate_spdx_license("FAKE-LICENSE").is_err());
/// assert!(validate_spdx_license("MIT OR").is_err());
/// assert!(validate_spdx_license("").is_err());
/// ```
pub fn validate_spdx_license(license: &str) -> Result<(), ExtensionError> {
    if license.trim().is_empty() {
        return Err(ExtensionError::new("license identifier must not be empty"));
    }

    let tokens = tokenize(license);
    let mut parser = ExprParser {
        rest: &tokens,
        depth: 0,
    };
    let unlisted = parser.expression().and_then(|unlisted| {
        parser.rest.first().map_or(Ok(unlisted), |extra| {
            Err(format!(
                "unexpected '{extra}' in license expression '{license}'"
            ))
        })
    });
    match unlisted {
        Ok(None) => Ok(()),
        // Deliberately not "is not a recognized SPDX identifier": this list is
        // a shortlist of ~40 out of 700+, so saying that would be wrong for
        // most valid identifiers.
        Ok(Some(Unlisted::License(id))) => Err(ExtensionError::new(format!(
            "license '{id}' is not in quack-rs's list of common SPDX identifiers. \
             It may still be valid — check https://spdx.org/licenses/. \
             Common choices: MIT, Apache-2.0, BSD-3-Clause, GPL-3.0-or-later, MPL-2.0"
        ))),
        // The exception list is the whole registry as of the version it was
        // taken from, so only a newer addition can be valid and missing.
        Ok(Some(Unlisted::Exception(id))) => Err(ExtensionError::new(format!(
            "license exception '{id}' is not in the SPDX license-exception list quack-rs \
             carries (SPDX license list 3.29.0). Check the spelling and case at \
             https://spdx.org/licenses/exceptions-index.html; an exception added to SPDX \
             since then may still be valid"
        ))),
        Err(msg) => Err(ExtensionError::new(format!(
            "{msg}; expected an SPDX identifier such as 'MIT' or an expression such as \
             'MIT OR Apache-2.0' — see https://spdx.org/licenses/"
        ))),
    }
}

/// Splits an SPDX expression into identifiers, operators and parentheses.
fn tokenize(expr: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (i, ch) in expr.char_indices() {
        if ch.is_whitespace() || ch == '(' || ch == ')' {
            if let Some(s) = start.take() {
                tokens.push(&expr[s..i]);
            }
            if !ch.is_whitespace() {
                tokens.push(&expr[i..=i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push(&expr[s..]);
    }
    tokens
}

/// A well-formed identifier that is not in the list it was checked against.
#[derive(Clone, Copy)]
enum Unlisted<'a> {
    /// Not in [`COMMON_SPDX_LICENSES`] and not a `LicenseRef-`.
    License(&'a str),
    /// Not in [`SPDX_LICENSE_EXCEPTIONS`] and not an `AdditionRef-`.
    Exception(&'a str),
}

/// A recursive-descent reader for the SPDX license-expression grammar:
/// `WITH` binds tightest, then `AND`, then `OR`.
///
/// Each method returns the first identifier that is well-formed but unlisted
/// (`Ok(Some(..))`), or a syntax error message.
///
/// The cursor is the unread tail of the token slice, and every read shortens
/// it, so each loop pass consumes at least one token and parsing always ends.
/// (An index cursor could be mutated into `pos -= 1` and loop forever.)
///
/// Recursion happens only at `(`, and `depth` caps it at [`MAX_NESTING`], so a
/// hostile `((((…` is an error rather than a stack overflow — which aborts the
/// process and cannot be caught.
struct ExprParser<'a> {
    rest: &'a [&'a str],
    depth: usize,
}

impl<'a> ExprParser<'a> {
    /// Consumes the next token if it is `expected`.
    fn eat(&mut self, expected: &str) -> bool {
        match self.rest.split_first() {
            Some((&token, tail)) if token == expected => {
                self.rest = tail;
                true
            }
            _ => false,
        }
    }

    fn expression(&mut self) -> Result<Option<Unlisted<'a>>, String> {
        let mut unlisted = self.term()?;
        while self.eat("OR") {
            let next = self.term()?;
            unlisted = unlisted.or(next);
        }
        Ok(unlisted)
    }

    fn term(&mut self) -> Result<Option<Unlisted<'a>>, String> {
        let mut unlisted = self.atom()?;
        while self.eat("AND") {
            let next = self.atom()?;
            unlisted = unlisted.or(next);
        }
        Ok(unlisted)
    }

    fn atom(&mut self) -> Result<Option<Unlisted<'a>>, String> {
        let Some((&token, tail)) = self.rest.split_first() else {
            return Err("license expression ends where an identifier was expected".into());
        };
        self.rest = tail;
        match token {
            "(" => {
                if self.depth >= MAX_NESTING {
                    return Err(format!(
                        "license expression nests parentheses more than {MAX_NESTING} deep"
                    ));
                }
                self.depth += 1;
                let unlisted = self.expression()?;
                self.depth -= 1;
                if self.eat(")") {
                    Ok(unlisted)
                } else {
                    Err("unbalanced '(' in license expression".into())
                }
            }
            ")" | "AND" | "OR" | "WITH" => {
                Err(format!("'{token}' where a license identifier was expected"))
            }
            id => {
                let license = (!COMMON_SPDX_LICENSES.contains(&id) && !is_license_ref(id))
                    .then_some(Unlisted::License(id));
                let exception = if self.eat("WITH") {
                    self.exception()?
                } else {
                    None
                };
                Ok(license.or(exception))
            }
        }
    }

    /// The exception after `WITH`: a single identifier, never an expression.
    fn exception(&mut self) -> Result<Option<Unlisted<'a>>, String> {
        let Some((&token, tail)) = self.rest.split_first() else {
            return Err("license expression ends where a license exception was expected".into());
        };
        self.rest = tail;
        match token {
            "(" | ")" | "AND" | "OR" | "WITH" => Err(format!(
                "'{token}' where a license exception identifier was expected after 'WITH'"
            )),
            id if SPDX_LICENSE_EXCEPTIONS.contains(&id) || is_addition_ref(id) => Ok(None),
            id => Ok(Some(Unlisted::Exception(id))),
        }
    }
}

/// `LicenseRef-<idstring>`, where `idstring` is letters, digits, `.` and `-`.
fn is_license_ref(id: &str) -> bool {
    is_user_ref(id, "LicenseRef-")
}

/// `AdditionRef-<idstring>`: SPDX 3.0's user-defined license exception.
fn is_addition_ref(id: &str) -> bool {
    is_user_ref(id, "AdditionRef-")
}

fn is_user_ref(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mit_accepted() {
        assert!(validate_spdx_license("MIT").is_ok());
    }

    #[test]
    fn apache_accepted() {
        assert!(validate_spdx_license("Apache-2.0").is_ok());
    }

    #[test]
    fn bsd_3_clause_accepted() {
        assert!(validate_spdx_license("BSD-3-Clause").is_ok());
    }

    #[test]
    fn gpl_accepted() {
        assert!(validate_spdx_license("GPL-3.0-only").is_ok());
        assert!(validate_spdx_license("GPL-2.0-or-later").is_ok());
    }

    #[test]
    fn unlicense_accepted() {
        assert!(validate_spdx_license("Unlicense").is_ok());
    }

    /// `MIT OR Apache-2.0` is the Rust ecosystem's default licence and is
    /// what four published community extensions declare; the validator's own
    /// module notes said expressions were fine, but it rejected them.
    #[test]
    fn expressions_of_listed_identifiers_are_accepted() {
        for expr in [
            "MIT OR Apache-2.0",
            "(MIT OR Apache-2.0)",
            "Apache-2.0 AND MIT",
            "MIT OR (Apache-2.0 AND BSD-3-Clause)",
            "LicenseRef-Proprietary",
            "MIT OR LicenseRef-My.Terms-2",
        ] {
            assert!(validate_spdx_license(expr).is_ok(), "{expr}");
        }
    }

    #[test]
    fn malformed_or_unlisted_expressions_are_rejected() {
        for expr in [
            "MIT OR",
            "OR MIT",
            "MIT Apache-2.0",
            "(MIT OR Apache-2.0",
            "MIT OR Apache-2.0)",
            "MIT or Apache-2.0",
            "MIT OR CC0-1.0",
            "()",
            "LicenseRef-",
            "BSL 1.1",
        ] {
            assert!(validate_spdx_license(expr).is_err(), "{expr}");
        }
    }

    /// Regression: the recursive-descent reader recursed once per `(`, so a
    /// licence field of a million `(` overflowed the stack and aborted the
    /// process — reachable from `parse_description_yml` on untrusted input.
    #[test]
    fn deeply_nested_parentheses_are_an_error_not_a_stack_overflow() {
        // Under Miri a million characters takes hours; 1,000 is still far
        // past the 64-level limit, so the recursion bound is exercised the
        // same way. The timing bound only means something natively.
        let depth = if cfg!(miri) { 1_000 } else { 1_000_000 };
        let started = std::time::Instant::now();
        let err = validate_spdx_license(&"(".repeat(depth)).unwrap_err();
        assert!(err.as_str().contains("nests parentheses"), "{err}");
        if !cfg!(miri) {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
        }

        let yml = format!(
            "extension:\n  name: my_ext\n  description: d\n  language: Rust\n  build: cargo\n  \
             license: \"{}\"\n  maintainers:\n    - a\nrepo:\n  github: a/b\n  ref: main\n",
            "(".repeat(depth)
        );
        let parsed = crate::validate::description_yml::parse_description_yml(&yml)
            .expect("an unusable license is a warning, not a parse failure");
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.contains("nests parentheses")),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn nesting_is_accepted_up_to_the_limit() {
        let nested = |depth: usize| format!("{}MIT{}", "(".repeat(depth), ")".repeat(depth));
        assert!(validate_spdx_license(&nested(MAX_NESTING)).is_ok());
        assert!(validate_spdx_license(&nested(MAX_NESTING + 1)).is_err());
    }

    /// The limit is on nesting depth, not on how many parenthesised groups
    /// an expression has: closing a group returns to the outer depth, so many
    /// sibling groups — each nested right up to the limit — are accepted.
    #[test]
    fn sibling_groups_do_not_count_towards_the_nesting_limit() {
        let group = format!("{}MIT{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
        let many = vec![group.as_str(); MAX_NESTING + 2].join(" AND ");
        assert!(validate_spdx_license(&many).is_ok(), "{many}");
        let flat = vec!["(MIT)"; 2 * MAX_NESTING].join(" OR ");
        assert!(validate_spdx_license(&flat).is_ok(), "{flat}");
    }

    /// `WITH <exception>` is SPDX's license-exception syntax, e.g. the
    /// `Apache-2.0 WITH LLVM-exception` that LLVM-derived code carries.
    #[test]
    fn with_exception_is_accepted() {
        for expr in [
            "Apache-2.0 WITH LLVM-exception",
            "GPL-2.0-or-later WITH Classpath-exception-2.0",
            "(MIT OR Apache-2.0 WITH LLVM-exception)",
            "MIT OR Apache-2.0 WITH LLVM-exception AND ISC",
            "LicenseRef-Mine WITH GCC-exception-3.1",
            "GPL-3.0-or-later WITH AdditionRef-My.Exception-1",
        ] {
            assert!(validate_spdx_license(expr).is_ok(), "{expr}");
        }
    }

    #[test]
    fn malformed_with_is_rejected() {
        for expr in [
            "MIT WITH",
            "WITH LLVM-exception",
            "MIT WITH LLVM-exception WITH LLVM-exception",
            "MIT WITH (LLVM-exception)",
            "(MIT OR ISC) WITH LLVM-exception",
            "MIT WITH MIT",
            "MIT with LLVM-exception",
            "MIT WITH AdditionRef-",
        ] {
            assert!(validate_spdx_license(expr).is_err(), "{expr}");
        }
    }

    #[test]
    fn unknown_exception_is_named_and_not_called_invalid() {
        let err = validate_spdx_license("MIT WITH Made-Up-exception").unwrap_err();
        assert!(err.as_str().contains("Made-Up-exception"), "{err}");
        assert!(err.as_str().contains("exceptions-index"), "{err}");
    }

    #[test]
    fn unlisted_license_with_a_real_exception_reports_the_license() {
        let err = validate_spdx_license("CC0-1.0 WITH LLVM-exception").unwrap_err();
        assert!(err.as_str().contains("'CC0-1.0'"), "{err}");
        assert!(err.as_str().contains("may still be valid"), "{err}");
    }

    #[test]
    fn exception_list_is_sorted_and_unique() {
        let mut sorted = SPDX_LICENSE_EXCEPTIONS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted.as_slice(), SPDX_LICENSE_EXCEPTIONS);
        sorted.dedup();
        assert_eq!(sorted.len(), SPDX_LICENSE_EXCEPTIONS.len());
    }

    #[test]
    fn empty_rejected() {
        let err = validate_spdx_license("").unwrap_err();
        assert!(err.as_str().contains("empty"));
    }

    #[test]
    fn unknown_license_rejected() {
        let err = validate_spdx_license("FAKE-LICENSE").unwrap_err();
        assert!(err.as_str().contains("not in quack-rs's list"));
    }

    #[test]
    fn rejection_does_not_claim_the_identifier_is_invalid() {
        // `CC0-1.0` is a real SPDX identifier that this shortlist omits. The
        // message must send the reader to the registry, not tell them their
        // perfectly valid license does not exist.
        let err = validate_spdx_license("CC0-1.0").unwrap_err();
        assert!(
            err.as_str().contains("may still be valid"),
            "misleading message: {err}"
        );
        assert!(err.as_str().contains("spdx.org/licenses"));
    }

    #[test]
    fn list_is_sorted_and_unique() {
        let mut sorted = COMMON_SPDX_LICENSES.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted.as_slice(),
            COMMON_SPDX_LICENSES,
            "keep the list sorted so additions are reviewable"
        );
        sorted.dedup();
        assert_eq!(sorted.len(), COMMON_SPDX_LICENSES.len());
    }

    #[test]
    fn case_sensitive() {
        // SPDX identifiers are case-sensitive
        assert!(validate_spdx_license("mit").is_err());
        assert!(validate_spdx_license("apache-2.0").is_err());
    }

    #[test]
    fn all_listed_licenses_validate() {
        for &license in COMMON_SPDX_LICENSES {
            assert!(
                validate_spdx_license(license).is_ok(),
                "expected '{license}' to be accepted"
            );
        }
    }
}
