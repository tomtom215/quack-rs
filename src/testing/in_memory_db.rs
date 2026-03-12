// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! In-memory `DuckDB` helper for integration tests.
//!
//! Available only when the `bundled-test` feature is enabled. Provides
//! [`InMemoryDb`], which opens a real in-memory `DuckDB` database using the
//! bundled `duckdb` Rust crate — bypassing the `loadable-extension` dispatch
//! mechanism entirely.
//!
//! # What `InMemoryDb` is for
//!
//! - Executing SQL statements and verifying results (e.g., after registering a
//!   SQL macro via its raw SQL string).
//! - Seeding test data and running `SELECT` queries to validate logic.
//! - Any test that needs a live `DuckDB` connection but doesn't need to go
//!   through quack-rs's FFI wrappers.
//!
//! # What `InMemoryDb` is NOT for
//!
//! `InMemoryDb` cannot be used to test quack-rs's FFI callback wrappers
//! (`VectorReader`, `VectorWriter`, `BindInfo`, etc.) because those wrappers
//! route through the `loadable-extension` dispatch table, which is uninitialized
//! in `cargo test`. For callback logic, use [`MockVectorReader`] and
//! [`MockVectorWriter`] instead.
//!
//! [`MockVectorReader`]: crate::testing::MockVectorReader
//! [`MockVectorWriter`]: crate::testing::MockVectorWriter
//!
//! # Enabling this feature
//!
//! ```toml
//! # In your extension's Cargo.toml:
//! [dev-dependencies]
//! quack-rs = { version = "0.5", features = ["bundled-test"] }
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! # #[cfg(feature = "bundled-test")]
//! # {
//! use quack_rs::testing::InMemoryDb;
//!
//! let db = InMemoryDb::open().unwrap();
//!
//! // Execute a SQL macro directly (SQL-level test, no FFI needed)
//! db.execute_batch("CREATE MACRO double(x) AS (x * 2)").unwrap();
//!
//! let result: i64 = db.query_one("SELECT double(21)").unwrap();
//! assert_eq!(result, 42);
//! # }
//! ```

/// An in-memory `DuckDB` database for integration testing.
///
/// Wraps [`duckdb::Connection`] opened in in-memory mode. Only available
/// when the `bundled-test` feature is enabled.
///
/// See the [module documentation][self] for usage examples and limitations.
pub struct InMemoryDb {
    conn: duckdb::Connection,
}

impl InMemoryDb {
    /// Opens a new in-memory `DuckDB` database.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` fails to initialize an in-memory database
    /// (extremely unlikely in practice).
    pub fn open() -> Result<Self, duckdb::Error> {
        Ok(Self {
            conn: duckdb::Connection::open_in_memory()?,
        })
    }

    /// Executes one or more SQL statements separated by semicolons.
    ///
    /// Useful for `CREATE TABLE`, `INSERT`, `CREATE MACRO`, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if any statement fails.
    pub fn execute_batch(&self, sql: &str) -> Result<(), duckdb::Error> {
        self.conn.execute_batch(sql)
    }

    /// Executes a single SQL statement and returns the number of affected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the statement fails.
    pub fn execute(&self, sql: &str) -> Result<usize, duckdb::Error> {
        self.conn.execute(sql, [])
    }

    /// Executes a query that returns a single value and returns it.
    ///
    /// This is a convenience helper for `SELECT` expressions that produce exactly
    /// one row and one column.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails, returns no rows, or the value
    /// cannot be converted to `T`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "bundled-test")]
    /// # {
    /// use quack_rs::testing::InMemoryDb;
    ///
    /// let db = InMemoryDb::open().unwrap();
    /// let answer: i64 = db.query_one("SELECT 6 * 7").unwrap();
    /// assert_eq!(answer, 42);
    /// # }
    /// ```
    pub fn query_one<T>(&self, sql: &str) -> Result<T, duckdb::Error>
    where
        T: duckdb::types::FromSql,
    {
        let mut stmt = self.conn.prepare(sql)?;
        stmt.query_row([], |row| row.get(0))
    }

    /// Returns a reference to the underlying [`duckdb::Connection`].
    ///
    /// Use this for queries that don't fit the convenience methods above.
    pub fn conn(&self) -> &duckdb::Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_db_opens() {
        let db = InMemoryDb::open().expect("should open in-memory db");
        let _: i64 = db.query_one("SELECT 1").expect("should query 1");
    }

    #[test]
    fn in_memory_db_execute_batch_and_query() {
        let db = InMemoryDb::open().unwrap();
        db.execute_batch("CREATE TABLE t(v INTEGER); INSERT INTO t VALUES (10), (20), (30)")
            .unwrap();
        let total: i64 = db.query_one("SELECT SUM(v) FROM t").unwrap();
        assert_eq!(total, 60);
    }

    #[test]
    fn in_memory_db_sql_macro() {
        let db = InMemoryDb::open().unwrap();
        // Test SQL macro SQL generation + execution
        use crate::sql_macro::SqlMacro;
        let macro_ = SqlMacro::scalar("triple", &["x"], "x * 3").unwrap();
        db.execute_batch(&macro_.to_sql()).unwrap();
        let result: i64 = db.query_one("SELECT triple(14)").unwrap();
        assert_eq!(result, 42);
    }
}
