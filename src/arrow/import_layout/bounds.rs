// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The bounds `DuckDB` takes on trust from an Arrow array: a list's offsets, a
//! list view's offsets and sizes, a node's length against the rows its parent
//! reads from it, and dictionary indices.
//!
//! `DuckDB`'s conversion (`arrow_conversion.cpp`, read at v1.4.4, v1.5.6 and
//! v2.0.0) checks none of them, and keeps most children zero-copy, so a value
//! outside its bounds is not an import error: it is an out-of-bounds read
//! when the imported chunk is read, after `data_chunk_from_arrow` returned
//! `Ok` (a SIGSEGV on 1.5.6 and 2.0.0 for a list offset past its child). A
//! valid Arrow array satisfies every check here; each refusal names the
//! value that breaks the specification.

use std::ffi::c_void;

use super::{read_index, Ctx, RawArrowArray};

/// The integer type of a dictionary's indices, from its format string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::arrow) struct Indices {
    /// Bytes per index: 1, 2, 4 or 8.
    width: u8,
    signed: bool,
}

impl Indices {
    /// 32-bit signed indices, what most producers write (`i`).
    #[cfg(test)]
    pub(super) const I32: Self = Self {
        width: 4,
        signed: true,
    };

    /// The index type a dictionary-encoded array of `format` has, or `None`
    /// if `format` is not one of Arrow's eight integer formats.
    pub(super) fn of(format: &str) -> Option<Self> {
        let (width, signed) = match format {
            "c" => (1, true),
            "C" => (1, false),
            "s" => (2, true),
            "S" => (2, false),
            "i" => (4, true),
            "I" => (4, false),
            "l" => (8, true),
            "L" => (8, false),
            _ => return None,
        };
        Some(Self { width, signed })
    }

    /// Index `at` of `data`, widened without loss.
    ///
    /// # Safety
    ///
    /// `data` must point to more than `at` indices of this type.
    unsafe fn read(self, data: *const c_void, at: usize) -> i128 {
        // SAFETY: the caller guarantees `at` is in the buffer; every read is
        // unaligned, as an Arrow buffer need not be aligned for its type.
        unsafe {
            match (self.width, self.signed) {
                (1, true) => i128::from(data.cast::<i8>().add(at).read_unaligned()),
                (1, false) => i128::from(data.cast::<u8>().add(at).read_unaligned()),
                (2, true) => i128::from(data.cast::<i16>().add(at).read_unaligned()),
                (2, false) => i128::from(data.cast::<u16>().add(at).read_unaligned()),
                (4, true) => i128::from(data.cast::<i32>().add(at).read_unaligned()),
                (4, false) => i128::from(data.cast::<u32>().add(at).read_unaligned()),
                (8, true) => i128::from(data.cast::<i64>().add(at).read_unaligned()),
                _ => i128::from(data.cast::<u64>().add(at).read_unaligned()),
            }
        }
    }
}

/// Refuses a node shorter than the rows `ctx` says `DuckDB` reads from it.
///
/// `DuckDB` reads rows `inherited..inherited + size` of a node (counted from
/// its own `offset`) without looking at its `length`: a struct's field, a
/// list's child, a fixed-size list's child or a union's member that is
/// shorter than its parent needs is read past its buffers. The Arrow
/// specification requires each to hold those rows.
pub(super) fn covers(node: &RawArrowArray, ctx: Ctx) -> Result<(), String> {
    if ctx.size == 0 {
        return Ok(());
    }
    let end = i64::try_from(ctx.size)
        .ok()
        .and_then(|size| ctx.inherited.checked_add(size));
    match end {
        Some(end) if ctx.inherited >= 0 && end <= node.length => Ok(()),
        _ => Err(format!(
            "the array has {} row(s), but its parent reads {} row(s) of it from row {} (a \
             struct's length, a list's offsets or a fixed-size list's size point past its \
             end), which DuckDB would read out of bounds",
            node.length, ctx.size, ctx.inherited
        )),
    }
}

/// The child elements a list's rows `start..start + size` cover, as
/// `(first offset, last offset - first offset)`, or an error if an offset is
/// negative, the offsets decrease, or the last one is past the child's
/// `child_len` elements.
///
/// `DuckDB` reads the offsets as unsigned and builds each row's entry from
/// `offsets[i + 1] - offsets[i]`, then imports `offsets[size] - offsets[0]`
/// child elements from `offsets[0]` (`ConvertArrowListOffsetsTemplated`): a
/// negative or decreasing offset becomes an enormous length, and an offset
/// past the child is read past its buffers. Every offset is read, as `DuckDB`
/// reads every one.
///
/// # Safety
///
/// `node` must be a valid list whose offsets buffer covers rows
/// `start..=start + size`.
pub(super) unsafe fn list_range(
    node: &RawArrowArray,
    start: i64,
    size: u64,
    wide: bool,
    child_len: i64,
) -> Result<(i64, i64), String> {
    let size = i64::try_from(size).map_err(|_| "row count out of range".to_owned())?;
    let offset = |i: i64| {
        let at = start
            .checked_add(i)
            .ok_or_else(|| "list offsets index out of range".to_owned())?;
        // SAFETY: `at` is in `start..=start + size`, covered by the caller's
        // contract.
        unsafe { read_index(node, 1, at, wide) }.ok_or_else(|| "no offsets buffer".to_owned())
    };
    let first = offset(0)?;
    if first < 0 {
        return Err(format!(
            "list offset {first} of row {start} is negative, which DuckDB would read as an \
             enormous unsigned offset"
        ));
    }
    let mut last = first;
    for i in 1..=size {
        let next = offset(i)?;
        if next < last {
            return Err(format!(
                "list offsets decrease from {last} to {next} at row {}, which DuckDB would \
                 read as an enormous list length",
                start + i - 1
            ));
        }
        last = next;
    }
    if last > child_len {
        return Err(format!(
            "list offsets run from child element {first} to {last}, past the child's \
             {child_len} element(s), which DuckDB would read out of bounds"
        ));
    }
    let total = last
        .checked_sub(first)
        .ok_or_else(|| "list offsets overflow".to_owned())?;
    Ok((first, total))
}

/// Refuses a negative list-view offset or size.
///
/// `DuckDB` reads both as unsigned (`ConvertArrowListViewOffsetsTemplated`),
/// so a negative size is an enormous length added to the child size it
/// imports, and a negative offset an enormous start.
pub(super) fn list_view_entry(row: i64, offset: i64, size: i64) -> Result<(), String> {
    if size < 0 {
        return Err(format!(
            "list view size {size} at row {row} is negative, which DuckDB would read as an \
             enormous length"
        ));
    }
    if offset < 0 {
        return Err(format!(
            "list view offset {offset} at row {row} is negative, which DuckDB would read as an \
             enormous unsigned offset"
        ));
    }
    Ok(())
}

/// Refuses a dictionary index outside `0..values` in any row `DuckDB` uses:
/// rows `start..start + size` of `node`'s indices buffer, except the rows its
/// own validity marks NULL when `skip_nulls` (`DuckDB` points those at a NULL
/// sentinel; its selection takes every other index as it is,
/// `SetSelectionVectorLoop`, so an index past the dictionary is read past it).
///
/// A row that is NULL only because an enclosing struct's row is must still
/// hold an index inside the dictionary: the Arrow specification requires it
/// of every index that is valid in its own array.
///
/// # Safety
///
/// `node` must be a valid dictionary-encoded array whose indices buffer (and,
/// with `skip_nulls`, validity bitmap) covers rows `start..start + size`.
pub(super) unsafe fn dictionary_indices(
    node: &RawArrowArray,
    indices: Indices,
    start: i64,
    size: u64,
    values: i64,
    skip_nulls: bool,
) -> Result<(), String> {
    let buffer = |i: usize| {
        if node.buffers.is_null() || usize::try_from(node.n_buffers).ok()? <= i {
            return None;
        }
        // SAFETY: `i < n_buffers`, and a valid array's `buffers` holds them.
        let data = unsafe { *node.buffers.add(i) };
        (!data.is_null()).then_some(data)
    };
    let start = usize::try_from(start).map_err(|_| "dictionary index row out of range")?;
    let size = usize::try_from(size).map_err(|_| "row count out of range")?;
    let data = buffer(1).ok_or("a dictionary-encoded array with no indices buffer")?;
    let validity = if skip_nulls { buffer(0) } else { None };
    for row in start..start.saturating_add(size) {
        if let Some(bits) = validity {
            // SAFETY: the bitmap covers `row`, per the caller's contract.
            if (unsafe { *bits.cast::<u8>().add(row / 8) } >> (row % 8)) & 1 == 0 {
                continue;
            }
        }
        // SAFETY: the indices buffer covers `row`, per the caller's contract.
        let index = unsafe { indices.read(data, row) };
        if index < 0 || index >= i128::from(values) {
            return Err(format!(
                "dictionary index {index} at row {row} is outside the dictionary's {values} \
                 value(s), which DuckDB would read out of bounds"
            ));
        }
    }
    Ok(())
}
