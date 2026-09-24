// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Every RAII handle frees what `DuckDB` allocated for it.
//!
//! Each wrapper's `Drop` calls a `duckdb_destroy_*` (or `duckdb_close_*`)
//! function, and nothing else observes it: a `Drop` that did nothing would
//! pass every functional test. The fifth audit's end-to-end mutation run
//! showed exactly that — a `Drop` replaced by `()` survived for
//! `ChunkWriter`, `DbConfig`, `Value`, `Appender`, `Catalog`,
//! `ClientContext`, `Expression`, `FileOpenOptions`, `FileSystem`,
//! `FileHandle`, `InstanceCache`, `SelectionVector`, `TableDescription` and
//! `OwnedDataChunk`, and for the free in `PreparedStatement::parameter_name`.
//!
//! Each scenario here creates and drops a handle many times and bounds the
//! growth of the C heap in use (glibc's `mallinfo2`, which counts `DuckDB`'s
//! `new`-allocated wrappers and Rust's allocations alike). A leaked handle
//! is at least a 32-byte chunk each, so a loop of `ROUNDS` leaks at least
//! 32 * `ROUNDS` bytes; the bound is a tenth of that, or all of it where each
//! round runs a query. A file handle's leak
//! is an open descriptor, counted in `/proc/self/fd`. Linux with glibc only,
//! and its own test binary with one `#[test]`, so nothing else allocates
//! concurrently.

#![cfg(all(feature = "_duckdb-testing", target_os = "linux", target_env = "gnu"))]
// Test code: casts at FFI edges and one long scenario list, as in
// `tests/ffi_roundtrip.rs`; each unsafe block's invariant is the test's setup.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::struct_field_names,
    clippy::undocumented_unsafe_blocks
)]

use libduckdb_sys::{duckdb_connection, duckdb_database};
use quack_rs::query::OwnedConnection;
use quack_rs::testing::InMemoryDb;

/// glibc's `struct mallinfo2`.
#[repr(C)]
struct MallInfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

extern "C" {
    fn mallinfo2() -> MallInfo2;
}

/// Bytes of C heap in use: small chunks plus mmapped ones.
fn heap_in_use() -> isize {
    // SAFETY: `mallinfo2` takes no arguments and returns a plain struct.
    let info = unsafe { mallinfo2() };
    isize::try_from(info.uordblks + info.hblkhd).unwrap_or(isize::MAX)
}

const ROUNDS: usize = 2_000;

/// A leak of one 32-byte chunk per round would exceed this ten times over.
const BOUND: isize = (ROUNDS as isize) * 32 / 10;

/// For scenarios that run a query each round, whose own churn measured up to
/// 4,672 bytes over `ROUNDS`: a leaked data chunk (kilobytes per column) or
/// expression wrapper still exceeds it many times over.
const QUERY_BOUND: isize = (ROUNDS as isize) * 32;

/// The scenarios that run a query each round.
const QUERY_SCENARIOS: &[&str] = &["OwnedDataChunk", "Expression"];

/// The heap growth, in bytes, over `ROUNDS` calls of `round`, after a warm-up
/// that lets caches fill.
fn growth(mut round: impl FnMut()) -> isize {
    for _ in 0..ROUNDS / 10 {
        round();
    }
    let before = heap_in_use();
    for _ in 0..ROUNDS {
        round();
    }
    heap_in_use() - before
}

#[cfg(feature = "duckdb-1-5")]
fn open_files() -> usize {
    std::fs::read_dir("/proc/self/fd").map_or(0, Iterator::count)
}

struct Db {
    db: duckdb_database,
    con: duckdb_connection,
    _dispatch: InMemoryDb,
}

impl Db {
    fn open() -> Self {
        let dispatch = InMemoryDb::open().expect("dispatch table");
        let mut db: duckdb_database = std::ptr::null_mut();
        let mut con: duckdb_connection = std::ptr::null_mut();
        unsafe {
            assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db), 0);
            assert_eq!(libduckdb_sys::duckdb_connect(db, &raw mut con), 0);
        }
        Self {
            db,
            con,
            _dispatch: dispatch,
        }
    }

    fn execute(&self, sql: &str) {
        unsafe { quack_rs::query::query(self.con, sql) }.expect(sql);
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        unsafe {
            libduckdb_sys::duckdb_disconnect(&raw mut self.con);
            libduckdb_sys::duckdb_close(&raw mut self.db);
        }
    }
}

#[cfg(feature = "duckdb-1-5")]
unsafe extern "C" fn inspect_argument(info: quack_rs::scalar::RawScalarBindInfo) {
    let bind = unsafe { quack_rs::scalar::ScalarBindInfo::new(info) };
    // Before v1.5.5 `argument` refuses and sets the bind error; a panic here
    // would cross this `extern "C"` function and abort.
    drop(unsafe { bind.argument(0) });
}

#[cfg(feature = "duckdb-1-5")]
quack_rs::scalar_callback!(pass_through, |_info, input, output| {
    let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { quack_rs::vector::VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row)) };
    }
});

#[test]
fn every_handle_frees_what_duckdb_allocated_for_it() {
    let db = Db::open();
    db.execute("CREATE TABLE t (a INTEGER, b VARCHAR)");
    let mut results: Vec<(&str, isize)> = Vec::new();

    results.push((
        "DbConfig",
        growth(|| drop(quack_rs::config::DbConfig::new().expect("config"))),
    ));
    results.push((
        "Value",
        growth(|| drop(quack_rs::value::Value::varchar("a value to leak"))),
    ));
    results.push((
        "Appender",
        growth(|| {
            let appender = unsafe { quack_rs::appender::Appender::new(db.con, None, c"t") };
            drop(appender.expect("appender"));
        }),
    ));
    results.push((
        "TableDescription",
        growth(|| {
            let desc = unsafe {
                quack_rs::table_description::TableDescription::create(db.con, "main", "t")
            };
            drop(desc.expect("table description"));
        }),
    ));
    let con = unsafe { OwnedConnection::open(db.db) }.expect("connect");
    results.push((
        "OwnedDataChunk",
        growth(|| {
            let mut result = con.query("SELECT 1").expect("query");
            drop(result.next_chunk().expect("fetch").expect("one chunk"));
        }),
    ));

    // `parameter_name` copies DuckDB's `strdup`ed name and must free it.
    let named = con.prepare("SELECT $a_parameter_name").expect("prepare");
    results.push((
        "PreparedStatement::parameter_name",
        growth(|| drop(named.parameter_name(1).expect("a name"))),
    ));
    drop(named);

    #[cfg(feature = "duckdb-1-5")]
    {
        // An `Expression` only comes from a bind callback, so each round is a
        // query; the bound leaves room for the query's own churn.
        unsafe {
            quack_rs::scalar::ScalarFunctionBuilder::try_new("leak_arg")
                .expect("name")
                .param(quack_rs::types::TypeId::BigInt)
                .returns(quack_rs::types::TypeId::BigInt)
                .bind(inspect_argument)
                .function(pass_through)
                .register(db.con)
                .expect("register leak_arg");
        }
        // `argument` refuses before v1.5.5, so there is no `Expression` to
        // leak there; the query must then fail with that refusal.
        let version = {
            let mut result = con.query("SELECT version()").expect("version");
            let chunk = result.next_chunk().expect("fetch").expect("one row");
            // SAFETY: one VARCHAR row, never NULL.
            unsafe { chunk.reader(0).read_str(0) }.to_owned()
        };
        if quack_rs::abi::parse_version(&version).is_some_and(|v| v >= (1, 5, 5)) {
            results.push((
                "Expression",
                growth(|| drop(con.query("SELECT leak_arg(1)").expect("query"))),
            ));
        } else {
            let err = con.query("SELECT leak_arg(1)").expect_err(&version);
            assert!(
                err.to_string().contains("before v1.5.5"),
                "{version}: {err}"
            );
        }
    }

    #[cfg(feature = "duckdb-1-5")]
    {
        use quack_rs::client_context::ClientContext;
        use quack_rs::file_system::{FileOpenOptions, FileSystem};

        results.push((
            "ClientContext",
            growth(|| {
                drop(unsafe { ClientContext::from_connection(db.con) }.expect("context"));
            }),
        ));
        results.push(("FileOpenOptions", growth(|| drop(FileOpenOptions::new()))));
        let ctx = unsafe { ClientContext::from_connection(db.con) }.expect("context");
        results.push((
            "FileSystem",
            growth(|| drop(FileSystem::from_client_context(&ctx).expect("file system"))),
        ));
        results.push((
            "SelectionVector",
            growth(|| drop(quack_rs::selection_vector::SelectionVector::new(64).expect("sel"))),
        ));
        results.push((
            "InstanceCache",
            growth(|| drop(quack_rs::instance_cache::InstanceCache::new())),
        ));
        db.execute("BEGIN");
        let ctx_in_txn = unsafe { ClientContext::from_connection(db.con) }.expect("context");
        results.push((
            "Catalog",
            growth(|| drop(unsafe { ctx_in_txn.catalog(c"memory") }.expect("catalog"))),
        ));
        drop(ctx_in_txn);
        db.execute("COMMIT");

        // A leaked file handle keeps its descriptor open.
        let fs = FileSystem::from_client_context(&ctx).expect("file system");
        let dir =
            std::env::temp_dir().join(format!("quack_rs_handle_leaks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("f.bin");
        std::fs::write(&path, b"x").expect("seed");
        let c_path = std::ffi::CString::new(path.to_str().expect("utf-8")).expect("no NUL");
        let before = open_files();
        for _ in 0..64 {
            let handle = fs
                .open(&c_path, &FileOpenOptions::read_only())
                .expect("open");
            drop(handle);
        }
        let leaked_fds = open_files().saturating_sub(before);
        // A leaked wrapper keeps its heap memory even when the descriptor is
        // closed.
        let read_only = FileOpenOptions::read_only();
        results.push((
            "FileHandle",
            growth(|| drop(fs.open(&c_path, &read_only).expect("open"))),
        ));
        std::fs::remove_dir_all(&dir).expect("clean up");
        assert_eq!(
            leaked_fds, 0,
            "dropping a FileHandle must close its descriptor"
        );
    }

    eprintln!("heap growth over {ROUNDS} rounds (bound {BOUND}): {results:?}");
    for (name, bytes) in results {
        let bound = if QUERY_SCENARIOS.contains(&name) {
            QUERY_BOUND
        } else {
            BOUND
        };
        assert!(
            bytes < bound,
            "{name} grew the heap by {bytes} bytes over {ROUNDS} rounds"
        );
    }
}
