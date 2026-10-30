// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The canary for the `leak-check` CI job: a leaked `DuckDB` handle must make
//! `LeakSanitizer` fail, or the job cannot catch a missing `duckdb_destroy_*`.
//!
//! Run plainly this test passes and leaks nothing. The CI job runs it again
//! under `LeakSanitizer` with `PROBE=logical_type` (and `PROBE=value`) and
//! requires the run to fail with a leak report, and with `PROBE=rust` as the
//! control. A suppression of `libduckdb.so` made the first two exit 0, because
//! `LeakSanitizer` tests a suppression against every frame of an allocation, and
//! every handle `DuckDB` allocates has a `libduckdb.so` frame (`TESTS-1`).

#![cfg(feature = "_duckdb-testing")]

use quack_rs::testing::InMemoryDb;
use quack_rs::types::{LogicalType, TypeId};
use quack_rs::value::Value;

#[test]
fn a_handle_leaked_on_request_is_what_leaksanitizer_reports() {
    let _db = InMemoryDb::open().expect("dispatch table");
    let which = std::env::var("PROBE").unwrap_or_default();
    // Leak on a thread that ends, so no live stack or register keeps the
    // pointer reachable: LeakSanitizer reports only unreachable memory.
    std::thread::spawn(move || match which.as_str() {
        // The control: a leaked Rust allocation, which no suppression covers.
        "rust" => std::mem::forget(Box::new([7_u8; 4096])),
        // A `DuckDB`-allocated handle whose destructor never runs.
        "logical_type" => std::mem::forget(LogicalType::new(TypeId::Integer)),
        "value" => std::mem::forget(Value::bigint(42)),
        _ => {}
    })
    .join()
    .expect("probe thread");
}
