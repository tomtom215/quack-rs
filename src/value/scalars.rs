// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The scalar constructors other than the temporal ones (`value/temporal.rs`):
//! numbers, text, `BLOB`, `UUID`, `DECIMAL` and SQL `NULL`.

use std::os::raw::c_char;

use super::{hugeint_from_i128, uhugeint_from_u128, validate_decimal, Value};
use crate::error::ExtensionError;

impl Value {
    // ── Construction ─────────────────────────────────────────────────────
    //
    // Needed for `ConfigOptionBuilder::default_value` and anywhere else DuckDB
    // wants a `duckdb_value` rather than a Rust scalar.

    /// Creates a `BOOLEAN` value.
    #[inline]
    #[must_use]
    pub fn boolean(value: bool) -> Self {
        // SAFETY: duckdb_create_bool accepts any bool and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_bool(value) },
        }
    }

    /// Creates a `BIGINT` value.
    #[inline]
    #[must_use]
    pub fn bigint(value: i64) -> Self {
        // SAFETY: duckdb_create_int64 accepts any i64 and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_int64(value) },
        }
    }

    /// Creates a `DOUBLE` value.
    #[inline]
    #[must_use]
    pub fn double(value: f64) -> Self {
        // SAFETY: duckdb_create_double accepts any f64 and returns an owned value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_double(value) },
        }
    }

    /// Creates a `VARCHAR` value.
    ///
    /// The length is passed explicitly, so no `CString` conversion can fail and
    /// `DuckDB` stores every byte — but note that [`as_str`][Self::as_str] reads
    /// back through a NUL-terminated C string and will truncate at an interior
    /// NUL.
    #[must_use]
    pub fn varchar(value: &str) -> Self {
        // SAFETY: `value` is valid for the duration of the call; DuckDB copies it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_varchar_length(
                value.as_ptr().cast::<c_char>(),
                libduckdb_sys::idx_t::try_from(value.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self { raw }
    }

    /// Creates a `UUID` value from its **textual** 128 bits, matching
    /// [`VectorWriter::write_uuid`][crate::vector::VectorWriter::write_uuid]
    /// and [`as_uuid`][Self::as_uuid].
    ///
    /// `DuckDB` applies its internal top-bit flip itself here, so these are the
    /// bits the value renders as — not the raw `HUGEINT` a `UUID` vector holds.
    #[inline]
    #[must_use]
    pub fn uuid(bits: u128) -> Self {
        let raw = uhugeint_from_u128(bits);
        // SAFETY: duckdb_create_uuid accepts any 128-bit pattern.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_uuid(raw) },
        }
    }

    /// Creates a SQL `NULL` value.
    #[inline]
    #[must_use]
    pub fn null_value() -> Self {
        // SAFETY: duckdb_create_null_value takes no arguments and returns an
        // owned SQLNULL value.
        Self {
            // SAFETY: the argument is a plain value DuckDB accepts unconditionally, and
            // the returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_null_value() },
        }
    }

    // ── The rest of DuckDB's scalar constructors ─────────────────────────
    //
    // Every one of these lives in the *stable* prefix of `duckdb_ext_api_v1`,
    // so they need no feature gate and work against every DuckDB from v1.2.0.

    /// Creates a `TINYINT` value.
    #[inline]
    #[must_use]
    pub fn tinyint(value: i8) -> Self {
        Self {
            // SAFETY: a plain scalar DuckDB accepts unconditionally; the
            // returned handle is owned by this `Value`.
            raw: unsafe { libduckdb_sys::duckdb_create_int8(value) },
        }
    }

    /// Creates a `SMALLINT` value.
    #[inline]
    #[must_use]
    pub fn smallint(value: i16) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_int16(value) },
        }
    }

    /// Creates an `INTEGER` value.
    #[inline]
    #[must_use]
    pub fn integer(value: i32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_int32(value) },
        }
    }

    /// Creates a `UTINYINT` value.
    #[inline]
    #[must_use]
    pub fn utinyint(value: u8) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint8(value) },
        }
    }

    /// Creates a `USMALLINT` value.
    #[inline]
    #[must_use]
    pub fn usmallint(value: u16) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint16(value) },
        }
    }

    /// Creates a `UINTEGER` value.
    #[inline]
    #[must_use]
    pub fn uinteger(value: u32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint32(value) },
        }
    }

    /// Creates a `UBIGINT` value.
    #[inline]
    #[must_use]
    pub fn ubigint(value: u64) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uint64(value) },
        }
    }

    /// Creates a `HUGEINT` value.
    #[inline]
    #[must_use]
    pub fn hugeint(value: i128) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_hugeint(hugeint_from_i128(value)) },
        }
    }

    /// Creates a `UHUGEINT` value.
    #[inline]
    #[must_use]
    pub fn uhugeint(value: u128) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_uhugeint(uhugeint_from_u128(value)) },
        }
    }

    /// Creates a `FLOAT` value.
    #[inline]
    #[must_use]
    pub fn float(value: f32) -> Self {
        Self {
            // SAFETY: see [`tinyint`][Self::tinyint].
            raw: unsafe { libduckdb_sys::duckdb_create_float(value) },
        }
    }

    /// Creates a `BLOB` value from arbitrary bytes.
    ///
    /// Unlike [`varchar`][Self::varchar] this is byte-exact in both directions:
    /// [`as_blob`][Self::as_blob] returns what went in, NUL bytes and all.
    #[must_use]
    pub fn blob(bytes: &[u8]) -> Self {
        // SAFETY: `bytes` is valid for the duration of the call; DuckDB copies it.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_blob(
                bytes.as_ptr(),
                libduckdb_sys::idx_t::try_from(bytes.len()).unwrap_or(libduckdb_sys::idx_t::MAX),
            )
        };
        Self { raw }
    }

    /// Creates a `DECIMAL(width, scale)` value from its unscaled integer.
    ///
    /// `unscaled` is the value multiplied by `10^scale` — `DECIMAL(18, 3)`
    /// holding `1.234` has `unscaled == 1234`.
    ///
    /// # Errors
    ///
    /// - `width` is not in `1..=38`, or `scale > width`;
    /// - `unscaled` has more than `width` digits (`|unscaled| >= 10^width`).
    ///
    /// All three are checked here, before `DuckDB` is called. For an
    /// `unscaled` that does not fit the width's physical type,
    /// `duckdb_create_decimal` throws (aborting the process); for one that
    /// fits the physical type but not the width it stores a value the type
    /// cannot hold; and one past `i64` with `width <= 18` kept only its low
    /// 64 bits.
    pub fn decimal(width: u8, scale: u8, unscaled: i128) -> Result<Self, ExtensionError> {
        validate_decimal(width, scale, unscaled)?;
        // SAFETY: a plain by-value struct, validated above so DuckDB's
        // narrowing `NumericCast` cannot throw.
        let raw = unsafe {
            libduckdb_sys::duckdb_create_decimal(libduckdb_sys::duckdb_decimal {
                width,
                scale,
                value: hugeint_from_i128(unscaled),
            })
        };
        if raw.is_null() {
            return Err(ExtensionError::new(format!(
                "duckdb_create_decimal returned null for DECIMAL({width}, {scale}): \
                 width must be 1..=38 and scale must not exceed width"
            )));
        }
        Ok(Self { raw })
    }
}
