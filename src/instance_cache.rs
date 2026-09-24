// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Database instance cache (`DuckDB` 1.5.0+).
//!
//! An [`InstanceCache`] lets multiple connections share a single underlying
//! `DuckDB` instance for a given database path. Opening the same path twice
//! through the cache returns handles backed by the *same* instance, which avoids
//! the "database is already open in another process/instance" conflict and saves
//! the cost of re-initialising the database.
//!
//! This is primarily useful for extensions or host integrations that open
//! secondary databases on behalf of a query.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::instance_cache::InstanceCache;
//!
//! # fn demo() -> Result<(), quack_rs::error::ExtensionError> {
//! let cache = InstanceCache::new();
//! // Returns a duckdb_database the caller owns and must close with duckdb_close.
//! let db = cache.get_or_create(c"my.db", None)?;
//! # let _ = db;
//! # Ok(())
//! # }
//! ```

use std::ffi::CStr;
use std::os::raw::c_char;

use libduckdb_sys::{
    duckdb_config, duckdb_create_instance_cache, duckdb_database, duckdb_destroy_instance_cache,
    duckdb_free, duckdb_get_or_create_from_cache, duckdb_instance_cache, DuckDBSuccess,
};

use crate::config::DbConfig;
use crate::error::ExtensionError;

/// RAII wrapper for a `duckdb_instance_cache`.
///
/// Automatically destroyed when dropped. A database obtained from the cache
/// holds its own reference to the instance, so it stays valid after the cache
/// is dropped, until it is closed.
///
/// # Threads
///
/// `InstanceCache` is `Send + Sync`: one cache can be shared by threads that
/// open databases through it concurrently, which is what it is for.
///
/// One ordering hazard is `DuckDB`'s, not a data race: while the **last**
/// handle to a cached file database is being closed, a concurrent
/// [`get_or_create`][Self::get_or_create] for the same path can fail with
/// "Unique file handle conflict ... is already attached" — the closing
/// instance still holds the file. Keep one handle open for as long as other
/// threads may open the path, or retry.
pub struct InstanceCache {
    cache: duckdb_instance_cache,
}

// SAFETY: the handle owns a heap-allocated `DBInstanceCacheWrapper` holding a
// `duckdb::DBInstanceCache` (src/main/capi/duckdb-c.cpp). That object has no
// thread affinity, so it may be dropped on any thread (`Send`). Its only
// shared-state operation reachable through `&self` is
// `duckdb_get_or_create_from_cache` → `DBInstanceCache::GetOrCreateInstance`,
// which takes the cache's `mutex cache_lock` before it reads or writes the
// `db_instances` map, and serialises creation of one database with that
// entry's `update_database_mutex` (src/main/db_instance_cache.cpp,
// src/include/duckdb/main/db_instance_cache.hpp). Concurrent calls through
// shared references are therefore data-race free (`Sync`). The one argument it
// mutates besides its own state is the `DBConfig` passed in; `DbConfig` is
// neither `Send` nor `Sync`, so two threads cannot pass the same one.
unsafe impl Send for InstanceCache {}
// SAFETY: see the `Send` impl above.
unsafe impl Sync for InstanceCache {}

impl InstanceCache {
    /// Creates a new, empty instance cache.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: duckdb_create_instance_cache allocates an owned handle.
        let cache = unsafe { duckdb_create_instance_cache() };
        Self { cache }
    }

    /// Opens `path` through the cache, creating the instance if it does not yet
    /// exist or returning a handle to the cached one if it does.
    ///
    /// Pass `config` to control how a freshly-created instance is configured.
    /// When an instance already exists for `path`, `config` must match the one
    /// it was created with: a different configuration is an error ("Can't open
    /// a connection to same database file with a different configuration than
    /// existing connections", `DBInstanceCache::GetInstanceInternal`), not
    /// silently ignored. `None` means `DuckDB`'s defaults, so it too conflicts
    /// with an instance created with a non-default `config`.
    ///
    /// An empty `path` (or `:memory:`) is never cached: each call creates a new,
    /// separate in-memory database.
    ///
    /// The returned `duckdb_database` is owned by the caller and **must** be
    /// closed with `duckdb_close` when no longer needed.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] carrying `DuckDB`'s message if the instance
    /// cannot be opened or created, or if `config` conflicts with the cached
    /// instance's.
    pub fn get_or_create(
        &self,
        path: &CStr,
        config: Option<&DbConfig>,
    ) -> Result<duckdb_database, ExtensionError> {
        let mut out_db: duckdb_database = std::ptr::null_mut();
        let mut out_err: *mut c_char = std::ptr::null_mut();
        let cfg: duckdb_config = config.map_or(std::ptr::null_mut(), DbConfig::as_raw);
        // SAFETY: self.cache and path are valid; out_db and out_err are valid
        // out-pointers; cfg is either null or a valid duckdb_config.
        let state = unsafe {
            duckdb_get_or_create_from_cache(
                self.cache,
                path.as_ptr(),
                &raw mut out_db,
                cfg,
                &raw mut out_err,
            )
        };
        if state == DuckDBSuccess && !out_db.is_null() {
            return Ok(out_db);
        }
        let message = if out_err.is_null() {
            "failed to open database from instance cache".to_owned()
        } else {
            // SAFETY: out_err is a valid null-terminated string allocated by DuckDB.
            let msg = unsafe { CStr::from_ptr(out_err) }
                .to_str()
                .unwrap_or("failed to open database from instance cache")
                .to_owned();
            // SAFETY: out_err was allocated by DuckDB and must be freed.
            unsafe { duckdb_free(out_err.cast()) };
            msg
        };
        Err(ExtensionError::new(message))
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_instance_cache {
        self.cache
    }
}

impl Default for InstanceCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for InstanceCache {
    fn drop(&mut self) {
        if !self.cache.is_null() {
            // SAFETY: self.cache is a valid handle that we own.
            unsafe { duckdb_destroy_instance_cache(&raw mut self.cache) };
        }
    }
}

#[cfg(all(test, feature = "_duckdb-testing"))]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_via_cache() {
        // Ensure the dispatch table is populated.
        let _db = crate::testing::InMemoryDb::open().unwrap();

        let cache = InstanceCache::new();
        // Empty path opens an in-memory database.
        let result = cache.get_or_create(c"", None);
        assert!(result.is_ok(), "get_or_create failed: {:?}", result.err());
        let mut db = result.unwrap();
        // SAFETY: db is a valid duckdb_database returned from the cache.
        unsafe { libduckdb_sys::duckdb_close(&raw mut db) };
    }

    /// Scratch database path unique to this test process.
    fn scratch_db(tag: &str) -> std::ffi::CString {
        let path = std::env::temp_dir().join(format!(
            "quack_rs_instance_cache_{tag}_{}.duckdb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::ffi::CString::new(path.to_str().expect("utf-8 temp dir")).expect("no NUL")
    }

    /// TBL-13: the doc said a different config for an existing instance is
    /// ignored; `DuckDB` rejects it.
    #[test]
    fn a_different_config_for_a_cached_instance_is_an_error() {
        let _dispatch = crate::testing::InMemoryDb::open().unwrap();
        let cache = InstanceCache::new();
        let path = scratch_db("config");
        let mut first = cache.get_or_create(&path, None).expect("first open");
        let config = DbConfig::new().unwrap().set("threads", "1").unwrap();
        let err = cache
            .get_or_create(&path, Some(&config))
            .expect_err("a conflicting config must be refused");
        assert!(err.as_str().contains("different configuration"), "{err}");
        // SAFETY: `first` came from the cache and is closed once.
        unsafe { libduckdb_sys::duckdb_close(&raw mut first) };
        let _ = std::fs::remove_file(path.to_str().unwrap_or_default());
    }

    /// TBL-25: a cache can be shared across threads.
    #[test]
    fn one_cache_serves_concurrent_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InstanceCache>();

        let _dispatch = crate::testing::InMemoryDb::open().unwrap();
        let cache = InstanceCache::new();
        let path = scratch_db("threads");
        // Every thread opens, then waits until all have opened before any
        // closes: closing the last handle while another thread opens the
        // same path is a DuckDB-level conflict (see "Threads").
        let opened = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let mut db = cache
                        .get_or_create(&path, None)
                        .expect("open from a thread");
                    opened.wait();
                    // SAFETY: `db` came from the cache and is closed once.
                    unsafe { libduckdb_sys::duckdb_close(&raw mut db) };
                });
            }
        });
        let _ = std::fs::remove_file(path.to_str().unwrap_or_default());
    }
}

crate::debug_repr::impl_handle_debug!(InstanceCache.cache);
