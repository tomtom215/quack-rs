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
//! (verified in `DuckDB` 1.5.4's `src/main/capi/*.cpp`), and
//! `duckdb_fetch_chunk` flattens query results. Three operations can make a
//! vector non-flat, and after each the guarantee has to be re-established by
//! hand: [`slice()`] (a dictionary vector), [`reference_value()`] (a constant
//! vector) and [`reference_vector()`] from a non-flat source (the target takes
//! the source's layout). `arrow::data_chunk_from_arrow` also receives non-flat
//! vectors from `DuckDB` — dictionary vectors for dictionary-encoded columns,
//! constant vectors for null-typed ones — and flattens them itself before
//! returning.
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
//! let mut sel = SelectionVector::new(kept.len())?;
//! sel.as_mut_slice().copy_from_slice(&kept);
//!
//! let dst = OwnedVector::new(&LogicalType::new(TypeId::BigInt), kept.len())?;
//! // SAFETY: `src` holds BIGINT, every index in `sel` is a valid `src` row,
//! // and `dst` has room for `kept.len()` of them.
//! unsafe { copy_selected(src, dst.as_raw(), &sel, kept.len(), 0, 0) };
//! # Ok(())
//! # }
//! ```

use libduckdb_sys::{duckdb_logical_type, duckdb_vector, idx_t};

use crate::error::ExtensionError;
use crate::selection_vector::SelectionVector;
use crate::types::LogicalType;
use crate::value::Value;

/// The most elements [`OwnedVector::new`] will ask `DuckDB` to put in one vector.
///
/// This is `DConstants::MAX_VECTOR_SIZE` (2^37), the same ceiling `DuckDB`
/// enforces when a list child vector grows (`VectorListBuffer::Reserve`).
///
/// It applies to the vector itself and to every child vector it creates. An
/// `ARRAY(T, n)` child holds `capacity * n` elements, so an array type lowers
/// the usable row capacity accordingly. Every physical element is at most 16
/// bytes, so a vector within this bound needs at most 2 TiB and its size can
/// never wrap a 64-bit multiply.
///
/// On a target whose `usize` cannot hold 2^37 no allocation can reach the
/// ceiling, so the bound is `usize::MAX` there.
pub const MAX_CAPACITY: usize = crate::vector::list_builder::MAX_CHILD_CAPACITY_USIZE;

/// The largest number of elements any single vector — `ty` itself or a child
/// vector `DuckDB` allocates for it — would hold for a `rows`-row vector of
/// `ty`, or `None` if that number does not fit in a `u64` (or `DuckDB`
/// returned no child type, which it does not for a well-formed type).
///
/// Mirrors `Vector::Initialize` and its buffers in `DuckDB`'s
/// `src/common/types/vector.cpp` / `vector_buffer.cpp`: `STRUCT` and `UNION`
/// children, the `LIST` child and both `MAP` children are created with the
/// parent's capacity; an `ARRAY(T, n)` child with `capacity * n`.
///
/// # Safety
///
/// `ty` must be a valid logical type handle.
unsafe fn max_elements_per_vector(ty: duckdb_logical_type, rows: u64) -> Option<u64> {
    use libduckdb_sys::{
        DUCKDB_TYPE_DUCKDB_TYPE_ARRAY as ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_LIST as LIST,
        DUCKDB_TYPE_DUCKDB_TYPE_MAP as MAP, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT as STRUCT,
        DUCKDB_TYPE_DUCKDB_TYPE_UNION as UNION,
    };
    // SAFETY: `ty` is valid per this function's contract. Each child handle
    // returned below is owned, and `LogicalType::from_raw` destroys it.
    unsafe {
        let child = |raw: duckdb_logical_type, child_rows: u64| {
            if raw.is_null() {
                return None;
            }
            let owned = LogicalType::from_raw(raw);
            max_elements_per_vector(owned.as_raw(), child_rows)
        };
        let mut most = rows;
        match libduckdb_sys::duckdb_get_type_id(ty) {
            ARRAY => {
                let size = libduckdb_sys::duckdb_array_type_array_size(ty);
                let child_rows = rows.checked_mul(size)?;
                most = most.max(child(
                    libduckdb_sys::duckdb_array_type_child_type(ty),
                    child_rows,
                )?);
            }
            LIST => {
                most = most.max(child(libduckdb_sys::duckdb_list_type_child_type(ty), rows)?);
            }
            MAP => {
                most = most.max(child(libduckdb_sys::duckdb_map_type_key_type(ty), rows)?);
                most = most.max(child(libduckdb_sys::duckdb_map_type_value_type(ty), rows)?);
            }
            STRUCT => {
                for i in 0..libduckdb_sys::duckdb_struct_type_child_count(ty) {
                    most = most.max(child(
                        libduckdb_sys::duckdb_struct_type_child_type(ty, i),
                        rows,
                    )?);
                }
            }
            UNION => {
                for i in 0..libduckdb_sys::duckdb_union_type_member_count(ty) {
                    most = most.max(child(
                        libduckdb_sys::duckdb_union_type_member_type(ty, i),
                        rows,
                    )?);
                }
            }
            _ => {}
        }
        Some(most)
    }
}

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
    /// Returns an error, before anything is allocated, if `capacity` — or
    /// `capacity` times the sizes of the `ARRAY` types nested in
    /// `logical_type` — exceeds [`MAX_CAPACITY`]. Also returns an error if
    /// `DuckDB` refuses to allocate the vector: an `INVALID` or `ANY` type, or
    /// an allocation that fails.
    ///
    /// The capacity check is not optional hygiene. `DuckDB` sizes the buffer
    /// as `capacity * element_size` with an unchecked multiply
    /// (`VectorBuffer::CreateStandardVector`, `vector_buffer.cpp`), and an
    /// `ARRAY` child as `capacity * array_size`, also unchecked. A capacity
    /// that wraps gets a small buffer and a success return, and writes inside
    /// the capacity this function reported would then overflow the heap.
    pub fn new(logical_type: &LogicalType, capacity: usize) -> Result<Self, ExtensionError> {
        let rows = u64::try_from(capacity).unwrap_or(u64::MAX);
        // SAFETY: `logical_type` is a live handle for the duration of the call.
        let elements = unsafe { max_elements_per_vector(logical_type.as_raw(), rows) };
        match elements {
            Some(n) if n <= MAX_CAPACITY as u64 => {}
            _ => {
                return Err(ExtensionError::new(format!(
                    "OwnedVector capacity {capacity} is out of range: the vector (or an \
                     ARRAY child, which holds capacity * array_size elements) would exceed \
                     DuckDB's maximum of {MAX_CAPACITY} elements per vector"
                )));
            }
        }
        // SAFETY: `logical_type` is a live handle for the duration of the call;
        // DuckDB returns an owned vector or null. `rows` fits in `idx_t` because
        // it is at most `MAX_CAPACITY`.
        let vector = unsafe { libduckdb_sys::duckdb_create_vector(logical_type.as_raw(), rows) };
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
    // The only effect is a `duckdb_destroy_vector`, which needs a live engine to
    // reach and a leak checker to observe. That is what CI's LeakSanitizer job
    // over the end-to-end suite is for; a `--lib` run cannot see it.
    #[mutants::skip]
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
/// - `sel` must not be modified while `vector` is in use. `DuckDB` copies a
///   `SelectionVector` by sharing its buffer (`selection_vector.hpp`), so the
///   sliced vector reads through `sel`'s indices, not a copy of them: a later
///   write through [`SelectionVector::as_mut_slice`] silently changes which
///   rows `vector` holds. Dropping `sel` is fine — the shared buffer is
///   reference-counted.
///
/// [`SelectionVector::as_mut_slice`]: crate::selection_vector::SelectionVector::as_mut_slice
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
/// vectors share ownership of the data." No payload is copied. That ownership
/// does not always reach the bytes: a column other than the first of a chunk
/// from `arrow::data_chunk_from_arrow` (`duckdb-1-5-4`) points
/// into Arrow buffers that only column 0 keeps alive.
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

    /// `OwnedVector::new` refuses anything above `DuckDB`'s
    /// `DConstants::MAX_VECTOR_SIZE` (2^37 elements) — and nothing below it.
    /// On a target whose `usize` cannot hold 2^37, no allocation can reach
    /// it, so the bound is `usize::MAX` there.
    #[test]
    fn max_capacity_is_duckdbs_max_vector_size() {
        #[cfg(target_pointer_width = "64")]
        assert_eq!(super::MAX_CAPACITY, 137_438_953_472);
        assert_eq!(
            super::MAX_CAPACITY,
            usize::try_from(137_438_953_472_u64).unwrap_or(usize::MAX)
        );
    }
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
        let mut sel = SelectionVector::new(kept.len()).expect("allocate");
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

    /// Regression: `DuckDB` sizes the buffer with an unchecked
    /// `capacity * element_size`, so `(2^60 + 2)` HUGEINT rows (16 bytes each)
    /// wrapped to a 32-byte allocation that `new` reported as success. Writing
    /// rows inside the reported capacity then overflowed the heap into
    /// neighbouring allocations.
    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn a_capacity_whose_byte_size_wraps_is_refused() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        let hugeint = LogicalType::new(TypeId::HugeInt);
        let err = OwnedVector::new(&hugeint, (1_usize << 60) + 2).expect_err("must refuse");
        assert!(err.as_str().contains("out of range"), "{err}");
        assert!(OwnedVector::new(&LogicalType::new(TypeId::BigInt), 1 << 61).is_err());
        assert!(OwnedVector::new(&hugeint, MAX_CAPACITY + 1).is_err());
    }

    /// An `ARRAY(T, n)` child holds `capacity * n` elements, so the bound must
    /// apply to that product, including through a `STRUCT` and a `LIST`.
    #[test]
    #[cfg(feature = "_duckdb-testing")]
    fn an_array_child_counts_against_the_capacity_bound() {
        let _db = crate::testing::InMemoryDb::open().expect("dispatch table");
        let arr = LogicalType::array(TypeId::BigInt, 1000);
        // 2^28 rows * 1000 elements > 2^37.
        assert!(OwnedVector::new(&arr, 1 << 28).is_err());
        assert!(OwnedVector::new(&arr, 2048).is_ok());

        // 99_999 is the largest size `duckdb_create_array_type` accepts.
        let nested = LogicalType::struct_type_from_logical(&[(
            "xs",
            LogicalType::list_from_logical(&LogicalType::array(TypeId::Integer, 99_999)),
        )]);
        // 2^21 rows * 99_999 elements > 2^37, reached through STRUCT -> LIST -> ARRAY.
        assert!(OwnedVector::new(&nested, 1 << 21).is_err());
        assert!(OwnedVector::new(&nested, 4).is_ok());

        // 99_999^4 does not fit in a u64: the element count itself overflows,
        // and even a single row must be refused rather than wrapped.
        let mut deep = LogicalType::array(TypeId::TinyInt, 99_999);
        for _ in 0..3 {
            deep = LogicalType::array_from_logical(&deep, 99_999);
        }
        assert!(OwnedVector::new(&deep, 1).is_err());
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
