// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Builder for registering `DuckDB` replacement scans.
//!
//! Replacement scans enable the `SELECT * FROM 'my_file.xyz'` pattern.
//! When DuckDB encounters an unrecognized table reference (including a string
//! literal like `'data.parquet'`), it calls all registered replacement scan
//! callbacks in registration order, giving each a chance to redirect the scan
//! to a known table function.
//!
//! # How replacement scans work
//!
//! ```text
//! SELECT * FROM 'data.myformat'
//!   ↓
//! DuckDB does not know 'data.myformat' as a table
//!   ↓
//! Calls each registered replacement scan callback with:
//!   - info: duckdb_replacement_scan_info   (use to redirect or error)
//!   - table_name: *const c_char            (the unrecognized identifier, e.g. "data.myformat")
//!   - data: *mut c_void                    (your extra_data, if any)
//!   ↓
//! Callback either:
//!   - Redirects: duckdb_replacement_scan_set_function_name + add_parameter
//!   - Ignores: returns without calling any scan API (DuckDB tries the next callback)
//!   - Errors: duckdb_replacement_scan_set_error
//! ```
//!
//! # Example: Route all `.xyz` files to a custom table function
//!
//! ```rust,no_run
//! use quack_rs::replacement_scan::{ReplacementScanBuilder, ReplacementScanInfo};
//! use libduckdb_sys::duckdb_replacement_scan_info;
//! use std::os::raw::c_char;
//!
//! unsafe extern "C" fn my_scan(
//!     info: duckdb_replacement_scan_info,
//!     table_name: *const c_char,
//!     _data: *mut std::os::raw::c_void,
//! ) {
//!     let name = unsafe { std::ffi::CStr::from_ptr(table_name).to_string_lossy() };
//!     if !name.ends_with(".xyz") {
//!         return; // not ours — let DuckDB try other callbacks
//!     }
//!     // Redirect to our table function and pass the filename as a parameter.
//!     unsafe {
//!         ReplacementScanInfo::new(info)
//!             .set_function("read_xyz")
//!             .add_varchar_parameter(name.as_ref());
//!     }
//! }
//!
//! // fn register(db: libduckdb_sys::duckdb_database) {
//! //     unsafe { ReplacementScanBuilder::register(db, my_scan, std::ptr::null_mut(), None); }
//! // }
//! ```

use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use libduckdb_sys::{
    duckdb_add_replacement_scan, duckdb_create_varchar_length, duckdb_database,
    duckdb_delete_callback_t, duckdb_destroy_value,
    duckdb_replacement_scan_add_parameter, duckdb_replacement_scan_info,
    duckdb_replacement_scan_set_error, duckdb_replacement_scan_set_function_name,
};

/// The replacement scan callback signature.
///
/// - `info` — use to redirect the scan or report an error.
/// - `table_name` — the unrecognized table identifier from the query.
/// - `data` — your extra_data pointer, if any.
pub type ReplacementScanFn = unsafe extern "C" fn(
    info: duckdb_replacement_scan_info,
    table_name: *const c_char,
    data: *mut c_void,
);

/// Ergonomic wrapper around `duckdb_replacement_scan_info`.
///
/// Provides safe methods for the common replacement scan operations.
pub struct ReplacementScanInfo {
    info: duckdb_replacement_scan_info,
}

impl ReplacementScanInfo {
    /// Wraps a raw `duckdb_replacement_scan_info`.
    ///
    /// # Safety
    ///
    /// `info` must be a valid `duckdb_replacement_scan_info` provided by DuckDB.
    #[inline]
    #[must_use]
    pub const unsafe fn new(info: duckdb_replacement_scan_info) -> Self {
        Self { info }
    }

    /// Redirects this scan to the named table function.
    ///
    /// Call this first to name the table function that will handle the scan,
    /// then use `add_varchar_parameter` or `add_parameter_raw` to pass arguments.
    ///
    /// # Panics
    ///
    /// Panics if `function_name` contains an interior null byte.
    pub fn set_function(&self, function_name: &str) -> &Self {
        let c_name =
            CString::new(function_name).expect("function name must not contain null bytes");
        // SAFETY: self.info is valid per constructor's contract.
        unsafe {
            duckdb_replacement_scan_set_function_name(self.info, c_name.as_ptr());
        }
        self
    }

    /// Adds a VARCHAR string parameter to the redirected function call.
    ///
    /// This is the most common use case: passing the original file path/name
    /// to the underlying table function.
    pub fn add_varchar_parameter(&self, value: &str) -> &Self {
        // SAFETY: creates a DuckDB VARCHAR value.
        let duckdb_val = unsafe {
            duckdb_create_varchar_length(value.as_ptr().cast::<i8>(), value.len() as u64)
        };
        // SAFETY: self.info is valid; duckdb_val is a valid newly created value.
        unsafe {
            duckdb_replacement_scan_add_parameter(self.info, duckdb_val);
        }
        // SAFETY: duckdb_val was just created; must be destroyed after passing.
        unsafe {
            duckdb_destroy_value(&mut { duckdb_val });
        }
        self
    }

    /// Reports an error, causing DuckDB to abort this replacement scan attempt.
    ///
    /// # Panics
    ///
    /// Panics if `message` contains an interior null byte.
    pub fn set_error(&self, message: &str) {
        let c_msg = CString::new(message).expect("error message must not contain null bytes");
        // SAFETY: self.info is valid.
        unsafe {
            duckdb_replacement_scan_set_error(self.info, c_msg.as_ptr());
        }
    }

    /// Returns the raw `duckdb_replacement_scan_info` handle.
    #[must_use]
    #[inline]
    pub const fn as_raw(&self) -> duckdb_replacement_scan_info {
        self.info
    }
}

/// Builder / registration helper for DuckDB replacement scans.
///
/// Unlike other builders in quack-rs, registration is a single static call
/// because the replacement scan API takes a raw function pointer and optional
/// extra data directly — there is no handle to configure step-by-step.
pub struct ReplacementScanBuilder;

impl ReplacementScanBuilder {
    /// Registers a replacement scan callback on the given database.
    ///
    /// # Arguments
    ///
    /// - `db` — a valid `duckdb_database`. Obtain this via
    ///   `access.get_database(info)` in the extension entry point.
    /// - `callback` — your replacement scan function.
    /// - `extra_data` — user data passed as the third argument to `callback`.
    ///   Pass `std::ptr::null_mut()` if you need no extra data.
    /// - `delete_callback` — called by DuckDB when the replacement scan is
    ///   removed (e.g., database closed). Pass `None` if `extra_data` needs
    ///   no special cleanup.
    ///
    /// # Safety
    ///
    /// - `db` must be a valid, open `duckdb_database`.
    /// - `extra_data` must remain valid until `delete_callback` is called
    ///   (or until the database is closed if `delete_callback` is `None`).
    pub unsafe fn register(
        db: duckdb_database,
        callback: ReplacementScanFn,
        extra_data: *mut c_void,
        delete_callback: duckdb_delete_callback_t,
    ) {
        // SAFETY: db is valid per caller; callback is a valid extern "C" fn.
        unsafe {
            duckdb_add_replacement_scan(db, Some(callback), extra_data, delete_callback);
        }
    }

    /// Registers a replacement scan with owned extra data.
    ///
    /// Boxes `data` and registers a drop destructor automatically.
    /// This is the safe, ergonomic way to attach Rust data to a replacement scan.
    ///
    /// # Safety
    ///
    /// `db` must be a valid, open `duckdb_database`.
    pub unsafe fn register_with_data<T: 'static>(
        db: duckdb_database,
        callback: ReplacementScanFn,
        data: T,
    ) {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();

        unsafe extern "C" fn drop_box<T>(ptr: *mut c_void) {
            if !ptr.is_null() {
                unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
            }
        }

        // SAFETY: db is valid; raw is a heap allocation; drop_box is a valid destructor.
        unsafe {
            duckdb_add_replacement_scan(db, Some(callback), raw, Some(drop_box::<T>));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_scan_info_wraps_null() {
        // Constructing with null should not crash in itself (no DuckDB calls made).
        let _info = unsafe { ReplacementScanInfo::new(std::ptr::null_mut()) };
    }
}
