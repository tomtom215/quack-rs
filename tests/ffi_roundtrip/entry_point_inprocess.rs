// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The whole entry-point sequence — `init_extension*`, `entry_point!` and
//! `entry_point_v2!` — driven in-process by a stand-in for `DuckDB`'s extension
//! loader, with no shared library.
//!
//! `DuckDB` hands an entry point an `info` pointer and a `duckdb_extension_access`
//! of three callbacks (`get_api`, `get_database`, `set_error`). The loader here
//! implements those over a real in-memory database and the real C API table
//! (the same `duckdb_ext_api_v1` the test harness uses to initialise the
//! dispatch table), so everything from `duckdb_rs_extension_api_init` to the
//! registered function being callable runs for real. Only what `LOAD` itself
//! does — finding the library and its symbol — is left out.

use std::ffi::{c_char, c_void, CStr};
use std::mem::MaybeUninit;
use std::sync::Mutex;

use libduckdb_sys::{
    duckdb_database, duckdb_ext_api_v1, duckdb_extension_access, duckdb_extension_info,
};
use quack_rs::abi::AbiPolicy;
use quack_rs::connection::Registrar;
use quack_rs::entry_point::{
    init_extension, init_extension_v2, init_extension_v2_with_policy, init_extension_with_policy,
};
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

use super::Fixture;

unsafe extern "C" {
    /// The harness's shim: fills a `duckdb_ext_api_v1` with the linked
    /// `DuckDB`'s functions, as `CreateAPIv1()` does. (`quack_rs::testing`
    /// declares the same two symbols privately.)
    fn quack_rs_fill_api_v1(out: *mut c_void);
    fn quack_rs_api_v1_size() -> usize;
}

/// A stand-in for `DuckDB`'s extension loader: the state behind the `info`
/// pointer and the three callbacks of its access struct.
struct Loader {
    /// What `get_api` returns; `None` makes it return null, as a `DuckDB` that
    /// does not provide the requested C API version does.
    api: Option<Box<MaybeUninit<duckdb_ext_api_v1>>>,
    /// What `get_database` points at; null makes it fail as `DuckDB`'s does
    /// after recording its own error.
    database: duckdb_database,
    database_fails: bool,
    /// The version string `get_api` was asked for.
    requested: Mutex<Option<String>>,
    /// The last message passed to `set_error`.
    error: Mutex<Option<String>>,
}

impl Loader {
    fn serving(database: duckdb_database) -> Self {
        // SAFETY: both are plain C functions without preconditions.
        let (bytes, expected) = unsafe {
            (
                quack_rs_api_v1_size(),
                std::mem::size_of::<duckdb_ext_api_v1>(),
            )
        };
        assert_eq!(
            bytes, expected,
            "the shim and the bindings agree on the API size"
        );
        let mut api = Box::new(MaybeUninit::<duckdb_ext_api_v1>::uninit());
        // SAFETY: the size check above passed, so the shim writes exactly the
        // struct, every byte of it.
        unsafe { quack_rs_fill_api_v1(api.as_mut_ptr().cast()) };
        Self {
            api: Some(api),
            database,
            database_fails: false,
            requested: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    const fn info(&self) -> duckdb_extension_info {
        std::ptr::from_ref(self).cast_mut().cast()
    }

    fn requested(&self) -> Option<String> {
        self.requested.lock().expect("lock").clone()
    }

    fn error(&self) -> Option<String> {
        self.error.lock().expect("lock").clone()
    }
}

/// # Safety
///
/// `info` is the `Loader` the test passed to the entry point.
unsafe fn loader<'a>(info: duckdb_extension_info) -> &'a Loader {
    // SAFETY: per the contract above.
    unsafe { &*info.cast::<Loader>() }
}

unsafe extern "C" fn get_api(info: duckdb_extension_info, version: *const c_char) -> *const c_void {
    // SAFETY: the entry point passes the `info` it was given, and a
    // NUL-terminated version.
    let loader = unsafe { loader(info) };
    let version = unsafe { CStr::from_ptr(version) }
        .to_string_lossy()
        .into_owned();
    *loader.requested.lock().expect("lock") = Some(version);
    loader
        .api
        .as_ref()
        .map_or(std::ptr::null(), |api| std::ptr::from_ref(&**api).cast())
}

unsafe extern "C" fn get_database(info: duckdb_extension_info) -> *mut duckdb_database {
    // SAFETY: as above.
    let loader = unsafe { loader(info) };
    if loader.database_fails {
        std::ptr::null_mut()
    } else {
        std::ptr::from_ref(&loader.database).cast_mut()
    }
}

unsafe extern "C" fn set_error(info: duckdb_extension_info, message: *const c_char) {
    // SAFETY: as above.
    let loader = unsafe { loader(info) };
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    *loader.error.lock().expect("lock") = Some(message);
}

fn access() -> duckdb_extension_access {
    duckdb_extension_access {
        set_error: Some(set_error),
        get_database: Some(get_database),
        get_api: Some(get_api),
    }
}

quack_rs::scalar_callback!(answer, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, 42) };
    }
});

fn answer_fn(name: &str) -> ScalarFunctionBuilder {
    ScalarFunctionBuilder::new(name)
        .returns(TypeId::BigInt)
        .function(answer)
}

fn call(fx: &Fixture, name: &str) -> Option<i64> {
    fx.scalar(&format!("SELECT {name}()"), |r, i| unsafe { r.read_i64(i) })
}

#[test]
fn init_extension_v2_registers_on_the_database_the_loader_names() {
    let fx = Fixture::open();
    let loader = Loader::serving(fx.db());
    let access = access();
    let mut saw_database = None;
    // SAFETY: `loader` and `access` outlive the call and are what an entry
    // point is given; the closure's connection is the live one DuckDB opened.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            saw_database = Some(con.as_raw_database());
            con.register_scalar(answer_fn("ep_v2_answer"))
        })
    };
    assert!(accepted, "init_extension_v2: {:?}", loader.error());
    assert_eq!(loader.error(), None);
    assert_eq!(loader.requested().as_deref(), Some("v1.2.0"));
    assert_eq!(
        saw_database,
        Some(fx.db()),
        "the closure sees the loader's database"
    );
    // The function the closure registered is visible to every connection.
    assert_eq!(call(&fx, "ep_v2_answer"), Some(42));
}

#[test]
fn init_extension_registers_on_a_raw_connection_to_the_same_database() {
    let fx = Fixture::open();
    let loader = Loader::serving(fx.db());
    let access = access();
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension(loader.info(), &raw const access, "v1.2.0", |con| {
            answer_fn("ep_v1_answer").register(con)
        })
    };
    assert!(accepted, "init_extension: {:?}", loader.error());
    assert_eq!(loader.error(), None);
    assert_eq!(call(&fx, "ep_v1_answer"), Some(42));
}

#[test]
fn every_abi_policy_loads_on_the_engine_it_was_built_for() {
    let fx = Fixture::open();
    let access = access();
    let policies = [
        AbiPolicy::Strict,
        AbiPolicy::AllowUnknownEngine,
        AbiPolicy::Warn,
        AbiPolicy::Trust,
    ];
    for (i, policy) in policies.into_iter().enumerate() {
        let loader = Loader::serving(fx.db());
        let (v1, v2) = (format!("ep_policy_v1_{i}"), format!("ep_policy_v2_{i}"));
        // SAFETY: as above.
        let accepted = unsafe {
            init_extension_with_policy(loader.info(), &raw const access, "v1.2.0", policy, |con| {
                answer_fn(&v1).register(con)
            })
        };
        assert!(accepted, "{policy:?}: {:?}", loader.error());
        // SAFETY: as above.
        let accepted = unsafe {
            init_extension_v2_with_policy(
                loader.info(),
                &raw const access,
                "v1.2.0",
                policy,
                |con| con.register_scalar(answer_fn(&v2)),
            )
        };
        assert!(accepted, "{policy:?}: {:?}", loader.error());
        assert_eq!(loader.error(), None, "{policy:?}");
        assert_eq!(call(&fx, &v1), Some(42), "{policy:?}");
        assert_eq!(call(&fx, &v2), Some(42), "{policy:?}");
    }
}

/// Registration is not transactional: what the closure registered before it
/// failed stays, and the failure reaches the loader's `set_error` verbatim.
#[test]
fn a_registration_error_fails_the_load_but_keeps_earlier_registrations() {
    let fx = Fixture::open();
    let access = access();
    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            con.register_scalar(answer_fn("ep_before_failure"))?;
            Err(ExtensionError::new("refused: configuration is missing"))
        })
    };
    assert!(!accepted);
    assert_eq!(
        loader.error().as_deref(),
        Some("refused: configuration is missing")
    );
    assert_eq!(call(&fx, "ep_before_failure"), Some(42));

    let loader = Loader::serving(fx.db());
    // SAFETY: as above.
    let accepted = unsafe {
        init_extension(loader.info(), &raw const access, "v1.2.0", |_con| {
            Err(ExtensionError::new("raw closure refused"))
        })
    };
    assert!(!accepted);
    assert_eq!(loader.error().as_deref(), Some("raw closure refused"));
}

#[test]
fn a_panicking_registration_fails_the_load_instead_of_unwinding_into_duckdb() {
    let fx = Fixture::open();
    let access = access();
    for v2 in [false, true] {
        let loader = Loader::serving(fx.db());
        // SAFETY: as above.
        let accepted = unsafe {
            if v2 {
                init_extension_v2(loader.info(), &raw const access, "v1.2.0", |_con| {
                    panic!("boom in the v2 closure")
                })
            } else {
                init_extension(loader.info(), &raw const access, "v1.2.0", |_con| {
                    panic!("boom in the v1 closure")
                })
            }
        };
        assert!(!accepted);
        let error = loader.error().expect("the panic is reported");
        assert!(
            error.contains("extension registration panicked: boom in the v"),
            "{error}"
        );
    }
}

/// A `DuckDB` that does not have the requested C API version answers `get_api`
/// with null: not an error, and the closure never runs. A failing
/// `get_database` has already recorded `DuckDB`'s own error, which quack-rs must
/// not overwrite.
#[test]
fn a_missing_api_or_database_ends_the_load_without_running_the_closure() {
    let fx = Fixture::open();
    let access = access();
    for v2 in [false, true] {
        let mut no_api = Loader::serving(fx.db());
        no_api.api = None;
        let mut no_database = Loader::serving(fx.db());
        no_database.database_fails = true;
        for (what, loader) in [("no API", &no_api), ("no database", &no_database)] {
            let mut ran = false;
            // SAFETY: as above.
            let accepted = unsafe {
                if v2 {
                    init_extension_v2(loader.info(), &raw const access, "v9.9.9", |_con| {
                        ran = true;
                        Ok(())
                    })
                } else {
                    init_extension(loader.info(), &raw const access, "v9.9.9", |_con| {
                        ran = true;
                        Ok(())
                    })
                }
            };
            assert!(!accepted, "{what} (v2: {v2})");
            assert!(!ran, "{what} (v2: {v2}): the closure must not run");
            assert_eq!(loader.error(), None, "{what} (v2: {v2})");
        }
        assert_eq!(no_api.requested().as_deref(), Some("v9.9.9"));
    }
}

// The macros wrap the same calls with the crate's own API version and the
// arguments evaluated under the panic guard.

quack_rs::entry_point!(ep_inprocess_v1_init_c_api, |con| {
    // SAFETY: `con` is the live connection DuckDB opened for us.
    unsafe { answer_fn("ep_macro_v1_answer").register(con) }
});

quack_rs::entry_point_v2!(ep_inprocess_v2_init_c_api, AbiPolicy::Trust, |con| {
    // SAFETY: `con` wraps the live connection DuckDB opened for us.
    unsafe { con.register_scalar(answer_fn("ep_macro_v2_answer")) }
});

#[test]
fn the_entry_point_macros_load_through_the_same_sequence() {
    let fx = Fixture::open();
    let access = access();
    for (entry, name) in [
        (
            ep_inprocess_v1_init_c_api as unsafe extern "C" fn(_, _) -> bool,
            "ep_macro_v1_answer",
        ),
        (ep_inprocess_v2_init_c_api, "ep_macro_v2_answer"),
    ] {
        let loader = Loader::serving(fx.db());
        // SAFETY: as above.
        let accepted = unsafe { entry(loader.info(), &raw const access) };
        assert!(accepted, "{name}: {:?}", loader.error());
        assert_eq!(loader.error(), None, "{name}");
        assert_eq!(
            loader.requested().as_deref(),
            Some(quack_rs::DUCKDB_API_VERSION),
            "{name}: the macros ask for the crate's C API version"
        );
        assert_eq!(call(&fx, name), Some(42), "{name}");
    }
}

// ─── What the closure's `Connection` can do ──────────────────────────────────

/// Handed to a replacement scan registered through the `Connection`.
struct Redirect {
    rows: i64,
}

/// Redirects `<anything><suffix>` to `range(rows)`.
fn redirect(info: libduckdb_sys::duckdb_replacement_scan_info, rows: i64) {
    // SAFETY: `info` is the pointer DuckDB passed in.
    unsafe { quack_rs::replacement_scan::ReplacementScanInfo::new(info) }
        .set_function("range")
        .add_i64_parameter(rows);
}

fn name_ends_with(table_name: *const c_char, suffix: &str) -> bool {
    // SAFETY: DuckDB passes a valid NUL-terminated identifier.
    unsafe { CStr::from_ptr(table_name) }
        .to_string_lossy()
        .ends_with(suffix)
}

quack_rs::replacement_scan_callback!(redirect_boxed, |info, table_name, data| {
    if name_ends_with(table_name, ".ep_boxed") {
        // SAFETY: `register_replacement_scan_with_data::<Redirect>` boxed it.
        redirect(info, unsafe { (*data.cast::<Redirect>()).rows });
    }
});

quack_rs::replacement_scan_callback!(redirect_raw, |info, table_name, data| {
    if name_ends_with(table_name, ".ep_raw") {
        // SAFETY: registered below with a `Box<i64>` behind `data`.
        redirect(info, unsafe { *data.cast::<i64>() });
    }
});

unsafe extern "C" fn drop_i64(ptr: *mut c_void) {
    if !ptr.is_null() {
        // SAFETY: allocated by `Box::into_raw` below.
        drop(unsafe { Box::from_raw(ptr.cast::<i64>()) });
    }
}

#[test]
fn the_connection_in_the_closure_runs_sql_and_outlives_it_on_request() {
    let fx = Fixture::open();
    let loader = Loader::serving(fx.db());
    let access = access();
    let mut kept = None;
    // SAFETY: `loader` and `access` outlive the call; the closure's
    // connection is the live one DuckDB opened.
    let accepted = unsafe {
        init_extension_v2(loader.info(), &raw const access, "v1.2.0", |con| {
            assert_eq!(con.execute("CREATE TABLE ep_rows(i BIGINT)")?, 0);
            assert_eq!(con.execute("INSERT INTO ep_rows VALUES (1), (2), (3)")?, 3);
            let mut sum = con.query("SELECT sum(i)::BIGINT FROM ep_rows")?;
            let chunk = sum.next_chunk()?.expect("one row");
            assert_eq!(chunk.reader(0).read_i64(0), 6);
            let stmt = con.prepare("SELECT count(*) FROM ep_rows WHERE i > ?")?;
            stmt.bind_i64(1, 1)?;
            let mut count = stmt.execute()?;
            assert_eq!(count.next_chunk()?.expect("row").reader(0).read_i64(0), 2);

            // A connection of its own, usable once the closure's is gone.
            kept = Some(con.open_connection()?);
            // Two replacement scans, one with a destructor-managed box.
            con.register_replacement_scan_with_data(redirect_boxed, Redirect { rows: 4 });
            con.register_replacement_scan(
                redirect_raw,
                Box::into_raw(Box::new(6_i64)).cast(),
                Some(drop_i64),
            );
            Ok(())
        })
    };
    assert!(accepted, "{:?}", loader.error());

    let kept = kept.expect("the closure opened a connection");
    let mut result = kept
        .query("SELECT count(*) FROM ep_rows")
        .expect("the kept connection outlives the closure's");
    let chunk = result.next_chunk().expect("fetch").expect("one row");
    // SAFETY: one BIGINT column.
    assert_eq!(unsafe { chunk.reader(0).read_i64(0) }, 3);
    // Tables the closure created are the database's, not the closure's.
    assert_eq!(
        fx.scalar("SELECT sum(i)::BIGINT FROM ep_rows", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(6)
    );

    let count = |table: &str| {
        fx.scalar(&format!("SELECT count(*) FROM '{table}'"), |r, i| unsafe {
            r.read_i64(i)
        })
    };
    assert_eq!(count("x.ep_boxed"), Some(4));
    assert_eq!(count("y.ep_raw"), Some(6));
}
