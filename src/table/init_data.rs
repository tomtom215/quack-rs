// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Type-safe init data management for table functions.
//!
//! `DuckDB` table functions have two init phases:
//!
//! - **Global init** (`init`): Called once per execution of a bound plan. Use
//!   [`FfiInitData`] to store global scan state (e.g., a file handle, row
//!   counter shared across threads).
//! - **Local init** (`local_init`): Called once per scanning thread. Use
//!   [`FfiLocalInitData`] to store per-thread scan state (e.g., a thread-local
//!   buffer or offset).
//!
//! # Threads
//!
//! Global init data is shared by **every** concurrent scan call. `DuckDB` runs
//! up to [`InitInfo::set_max_threads`][crate::table::InitInfo::set_max_threads]
//! scans at once — whether or not `local_init` is set — and the default is 1.
//! So with `max_threads > 1`:
//!
//! - [`FfiInitData::get`] hands several threads `&T` at once, which is why
//!   [`FfiInitData::set`] requires `T: Send + Sync`;
//! - [`FfiInitData::get_mut`] would hand several threads `&mut T` at once — a
//!   data race. Keep mutable global state behind a `Mutex` or atomics and use
//!   `get`, or leave `max_threads` at 1.
//!
//! Local init data belongs to one scanning task at a time, but that task may be
//! resumed on a different worker thread, so [`FfiLocalInitData::set`] requires
//! `T: Send`.
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
    /// `T` must be `Send + Sync`: the value is created on the init thread,
    /// read by up to `max_threads` concurrent scan calls, and dropped on
    /// whichever thread tears the plan down.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_init_info` passed to the table function's
    ///   global `init` callback, not its `local_init`. Both kinds of init data
    ///   are stored through `duckdb_init_set_init_data`, so the callback alone
    ///   decides which one this sets; called from `local_init`, it sets the
    ///   local init data, which [`FfiLocalInitData::get`] would then read as its
    ///   own type.
    /// - Must be called at most once per init invocation.
    pub unsafe fn set(info: duckdb_init_info, data: T)
    where
        T: Send + Sync,
    {
        let raw = Box::into_raw(Box::new(data)).cast::<c_void>();
        // SAFETY: info is valid; raw is a heap allocation; destroy is a valid fn pointer.
        unsafe {
            duckdb_init_set_init_data(info, raw, Some(Self::destroy));
        }
    }

    /// Retrieves a shared reference to the global init data from a scan callback.
    ///
    /// Returns `None` if no init data was set. Concurrent scan calls may hold
    /// this reference at the same time; [`set`][Self::set] requires `T: Sync`
    /// for that reason.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info` from a scan callback.
    /// - `T` must be the type passed to [`set`][Self::set].
    /// - No mutable reference to the same data must exist simultaneously.
    /// - The returned reference must not outlive the scan callback: `DuckDB`
    ///   destroys the data with the scan state.
    pub unsafe fn get<'a>(info: duckdb_function_info) -> Option<&'a T> {
        // SAFETY: info is valid per caller's contract.
        let raw = unsafe { duckdb_function_get_init_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw is non-null (checked above) and, by the `# Safety`
        // clauses, is the live `Box<T>` that `set` leaked for this same `T`.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Retrieves a mutable reference to the global init data from a scan callback.
    ///
    /// Returns `None` if no init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info` from a scan callback.
    /// - `T` must be the type passed to [`set`][Self::set].
    /// - The returned reference must not outlive the scan callback: `DuckDB`
    ///   destroys the data with the scan state.
    /// - No other reference to the same data must exist simultaneously. Global
    ///   init data is shared by every concurrent scan call of the query, so this
    ///   holds only when the init callback left `max_threads` at 1 (the default)
    ///   or the scan otherwise serialises calls to `get_mut`. With
    ///   [`InitInfo::set_max_threads`][crate::table::InitInfo::set_max_threads]
    ///   above 1, use [`get`][Self::get] and interior mutability (`Mutex`,
    ///   atomics) instead — `local_init` does **not** change this.
    pub unsafe fn get_mut<'a>(info: duckdb_function_info) -> Option<&'a mut T> {
        // SAFETY: info is valid per caller's contract.
        let raw = unsafe { duckdb_function_get_init_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw is non-null (checked above) and, by the `# Safety`
        // clauses, is the live `Box<T>` that `set` leaked for this same `T`;
        // exclusivity is the caller's contract above.
        Some(unsafe { &mut *raw.cast::<T>() })
    }

    /// Destroy callback: drops the `Box<T>`.
    ///
    /// # Pitfall L3: `T::drop` is user code
    ///
    /// `DuckDB` calls this through an `extern "C"` function pointer that has no
    /// error channel, and an unwind across `extern "C"` aborts the process on
    /// Rust 1.81+. The drop runs under
    /// [`catch_ffi_panic`][crate::callback::catch_ffi_panic] so a panicking
    /// `Drop` impl cannot take the session down.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by [`set`][FfiInitData::set].
    pub unsafe extern "C" fn destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            // SAFETY: `ptr` came from `Box::into_raw` in `set`; `T::drop` is
            // arbitrary user code, so its unwind is contained here.
            drop(crate::callback::catch_ffi_panic(|| unsafe {
                drop(Box::from_raw(ptr.cast::<T>()));
            }));
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
    /// `T` must be `Send`: a scanning task may be resumed on a different
    /// worker thread, and the value is dropped wherever the plan is torn down.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_init_info` passed to the table function's
    ///   `local_init` callback, not its global `init`. Both kinds of init data
    ///   are stored through `duckdb_init_set_init_data`, so the callback alone
    ///   decides which one this sets; called from `init`, it sets the global
    ///   init data, which [`FfiInitData::get`] would then read as its own type.
    /// - Must be called at most once per `local_init` invocation.
    pub unsafe fn set(info: duckdb_init_info, data: T)
    where
        T: Send,
    {
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
    /// - `T` must be the type passed to [`set`][Self::set].
    /// - No mutable reference to the same data must exist simultaneously.
    /// - The returned reference must not outlive the scan callback: `DuckDB`
    ///   destroys the data with the scan state.
    pub unsafe fn get<'a>(info: duckdb_function_info) -> Option<&'a T> {
        // SAFETY: `info` is a valid `duckdb_function_info` per the first `# Safety` clause;
        // `duckdb_function_get_local_init_data` only returns the stored
        // `local_data.init_data` pointer (null if none, table_function-c.cpp).
        let raw = unsafe { duckdb_function_get_local_init_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is non-null (checked above) and, by the second `# Safety`
        // clause, the live `Box<T>` that `set` leaked for this same `T`; the
        // third rules out a `&mut`, the fourth bounds `'a` by the scan call.
        Some(unsafe { &*raw.cast::<T>() })
    }

    /// Retrieves a mutable reference to the per-thread local init data.
    ///
    /// Returns `None` if no local init data was set.
    ///
    /// # Safety
    ///
    /// - `info` must be a valid `duckdb_function_info`.
    /// - `T` must be the type passed to [`set`][Self::set].
    /// - No other reference to the same data must exist simultaneously.
    /// - The returned reference must not outlive the scan callback: `DuckDB`
    ///   destroys the data with the scan state.
    pub unsafe fn get_mut<'a>(info: duckdb_function_info) -> Option<&'a mut T> {
        // SAFETY: `info` is a valid `duckdb_function_info` per the first `# Safety` clause;
        // `duckdb_function_get_local_init_data` only returns the stored
        // `local_data.init_data` pointer (null if none, table_function-c.cpp).
        let raw = unsafe { duckdb_function_get_local_init_data(info) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is non-null (checked above) and, by the second `# Safety`
        // clause, the live `Box<T>` that `set` leaked for this same `T`; the
        // third gives exclusivity, the fourth bounds `'a` by the scan call.
        Some(unsafe { &mut *raw.cast::<T>() })
    }

    /// Destroy callback: drops the `Box<T>`.
    ///
    /// # Pitfall L3: `T::drop` is user code
    ///
    /// `DuckDB` calls this through an `extern "C"` function pointer that has no
    /// error channel, and an unwind across `extern "C"` aborts the process on
    /// Rust 1.81+. The drop runs under
    /// [`catch_ffi_panic`][crate::callback::catch_ffi_panic] so a panicking
    /// `Drop` impl cannot take the session down.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by [`set`][FfiLocalInitData::set].
    pub unsafe extern "C" fn destroy(ptr: *mut c_void) {
        if !ptr.is_null() {
            // SAFETY: `ptr` came from `Box::into_raw` in `set`; `T::drop` is
            // arbitrary user code, so its unwind is contained here.
            drop(crate::callback::catch_ffi_panic(|| unsafe {
                drop(Box::from_raw(ptr.cast::<T>()));
            }));
        }
    }
}

impl<T: 'static> core::fmt::Debug for FfiInitData<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FfiInitData<")?;
        f.write_str(core::any::type_name::<T>())?;
        f.write_str(">")
    }
}

impl<T: 'static> core::fmt::Debug for FfiLocalInitData<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FfiLocalInitData<")?;
        f.write_str(core::any::type_name::<T>())?;
        f.write_str(">")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
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
