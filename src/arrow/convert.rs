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
/// # Values `DuckDB` exports wrongly, without an error
///
/// Checked on `DuckDB` 1.5.0 and 1.5.5 (`docs/upstream-duckdb-reports.md`):
///
/// - `INTERVAL`: Arrow's `month_day_nano` counts nanoseconds in an `i64`, and
///   `DuckDB` multiplies the microseconds by 1000 with no overflow check. An
///   interval whose microsecond field is beyond ±`i64::MAX / 1000` (about
///   ±106,751 days, or 2,562,047 hours) wraps: `INTERVAL 2562048 HOUR` exports
///   as a negative interval.
/// - `UHUGEINT`: exported as a signed `decimal128(38, 0)`, so a value of
///   2^127 or more comes back negative (`2^128 - 1` as `-1`), and importing it
///   back fails later with "Negation of HUGEINT is out of range".
///
/// Check such values before exporting them when the consumer must see them
/// exactly.
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
/// - a dictionary-encoded array anywhere in `array` has a validity buffer, a
///   nonzero null count and more than
///   [`duckdb_vector_size`](libduckdb_sys::duckdb_vector_size) (2048)
///   entries. `DuckDB` imports that shape by writing past a heap allocation
///   (see `docs/upstream-duckdb-reports.md`); a `LIST` of 1025 or more rows
///   whose two-element lists are dictionary-encoded is enough to reach it.
///   Split such batches before importing them.
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
    // SAFETY: `array` is a valid, unreleased Arrow array per this function's
    // contract.
    if let Err(message) = unsafe { check_dictionary_validity(&array.0) } {
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
            unsafe { has_dictionary(&**array.0.children.add(index)) }
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
        unsafe { flatten_if_needed(chunk.vector(column), chunk.size(), dictionary) }.map_err(
            |e| {
                ErrorData::new(
                    DuckDbErrorType::Internal,
                    &format!("data_chunk_from_arrow: flattening column {column}: {e}"),
                )
            },
        )?;
    }
    Ok(chunk)
}

/// Rewrites a column `DuckDB` imported as a non-flat vector into a flat one.
///
/// Every reader in this crate indexes `duckdb_vector_get_data` directly, which
/// is only correct for a flat vector, and the C API has no call that reports a
/// vector's physical layout. The import makes two kinds of non-flat vector:
///
/// - a **dictionary** vector for a dictionary-encoded array, at any depth
///   (`ColumnArrowToDuckDBDictionary` ends in `vector.Slice(...)`, and nested
///   dictionaries go through the same function). Its data buffer holds the
///   dictionary, not one entry per row, so a flat read of row `i` returns the
///   wrong value and, past the dictionary's length, reads out of bounds;
/// - a **constant** vector for an Arrow null-type array
///   (`vector.Reference(Value())`), whose validity describes row 0 only.
///
/// Both are copied through an identity selection into a fresh flat vector,
/// which the column then references. `VectorOperations::Copy` resolves the
/// dictionary at every level and copies string payloads into the new
/// vector's own heap (`vector_copy.cpp`), so the result does not depend on
/// the Arrow buffers.
///
/// # Safety
///
/// `vector` must be a live column of an imported chunk holding `rows` rows.
unsafe fn flatten_if_needed(
    vector: libduckdb_sys::duckdb_vector,
    rows: usize,
    dictionary: bool,
) -> Result<(), crate::error::ExtensionError> {
    use crate::selection_vector::SelectionVector;
    use crate::vector::ops::{copy_selected, reference_vector, OwnedVector};

    // SAFETY: `vector` is live per the contract; the returned type is owned.
    let logical_type =
        unsafe { LogicalType::from_raw(libduckdb_sys::duckdb_vector_get_column_type(vector)) };
    // SAFETY: `logical_type` is a live, owned handle.
    let null_type =
        unsafe { logical_type.try_get_type_id() } == Some(crate::types::TypeId::SqlNull);
    if !dictionary && !null_type {
        return Ok(());
    }
    let flat = OwnedVector::new(&logical_type, rows)?;
    let mut identity = SelectionVector::new(rows)?;
    for (row, slot) in identity.as_mut_slice().iter_mut().enumerate() {
        // `SelectionVector::new` refused any `rows` whose indices `sel_t`
        // cannot hold, so this conversion cannot fail.
        *slot = libduckdb_sys::sel_t::try_from(row).map_err(|_| {
            crate::error::ExtensionError::new("row index does not fit a selection vector")
        })?;
    }
    // SAFETY: both vectors are live and of `logical_type`; the identity
    // selection names rows `0..rows` of `vector`, and `flat` has room for
    // `rows`. `flat`'s data is shared with `vector` by the reference, so it
    // stays alive after `flat` is dropped.
    unsafe {
        copy_selected(vector, flat.as_raw(), &identity, rows, 0, 0);
        reference_vector(vector, flat.as_raw());
    }
    Ok(())
}

/// Whether `raw`, or any array beneath it, is dictionary-encoded.
///
/// # Safety
///
/// `raw` must be a valid, unreleased Arrow array.
unsafe fn has_dictionary(raw: &RawArrowArray) -> bool {
    let mut found = false;
    // SAFETY: forwarded from this function's own contract.
    let _ = unsafe {
        walk(raw, &mut |node| {
            found |= !node.dictionary.is_null();
            Ok(())
        })
    };
    found
}

/// Refuses the one dictionary shape `DuckDB` imports by overflowing the heap.
///
/// `ColumnArrowToDuckDBDictionary` (`arrow_conversion.cpp`) copies the
/// indices' validity into a default-constructed `ValidityMask`, which
/// `EnsureWritable` sizes for `STANDARD_VECTOR_SIZE` rows, with a `memcpy` of
/// as many bits as it is converting. A dictionary-encoded array with a
/// validity buffer, a nonzero null count and more than
/// [`duckdb_vector_size`](libduckdb_sys::duckdb_vector_size) entries writes
/// past that allocation. A `LIST` whose child is dictionary-encoded reaches it
/// with a chunk of only 1025 rows of two elements each. The condition below is
/// the one `GetValidityMask` copies under, applied to every dictionary-encoded
/// node, since the number of entries converted is at most the node's length.
///
/// # Safety
///
/// `raw` must be a valid, unreleased Arrow array.
unsafe fn check_dictionary_validity(raw: &RawArrowArray) -> Result<(), String> {
    // SAFETY: takes no arguments and reads a compile-time constant.
    let limit = unsafe { libduckdb_sys::duckdb_vector_size() };
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        walk(raw, &mut |node| {
            let copies_validity = !node.dictionary.is_null()
                && node.null_count != 0
                && node.n_buffers > 0
                && !node.buffers.is_null()
                // SAFETY: `buffers` is non-null and holds `n_buffers > 0`
                // entries.
                && !(*node.buffers).is_null();
            let entries = u64::try_from(node.length).unwrap_or(0);
            if copies_validity && entries > limit {
                return Err(format!(
                    "a dictionary-encoded Arrow array with {entries} entries and nulls cannot be \
                    imported: DuckDB copies the validity of more than {limit} dictionary \
                    indices into a {limit}-row mask and overflows the heap \
                    (`ColumnArrowToDuckDBDictionary`, `arrow_conversion.cpp`). Split the \
                    batch so that no dictionary-encoded array, including a list's child, holds \
                    more than {limit} entries, or decode the dictionary before importing"
                ));
            }
            Ok(())
        })
    }
}

/// Calls `visit` on `raw` and on every array reachable from it through
/// `children` and `dictionary`, depth first, stopping at the first error.
/// Null child and dictionary pointers are skipped.
///
/// # Safety
///
/// `raw` must be a valid, unreleased Arrow array: every non-null pointer in
/// `children[..n_children]` and `dictionary` points at a live record.
unsafe fn walk(
    raw: &RawArrowArray,
    visit: &mut dyn FnMut(&RawArrowArray) -> Result<(), String>,
) -> Result<(), String> {
    visit(raw)?;
    let children = usize::try_from(raw.n_children).unwrap_or(0);
    if !raw.children.is_null() {
        for index in 0..children {
            // SAFETY: `children` holds `n_children` pointers per the contract.
            let child = unsafe { *raw.children.add(index) };
            if !child.is_null() {
                // SAFETY: a non-null child of a valid array is a live record.
                unsafe { walk(&*child, visit)? };
            }
        }
    }
    if !raw.dictionary.is_null() {
        // SAFETY: a non-null dictionary of a valid array is a live record.
        unsafe { walk(&*raw.dictionary, visit)? };
    }
    Ok(())
}

/// The structural checks `duckdb_data_chunk_from_arrow` skips before it reads
/// `arrow_array->children[i]` for every column: a negative length or offset,
/// a nonzero offset (which `DuckDB` ignores), a null `children` array or
/// child, and a child with fewer than the parent's `length` rows. `array` must
/// not be released.
fn check_struct_children(array: &ArrowArray) -> Result<(), String> {
    let raw = &array.0;
    if raw.length < 0 || raw.offset < 0 {
        return Err(format!(
            "the Arrow array has a negative length ({}) or offset ({})",
            raw.length, raw.offset
        ));
    }
    // `DuckDB` reads a column from row 0 of its child, not from the parent's
    // `offset`: a struct array of 3 rows at offset 2 imports child rows 0..3
    // instead of 2..5 (wrong rows, no error; `docs/upstream-duckdb-reports.md`).
    if raw.offset != 0 {
        return Err(format!(
            "the Arrow struct array has offset {}: DuckDB ignores a top-level offset and would \
             import the wrong rows. Export the batch without an offset (slice its children \
             instead)",
            raw.offset
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
    let needed = raw.length;
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
                 array's length needs {needed}"
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

    /// Zero is a valid length and a valid offset: an empty, childless record
    /// (a zero-column result) passes the structural check.
    #[test]
    fn a_zero_length_zero_offset_childless_array_passes() {
        let mut raw = RawArrowArray::empty();
        raw.release = Some(no_op_release);
        // SAFETY: no buffers or children; `no_op_release` frees nothing.
        let array = unsafe { ArrowArray::from_raw(raw) };
        assert_eq!(check_struct_children(&array), Ok(()));
    }

    /// Checks a one-column struct array of `length` rows at `offset` whose
    /// child holds `child_length` rows.
    fn check_one_child(length: i64, offset: i64, child_length: i64) -> Result<(), String> {
        let mut child = RawArrowArray::empty();
        child.length = child_length;
        child.release = Some(no_op_release);
        let mut child_ptrs = [std::ptr::from_mut(&mut child)];

        let mut raw = RawArrowArray::empty();
        raw.length = length;
        raw.offset = offset;
        raw.n_children = 1;
        raw.children = child_ptrs.as_mut_ptr();
        raw.release = Some(no_op_release);
        // SAFETY: `no_op_release` frees nothing, and `child` / `child_ptrs`
        // are locals declared before `array`, so they outlive it.
        let array = unsafe { ArrowArray::from_raw(raw) };
        check_struct_children(&array)
    }

    /// A child must hold at least the parent's `length` rows: exactly
    /// enough or more is accepted, one short is refused by name.
    #[test]
    fn a_child_must_cover_the_parents_length() {
        assert_eq!(check_one_child(3, 0, 3), Ok(()), "exactly enough rows");
        assert_eq!(check_one_child(3, 0, 4), Ok(()), "more rows than needed");
        let err = check_one_child(3, 0, 2).expect_err("one row short");
        assert!(
            err.contains("child 0 of the Arrow array has 2 row(s)") && err.contains("needs 3"),
            "{err}"
        );
        // An empty struct array at offset 0 needs nothing from its child.
        assert_eq!(check_one_child(0, 0, 0), Ok(()));
    }

    /// `DuckDB` imports child rows `0..length` whatever the parent's offset,
    /// so any nonzero offset is refused, even with children long enough for
    /// it; offset 0 is not.
    #[test]
    fn a_nonzero_parent_offset_is_refused_by_name() {
        for offset in [1, 2, i64::MAX] {
            let err = check_one_child(3, offset, 10).expect_err("nonzero offset");
            assert!(err.contains(&format!("has offset {offset}")), "{err}");
        }
        assert_eq!(check_one_child(3, 0, 10), Ok(()));
    }
}
