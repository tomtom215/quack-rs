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
//! [`validate_function_name`](crate::validate::validate_function_name):
//! only `[a-z][a-z0-9_]*` identifiers are accepted. These names are
//! interpolated literally into the generated SQL (no quoting required
//! because they are already restricted to safe characters).
//!
//! The SQL body (`expression` / `query`) is your own extension code, not
//! user-supplied input. **Never build macro bodies from untrusted runtime
//! data.** There is no escaping applied to the body.

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
/// assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO add(a, b) AS (a + b)");
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
    /// See [`validate_function_name`](crate::validate::validate_function_name)
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
    /// Useful for logging, testing, and inspection without a live connection.
    ///
    /// # Example
    ///
    /// ```rust
    /// use quack_rs::sql_macro::SqlMacro;
    ///
    /// let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
    /// assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO add(a, b) AS (a + b)");
    ///
    /// let t = SqlMacro::table("active_rows", &["tbl"], "SELECT * FROM tbl WHERE active = true").unwrap();
    /// assert_eq!(
    ///     t.to_sql(),
    ///     "CREATE OR REPLACE MACRO active_rows(tbl) AS TABLE SELECT * FROM tbl WHERE active = true"
    /// );
    /// ```
    #[must_use]
    pub fn to_sql(&self) -> String {
        let params = self.params.join(", ");
        match &self.body {
            MacroBody::Scalar(expr) => {
                format!(
                    "CREATE OR REPLACE MACRO {}({}) AS ({})",
                    self.name, params, expr
                )
            }
            MacroBody::Table(query) => {
                format!(
                    "CREATE OR REPLACE MACRO {}({}) AS TABLE {}",
                    self.name, params, query
                )
            }
        }
    }

    /// Registers this macro on the given connection.
    ///
    /// Executes the `CREATE OR REPLACE MACRO` statement via `duckdb_query`.
    ///
    /// # Errors
    ///
    /// Returns [`ExtensionError`] if `DuckDB` rejects the SQL statement.
    /// The error message is extracted from `duckdb_result_error`.
    ///
    /// # Safety
    ///
    /// `con` must be a valid, open [`duckdb_connection`].
    pub unsafe fn register(self, con: duckdb_connection) -> Result<(), ExtensionError> {
        let sql = self.to_sql();
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
    pub fn body(&self) -> &MacroBody {
        &self.body
    }
}

/// Validates a macro name and all parameter names using the same rules as
/// function names: `[a-z_][a-z0-9_]*`, max 256 chars.
fn validate_name_and_params(
    name: &str,
    params: &[&str],
) -> Result<(String, Vec<String>), ExtensionError> {
    validate_function_name(name)?;
    for &param in params {
        validate_function_name(param).map_err(|e| {
            ExtensionError::new(format!("invalid parameter name '{param}': {e}"))
        })?;
    }
    Ok((
        name.to_owned(),
        params.iter().map(|&p| p.to_owned()).collect(),
    ))
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
            "CREATE OR REPLACE MACRO pi() AS (3.14159265358979)"
        );
    }

    #[test]
    fn scalar_one_param_to_sql() {
        let m = SqlMacro::scalar("double_it", &["x"], "x * 2").unwrap();
        assert_eq!(
            m.to_sql(),
            "CREATE OR REPLACE MACRO double_it(x) AS (x * 2)"
        );
    }

    #[test]
    fn scalar_multiple_params_to_sql() {
        let m = SqlMacro::scalar("add", &["a", "b"], "a + b").unwrap();
        assert_eq!(m.to_sql(), "CREATE OR REPLACE MACRO add(a, b) AS (a + b)");
    }

    #[test]
    fn scalar_complex_expression_to_sql() {
        let m =
            SqlMacro::scalar("clamp", &["x", "lo", "hi"], "greatest(lo, least(hi, x))").unwrap();
        assert_eq!(
            m.to_sql(),
            "CREATE OR REPLACE MACRO clamp(x, lo, hi) AS (greatest(lo, least(hi, x)))"
        );
    }

    #[test]
    fn table_no_params_to_sql() {
        let m = SqlMacro::table("all_data", &[], "SELECT 1 AS n").unwrap();
        assert_eq!(
            m.to_sql(),
            "CREATE OR REPLACE MACRO all_data() AS TABLE SELECT 1 AS n"
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
            "CREATE OR REPLACE MACRO active_rows(tbl) AS TABLE SELECT * FROM tbl WHERE active = true"
        );
    }

    // -----------------------------------------------------------------------
    // Name and parameter validation
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_macro_name_uppercase_rejected() {
        assert!(SqlMacro::scalar("MyMacro", &[], "1").is_err());
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
    fn invalid_param_uppercase_rejected() {
        let err = SqlMacro::scalar("f", &["BadParam"], "1").unwrap_err();
        assert!(err.as_str().contains("BadParam"));
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
