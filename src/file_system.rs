// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! File system access (`DuckDB` 1.5.0+).
//!
//! This module exposes `DuckDB`'s virtual file system (VFS) to extensions, so a
//! custom table function, replacement scan, or copy function can read and write
//! files through the *same* abstraction `DuckDB` uses internally. That means
//! transparently honouring `httpfs` (`s3://`, `http://`), in-memory files, and
//! any other registered file system — rather than reaching for `std::fs` and
//! only ever seeing local disk.
//!
//! # Obtaining a [`FileSystem`]
//!
//! Get one from a [`ClientContext`] (which you can obtain from most function
//! callbacks):
//!
//! ```rust,no_run
//! use quack_rs::client_context::ClientContext;
//! use quack_rs::file_system::{FileOpenOptions, FileSystem};
//!
//! # fn demo(ctx: &ClientContext) -> Option<()> {
//! let fs = FileSystem::from_client_context(ctx)?;
//! let opts = FileOpenOptions::read_only();
//! let handle = fs.open(c"data.csv", &opts).ok()?;
//! let mut buf = vec![0u8; handle.size().max(0) as usize];
//! let _n = handle.read(&mut buf).ok()?;
//! # Some(())
//! # }
//! ```

use std::ffi::CStr;
use std::os::raw::c_void;

use libduckdb_sys::{
    duckdb_client_context_get_file_system, duckdb_create_file_open_options,
    duckdb_destroy_file_handle, duckdb_destroy_file_open_options, duckdb_destroy_file_system,
    duckdb_file_flag, duckdb_file_flag_DUCKDB_FILE_FLAG_APPEND,
    duckdb_file_flag_DUCKDB_FILE_FLAG_CREATE, duckdb_file_flag_DUCKDB_FILE_FLAG_CREATE_NEW,
    duckdb_file_flag_DUCKDB_FILE_FLAG_READ, duckdb_file_flag_DUCKDB_FILE_FLAG_WRITE,
    duckdb_file_handle, duckdb_file_handle_close, duckdb_file_handle_error_data,
    duckdb_file_handle_read, duckdb_file_handle_seek, duckdb_file_handle_size,
    duckdb_file_handle_sync, duckdb_file_handle_tell, duckdb_file_handle_write,
    duckdb_file_open_options, duckdb_file_open_options_set_flag, duckdb_file_system,
    duckdb_file_system_error_data, duckdb_file_system_open, DuckDBSuccess,
};

use crate::client_context::ClientContext;
use crate::error_data::ErrorData;

/// A file-open mode flag, mirroring `duckdb_file_flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileFlag {
    /// Open for reading.
    Read,
    /// Open for writing.
    Write,
    /// Create the file if it does not exist.
    Create,
    /// Create the file, failing if it already exists.
    CreateNew,
    /// Open in append mode.
    Append,
}

impl FileFlag {
    /// Converts to the `DuckDB` C API constant.
    #[must_use]
    const fn to_raw(self) -> duckdb_file_flag {
        match self {
            Self::Read => duckdb_file_flag_DUCKDB_FILE_FLAG_READ,
            Self::Write => duckdb_file_flag_DUCKDB_FILE_FLAG_WRITE,
            Self::Create => duckdb_file_flag_DUCKDB_FILE_FLAG_CREATE,
            Self::CreateNew => duckdb_file_flag_DUCKDB_FILE_FLAG_CREATE_NEW,
            Self::Append => duckdb_file_flag_DUCKDB_FILE_FLAG_APPEND,
        }
    }
}

/// RAII wrapper for `duckdb_file_open_options`.
///
/// Describes how a file should be opened. Automatically destroyed when dropped.
pub struct FileOpenOptions {
    options: duckdb_file_open_options,
}

impl FileOpenOptions {
    /// Creates an empty set of file-open options.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: duckdb_create_file_open_options allocates an owned handle.
        let options = unsafe { duckdb_create_file_open_options() };
        Self { options }
    }

    /// Creates options configured for read-only access.
    #[must_use]
    pub fn read_only() -> Self {
        let opts = Self::new();
        opts.set_flag(FileFlag::Read, true);
        opts
    }

    /// Creates options configured for writing, creating the file if needed.
    #[must_use]
    pub fn write_create() -> Self {
        let opts = Self::new();
        opts.set_flag(FileFlag::Write, true);
        opts.set_flag(FileFlag::Create, true);
        opts
    }

    /// Sets a file-open flag, returning `true` on success.
    pub fn set_flag(&self, flag: FileFlag, value: bool) -> bool {
        if self.options.is_null() {
            return false;
        }
        // SAFETY: self.options is a valid duckdb_file_open_options.
        let state =
            unsafe { duckdb_file_open_options_set_flag(self.options, flag.to_raw(), value) };
        state == DuckDBSuccess
    }

    /// Returns the raw handle without consuming the options.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_file_open_options {
        self.options
    }
}

impl Default for FileOpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FileOpenOptions {
    fn drop(&mut self) {
        if !self.options.is_null() {
            // SAFETY: self.options is a valid handle that we own.
            unsafe { duckdb_destroy_file_open_options(&raw mut self.options) };
        }
    }
}

/// RAII wrapper for a `duckdb_file_system`.
///
/// Automatically destroyed when dropped.
pub struct FileSystem {
    fs: duckdb_file_system,
}

impl FileSystem {
    /// Obtains the file system associated with a [`ClientContext`].
    ///
    /// Returns `None` if `DuckDB` does not provide one.
    #[must_use]
    pub fn from_client_context(context: &ClientContext) -> Option<Self> {
        // SAFETY: context.as_raw() is a valid duckdb_client_context.
        let fs = unsafe { duckdb_client_context_get_file_system(context.as_raw()) };
        if fs.is_null() {
            None
        } else {
            Some(Self { fs })
        }
    }

    /// Wraps a raw `duckdb_file_system` handle, taking ownership.
    ///
    /// # Safety
    ///
    /// `fs` must be a valid, non-null `duckdb_file_system` handle that the caller
    /// no longer manages.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(fs: duckdb_file_system) -> Self {
        Self { fs }
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_file_system {
        self.fs
    }

    /// Opens `path` with the given `options`.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the file cannot be opened.
    pub fn open(&self, path: &CStr, options: &FileOpenOptions) -> Result<FileHandle, ErrorData> {
        let mut handle: duckdb_file_handle = std::ptr::null_mut();
        // SAFETY: self.fs, path, and options.as_raw() are all valid; handle is a
        // valid out-pointer.
        let state = unsafe {
            duckdb_file_system_open(self.fs, path.as_ptr(), options.as_raw(), &raw mut handle)
        };
        if state == DuckDBSuccess && !handle.is_null() {
            // SAFETY: open succeeded, so handle is a valid owned file handle.
            Ok(unsafe { FileHandle::from_raw(handle) })
        } else {
            Err(self.error_data())
        }
    }

    /// Returns the structured error from the most recent failed operation.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        // SAFETY: self.fs is valid; the call returns an owned error data handle.
        let raw = unsafe { duckdb_file_system_error_data(self.fs) };
        // SAFETY: raw is an owned duckdb_error_data (possibly null).
        unsafe { ErrorData::from_raw(raw) }
    }
}

impl Drop for FileSystem {
    fn drop(&mut self) {
        if !self.fs.is_null() {
            // SAFETY: self.fs is a valid handle that we own.
            unsafe { duckdb_destroy_file_system(&raw mut self.fs) };
        }
    }
}

/// RAII wrapper for an open `duckdb_file_handle`.
///
/// Automatically closed and destroyed when dropped.
pub struct FileHandle {
    handle: duckdb_file_handle,
}

impl FileHandle {
    /// Wraps a raw `duckdb_file_handle`, taking ownership.
    ///
    /// # Safety
    ///
    /// `handle` must be a valid, non-null `duckdb_file_handle` that the caller no
    /// longer manages.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(handle: duckdb_file_handle) -> Self {
        Self { handle }
    }

    /// Returns the raw handle.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_file_handle {
        self.handle
    }

    /// Reads up to `buf.len()` bytes into `buf`, returning the number of bytes
    /// read (0 at end of file).
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] on read failure.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, ErrorData> {
        let size = i64::try_from(buf.len()).unwrap_or(i64::MAX);
        // SAFETY: self.handle is valid; buf is writable for `size` bytes.
        let n = unsafe {
            duckdb_file_handle_read(self.handle, buf.as_mut_ptr().cast::<c_void>(), size)
        };
        if n < 0 {
            Err(self.error_data())
        } else {
            Ok(usize::try_from(n).unwrap_or(0))
        }
    }

    /// Writes up to `buf.len()` bytes from `buf`, returning the number written.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] on write failure.
    pub fn write(&self, buf: &[u8]) -> Result<usize, ErrorData> {
        let size = i64::try_from(buf.len()).unwrap_or(i64::MAX);
        // SAFETY: self.handle is valid; buf is readable for `size` bytes.
        let n =
            unsafe { duckdb_file_handle_write(self.handle, buf.as_ptr().cast::<c_void>(), size) };
        if n < 0 {
            Err(self.error_data())
        } else {
            Ok(usize::try_from(n).unwrap_or(0))
        }
    }

    /// Seeks to an absolute byte `position`.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the seek fails.
    pub fn seek(&self, position: u64) -> Result<(), ErrorData> {
        let pos = i64::try_from(position).unwrap_or(i64::MAX);
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_file_handle_seek(self.handle, pos) };
        self.check(state)
    }

    /// Returns the current byte offset within the file.
    #[must_use]
    pub fn tell(&self) -> i64 {
        // SAFETY: self.handle is valid.
        unsafe { duckdb_file_handle_tell(self.handle) }
    }

    /// Returns the total size of the file in bytes.
    #[must_use]
    pub fn size(&self) -> i64 {
        // SAFETY: self.handle is valid.
        unsafe { duckdb_file_handle_size(self.handle) }
    }

    /// Flushes buffered writes to durable storage.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the sync fails.
    pub fn sync(&self) -> Result<(), ErrorData> {
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_file_handle_sync(self.handle) };
        self.check(state)
    }

    /// Closes the file. The handle is still destroyed on drop.
    ///
    /// # Errors
    ///
    /// Returns the structured [`ErrorData`] if the close fails.
    pub fn close(&self) -> Result<(), ErrorData> {
        // SAFETY: self.handle is valid.
        let state = unsafe { duckdb_file_handle_close(self.handle) };
        self.check(state)
    }

    /// Returns the structured error from the most recent failed operation.
    #[must_use]
    pub fn error_data(&self) -> ErrorData {
        // SAFETY: self.handle is valid; the call returns an owned error data.
        let raw = unsafe { duckdb_file_handle_error_data(self.handle) };
        // SAFETY: raw is an owned duckdb_error_data (possibly null).
        unsafe { ErrorData::from_raw(raw) }
    }

    /// Converts a `duckdb_state` into a `Result`, reading the handle's error
    /// data on failure.
    fn check(&self, state: libduckdb_sys::duckdb_state) -> Result<(), ErrorData> {
        if state == DuckDBSuccess {
            Ok(())
        } else {
            Err(self.error_data())
        }
    }
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: self.handle is a valid handle that we own.
            unsafe { duckdb_destroy_file_handle(&raw mut self.handle) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_flag_distinct_raw_values() {
        let flags = [
            FileFlag::Read,
            FileFlag::Write,
            FileFlag::Create,
            FileFlag::CreateNew,
            FileFlag::Append,
        ];
        for (i, a) in flags.iter().enumerate() {
            for b in flags.iter().skip(i + 1) {
                assert_ne!(a.to_raw(), b.to_raw(), "{a:?} and {b:?} share a raw value");
            }
        }
    }
}
