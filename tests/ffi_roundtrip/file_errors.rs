// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `FileHandle` reports the failures `DuckDB`'s file system reports.
//!
//! `/dev/full` accepts an open for writing, then fails every write with
//! `ENOSPC`, and `fdatasync` on it fails too (a character device cannot be
//! synchronised), so both error paths are reachable without a custom file
//! system. Linux only.

#![cfg(target_os = "linux")]

use quack_rs::client_context::ClientContext;
use quack_rs::file_system::{FileFlag, FileOpenOptions, FileSystem};

use super::Fixture;

#[test]
fn a_failed_write_or_sync_is_an_error_and_an_empty_write_is_not() {
    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let options = FileOpenOptions::new();
    options.set_flag(FileFlag::Write, true);
    let file = fs
        .open(c"/dev/full", &options)
        .expect("open /dev/full for writing");

    assert_eq!(file.write(&[]).expect("an empty write"), 0);
    let err = file
        .write(b"abc")
        .expect_err("/dev/full refuses every byte");
    let message = err.message().unwrap_or_default();
    assert!(message.contains("/dev/full"), "{message}");
    let err = file
        .sync()
        .expect_err("a character device cannot be synced");
    let message = err.message().unwrap_or_default();
    assert!(message.contains("fsync"), "{message}");
    file.close().expect("close");
}
