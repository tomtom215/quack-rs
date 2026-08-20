// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Arrow C Data Interface bridge (`DuckDB` 1.5.0+, `duckdb-1-5-4` feature).
//!
//! `DuckDB` 1.5.0 added a conversion family that moves data between a
//! `duckdb_data_chunk` and the [Arrow C Data Interface] without going through a
//! query result:
//!
//! | `DuckDB` C API | quack-rs |
//! |---|---|
//! | `duckdb_connection_get_arrow_options` | [`ArrowOptions::from_connection`] |
//! | `duckdb_result_get_arrow_options` | [`ArrowOptions::from_result`] |
//! | `duckdb_destroy_arrow_options` | [`ArrowOptions`]'s `Drop` |
//! | `duckdb_to_arrow_schema` | [`to_arrow_schema`] |
//! | `duckdb_data_chunk_to_arrow` | [`data_chunk_to_arrow`] |
//! | `duckdb_schema_from_arrow` | [`schema_from_arrow`] |
//! | `duckdb_data_chunk_from_arrow` | [`data_chunk_from_arrow`] |
//! | `duckdb_destroy_arrow_converted_schema` | [`ArrowConvertedSchema`]'s `Drop` |
//! | `out_schema->release(out_schema)` | [`ArrowSchema`]'s `Drop` |
//! | `out_arrow_array->release(out_arrow_array)` | [`ArrowArray`]'s `Drop` |
//!
//! # No `arrow` crate dependency
//!
//! The Arrow C Data Interface is an ABI, not a library: `ArrowSchema` and
//! `ArrowArray` are plain `#[repr(C)]` records with a `release` callback.
//! `libduckdb-sys` defines them directly (and asserts in its own test suite that
//! they match arrow-rs's `FFI_ArrowSchema` / `FFI_ArrowArray` field-for-field),
//! so quack-rs bridges to Arrow without pulling in `arrow`, and an extension
//! that *does* use arrow-rs can hand values across with a pointer cast — see
//! [Bridging to arrow-rs](#bridging-to-arrow-rs).
//!
//! # Ownership, as `DuckDB` actually implements it
//!
//! Every rule below was read out of `src/main/capi/arrow-c.cpp` and
//! `src/common/arrow/arrow_converter.cpp`, not inferred from the header:
//!
//! - **`duckdb_to_arrow_schema` / `duckdb_data_chunk_to_arrow` fill a
//!   caller-allocated struct.** They never release what was already there, so
//!   the destination must start out empty. Both install `release` **last**,
//!   after every fallible step, so a failed conversion leaves a struct with
//!   `release == NULL` and nothing to free. [`to_arrow_schema`] and
//!   [`data_chunk_to_arrow`] therefore start from a fresh
//!   [`ArrowSchema::empty`] / [`ArrowArray::empty`] and only build the owning
//!   wrapper on success.
//! - **`duckdb_schema_from_arrow` does not take the schema.** It reads it
//!   (`PopulateArrowTableSchema` takes `const ArrowSchema &`) and the caller
//!   still owns it, which is why [`schema_from_arrow`] borrows.
//! - **`duckdb_data_chunk_from_arrow` takes the array.** It sets
//!   `arrow_array->release = nullptr` *before* the conversion loop body, so
//!   ownership moves on the error path too. [`data_chunk_from_arrow`] takes the
//!   [`ArrowArray`] **by value** for exactly that reason. The one case where
//!   `DuckDB` does *not* claim it is a zero-column schema, where the loop never
//!   runs — which is handled by the same code, because the by-value array is
//!   dropped on the way out and its `Drop` releases only if `release` survived.
//!
//! # What this module refuses that `DuckDB` would not
//!
//! `duckdb_data_chunk_from_arrow` indexes `arrow_array->children[i]` once per
//! column in the converted schema, with no bounds check, and dereferences the
//! array without checking whether it has already been released. Both are
//! segfaults rather than errors. [`data_chunk_from_arrow`] checks them first and
//! returns an [`ErrorData`] instead — which is why [`ArrowConvertedSchema`]
//! remembers the column count of the schema it was built from.
//!
//! It also refuses a **zero-row** array. `duckdb_data_chunk_from_arrow` passes
//! `arrow_array->length` through as the chunk's *capacity*
//! (`dchunk->Initialize(alloc, types, length)`), and `VectorCacheBuffer`
//! turns a capacity of zero into `Allocator::AllocateData(0)`, whose
//! `D_ASSERT(size > 0)` aborts a **debug** build of `DuckDB`. A release build
//! allocates nothing and carries on — so whether an empty batch works depends
//! on how the engine happens to have been compiled, which is not a contract
//! worth exposing. Skip empty batches, or build the empty chunk directly with
//! `duckdb_create_data_chunk`, which defaults to a full-size capacity and is
//! unaffected.
//!
//! # What it still cannot check
//!
//! Whether each child array's **buffers** match the type its schema declares.
//! `ArrowToDuckDBConversion::ColumnArrowToDuckDB` reads `array.buffers[1]` for a
//! primitive column without testing `n_buffers` or the pointer, so a child that
//! says `"i"` but carries no data buffer is a null dereference inside `DuckDB`,
//! not an error. (Its sibling `GetValidityMask` *is* guarded — it tests
//! `n_buffers > 0 && buffers[0]` — so the crash comes from the data buffer, not
//! the validity one.) Validating that would mean reimplementing Arrow's layout
//! rules for every format string, so this module does not pretend to: an array
//! handed to [`data_chunk_from_arrow`] must be one a conforming Arrow producer
//! built. Arrays that came from [`data_chunk_to_arrow`], from arrow-rs, or from
//! any other real Arrow implementation qualify.
//!
//! # Example: chunk → Arrow → chunk
//!
//! ```rust,no_run
//! use quack_rs::arrow::{
//!     data_chunk_from_arrow, data_chunk_to_arrow, schema_from_arrow, to_arrow_schema,
//!     ArrowOptions,
//! };
//! use quack_rs::types::{LogicalType, TypeId};
//!
//! # fn demo(
//! #     con: libduckdb_sys::duckdb_connection,
//! #     chunk: &quack_rs::data_chunk::DataChunk,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! // SAFETY: `con` is a live DuckDB connection.
//! let options = unsafe { ArrowOptions::from_connection(con) }?;
//!
//! let id = LogicalType::new(TypeId::Integer);
//! let mut schema = to_arrow_schema(&options, &[("id", &id)])?;
//! let array = data_chunk_to_arrow(&options, chunk)?;
//!
//! // ... hand `schema` / `array` to any Arrow consumer, or convert back:
//! // SAFETY: `con` is a live DuckDB connection.
//! let converted = unsafe { schema_from_arrow(con, &mut schema) }?;
//! // SAFETY: same connection, and `array` was produced against `schema`.
//! let chunk = unsafe { data_chunk_from_arrow(con, array, &converted) }?;
//! assert_eq!(chunk.column_count(), 1);
//! # Ok(())
//! # }
//! ```
//!
//! # Bridging to arrow-rs
//!
//! [`RawArrowSchema`] and [`RawArrowArray`] are the ABI records themselves, so a
//! `*mut FFI_ArrowSchema` and a `*mut RawArrowSchema` address the same bytes:
//!
//! ```rust,ignore
//! // Export a quack-rs array into arrow-rs.
//! let ffi: FFI_ArrowArray = unsafe { std::mem::transmute(array.into_raw()) };
//!
//! // Import an arrow-rs array into quack-rs, neutralising the source so only
//! // one side ever calls `release`.
//! let mut ffi = /* FFI_ArrowArray */;
//! let array = unsafe { ArrowArray::take_from(std::ptr::from_mut(&mut ffi).cast()) };
//! ```
//!
//! [`take_from`][ArrowArray::take_from] is the safer half of that pair: it moves
//! the record out and writes a released placeholder back, so the foreign
//! wrapper's own `Drop` becomes a no-op instead of a double free.
//!
//! # Thread safety
//!
//! None of these types are `Send` or `Sync`. The Arrow C Data Interface says
//! nothing about which thread may call `release`, and `duckdb_arrow_options`
//! wraps a `ClientProperties` that borrows the connection's client context.
//!
//! [Arrow C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html

use std::ffi::{CStr, CString};
use std::mem::ManuallyDrop;
use std::os::raw::c_char;
use std::ptr;

use libduckdb_sys::{
    duckdb_arrow_converted_schema, duckdb_arrow_options, duckdb_connection,
    duckdb_connection_get_arrow_options, duckdb_data_chunk, duckdb_data_chunk_from_arrow,
    duckdb_data_chunk_to_arrow, duckdb_destroy_arrow_converted_schema,
    duckdb_destroy_arrow_options, duckdb_logical_type, duckdb_result_get_arrow_options,
    duckdb_schema_from_arrow, duckdb_to_arrow_schema, idx_t,
};

use crate::data_chunk::DataChunk;
use crate::error::ExtensionError;
use crate::error_data::{DuckDbErrorType, ErrorData};
use crate::query::{OwnedDataChunk, QueryResult};
use crate::types::LogicalType;

/// The Arrow C Data Interface `ArrowSchema` ABI record, re-exported from
/// `libduckdb-sys`.
///
/// Field-for-field identical to arrow-rs's `FFI_ArrowSchema`. Prefer the owning
/// [`ArrowSchema`] wrapper; this is here for interop with code that already
/// speaks the raw ABI.
pub use libduckdb_sys::ArrowSchema as RawArrowSchema;

/// The Arrow C Data Interface `ArrowArray` ABI record, re-exported from
/// `libduckdb-sys`.
///
/// Field-for-field identical to arrow-rs's `FFI_ArrowArray`. Prefer the owning
/// [`ArrowArray`] wrapper; this is here for interop with code that already
/// speaks the raw ABI.
pub use libduckdb_sys::ArrowArray as RawArrowArray;

// `libduckdb-sys` below 1.10504.0 declares both records as opaque zero-sized
// bindgen placeholders (`_unused: [u8; 0]`), which cannot be allocated — so the
// whole Arrow C Data Interface is unusable there. Fail with a sentence that says
// so, rather than a pile of "no method named `empty`" errors.
const _: () = assert!(
    size_of::<RawArrowSchema>() > 0 && size_of::<RawArrowArray>() > 0,
    "the `duckdb-1-5-4` feature requires libduckdb-sys >= 1.10504.0: earlier versions declare \
     ArrowSchema/ArrowArray as opaque zero-sized types, so the caller-allocated structs the \
     Arrow C Data Interface requires cannot be created"
);

// ─── Arrow options ───────────────────────────────────────────────────────────

/// The Arrow production settings of a connection or a result.
///
/// `DuckDB` needs these to decide how to render its types as Arrow — the
/// timezone to stamp on `TIMESTAMPTZ`, whether to emit large or regular string
/// offsets, which extension types are registered. Both
/// [`to_arrow_schema`] and [`data_chunk_to_arrow`] require one.
///
/// Destroyed on drop (`duckdb_destroy_arrow_options`).
pub struct ArrowOptions {
    raw: duckdb_arrow_options,
}

impl ArrowOptions {
    /// Reads the Arrow options of a connection.
    ///
    /// # Errors
    ///
    /// `duckdb_connection_get_arrow_options` has no error channel: it writes
    /// null when the connection is null or the allocation throws. That is
    /// reported here as an [`ExtensionError`].
    ///
    /// # Safety
    ///
    /// `connection` must be a live `duckdb_connection`.
    #[mutants::skip] // FFI wrapper — needs a live DuckDB connection
    pub unsafe fn from_connection(connection: duckdb_connection) -> Result<Self, ExtensionError> {
        let mut raw: duckdb_arrow_options = ptr::null_mut();
        // SAFETY: `connection` is live per this function's contract and `raw` is
        // a valid out-parameter.
        unsafe { duckdb_connection_get_arrow_options(connection, &raw mut raw) };
        if raw.is_null() {
            return Err(ExtensionError::new(
                "duckdb_connection_get_arrow_options returned null: the connection was null or \
                 DuckDB could not allocate its client properties",
            ));
        }
        Ok(Self { raw })
    }

    /// Reads the Arrow options a result was produced with.
    ///
    /// Prefer this over [`from_connection`][Self::from_connection] when
    /// exporting that result's chunks: a result carries the client properties
    /// captured when it ran, which is what its chunks were built against.
    ///
    /// # Errors
    ///
    /// Returns an [`ExtensionError`] when `DuckDB` returns null, which it does
    /// for a result with no internal data.
    #[mutants::skip] // FFI wrapper — needs a live DuckDB result
    pub fn from_result(result: &QueryResult) -> Result<Self, ExtensionError> {
        // `duckdb_result_get_arrow_options` takes a `duckdb_result *` but only
        // reads `internal_data`, so a copy of the POD struct is enough — the
        // same pattern every accessor in `crate::query` uses.
        let mut copy = *result.as_raw();
        // SAFETY: `copy` aliases a live result owned by `result`; DuckDB only
        // reads through the pointer.
        let raw = unsafe { duckdb_result_get_arrow_options(&raw mut copy) };
        if raw.is_null() {
            return Err(ExtensionError::new(
                "duckdb_result_get_arrow_options returned null: the result carries no data",
            ));
        }
        Ok(Self { raw })
    }

    /// Takes ownership of a raw `duckdb_arrow_options`.
    ///
    /// # Safety
    ///
    /// `raw` must be a non-null handle the caller is responsible for
    /// destroying, and nobody else may destroy it.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_arrow_options) -> Self {
        Self { raw }
    }

    /// The raw handle, still owned by this value.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_arrow_options {
        self.raw
    }

    /// Relinquishes ownership, returning the raw handle.
    ///
    /// The caller becomes responsible for `duckdb_destroy_arrow_options`.
    #[inline]
    #[must_use]
    pub const fn into_raw(self) -> duckdb_arrow_options {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }
}

impl Drop for ArrowOptions {
    #[mutants::skip] // frees a DuckDB handle; nothing observable without a runtime
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        // SAFETY: `self.raw` was owned by this value and is destroyed once;
        // DuckDB nulls it.
        unsafe { duckdb_destroy_arrow_options(&raw mut self.raw) };
    }
}

impl core::fmt::Debug for ArrowOptions {
    #[mutants::skip] // Debug rendering is not a behavioural contract
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArrowOptions")
            .field("raw", &self.raw)
            .finish()
    }
}

// ─── Arrow schema ────────────────────────────────────────────────────────────

/// An owned Arrow C Data Interface schema.
///
/// Released on drop, unless it was already released or moved out with
/// [`into_raw`][Self::into_raw].
///
/// # Reading a released schema
///
/// The Arrow specification says that once `release` has run, every other field
/// of the record is undefined — `DuckDB`'s own release callback frees the block
/// that `format`, `name` and `children` point into. So [`format`][Self::format],
/// [`name`][Self::name] and [`child`][Self::child] all return `None` once
/// [`is_released`][Self::is_released] is true, rather than handing out a
/// dangling pointer.
#[repr(transparent)]
pub struct ArrowSchema(RawArrowSchema);

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
    /// `raw` must be a record whose `release` callback this value may call
    /// exactly once, and no other wrapper may hold the same record.
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

// ─── Arrow array ─────────────────────────────────────────────────────────────

/// An owned Arrow C Data Interface array.
///
/// Released on drop, unless it was already released, moved out with
/// [`into_raw`][Self::into_raw], or handed to
/// [`data_chunk_from_arrow`] — which is why that function takes it by value.
///
/// As with [`ArrowSchema`], every accessor returns a neutral value once
/// [`is_released`][Self::is_released] is true: the specification leaves the
/// other fields undefined after release.
#[repr(transparent)]
pub struct ArrowArray(RawArrowArray);

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
    /// `raw` must be a record whose `release` callback this value may call
    /// exactly once, and no other wrapper may hold the same record.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: RawArrowArray) -> Self {
        Self(raw)
    }

    /// Moves the record out of `ptr`, leaving a released placeholder behind.
    ///
    /// See [`ArrowSchema::take_from`].
    ///
    /// # Safety
    ///
    /// - `ptr` must be valid for reads and writes and properly aligned for
    ///   [`RawArrowArray`].
    /// - The caller must not use the record behind `ptr` afterwards.
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
            // SAFETY: `release` came from the producer that filled this record,
            // and is called at most once — it nulls itself.
            unsafe { release(&raw mut self.0) };
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

// ─── Converted schema ────────────────────────────────────────────────────────

/// An Arrow schema translated into `DuckDB`'s own type descriptors.
///
/// Produced by [`schema_from_arrow`] and consumed by [`data_chunk_from_arrow`].
/// Destroyed on drop (`duckdb_destroy_arrow_converted_schema`).
///
/// # Why it remembers a column count
///
/// The C API exposes no accessor for how many columns a converted schema
/// describes, yet `duckdb_data_chunk_from_arrow` walks
/// `arrow_array->children[i]` once per column with no bounds check. Recording
/// the source schema's `n_children` — which is exactly what
/// `PopulateArrowTableSchema` iterates — lets [`data_chunk_from_arrow`] reject a
/// mismatched array instead of reading past its children.
pub struct ArrowConvertedSchema {
    raw: duckdb_arrow_converted_schema,
    column_count: usize,
}

impl ArrowConvertedSchema {
    /// Takes ownership of a raw converted schema.
    ///
    /// # Safety
    ///
    /// - `raw` must be a non-null handle the caller is responsible for
    ///   destroying, and nobody else may destroy it.
    /// - `column_count` must be the number of columns `raw` describes, i.e. the
    ///   `n_children` of the Arrow schema it was built from. A wrong value
    ///   defeats the bounds check in [`data_chunk_from_arrow`].
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw(raw: duckdb_arrow_converted_schema, column_count: usize) -> Self {
        Self { raw, column_count }
    }

    /// The raw handle, still owned by this value.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> duckdb_arrow_converted_schema {
        self.raw
    }

    /// How many columns this schema describes.
    #[inline]
    #[must_use]
    pub const fn column_count(&self) -> usize {
        self.column_count
    }
}

impl Drop for ArrowConvertedSchema {
    #[mutants::skip] // frees a DuckDB handle; nothing observable without a runtime
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        // SAFETY: `self.raw` was owned by this value and is destroyed once;
        // DuckDB nulls it.
        unsafe { duckdb_destroy_arrow_converted_schema(&raw mut self.raw) };
    }
}

impl core::fmt::Debug for ArrowConvertedSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArrowConvertedSchema")
            .field("raw", &self.raw)
            .field("column_count", &self.column_count)
            .finish()
    }
}

// ─── Conversions ─────────────────────────────────────────────────────────────

/// Renders a `DuckDB` schema as an Arrow schema
/// (`duckdb_to_arrow_schema`).
///
/// `columns` pairs each column's name with its logical type, in order. The
/// result is a struct schema (`"+s"`) with one child per column — the shape
/// every Arrow consumer expects for a record batch.
///
/// # Errors
///
/// - [`DuckDbErrorType::InvalidInput`] if a name contains an interior NUL byte,
///   since `DuckDB` reads names as C strings.
/// - [`DuckDbErrorType::OutOfRange`] if there are more columns than `idx_t` can
///   hold.
/// - Whatever `DuckDB` reports for a type it cannot render as Arrow.
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub fn to_arrow_schema(
    options: &ArrowOptions,
    columns: &[(&str, &LogicalType)],
) -> Result<ArrowSchema, ErrorData> {
    let column_count = idx_t::try_from(columns.len()).map_err(|_| {
        ErrorData::new(
            DuckDbErrorType::OutOfRange,
            "to_arrow_schema: more columns than idx_t can represent",
        )
    })?;

    let mut names: Vec<CString> = Vec::with_capacity(columns.len());
    for (name, _) in columns {
        let owned = CString::new(*name).map_err(|_| {
            ErrorData::new(
                DuckDbErrorType::InvalidInput,
                &format!(
                    "to_arrow_schema: column name {name:?} contains an interior NUL byte, but \
                     DuckDB reads schema names as C strings"
                ),
            )
        })?;
        names.push(owned);
    }
    let mut name_ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
    let mut types: Vec<duckdb_logical_type> = columns.iter().map(|(_, ty)| ty.as_raw()).collect();

    let mut out = RawArrowSchema::empty();
    // SAFETY: `options` owns a live handle; `types` and `name_ptrs` are each
    // valid for `column_count` elements and outlive the call; `out` is a fresh
    // record DuckDB may fill. DuckDB copies the names and the types.
    let raw_err = unsafe {
        duckdb_to_arrow_schema(
            options.as_raw(),
            types.as_mut_ptr(),
            name_ptrs.as_mut_ptr(),
            column_count,
            &raw mut out,
        )
    };
    // SAFETY: the return value is an owned `duckdb_error_data`, or null.
    let err = unsafe { ErrorData::from_raw(raw_err) };
    if err.has_error() {
        // `ArrowConverter::ToArrowSchema` installs `out.release` as its last
        // statement, after everything that can throw, so a failed call leaves
        // `release == NULL` and there is nothing here to free. Dropping the
        // record without releasing it is therefore correct, not a leak.
        return Err(err);
    }
    // SAFETY: DuckDB filled the record and installed its release callback.
    Ok(unsafe { ArrowSchema::from_raw(out) })
}

/// Exports a `DuckDB` data chunk as an Arrow struct array
/// (`duckdb_data_chunk_to_arrow`).
///
/// The chunk is only read; it stays valid and owned by the caller. The returned
/// array owns copies of the data, so it outlives the chunk.
///
/// Pair it with the schema [`to_arrow_schema`] produces from the same column
/// types and the same [`ArrowOptions`] — an Arrow consumer needs both.
///
/// # Errors
///
/// Whatever `DuckDB` reports for a type it cannot render as Arrow.
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub fn data_chunk_to_arrow(
    options: &ArrowOptions,
    chunk: &DataChunk,
) -> Result<ArrowArray, ErrorData> {
    let mut out = RawArrowArray::empty();
    // SAFETY: `options` owns a live handle, `chunk` is valid per `DataChunk`'s
    // constructor contract, and `out` is a fresh record DuckDB may fill.
    let raw_err =
        unsafe { duckdb_data_chunk_to_arrow(options.as_raw(), chunk.as_raw(), &raw mut out) };
    // SAFETY: the return value is an owned `duckdb_error_data`, or null.
    let err = unsafe { ErrorData::from_raw(raw_err) };
    if err.has_error() {
        // `ArrowConverter::ToArrowArray` assigns `*out_array` as its last
        // statement, so a failed call never wrote to `out` and there is nothing
        // to release.
        return Err(err);
    }
    // SAFETY: DuckDB filled the record and installed its release callback.
    Ok(unsafe { ArrowArray::from_raw(out) })
}

/// Translates an Arrow schema into `DuckDB` type descriptors
/// (`duckdb_schema_from_arrow`).
///
/// `schema` is read, not consumed: the caller still owns it and it is released
/// when its [`ArrowSchema`] drops. It is taken by `&mut` because the C signature
/// is `struct ArrowSchema *`, even though `PopulateArrowTableSchema` receives it
/// as `const ArrowSchema &`.
///
/// `schema` must be the struct schema at the root of a record batch: `DuckDB`
/// makes one column per child.
///
/// # Errors
///
/// - [`DuckDbErrorType::InvalidInput`] if `schema` has already been released,
///   which `DuckDB` would dereference rather than diagnose.
/// - Whatever `DuckDB` reports for an Arrow type it cannot map — a released
///   child schema, or an unsupported format string.
///
/// # Safety
///
/// `connection` must be a live `duckdb_connection`.
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub unsafe fn schema_from_arrow(
    connection: duckdb_connection,
    schema: &mut ArrowSchema,
) -> Result<ArrowConvertedSchema, ErrorData> {
    if schema.is_released() {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            "schema_from_arrow: the Arrow schema has already been released, so its children \
             pointer no longer refers to live memory",
        ));
    }
    // `PopulateArrowTableSchema` adds exactly one column per child of the root
    // schema, so this is the converted schema's column count.
    let column_count = schema.child_count();

    let mut out: duckdb_arrow_converted_schema = ptr::null_mut();
    // SAFETY: `connection` is live per this function's contract, `schema` points
    // at a live record, and `out` is a valid out-parameter.
    let raw_err =
        unsafe { duckdb_schema_from_arrow(connection, schema.as_mut_ptr(), &raw mut out) };
    // SAFETY: the return value is an owned `duckdb_error_data`, or null.
    let err = unsafe { ErrorData::from_raw(raw_err) };
    if err.has_error() {
        return Err(err);
    }
    if out.is_null() {
        return Err(ErrorData::new(
            DuckDbErrorType::Internal,
            "duckdb_schema_from_arrow reported success but produced no converted schema",
        ));
    }
    // SAFETY: `out` is a fresh handle this value now owns, and `column_count`
    // is the child count of the schema DuckDB just walked.
    Ok(unsafe { ArrowConvertedSchema::from_raw(out, column_count) })
}

/// Imports an Arrow struct array as a `DuckDB` data chunk
/// (`duckdb_data_chunk_from_arrow`).
///
/// # Why `array` is taken by value
///
/// `DuckDB` claims the array: it copies the record into the chunk's owned data
/// and then sets `arrow_array->release = nullptr` — *before* the conversion
/// loop body, so the transfer happens on the error path too. Taking it by value
/// makes that visible in the signature, and the by-value binding is dropped on
/// the way out, which releases the array in the one case where `DuckDB` does not
/// claim it (a zero-column converted schema, where the loop never runs).
///
/// The resulting chunk keeps the Arrow buffers alive, so the data is shared, not
/// copied.
///
/// # Errors
///
/// - [`DuckDbErrorType::InvalidInput`] if `array` has already been released, or
///   if its child count does not match `converted`'s column count. `DuckDB`
///   checks neither and would read out of bounds.
/// - Whatever `DuckDB` reports for a layout it cannot convert — dictionary and
///   run-end encoded children are rejected with
///   [`DuckDbErrorType::NotImplemented`].
///
/// # Safety
///
/// `connection` must be a live `duckdb_connection`, and `converted` must have
/// been produced from it (or from another connection of the same database).
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub unsafe fn data_chunk_from_arrow(
    connection: duckdb_connection,
    mut array: ArrowArray,
    converted: &ArrowConvertedSchema,
) -> Result<OwnedDataChunk, ErrorData> {
    if array.is_released() {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            "data_chunk_from_arrow: the Arrow array has already been released",
        ));
    }
    let children = array.child_count();
    if children != converted.column_count() {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            &format!(
                "data_chunk_from_arrow: the Arrow array has {children} child array(s) but the \
                 converted schema describes {} column(s). DuckDB indexes \
                 `arrow_array->children[i]` once per schema column without a bounds check, so \
                 this is refused here rather than read out of bounds.",
                converted.column_count(),
            ),
        ));
    }
    if array.is_empty() {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            "data_chunk_from_arrow: DuckDB cannot import a zero-row Arrow array. It passes \
             `arrow_array->length` straight through as the chunk's *capacity* \
             (`dchunk->Initialize(alloc, types, length)`), and a capacity of zero reaches \
             `Allocator::AllocateData(0)`, whose `D_ASSERT(size > 0)` aborts a debug build of \
             DuckDB. A release build allocates nothing and carries on, so this is refused here \
             rather than left to depend on how the engine was compiled. Skip empty batches, or \
             build the empty chunk directly with `duckdb_create_data_chunk`.",
        ));
    }

    let mut out: duckdb_data_chunk = ptr::null_mut();
    // SAFETY: `connection` is live per this function's contract, `array` points
    // at a live record with as many children as `converted` has columns, and
    // `out` is a valid out-parameter. DuckDB takes ownership of the array's
    // data; `array`'s own `release` is nulled by DuckDB, so the drop below is a
    // no-op on that path.
    let raw_err = unsafe {
        duckdb_data_chunk_from_arrow(
            connection,
            array.as_mut_ptr(),
            converted.as_raw(),
            &raw mut out,
        )
    };
    // SAFETY: the return value is an owned `duckdb_error_data`, or null.
    let err = unsafe { ErrorData::from_raw(raw_err) };
    if err.has_error() {
        return Err(err);
    }
    if out.is_null() {
        return Err(ErrorData::new(
            DuckDbErrorType::Internal,
            "duckdb_data_chunk_from_arrow reported success but produced no data chunk",
        ));
    }
    // SAFETY: `out` is a fresh chunk this value now owns.
    Ok(unsafe { OwnedDataChunk::from_raw(out) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // These tests exercise the ownership bookkeeping of the wrappers, which is
    // pure Rust: they run without a DuckDB dispatch table. The end-to-end
    // conversions live in `tests/ffi_roundtrip.rs`, where a real database is
    // available.

    // Each test owns its counter and hands it to the release callback through
    // the record's own `private_data`, which is exactly what that field is for.
    // A shared `static` would race: `cargo test` runs these in parallel, so one
    // test's reset could land between another's drop and its assertion.

    /// Increments the counter in `private_data`, if the test installed one.
    ///
    /// # Safety
    ///
    /// `private_data` must be null or a pointer to a live `AtomicUsize`.
    unsafe fn bump(private_data: *mut std::os::raw::c_void) {
        // SAFETY: forwarded from this function's own contract.
        if let Some(counter) = unsafe { private_data.cast::<AtomicUsize>().as_ref() } {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    unsafe extern "C" fn count_schema_release(schema: *mut RawArrowSchema) {
        // SAFETY: DuckDB and every other Arrow producer pass a valid pointer,
        // and every schema built here stores its counter in `private_data`.
        unsafe {
            bump((*schema).private_data);
            // The Arrow specification requires the callback to null its own
            // `release`, which is how "already released" is observable.
            (*schema).release = None;
        }
    }

    unsafe extern "C" fn count_array_release(array: *mut RawArrowArray) {
        // SAFETY: as above.
        unsafe {
            bump((*array).private_data);
            (*array).release = None;
        }
    }

    fn live_schema(counter: &'static AtomicUsize) -> ArrowSchema {
        let mut raw = RawArrowSchema::empty();
        raw.private_data = std::ptr::from_ref(counter).cast_mut().cast();
        raw.release = Some(count_schema_release);
        // SAFETY: `count_schema_release` frees nothing and nulls itself.
        unsafe { ArrowSchema::from_raw(raw) }
    }

    fn live_array(counter: &'static AtomicUsize) -> ArrowArray {
        let mut raw = RawArrowArray::empty();
        raw.private_data = std::ptr::from_ref(counter).cast_mut().cast();
        raw.release = Some(count_array_release);
        // SAFETY: `count_array_release` frees nothing and nulls itself.
        unsafe { ArrowArray::from_raw(raw) }
    }

    /// A populated two-column struct schema, owned entirely by the caller.
    ///
    /// The accessors have to be exercised against a record that actually
    /// carries values: reading only `empty()` proves the released guards work
    /// and nothing else, so "always return None" would go unnoticed.
    struct PopulatedSchema {
        root: ArrowSchema,
        // Kept alive for as long as `root` points into them. Declared after
        // `root` in the struct but dropped after it in `Drop` order terms is
        // not guaranteed, so `PopulatedSchema` never outlives a borrow of
        // `root` and nothing here frees anything.
        _children: Box<[RawArrowSchema; 2]>,
        _child_ptrs: Box<[*mut RawArrowSchema; 2]>,
        _strings: Vec<CString>,
    }

    fn populated_schema() -> PopulatedSchema {
        let strings: Vec<CString> = ["+s", "duckdb_query_result", "i", "id", "u", "label"]
            .iter()
            .map(|s| CString::new(*s).expect("no interior NUL"))
            .collect();

        let mut children = Box::new([RawArrowSchema::empty(), RawArrowSchema::empty()]);
        children[0].format = strings[2].as_ptr();
        children[0].name = strings[3].as_ptr();
        children[0].release = Some(count_schema_release);
        children[1].format = strings[4].as_ptr();
        children[1].name = strings[5].as_ptr();
        children[1].release = Some(count_schema_release);

        let child_ptrs = Box::new([
            std::ptr::from_mut(&mut children[0]),
            std::ptr::from_mut(&mut children[1]),
        ]);

        let mut raw = RawArrowSchema::empty();
        raw.format = strings[0].as_ptr();
        raw.name = strings[1].as_ptr();
        raw.flags = 2; // ARROW_FLAG_NULLABLE
        raw.n_children = 2;
        raw.children = child_ptrs.as_ptr().cast_mut();
        raw.release = Some(count_schema_release);

        PopulatedSchema {
            // SAFETY: `count_schema_release` frees nothing and nulls itself;
            // every pointer above outlives the returned value.
            root: unsafe { ArrowSchema::from_raw(raw) },
            _children: children,
            _child_ptrs: child_ptrs,
            _strings: strings,
        }
    }

    #[test]
    fn a_populated_schema_reports_its_format_name_flags_and_children() {
        let schema = populated_schema();
        let root = &schema.root;
        assert!(!root.is_released());
        assert_eq!(root.format(), Some("+s"));
        assert_eq!(root.name(), Some("duckdb_query_result"));
        assert_eq!(root.flags(), 2);
        assert_eq!(root.child_count(), 2);

        let id = root.child(0).expect("child 0");
        assert_eq!(id.format(), Some("i"));
        assert_eq!(id.name(), Some("id"));

        let label = root.child(1).expect("child 1");
        assert_eq!(label.format(), Some("u"));
        assert_eq!(label.name(), Some("label"));

        assert!(root.child(2).is_none(), "past the end");
    }

    /// A populated array with two children, owned entirely by the caller.
    struct PopulatedArray {
        root: ArrowArray,
        _children: Box<[RawArrowArray; 2]>,
        _child_ptrs: Box<[*mut RawArrowArray; 2]>,
    }

    fn populated_array() -> PopulatedArray {
        let mut children = Box::new([RawArrowArray::empty(), RawArrowArray::empty()]);
        for (i, child) in children.iter_mut().enumerate() {
            child.length = 7;
            child.null_count = i64::try_from(i).expect("small");
            child.release = Some(count_array_release);
        }
        let child_ptrs = Box::new([
            std::ptr::from_mut(&mut children[0]),
            std::ptr::from_mut(&mut children[1]),
        ]);

        let mut raw = RawArrowArray::empty();
        raw.length = 7;
        raw.null_count = 3;
        raw.offset = 2;
        raw.n_children = 2;
        raw.children = child_ptrs.as_ptr().cast_mut();
        raw.release = Some(count_array_release);

        PopulatedArray {
            // SAFETY: `count_array_release` frees nothing and nulls itself;
            // every pointer above outlives the returned value.
            root: unsafe { ArrowArray::from_raw(raw) },
            _children: children,
            _child_ptrs: child_ptrs,
        }
    }

    #[test]
    fn a_populated_array_reports_its_length_nulls_offset_and_children() {
        let array = populated_array();
        let root = &array.root;
        assert!(!root.is_released());
        assert_eq!(root.len(), 7);
        assert!(!root.is_empty());
        assert_eq!(root.null_count(), 3);
        assert_eq!(root.offset(), 2);
        assert_eq!(root.child_count(), 2);

        assert_eq!(root.child(0).expect("child 0").len(), 7);
        assert_eq!(root.child(0).expect("child 0").null_count(), 0);
        assert_eq!(root.child(1).expect("child 1").null_count(), 1);
        assert!(root.child(2).is_none(), "past the end");
    }

    #[test]
    fn a_length_zero_array_is_empty_but_not_released() {
        let mut raw = RawArrowArray::empty();
        raw.release = Some(count_array_release);
        // SAFETY: `count_array_release` frees nothing and nulls itself.
        let array = unsafe { ArrowArray::from_raw(raw) };
        assert!(!array.is_released(), "a live producer set `release`");
        assert_eq!(array.len(), 0);
        assert!(array.is_empty(), "zero rows is empty");
    }

    #[test]
    fn an_empty_schema_is_already_released_and_frees_nothing() {
        let schema = ArrowSchema::empty();
        assert!(schema.is_released());
        assert_eq!(schema.format(), None);
        assert_eq!(schema.name(), None);
        assert_eq!(schema.child_count(), 0);
        assert!(schema.child(0).is_none());
        drop(schema);
    }

    #[test]
    fn an_empty_array_is_already_released_and_frees_nothing() {
        let array = ArrowArray::empty();
        assert!(array.is_released());
        assert_eq!(array.len(), 0);
        assert!(array.is_empty());
        assert_eq!(array.child_count(), 0);
        assert!(array.child(0).is_none());
        drop(array);
    }

    #[test]
    fn dropping_a_live_schema_releases_it_once() {
        static RELEASES: AtomicUsize = AtomicUsize::new(0);
        drop(live_schema(&RELEASES));
        assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_a_live_array_releases_it_once() {
        static RELEASES: AtomicUsize = AtomicUsize::new(0);
        drop(live_array(&RELEASES));
        assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn releasing_explicitly_is_idempotent_and_drop_adds_nothing() {
        static SCHEMA: AtomicUsize = AtomicUsize::new(0);
        static ARRAY: AtomicUsize = AtomicUsize::new(0);

        let mut schema = live_schema(&SCHEMA);
        schema.release();
        schema.release();
        assert!(schema.is_released());
        drop(schema);
        assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

        let mut array = live_array(&ARRAY);
        array.release();
        array.release();
        assert!(array.is_released());
        drop(array);
        assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn into_raw_hands_the_release_callback_to_the_caller() {
        static SCHEMA: AtomicUsize = AtomicUsize::new(0);
        static ARRAY: AtomicUsize = AtomicUsize::new(0);

        let mut raw = live_schema(&SCHEMA).into_raw();
        assert_eq!(
            SCHEMA.load(Ordering::SeqCst),
            0,
            "into_raw must not release"
        );
        let release = raw.release.expect("release survived into_raw");
        // SAFETY: the caller now owns the record; releasing it once is correct.
        unsafe { release(&raw mut raw) };
        assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

        let mut raw = live_array(&ARRAY).into_raw();
        assert_eq!(ARRAY.load(Ordering::SeqCst), 0);
        let release = raw.release.expect("release survived into_raw");
        // SAFETY: as above.
        unsafe { release(&raw mut raw) };
        assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn take_from_neutralises_the_source_so_only_one_side_releases() {
        static SCHEMA: AtomicUsize = AtomicUsize::new(0);
        static ARRAY: AtomicUsize = AtomicUsize::new(0);

        let mut source = RawArrowSchema::empty();
        source.private_data = std::ptr::from_ref(&SCHEMA).cast_mut().cast();
        source.release = Some(count_schema_release);
        // SAFETY: `source` is a live, uniquely-owned record.
        let taken = unsafe { ArrowSchema::take_from(&raw mut source) };
        assert!(!taken.is_released(), "the callback moved to the wrapper");
        assert!(
            source.release.is_none(),
            "the source must be left released so its own Drop is a no-op"
        );
        drop(taken);
        assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

        let mut source = RawArrowArray::empty();
        source.private_data = std::ptr::from_ref(&ARRAY).cast_mut().cast();
        source.release = Some(count_array_release);
        // SAFETY: as above.
        let taken = unsafe { ArrowArray::take_from(&raw mut source) };
        assert!(!taken.is_released());
        assert!(source.release.is_none());
        drop(taken);
        assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_released_record_reports_neutral_values_instead_of_dangling_ones() {
        // After release the Arrow specification leaves every other field
        // undefined, so the accessors must not read them. Simulate a producer
        // that released without clearing the stale pointers.
        let mut raw = RawArrowSchema::empty();
        raw.format = c"+s".as_ptr();
        raw.n_children = 7;
        raw.children = ptr::dangling_mut();
        // SAFETY: `release` is None, so nothing is called and nothing is freed.
        let schema = unsafe { ArrowSchema::from_raw(raw) };
        assert!(schema.is_released());
        assert_eq!(schema.format(), None, "must not read a stale format");
        assert_eq!(schema.child_count(), 0, "must not trust a stale n_children");
        assert!(schema.child(0).is_none(), "must not walk stale children");

        let mut raw = RawArrowArray::empty();
        raw.length = 42;
        raw.n_children = 7;
        raw.children = ptr::dangling_mut();
        // SAFETY: as above.
        let array = unsafe { ArrowArray::from_raw(raw) };
        assert_eq!(array.len(), 0);
        assert_eq!(array.child_count(), 0);
        assert!(array.child(0).is_none());
    }

    #[test]
    fn debug_says_released_without_touching_the_other_fields() {
        static SCHEMA: AtomicUsize = AtomicUsize::new(0);
        static ARRAY: AtomicUsize = AtomicUsize::new(0);

        assert!(format!("{:?}", ArrowSchema::empty()).contains("released"));
        assert!(format!("{:?}", ArrowArray::empty()).contains("released"));

        let schema = live_schema(&SCHEMA);
        let rendered = format!("{schema:?}");
        assert!(rendered.contains("n_children"), "{rendered}");
        let array = live_array(&ARRAY);
        let rendered = format!("{array:?}");
        assert!(rendered.contains("length"), "{rendered}");
    }

    #[test]
    fn a_converted_schema_remembers_its_column_count() {
        // SAFETY: a null handle is what `duckdb_destroy_arrow_converted_schema`
        // ignores, so this never calls into DuckDB.
        let converted = unsafe { ArrowConvertedSchema::from_raw(ptr::null_mut(), 3) };
        assert_eq!(converted.column_count(), 3);
        assert!(format!("{converted:?}").contains("column_count"));
    }
}
