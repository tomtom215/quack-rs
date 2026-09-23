// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `extern "C"` trampolines behind [`TypedTableFunctionBuilder`][super::TypedTableFunctionBuilder].
//!
//! State flow:
//!
//! ```text
//! bind  → user bind closure → Box<StateFactory<S>>   stored as bind data (shared, immutable)
//! init  → factory()         → Mutex<S>               stored as init data (one per execution)
//! scan  → lock              → &mut S                 passed to the user scan closure
//! ```
//!
//! `DuckDB` runs `init` against the same bind data once per execution of a
//! bound plan, so `init` only ever *borrows* the factory.

use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use libduckdb_sys::{
    duckdb_bind_info, duckdb_data_chunk, duckdb_data_chunk_set_size, duckdb_function_info,
    duckdb_init_info,
};

use super::{StateFactory, TypedCallbacks};
use crate::data_chunk::DataChunk;
use crate::table::bind_data::FfiBindData;
use crate::table::builder::TableFunctionBuilder;
use crate::table::info::{BindInfo, FunctionInfo, InitInfo};
use crate::table::init_data::FfiInitData;

/// Bind data stored by [`typed_bind_trampoline`].
type Factory<S> = Box<StateFactory<S>>;

/// Wires the trampolines and the boxed callbacks into `inner`.
pub(super) fn wire<S: Send + 'static>(
    inner: TableFunctionBuilder,
    callbacks: TypedCallbacks<S>,
) -> TableFunctionBuilder {
    let raw = Box::into_raw(Box::new(callbacks)).cast::<c_void>();
    // SAFETY: `raw` is a freshly-allocated, non-null pointer to a
    // `TypedCallbacks<S>`, and `destroy_extra::<S>` drops exactly that type.
    // The builder's `ExtraInfo` frees it if registration never happens.
    unsafe {
        inner
            .bind(typed_bind_trampoline::<S>)
            .init(typed_init_trampoline::<S>)
            .scan(typed_scan_trampoline::<S>)
            .extra_info(raw, destroy_extra::<S>)
    }
}

/// `extra_info` destructor passed to `DuckDB`.
///
/// # Safety
///
/// `ptr` must have been produced by [`Box::into_raw`] on a
/// `Box<TypedCallbacks<S>>` created by [`wire`]. Called exactly once.
unsafe extern "C" fn destroy_extra<S: Send + 'static>(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was produced by Box::into_raw in `wire`. The boxed closures
    // capture user data whose `Drop` may panic, and this is an `extern "C"`
    // boundary with no error channel, so contain the unwind.
    drop(crate::callback::catch_ffi_panic(|| unsafe {
        drop(Box::from_raw(ptr.cast::<TypedCallbacks<S>>()));
    }));
}

/// Extracts a human-readable message from a `catch_unwind` panic payload.
///
/// The payload text is included: `DuckDB`'s `set_error` takes an ordinary
/// `&str`, so there is no reason to discard the one piece of information that
/// tells the user *which* assertion or `unwrap` failed.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&'static str>().map_or_else(
        || {
            payload.downcast_ref::<String>().map_or_else(
                || {
                    String::from(
                        "quack-rs: typed table function closure panicked (unknown payload)",
                    )
                },
                |s| format!("quack-rs: typed table function closure panicked: {s}"),
            )
        },
        |s| format!("quack-rs: typed table function closure panicked: {s}"),
    )
}

/// Copies the message out of a caught panic payload, then disposes of the
/// payload without letting a panicking `Drop` escape.
///
/// The payload is user data: `panic_any` can carry any type, including one whose
/// destructor panics. Dropped at the end of the trampoline, that second panic
/// would unwind out of an `extern "C" fn` and abort the process.
fn contain_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    let message = panic_message(&*payload);
    crate::callback::drop_panic_payload(payload);
    message
}

/// Bind trampoline monomorphised per state type `S`.
///
/// # Safety
///
/// Invoked by `DuckDB` during binding. `info` is a valid `duckdb_bind_info`.
unsafe extern "C" fn typed_bind_trampoline<S: Send + 'static>(info: duckdb_bind_info) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: DuckDB guarantees `info` is valid for the duration of the bind callback.
        let bind_info = unsafe { BindInfo::new(info) };
        // SAFETY: extra_info was set by `wire()` to a `Box<TypedCallbacks<S>>`.
        let raw = unsafe { bind_info.get_extra_info() };
        if raw.is_null() {
            bind_info.set_error("quack-rs: typed table function missing extra_info");
            return;
        }
        // SAFETY: `raw` originated from `Box::into_raw(Box::new(TypedCallbacks::<S>))`
        // in `wire()`. It remains valid until DuckDB invokes `destroy_extra`.
        let cbs = unsafe { &*raw.cast::<TypedCallbacks<S>>() };

        match (cbs.bind)(&bind_info) {
            Ok(factory) => {
                // SAFETY: `info` is valid; this is the bind callback's single
                // opportunity to set bind data. The factory is `Send + Sync`,
                // as bind data shared across init threads must be.
                unsafe { FfiBindData::<Factory<S>>::set(info, factory) };
            }
            Err(e) => bind_info.set_error(e.as_str()),
        }
    }));

    if let Err(payload) = outcome {
        let message = contain_payload(payload);
        // SAFETY: `info` is valid.
        let bind_info = unsafe { BindInfo::new(info) };
        bind_info.set_error(&message);
    }
}

/// Init trampoline monomorphised per state type `S`.
///
/// Builds a fresh `S` from the bind data's factory — never moving anything out
/// of the bind data, which `DuckDB` reuses for the next execution.
///
/// # Safety
///
/// Invoked by `DuckDB` once per execution of a bound plan. `info` is a valid
/// `duckdb_init_info`.
unsafe extern "C" fn typed_init_trampoline<S: Send + 'static>(info: duckdb_init_info) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `info` is valid for the duration of this callback.
        let init_info = unsafe { InitInfo::new(info) };

        // SAFETY: bind_data was set by `typed_bind_trampoline` as a
        // `Factory<S>`, which is only ever read. It lives until plan teardown.
        let Some(factory) = (unsafe { FfiBindData::<Factory<S>>::get_from_init(info) }) else {
            init_info.set_error("quack-rs: typed table function missing bind state");
            return;
        };

        match factory() {
            Ok(state) => {
                // SAFETY: `info` is valid; `FfiInitData::set` boxes the state
                // and registers a drop-on-destroy callback with DuckDB.
                // `Mutex<S>` is `Send + Sync` for any `S: Send`.
                unsafe { FfiInitData::<Mutex<S>>::set(info, Mutex::new(state)) };
            }
            Err(e) => {
                init_info.set_error(e.as_str());
                return;
            }
        }

        // One scan at a time: the mutex already serialises access to `S`,
        // and a second worker would only find the state exhausted.
        init_info.set_max_threads(1);
    }));

    if let Err(payload) = outcome {
        let message = contain_payload(payload);
        // SAFETY: `info` is valid.
        let init_info = unsafe { InitInfo::new(info) };
        init_info.set_error(&message);
    }
}

/// Scan trampoline monomorphised per state type `S`.
///
/// # Safety
///
/// Invoked by `DuckDB` repeatedly until the chunk size is set to zero.
unsafe extern "C" fn typed_scan_trampoline<S: Send + 'static>(
    info: duckdb_function_info,
    output: duckdb_data_chunk,
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `info` is valid per DuckDB contract.
        let fninfo = unsafe { FunctionInfo::new(info) };
        let fail = |message: &str| {
            fninfo.set_error(message);
            // SAFETY: `output` is a valid data chunk.
            unsafe { duckdb_data_chunk_set_size(output, 0) };
        };
        // SAFETY: extra_info was set by `wire()`.
        let raw = unsafe { fninfo.get_extra_info() };
        if raw.is_null() {
            fail("quack-rs: typed table function missing extra_info");
            return;
        }
        // SAFETY: same provenance as the bind trampoline.
        let cbs = unsafe { &*raw.cast::<TypedCallbacks<S>>() };

        // SAFETY: init_data was set by `typed_init_trampoline` as a
        // `Mutex<S>`; only shared references are taken, and the mutex
        // arbitrates the `&mut S`.
        let Some(cell) = (unsafe { FfiInitData::<Mutex<S>>::get(info) }) else {
            fail("quack-rs: typed table function missing scan state");
            return;
        };
        let Ok(mut state) = cell.lock() else {
            // A previous scan call panicked mid-update.
            fail("quack-rs: typed table function scan state poisoned by an earlier panic");
            return;
        };

        // SAFETY: `output` is a valid data chunk provided by DuckDB.
        let chunk = unsafe { DataChunk::from_raw(output) };
        if let Err(e) = (cbs.scan)(&mut state, &chunk) {
            fail(e.as_str());
        }
    }));

    if let Err(payload) = outcome {
        let message = contain_payload(payload);
        // SAFETY: `info` is valid.
        let fninfo = unsafe { FunctionInfo::new(info) };
        fninfo.set_error(&message);
        // SAFETY: `output` is valid.
        unsafe { duckdb_data_chunk_set_size(output, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyState;

    #[test]
    fn destroy_extra_null_is_noop() {
        // Must not panic.
        unsafe { destroy_extra::<DummyState>(std::ptr::null_mut()) };
    }

    #[test]
    fn destroy_extra_drops_box() {
        let cbs: Box<TypedCallbacks<DummyState>> = Box::new(TypedCallbacks {
            bind: Box::new(|_| Ok(Box::new(|| Ok(DummyState)))),
            scan: Box::new(|_, _| Ok(())),
        });
        let raw = Box::into_raw(cbs).cast::<c_void>();
        unsafe { destroy_extra::<DummyState>(raw) };
    }

    #[test]
    fn panic_message_classifies_known_payloads() {
        let s: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert!(panic_message(&*s).contains("panicked"));
        let s: Box<dyn std::any::Any + Send> = Box::new(String::from("boom"));
        assert!(panic_message(&*s).contains("panicked"));
        // Unknown payload falls through to the "unknown payload" branch.
        let s: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert!(panic_message(&*s).contains("unknown payload"));
    }

    #[test]
    fn contain_payload_returns_the_payload_message() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(
            contain_payload(payload),
            "quack-rs: typed table function closure panicked: boom"
        );
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("bang"));
        assert_eq!(
            contain_payload(payload),
            "quack-rs: typed table function closure panicked: bang"
        );
    }

    #[test]
    fn contain_payload_survives_a_payload_whose_drop_panics() {
        struct DropBomb;
        impl Drop for DropBomb {
            fn drop(&mut self) {
                panic!("payload destructor panicked");
            }
        }
        let payload: Box<dyn std::any::Any + Send> = Box::new(DropBomb);
        assert_eq!(
            contain_payload(payload),
            "quack-rs: typed table function closure panicked (unknown payload)"
        );
    }
}
