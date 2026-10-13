// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The documented registration closure, `|con| unsafe { register(con) }`,
//! compiles warning-free through both entry-point macros. The macros expand
//! the closure inside their own `unsafe` block, so the closure's `unsafe` used
//! to warn as nested (`unused_unsafe`) and fail an extension's `-D warnings`
//! clippy run — the scaffold's generated CI runs exactly that.

#![deny(warnings)]
// Test code: the stand-ins mirror an extension's real `register` signature.
#![allow(
    clippy::undocumented_unsafe_blocks,
    clippy::missing_const_for_fn,
    clippy::unnecessary_wraps
)]

use libduckdb_sys::duckdb_connection;
use quack_rs::error::ExtensionError;

/// Stands in for an extension's `register` function.
///
/// # Safety
///
/// `con` must be a live connection (never called here).
unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    let _ = con;
    Ok(())
}

/// Stands in for an `entry_point_v2!` registration function.
///
/// # Safety
///
/// `con` must wrap a live connection (never called here).
unsafe fn register_v2(con: &quack_rs::connection::Connection) -> Result<(), ExtensionError> {
    let _ = con;
    Ok(())
}

quack_rs::entry_point!(lint_check_init_c_api, |con| unsafe { register(con) });
quack_rs::entry_point_v2!(lint_check_v2_init_c_api, |con| unsafe { register_v2(con) });

#[test]
fn the_entry_point_macros_accept_an_unsafe_registration_closure_without_warnings() {
    // Compiling this file under `#![deny(warnings)]` is the test; the symbols
    // exist with the C ABI DuckDB looks them up by.
    let v1: unsafe extern "C" fn(_, _) -> bool = lint_check_init_c_api;
    let v2: unsafe extern "C" fn(_, _) -> bool = lint_check_v2_init_c_api;
    assert_ne!(v1 as usize, 0);
    assert_ne!(v2 as usize, 0);
}
