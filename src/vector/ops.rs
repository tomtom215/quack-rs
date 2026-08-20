// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Whole-vector operations (`DuckDB` 1.5.0+).
//!
//! [`SelectionVector`] is `DuckDB`'s
//! zero-copy filtering primitive — and on its own it does nothing. These are the
//! operations that consume one, plus the standalone [`OwnedVector`] that gives a
//! copy somewhere to land.
//!
//! # The one thing to know before slicing
//!
//! [`slice()`] turns its vector into a **dictionary vector**: the payload stays
//! put and the vector gains an indirection through the selection. That is the
//! point — no data is copied — but it also means
//! `duckdb_vector_get_data` no longer maps row `i` to element `i`, so a
//! [`VectorReader`][crate::vector::VectorReader] over a sliced vector reads the
//! wrong rows, silently.
//!
//! quack-rs's readers are flat readers, which is correct everywhere `DuckDB`
//! hands an extension a vector — `CAPIScalarFunction`, `CAPIAggregateUpdate`,
//! the cast bridge and the copy sink all call `Flatten()` on their inputs first
//! (verified in `DuckDB` 1.5.4's `src/main/capi/*.cpp`). [`slice()`] is the one
//! way an extension can produce a non-flat vector for itself, so it is the one
//! place that guarantee has to be re-established by hand.
//!
//! Prefer [`copy_selected()`]: it writes the selected rows into a flat
//! destination, which every reader in this crate can then read normally.
//!
//! # Example: filter without copying the payload
//!
//! ```rust,no_run
//! use quack_rs::selection_vector::SelectionVector;
//! use quack_rs::vector::ops::{copy_selected, OwnedVector};
//! use quack_rs::types::{LogicalType, TypeId};
//!
//! # fn demo(src: libduckdb_sys::duckdb_vector, row_count: usize)
//! # -> Result<(), quack_rs::error::ExtensionError> {
//! // Keep every third row.
//! let kept: Vec<u32> = (0..row_count as u32).step_by(3).collect();
//! let mut sel = SelectionVector::new(kept.len());
//! sel.as_mut_slice().copy_from_slice(&kept);
//!
//! let dst = OwnedVector::new(&LogicalType::new(TypeId::BigInt), kept.len())?;
//! // SAFETY: `src` holds BIGINT, every index in `sel` is a valid `src` row,
//! // and `dst` has room for `kept.len()` of them.
//! unsafe { copy_selected(src, dst.as_raw(), &sel, kept.len(), 0, 0) };
//! # Ok(())
//! # }
//! ```

use libduckdb_sys::{duckdb_vector, idx_t};

use crate::error::ExtensionError;
use crate::selection_vector::SelectionVector;
use crate::types::LogicalType;
use crate::value::Value;

/// A flat `duckdb_vector` this crate allocated, destroyed on drop.
///
/// `DuckDB` normally hands an extension the vectors it should read or write.
/// This is for the cases where an extension needs one of its own: a
/// destination for [`copy_selected`], a staging buffer, or a vector to hand to
/// [`Appender::append_chunk`][crate::appender::Appender::append_chunk].
///
/// Requires `duckdb-1-5`: `duckdb_create_vector` sits in the unstable region of
/// the C API struct.
#[derive(Debug)]
pub struct OwnedVector {
    vector: duckdb_vector,
}

impl OwnedVector {
    /// Allocates a flat vector of `logical_type` with room for `capacity` rows.
    ///
    /// # Errors
    ///
    /// Returns an error if `DuckDB` refuses to allocate the vector — an
    /// `INVALID` or `ANY` type, or a capacity it cannot satisfy.
    pub fn new(logical_type: &LogicalType, capacity: usize) -> Result<Self, ExtensionError> {
        // SAFETY: `logical_type` is a live handle for the duration of the call;
        // DuckDB returns an owned vector or null.
        let vector = unsafe {
            libduckdb_sys::duckdb_create_vector(
                logical_type.as_raw(),
                idx_t::try_from(capacity).unwrap_or(idx_t::MAX),
            )
        };
        if vector.is_null() {
            return Err(ExtensionError::new(
                "duckdb_create_vector returned null: the logical type cannot back a vector \
                 (ANY and INVALID cannot), or the capacity is out of range",
            ));
        }
        Ok(Self { vector })
    }

    /// Returns the raw handle. Do not destroy it — this value still owns it.
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_vector {
        self.vector
    }

    /// Relinquishes ownership, returning the raw handle.
    ///
    /// The caller becomes responsible for `duckdb_destroy_vector`.
    #[must_use]
    pub const fn into_raw(self) -> duckdb_vector {
        let raw = self.vector;
        std::mem::forget(self);
        raw
    }
}

impl Drop for OwnedVector {
    fn drop(&mut self) {
        // SAFETY: `self.vector` was allocated by `duckdb_create_vector` and is
        // destroyed exactly once.
        unsafe { libduckdb_sys::duckdb_destroy_vector(&raw mut self.vector) };
    }
}

/// Copies the rows `sel` names from `src` into `dst`.
///
/// The destination stays **flat**, so the ordinary
/// [`VectorReader`][crate::vector::VectorReader] reads it correctly — which is
/// why this is the filtering primitive to reach for first, ahead of [`slice()`].
///
/// `duckdb.h` on the offsets: `src_count` is "the number of entries from
/// selection vector to copy … the effective length of the selection vector
/// starting from index 0", `src_offset` is "the offset in the selection vector
/// to copy from (important: actual number of items copied = `src_count` -
/// `src_offset`)", and `dst_offset` is where in `dst` to start writing. So to copy
/// all of `sel`, pass `src_count = sel.len()` and `src_offset = 0`.
///
/// # Safety
///
/// - `src` and `dst` must be valid vectors of the **same** logical type.
/// - Every index in `sel[src_offset..src_count]` must be a valid row of `src`.
/// - `dst` must have room for `dst_offset + (src_count - src_offset)` rows.
/// - `src_offset <= src_count <= sel.len()`.
pub unsafe fn copy_selected(
    src: duckdb_vector,
    dst: duckdb_vector,
    sel: &SelectionVector,
    src_count: usize,
    src_offset: usize,
    dst_offset: usize,
) {
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        libduckdb_sys::duckdb_vector_copy_sel(
            src,
            dst,
            sel.as_raw(),
            idx_t::try_from(src_count).unwrap_or(idx_t::MAX),
            idx_t::try_from(src_offset).unwrap_or(idx_t::MAX),
            idx_t::try_from(dst_offset).unwrap_or(idx_t::MAX),
        );
    }
}

/// Reorders or filters `vector` in place through `sel`, without copying its
/// payload.
///
/// # This makes the vector non-flat
///
/// `duckdb.h`: "Turns the vector into a dictionary vector." Row `i` of the
/// result is element `sel[i]` of the original payload, so
/// `duckdb_vector_get_data` — and therefore every
/// [`VectorReader`][crate::vector::VectorReader] in this crate — no longer
/// indexes it correctly. Nothing detects this at run time; the reads are simply
/// wrong.
///
/// Use it when the vector's next stop is `DuckDB` itself, which understands
/// dictionary vectors. When *you* need to read the result, use
/// [`copy_selected`] into a flat destination instead.
///
/// # Safety
///
/// - `vector` must be a valid vector this extension owns or was handed to write.
/// - `len` must not exceed the vector's length, and every index in `sel[..len]`
///   must be a valid row of `vector`.
/// - No [`VectorReader`][crate::vector::VectorReader] or
///   [`VectorWriter`][crate::vector::VectorWriter] over `vector` may be used
///   afterwards: both cache the pre-slice data pointer and both assume flat
///   indexing.
pub unsafe fn slice(vector: duckdb_vector, sel: &SelectionVector, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        libduckdb_sys::duckdb_slice_vector(
            vector,
            sel.as_raw(),
            idx_t::try_from(len).unwrap_or(idx_t::MAX),
        );
    }
}

/// Fills `vector` with a single constant value.
///
/// `duckdb.h`: "Copies the value from `value` to `vector`." This is how an
/// extension emits "the same answer for every row" without writing it row by
/// row, and it is what `DuckDB` does internally for a constant expression.
///
/// # Safety
///
/// - `vector` must be a valid, writable vector.
/// - `value`'s type must match the vector's, or `DuckDB` will reinterpret it.
/// - As with [`slice()`], the result is not a flat vector: do not read it back
///   through a [`VectorReader`][crate::vector::VectorReader].
pub unsafe fn reference_value(vector: duckdb_vector, value: &Value) {
    // SAFETY: forwarded from this function's own contract; `value` outlives the
    // call and DuckDB copies from it.
    unsafe { libduckdb_sys::duckdb_vector_reference_value(vector, value.as_raw()) };
}

/// Makes `to` reference `from`'s data instead of its own.
///
/// `duckdb.h`: "Changes `to_vector` to reference `from_vector`. After, the
/// vectors share ownership of the data." No payload is copied.
///
/// # Safety
///
/// - Both vectors must be valid and of the same logical type.
/// - `from` must outlive every read of `to`.
/// - Any [`VectorReader`][crate::vector::VectorReader] or
///   [`VectorWriter`][crate::vector::VectorWriter] built over `to` before this
///   call caches the old data pointer and must not be used afterwards.
pub unsafe fn reference_vector(to: duckdb_vector, from: duckdb_vector) {
    // SAFETY: forwarded from this function's own contract.
    unsafe { libduckdb_sys::duckdb_vector_reference_vector(to, from) };
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "_duckdb-testing")]
    use super::*;
    #[cfg(feature = "_duckdb-testing")]
    use crate::types::TypeId;

    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn an_owned_vector_round_trips_values() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        let ty = LogicalType::new(TypeId::BigInt);
        let vector = OwnedVector::new(&ty, 16).expect("allocate");
        assert!(!vector.as_raw().is_null());

        // SAFETY: the vector holds BIGINT and has capacity 16.
        let mut writer = unsafe { crate::vector::VectorWriter::from_vector(vector.as_raw()) };
        for row in 0..16 {
            // SAFETY: `row` is within the capacity declared above.
            unsafe { writer.write_i64(row, i64::try_from(row).unwrap() * 10) };
        }
        // SAFETY: the vector holds 16 BIGINT rows written just above.
        let reader = unsafe { crate::vector::VectorReader::from_vector(vector.as_raw(), 16) };
        for row in 0..16 {
            // SAFETY: `row < 16`.
            assert_eq!(
                unsafe { reader.read_i64(row) },
                i64::try_from(row).unwrap() * 10
            );
        }
    }

    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn copy_selected_lands_flat_and_readable() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        let ty = LogicalType::new(TypeId::BigInt);
        let src = OwnedVector::new(&ty, 32).expect("allocate src");
        let dst = OwnedVector::new(&ty, 32).expect("allocate dst");

        // SAFETY: `src` holds 32 BIGINT rows.
        let mut writer = unsafe { crate::vector::VectorWriter::from_vector(src.as_raw()) };
        for row in 0..32 {
            // SAFETY: `row < 32`.
            unsafe { writer.write_i64(row, i64::try_from(row).unwrap()) };
        }

        // Keep the odd rows, in reverse.
        let kept: Vec<u32> = (0..32u32).filter(|i| i % 2 == 1).rev().collect();
        let mut sel = SelectionVector::new(kept.len());
        sel.as_mut_slice().copy_from_slice(&kept);

        // SAFETY: same type, every index is a valid `src` row, `dst` has room.
        unsafe { copy_selected(src.as_raw(), dst.as_raw(), &sel, kept.len(), 0, 0) };

        // SAFETY: `dst` is flat and holds `kept.len()` BIGINT rows.
        let reader = unsafe { crate::vector::VectorReader::from_vector(dst.as_raw(), kept.len()) };
        for (row, &want) in kept.iter().enumerate() {
            // SAFETY: `row < kept.len()`.
            assert_eq!(
                unsafe { reader.read_i64(row) },
                i64::from(want),
                "copy_selected must preserve the selection's order"
            );
        }
    }

    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn creating_a_vector_of_an_unusable_type_is_an_error() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        // ANY cannot back a vector; DuckDB returns null rather than throwing.
        let ty = LogicalType::new(TypeId::Any);
        assert!(OwnedVector::new(&ty, 8).is_err());
    }
}
