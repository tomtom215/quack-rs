// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The fixed-width numeric `append_*` methods, `append_bool` through `append_u128`.

use libduckdb_sys::{
    duckdb_append_hugeint, duckdb_append_uhugeint, duckdb_hugeint, duckdb_uhugeint,
};

use super::{AppendError, Appender};

/// Generates the fixed-width numeric `append_*` methods, which differ only in
/// the C function they call.
macro_rules! append_scalar {
    ($($(#[$attr:meta])* $name:ident($ty:ty) => $c_fn:ident),* $(,)?) => {
        impl Appender {
            $(
                $(#[$attr])*
                ///
                /// # Errors
                ///
                /// Returns an [`AppendError`] if the append fails.
                pub fn $name(&self, value: $ty) -> Result<(), AppendError> {
                    // SAFETY: self.handle is valid.
                    self.append_one(|| unsafe { libduckdb_sys::$c_fn(self.handle, value) })
                }
            )*
        }
    };
}

append_scalar! {
    /// Appends a `BOOLEAN`.
    append_bool(bool) => duckdb_append_bool,
    /// Appends a `TINYINT`.
    append_i8(i8) => duckdb_append_int8,
    /// Appends a `SMALLINT`.
    append_i16(i16) => duckdb_append_int16,
    /// Appends an `INTEGER`.
    append_i32(i32) => duckdb_append_int32,
    /// Appends a `BIGINT`.
    append_i64(i64) => duckdb_append_int64,
    /// Appends a `UTINYINT`.
    append_u8(u8) => duckdb_append_uint8,
    /// Appends a `USMALLINT`.
    append_u16(u16) => duckdb_append_uint16,
    /// Appends a `UINTEGER`.
    append_u32(u32) => duckdb_append_uint32,
    /// Appends a `UBIGINT`.
    append_u64(u64) => duckdb_append_uint64,
    /// Appends a `FLOAT`.
    append_f32(f32) => duckdb_append_float,
    /// Appends a `DOUBLE`.
    append_f64(f64) => duckdb_append_double,
}

impl Appender {
    /// Appends a `HUGEINT`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_i128(&self, value: i128) -> Result<(), AppendError> {
        let raw = duckdb_hugeint {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            lower: value as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (value >> 64) as i64,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_hugeint(self.handle, raw) })
    }

    /// Appends a `UHUGEINT`.
    ///
    /// # Errors
    ///
    /// Returns an [`AppendError`] if the append fails.
    pub fn append_u128(&self, value: u128) -> Result<(), AppendError> {
        let raw = duckdb_uhugeint {
            #[allow(clippy::cast_possible_truncation)]
            lower: value as u64,
            #[allow(clippy::cast_possible_truncation)]
            upper: (value >> 64) as u64,
        };
        // SAFETY: self.handle is valid.
        self.append_one(|| unsafe { duckdb_append_uhugeint(self.handle, raw) })
    }
}
