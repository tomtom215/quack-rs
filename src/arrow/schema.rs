// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `ArrowSchema` — an owned Arrow C Data Interface schema.

use std::ffi::CStr;
use std::mem::ManuallyDrop;
use std::ptr;

use super::{ArrowSchema, RawArrowSchema};

impl ArrowSchema {
    /// An unfilled record, ready for a producer to write into.
    ///
    /// `release` is null, so dropping one frees nothing.
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(RawArrowSchema::empty())
    }

    /// Takes ownership of a filled ABI record.
    ///
    /// # Safety
    ///
    /// - `raw` must be a record whose `release` callback this value may call
    ///   exactly once, and no other wrapper may hold the same record.
    /// - It must be a valid Arrow C Data Interface schema: `format` and
    ///   `name` null or NUL-terminated, `children` null or pointing at
    ///   `n_children` valid child schemas, recursively. The safe accessors
    ///   ([`format`][Self::format], [`name`][Self::name],
    ///   [`child`][Self::child]) and `schema_from_arrow` read through those
    ///   pointers.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: RawArrowSchema) -> Self {
        Self(raw)
    }

    /// Moves the record out of `ptr`, leaving a released placeholder behind.
    ///
    /// This is how to import from another Arrow binding — arrow-rs's
    /// `FFI_ArrowSchema`, say — without a double free: the source is overwritten
    /// with [`RawArrowSchema::empty`], so its own destructor sees `release ==
    /// NULL` and does nothing.
    ///
    /// # Safety
    ///
    /// - `ptr` must be valid for reads and writes and properly aligned for
    ///   [`RawArrowSchema`], which every Arrow C Data Interface `ArrowSchema` is.
    /// - The caller must not use the record behind `ptr` afterwards, other than
    ///   to drop the (now released) wrapper holding it.
    /// - The record behind `ptr` must meet [`from_raw`][Self::from_raw]'s
    ///   contract: this value calls its `release` callback, and the safe
    ///   accessors read through its pointers.
    #[must_use]
    pub const unsafe fn take_from(ptr: *mut RawArrowSchema) -> Self {
        // SAFETY: `ptr` is valid for reads and writes per this function's contract.
        let raw = unsafe { ptr::read(ptr) };
        // SAFETY: as above; the placeholder has `release == NULL`.
        unsafe { ptr::write(ptr, RawArrowSchema::empty()) };
        Self(raw)
    }

    /// Relinquishes ownership, returning the ABI record.
    ///
    /// The caller becomes responsible for calling `release` on it.
    #[must_use]
    pub fn into_raw(self) -> RawArrowSchema {
        let this = ManuallyDrop::new(self);
        // SAFETY: `this` is never dropped, so the record is moved out exactly once.
        unsafe { ptr::read(&raw const this.0) }
    }

    /// A pointer to the ABI record, for C APIs that read it.
    #[inline]
    #[must_use]
    pub const fn as_ptr(&self) -> *const RawArrowSchema {
        &raw const self.0
    }

    /// A mutable pointer to the ABI record, for C APIs that fill it.
    #[inline]
    #[must_use]
    pub const fn as_mut_ptr(&mut self) -> *mut RawArrowSchema {
        &raw mut self.0
    }

    /// Whether `release` is null — either never filled, or already released.
    #[inline]
    #[must_use]
    pub const fn is_released(&self) -> bool {
        self.0.release.is_none()
    }

    /// Releases the schema now instead of at drop. Idempotent.
    pub fn release(&mut self) {
        if let Some(release) = self.0.release {
            // SAFETY: `release` came from the producer that filled this record,
            // and is called at most once — it nulls itself.
            unsafe { release(&raw mut self.0) };
        }
    }

    /// The Arrow format string, e.g. `"+s"` for the struct at the root of a
    /// converted `DuckDB` schema.
    ///
    /// `None` if the schema is released, the pointer is null, or the string is
    /// not valid UTF-8.
    #[must_use]
    pub fn format(&self) -> Option<&str> {
        if self.is_released() || self.0.format.is_null() {
            return None;
        }
        // SAFETY: a live record's `format` is a null-terminated string owned by
        // the producer and valid for as long as the record is.
        unsafe { CStr::from_ptr(self.0.format) }.to_str().ok()
    }

    /// The column name, or `None` when released, null, or not UTF-8.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        if self.is_released() || self.0.name.is_null() {
            return None;
        }
        // SAFETY: as in `format`.
        unsafe { CStr::from_ptr(self.0.name) }.to_str().ok()
    }

    /// The Arrow flag bits (`ARROW_FLAG_NULLABLE` and friends).
    #[inline]
    #[must_use]
    pub const fn flags(&self) -> i64 {
        self.0.flags
    }

    /// Number of child schemas — the column count, at the root of a converted
    /// `DuckDB` schema. `0` once released.
    #[must_use]
    pub fn child_count(&self) -> usize {
        if self.is_released() {
            return 0;
        }
        usize::try_from(self.0.n_children).unwrap_or(0)
    }

    /// Borrows child schema `index`.
    ///
    /// Children are owned by this schema's producer and released with it, which
    /// is why this borrows rather than handing out an [`ArrowSchema`] that would
    /// try to release them again.
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
        // SAFETY: `Self` is `#[repr(transparent)]` over `RawArrowSchema`, and the
        // child lives as long as this schema does.
        Some(unsafe { &*child.cast::<Self>() })
    }
}

impl Drop for ArrowSchema {
    fn drop(&mut self) {
        self.release();
    }
}

impl core::fmt::Debug for ArrowSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_released() {
            return f
                .debug_struct("ArrowSchema")
                .field("released", &true)
                .finish();
        }
        f.debug_struct("ArrowSchema")
            .field("format", &self.format())
            .field("name", &self.name())
            .field("flags", &self.0.flags)
            .field("n_children", &self.0.n_children)
            .finish_non_exhaustive()
    }
}
