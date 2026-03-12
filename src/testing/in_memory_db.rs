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

// ── Dispatch-table initialisation ────────────────────────────────────────────
//
// When `bundled-test` is active, Cargo's feature-unification merges the
// `loadable-extension` feature (required by the library to build as a DuckDB
// extension) and the `bundled-full` feature (pulled in by `duckdb` with
// `features = ["bundled"]`) into a single `libduckdb-sys` build.
//
// In `loadable-extension` mode every DuckDB C API call is routed through an
// atomic function-pointer dispatch table that is normally populated by DuckDB
// at extension-load time.  In `cargo test`, no DuckDB host process loads the
// extension, so the table stays uninitialised and every call panics with
// "DuckDB API not initialized or DuckDB feature omitted".
//
// The fix: before opening the first connection, call
// `init_dispatch_table_once()`, which invokes our tiny C++ shim
// (`bundled_api_init.cpp`) to call DuckDB's internal `CreateAPIv1()`.
// That function returns a `duckdb_ext_api_v1` struct with every field set to
// the corresponding bundled DuckDB C function pointer.  We pass this struct
// through the `duckdb_rs_extension_api_init` Rust entry-point so that the
// atomic table is populated in one go — after which the `duckdb` crate can
// open connections and execute queries as usual.

extern "C" {
    /// Calls `DuckDB`'s internal `CreateAPIv1()` and returns the resulting
    /// `duckdb_ext_api_v1` struct with every function pointer set to the
    /// corresponding bundled `DuckDB` symbol.
    ///
    /// Defined in `src/testing/bundled_api_init.cpp`, compiled by `build.rs`.
    fn quack_rs_create_api_v1() -> libduckdb_sys::duckdb_ext_api_v1;
}

/// Populates the `loadable-extension` dispatch table exactly once.
///
/// Uses `std::sync::Once` so it is safe to call from multiple threads and
/// from multiple test cases; subsequent calls are no-ops.
fn init_dispatch_table_once() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // SAFETY: quack_rs_create_api_v1 is a thin C++ wrapper around
        // DuckDB's own CreateAPIv1().  It sets every field of the returned
        // struct to the matching bundled DuckDB C function pointer, so the
        // values are valid function pointers for the lifetime of the process.
        let api = unsafe { quack_rs_create_api_v1() };

        // Box the struct so we can hand a stable pointer to get_api_fn.
        // The allocation is intentionally leaked; it must live for the
        // duration of the process (the dispatch table holds no copy of it
        // — duckdb_rs_extension_api_init reads through the pointer and
        // stores each function pointer into its own atomic, after which
        // the struct is no longer needed).
        let api_ptr = Box::into_raw(Box::new(api));

        // A bare function (no closure captures) that satisfies the
        // duckdb_extension_access::get_api signature.  We pass the API
        // pointer out through a thread-local so that we don't need a
        // capturing closure.
        std::thread_local! {
            static TL_API_PTR: std::cell::Cell<*const libduckdb_sys::duckdb_ext_api_v1> =
                const { std::cell::Cell::new(std::ptr::null()) };
        }
        TL_API_PTR.with(|cell| cell.set(api_ptr));

        unsafe extern "C" fn get_api_fn(
            _info: libduckdb_sys::duckdb_extension_info,
            _version: *const std::os::raw::c_char,
        ) -> *const std::os::raw::c_void {
            TL_API_PTR.with(|cell| cell.get().cast())
        }

        let access = libduckdb_sys::duckdb_extension_access {
            set_error: None,
            get_database: None,
            get_api: Some(get_api_fn),
        };

        // SAFETY: api_ptr is a valid, non-null pointer to a
        // duckdb_ext_api_v1 that lives for the duration of the process.
        // duckdb_rs_extension_api_init reads each field and stores it into
        // the corresponding AtomicPtr, then returns.  The access struct
        // lives on this stack frame and outlives the call.
        // SAFETY: same as above.  std::ptr::addr_of!(access) yields a raw
        // pointer without creating an intermediate reference.
        unsafe {
            libduckdb_sys::duckdb_rs_extension_api_init(
                std::ptr::null_mut(),
                std::ptr::addr_of!(access),
                "v1",
            )
            .expect("failed to initialise DuckDB loadable-extension dispatch table");
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────

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
        // Ensure the loadable-extension dispatch table is populated from the
        // bundled DuckDB symbols before we hand off to the `duckdb` crate.
        // This is a no-op after the first call.
        init_dispatch_table_once();
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
    pub const fn conn(&self) -> &duckdb::Connection {
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
        use crate::sql_macro::SqlMacro;
        let db = InMemoryDb::open().unwrap();
        let macro_ = SqlMacro::scalar("triple", &["x"], "x * 3").unwrap();
        db.execute_batch(&macro_.to_sql()).unwrap();
        let result: i64 = db.query_one("SELECT triple(14)").unwrap();
        assert_eq!(result, 42);
    }
}
