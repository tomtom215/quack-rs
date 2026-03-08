// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! RAII wrapper for `DuckDB` database configuration.
//!
//! [`DbConfig`] wraps `duckdb_config` and provides a builder-style API for
//! setting configuration options before opening a `DuckDB` database.
//!
//! Extension authors typically receive an already-opened connection and do not
//! need to open databases themselves.  `DbConfig` is useful when an extension
//! needs to open a **secondary** `DuckDB` database from within its callbacks —
//! for example, a virtual table that reads from another `.duckdb` file.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::config::DbConfig;
//!
//! // fn open_secondary() -> Result<(), quack_rs::error::ExtensionError> {
//! //     let config = DbConfig::new()?
//! //         .set("access_mode", "READ_ONLY")?
//! //         .set("threads", "4")?;
//! //     // Pass config.as_raw() to duckdb_open_ext(...)
//! //     Ok(())
//! // }
//! ```
//!
//! # Available configuration flags
//!
//! Use [`DbConfig::flag_count`] and [`DbConfig::get_flag`] to enumerate all
//! supported option names and their descriptions at runtime.

use std::ffi::{CStr, CString};

use libduckdb_sys::{
    duckdb_config, duckdb_config_count, duckdb_create_config, duckdb_destroy_config,
    duckdb_get_config_flag, duckdb_set_config, DuckDBSuccess,
};

use crate::error::ExtensionError;

/// RAII wrapper for a `duckdb_config` handle.
///
/// Configuration options are set via [`set`][DbConfig::set] and consumed by
/// passing [`as_raw`][DbConfig::as_raw] to `duckdb_open_ext`.
/// The handle is destroyed automatically when [`DbConfig`] is dropped.
#[must_use]
pub struct DbConfig {
    config: duckdb_config,
}

impl DbConfig {
    /// Creates a new, empty `DuckDB` configuration object.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if `DuckDB` fails to allocate the config
    /// (which is extremely rare in practice).
    pub fn new() -> Result<Self, ExtensionError> {
        let mut config: duckdb_config = std::ptr::null_mut();
        // SAFETY: out_config is a valid pointer to a null duckdb_config.
        let state = unsafe { duckdb_create_config(&raw mut config) };
        if state == DuckDBSuccess {
            Ok(Self { config })
        } else {
            Err(ExtensionError::new("duckdb_create_config failed"))
        }
    }

    /// Sets a single configuration option.
    ///
    /// Common options include `"access_mode"` (`"READ_ONLY"` / `"READ_WRITE"`),
    /// `"threads"` (number of CPU threads), and `"memory_limit"` (e.g. `"1GB"`).
    /// Use [`DbConfig::flag_count`] / [`DbConfig::get_flag`] to enumerate all
    /// available options at runtime.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the option name or value is not recognised
    /// by `DuckDB`.
    ///
    /// # Panics
    ///
    /// Panics if `name` or `value` contain interior null bytes.
    pub fn set(self, name: &str, value: &str) -> Result<Self, ExtensionError> {
        let c_name = CString::new(name).expect("config name must not contain null bytes");
        let c_value = CString::new(value).expect("config value must not contain null bytes");
        // SAFETY: self.config is a valid handle; c_name and c_value are NUL-terminated.
        let state = unsafe { duckdb_set_config(self.config, c_name.as_ptr(), c_value.as_ptr()) };
        if state == DuckDBSuccess {
            Ok(self)
        } else {
            Err(ExtensionError::new(format!(
                "duckdb_set_config failed for option '{name}' = '{value}'"
            )))
        }
    }

    /// Returns the total number of available configuration flags.
    ///
    /// Use this together with [`get_flag`][DbConfig::get_flag] to enumerate all
    /// configuration options that `DuckDB` accepts.
    #[must_use]
    pub fn flag_count() -> usize {
        // SAFETY: pure read of a DuckDB global table; no state required.
        unsafe { duckdb_config_count() }
    }

    /// Returns the name and description for the configuration flag at `index`.
    ///
    /// `index` must be less than [`flag_count()`][DbConfig::flag_count].
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if `index` is out of range or `DuckDB` fails
    /// to retrieve the flag information.
    pub fn get_flag(index: usize) -> Result<(String, String), ExtensionError> {
        let mut name_ptr: *const std::os::raw::c_char = std::ptr::null();
        let mut desc_ptr: *const std::os::raw::c_char = std::ptr::null();

        // SAFETY: out-pointers are valid stack locations; DuckDB sets them to
        // pointers into its own static tables (no allocation, no free needed).
        let state = unsafe { duckdb_get_config_flag(index, &raw mut name_ptr, &raw mut desc_ptr) };

        if state != DuckDBSuccess {
            return Err(ExtensionError::new(format!(
                "duckdb_get_config_flag({index}) failed"
            )));
        }

        // SAFETY: DuckDB sets these pointers to valid NUL-terminated strings when
        // the call succeeds.
        let name = unsafe { CStr::from_ptr(name_ptr) }
            .to_string_lossy()
            .into_owned();
        let desc = unsafe { CStr::from_ptr(desc_ptr) }
            .to_string_lossy()
            .into_owned();

        Ok((name, desc))
    }

    /// Returns the underlying `duckdb_config` handle.
    ///
    /// Pass this to `duckdb_open_ext` to open a database with these settings.
    ///
    /// The handle remains owned by `DbConfig`; do **not** call
    /// `duckdb_destroy_config` on the returned value.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_config {
        self.config
    }
}

impl Drop for DbConfig {
    fn drop(&mut self) {
        if !self.config.is_null() {
            // SAFETY: self.config is a valid handle allocated by duckdb_create_config.
            unsafe {
                duckdb_destroy_config(&raw mut self.config);
            }
        }
    }
}

// Note: DbConfig calls real DuckDB C API functions (duckdb_create_config,
// duckdb_config_count, etc.) which are only available once DuckDB has
// initialized the loadable-extension function pointers.  Unit tests in this
// crate run without a live DuckDB process so no tests can be written here
// that actually call DuckDB.  Live tests are exercised via examples/hello-ext.
