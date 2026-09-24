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
//! # Testing the FFI wrappers
//!
//! Opening an `InMemoryDb` also populates the `loadable-extension` dispatch
//! table, and it stays populated for the rest of the process. After that, every
//! C API call works — so quack-rs's own wrappers (`VectorReader`,
//! `VectorWriter`, `BindInfo`, `Connection::register_*`) can be exercised
//! against a real `DuckDB` inside `cargo test`:
//!
//! 1. `InMemoryDb::open()` to initialise the dispatch table.
//! 2. `duckdb_open` / `duckdb_connect` for a raw connection.
//! 3. Register your function with the usual builder.
//! 4. Run SQL through [`query`][crate::query::query] and assert on the result.
//!
//! `tests/ffi_roundtrip.rs` in this repository does exactly that for every
//! vector type. [`MockVectorReader`] / [`MockVectorWriter`] remain the right
//! tool for unit-testing callback logic without a database, and are the only
//! option when the `bundled-test` features are off.
//!
//! [`MockVectorReader`]: crate::testing::MockVectorReader
//! [`MockVectorWriter`]: crate::testing::MockVectorWriter
//!
//! # Enabling this feature
//!
//! Compile `DuckDB` from source (zero-config, ~5-10 min cold):
//!
//! ```toml
//! # In your extension's Cargo.toml:
//! [dev-dependencies]
//! quack-rs = { version = "0.18", features = ["bundled-test"] }
//! ```
//!
//! …or link a pre-built libduckdb for a much faster build:
//!
//! ```toml
//! [dev-dependencies]
//! quack-rs = { version = "0.18", features = ["bundled-test-prebuilt"] }
//! ```
//!
//! With `bundled-test-prebuilt`, build with `DUCKDB_DOWNLOAD_LIB=1` (let
//! libduckdb-sys download the prebuilt zip) or set `DUCKDB_LIB_DIR=...` to point
//! at a libduckdb tree you already have extracted locally.
//!
//! # Example
//!
//! ```rust,no_run
//! # #[cfg(feature = "_duckdb-testing")]
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
// When `bundled-test` / `bundled-test-prebuilt` is active, Cargo's
// feature-unification merges the `loadable-extension` feature (required by the
// library to build as a DuckDB extension) and the `duckdb`-provided
// `libduckdb-sys` build (compiled from source for `bundled-test`, or linked
// against a pre-built library for `bundled-test-prebuilt`) into a single
// `libduckdb-sys` build.
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

    /// `sizeof(duckdb_ext_api_v1)` in the C++ headers the shim was compiled
    /// against.
    fn quack_rs_api_v1_size() -> usize;

    /// `DUCKDB_VERSION` from those headers, or `"unknown"`. A static string.
    fn quack_rs_header_duckdb_version() -> *const std::os::raw::c_char;
}

/// Checks that the C++ headers compiled into the shim and the `libduckdb-sys`
/// bindings agree on the size of `duckdb_ext_api_v1`.
///
/// `quack_rs_create_api_v1()` returns the struct by value into a buffer sized
/// by the Rust bindings. With `bundled-test-prebuilt`, the headers come from
/// whatever `DUCKDB_LIB_DIR` (or the `DUCKDB_DOWNLOAD_LIB` download) supplies,
/// which nothing ties to the `libduckdb-sys` version Cargo resolved: larger
/// headers overrun the buffer, smaller ones leave the last slots as stack
/// garbage and shift nothing back into place. Either must stop the test run.
fn check_api_struct_size(
    header_bytes: usize,
    bindings_bytes: usize,
    header_version: &str,
) -> Result<(), String> {
    if header_bytes == bindings_bytes {
        return Ok(());
    }
    let slot = core::mem::size_of::<*const core::ffi::c_void>();
    Err(format!(
        "quack-rs testing: the DuckDB headers compiled into the test shim (DuckDB \
         {header_version}: duckdb_ext_api_v1 is {} slots) do not match the libduckdb-sys \
         bindings Cargo resolved ({} slots). With `bundled-test-prebuilt`, DUCKDB_LIB_DIR (or \
         the DUCKDB_DOWNLOAD_LIB download) must be the DuckDB release libduckdb-sys was \
         generated for: run `cargo tree -i libduckdb-sys` (1.10505.x is DuckDB v1.5.5, 1.4.4 is \
         v1.4.4) and point DUCKDB_LIB_DIR at that release, or pin libduckdb-sys to DuckDB \
         {header_version}. Refusing to initialise the dispatch table: the struct is returned \
         by value, so a size mismatch would corrupt memory.",
        header_bytes / slot,
        bindings_bytes / slot,
    ))
}

/// Populates the `loadable-extension` dispatch table exactly once.
///
/// Uses `std::sync::Once` so it is safe to call from multiple threads and
/// from multiple test cases; subsequent calls are no-ops.
fn init_dispatch_table_once() {
    // Checked before anything else, and remembered, so that every test in the
    // run fails with the diagnostic rather than only the first one (a panic
    // inside `INIT` would leave the rest with "Once instance has previously
    // been poisoned").
    static SIZE_CHECK: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    static INIT: std::sync::Once = std::sync::Once::new();
    let size_check = SIZE_CHECK.get_or_init(|| {
        // SAFETY: both are plain C++ functions with no preconditions; the
        // version string is a static literal.
        let (header_bytes, header_version) = unsafe {
            (
                quack_rs_api_v1_size(),
                std::ffi::CStr::from_ptr(quack_rs_header_duckdb_version()).to_string_lossy(),
            )
        };
        check_api_struct_size(
            header_bytes,
            core::mem::size_of::<libduckdb_sys::duckdb_ext_api_v1>(),
            &header_version,
        )
    });
    if let Err(message) = size_check {
        panic!("{message}");
    }

    INIT.call_once(|| {
        // SAFETY: quack_rs_create_api_v1 is a thin C++ wrapper around
        // DuckDB's own CreateAPIv1().  It sets every field of the returned
        // struct to the matching bundled DuckDB C function pointer, so the
        // values are valid function pointers for the lifetime of the process.
        let api = unsafe { quack_rs_create_api_v1() };

        // The struct stays on this stack frame. `duckdb_rs_extension_api_init`
        // reads it through the pointer and copies each function pointer into
        // its own `AtomicPtr`, keeping no reference afterwards (verified in
        // `libduckdb-sys`'s generated `bindgen_bundled_version_loadable.rs`),
        // so it only has to outlive that one call.
        //
        // This used to be `Box::into_raw`, deliberately leaked. It worked, but
        // it was the only allocation the `leak-check` CI job had to suppress —
        // and a leak-detector job that starts with a suppression for your own
        // code is one bad day away from hiding a real missing destructor.
        let api_ptr: *const libduckdb_sys::duckdb_ext_api_v1 = &raw const api;

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

        // `api` dies with this frame, so the thread-local must not keep
        // pointing at it.
        TL_API_PTR.with(|cell| cell.set(std::ptr::null()));
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

impl core::fmt::Debug for InMemoryDb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The wrapped `duckdb::Connection` has nothing worth printing, and the
        // point of this impl is that a test fixture holding an `InMemoryDb` can
        // still `#[derive(Debug)]`.
        f.debug_struct("InMemoryDb").finish_non_exhaustive()
    }
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

    /// Opens a new in-memory `DuckDB` database with `allow_unsigned_extensions`
    /// enabled, so unsigned `.duckdb_extension` artifacts can be `LOAD`ed for
    /// integration testing.
    ///
    /// `allow_unsigned_extensions` is a startup-only config option; it cannot
    /// be toggled via `SET` after the database has opened. Use this constructor
    /// when the test needs to `LOAD '/path/to/<your>.duckdb_extension'` against
    /// an artifact you built locally (and therefore haven't signed).
    ///
    /// # Errors
    ///
    /// Returns an error if the config can't be constructed or if `DuckDB` fails
    /// to initialize an in-memory database.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[cfg(feature = "_duckdb-testing")]
    /// # {
    /// use quack_rs::testing::InMemoryDb;
    ///
    /// let db = InMemoryDb::open_unsigned().unwrap();
    /// db.execute_batch("LOAD '/path/to/my_ext.duckdb_extension'").unwrap();
    /// # }
    /// ```
    pub fn open_unsigned() -> Result<Self, duckdb::Error> {
        init_dispatch_table_once();
        let config = duckdb::Config::default().allow_unsigned_extensions()?;
        Ok(Self {
            conn: duckdb::Connection::open_in_memory_with_flags(config)?,
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
    /// # #[cfg(feature = "_duckdb-testing")]
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
    fn in_memory_db_open_unsigned_enables_allow_unsigned_extensions() {
        let db = InMemoryDb::open_unsigned().expect("should open in-memory db");
        let v: bool = db
            .query_one("SELECT current_setting('allow_unsigned_extensions')")
            .expect("should read config");
        assert!(v);
    }

    #[test]
    fn in_memory_db_open_does_not_enable_allow_unsigned_extensions() {
        let db = InMemoryDb::open().expect("should open in-memory db");
        let v: bool = db
            .query_one("SELECT current_setting('allow_unsigned_extensions')")
            .expect("should read config");
        assert!(!v);
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

    #[test]
    fn the_shim_headers_match_the_bindings() {
        // SAFETY: a plain C++ function with no preconditions.
        let header_bytes = unsafe { quack_rs_api_v1_size() };
        assert_eq!(
            header_bytes,
            core::mem::size_of::<libduckdb_sys::duckdb_ext_api_v1>()
        );
    }

    #[test]
    fn a_struct_size_mismatch_is_refused_with_both_versions_named() {
        let slot = core::mem::size_of::<*const core::ffi::c_void>();
        assert!(check_api_struct_size(546 * slot, 546 * slot, "v1.5.5").is_ok());
        for (header, bindings) in [(545, 546), (546, 545), (459, 546)] {
            let msg = check_api_struct_size(header * slot, bindings * slot, "v1.5.0")
                .expect_err("a mismatch must be refused");
            assert!(msg.contains(&format!("{header} slots")), "{msg}");
            assert!(msg.contains(&format!("({bindings} slots)")), "{msg}");
            assert!(msg.contains("DuckDB v1.5.0"), "{msg}");
            assert!(msg.contains("DUCKDB_LIB_DIR"), "{msg}");
            assert!(msg.contains("cargo tree -i libduckdb-sys"), "{msg}");
        }
    }
}
