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
/// Automatically destroyed when dropped. Databases obtained from the cache
/// remain valid until they are individually closed and the cache is dropped.
pub struct InstanceCache {
    cache: duckdb_instance_cache,
}

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
    /// Pass `config` to control how a freshly-created instance is configured; it
    /// is ignored when an instance already exists for `path`.
    ///
    /// The returned `duckdb_database` is owned by the caller and **must** be
    /// closed with `duckdb_close` when no longer needed.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] carrying `DuckDB`'s message if the instance
    /// cannot be opened or created.
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
}
