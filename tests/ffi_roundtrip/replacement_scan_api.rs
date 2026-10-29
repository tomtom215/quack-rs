// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ReplacementScanInfo`'s parameter methods and `ReplacementScanBuilder`'s
//! data-carrying registration, against a table function that echoes every
//! argument it is redirected to.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use quack_rs::replacement_scan::{ReplacementScanBuilder, ReplacementScanInfo};
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;

use super::Fixture;

/// `rs_echo(VARCHAR, BOOLEAN, BIGINT, DOUBLE)`: one row holding its four
/// arguments, in order.
fn register_echo(fx: &Fixture) {
    #[derive(Clone)]
    struct Echo {
        path: String,
        flag: bool,
        n: i64,
        x: f64,
        done: bool,
    }

    let echo = TableFunctionBuilder::new("rs_echo")
        .param(TypeId::Varchar)
        .param(TypeId::Boolean)
        .param(TypeId::BigInt)
        .param(TypeId::Double)
        .with_state::<Echo, _>(|bind| {
            for (name, ty) in [
                ("path", TypeId::Varchar),
                ("flag", TypeId::Boolean),
                ("n", TypeId::BigInt),
                ("x", TypeId::Double),
            ] {
                bind.add_result_column(name, ty);
            }
            // SAFETY: all four positional parameters are declared.
            let (path, flag, n, x) = unsafe {
                (
                    bind.get_parameter_value(0),
                    bind.get_parameter_value(1),
                    bind.get_parameter_value(2),
                    bind.get_parameter_value(3),
                )
            };
            Ok(Echo {
                path: path.as_str().unwrap_or_else(|e| format!("<{e}>")),
                flag: flag.as_bool().unwrap_or(false),
                n: n.as_i64().unwrap_or(i64::MIN),
                x: x.as_f64().unwrap_or(f64::NAN),
                done: false,
            })
        })
        .scan(|echo, chunk| {
            // SAFETY: four columns of the declared types; a zero size ends
            // the scan.
            unsafe {
                if echo.done {
                    chunk.set_size(0);
                } else {
                    chunk.writer(0).write_varchar(0, &echo.path);
                    chunk.writer(1).write_bool(0, echo.flag);
                    chunk.writer(2).write_i64(0, echo.n);
                    chunk.writer(3).write_f64(0, echo.x);
                    chunk.set_size(1);
                    echo.done = true;
                }
            }
            Ok(())
        })
        .build()
        .expect("build rs_echo");
    // SAFETY: `con` is open.
    unsafe { echo.register(fx.con()) }.expect("register rs_echo");
}

/// The one row `SELECT path, flag, n, x FROM <table>` returns.
fn echoed(fx: &Fixture, table: &str) -> (String, bool, i64, f64) {
    let mut result = fx.query(&format!("SELECT path, flag, n, x FROM {table}"));
    let chunk = result
        .next_chunk()
        .expect("fetch")
        .expect("one chunk with one row");
    assert_eq!(chunk.size(), 1, "{table}");
    // SAFETY: the four columns are (VARCHAR, BOOLEAN, BIGINT, DOUBLE).
    unsafe {
        (
            chunk.reader(0).read_str(0).to_owned(),
            chunk.reader(1).read_bool(0),
            chunk.reader(2).read_i64(0),
            chunk.reader(3).read_f64(0),
        )
    }
}

fn name_of(table_name: *const std::os::raw::c_char) -> String {
    // SAFETY: DuckDB passes a valid NUL-terminated identifier.
    unsafe { std::ffi::CStr::from_ptr(table_name) }
        .to_string_lossy()
        .into_owned()
}

quack_rs::replacement_scan_callback!(redirect_with_every_kind, |info, table_name, _data| {
    let name = name_of(table_name);
    // SAFETY: `info` is the pointer DuckDB passed in.
    let scan = unsafe { ReplacementScanInfo::new(info) };
    if name.ends_with(".rs_all") {
        scan.set_function("rs_echo")
            .add_varchar_parameter(&name)
            .add_bool_parameter(true)
            .add_i64_parameter(42);
        // A kind with no convenience method goes through the raw handle,
        // which DuckDB copies: the caller destroys its own.
        // SAFETY: a fresh value, destroyed once after being added.
        unsafe {
            let mut value = libduckdb_sys::duckdb_create_double(2.5);
            scan.add_parameter_raw(value);
            libduckdb_sys::duckdb_destroy_value(&raw mut value);
        }
    } else if name.ends_with(".rs_other") {
        // Different values of each kind, so a parameter that ignores its
        // argument (or a constant) is visible; the VARCHAR is multi-byte text,
        // which a copy of the wrong length would cut mid-character.
        scan.set_function("rs_echo")
            .add_varchar_parameter("ab\u{e9}\u{4e16} caf\u{e9}")
            .add_bool_parameter(false)
            .add_i64_parameter(-7);
        // SAFETY: as above.
        unsafe {
            let mut value = libduckdb_sys::duckdb_create_double(-0.125);
            scan.add_parameter_raw(value);
            libduckdb_sys::duckdb_destroy_value(&raw mut value);
        }
    } else if name.ends_with(".rs_err") {
        scan.set_error("custom replacement failure");
    }
});

#[test]
fn a_redirect_passes_each_kind_of_parameter_in_order() {
    let fx = Fixture::open();
    register_echo(&fx);
    // SAFETY: `db` is open; no extra data.
    unsafe {
        ReplacementScanBuilder::register(
            fx.db(),
            redirect_with_every_kind,
            std::ptr::null_mut(),
            None,
        );
    }

    assert_eq!(
        echoed(&fx, "'dir/file.rs_all'"),
        ("dir/file.rs_all".to_owned(), true, 42, 2.5)
    );
    assert_eq!(
        echoed(&fx, "'z.rs_other'"),
        ("ab\u{e9}\u{4e16} caf\u{e9}".to_owned(), false, -7, -0.125)
    );
    // Nine characters, thirteen bytes: DuckDB measures the first.
    assert_eq!(
        fx.scalar("SELECT length(path) FROM 'z.rs_other'", |r, i| unsafe {
            r.read_i64(i)
        }),
        Some(9)
    );
}

#[test]
fn a_replacement_scan_error_carries_its_message_to_the_query() {
    let fx = Fixture::open();
    register_echo(&fx);
    // SAFETY: `db` is open; no extra data.
    unsafe {
        ReplacementScanBuilder::register(
            fx.db(),
            redirect_with_every_kind,
            std::ptr::null_mut(),
            None,
        );
    }
    // SAFETY: `con` is open.
    let err = unsafe { quack_rs::query::query(fx.con(), "SELECT * FROM 'x.rs_err'") }
        .map(drop)
        .expect_err("the replacement scan reported an error");
    assert!(err.as_str().contains("custom replacement failure"), "{err}");
    // The error belongs to that name only.
    assert_eq!(echoed(&fx, "'ok.rs_all'").2, 42);
}

// ─── Registration with data ──────────────────────────────────────────────────

/// What `register_with_data` boxes: read back by the callback through the
/// `data` pointer, and counted when dropped.
struct Config {
    label: String,
    n: i64,
}

static CONFIG_DROPS: AtomicUsize = AtomicUsize::new(0);

impl Drop for Config {
    fn drop(&mut self) {
        CONFIG_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

quack_rs::replacement_scan_callback!(redirect_from_config, |info, table_name, data| {
    let name = name_of(table_name);
    if !name.ends_with(".rs_data") {
        return;
    }
    // SAFETY: `register_with_data::<Config>` boxed a `Config` behind `data`,
    // which stays live until the database closes.
    let config = unsafe { &*data.cast::<Config>() };
    // SAFETY: `info` is the pointer DuckDB passed in.
    let scan = unsafe { ReplacementScanInfo::new(info) };
    scan.set_function("rs_echo")
        .add_varchar_parameter(&format!("{}:{name}", config.label))
        .add_bool_parameter(config.n > 0)
        .add_i64_parameter(config.n);
    // SAFETY: a fresh value, destroyed once after being added.
    unsafe {
        let mut value = libduckdb_sys::duckdb_create_double(
            f64::from(u32::try_from(config.n).unwrap_or(0)) / 4.0,
        );
        scan.add_parameter_raw(value);
        libduckdb_sys::duckdb_destroy_value(&raw mut value);
    }
});

#[test]
fn register_with_data_hands_the_callback_its_value_and_drops_it_with_the_database() {
    let drops_before = CONFIG_DROPS.load(Ordering::SeqCst);
    let fx = Fixture::open();
    register_echo(&fx);
    // SAFETY: `db` is open; `Config` is `Send + Sync`.
    unsafe {
        ReplacementScanBuilder::register_with_data(
            fx.db(),
            redirect_from_config,
            Config {
                label: "cfg".to_owned(),
                n: 10,
            },
        );
    }
    assert_eq!(
        echoed(&fx, "'t.rs_data'"),
        ("cfg:t.rs_data".to_owned(), true, 10, 2.5)
    );
    // A name the callback declines falls through to DuckDB.
    // SAFETY: `con` is open.
    let err = unsafe { quack_rs::query::query(fx.con(), "SELECT * FROM 'nothing.here'") }
        .map(drop)
        .expect_err("no such table or file");
    assert!(err.as_str().contains("nothing.here"), "{err}");

    assert_eq!(
        CONFIG_DROPS.load(Ordering::SeqCst),
        drops_before,
        "the data lives as long as the database"
    );
    drop(fx);
    assert_eq!(
        CONFIG_DROPS.load(Ordering::SeqCst),
        drops_before + 1,
        "closing the database drops the boxed value exactly once"
    );
}

/// Two registrations on one database are tried in order, each given its own
/// data, and the first to redirect wins.
#[test]
fn replacement_scans_are_tried_in_registration_order_each_with_its_own_data() {
    static DELETES: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn delete_tag(ptr: *mut c_void) {
        if !ptr.is_null() {
            DELETES.fetch_add(1, Ordering::SeqCst);
            // SAFETY: allocated by `Box::into_raw` below.
            drop(unsafe { Box::from_raw(ptr.cast::<i64>()) });
        }
    }

    // The data is an `i64` here, read back as the BIGINT argument.
    quack_rs::replacement_scan_callback!(redirect_tagged, |info, table_name, data| {
        if !name_of(table_name).ends_with(".rs_tag") {
            return;
        }
        // SAFETY: `data` is the `Box<i64>` registered below.
        let tag = unsafe { *data.cast::<i64>() };
        // SAFETY: `info` is the pointer DuckDB passed in.
        let scan = unsafe { ReplacementScanInfo::new(info) };
        scan.set_function("rs_echo")
            .add_varchar_parameter("tagged")
            .add_bool_parameter(true)
            .add_i64_parameter(tag)
            .add_i64_parameter(0);
    });

    let fx = Fixture::open();
    register_echo(&fx);
    let before = DELETES.load(Ordering::SeqCst);
    for tag in [1_i64, 2] {
        let data = Box::into_raw(Box::new(tag)).cast::<c_void>();
        // SAFETY: `db` is open; `data` is a Box that `delete_tag` frees once.
        unsafe {
            ReplacementScanBuilder::register(fx.db(), redirect_tagged, data, Some(delete_tag));
        }
    }
    // The fourth argument is a BIGINT 0 where DOUBLE is declared; DuckDB
    // casts it.
    assert_eq!(
        echoed(&fx, "'x.rs_tag'"),
        ("tagged".to_owned(), true, 1, 0.0)
    );
    drop(fx);
    assert_eq!(
        DELETES.load(Ordering::SeqCst),
        before + 2,
        "each registration's data is deleted once"
    );
}
