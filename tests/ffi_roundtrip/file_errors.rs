// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `FileHandle` reports the failures `DuckDB`'s file system reports, and
//! refuses what `DuckDB`'s file system cannot take.

use quack_rs::client_context::ClientContext;
use quack_rs::error_data::DuckDbErrorType;
use quack_rs::file_system::{FileOpenOptions, FileSystem};

use super::Fixture;

/// A scratch directory under the platform's temp dir, removed on drop.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("quack_rs_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }

    /// `name` in the directory, seeded with `contents`, as a C path.
    fn file(&self, name: &str, contents: &[u8]) -> std::ffi::CString {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("seed");
        std::ffi::CString::new(path.to_str().expect("utf-8 path")).expect("no NUL")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A write through a handle opened for reading only fails in the operating
/// system on every platform (`EBADF`, or `ERROR_ACCESS_DENIED` on Windows),
/// and `FileHandle::write` reports it rather than a byte count.
#[test]
fn a_failed_write_is_an_error_and_an_empty_write_is_not() {
    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let scratch = Scratch::new("write");
    let path = scratch.file("f.bin", b"abc");
    let file = fs
        .open(&path, &FileOpenOptions::read_only())
        .expect("open for reading");
    assert_eq!(file.write(&[]).expect("an empty write"), 0);
    file.write(b"xyz")
        .expect_err("a handle opened for reading refuses a write");
    drop(file);
    assert_eq!(
        std::fs::read(scratch.0.join("f.bin")).expect("still there"),
        b"abc"
    );
}

/// `/dev/full` accepts an open for writing, then fails every write with
/// `ENOSPC`, and `fdatasync` on it fails (a character device cannot be
/// synchronised). A failed sync has no portable trigger, hence Linux only.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_or_sync_on_dev_full_is_an_error() {
    use quack_rs::file_system::FileFlag;

    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let options = FileOpenOptions::new();
    options.set_flag(FileFlag::Write, true);
    let file = fs
        .open(c"/dev/full", &options)
        .expect("open /dev/full for writing");

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
/// `i64::MAX` used to be clamped to it: where the file system accepts that
/// offset (tmpfs does) the call returned `Ok` at a position the caller never
/// asked for, and elsewhere it failed with the file system's error. It is
/// now refused before `DuckDB` is called, with `InvalidInput`, which no file
/// system reports for a seek — so this holds, and tells the two apart, on
/// every platform.
#[test]
fn a_seek_past_i64_max_is_an_error_not_a_clamp() {
    let fx = Fixture::open();
    // SAFETY: `con` is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("client context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let scratch = Scratch::new("seek");
    let path = scratch.file("f.bin", b"abcdef");
    let file = fs.open(&path, &FileOpenOptions::read_only()).expect("open");
    for position in [u64::MAX, i64::MAX as u64 + 1] {
        let err = file.seek(position).expect_err("past i64::MAX");
        assert_eq!(
            err.error_type(),
            DuckDbErrorType::InvalidInput,
            "{position}"
        );
        let message = err.message().unwrap_or_default();
        assert!(message.contains("past i64::MAX"), "{position}: {message}");
    }
    file.seek(3).expect("an ordinary seek");
    let mut buf = [0_u8; 3];
    assert_eq!(file.read(&mut buf).expect("read"), 3);
    assert_eq!(&buf, b"def");
}
