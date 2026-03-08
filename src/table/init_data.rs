// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Type-safe init data management for table functions.
//!
//! `DuckDB` table functions have two init phases:
//!
//! - **Global init** (`init`): Called once per query. Use [`FfiInitData`] to store
//!   global scan state (e.g., a file handle, row counter shared across threads).
//! - **Local init** (`local_init`): Called once per thread. Use [`FfiLocalInitData`]
//!   to store per-thread scan state (e.g., a thread-local buffer or offset).
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::table::{FfiInitData, FfiLocalInitData};
//! use libduckdb_sys::{duckdb_init_info, duckdb_function_info};
//!
//! struct GlobalState { rows_remaining: u64 }
//! struct LocalState  { thread_offset: u64 }
//!
//! unsafe extern "C" fn my_init(info: duckdb_init_info) {
//!     unsafe { FfiInitData::<GlobalState>::set(info, GlobalState { rows_remaining: 1000 }); }
//! }
//!
//! unsafe extern "C" fn my_local_init(info: duckdb_init_info) {
//!     unsafe { FfiLocalInitData::<LocalState>::set(info, LocalState { thread_offset: 0 }); }
//! }
//!
//! unsafe extern "C" fn my_scan(info: duckdb_function_info, _output: libduckdb_sys::duckdb_data_chunk) {
//!     let _global = unsafe { FfiInitData::<GlobalState>::get(info) };
//!     let _local  = unsafe { FfiLocalInitData::<LocalState>::get(info) };
//! }
//! ```

use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_function_get_init_data, duckdb_function_get_local_init_data, duckdb_function_info,
    duckdb_init_info, duckdb_init_set_init_data,
};

/// Type-safe global init data for `DuckDB` table functions.
///
/// Set in the global `init` callback; retrieved in `scan`.
pub struct FfiInitData<T: 'static> {
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static> FfiInitData<T> {
    /// Stores `data` as the global init data for this query.
    ///
    /// Call inside your global `init` callback.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_init_info`.
    /// - Must be called at most once per init invocation.
    pub unsafe fn set(info: duckdb_init_info, data: T) {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: info is valid; raw is a heap allocation; destroy is a valid fn pointer.
        unsafe {
            duckdb_init_set_init_data(info, raw, Some(Self::destroy));
        }
    }

    /// Retrieves a shared reference to the global init data from a scan callback.
    ///
    /// Returns `None` if no init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info` from a scan callback.
    /// - No mutable reference to the same data must exist simultaneously.
    pub unsafe fn get<'a>(info: duckdb_function_info) -> Option<&'a T> {
        // SAFETY: info is valid per caller's contract.
        let raw = unsafe { duckdb_function_get_init_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw was created by set() via Box::into_raw.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Retrieves a mutable reference to the global init data from a scan callback.
    ///
    /// Returns `None` if no init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info` from a scan callback.
    /// - No other reference to the same data must exist simultaneously.
    pub unsafe fn get_mut<'a>(info: duckdb_function_info) -> Option<&'a mut T> {
        let raw = unsafe { duckdb_function_get_init_data(info) };
        if raw.is_null() {
            return None;
        }
        Some(unsafe { &mut *raw.cast::<T>() })
    }

    /// Destroy callback: drops the `Box<T>`.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by [`set`][FfiInitData::set].
    pub unsafe extern "C" fn destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
        }
    }
}

/// Type-safe per-thread local init data for `DuckDB` table functions.
///
/// Set in the `local_init` callback; retrieved in `scan`.
pub struct FfiLocalInitData<T: 'static> {
    _marker: std::marker::PhantomData<T>,
}

impl<T: 'static> FfiLocalInitData<T> {
    /// Stores `data` as the per-thread local init data.
    ///
    /// Call inside your `local_init` callback.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_init_info`.
    /// - Must be called at most once per `local_init` invocation.
    pub unsafe fn set(info: duckdb_init_info, data: T) {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: info is valid; raw is non-null. The same duckdb_init_set_init_data
        // function is used for both global and local init; DuckDB tracks which
        // phase is active when the callback is invoked.
        unsafe {
            duckdb_init_set_init_data(info, raw, Some(Self::destroy));
        }
    }

    /// Retrieves a shared reference to the per-thread local init data.
    ///
    /// Returns `None` if no local init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info`.
    /// - No mutable reference to the same data must exist simultaneously.
    pub unsafe fn get<'a>(info: duckdb_function_info) -> Option<&'a T> {
        let raw = unsafe { duckdb_function_get_local_init_data(info) };
        if raw.is_null() {
            return None;
        }
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Retrieves a mutable reference to the per-thread local init data.
    ///
    /// Returns `None` if no local init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info`.
    /// - No other reference to the same data must exist simultaneously.
    pub unsafe fn get_mut<'a>(info: duckdb_function_info) -> Option<&'a mut T> {
        let raw = unsafe { duckdb_function_get_local_init_data(info) };
        if raw.is_null() {
            return None;
        }
        Some(unsafe { &mut *raw.cast::<T>() })
    }

    /// Destroy callback: drops the `Box<T>`.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by [`set`][FfiLocalInitData::set].
    pub unsafe extern "C" fn destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MyState {
        counter: u64,
    }

    #[test]
    fn destroy_null_is_noop() {
        unsafe { FfiInitData::<MyState>::destroy(std::ptr::null_mut()) };
        unsafe { FfiLocalInitData::<MyState>::destroy(std::ptr::null_mut()) };
    }

    #[test]
    fn destroy_allocated_drops() {
        let raw = Box::into_raw(Box::new(MyState { counter: 7 })).cast::<c_void>();
        unsafe { FfiInitData::<MyState>::destroy(raw) };

        let raw2 = Box::into_raw(Box::new(MyState { counter: 3 })).cast::<c_void>();
        unsafe { FfiLocalInitData::<MyState>::destroy(raw2) };
    }
}
