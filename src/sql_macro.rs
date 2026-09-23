// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! SQL macro registration for `DuckDB` extensions.
//!
//! SQL macros let you package reusable SQL expressions and queries as
//! named `DuckDB` functions — no FFI callbacks required. This module
//! provides a safe Rust builder for creating both scalar and table macros
//! via `CREATE OR REPLACE MACRO` statements executed during extension
//! initialization.
//!
//! # Macro types
//!
//! | Type | SQL | Returns |
//! |------|-----|---------|
//! | **Scalar** | `AS (expression)` | one value per row |
//! | **Table** | `AS TABLE query`  | a table |
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::sql_macro::SqlMacro;
//! use quack_rs::error::ExtensionError;
//!
//! fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
//!     unsafe {
//!         // Scalar macro: clamp(x, lo, hi) — no C++ needed!
//!         SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))")?
//!             .register(con)?;
//!
//!         // Table macro: active_rows(tbl) — returns filtered rows
//!         SqlMacro::table("active_rows", &["tbl"], "SELECT * FROM tbl WHERE active = true")?
//!             .register(con)?;
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # SQL injection safety
//!
//! Macro names and parameter names are validated against
//! [`validate_function_name`]: an ASCII letter or underscore followed by ASCII
//! letters, digits or underscores (`[A-Za-z_][A-Za-z0-9_]*`, at most 256
//! characters), and not one of `DuckDB`'s reserved keywords. Mixed case is
//! accepted. The keyword rule applies to parameters too: a parameter called
//! `order` could only be referred to in the body as `"order"`, so it is
//! refused rather than left to fail inside the body.
//!
//! The generated SQL always emits the names as **double-quoted identifiers**
//! (`"name"`). The validated character set cannot contain `"`, so quoting
//! needs no escaping. Quoting does not make the name case-sensitive: `DuckDB`
//! resolves identifiers case-insensitively whether or not they were quoted, so
//! a macro registered as `MyMacro` is callable as `mymacro(...)` or
//! `MYMACRO(...)`.
//!
//! The SQL body (`expression` / `query`) is your own extension code, not
//! user-supplied input. **Never build macro bodies from untrusted runtime
//! data.** There is no escaping applied to the body. [`SqlMacro::register`]
//! does refuse a body that turns the statement into several (`1); DROP TABLE
//! t; SELECT (1`), using `DuckDB`'s own parser to count them, but a body can
//! still change the meaning of the one statement it is part of.
//!
//! # Where a macro lives, and what that means
//!
//! A macro is not a function registration: [`SqlMacro::register`] runs
//! `CREATE OR REPLACE MACRO` on the connection, so the macro is an ordinary
//! catalog object in the connection's **default database and schema** — the
//! user's database. That has consequences a registered function does not:
//!
//! - **It persists.** In a database file the macro is written to disk and is
//!   still there in the next session, even if the extension is never loaded
//!   again. Loading the extension again is fine: `CREATE OR REPLACE` simply
//!   replaces it.
//! - **It needs a writable database.** On a database opened read-only the
//!   `CREATE` fails, so an entry point that propagates the error with `?`
//!   makes `LOAD` fail. Decide whether a macro is essential or can be skipped
//!   there.
//! - **It replaces a user's macro of the same name**, silently — that is what
//!   `OR REPLACE` means. Prefix macro names with the extension's name.
//! - **It can shadow a built-in function.** A macro named `abs` in the
//!   default schema is found before the built-in `abs` in the system catalog,
//!   so `abs(-1)` calls the macro.

use std::ffi::{CStr, CString};

use libduckdb_sys::{
    duckdb_connection, duckdb_destroy_result, duckdb_query, duckdb_result, duckdb_result_error,
    DuckDBSuccess,
};

use crate::error::ExtensionError;
use crate::validate::validate_function_name;

/// The body of a SQL macro: a scalar expression or a table query.
///
/// Constructed implicitly by [`SqlMacro::scalar`] and [`SqlMacro::table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroBody {
    /// A SQL expression — generates `AS (expression)`.
    ///
    /// Example: `"greatest(lo, least(hi, x))"`
    Scalar(String),

    /// A SQL query — generates `AS TABLE query`.
    ///
    /// Example: `"SELECT * FROM tbl WHERE active = true"`
    Table(String),
}

/// A SQL macro definition ready to be registered with `DuckDB`.
///
/// Use [`SqlMacro::scalar`] or [`SqlMacro::table`] to construct, then call
/// [`SqlMacro::register`] to install. Use [`SqlMacro::to_sql`] to inspect
/// the generated `CREATE MACRO` statement without a live connection.
///
/// # Example
///
/// ```rust
/// use quack_rs::sql_macro::SqlMacro;
///
/// let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
/// assert_eq!(m.to_sql(), r#"CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)"#);
/// ```
#[derive(Debug, Clone)]
pub struct SqlMacro {
    name: String,
    params: Vec<String>,
    body: MacroBody,
}

impl SqlMacro {
    /// Creates a scalar SQL macro definition.
    ///
    /// Registers as:
    /// ```sql
    /// CREATE OR REPLACE MACRO name(params) AS (expression)
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `name` or any parameter name is invalid.
    /// See [`validate_function_name`]
    /// for naming rules.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::sql_macro::SqlMacro;
    ///
    /// let m = SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))")?;
    /// # Ok::<_, quack_rs::error::ExtensionError>(())
    /// ```
    pub fn scalar(
        name: &str,
        params: &[&str],
        expression: impl Into<String>,
    ) -> Result<Self, ExtensionError> {
        let (name, params) = validate_name_and_params(name, params)?;
        Ok(Self {
            name,
            params,
            body: MacroBody::Scalar(expression.into()),
        })
    }

    /// Creates a table SQL macro definition.
    ///
    /// Registers as:
    /// ```sql
    /// CREATE OR REPLACE MACRO name(params) AS TABLE query
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `name` or any parameter name is invalid.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::sql_macro::SqlMacro;
    ///
    /// let m = SqlMacro::table(
    ///     "active_rows",
    ///     &["tbl"],
    ///     "SELECT * FROM tbl WHERE active = true",
    /// )?;
    /// # Ok::<_, quack_rs::error::ExtensionError>(())
    /// ```
    pub fn table(
        name: &str,
        params: &[&str],
        query: impl Into<String>,
    ) -> Result<Self, ExtensionError> {
        let (name, params) = validate_name_and_params(name, params)?;
        Ok(Self {
            name,
            params,
            body: MacroBody::Table(query.into()),
        })
    }

    /// Returns the `CREATE OR REPLACE MACRO` SQL statement for this definition.
    ///
    /// The macro name and parameter names are emitted as double-quoted
    /// identifiers; see the [module docs][crate::sql_macro#sql-injection-safety].
    /// The body is emitted verbatim — except that a scalar body containing
    /// `--` is followed by a newline before the closing parenthesis, so a
    /// trailing line comment cannot swallow it.
    ///
    /// Useful for logging, testing, and inspection without a live connection.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::sql_macro::SqlMacro;
    ///
    /// let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
    /// assert_eq!(m.to_sql(), r#"CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)"#);
    ///
    /// // A table parameter is read through `query_table`: a bare `FROM tbl` would
    /// // look for a table named `tbl` when the macro is created, and fail.
    /// let t = SqlMacro::table("active_rows", &["tbl"], "SELECT * FROM query_table(tbl) WHERE active = true")
    ///     .unwrap();
    /// assert_eq!(
    ///     t.to_sql(),
    ///     r#"CREATE OR REPLACE MACRO "active_rows"("tbl") AS TABLE SELECT * FROM query_table(tbl) WHERE active = true"#
    /// );
    /// ```
    #[must_use]
    pub fn to_sql(&self) -> String {
        // Validation (`validate_function_name`) guarantees no `"` in any
        // identifier, so wrapping in quotes needs no escaping.
        let quote = |ident: &str| format!("\"{ident}\"");
        let name = quote(&self.name);
        let params = self
            .params
            .iter()
            .map(|p| quote(p))
            .collect::<Vec<_>>()
            .join(", ");
        match &self.body {
            // A `--` comment runs to the end of the line, and would comment
            // out the `)` appended here. Only bodies that could hold one get
            // the newline, so ordinary definitions keep their one-line SQL.
            MacroBody::Scalar(expr) if expr.contains("--") => {
                format!("CREATE OR REPLACE MACRO {name}({params}) AS ({expr}\n)")
            }
            MacroBody::Scalar(expr) => {
                format!("CREATE OR REPLACE MACRO {name}({params}) AS ({expr})")
            }
            MacroBody::Table(query) => {
                format!("CREATE OR REPLACE MACRO {name}({params}) AS TABLE {query}")
            }
        }
    }

    /// Registers this macro on the given connection.
    ///
    /// Executes the `CREATE OR REPLACE MACRO` statement via `duckdb_query` —
    /// after checking with `duckdb_extract_statements`, `DuckDB`'s own parser,
    /// that it is exactly one statement. Read
    /// [where a macro lives](crate::sql_macro#where-a-macro-lives-and-what-that-means)
    /// first: the macro is created in the user's database, persists there,
    /// and fails on a read-only database.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if the body makes the SQL more than one
    /// statement (nothing is executed then), or if `DuckDB` rejects the SQL
    /// statement — for example on a read-only database. The error message is
    /// extracted from `duckdb_result_error`.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open [`duckdb_connection`].
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let sql = self.to_sql();
        // SAFETY: caller guarantees con is valid and open.
        let statements = unsafe { count_statements(con, &sql) }?;
        if statements > 1 {
            return Err(ExtensionError::new(format!(
                "macro '{}': the body turns CREATE MACRO into {statements} SQL statements; a \
                 macro body must be a single expression or query, with no top-level `;`. \
                 Nothing was executed.",
                self.name
            )));
        }
        // SAFETY: caller guarantees con is valid and open.
        unsafe { execute_sql(con, &sql) }
    }

    /// Returns the macro name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the macro parameter names.
    #[must_use]
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// Returns the macro body.
    #[must_use]
    pub const fn body(&self) -> &MacroBody {
        &self.body
    }
}

/// Validates a macro name and all parameter names using the same rules as
/// function names ([`validate_function_name`]).
fn validate_name_and_params(
    name: &str,
    params: &[&str],
) -> Result<(String, Vec<String>), ExtensionError> {
    validate_function_name(name)?;
    for &param in params {
        validate_function_name(param).map_err(|e| param_error(param, e.as_str()))?;
    }
    Ok((
        name.to_owned(),
        params.iter().map(|&p| p.to_owned()).collect(),
    ))
}

/// Rewords a [`validate_function_name`] error for a parameter.
///
/// The validator speaks of a "function name"; for a parameter that is
/// misleading, and its keyword advice (`SELECT order(...)` is a parser error)
/// is about calling a function, not about referring to a parameter.
fn param_error(param: &str, detail: &str) -> ExtensionError {
    if detail.contains("reserved keyword") {
        return ExtensionError::new(format!(
            "invalid parameter name '{param}': it is a reserved keyword in DuckDB's SQL, so the \
             macro body could only refer to it as \"{param}\"; choose another name"
        ));
    }
    let detail = detail
        .strip_prefix("function name")
        .map_or_else(|| detail.to_owned(), |rest| format!("parameter name{rest}"));
    ExtensionError::new(format!("invalid parameter name '{param}': {detail}"))
}

/// Counts the statements `DuckDB`'s parser finds in `sql`.
///
/// Returns 0 when `sql` does not parse; executing it then reports the parse
/// error.
///
/// # Safety
///
/// `con` must be a valid, open [`duckdb_connection`].
unsafe fn count_statements(con: duckdb_connection, sql: &str) -> Result<u64, ExtensionError> {
    let c_sql = CString::new(sql)
        .map_err(|_| ExtensionError::new("SQL statement contains interior null bytes"))?;
    let mut extracted: libduckdb_sys::duckdb_extracted_statements = std::ptr::null_mut();
    // SAFETY: con is valid; c_sql is NUL-terminated; `extracted` is a valid
    // out-pointer, always allocated by DuckDB when `con` and `sql` are non-null.
    let count = unsafe {
        libduckdb_sys::duckdb_extract_statements(con, c_sql.as_ptr(), &raw mut extracted)
    };
    // SAFETY: `extracted` came from `duckdb_extract_statements` (or is null,
    // which the destructor accepts) and is destroyed once.
    unsafe { libduckdb_sys::duckdb_destroy_extracted(&raw mut extracted) };
    Ok(count)
}

/// Executes a SQL statement on `con`, surfacing any `DuckDB` error.
///
/// Always calls `duckdb_destroy_result`, even on failure.
///
/// # Safety
///
/// `con` must be a valid, open [`duckdb_connection`].
unsafe fn execute_sql(con: duckdb_connection, sql: &str) -> Result<(), ExtensionError> {
    let c_sql = CString::new(sql)
        .map_err(|_| ExtensionError::new("SQL statement contains interior null bytes"))?;

    // Zero-initialize: duckdb_result contains only integer and pointer fields,
    // all of which are valid when zero / null.
    //
    // SAFETY: duckdb_result is a C struct; zero is a valid bit pattern for every field.
    let mut result: duckdb_result = unsafe { std::mem::zeroed() };

    // SAFETY: con is valid; c_sql is a valid nul-terminated C string.
    let rc = unsafe { duckdb_query(con, c_sql.as_ptr(), &raw mut result) };

    // Extract the error message before freeing, because duckdb_result_error
    // returns a pointer into the result's internal buffer.
    let outcome = if rc == DuckDBSuccess {
        Ok(())
    } else {
        // SAFETY: result was populated by duckdb_query; duckdb_result_error
        // returns a pointer valid until duckdb_destroy_result.
        let ptr = unsafe { duckdb_result_error(&raw mut result) };
        let msg = if ptr.is_null() {
            "DuckDB macro registration failed (no error message available)".to_string()
        } else {
            // SAFETY: ptr is a valid nul-terminated C string owned by the result.
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        };
        Err(ExtensionError::new(msg))
    };

    // SAFETY: result was populated by duckdb_query and must always be freed.
    unsafe { duckdb_destroy_result(&raw mut result) };

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // to_sql() — pure-Rust, no DuckDB connection needed
    // -----------------------------------------------------------------------

    #[test]
    fn scalar_no_params_to_sql() {
        let m = SqlMacro::scalar("pi", &[], "3.14159265358979").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "pi"() AS (3.14159265358979)"#
        );
    }

    #[test]
    fn scalar_one_param_to_sql() {
        let m = SqlMacro::scalar("double_it", &["x"], "x * 2").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "double_it"("x") AS (x * 2)"#
        );
    }

    #[test]
    fn scalar_multiple_params_to_sql() {
        let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)"#
        );
    }

    #[test]
    fn scalar_complex_expression_to_sql() {
        let m =
            SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "clamp"("x", "lo", "hi") AS (greatest(lo, least(hi, x)))"#
        );
    }

    #[test]
    fn table_no_params_to_sql() {
        let m = SqlMacro::table("all_data", &[], "SELECT 1 AS n").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "all_data"() AS TABLE SELECT 1 AS n"#
        );
    }

    #[test]
    fn table_with_param_to_sql() {
        let m = SqlMacro::table(
            "active_rows",
            &["tbl"],
            "SELECT * FROM tbl WHERE active = true",
        )
        .unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "active_rows"("tbl") AS TABLE SELECT * FROM tbl WHERE active = true"#
        );
    }

    /// Identifiers are always quoted, so a keyword or mixed-case name reaches
    /// `DuckDB` intact instead of as a parser error.
    #[test]
    fn identifiers_are_double_quoted() {
        let m = SqlMacro::scalar("MyMacro", &["X", "_y"], "X + _y").unwrap();
        assert_eq!(
            m.to_sql(),
            r#"CREATE OR REPLACE MACRO "MyMacro"("X", "_y") AS (X + _y)"#
        );
    }

    // -----------------------------------------------------------------------
    // Name and parameter validation
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_case_macro_name_accepted() {
        // DuckDB identifiers are case-insensitive and DuckDB ships mixed-case
        // functions, so rejecting these made a legal name unusable.
        assert!(SqlMacro::scalar("MyMacro", &[], "1").is_ok());
        assert!(SqlMacro::scalar("my-macro", &[], "1").is_err());
    }

    #[test]
    fn invalid_macro_name_hyphen_rejected() {
        assert!(SqlMacro::scalar("my-macro", &[], "1").is_err());
    }

    #[test]
    fn invalid_macro_name_empty_rejected() {
        assert!(SqlMacro::scalar("", &[], "1").is_err());
    }

    #[test]
    fn mixed_case_param_accepted_but_quoting_chars_are_not() {
        assert!(SqlMacro::scalar("f", &["GoodParam"], "1").is_ok());
        let err = SqlMacro::scalar("f", &["bad param"], "1").unwrap_err();
        assert!(err.as_str().contains("bad param"));
        assert!(
            err.as_str()
                .contains("parameter name contains invalid character"),
            "{err}"
        );
        assert!(!err.as_str().contains("function name"), "{err}");
    }

    #[test]
    fn a_keyword_parameter_is_refused_with_a_parameter_specific_message() {
        let err = SqlMacro::scalar("f", &["order"], "1").unwrap_err();
        assert!(
            err.as_str().contains("invalid parameter name 'order'"),
            "{err}"
        );
        assert!(err.as_str().contains("reserved keyword"), "{err}");
        assert!(!err.as_str().contains("function name"), "{err}");
    }

    #[test]
    fn a_scalar_body_with_a_line_comment_ends_with_a_newline() {
        let m = SqlMacro::scalar("f", &["x"], "x + 1 -- plus one").unwrap();
        assert_eq!(
            m.to_sql(),
            "CREATE OR REPLACE MACRO \"f\"(\"x\") AS (x + 1 -- plus one\n)"
        );
    }

    #[test]
    fn invalid_param_hyphen_rejected() {
        assert!(SqlMacro::scalar("f", &["a-b"], "1").is_err());
    }

    #[test]
    fn valid_underscore_prefix_param() {
        assert!(SqlMacro::scalar("f", &["_x"], "1").is_ok());
    }

    #[test]
    fn valid_single_letter_params() {
        let m = SqlMacro::scalar("clamp", &["x", "lo", "hi"], "1").unwrap();
        assert_eq!(m.params(), ["x", "lo", "hi"]);
    }

    #[test]
    fn name_and_params_stored_correctly() {
        let m = SqlMacro::scalar("f", &["a", "b", "c"], "a+b+c").unwrap();
        assert_eq!(m.name(), "f");
        assert_eq!(m.params(), ["a", "b", "c"]);
    }

    // -----------------------------------------------------------------------
    // Body variant accessors
    // -----------------------------------------------------------------------

    #[test]
    fn scalar_body_variant() {
        let m = SqlMacro::scalar("f", &["x"], "x + 1").unwrap();
        assert_eq!(m.body(), &MacroBody::Scalar("x + 1".to_string()));
    }

    #[test]
    fn table_body_variant() {
        let m = SqlMacro::table("t", &[], "SELECT 1").unwrap();
        assert_eq!(m.body(), &MacroBody::Table("SELECT 1".to_string()));
    }

    // -----------------------------------------------------------------------
    // Clone and Debug
    // -----------------------------------------------------------------------

    #[test]
    fn sql_macro_is_cloneable() {
        let m = SqlMacro::scalar("f", &["x"], "x").unwrap();
        let m2 = m.clone();
        assert_eq!(m.to_sql(), m2.to_sql());
    }

    #[test]
    fn macro_body_is_eq() {
        assert_eq!(MacroBody::Scalar("x".into()), MacroBody::Scalar("x".into()));
        assert_ne!(MacroBody::Scalar("x".into()), MacroBody::Table("x".into()));
    }
}
