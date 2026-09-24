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
/// # Errors
///
/// Whatever `DuckDB` reports for a type it cannot render as Arrow.
#[mutants::skip] // FFI conversion — covered by tests/ffi_roundtrip.rs, which `--lib` does not run
pub fn data_chunk_to_arrow(
    options: &ArrowOptions<'_>,
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
/// copied. Dictionary-encoded and run-end-encoded children are converted
/// (`ColumnArrowToDuckDBDictionary` / `…RunEndEncoded`) as well as plain ones.
///
/// # Errors
///
/// [`DuckDbErrorType::InvalidInput`], checked here because `DuckDB` does not
/// check them and would read out of bounds or through a null pointer:
///
/// - `array` has already been released, has a negative `length` or `offset`,
///   or has no rows (see below);
/// - its child count does not match `converted`'s column count;
/// - its `children` pointer, or one of the child pointers, is null;
/// - a child is shorter than `offset + length` (the Arrow specification
///   requires every child of a struct array to hold that many rows).
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
    if let Err(message) = check_struct_children(&array) {
        return Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            &format!("data_chunk_from_arrow: {message}"),
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

/// The structural checks `duckdb_data_chunk_from_arrow` skips before it reads
/// `arrow_array->children[i]` for every column: a negative length or offset,
/// a null `children` array or child, and a child with fewer than the parent's
/// `offset + length` rows. `array` must not be released.
fn check_struct_children(array: &ArrowArray) -> Result<(), String> {
    let raw = &array.0;
    if raw.length < 0 || raw.offset < 0 {
        return Err(format!(
            "the Arrow array has a negative length ({}) or offset ({})",
            raw.length, raw.offset
        ));
    }
    let children = array.child_count();
    if children == 0 {
        return Ok(());
    }
    if raw.children.is_null() {
        return Err(format!(
            "the Arrow array declares {children} child array(s) but its `children` pointer is \
             null"
        ));
    }
    let needed = raw.length.checked_add(raw.offset).ok_or_else(|| {
        format!(
            "the Arrow array's offset ({}) plus length ({}) overflows",
            raw.offset, raw.length
        )
    })?;
    for index in 0..children {
        // SAFETY: `children` is non-null and, per `ArrowArray::from_raw`'s
        // contract, points at `n_children` child pointers.
        let child = unsafe { *raw.children.add(index) };
        if child.is_null() {
            return Err(format!("child {index} of the Arrow array is null"));
        }
        // SAFETY: a non-null child pointer of a valid array points at a live
        // record.
        let child_length = unsafe { (*child).length };
        if child_length < needed {
            return Err(format!(
                "child {index} of the Arrow array has {child_length} row(s), but the struct \
                 array's offset + length needs {needed}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_struct_children, ArrowArray, RawArrowArray};

    unsafe extern "C" fn no_op_release(array: *mut RawArrowArray) {
        // SAFETY: `array` is the live record `ArrowArray`
        // passes to its own release callback.
        unsafe { (*array).release = None };
    }

    /// A negative length is refused by the structural check, which
    /// `data_chunk_from_arrow` runs before its zero-row check, so the error
    /// names the negative length rather than calling the array empty
    /// (`ArrowArray::len` maps a negative length to 0). `DuckDB` itself
    /// aborts on it: `NumericCast<idx_t>(arrow_array->length)` runs before
    /// its try block.
    #[test]
    fn a_negative_length_or_offset_is_refused_by_name() {
        for (length, offset) in [(-1, 0), (1, -1)] {
            let mut raw = RawArrowArray::empty();
            raw.length = length;
            raw.offset = offset;
            raw.release = Some(no_op_release);
            // SAFETY: no buffers or children; `no_op_release` frees nothing.
            let array = unsafe { ArrowArray::from_raw(raw) };
            let err = check_struct_children(&array).expect_err("negative length or offset");
            assert!(
                err.contains(&format!("negative length ({length}) or offset ({offset})")),
                "{err}"
            );
        }
    }
}
