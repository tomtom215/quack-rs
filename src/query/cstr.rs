// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The two C-string conversions the `query` module runs everything through.
//!
//! Both are pure Rust — no `DuckDB` call on any path — which is why they live
//! here rather than in `query.rs`: `mutants.toml` excludes that file wholesale,
//! because every function left in it wraps a `DuckDB` C call the `--lib`
//! mutation run cannot reach. These two can be, and are, killed by ordinary
//! unit tests.

use std::ffi::{CStr, CString};

use crate::error::ExtensionError;

/// Builds a `CString` from `sql`, rejecting interior NULs with a useful message.
pub(super) fn to_c_sql(sql: &str) -> Result<CString, ExtensionError> {
    CString::new(sql)
        .map_err(|_| ExtensionError::new("SQL text must not contain an interior NUL byte"))
}

/// Reads a NUL-terminated C string, or `None` if the pointer is null or the
/// bytes are not UTF-8.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated string that stays valid for
/// the duration of the call.
pub(super) unsafe fn c_str_to_owned(ptr: *const std::os::raw::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null and NUL-terminated per the caller's contract.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sql_containing_an_interior_nul() {
        let err = to_c_sql("SELECT 1\0; DROP TABLE t").expect_err("must reject NUL");
        assert!(err.as_str().contains("NUL"), "{err}");
    }

    #[test]
    fn accepts_ordinary_sql() {
        assert_eq!(
            to_c_sql("SELECT 1")
                .expect("valid SQL")
                .to_str()
                .expect("utf8"),
            "SELECT 1"
        );
    }

    #[test]
    fn null_c_string_reads_as_none() {
        // SAFETY: a null pointer is explicitly handled.
        assert_eq!(unsafe { c_str_to_owned(std::ptr::null()) }, None);
    }

    #[test]
    fn a_c_string_reads_back_as_its_contents() {
        let owned = CString::new("SELECT 1").expect("no interior NUL");
        // SAFETY: `owned` outlives the call and is NUL-terminated.
        let read = unsafe { c_str_to_owned(owned.as_ptr()) };
        assert_eq!(read.as_deref(), Some("SELECT 1"));
    }

    #[test]
    fn non_utf8_bytes_read_as_none() {
        // A lone 0xff is not valid UTF-8. DuckDB is supposed to hand back UTF-8,
        // but this is an FFI boundary: the decode must fail closed rather than
        // panic or hand out garbage.
        let owned = CString::new([0xff_u8, 0xfe]).expect("no interior NUL");
        // SAFETY: `owned` outlives the call and is NUL-terminated.
        assert_eq!(unsafe { c_str_to_owned(owned.as_ptr()) }, None);
    }
}
