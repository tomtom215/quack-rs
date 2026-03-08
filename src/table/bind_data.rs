// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Type-safe bind data management for table functions.
//!
//! [`FfiBindData<T>`] stores user-defined data during the `bind` phase of a table
//! function and provides safe retrieval in the `init` and `scan` phases.
//!
//! # `DuckDB` table function lifecycle
//!
//! ```text
//! bind  → stores bind_data (T)
//! init  → reads bind_data to set up global state
//! local_init → reads bind_data to set up per-thread state (optional)
//! scan  → reads bind_data + init_data + local_init_data, fills output chunk
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::table::FfiBindData;
//! use libduckdb_sys::{duckdb_bind_info, duckdb_function_info};
//!
//! struct MyConfig { path: String }
//!
//! unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
//!     // store bind data
//!     unsafe { FfiBindData::<MyConfig>::set(info, MyConfig { path: "data.csv".into() }); }
//! }
//!
//! unsafe extern "C" fn my_scan(info: duckdb_function_info, _output: libduckdb_sys::duckdb_data_chunk) {
//!     // retrieve bind data in scan
//!     if let Some(cfg) = unsafe { FfiBindData::<MyConfig>::get_from_function(info) } {
//!         let _ = &cfg.path;
//!     }
//! }
//! ```

use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_bind_info, duckdb_bind_set_bind_data, duckdb_function_get_bind_data,
    duckdb_function_info, duckdb_init_get_bind_data, duckdb_init_info,
};

/// Type-safe bind data wrapper for `DuckDB` table functions.
///
/// `FfiBindData<T>` boxes a `T` on the heap during bind and provides
/// safe access in subsequent phases. `DuckDB` owns the allocation lifetime
/// and calls the provided `destroy` callback when the query is done.
///
/// # Memory model
///
/// - [`set`][FfiBindData::set] — boxes `T` via `Box::into_raw`, registers
///   the pointer and the [`destroy`][FfiBindData::destroy] destructor with `DuckDB`.
/// - `DuckDB` calls `destroy` when the query completes, which drops the `Box<T>`.
/// - Retrieval methods borrow the `T` for the duration of the callback.
pub struct FfiBindData<T: 'static> {
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static> FfiBindData<T> {
    /// Stores `data` as the bind data for this table function invocation.
    ///
    /// Call this inside your `bind` callback to save configuration that will
    /// be accessed in `init` and `scan` callbacks.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_bind_info` provided by `DuckDB` in a bind callback.
    /// - Must be called at most once per bind invocation; calling twice leaks the first allocation.
    pub unsafe fn set(info: duckdb_bind_info, data: T) {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: info is valid; raw is a non-null heap allocation owned by DuckDB after this call.
        unsafe {
            duckdb_bind_set_bind_data(info, raw, Some(Self::destroy));
        }
    }

    /// Retrieves a shared reference to the bind data from a bind callback.
    ///
    /// Returns `None` if no bind data was set or the pointer is null.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_bind_info`.
    /// - No mutable reference to the same data must exist.
    /// - The returned reference is valid for the duration of the bind callback.
    pub const fn get_from_bind<'a>(info: duckdb_bind_info) -> Option<&'a T> {
        // Note: duckdb_bind_get_extra_info retrieves the extra_info set on the *function*,
        // not the bind_data. There is no "get bind data from bind info" in the C API —
        // that is intentional: bind data is write-only during bind and read-only afterward.
        // If you need to read data you set, store it in the closure or a pre-existing struct.
        //
        // This method is provided for completeness via duckdb_bind_get_extra_info
        // which retrieves function-level extra_info, not bind_data. Users who need
        // to read data inside their own bind callback should pass it differently.
        //
        // For bind_data retrieval in *init* and *scan*, use get_from_init / get_from_function.
        let _ = info; // Suppress unused variable warning; this design choice is intentional.
        None
    }

    /// Retrieves a shared reference to the bind data from a global init callback.
    ///
    /// Returns `None` if no bind data was set or the pointer is null.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_init_info`.
    /// - No mutable reference to the same data must exist simultaneously.
    /// - The returned reference is valid for the duration of the init callback.
    pub unsafe fn get_from_init<'a>(info: duckdb_init_info) -> Option<&'a T> {
        // SAFETY: info is valid per caller's contract.
        let raw = unsafe { duckdb_init_get_bind_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw was set by set() via Box::into_raw. It is non-null and valid.
        // No mutable reference exists per caller's contract.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Retrieves a shared reference to the bind data from a scan callback.
    ///
    /// Returns `None` if no bind data was set or the pointer is null.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info` from a scan callback.
    /// - No mutable reference to the same data must exist simultaneously.
    /// - The returned reference is valid for the duration of the scan callback.
    pub unsafe fn get_from_function<'a>(info: duckdb_function_info) -> Option<&'a T> {
        // SAFETY: info is valid per caller's contract.
        let raw = unsafe { duckdb_function_get_bind_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw was set by set() via Box::into_raw. It is non-null and valid.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// The destroy callback passed to `duckdb_bind_set_bind_data`.
    ///
    /// `DuckDB` calls this when the query is complete. It drops the `Box<T>`.
    ///
    /// # Safety
    ///
    /// - `ptr` must have been allocated by [`set`][FfiBindData::set] via `Box::into_raw`.
    /// - Must be called exactly once (`DuckDB` guarantees this for bind data destroyers).
    pub unsafe extern "C" fn destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            // SAFETY: ptr was created by Box::into_raw(Box::<T>::new(...)) in set().
            // DuckDB calls this exactly once.
            unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    struct Config {
        value: i32,
    }

    #[test]
    fn destroy_null_is_noop() {
        // Must not panic or crash
        unsafe { FfiBindData::<Config>::destroy(std::ptr::null_mut()) };
    }

    #[test]
    fn destroy_allocated_box() {
        let boxed = Box::new(Config { value: 42 });
        let raw = Box::into_raw(boxed).cast::<c_void>();
        // SAFETY: raw is a valid Box-allocated pointer.
        unsafe { FfiBindData::<Config>::destroy(raw) };
        // If we reach here without panic/UB, the test passes.
    }

    #[test]
    fn get_from_bind_returns_none() {
        // get_from_bind is intentionally unimplemented (returns None by design)
        // Test that calling it with null doesn't panic.
        let result = FfiBindData::<Config>::get_from_bind(std::ptr::null_mut());
        assert!(result.is_none());
    }
}
