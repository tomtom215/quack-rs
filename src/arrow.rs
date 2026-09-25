// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Arrow C Data Interface bridge (`duckdb-1-5-4` feature).
//!
//! `DuckDB`'s C API has a conversion family (already in 1.4.4) that moves data
//! between a `duckdb_data_chunk` and the [Arrow C Data Interface] without going through a
//! query result:
//!
//! | `DuckDB` C API | quack-rs |
//! |---|---|
//! | `duckdb_connection_get_arrow_options` | [`ArrowOptions::from_connection`], [`ArrowOptions::from_raw_connection`] |
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
//! column in the converted schema, with no bounds check, dereferences each
//! child without a null check, reads `offset + length` rows of each child
//! without comparing its length, and dereferences the array without checking
//! whether it has already been released. Those are segfaults or out-of-bounds
//! reads rather than errors. [`data_chunk_from_arrow`] checks them first and
//! returns an [`ErrorData`][crate::error_data::ErrorData] instead — which is why [`ArrowConvertedSchema`]
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
//! Nor whether the array **conforms to the converted schema**: an Arrow array
//! carries no type, so `DuckDB` reads each child as the format the schema
//! declares, and an `int32` child imported under a `utf8` schema is read as
//! string offsets. And `length` is taken on trust: `DuckDB` allocates the
//! chunk for that many rows before its error handling starts, so an absurd
//! length aborts the process with an allocation failure. Both are part of
//! [`data_chunk_from_arrow`]'s `# Safety` contract.
//!
//! # Round trips that lose information
//!
//! `DuckDB`'s own converters do not round-trip every type exactly: a `TIMETZ`
//! comes back as `TIME` with its offset dropped (`01:02:03+05:30` returns as
//! `01:02:03`), and a `BIT` comes back as `BLOB`.
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
//! // SAFETY: `con` is a live DuckDB connection that outlives `options`.
//! let options = unsafe { ArrowOptions::from_raw_connection(con) }?;
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

mod array;
mod convert;
mod converted;
mod export_check;
mod import_check;
mod import_layout;
mod options;
mod schema;
#[cfg(test)]
mod tests;

pub use convert::{data_chunk_from_arrow, data_chunk_to_arrow, schema_from_arrow, to_arrow_schema};

use std::marker::PhantomData;

use libduckdb_sys::{duckdb_arrow_converted_schema, duckdb_arrow_options};

use crate::query::OwnedConnection;

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
///
/// # Lifetime
///
/// The handle is a copy of the connection's `ClientProperties`, and that
/// struct keeps a **raw pointer to the connection's `ClientContext`**, which
/// the conversion functions dereference (`DBConfig::GetConfig(context)`). Once
/// the connection closes, every use is a heap use-after-free. So the options
/// borrow the connection for `'conn`:
///
/// ```rust,compile_fail,E0505
/// use quack_rs::arrow::{data_chunk_to_arrow, ArrowOptions};
/// use quack_rs::query::OwnedConnection;
///
/// fn demo(con: OwnedConnection, chunk: &quack_rs::data_chunk::DataChunk) {
///     let options = ArrowOptions::from_connection(&con).unwrap();
///     drop(con); // error[E0505]: cannot move out of `con` because it is borrowed
///     let _ = data_chunk_to_arrow(&options, chunk);
/// }
/// ```
///
/// Options read from a [`QueryResult`][crate::query::QueryResult] point at the connection that ran the
/// query, which a `QueryResult` does not borrow, so
/// [`from_result`][Self::from_result] is `unsafe`.
pub struct ArrowOptions<'conn> {
    raw: duckdb_arrow_options,
    _conn: PhantomData<&'conn OwnedConnection>,
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
///
/// # Why it remembers the schema's shape
///
/// `duckdb_data_chunk_from_arrow` imports some valid layouts wrongly (see
/// [`data_chunk_from_arrow`]), and the array alone does not say which type each
/// node has. The converted schema keeps the formats of the schema it was built
/// from, so the array can be checked against them before the import.
pub struct ArrowConvertedSchema {
    raw: duckdb_arrow_converted_schema,
    column_count: usize,
    shapes: Vec<import_layout::Shape>,
}
