// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `AbiPolicy` enforced against an engine whose layout differs, in-process.
//!
//! `enforce_abi_policy` is the glue between `abi::check` and the entry point:
//! `Trust` skips the check, `Strict` and `AllowUnknownEngine` refuse a layout
//! mismatch, `Warn` reports it and loads. The live suite only loads on the
//! engine the extension was built for, where every policy accepts, so
//! replacing the function by `Ok(())` (or flipping its `Trust` test) left every
//! `cargo test` green; the `abi-guard-layout` CI job covers it by loading real
//! extensions into real `DuckDB` releases. This drives the same sequence in-process by
//! serving the loader an API table whose `duckdb_library_version` names an
//! older release (v1.4.4: 459 slots, against the 546 the bindings carry).
//!
//! Its own test binary, because the dispatch table is process-wide: serving
//! this table to other tests would make them see a v1.4.4 engine.

#![cfg(all(feature = "_duckdb-testing", feature = "duckdb-1-5"))]

use std::ffi::{c_char, c_void, CStr};
use std::mem::MaybeUninit;
use std::sync::Mutex;

use libduckdb_sys::{
    duckdb_database, duckdb_ext_api_v1, duckdb_extension_access, duckdb_extension_info,
    DuckDBSuccess,
};
use quack_rs::abi::{AbiCheck, AbiPolicy};
use quack_rs::entry_point::init_extension_with_policy;
use quack_rs::testing::InMemoryDb;

extern "C" {
    fn quack_rs_fill_api_v1(out: *mut c_void);
    fn quack_rs_api_v1_size() -> usize;
}

/// The engine this table claims to be.
const unsafe extern "C" fn older_engine() -> *const c_char {
    c"v1.4.4".as_ptr()
}

struct Loader {
    api: Box<MaybeUninit<duckdb_ext_api_v1>>,
    database: duckdb_database,
    error: Mutex<Option<String>>,
}

impl Loader {
    fn serving_an_older_engine(database: duckdb_database) -> Self {
        // SAFETY: a plain C function without preconditions.
        let shim_size = unsafe { quack_rs_api_v1_size() };
        assert_eq!(shim_size, std::mem::size_of::<duckdb_ext_api_v1>());
        let mut api = Box::new(MaybeUninit::<duckdb_ext_api_v1>::uninit());
        // SAFETY: the size matches, so the shim writes every byte of the struct.
        unsafe { quack_rs_fill_api_v1(api.as_mut_ptr().cast()) };
        // SAFETY: just fully initialised.
        unsafe { api.assume_init_mut() }.duckdb_library_version = Some(older_engine);
        Self {
            api,
            database,
            error: Mutex::new(None),
        }
    }

    const fn info(&self) -> duckdb_extension_info {
        std::ptr::from_ref(self).cast_mut().cast()
    }
}

/// # Safety
///
/// `info` is the `Loader` the test passed to the entry point.
unsafe fn loader<'a>(info: duckdb_extension_info) -> &'a Loader {
    // SAFETY: per the contract above.
    unsafe { &*info.cast::<Loader>() }
}

unsafe extern "C" fn get_api(
    info: duckdb_extension_info,
    _version: *const c_char,
) -> *const c_void {
    // SAFETY: the entry point passes the `info` it was given.
    std::ptr::from_ref(unsafe { &*loader(info).api }).cast()
}

unsafe extern "C" fn get_database(info: duckdb_extension_info) -> *mut duckdb_database {
    // SAFETY: as above.
    std::ptr::from_ref(&unsafe { loader(info) }.database).cast_mut()
}

unsafe extern "C" fn set_error(info: duckdb_extension_info, message: *const c_char) {
    // SAFETY: as above, with a NUL-terminated message.
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: as above.
    let loader = unsafe { loader(info) };
    *loader.error.lock().expect("lock") = Some(message);
}

#[test]
fn each_policy_treats_a_layout_mismatch_as_it_documents() {
    let _dispatch = InMemoryDb::open().expect("initialise the dispatch table");
    let mut db: duckdb_database = std::ptr::null_mut();
    let mut con = std::ptr::null_mut();
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
    let access = duckdb_extension_access {
        set_error: Some(set_error),
        get_database: Some(get_database),
        get_api: Some(get_api),
    };

    for (policy, loads) in [
        (AbiPolicy::Strict, false),
        (AbiPolicy::AllowUnknownEngine, false), // v1.4.4 is known, and different
        (AbiPolicy::Warn, true),
        (AbiPolicy::Trust, true),
    ] {
        let loader = Loader::serving_an_older_engine(db);
        let mut ran = false;
        // SAFETY: `loader` and `access` outlive the call; the connection the
        // closure gets is live.
        let accepted = unsafe {
            init_extension_with_policy(loader.info(), &raw const access, "v1.2.0", policy, |_con| {
                ran = true;
                Ok(())
            })
        };
        let error = loader.error.lock().expect("lock").clone();
        assert_eq!(accepted, loads, "{policy:?}: {error:?}");
        assert_eq!(
            ran, loads,
            "{policy:?}: the registration closure runs only if the load goes on"
        );
        if loads {
            assert_eq!(error, None, "{policy:?}: a warning is not an error");
        } else {
            let error = error.unwrap_or_else(|| panic!("{policy:?}: a refusal must say why"));
            assert!(error.contains("v1.4.4"), "{policy:?}: {error}");
        }
    }
    // The stand-in really does look like a different layout.
    // SAFETY: the table is initialised.
    let check = unsafe { quack_rs::abi::check() };
    assert!(
        matches!(check, AbiCheck::LayoutMismatch { .. }),
        "{check:?}"
    );

    // SAFETY: both handles were opened above and are closed once.
    unsafe {
        libduckdb_sys::duckdb_disconnect(&raw mut con);
        libduckdb_sys::duckdb_close(&raw mut db);
    }
}
