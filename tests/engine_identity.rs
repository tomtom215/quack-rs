// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! A refused load leaves `libduckdb-sys`'s process-wide dispatch table as it
//! was (F-07, `AUDIT.md` section 17).
//!
//! The bindings keep one C API function table per process and overwrite it on
//! every load. Before 0.21.0 the entry point published the new engine's table
//! first and checked its layout afterwards, so a load that `AbiPolicy::Strict`
//! then refused had already replaced the pointers a working engine's extension
//! functions call through; and a load into a second `DuckDB` library image in
//! the same process replaced them with that image's. The verdict and the
//! engine-identity check now run on the engine's own table before the bindings
//! see it, and a refusal hands them a null table, which they return from
//! without storing anything.
//!
//! No `DuckDB` is involved: each "engine" is a table of the right length whose
//! `duckdb_open` (slot 0, the identity) and `duckdb_library_version` (slot 7)
//! are this file's functions, and whose `get_database` returns null so the load
//! stops right after the dispatch table and the layout check. Its own test
//! binary, because the dispatch table and the recorded engine are process-wide,
//! and one `#[test]` because the steps depend on each other's state.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use libduckdb_sys::{duckdb_database, duckdb_extension_access, duckdb_extension_info};
use quack_rs::abi::AbiPolicy;
use quack_rs::entry_point::init_extension_with_policy;

type Slot = Option<unsafe extern "C" fn()>;

/// Calls of each engine's `duckdb_library_version`, so a test can tell whose
/// function the bindings' table holds.
static CALLS_A: AtomicUsize = AtomicUsize::new(0);
static CALLS_B: AtomicUsize = AtomicUsize::new(0);
static CALLS_OLD: AtomicUsize = AtomicUsize::new(0);

/// The release every engine here reports unless it is the "older" one: one
/// whose layout the compiled bindings match, so `Strict` accepts it (CI also
/// runs this file on the 548-slot `DuckDB` 2.0 bindings).
const fn compiled_release() -> &'static CStr {
    match quack_rs::abi::compiled_slot_count() {
        548 => c"v2.0.1",
        _ => c"v1.5.6",
    }
}

unsafe extern "C" fn version_a() -> *const c_char {
    CALLS_A.fetch_add(1, Ordering::SeqCst);
    compiled_release().as_ptr()
}
unsafe extern "C" fn version_b() -> *const c_char {
    CALLS_B.fetch_add(1, Ordering::SeqCst);
    compiled_release().as_ptr()
}
unsafe extern "C" fn version_old() -> *const c_char {
    CALLS_OLD.fetch_add(1, Ordering::SeqCst);
    c"v1.4.4".as_ptr()
}
// Two distinct `duckdb_open`s stand for two library images. Their bodies
// differ so the linker cannot fold them into one address.
unsafe extern "C" fn open_a() {
    CALLS_A.fetch_add(1000, Ordering::SeqCst);
}
unsafe extern "C" fn open_b() {
    CALLS_B.fetch_add(1000, Ordering::SeqCst);
}
const unsafe extern "C" fn filler() {}

/// One engine's API table and the error its load reported.
struct Engine {
    table: Vec<Slot>,
    error: Mutex<Option<String>>,
}

impl Engine {
    fn new(open: unsafe extern "C" fn(), version: unsafe extern "C" fn() -> *const c_char) -> Self {
        // Long enough for every layout quack-rs knows (548 slots, v2.0) and
        // for the compiled struct, so no read can leave it.
        let len = quack_rs::abi::compiled_slot_count().max(548);
        let mut table: Vec<Slot> = vec![Some(filler as unsafe extern "C" fn()); len];
        table[0] = Some(open);
        // SAFETY: a function pointer cast to another function pointer type of
        // the same size; the bindings transmute it back before calling it.
        table[7] = Some(unsafe {
            std::mem::transmute::<unsafe extern "C" fn() -> *const c_char, unsafe extern "C" fn()>(
                version,
            )
        });
        Self {
            table,
            error: Mutex::new(None),
        }
    }

    const fn info(&self) -> duckdb_extension_info {
        std::ptr::from_ref(self).cast_mut().cast()
    }

    /// Loads an extension into this engine under `policy`; `Some(message)` if
    /// the load reported an error.
    fn load(&self, policy: AbiPolicy) -> Option<String> {
        let access = duckdb_extension_access {
            set_error: Some(set_error),
            get_database: Some(get_database),
            get_api: Some(get_api),
        };
        // SAFETY: `self` and `access` outlive the call; the registration
        // closure is never reached (`get_database` returns null).
        let loaded = unsafe {
            init_extension_with_policy(self.info(), &raw const access, "v1.2.0", policy, |_| {
                unreachable!("get_database returns null, so registration never runs")
            })
        };
        assert!(!loaded);
        self.error.lock().expect("lock").take()
    }
}

/// # Safety
///
/// `info` is the `Engine` the test passed to the entry point.
unsafe fn engine<'a>(info: duckdb_extension_info) -> &'a Engine {
    // SAFETY: per the contract above.
    unsafe { &*info.cast::<Engine>() }
}

unsafe extern "C" fn get_api(info: duckdb_extension_info, _: *const c_char) -> *const c_void {
    // SAFETY: the entry point passes the `info` it was given.
    unsafe { engine(info) }.table.as_ptr().cast()
}

const unsafe extern "C" fn get_database(_: duckdb_extension_info) -> *mut duckdb_database {
    std::ptr::null_mut()
}

unsafe extern "C" fn set_error(info: duckdb_extension_info, message: *const c_char) {
    // SAFETY: a NUL-terminated message, and the `info` the test passed.
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: as above.
    *unsafe { engine(info) }.error.lock().expect("lock") = Some(message);
}

/// Whose `duckdb_library_version` the bindings' table holds now.
fn published() -> &'static str {
    let (a, b, old) = (
        CALLS_A.load(Ordering::SeqCst),
        CALLS_B.load(Ordering::SeqCst),
        CALLS_OLD.load(Ordering::SeqCst),
    );
    // SAFETY: the table was published by the first load below.
    let _ = unsafe { libduckdb_sys::duckdb_library_version() };
    if CALLS_A.load(Ordering::SeqCst) > a {
        "A"
    } else if CALLS_B.load(Ordering::SeqCst) > b {
        "B"
    } else if CALLS_OLD.load(Ordering::SeqCst) > old {
        "old"
    } else {
        "none"
    }
}

#[test]
fn a_refused_load_leaves_the_published_dispatch_table_alone() {
    let a = Engine::new(open_a, version_a);
    assert_eq!(a.load(AbiPolicy::Strict), None, "engine A loads");
    assert_eq!(published(), "A");

    // Loading into the same image again is fine, under any policy.
    assert_eq!(a.load(AbiPolicy::Strict), None);
    assert_eq!(a.load(AbiPolicy::Trust), None);
    assert_eq!(published(), "A");

    // Another library image: refused under every policy, `Trust` included,
    // because what it would break is the *other* engine's extension.
    let b = Engine::new(open_b, version_b);
    for policy in [
        AbiPolicy::Strict,
        AbiPolicy::AllowUnknownEngine,
        AbiPolicy::Warn,
        AbiPolicy::Trust,
    ] {
        let error = b.load(policy).expect("a second engine image is refused");
        assert!(
            error.contains("another DuckDB engine"),
            "{policy:?}: {error}"
        );
        assert_eq!(
            published(),
            "A",
            "{policy:?} let engine B overwrite the table"
        );
    }

    // Engine A's image again, but reporting a release whose layout differs:
    // with the unstable API compiled in, `Strict` refuses it before anything
    // is published (it used to refuse after publishing). Without it the stable
    // prefix is all the build uses, and every engine loads.
    let older = Engine::new(open_a, version_old);
    if quack_rs::abi::uses_unstable_api() {
        let error = older
            .load(AbiPolicy::Strict)
            .expect("a layout mismatch is refused");
        assert!(error.contains("v1.4.4"), "{error}");
        assert_eq!(published(), "A", "a refused layout overwrote the table");
    } else {
        assert_eq!(older.load(AbiPolicy::Strict), None);
        assert_eq!(published(), "old");
    }
}
