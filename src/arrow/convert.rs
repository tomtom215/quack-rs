// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The four conversions between `DuckDB` data chunks and the Arrow C Data Interface.

use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

use libduckdb_sys::{
    duckdb_arrow_converted_schema, duckdb_connection, duckdb_data_chunk,
    duckdb_data_chunk_from_arrow, duckdb_data_chunk_to_arrow, duckdb_logical_type,
    duckdb_schema_from_arrow, duckdb_to_arrow_schema, idx_t,
};

use super::{
    ArrowArray, ArrowConvertedSchema, ArrowOptions, ArrowSchema, RawArrowArray, RawArrowSchema,
};
use crate::data_chunk::DataChunk;
use crate::error_data::{DuckDbErrorType, ErrorData};
use crate::query::OwnedDataChunk;
use crate::types::LogicalType;

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
    options: &ArrowOptions<'_>,
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
/// # Values `DuckDB` would export wrongly are refused
///
/// `DuckDB` exports these as different values, with no error (checked on
/// 1.4.4, 1.5.0 and 1.5.5; `docs/upstream-duckdb-reports.md`, item 16), so
/// the chunk is checked first and refused if it holds one, at any depth:
///
/// - `INTERVAL`: Arrow's `month_day_nano` counts nanoseconds in an `i64`, and
///   `DuckDB` multiplies the microseconds by 1000 with no overflow check. An
///   interval whose microsecond field is beyond ±`i64::MAX / 1000` (about
///   ±106,751 days, or 2,562,047 hours) would wrap: `INTERVAL 2562048 HOUR`
///   would export as a negative interval.
/// - `UHUGEINT`, and `HUGEINT` unless `arrow_lossless_conversion` is set:
///   exported as `decimal128(38, 0)`, which holds 38 digits. A `UHUGEINT` of
///   2^127 or more would come back negative (`2^128 - 1` as `-1`), and a
///   39-digit `HUGEINT` would not survive either.
///
/// The check copies each column whose type contains one of these types (a
/// chunk's vectors may be dictionary or constant vectors), so it costs one
/// copy of those columns.
///
/// # Errors
///
/// - [`DuckDbErrorType::InvalidInput`] naming the column and value, for a
///   value `DuckDB` would export wrongly (see above).
/// - Whatever `DuckDB` reports for a type it cannot render as Arrow.
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub fn data_chunk_to_arrow(
    options: &ArrowOptions<'_>,
    chunk: &DataChunk,
) -> Result<ArrowArray, ErrorData> {
    super::export_check::check_chunk(options, chunk)?;
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
    let array = unsafe { ArrowArray::from_raw(out) };
    // SAFETY: `array` is what DuckDB just exported from `chunk` under
    // `options`. On an error it is dropped here, which releases it.
    unsafe { super::export_check::check_layout(options, chunk, &array) }?;
    Ok(array)
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
    // SAFETY: `out` is a fresh handle this value now owns, built from `schema`
    // (`PopulateArrowTableSchema` makes one column per child of it).
    Ok(unsafe { ArrowConvertedSchema::from_raw(out, schema) })
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
/// copied — with two exceptions, which are copied so that the chunk is flat
/// and every [`VectorReader`](crate::vector::VectorReader) reads it
/// correctly. `DuckDB` imports a column that is dictionary-encoded at any
/// depth as a *dictionary* vector, and an Arrow null-type column as a
/// *constant* vector; a flat read of either returns wrong values (and, for a
/// dictionary, reads past the end of its buffer). Those columns are copied
/// into fresh flat vectors before this function returns. Run-end-encoded
/// children are expanded by `DuckDB` itself.
///
/// # Errors
///
/// [`DuckDbErrorType::InvalidInput`], checked here because `DuckDB` does not
/// check them and would read out of bounds or through a null pointer:
///
/// - `array` has already been released, has a negative `length`, has a
///   nonzero `offset` (`DuckDB` ignores the top-level offset and imports the
///   wrong rows, see `docs/upstream-duckdb-reports.md`), or has no rows (see
///   below);
/// - its child count does not match `converted`'s column count;
/// - its `children` pointer, or one of the child pointers, is null;
/// - a child is shorter than `length` (the Arrow specification requires
///   every child of a struct array to hold `offset + length` rows);
/// - a node anywhere in `array` has a valid Arrow layout that `DuckDB`
///   imports from the wrong rows or out of bounds: an offset below the top
///   level that `DuckDB` misapplies, a dictionary under a list or with more
///   than [`duckdb_vector_size`](libduckdb_sys::duckdb_vector_size) (2048)
///   rows that can hold NULLs, a nested dictionary, overlapping or gapped
///   list views, a sparse union with recoded type ids, or a run-end-encoded
///   array where `DuckDB` reads a plain one. The message names the node; the
///   module docs of `src/arrow/import_layout.rs` and
///   `docs/upstream-duckdb-reports.md` (items 9 and 24 to 28) give the
///   details;
/// - the array and its schema disagree on a node's child count or
///   dictionary encoding.
///
/// [`DuckDbErrorType::Internal`] if copying a non-flat column into a flat
/// vector fails (see above), which needs an allocation failure.
///
/// Every error `DuckDB` itself reports from the conversion also arrives as
/// [`DuckDbErrorType::InvalidInput`]: `duckdb_data_chunk_from_arrow` maps all
/// of them to `DUCKDB_ERROR_INVALID_INPUT`.
///
/// # Safety
///
/// - `connection` must be a live `duckdb_connection`, and `converted` must
///   have been produced from it (or from another connection of the same
///   database).
/// - `array` must be a valid Arrow struct array (see
///   [`ArrowArray::from_raw`]) that **conforms to `converted`'s schema**: each
///   child's physical layout must be the one its column's Arrow format
///   describes. `DuckDB` reads a child's buffers as that format with no check
///   — an `int32` child imported under a `utf8` schema has its values read as
///   string offsets into a buffer that does not exist (a crash, or worse). An
///   array exported with [`data_chunk_to_arrow`] under the schema that
///   `converted` was made from conforms.
/// - `length` must be the array's true row count. `DuckDB` allocates a chunk
///   of that capacity before its error handling starts
///   (`dchunk->Initialize(…, length)` in `arrow-c.cpp`), so a length too large
///   to allocate throws through the C API and aborts the process. No bound
///   short of the memory the producer's own buffers already occupy separates
///   a valid length from an absurd one, so it cannot be checked here.
/// - Every validity bitmap that `DuckDB` reads (one with a nonzero
///   `null_count`) must be readable for **one byte past** the last byte that
///   holds its rows' bits. When a node's effective bit offset is not a
///   multiple of 8, `GetValidityMask` (`arrow_conversion.cpp`) copies
///   `ceil(rows / 8) + 1` bytes from the first byte it needs, which can be one
///   more than the rows occupy: an `int32` column at offset 1 of length 7 with
///   a 1-byte bitmap is read as 2 bytes (an invalid read under valgrind on
///   1.5.5). That byte only supplies bits for rows past the end, so the
///   values imported are right; the read itself is the hazard. Buffers padded to a multiple of 8 or 64 bytes, as
///   the Arrow columnar format recommends, satisfy this; the allocation size
///   is invisible through the C Data Interface, so it cannot be checked here.
/// - A dictionary whose values are a fixed-width type (integers, floats,
///   `DATE` in days, `TIMESTAMP`, `DECIMAL`, ...) and whose indices can be
///   NULL (a nonzero `null_count`, or NULL rows in an enclosing struct) must
///   have its values buffer readable for **one element past**
///   `offset + length`. `DuckDB` points every NULL index at a sentinel entry
///   one past the dictionary, but imports those types without copying, so
///   the sentinel lies in the producer's buffer; copying the column, as this
///   function does for every dictionary-encoded column, reads it (an invalid
///   read under valgrind on 1.4.4 to 1.5.5,
///   `docs/upstream-duckdb-reports.md`, item 29). The value read is
///   discarded: those rows are NULL. A buffer padded to a multiple of 64
///   bytes satisfies this unless the values fill it exactly.
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
    if let Err(message) = super::import_check::check_struct_children(&array) {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            &format!("data_chunk_from_arrow: {message}"),
        ));
    }
    // SAFETY: takes no arguments and reads a compile-time constant.
    let limit = unsafe { libduckdb_sys::duckdb_vector_size() };
    // SAFETY: `array` is a valid, unreleased array that conforms to
    // `converted`'s schema (this function's contract), and
    // `check_struct_children` confirmed one live child per column.
    let layout = unsafe { super::import_layout::check(&array.0, converted.shapes(), limit) };
    if let Err(message) = layout {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            &format!("data_chunk_from_arrow: {message}"),
        ));
    }
    // Which columns come back non-flat, decided before `DuckDB` claims the
    // array (its buffers stay alive in the chunk, but the record is `DuckDB`'s
    // afterwards).
    let dictionary_columns: Vec<bool> = (0..children)
        .map(|index| {
            // SAFETY: `check_struct_children` confirmed `children` holds
            // `children` non-null pointers to live records.
            unsafe { super::import_check::has_dictionary(&**array.0.children.add(index)) }
        })
        .collect();
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
    let chunk = unsafe { OwnedDataChunk::from_raw(out) };
    for (column, &dictionary) in dictionary_columns.iter().enumerate() {
        // SAFETY: `column` is below the chunk's column count, which is
        // `converted`'s, which matched the array's child count above.
        unsafe {
            super::import_check::flatten_if_needed(chunk.vector(column), chunk.size(), dictionary)
        }
        .map_err(|e| {
            ErrorData::new(
                DuckDbErrorType::Internal,
                &format!("data_chunk_from_arrow: flattening column {column}: {e}"),
            )
        })?;
    }
    Ok(chunk)
}
