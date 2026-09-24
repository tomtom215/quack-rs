// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Grouped-aggregate states the scan never reaches must not leak memory
//! quack-rs allocated.
//!
//! `DuckDB` 1.4.4 to 1.5.5 destroys a grouped aggregate's states as its scan
//! passes them (`RadixHTGlobalSinkState::Destroy` returns early while the
//! pin properties are still `DESTROY_AFTER_DONE`). A `LIMIT`, or an error raised
//! above the aggregate, stops the scan, and the states it did not reach are
//! never destroyed: 2,048 of 300,000 destroyed under `LIMIT 10` on one thread
//! (`docs/upstream-duckdb-reports.md`, item 20). `FfiState<T>` used to box
//! every `T`, so each of those states leaked a heap allocation. It now keeps a
//! small `T` in `DuckDB`'s own state bytes, which `DuckDB` frees with the rest
//! of the hash table.
//!
//! A counting global allocator measures the Rust heap, which `DuckDB`'s own
//! C++ allocations do not touch. This is its own test binary because the
//! allocator is process-wide, and one `#[test]` runs every scenario in
//! sequence so nothing else allocates concurrently.

#![cfg(feature = "_duckdb-testing")]
// Test code: casts at FFI edges and one long scenario list, as in
// `tests/ffi_roundtrip.rs`; each unsafe block's invariant is the test's setup.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::struct_field_names,
    clippy::undocumented_unsafe_blocks
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::data_chunk::DataChunk;
use quack_rs::query::OwnedConnection;
use quack_rs::testing::InMemoryDb;
use quack_rs::types::TypeId;
use quack_rs::vector::VectorWriter;

struct Counting;

/// Bytes currently allocated through the Rust global allocator.
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

fn bytes(layout: Layout) -> isize {
    isize::try_from(layout.size()).unwrap_or(isize::MAX)
}

// SAFETY: every method forwards to `System` with its arguments unchanged and
// only adjusts a counter.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(bytes(layout), Ordering::SeqCst);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(bytes(layout), Ordering::SeqCst);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(bytes(layout), Ordering::SeqCst);
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE_BYTES.fetch_add(
                isize::try_from(new_size).unwrap_or(isize::MAX) - bytes(layout),
                Ordering::SeqCst,
            );
        }
        new
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// A typical small state: no heap of its own.
#[derive(Default)]
struct Sum {
    total: i64,
}
impl AggregateState for Sum {}

/// A state too large to keep in `DuckDB`'s row, so `FfiState` boxes it.
struct Wide {
    totals: [i64; 64],
}
impl Default for Wide {
    fn default() -> Self {
        Self { totals: [0; 64] }
    }
}
impl AggregateState for Wide {}

quack_rs::aggregate_update_callback!(sum_update, |_info, input, states| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if let Some(s) = unsafe { FfiState::<Sum>::with_state_mut(*states.add(row)) } {
            s.total += unsafe { reader.read_i64(row) };
        }
    }
});
quack_rs::aggregate_combine_callback!(sum_combine, |_info, source, target, count| {
    for i in 0..count as usize {
        let v = unsafe { FfiState::<Sum>::with_state(*source.add(i)) }.map_or(0, |s| s.total);
        if let Some(t) = unsafe { FfiState::<Sum>::with_state_mut(*target.add(i)) } {
            t.total += v;
        }
    }
});
quack_rs::aggregate_finalize_callback!(sum_finalize, |_info, source, result, count, offset| {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        match unsafe { FfiState::<Sum>::with_state(*source.add(i)) } {
            Some(s) => unsafe { writer.write_i64(offset as usize + i, s.total) },
            None => unsafe { writer.set_null(offset as usize + i) },
        }
    }
});

quack_rs::aggregate_update_callback!(wide_update, |_info, input, states| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        if let Some(s) = unsafe { FfiState::<Wide>::with_state_mut(*states.add(row)) } {
            s.totals[0] += unsafe { reader.read_i64(row) };
        }
    }
});
quack_rs::aggregate_combine_callback!(wide_combine, |_info, source, target, count| {
    for i in 0..count as usize {
        let v = unsafe { FfiState::<Wide>::with_state(*source.add(i)) }.map_or(0, |s| s.totals[0]);
        if let Some(t) = unsafe { FfiState::<Wide>::with_state_mut(*target.add(i)) } {
            t.totals[0] += v;
        }
    }
});
quack_rs::aggregate_finalize_callback!(wide_finalize, |_info, source, result, count, offset| {
    let mut writer = unsafe { VectorWriter::from_vector(result) };
    for i in 0..count as usize {
        match unsafe { FfiState::<Wide>::with_state(*source.add(i)) } {
            Some(s) => unsafe { writer.write_i64(offset as usize + i, s.totals[0]) },
            None => unsafe { writer.set_null(offset as usize + i) },
        }
    }
});

/// The change in Rust heap bytes across `sql` and the statement after it
/// (which finishes the first one's cleanup).
fn heap_growth(con: &OwnedConnection, sql: &str) -> isize {
    let before = LIVE_BYTES.load(Ordering::SeqCst);
    // The outcome is not the point: the error case fails by design.
    drop(con.execute(sql));
    con.execute("SELECT 1").expect("next statement");
    LIVE_BYTES.load(Ordering::SeqCst) - before
}

#[test]
fn states_a_grouped_scan_never_reaches_leak_no_rust_heap() {
    // Populates the loadable-extension dispatch table from the linked DuckDB.
    let _dispatch = InMemoryDb::open().expect("dispatch table");
    let mut db: libduckdb_sys::duckdb_database = std::ptr::null_mut();
    let mut raw: libduckdb_sys::duckdb_connection = std::ptr::null_mut();
    // SAFETY: a fresh in-memory database and a connection to it.
    unsafe {
        assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &raw mut db), 0);
        assert_eq!(libduckdb_sys::duckdb_connect(db, &raw mut raw), 0);
    }
    // SAFETY: `raw` is open; the callbacks match the builders' signatures.
    unsafe {
        AggregateFunctionBuilder::try_new("leak_sum")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<Sum>::size_callback)
            .init(FfiState::<Sum>::init_callback)
            .update(sum_update)
            .combine(sum_combine)
            .finalize(sum_finalize)
            .destructor(FfiState::<Sum>::destroy_callback)
            .register(raw)
            .expect("register leak_sum");
        AggregateFunctionBuilder::try_new("leak_wide")
            .expect("name")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<Wide>::size_callback)
            .init(FfiState::<Wide>::init_callback)
            .update(wide_update)
            .combine(wide_combine)
            .finalize(wide_finalize)
            .destructor(FfiState::<Wide>::destroy_callback)
            .register(raw)
            .expect("register leak_wide");
    }
    // SAFETY: `db` outlives the connection, which is dropped below.
    let con = unsafe { OwnedConnection::open(db) }.expect("connect");
    con.execute("SET threads = 1").expect("threads");
    let grouped = |agg: &str| {
        format!("(SELECT i % 300000 AS g, {agg}(i) AS s FROM range(1000000) t(i) GROUP BY g)")
    };
    // Warm up: the first query allocates caches that outlive it.
    let _ = heap_growth(
        &con,
        &format!("SELECT count(s) FROM {}", grouped("leak_sum")),
    );

    // Control: a scan that reaches every state destroys every state.
    let whole = heap_growth(
        &con,
        &format!("SELECT count(s) FROM {}", grouped("leak_sum")),
    );
    // 300,000 abandoned boxed `Sum`s would be 2,400,000 bytes.
    let limit = heap_growth(
        &con,
        &format!(
            "SELECT count(s) FROM (SELECT s FROM {} LIMIT 10)",
            grouped("leak_sum")
        ),
    );
    let error = heap_growth(
        &con,
        &format!(
            "SELECT count(CASE WHEN s > -1 AND g = 150000 THEN error('stop') END) FROM {}",
            grouped("leak_sum")
        ),
    );
    // Pins the `DuckDB` defect: a state too large to keep inline is still
    // boxed, and still leaks when the scan stops early. If this starts
    // passing with no growth, `DuckDB` has fixed item 20.
    let wide = heap_growth(
        &con,
        &format!(
            "SELECT count(s) FROM (SELECT s FROM {} LIMIT 10)",
            grouped("leak_wide")
        ),
    );
    eprintln!("Rust heap growth: whole {whole}, LIMIT {limit}, error {error}, boxed LIMIT {wide}");
    let bound = 64 * 1024;
    assert!(
        whole < bound,
        "a completed scan grew the heap by {whole} bytes"
    );
    assert!(
        limit < bound,
        "LIMIT above the aggregate grew the heap by {limit} bytes"
    );
    assert!(
        error < bound,
        "an error above the aggregate grew the heap by {error} bytes"
    );
    let wide_bytes = isize::try_from(core::mem::size_of::<Wide>()).expect("size");
    assert!(
        wide > 100_000 * wide_bytes,
        "boxed states abandoned under LIMIT grew the heap by only {wide} bytes"
    );
    drop(con);
    // SAFETY: both handles are open and nothing uses them afterwards.
    unsafe {
        libduckdb_sys::duckdb_disconnect(&raw mut raw);
        libduckdb_sys::duckdb_close(&raw mut db);
    }
}
