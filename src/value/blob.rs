// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use libduckdb_sys::{duckdb_blob, duckdb_free, duckdb_get_blob};

use super::Value;
use crate::error::ExtensionError;

fn blob_size(blob: &duckdb_blob) -> Result<Option<usize>, ExtensionError> {
    if blob.data.is_null() {
        return if blob.size == 0 {
            Ok(None)
        } else {
            Err(ExtensionError::new("duckdb_get_blob returned null data"))
        };
    }
    usize::try_from(blob.size)
        .map(Some)
        .map_err(|_| ExtensionError::new("duckdb_get_blob returned an unsupported blob size"))
}

#[mutants::skip] // DuckDB allocator effects are not observable from safe Rust tests.
unsafe fn free_blob_data(data: *mut core::ffi::c_void) {
    if !data.is_null() {
        unsafe { duckdb_free(data) };
    }
}

impl Value {
    /// Extracts the value as an owned `Vec<u8>` (`BLOB`).
    ///
    /// `DuckDB` allocates the blob's backing buffer; this method copies it into
    /// an owned `Vec<u8>` and frees the original with `duckdb_free`. The bytes
    /// are copied without UTF-8 validation.
    ///
    /// # Errors
    ///
    /// Returns `ExtensionError` if the value handle is null, `duckdb_get_blob`
    /// returns a null data pointer for a non-empty blob, or the blob size cannot
    /// be represented by `usize` on the current platform.
    pub fn as_blob(&self) -> Result<Vec<u8>, ExtensionError> {
        if self.raw.is_null() {
            return Err(ExtensionError::new("Value is null"));
        }
        // SAFETY: self.raw is a valid duckdb_value per constructor contract.
        let blob: duckdb_blob = unsafe { duckdb_get_blob(self.raw) };
        let size = match blob_size(&blob) {
            Ok(None) => return Ok(Vec::new()),
            Ok(Some(size)) => size,
            Err(error) => {
                // SAFETY: non-null blob data was allocated by DuckDB. The
                // helper also accepts null for the invalid-pointer error path.
                unsafe { free_blob_data(blob.data) };
                return Err(error);
            }
        };
        // SAFETY: blob.data is a DuckDB-allocated buffer of exactly `blob.size`
        // bytes, valid until we free it below.
        let slice = unsafe { std::slice::from_raw_parts(blob.data.cast::<u8>(), size) };
        let out = slice.to_vec();
        // SAFETY: blob.data was allocated by DuckDB and must be freed with duckdb_free.
        unsafe { free_blob_data(blob.data) };
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_size_null_empty_blob_is_valid() {
        let blob = duckdb_blob {
            data: std::ptr::null_mut(),
            size: 0,
        };
        assert!(matches!(blob_size(&blob), Ok(None)));
    }

    #[test]
    fn blob_size_null_non_empty_blob_is_invalid() {
        let blob = duckdb_blob {
            data: std::ptr::null_mut(),
            size: 1,
        };
        assert!(blob_size(&blob).is_err());
    }

    #[test]
    fn null_value_as_blob_returns_error() {
        let value = unsafe { Value::from_raw(std::ptr::null_mut()) };
        assert!(value.as_blob().is_err());
    }

    #[cfg(feature = "_duckdb-testing")]
    #[test]
    fn blob_value_non_utf8_bytes_round_trip() {
        let _db = crate::testing::InMemoryDb::open().expect("should initialize DuckDB");
        let payload = [
            0x80u8, 0xF0, 0x01, 0x42, 0xFF, 0x00, 0xFE, 0x7F, 0xAA, 0x55, 0xC0, 0xAF, 0x90,
        ];
        let len = u64::try_from(payload.len()).expect("test payload length fits in u64");
        // SAFETY: payload points to `len` readable bytes for the duration of the call.
        let raw = unsafe { libduckdb_sys::duckdb_create_blob(payload.as_ptr(), len) };
        // SAFETY: duckdb_create_blob returns an owned value handle.
        let value = unsafe { Value::from_raw(raw) };

        assert_eq!(value.as_blob().expect("blob should be readable"), payload);
    }

    #[cfg(feature = "_duckdb-testing")]
    #[test]
    fn blob_value_empty_bytes_round_trip() {
        let _db = crate::testing::InMemoryDb::open().expect("should initialize DuckDB");
        let payload = [];
        // SAFETY: payload is readable for zero bytes for the duration of the call.
        let raw = unsafe { libduckdb_sys::duckdb_create_blob(payload.as_ptr(), 0) };
        // SAFETY: duckdb_create_blob returns an owned value handle.
        let value = unsafe { Value::from_raw(raw) };

        assert!(value.as_blob().expect("blob should be readable").is_empty());
    }
}
