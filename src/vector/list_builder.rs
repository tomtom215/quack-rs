// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Safe construction of `LIST` and `MAP` output vectors.
//!
//! # The hazard this exists to remove
//!
//! A `LIST` vector stores a `{ offset, length }` entry per row, plus one flat
//! child vector holding every row's elements end to end. Writing one means:
//!
//! 1. `duckdb_list_vector_reserve` — make room in the child,
//! 2. write the elements into the child,
//! 3. `duckdb_list_vector_set_size` — declare how many are valid,
//! 4. write each parent row's `{ offset, length }` entry.
//!
//! Step 1 is the trap. `duckdb_list_vector_reserve` takes a **total** capacity,
//! not an increment, and when it grows it calls `Vector::Resize`, which
//! **reallocates the child's data buffer**. Any [`VectorWriter`] made from the
//! child before that call now holds a dangling pointer — and the natural way to
//! write a list row by row is to keep one writer and reserve as you go:
//!
//! ```rust,ignore
//! // WRONG: `writer` dangles after the second reserve grows the child.
//! let mut writer = unsafe { ListVector::child_writer(vec) };
//! for (row, items) in rows.iter().enumerate() {
//!     unsafe { ListVector::reserve(vec, total + items.len()) };  // may realloc
//!     for (i, item) in items.iter().enumerate() {
//!         unsafe { writer.write_i64(total + i, *item) };          // use-after-free
//!     }
//!     total += items.len();
//! }
//! ```
//!
//! [`ListBuilder`] re-fetches the child writer after every reserve, tracks the
//! running offset, and writes the parent entries for you, so the sequence cannot
//! be got wrong.
//!
//! # Example
//!
//! ```rust,no_run
//! use quack_rs::vector::ListBuilder;
//!
//! # fn demo(list_vector: libduckdb_sys::duckdb_vector) {
//! let rows = [vec![1_i64, 2, 3], vec![], vec![42]];
//! // SAFETY: `list_vector` is a LIST output vector with BIGINT elements.
//! let mut builder = unsafe { ListBuilder::new(list_vector) };
//! for (row, items) in rows.iter().enumerate() {
//!     // SAFETY: the child vector holds BIGINT, and `row` is in bounds.
//!     unsafe {
//!         builder.push_row(row, items.len(), |writer, base| {
//!             for (i, item) in items.iter().enumerate() {
//!                 writer.write_i64(base + i, *item);
//!             }
//!         });
//!     }
//! }
//! // SAFETY: every element promised by `push_row` was written.
//! unsafe { builder.finish() };
//! # }
//! ```
//!
//! # `MAP`
//!
//! A `MAP` is a `LIST<STRUCT{key, value}>`, so the same builder drives it —
//! [`push_map_row`][ListBuilder::push_map_row] hands the closure a writer for
//! the key child and one for the value child.

use libduckdb_sys::{duckdb_logical_type, duckdb_vector};

use crate::types::{LogicalType, TypeId};
use crate::vector::complex::{ListVector, MapVector, StructVector};
use crate::vector::VectorWriter;

/// `DuckDB`'s ceiling on one buffer of a child vector, **in bytes**
/// (`DConstants::MAX_VECTOR_SIZE`).
///
/// `Vector::Resize` (`vector.cpp`, identical in 1.4.4 and 1.5.5) computes
/// `capacity * element size * multiplier` for every buffer it grows — the
/// child's own, each `STRUCT` field's, and each `ARRAY` child's, whose
/// multiplier is the array size — and throws an `OutOfRangeException` when
/// one exceeds this. `duckdb_list_vector_reserve` does not catch it, so the
/// exception unwinds into Rust and the process aborts ("Rust cannot catch
/// foreign exceptions"). `DuckDB` checks the capacity after rounding it up to
/// a power of two (`VectorListBuffer::Reserve`). The element limit therefore
/// depends on the child's type: 2^34 `BIGINT`s, 2^33 `VARCHAR`s, 2^37
/// `BOOLEAN`s, 2^25 `INTEGER[1000]`s (not 2^37 / 4000).
/// [`ListBuilder`] computes it from the child's type ([`max_child_capacity`])
/// and refuses to make a call past it.
///
/// Until September 2026 this was documented, and applied, as an element
/// count: a `BIGINT` list of 2^34 + 1 elements passed the check and aborted
/// the process.
///
/// Staying below it is necessary, not sufficient: a reservation under the
/// ceiling that the allocator cannot satisfy throws through the same uncaught
/// path. Use [`ListBuilder::with_element_limit`] to bound lengths that come
/// from untrusted input.
///
/// Typed `u64` to match `duckdb::DConstants::MAX_VECTOR_SIZE`, which is
/// `1ULL << 37ULL` — an `idx_t`, not a pointer-sized value. It does not fit in a
/// 32-bit `usize`, so typing it as one made the crate fail to compile for
/// `wasm32`, which is a target `DuckDB`'s own extension CI builds.
pub const MAX_LIST_CHILD_CAPACITY: u64 = 1 << 37;

/// [`MAX_LIST_CHILD_CAPACITY`] clamped to this target's `usize`: no element
/// count can exceed it, whatever the type (`BOOLEAN` and `TINYINT` elements are
/// one byte each).
///
/// On a 64-bit target this is the ceiling itself. On a 32-bit target the
/// ceiling is larger than any allocation `usize` can describe, so `usize::MAX`
/// is the real limit and `DuckDB`'s is unreachable.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the branch above proves the value fits"
)]
pub(crate) const MAX_CHILD_CAPACITY_USIZE: usize = if MAX_LIST_CHILD_CAPACITY > usize::MAX as u64 {
    usize::MAX
} else {
    MAX_LIST_CHILD_CAPACITY as usize
};

/// Bytes one element of a childless type occupies in its vector's data buffer
/// (`GetTypeIdSize` of its physical type), for [`max_child_capacity`].
///
/// `DECIMAL` and `ENUM` count their widest storage (16 and 4 bytes), and a
/// type this crate does not know counts 16, the widest any physical type is,
/// so the limit errs low rather than high. `VARCHAR`, `BLOB`, `BIT`,
/// `BIGNUM` and `GEOMETRY` are 16-byte string records; `LIST` and `MAP` are
/// 16-byte entries (their own children are separate buffers).
#[must_use]
pub(crate) const fn element_bytes(id: Option<TypeId>) -> u64 {
    match id {
        Some(TypeId::Boolean | TypeId::TinyInt | TypeId::UTinyInt) => 1,
        Some(TypeId::SmallInt | TypeId::USmallInt) => 2,
        Some(TypeId::Integer | TypeId::UInteger | TypeId::Float | TypeId::Date | TypeId::Enum) => 4,
        Some(
            TypeId::BigInt
            | TypeId::UBigInt
            | TypeId::Double
            | TypeId::Timestamp
            | TypeId::TimestampTz
            | TypeId::TimestampS
            | TypeId::TimestampMs
            | TypeId::TimestampNs
            | TypeId::Time
            | TypeId::TimeTz
            | TypeId::TimeNs,
        ) => 8,
        _ => 16,
    }
}

/// The most elements a child vector of `bytes_per_element` bytes can be
/// reserved for under [`MAX_LIST_CHILD_CAPACITY`], clamped to `usize`. `None`
/// (the per-element size overflowed `u64`) allows nothing.
///
/// `DuckDB` rounds a reservation up to the next power of two before checking
/// its size (`VectorListBuffer::Reserve`, `vector_buffer.cpp`), so the limit
/// is the largest power of two whose bytes fit, not the ceiling divided by
/// the element size: 2^25 for a 4000-byte `INTEGER[1000]`, where
/// 2^37 / 4000 = 34,359,738 elements would be checked as 2^26.
#[must_use]
pub(crate) const fn capacity_for(bytes_per_element: Option<u64>) -> usize {
    let Some(bytes) = bytes_per_element else {
        return 0;
    };
    let fitting = MAX_LIST_CHILD_CAPACITY / if bytes == 0 { 1 } else { bytes };
    if fitting == 0 {
        return 0;
    }
    // The largest power of two not above `fitting`: a request up to it rounds
    // up to at most it.
    let elements = 1_u64 << (u64::BITS - 1 - fitting.leading_zeros());
    if elements > MAX_CHILD_CAPACITY_USIZE as u64 {
        MAX_CHILD_CAPACITY_USIZE
    } else {
        // The branch above proves it fits.
        #[allow(clippy::cast_possible_truncation)]
        {
            elements as usize
        }
    }
}

/// The widest buffer, in bytes per element, that `Vector::Resize` grows for a
/// vector of type `ty`: its own element size; through `STRUCT` fields and
/// `UNION` members (and a `UNION`'s one-byte tag) the widest of theirs; and
/// through an `ARRAY` its child's times the array size. `None` if that
/// overflows `u64`.
///
/// # Safety
///
/// `ty` must be a live logical type.
unsafe fn resize_bytes_per_element(ty: duckdb_logical_type) -> Option<u64> {
    use libduckdb_sys::{
        duckdb_array_type_array_size, duckdb_array_type_child_type, duckdb_get_type_id,
        duckdb_struct_type_child_count, duckdb_struct_type_child_type,
        duckdb_union_type_member_count, duckdb_union_type_member_type,
        DUCKDB_TYPE_DUCKDB_TYPE_ARRAY as ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT as STRUCT,
        DUCKDB_TYPE_DUCKDB_TYPE_UNION as UNION,
    };
    // SAFETY: `ty` is live per the contract; each child handle below is owned
    // and destroyed when its `LogicalType` drops.
    unsafe {
        let child = |raw: duckdb_logical_type| {
            if raw.is_null() {
                return Some(16);
            }
            let owned = LogicalType::from_raw(raw);
            resize_bytes_per_element(owned.as_raw())
        };
        match duckdb_get_type_id(ty) {
            STRUCT => {
                let mut widest = 0;
                for i in 0..duckdb_struct_type_child_count(ty) {
                    widest = widest.max(child(duckdb_struct_type_child_type(ty, i))?);
                }
                Some(widest)
            }
            UNION => {
                let mut widest = 1;
                for i in 0..duckdb_union_type_member_count(ty) {
                    widest = widest.max(child(duckdb_union_type_member_type(ty, i))?);
                }
                Some(widest)
            }
            ARRAY => duckdb_array_type_array_size(ty)
                .checked_mul(child(duckdb_array_type_child_type(ty))?),
            raw => Some(element_bytes(TypeId::try_from_duckdb_type(raw))),
        }
    }
}

/// The most child elements a `LIST` or `MAP` vector's child can be reserved
/// for without `DuckDB` throwing through the C API.
///
/// That is the largest power of two whose elements fit in
/// [`MAX_LIST_CHILD_CAPACITY`] bytes in the widest buffer the child's type
/// grows (see [`MAX_LIST_CHILD_CAPACITY`]); `DuckDB` rounds a reservation up
/// to a power of two before it checks the size. 2^34 for `BIGINT`, 2^33 for
/// `VARCHAR`, 2^25 for `INTEGER[1000]`.
///
/// [`ListBuilder`] applies it by itself; call this before
/// [`ListVector::reserve`] or [`MapVector::reserve`].
///
/// # Safety
///
/// `vector` must be a valid `LIST` or `MAP` vector.
#[must_use]
pub unsafe fn max_child_capacity(vector: duckdb_vector) -> usize {
    // SAFETY: `vector` is valid per the contract; the returned types are owned
    // and destroyed when their `LogicalType`s drop. `duckdb_list_type_child_type`
    // accepts a MAP too (it returns the key/value STRUCT).
    unsafe {
        let list = LogicalType::from_raw(libduckdb_sys::duckdb_vector_get_column_type(vector));
        let raw = libduckdb_sys::duckdb_list_type_child_type(list.as_raw());
        if raw.is_null() {
            return 0;
        }
        let child = LogicalType::from_raw(raw);
        capacity_for(resize_bytes_per_element(child.as_raw()))
    }
}

/// Incremental builder for a `LIST` (or `MAP`) output vector.
///
/// See the [module docs][crate::vector::list_builder] for why the manual
/// sequence is easy to get wrong.
#[derive(Debug)]
pub struct ListBuilder {
    vector: duckdb_vector,
    /// Whether `written` has been initialised from the child's current size.
    /// Deferred to the first write so that [`new`][Self::new] stays `const`
    /// and makes no `DuckDB` call.
    started: bool,
    /// Elements in the child so far, including any already there when the
    /// builder started — the offset of the next row.
    written: usize,
    /// Capacity most recently requested, so `push_row` only reserves when it
    /// must (each reserve that grows is a reallocation plus a copy).
    reserved: usize,
    /// Most child elements this builder will reserve: [`MAX_LIST_CHILD_CAPACITY`]
    /// unless lowered with [`with_element_limit`][Self::with_element_limit].
    limit: usize,
    /// Set when a requested capacity exceeded `limit`; the builder then writes
    /// every remaining row as NULL rather than letting `DuckDB` throw.
    overflowed: bool,
}

impl ListBuilder {
    /// Starts building into `vector`.
    ///
    /// Rows are **appended** after any elements the vector's child already
    /// holds, so several builders — or several calls of a callback — can fill
    /// one vector in turn. `DuckDB` relies on this: it can call an
    /// aggregate's `finalize` many times on the same result vector with an
    /// increasing `offset`, one row per call (`agg(x ORDER BY y)` finalizes
    /// through `SortedAggregateFunction` that way), and a builder that started
    /// at child offset 0 each time would overwrite the earlier rows' elements.
    /// A fresh output vector has an empty child, so for a scalar function this
    /// makes no difference. The element limit counts the elements already
    /// there.
    ///
    /// # Safety
    ///
    /// `vector` must be a valid, writable `LIST` or `MAP` output vector.
    #[must_use]
    pub const unsafe fn new(vector: duckdb_vector) -> Self {
        Self {
            vector,
            started: false,
            written: 0,
            reserved: 0,
            limit: MAX_CHILD_CAPACITY_USIZE,
            overflowed: false,
        }
    }

    /// Lowers the number of child elements this builder will reserve, in total
    /// across all rows. Values above [`MAX_LIST_CHILD_CAPACITY`] are clamped to
    /// it.
    ///
    /// # Why an extension needs this
    ///
    /// [`max_child_capacity`] is `DuckDB`'s own ceiling for the child's type
    /// (2^37 bytes, rounded to a power-of-two element count), which the
    /// builder applies by itself — far more memory than most machines have. A request below that ceiling
    /// that the allocator cannot satisfy makes `DuckDB` throw from
    /// `duckdb_list_vector_reserve`, which has no `try`/`catch` (`DuckDB`
    /// 1.5.5, `src/main/capi/data_chunk-c.cpp`), so the exception unwinds into
    /// Rust and the process aborts. Nothing in this crate can catch it. When
    /// row lengths come from untrusted input, set a limit that fits in memory;
    /// a row that would exceed it is written as NULL and
    /// [`overflowed`][Self::overflowed] reports it.
    #[must_use]
    pub const fn with_element_limit(mut self, limit: usize) -> Self {
        self.limit = if limit < MAX_CHILD_CAPACITY_USIZE {
            limit
        } else {
            MAX_CHILD_CAPACITY_USIZE
        };
        self
    }

    /// Elements in the child vector so far: those this builder wrote, plus any
    /// the vector already held when the builder started (see
    /// [`new`][Self::new]).
    #[must_use]
    #[inline]
    pub const fn element_count(&self) -> usize {
        self.written
    }

    /// Returns `true` if a row would have taken the child vector past the
    /// element limit ([`max_child_capacity`], or the one set with
    /// [`with_element_limit`][Self::with_element_limit]).
    ///
    /// From that row on, the builder writes nothing into the child: that row
    /// and every later [`push_row`][Self::push_row] /
    /// [`push_map_row`][Self::push_map_row] row is set to NULL, with an empty
    /// list entry. Rows written before it stay valid. Check this before
    /// [`finish`][Self::finish] if the row lengths come from untrusted input,
    /// and report an error if a NULL is not an acceptable answer.
    #[must_use]
    #[inline]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Grows the child vector to hold at least `capacity` elements in total.
    ///
    /// Returns `false` — writing nothing — if `capacity` exceeds the element
    /// limit, or an earlier request already did.
    ///
    /// # Safety
    ///
    /// `self.vector` must still be a valid LIST/MAP vector.
    unsafe fn ensure_capacity(&mut self, capacity: usize) -> bool {
        if self.overflowed {
            return false;
        }
        // `self.limit <= max_child_capacity(self.vector)`, DuckDB's ceiling for
        // this child type, since `start` ran (on a 32-bit target no `usize`
        // can reach it). It is a power of two or the caller's lower limit, so
        // DuckDB's rounding of `target` stays within it.
        if capacity > self.limit {
            self.overflowed = true;
            return false;
        }
        if capacity > self.reserved {
            // Grow geometrically: each reserve that grows reallocates and copies
            // the whole child, so doing it once per row is quadratic.
            let target = capacity.next_power_of_two().min(self.limit);
            // SAFETY: `self.vector` is valid per this function's contract, and
            // `target` is within DuckDB's limit.
            unsafe { ListVector::reserve(self.vector, target) };
            self.reserved = target;
        }
        true
    }

    /// Appends `len` elements for parent row `row_idx`.
    ///
    /// The closure receives a [`VectorWriter`] for the child vector — freshly
    /// obtained *after* the reserve, so it is never stale — and `base`, the
    /// index the row's first element occupies. Write indices `base..base + len`.
    ///
    /// If the row would exceed the element limit — or an earlier row already
    /// did — the closure is not called and the row is written as NULL; see
    /// [`overflowed`][Self::overflowed].
    ///
    /// # Safety
    ///
    /// - `row_idx` must be within the parent vector's capacity.
    /// - The closure must only write child indices in `base..base + len`, using
    ///   accessors matching the child's type.
    pub unsafe fn push_row<F>(&mut self, row_idx: usize, len: usize, write: F)
    where
        F: FnOnce(&mut VectorWriter, usize),
    {
        // SAFETY: `self.vector` is valid per the constructor's contract.
        unsafe { self.start() };
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            // SAFETY: `row_idx` is in bounds per the caller's contract.
            unsafe { self.refuse_row(row_idx) };
            return;
        }
        if len > 0 {
            // SAFETY: the child was just reserved, so this writer is fresh.
            let mut writer = unsafe { ListVector::child_writer(self.vector) };
            write(&mut writer, base);
        }
        // SAFETY: `row_idx` is in bounds per the caller's contract.
        unsafe { ListVector::set_entry(self.vector, row_idx, base as u64, len as u64) };
        self.written = base + len;
    }

    /// Appends `len` key/value pairs for parent row `row_idx` of a `MAP`.
    ///
    /// The closure receives a writer for the key child, a writer for the value
    /// child, and the base index. Write indices `base..base + len` in both.
    ///
    /// Past the element limit the row is written as NULL, exactly as for
    /// [`push_row`][Self::push_row].
    ///
    /// # Safety
    ///
    /// - `self.vector` must be a `MAP` vector.
    /// - `row_idx` must be within the parent vector's capacity.
    /// - The closure must only write child indices in `base..base + len`.
    pub unsafe fn push_map_row<F>(&mut self, row_idx: usize, len: usize, write: F)
    where
        F: FnOnce(&mut VectorWriter, &mut VectorWriter, usize),
    {
        // SAFETY: `self.vector` is valid per the constructor's contract.
        unsafe { self.start() };
        let base = self.written;
        // SAFETY: `self.vector` is valid per the constructor's contract.
        if !unsafe { self.ensure_capacity(base.saturating_add(len)) } {
            // SAFETY: `row_idx` is in bounds per the caller's contract.
            unsafe { self.refuse_row(row_idx) };
            return;
        }
        if len > 0 {
            // SAFETY: the child STRUCT was just reserved, so both writers are
            // fresh. MAP children are always key at field 0, value at field 1.
            let (mut keys, mut values) = unsafe {
                let struct_child = MapVector::struct_child(self.vector);
                (
                    VectorWriter::from_vector(StructVector::get_child(struct_child, 0)),
                    VectorWriter::from_vector(StructVector::get_child(struct_child, 1)),
                )
            };
            write(&mut keys, &mut values, base);
        }
        // SAFETY: `row_idx` is in bounds per the caller's contract.
        unsafe { ListVector::set_entry(self.vector, row_idx, base as u64, len as u64) };
        self.written = base + len;
    }

    /// Picks up the child's current size on first use, so that rows are
    /// appended after elements an earlier builder or callback wrote.
    ///
    /// # Safety
    ///
    /// `self.vector` must be a valid `LIST` or `MAP` vector.
    unsafe fn start(&mut self) {
        if !self.started {
            // SAFETY: forwarded from this function's own contract.
            let existing = unsafe { ListVector::get_size(self.vector) };
            self.written = existing;
            self.reserved = existing;
            // SAFETY: as above. DuckDB's ceiling depends on the child's type.
            self.limit = self.limit.min(unsafe { max_child_capacity(self.vector) });
            self.started = true;
        }
    }

    /// Writes `row_idx` as a NULL with an empty entry.
    ///
    /// The entry matters as much as the NULL: `DuckDB` reuses output vectors
    /// across chunks, so a row whose entry is never written keeps the
    /// `{offset, length}` of an earlier chunk, pointing into child data this
    /// chunk never wrote.
    ///
    /// # Safety
    ///
    /// `row_idx` must be within the parent vector's capacity.
    unsafe fn refuse_row(&mut self, row_idx: usize) {
        // SAFETY: `self.vector` is valid per the constructor's contract and
        // `row_idx` is in bounds per this function's.
        unsafe {
            ListVector::set_entry(self.vector, row_idx, self.written as u64, 0);
            VectorWriter::from_vector(self.vector).set_null(row_idx);
        }
    }

    /// Declares how many child elements are valid, completing the vector.
    ///
    /// Forgetting this leaves `DuckDB` reading a child size of zero, so the
    /// column comes back as empty lists with no error.
    ///
    /// # Safety
    ///
    /// Every element promised by a [`push_row`][Self::push_row] /
    /// [`push_map_row`][Self::push_map_row] call must actually have been
    /// written.
    pub unsafe fn finish(mut self) {
        // SAFETY: `self.vector` is valid per the constructor's contract. A
        // builder that wrote nothing keeps the child's existing size.
        unsafe { self.start() };
        // SAFETY: `self.vector` is valid per the constructor's contract, and
        // `self.written` counts exactly the elements the caller wrote.
        unsafe { ListVector::set_size(self.vector, self.written) };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        capacity_for, element_bytes, ListBuilder, MAX_CHILD_CAPACITY_USIZE, MAX_LIST_CHILD_CAPACITY,
    };
    use crate::types::TypeId;

    #[test]
    fn capacity_is_duckdbs_byte_ceiling_over_the_element_size() {
        assert_eq!(capacity_for(Some(8)), 1 << 34);
        assert_eq!(capacity_for(Some(16)), 1 << 33);
        // `DuckDB` checks the reservation rounded up to a power of two, so the
        // limit is a power of two too: 2^25, not 2^37 / 4000 = 34,359,738.
        assert_eq!(capacity_for(Some(4000)), 1 << 25);
        assert_eq!(capacity_for(Some(1)), MAX_CHILD_CAPACITY_USIZE);
        assert_eq!(capacity_for(Some(0)), MAX_CHILD_CAPACITY_USIZE);
        assert_eq!(
            capacity_for(Some(3)),
            1 << 35,
            "2^37 / 3 rounded down to 2^35"
        );
        assert_eq!(capacity_for(Some(1 << 37)), 1);
        assert_eq!(capacity_for(Some((1 << 37) + 1)), 0);
        assert_eq!(capacity_for(None), 0);
        assert_eq!(capacity_for(Some(u64::MAX)), 0);
    }

    /// `GetTypeIdSize` of each type's physical type.
    #[test]
    fn element_sizes_are_duckdbs_physical_sizes() {
        for (id, bytes) in [
            (TypeId::Boolean, 1),
            (TypeId::TinyInt, 1),
            (TypeId::UTinyInt, 1),
            (TypeId::SmallInt, 2),
            (TypeId::USmallInt, 2),
            (TypeId::Integer, 4),
            (TypeId::UInteger, 4),
            (TypeId::Float, 4),
            (TypeId::Date, 4),
            (TypeId::Enum, 4),
            (TypeId::BigInt, 8),
            (TypeId::UBigInt, 8),
            (TypeId::Double, 8),
            (TypeId::Timestamp, 8),
            (TypeId::TimestampTz, 8),
            (TypeId::TimestampS, 8),
            (TypeId::TimestampMs, 8),
            (TypeId::TimestampNs, 8),
            (TypeId::Time, 8),
            (TypeId::TimeTz, 8),
            (TypeId::TimeNs, 8),
            (TypeId::HugeInt, 16),
            (TypeId::UHugeInt, 16),
            (TypeId::Uuid, 16),
            (TypeId::Interval, 16),
            (TypeId::Decimal, 16),
            (TypeId::Varchar, 16),
            (TypeId::Blob, 16),
            (TypeId::List, 16),
        ] {
            assert_eq!(element_bytes(Some(id)), bytes, "{id:?}");
        }
        assert_eq!(element_bytes(None), 16, "an unknown type counts the widest");
    }

    #[test]
    fn the_element_limit_defaults_to_and_is_clamped_by_duckdbs_ceiling() {
        // SAFETY: `new` and `with_element_limit` only store the pointer; no
        // DuckDB call is made, so a null vector is never dereferenced.
        let fresh = unsafe { ListBuilder::new(std::ptr::null_mut()) };
        assert_eq!(fresh.limit, MAX_CHILD_CAPACITY_USIZE);
        assert!(!fresh.overflowed());
        let lowered = fresh.with_element_limit(100);
        assert_eq!(lowered.limit, 100);
        // SAFETY: as above.
        let raised =
            unsafe { ListBuilder::new(std::ptr::null_mut()) }.with_element_limit(usize::MAX);
        assert_eq!(raised.limit, MAX_CHILD_CAPACITY_USIZE);
    }

    /// A request past the element limit is refused before `DuckDB` is asked
    /// for anything, and the refusal sticks: `overflowed` reports it and every
    /// later request — however small — is refused too, so no row after the
    /// first overflowing one is written.
    #[test]
    fn a_request_past_the_element_limit_is_refused_and_reported() {
        // SAFETY: `ensure_capacity` makes no DuckDB call for a request of 0
        // elements or one past the limit, so the null vector is never used.
        let mut builder = unsafe { ListBuilder::new(std::ptr::null_mut()) }.with_element_limit(10);
        // SAFETY: as above.
        assert!(unsafe { builder.ensure_capacity(0) });
        assert!(!builder.overflowed());
        // SAFETY: as above.
        assert!(!unsafe { builder.ensure_capacity(11) });
        assert!(builder.overflowed());
        // SAFETY: as above; the builder has overflowed, so nothing is reserved.
        assert!(!unsafe { builder.ensure_capacity(0) });
        assert!(builder.overflowed());
        assert_eq!(builder.element_count(), 0);
    }

    #[test]
    fn max_capacity_matches_duckdbs_constant() {
        // duckdb::DConstants::MAX_VECTOR_SIZE = 1ULL << 37ULL. Above this,
        // ListVector::Reserve throws a C++ exception that the C API does not
        // catch, so it must never be reached from Rust.
        assert_eq!(MAX_LIST_CHILD_CAPACITY, 137_438_953_472_u64);
        // The ceiling is an `idx_t`, so it must not be pointer-sized: typing it
        // `usize` made `1 << 37` a const-eval overflow on 32-bit targets and
        // broke the wasm32 build outright.
        assert_eq!(
            MAX_CHILD_CAPACITY_USIZE,
            usize::try_from(MAX_LIST_CHILD_CAPACITY).unwrap_or(usize::MAX)
        );
    }
}
