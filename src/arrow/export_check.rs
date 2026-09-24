// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The values `duckdb_data_chunk_to_arrow` exports as different values, with
//! no error (`docs/upstream-duckdb-reports.md`, item 16), found before the
//! export so that [`data_chunk_to_arrow`][super::data_chunk_to_arrow] can
//! refuse the chunk:
//!
//! - an `INTERVAL`, exported as Arrow's `month_day_nano`, whose microseconds
//!   times 1000 overflow an `i64` (`DuckDB` multiplies without a check);
//! - a `HUGEINT` or `UHUGEINT` exported as `decimal128(38, 0)` — always for
//!   `UHUGEINT`, and for `HUGEINT` unless `arrow_lossless_conversion` is set —
//!   with more than 38 digits, which `decimal128(38, 0)` cannot hold (a
//!   `UHUGEINT` of 2^127 or more comes back negative).
//!
//! Which rules apply is read from the Arrow format `DuckDB` would use for
//! each type under the same options. Only columns whose type contains an
//! affected type are examined: each is copied flat (a chunk's vectors may be
//! dictionary or constant vectors, which the C API cannot index) and walked
//! through `LIST`, `MAP`, `STRUCT`, `UNION` and `ARRAY`, checking only rows
//! that are valid under valid parents.

use libduckdb_sys::{
    duckdb_array_type_array_size, duckdb_interval, duckdb_list_entry, duckdb_logical_type,
    duckdb_validity_row_is_valid, duckdb_vector, duckdb_vector_get_column_type,
    duckdb_vector_get_data, duckdb_vector_get_validity, idx_t,
};

use super::{to_arrow_schema, ArrowOptions, ArrowSchema, RawArrowArray};
use crate::data_chunk::DataChunk;
use crate::error_data::{DuckDbErrorType, ErrorData};
use crate::selection_vector::SelectionVector;
use crate::types::{LogicalType, TypeId};
use crate::vector::complex::{ArrayVector, ListVector, MapVector, StructVector};
use crate::vector::ops::{copy_selected, OwnedVector};

/// The largest magnitude `decimal128(38, 0)` holds: `10^38 - 1`.
const DECIMAL38_MAX: u128 = 10_u128.pow(38) - 1;

/// Whether an interval with `micros` microseconds exports exactly as
/// `month_day_nano`, whose nanoseconds are `micros * 1000` in an `i64`.
pub(super) const fn interval_exports_exactly(micros: i64) -> bool {
    micros.checked_mul(1000).is_some()
}

/// Whether a `HUGEINT` fits `decimal128(38, 0)`.
pub(super) const fn hugeint_fits_decimal38(value: i128) -> bool {
    value.unsigned_abs() <= DECIMAL38_MAX
}

/// Whether a `UHUGEINT` fits `decimal128(38, 0)`.
pub(super) const fn uhugeint_fits_decimal38(value: u128) -> bool {
    value <= DECIMAL38_MAX
}

/// Whether an Arrow format string is a decimal (`d:precision,scale[,bits]`).
pub(super) fn is_decimal_format(format: &str) -> bool {
    format.starts_with("d:")
}

/// Which of the affected types this export converts lossily.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rules {
    /// `INTERVAL` is exported as `month_day_nano`.
    pub interval: bool,
    /// `HUGEINT` is exported as a decimal.
    pub hugeint: bool,
    /// `UHUGEINT` is exported as a decimal.
    pub uhugeint: bool,
}

impl Rules {
    /// Whether a value of type `id` is checked under these rules.
    pub(super) const fn applies(self, id: Option<TypeId>) -> bool {
        match id {
            Some(TypeId::Interval) => self.interval,
            Some(TypeId::HugeInt) => self.hugeint,
            Some(TypeId::UHugeInt) => self.uhugeint,
            _ => false,
        }
    }

    /// Reads the rules from the formats `DuckDB` gives the affected types
    /// under `options`.
    fn of(options: &ArrowOptions<'_>) -> Result<Self, ErrorData> {
        let interval = LogicalType::new(TypeId::Interval);
        let hugeint = LogicalType::new(TypeId::HugeInt);
        let uhugeint = LogicalType::new(TypeId::UHugeInt);
        let schema = to_arrow_schema(
            options,
            &[("i", &interval), ("h", &hugeint), ("u", &uhugeint)],
        )?;
        let format = |i| {
            schema
                .child(i)
                .and_then(super::ArrowSchema::format)
                .unwrap_or("")
        };
        Ok(Self {
            interval: format(0) == "tin",
            hugeint: is_decimal_format(format(1)),
            uhugeint: is_decimal_format(format(2)),
        })
    }
}

/// Whether row `row` of a vector whose validity mask is `validity` is valid.
///
/// # Safety
///
/// `validity` is null or the mask of a vector with more than `row` rows.
unsafe fn is_valid(validity: *mut u64, row: usize) -> bool {
    // SAFETY: forwarded from this function's contract; a null mask means
    // every row is valid.
    validity.is_null() || unsafe { duckdb_validity_row_is_valid(validity, row as idx_t) }
}

/// The child rows of the valid `rows` of a `LIST` or `MAP` with `entries`.
pub(super) fn list_child_rows(
    entries: &[duckdb_list_entry],
    rows: &[usize],
    valid: impl Fn(usize) -> bool,
) -> Vec<usize> {
    rows.iter()
        .filter(|&&row| valid(row))
        .flat_map(|&row| {
            let entry = entries[row];
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a list entry indexes a child vector in memory, so it fits a usize"
            )]
            (entry.offset..entry.offset + entry.length).map(|i| i as usize)
        })
        .collect()
}

/// The child rows of the valid `rows` of an `ARRAY` of `size` elements.
pub(super) fn array_child_rows(
    size: usize,
    rows: &[usize],
    valid: impl Fn(usize) -> bool,
) -> Vec<usize> {
    rows.iter()
        .filter(|&&row| valid(row))
        .flat_map(|&row| row * size..(row + 1) * size)
        .collect()
}

/// Whether `ty` contains, at any depth, a type `rules` check.
///
/// # Safety
///
/// `ty` must be a live logical type.
unsafe fn contains_checked(ty: duckdb_logical_type, rules: Rules) -> bool {
    // SAFETY: `ty` is live per this function's contract.
    unsafe {
        crate::table::type_check::contains_type(ty, &|raw| {
            rules.applies(TypeId::try_from_duckdb_type(raw))
        })
    }
}

/// The first value in `rows` of the flat `vector` that `rules` refuse, as a
/// description.
///
/// # Safety
///
/// `vector` must be a live flat vector (with flat children) whose rows
/// include every index in `rows`.
unsafe fn check_vector(vector: duckdb_vector, rows: &[usize], rules: Rules) -> Option<String> {
    use libduckdb_sys::{
        DUCKDB_TYPE_DUCKDB_TYPE_ARRAY as ARRAY, DUCKDB_TYPE_DUCKDB_TYPE_LIST as LIST,
        DUCKDB_TYPE_DUCKDB_TYPE_MAP as MAP, DUCKDB_TYPE_DUCKDB_TYPE_STRUCT as STRUCT,
        DUCKDB_TYPE_DUCKDB_TYPE_UNION as UNION,
    };
    if rows.is_empty() {
        return None;
    }
    // SAFETY: `vector` is live; the returned type is owned by `ty`.
    let ty = unsafe { LogicalType::from_raw(duckdb_vector_get_column_type(vector)) };
    // SAFETY: `ty` is live.
    if !unsafe { contains_checked(ty.as_raw(), rules) } {
        return None;
    }
    // SAFETY: `vector` is live.
    let validity = unsafe { duckdb_vector_get_validity(vector) };
    // SAFETY: every row checked is one of `rows`, which the vector has.
    let valid = |row: usize| unsafe { is_valid(validity, row) };
    // SAFETY: `ty` is live.
    let raw_id = unsafe { libduckdb_sys::duckdb_get_type_id(ty.as_raw()) };
    // SAFETY: in every arm, `vector` is a live flat vector of type `ty` holding
    // every row in `rows`, and each child handle and child row index comes
    // from `DuckDB`'s own layout of that vector.
    unsafe {
        match raw_id {
            LIST | MAP => {
                let entries = std::slice::from_raw_parts(
                    duckdb_vector_get_data(vector).cast::<duckdb_list_entry>(),
                    rows.iter().max().map_or(0, |&m| m + 1),
                );
                let child_rows = list_child_rows(entries, rows, valid);
                let child = if raw_id == MAP {
                    MapVector::struct_child(vector)
                } else {
                    ListVector::get_child(vector)
                };
                check_vector(child, &child_rows, rules)
            }
            STRUCT | UNION => {
                let valid_rows: Vec<usize> = rows.iter().copied().filter(|&r| valid(r)).collect();
                let count =
                    usize::try_from(libduckdb_sys::duckdb_struct_type_child_count(ty.as_raw()))
                        .unwrap_or(0);
                (0..count).find_map(|i| {
                    check_vector(StructVector::get_child(vector, i), &valid_rows, rules)
                })
            }
            ARRAY => {
                let size = usize::try_from(duckdb_array_type_array_size(ty.as_raw())).unwrap_or(0);
                let child_rows = array_child_rows(size, rows, valid);
                check_vector(ArrayVector::get_child(vector), &child_rows, rules)
            }
            _ => {
                let id = TypeId::try_from_duckdb_type(raw_id);
                let data = duckdb_vector_get_data(vector);
                rows.iter().copied().filter(|&r| valid(r)).find_map(|row| match id {
                    Some(TypeId::Interval) => {
                        let v = *data.cast::<duckdb_interval>().add(row);
                        (!interval_exports_exactly(v.micros)).then(|| {
                            format!(
                                "an INTERVAL of {} microseconds, whose nanoseconds overflow \
                                 Arrow's i64",
                                v.micros
                            )
                        })
                    }
                    Some(TypeId::HugeInt) => {
                        // `hugeint_t` is 8-byte aligned; `i128` wants 16.
                        let v = data.cast::<i128>().add(row).read_unaligned();
                        (!hugeint_fits_decimal38(v)).then(|| {
                            format!("the HUGEINT {v}, which has more digits than decimal128(38, 0) holds")
                        })
                    }
                    Some(TypeId::UHugeInt) => {
                        // `hugeint_t` is 8-byte aligned; `u128` wants 16.
                        let v = data.cast::<u128>().add(row).read_unaligned();
                        (!uhugeint_fits_decimal38(v)).then(|| {
                            format!(
                                "the UHUGEINT {v}, which has more digits than decimal128(38, 0) holds"
                            )
                        })
                    }
                    _ => None,
                })
            }
        }
    }
}

/// Refuses `chunk` if `duckdb_data_chunk_to_arrow` would export one of its
/// values as a different value; see the [module docs](self).
///
/// # Errors
///
/// [`DuckDbErrorType::InvalidInput`] naming the column and the value, or
/// whatever `DuckDB` reports while the check copies a column or reads the
/// export's formats.
pub(super) fn check_chunk(options: &ArrowOptions<'_>, chunk: &DataChunk) -> Result<(), ErrorData> {
    let rules = Rules::of(options)?;
    if rules
        == (Rules {
            interval: false,
            hugeint: false,
            uhugeint: false,
        })
    {
        return Ok(());
    }
    let size = chunk.size();
    if size == 0 {
        return Ok(());
    }
    for column in 0..chunk.column_count() {
        // SAFETY: `column` is in range for the chunk.
        let source = unsafe { chunk.vector(column) };
        // SAFETY: `source` is a live vector; the type is owned by `ty`.
        let ty = unsafe { LogicalType::from_raw(duckdb_vector_get_column_type(source)) };
        // SAFETY: `ty` is live.
        if !unsafe { contains_checked(ty.as_raw(), rules) } {
            continue;
        }
        let failed = |e: crate::error::ExtensionError| {
            ErrorData::new(
                DuckDbErrorType::InvalidInput,
                &format!("data_chunk_to_arrow: copying column {column} to check it: {e}"),
            )
        };
        let flat = OwnedVector::new(&ty, size).map_err(failed)?;
        let mut identity = SelectionVector::new(size).map_err(failed)?;
        for (i, slot) in identity.as_mut_slice().iter_mut().enumerate() {
            // `size` is at most a vector's capacity, far below `u32::MAX`.
            *slot = u32::try_from(i).unwrap_or(u32::MAX);
        }
        let rows: Vec<usize> = (0..size).collect();
        // SAFETY: `source` and `flat` have the same type, `flat` has room for
        // `size` rows, and the identity selection names rows `0..size` of the
        // chunk. The copy is flat, children included, so the walk may index
        // it directly.
        let found = unsafe {
            copy_selected(source, flat.as_raw(), &identity, size, 0, 0);
            check_vector(flat.as_raw(), &rows, rules)
        };
        if let Some(what) = found {
            return Err(ErrorData::new(
                DuckDbErrorType::InvalidInput,
                &format!(
                    "data_chunk_to_arrow: column {column} holds {what}. DuckDB would export it \
                     as a different value without an error (docs/upstream-duckdb-reports.md, \
                     item 16), so the chunk is refused"
                ),
            ));
        }
    }
    Ok(())
}

/// Buffers a node of Arrow `format` must have, when the format fixes it and
/// `DuckDB` has been seen to write another count: plain (non-view) binary and
/// UTF-8, which have validity, offsets and data.
fn expected_buffers(format: &str) -> Option<i64> {
    matches!(format, "z" | "Z" | "u" | "U").then_some(3)
}

/// The first node of `array` whose buffer count contradicts the format its
/// `schema` declares, as a path of child indices.
///
/// # Safety
///
/// `array` must be a valid Arrow array whose tree mirrors `schema`'s (the
/// same node counts, child for child), as `DuckDB` exports them.
unsafe fn layout_mismatch(schema: &ArrowSchema, array: &RawArrowArray) -> Option<String> {
    let format = schema.format().unwrap_or("");
    if let Some(want) = expected_buffers(format) {
        if array.n_buffers != want {
            return Some(format!(
                "format {format:?} declares {want} buffers, the array has {}",
                array.n_buffers
            ));
        }
    }
    let children = usize::try_from(array.n_children).unwrap_or(0);
    for i in 0..children.min(schema.child_count()) {
        let (Some(child_schema), false) = (schema.child(i), array.children.is_null()) else {
            continue;
        };
        // SAFETY: `i < n_children` and a valid array's children are live.
        let Some(child) = (unsafe { (*array.children.add(i)).as_ref() }) else {
            continue;
        };
        // SAFETY: the child mirrors `child_schema` as its parent mirrors `schema`.
        if let Some(found) = unsafe { layout_mismatch(child_schema, child) } {
            return Some(format!("child {i}: {found}"));
        }
    }
    // SAFETY: a valid array's dictionary is null or live, and mirrors the
    // schema's.
    if let (Some(values), Some(dict)) = (schema.dictionary(), unsafe { array.dictionary.as_ref() })
    {
        // SAFETY: as above.
        return unsafe { layout_mismatch(values, dict) }
            .map(|found| format!("dictionary: {found}"));
    }
    None
}

/// Checks `array`, just exported from `chunk` under `options`, against the
/// schema `DuckDB` declares for the chunk's types: before 1.5.5, `BIGNUM` (and
/// from 1.5.0 `GEOMETRY`) under `arrow_output_version = '1.4'` were written as
/// binary views (four buffers) while the schema declared plain binary (three),
/// which an importer reads as offsets (`docs/upstream-duckdb-reports.md`,
/// item 34).
///
/// # Safety
///
/// `array` must be what `duckdb_data_chunk_to_arrow` produced from `chunk`
/// under `options`.
pub(super) unsafe fn check_layout(
    options: &ArrowOptions<'_>,
    chunk: &DataChunk,
    array: &super::ArrowArray,
) -> Result<(), ErrorData> {
    let types: Vec<LogicalType> = (0..chunk.column_count())
        .map(|column| {
            // SAFETY: `column` is in range; the returned type is owned.
            unsafe { LogicalType::from_raw(duckdb_vector_get_column_type(chunk.vector(column))) }
        })
        .collect();
    let names: Vec<String> = (0..types.len()).map(|i| format!("c{i}")).collect();
    let columns: Vec<(&str, &LogicalType)> =
        names.iter().map(String::as_str).zip(types.iter()).collect();
    let schema = to_arrow_schema(options, &columns)?;
    // SAFETY: `array.as_ptr()` is live for `array`'s lifetime.
    let raw = unsafe { &*array.as_ptr() };
    // SAFETY: DuckDB exported `raw` from these types, so it mirrors `schema`.
    unsafe { layout_mismatch(&schema, raw) }.map_or(Ok(()), |found| {
        Err(ErrorData::new(
            DuckDbErrorType::InvalidInput,
            &format!(
                "data_chunk_to_arrow: the exported array contradicts its declared schema ({found}). \
                 DuckDB before 1.5.5 writes BIGNUM and GEOMETRY as binary views under \
                 arrow_output_version = '1.4' while declaring plain binary \
                 (docs/upstream-duckdb-reports.md, item 34); set arrow_output_version = '1.0' \
                 or upgrade DuckDB"
            ),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn keep_schema(schema: *mut crate::arrow::RawArrowSchema) {
        // SAFETY: called with a live record; this test owns everything it
        // points at.
        unsafe { (*schema).release = None };
    }

    /// A struct schema with one child of `format`, and a struct array whose
    /// child has `buffers` buffers; `layout_mismatch`'s verdict on the pair.
    fn mismatch(format: &str, buffers: i64) -> Option<String> {
        use crate::arrow::RawArrowSchema;
        let formats = [
            std::ffi::CString::new("+s").expect("no NUL"),
            std::ffi::CString::new(format).expect("no NUL"),
        ];
        let mut child_schema = RawArrowSchema::empty();
        child_schema.format = formats[1].as_ptr();
        child_schema.release = Some(keep_schema);
        let mut schema_children = [std::ptr::from_mut(&mut child_schema)];
        let mut root = RawArrowSchema::empty();
        root.format = formats[0].as_ptr();
        root.n_children = 1;
        root.children = schema_children.as_mut_ptr();
        root.release = Some(keep_schema);
        // SAFETY: `root` and everything it points at outlive `schema`, and
        // `keep_schema` frees nothing.
        let schema = unsafe { ArrowSchema::from_raw(root) };

        let mut child = RawArrowArray::empty();
        child.n_buffers = buffers;
        let mut array_children = [std::ptr::from_mut(&mut child)];
        let mut array = RawArrowArray::empty();
        array.n_children = 1;
        array.children = array_children.as_mut_ptr();
        // SAFETY: `array` mirrors `schema` and its child pointer is live.
        unsafe { layout_mismatch(&schema, &array) }
    }

    #[test]
    fn an_array_whose_buffer_count_contradicts_its_declared_format_is_found() {
        assert_eq!(mismatch("z", 3), None);
        assert_eq!(mismatch("u", 3), None);
        assert_eq!(mismatch("vz", 4), None);
        assert_eq!(mismatch("i", 2), None);
        let found = mismatch("z", 4).expect("BIGNUM before 1.5.5");
        assert!(
            found.contains("child 0") && found.contains("3 buffers"),
            "{found}"
        );
        assert!(mismatch("U", 2).is_some());
    }

    #[test]
    fn an_interval_exports_exactly_while_its_nanoseconds_fit_an_i64() {
        let edge = i64::MAX / 1000;
        assert!(interval_exports_exactly(edge));
        assert!(interval_exports_exactly(-edge));
        assert!(interval_exports_exactly(0));
        assert!(!interval_exports_exactly(edge + 1));
        assert!(!interval_exports_exactly(-edge - 1));
        assert!(!interval_exports_exactly(i64::MIN));
    }

    #[test]
    fn decimal128_holds_38_digits() {
        let max = 10_i128.pow(38) - 1;
        assert!(hugeint_fits_decimal38(max));
        assert!(hugeint_fits_decimal38(-max));
        assert!(!hugeint_fits_decimal38(max + 1));
        assert!(!hugeint_fits_decimal38(-max - 1));
        assert!(!hugeint_fits_decimal38(i128::MIN));
        assert!(uhugeint_fits_decimal38(max.unsigned_abs()));
        assert!(!uhugeint_fits_decimal38(max.unsigned_abs() + 1));
        assert!(!uhugeint_fits_decimal38(1 << 127));
        assert!(!uhugeint_fits_decimal38(u128::MAX));
    }

    #[test]
    fn a_decimal_format_is_d_colon() {
        assert!(is_decimal_format("d:38,0"));
        assert!(is_decimal_format("d:38,0,128"));
        assert!(!is_decimal_format("w:16"));
        assert!(!is_decimal_format("tin"));
        assert!(!is_decimal_format(""));
    }

    #[test]
    fn rules_apply_only_to_the_types_they_name() {
        let all = Rules {
            interval: true,
            hugeint: true,
            uhugeint: true,
        };
        let none = Rules {
            interval: false,
            hugeint: false,
            uhugeint: false,
        };
        for id in [TypeId::Interval, TypeId::HugeInt, TypeId::UHugeInt] {
            assert!(all.applies(Some(id)), "{id:?}");
            assert!(!none.applies(Some(id)), "{id:?}");
        }
        let only_hugeint = Rules {
            interval: false,
            hugeint: true,
            uhugeint: false,
        };
        assert!(only_hugeint.applies(Some(TypeId::HugeInt)));
        assert!(!only_hugeint.applies(Some(TypeId::UHugeInt)));
        assert!(!only_hugeint.applies(Some(TypeId::Interval)));
        for id in [
            Some(TypeId::BigInt),
            Some(TypeId::Decimal),
            Some(TypeId::List),
            None,
        ] {
            assert!(!all.applies(id), "{id:?}");
        }
    }

    #[test]
    fn child_rows_come_from_valid_parents_only() {
        let entries = [
            duckdb_list_entry {
                offset: 0,
                length: 2,
            },
            duckdb_list_entry {
                offset: 2,
                length: 0,
            },
            duckdb_list_entry {
                offset: 2,
                length: 3,
            },
        ];
        assert_eq!(
            list_child_rows(&entries, &[0, 1, 2], |_| true),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(list_child_rows(&entries, &[0, 1, 2], |r| r != 2), [0, 1]);
        assert_eq!(list_child_rows(&entries, &[2], |_| true), [2, 3, 4]);
        assert_eq!(array_child_rows(3, &[0, 2], |_| true), [0, 1, 2, 6, 7, 8]);
        assert_eq!(array_child_rows(3, &[0, 2], |r| r == 2), [6, 7, 8]);
        assert_eq!(array_child_rows(0, &[0, 1], |_| true), [] as [usize; 0]);
    }
}
