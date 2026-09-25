// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ArrowArray` — an owned Arrow C Data Interface array.

use std::mem::ManuallyDrop;
use std::ptr;

use super::{ArrowArray, RawArrowArray};

impl ArrowArray {
    /// An unfilled record, ready for a producer to write into.
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(RawArrowArray::empty())
    }

    /// Takes ownership of a filled ABI record.
    ///
    /// # Safety
    ///
    /// - `raw` must be a record whose `release` callback this value may call
    ///   exactly once, and no other wrapper may hold the same record.
    /// - It must be a valid Arrow C Data Interface array: `length`, `offset`
    ///   and `null_count` true, and every buffer, child and dictionary pointer
    ///   valid for the lengths and offsets the record and its descendants
    ///   declare. The safe accessors here and [`data_chunk_from_arrow`][super::data_chunk_from_arrow] read
    ///   through those pointers; the Arrow C Data Interface carries no sizes
    ///   that could be checked instead.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: RawArrowArray) -> Self {
        Self(raw)
    }

    /// Moves the record out of `ptr`, leaving a released placeholder behind.
    ///
    /// See [`ArrowSchema::take_from`][super::ArrowSchema::take_from].
    ///
    /// # Safety
    ///
    /// - `ptr` must be valid for reads and writes and properly aligned for
    ///   [`RawArrowArray`].
    /// - The caller must not use the record behind `ptr` afterwards.
    /// - The record behind `ptr` must meet [`from_raw`][Self::from_raw]'s
    ///   contract: this value calls its `release` callback, and the safe
    ///   accessors and `data_chunk_from_arrow` read through its pointers.
    #[must_use]
    pub const unsafe fn take_from(ptr: *mut RawArrowArray) -> Self {
        // SAFETY: `ptr` is valid for reads and writes per this function's contract.
        let raw = unsafe { ptr::read(ptr) };
        // SAFETY: as above; the placeholder has `release == NULL`.
        unsafe { ptr::write(ptr, RawArrowArray::empty()) };
        Self(raw)
    }

    /// Relinquishes ownership, returning the ABI record.
    ///
    /// The caller becomes responsible for calling `release` on it.
    #[must_use]
    pub fn into_raw(self) -> RawArrowArray {
        let this = ManuallyDrop::new(self);
        // SAFETY: `this` is never dropped, so the record is moved out exactly once.
        unsafe { ptr::read(&raw const this.0) }
    }

    /// A pointer to the ABI record, for C APIs that read it.
    #[inline]
    #[must_use]
    pub const fn as_ptr(&self) -> *const RawArrowArray {
        &raw const self.0
    }

    /// A mutable pointer to the ABI record, for C APIs that fill it.
    #[inline]
    #[must_use]
    pub const fn as_mut_ptr(&mut self) -> *mut RawArrowArray {
        &raw mut self.0
    }

    /// Whether `release` is null — either never filled, or already released.
    #[inline]
    #[must_use]
    pub const fn is_released(&self) -> bool {
        self.0.release.is_none()
    }

    /// Releases the array now instead of at drop. Idempotent.
    pub fn release(&mut self) {
        if let Some(release) = self.0.release {
            // SAFETY: `release` came from the producer that filled this record.
            // It runs at most once: the specification has it null itself, and
            // the line after it nulls it for a producer that does not.
            unsafe { release(&raw mut self.0) };
            self.0.release = None;
        }
    }

    /// Number of rows. `0` once released.
    #[must_use]
    pub fn len(&self) -> usize {
        if self.is_released() {
            return 0;
        }
        usize::try_from(self.0.length).unwrap_or(0)
    }

    /// Whether the array carries no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of null entries, or `-1` when the producer did not compute it.
    #[inline]
    #[must_use]
    pub const fn null_count(&self) -> i64 {
        self.0.null_count
    }

    /// Logical offset into the buffers.
    #[inline]
    #[must_use]
    pub const fn offset(&self) -> i64 {
        self.0.offset
    }

    /// Number of child arrays — the column count, for the struct array a
    /// converted `DuckDB` chunk produces. `0` once released.
    #[must_use]
    pub fn child_count(&self) -> usize {
        if self.is_released() {
            return 0;
        }
        usize::try_from(self.0.n_children).unwrap_or(0)
    }

    /// Borrows child array `index`.
    ///
    /// Children are owned by this array's producer and released with it.
    #[must_use]
    pub fn child(&self, index: usize) -> Option<&Self> {
        if index >= self.child_count() || self.0.children.is_null() {
            return None;
        }
        // SAFETY: `children` points to at least `n_children` pointers, and
        // `index` is in range.
        let child = unsafe { *self.0.children.add(index) };
        if child.is_null() {
            return None;
        }
        // SAFETY: `Self` is `#[repr(transparent)]` over `RawArrowArray`, and the
        // child lives as long as this array does.
        Some(unsafe { &*child.cast::<Self>() })
    }
}

impl Drop for ArrowArray {
    fn drop(&mut self) {
        self.release();
    }
}

impl core::fmt::Debug for ArrowArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_released() {
            return f
                .debug_struct("ArrowArray")
                .field("released", &true)
                .finish();
        }
        f.debug_struct("ArrowArray")
            .field("length", &self.0.length)
            .field("null_count", &self.0.null_count)
            .field("offset", &self.0.offset)
            .field("n_buffers", &self.0.n_buffers)
            .field("n_children", &self.0.n_children)
            .finish_non_exhaustive()
    }
}
