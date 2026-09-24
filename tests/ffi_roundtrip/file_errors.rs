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

/// `seek` takes a `u64`, and `DuckDB`'s seek an `i64`. A position past
/// `i64::MAX` used to be clamped to it, which `lseek` accepts, so the call
/// returned `Ok` at a position the caller never asked for.
#[test]
fn a_seek_past_i64_max_is_an_error_not_a_clamp() {
    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    // tmpfs accepts any offset up to `i64::MAX` (`MAX_LFS_FILESIZE`), so a
    // clamped seek succeeds there; other file systems may refuse it.
    let dir =
        std::path::Path::new("/dev/shm").join(format!("quack_rs_seek_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("f.bin");
    std::fs::write(&path, b"abcdef").expect("seed");
    let c_path = std::ffi::CString::new(path.to_str().expect("utf-8")).expect("no NUL");
    let file = fs
        .open(&c_path, &FileOpenOptions::read_only())
        .expect("open");
    for position in [u64::MAX, i64::MAX as u64 + 1] {
        assert!(file.seek(position).is_err(), "{position}");
    }
    file.seek(3).expect("an ordinary seek");
    let mut buf = [0_u8; 3];
    assert_eq!(file.read(&mut buf).expect("read"), 3);
    assert_eq!(&buf, b"def");
    drop(file);
    std::fs::remove_dir_all(&dir).expect("clean up");
}
