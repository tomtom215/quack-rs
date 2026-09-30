// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>

//! `FileHandle`'s `Drop` when the close fails.
//!
//! `duckdb_destroy_file_handle` calls `Close()` outside any `try`
//! (`file_system-c.cpp`), so a close that throws would escape the C API and
//! abort the process. `Drop` therefore closes through
//! `duckdb_file_handle_close`, which catches, and destroys only if that
//! succeeded; after a failed close it leaks the handle rather than let
//! destroy retry the close unguarded.
//!
//! No file system `DuckDB` ships throws from `Close()`, and the C API cannot
//! register one, so the failure is injected one level up: the test replaces
//! the `duckdb_file_handle_close` and `duckdb_destroy_file_handle` entries of
//! the `loadable-extension` dispatch table with stubs that record their
//! calls. That table is process-wide, which is why this is its own test
//! binary with a single `#[test]`.
//!
//! Before the fix, `Drop` called `duckdb_destroy_file_handle` directly: the
//! failing case below saw a destroy and no close.
#![cfg(all(feature = "_duckdb-testing", feature = "duckdb-1-5"))]

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use libduckdb_sys::{
    duckdb_connection, duckdb_database, duckdb_ext_api_v1, duckdb_extension_access,
    duckdb_extension_info, duckdb_file_handle, duckdb_state, DuckDBError, DuckDBSuccess,
};
use quack_rs::client_context::ClientContext;
use quack_rs::file_system::{FileOpenOptions, FileSystem};
use quack_rs::testing::InMemoryDb;

static CLOSE_CALLS: AtomicUsize = AtomicUsize::new(0);
static DESTROY_CALLS: AtomicUsize = AtomicUsize::new(0);
/// What the stub close reports.
static CLOSE_STATE: AtomicU32 = AtomicU32::new(DuckDBError);

/// Stands in for `duckdb_file_handle_close`: records the call and reports
/// `CLOSE_STATE`, without touching the handle.
unsafe extern "C" fn stub_close(_handle: duckdb_file_handle) -> duckdb_state {
    CLOSE_CALLS.fetch_add(1, Ordering::SeqCst);
    CLOSE_STATE.load(Ordering::SeqCst)
}

/// Stands in for `duckdb_destroy_file_handle`: records the call and nulls
/// the handle, as the real one does. The handle's memory is leaked, which is
/// harmless in a test process.
unsafe extern "C" fn stub_destroy(handle: *mut duckdb_file_handle) {
    DESTROY_CALLS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: `FileHandle::drop` passes a pointer to its own handle field.
    unsafe { *handle = std::ptr::null_mut() };
}

/// The struct `get_api` hands out.
static STUBS: std::sync::atomic::AtomicPtr<duckdb_ext_api_v1> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Hands out `STUBS`; both arguments are ignored.
unsafe extern "C" fn get_api(
    _info: duckdb_extension_info,
    _version: *const std::os::raw::c_char,
) -> *const std::os::raw::c_void {
    STUBS.load(Ordering::SeqCst).cast_const().cast()
}

/// Overwrites the close and destroy entries of the dispatch table.
fn install_stubs() {
    // SAFETY: every field of `duckdb_ext_api_v1` is an
    // `Option<unsafe extern "C" fn ..>`, for which all-zero bytes are `None`.
    let mut api: duckdb_ext_api_v1 = unsafe { std::mem::zeroed() };
    api.duckdb_file_handle_close = Some(stub_close);
    api.duckdb_destroy_file_handle = Some(stub_destroy);
    let api: &'static duckdb_ext_api_v1 = Box::leak(Box::new(api));
    STUBS.store(std::ptr::from_ref(api).cast_mut(), Ordering::SeqCst);

    let access = duckdb_extension_access {
        set_error: None,
        get_database: None,
        get_api: Some(get_api),
    };
    // SAFETY: `access.get_api` is set and returns a `'static` struct.
    // `duckdb_rs_extension_api_init` stores only the fields that are `Some`
    // (`libduckdb-sys`'s generated `bindgen_bundled_version_loadable.rs`), so
    // every other entry keeps the real function `InMemoryDb::open` installed.
    unsafe {
        libduckdb_sys::duckdb_rs_extension_api_init(std::ptr::null_mut(), &raw const access, "v1")
            .expect("re-initialise the dispatch table");
    }
}

#[test]
fn a_handle_whose_close_fails_is_leaked_not_destroyed() {
    let _dispatch = InMemoryDb::open().expect("initialise the DuckDB C API dispatch table");
    let mut db: duckdb_database = std::ptr::null_mut();
    let mut con: duckdb_connection = std::ptr::null_mut();
    // SAFETY: standard open/connect against a fresh in-memory database.
    unsafe {
        assert_eq!(
            libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db),
            DuckDBSuccess
        );
        assert_eq!(
            libduckdb_sys::duckdb_connect(db, &raw mut con),
            DuckDBSuccess
        );
    }
    // SAFETY: `con` is open until the end of the test.
    let ctx = unsafe { ClientContext::from_connection(con) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let dir = std::env::temp_dir().join(format!("quack_rs_close_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let open = |name: &str| {
        let path = CString::new(dir.join(name).to_str().expect("utf-8 path")).expect("no NUL");
        fs.open(&path, &FileOpenOptions::write_create())
            .expect("open")
    };
    // Opened with the real functions, before the stubs go in.
    let failing = open("failing.bin");
    let succeeding = open("succeeding.bin");

    install_stubs();

    CLOSE_STATE.store(DuckDBError, Ordering::SeqCst);
    drop(failing);
    assert_eq!(
        CLOSE_CALLS.load(Ordering::SeqCst),
        1,
        "Drop must close first"
    );
    assert_eq!(
        DESTROY_CALLS.load(Ordering::SeqCst),
        0,
        "after a failed close, destroy would retry the close outside any `try`"
    );

    CLOSE_STATE.store(DuckDBSuccess, Ordering::SeqCst);
    drop(succeeding);
    assert_eq!(CLOSE_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(
        DESTROY_CALLS.load(Ordering::SeqCst),
        1,
        "a handle that closed must be destroyed"
    );

    drop(fs);
    drop(ctx);
    // SAFETY: both handles were created above and are released once.
    unsafe {
        libduckdb_sys::duckdb_disconnect(&raw mut con);
        libduckdb_sys::duckdb_close(&raw mut db);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
