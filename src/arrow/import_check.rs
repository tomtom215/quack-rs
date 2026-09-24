// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The structural checks `data_chunk_from_arrow` makes on the raw record
//! before `DuckDB` sees it, and the flattening it applies to what `DuckDB`
//! returns. The layout checks that need the schema are in `import_layout`.

use super::{ArrowArray, RawArrowArray};
use crate::types::LogicalType;

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
pub(super) unsafe fn flatten_if_needed(
    vector: libduckdb_sys::duckdb_vector,
    rows: usize,
    dictionary: bool,
) -> Result<(), crate::error::ExtensionError> {
    use crate::selection_vector::SelectionVector;
    use crate::vector::ops::{copy_selected, reference_vector, OwnedVector};

    // SAFETY: `vector` is live per the contract; the returned type is owned.
    let logical_type =
        unsafe { LogicalType::from_raw(libduckdb_sys::duckdb_vector_get_column_type(vector)) };
    // A null-type array anywhere below the column is a constant vector too,
    // inside a struct or list the readers index as flat.
    // SAFETY: `logical_type` is a live, owned handle.
    let null_type = unsafe {
        crate::table::type_check::contains_type(logical_type.as_raw(), &|raw| {
            raw == libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL
        })
    };
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
pub(super) unsafe fn has_dictionary(raw: &RawArrowArray) -> bool {
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
pub(super) fn check_struct_children(array: &ArrowArray) -> Result<(), String> {
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
