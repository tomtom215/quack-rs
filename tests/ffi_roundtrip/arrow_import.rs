// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `arrow::data_chunk_from_arrow` against a live `DuckDB`: what it checks
//! before handing an array to `duckdb_data_chunk_from_arrow`, and which
//! layouts `DuckDB` actually converts.
//!
//! `duckdb_data_chunk_from_arrow` dereferences `arrow_array->children[i]`
//! once per converted-schema column with no null or length check
//! (`arrow-c.cpp`), so the structural checks quack-rs can make on the raw
//! record are made first.

use std::ffi::c_void;
use std::ptr;

use quack_rs::arrow::{
    data_chunk_from_arrow, data_chunk_to_arrow, schema_from_arrow, to_arrow_schema, ArrowArray,
    ArrowOptions, RawArrowArray,
};
use quack_rs::error_data::DuckDbErrorType;
use quack_rs::query::OwnedConnection;
use quack_rs::types::{LogicalType, TypeId};

use super::Fixture;

/// A release callback for records whose buffers the test owns.
unsafe extern "C" fn release_nothing(array: *mut RawArrowArray) {
    // SAFETY: called with a valid pointer; the specification requires the
    // callback to null its own `release`.
    unsafe { (*array).release = None };
}

/// A struct-array record with `n_children` children at `children`, and the
/// buffer list it points into (one null validity buffer). The caller keeps
/// the buffer list alive for as long as the record is in use; the records
/// release nothing, so the test owns and frees everything they point at.
fn parent(
    length: i64,
    n_children: i64,
    children: *mut *mut RawArrowArray,
) -> (RawArrowArray, Box<[*const c_void; 1]>) {
    let mut buffers = Box::new([ptr::null()]);
    let raw = RawArrowArray {
        length,
        null_count: 0,
        offset: 0,
        n_buffers: 1,
        n_children,
        buffers: buffers.as_mut_ptr(),
        children,
        dictionary: ptr::null_mut(),
        release: Some(release_nothing),
        private_data: ptr::null_mut(),
    };
    (raw, buffers)
}

/// An `int32` child of `length` rows over a buffer of `data_len`, with the
/// allocations it points into.
struct IntChild {
    raw: Box<RawArrowArray>,
    _buffers: Box<[*const c_void; 2]>,
    _data: Box<[i32]>,
}

fn int_child(length: i64, data_len: usize) -> IntChild {
    let data = vec![7_i32; data_len].into_boxed_slice();
    let mut buffers = Box::new([ptr::null(), data.as_ptr().cast()]);
    let raw = Box::new(RawArrowArray {
        length,
        null_count: 0,
        offset: 0,
        n_buffers: 2,
        n_children: 0,
        buffers: buffers.as_mut_ptr(),
        children: ptr::null_mut(),
        dictionary: ptr::null_mut(),
        release: Some(release_nothing),
        private_data: ptr::null_mut(),
    });
    IntChild {
        raw,
        _buffers: buffers,
        _data: data,
    }
}

/// Converts a one-column `INTEGER` schema on `con`.
fn integer_schema(con: &OwnedConnection) -> quack_rs::arrow::ArrowConvertedSchema {
    let options = ArrowOptions::from_connection(con).expect("options");
    let int = LogicalType::new(TypeId::Integer);
    let mut schema = to_arrow_schema(&options, &[("v", &int)]).expect("schema");
    // SAFETY: `con` is live.
    unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted")
}

fn import(con: &OwnedConnection, raw: RawArrowArray) -> Result<usize, String> {
    let converted = integer_schema(con);
    // SAFETY: the record's release callback may be called once; nothing else
    // holds it.
    let array = unsafe { ArrowArray::from_raw(raw) };
    // SAFETY: `con` is live and `converted` came from it.
    unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }
        .map(|chunk| chunk.size())
        .map_err(|e| {
            assert_eq!(e.error_type(), DuckDbErrorType::InvalidInput, "{e}");
            e.to_string()
        })
}

/// A struct array claiming a child but with a null `children` pointer:
/// `DuckDB` dereferenced it (`SIGSEGV`).
#[test]
fn a_null_children_pointer_is_refused() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let (raw, _buffers) = parent(4, 1, ptr::null_mut());
    let err = import(&con, raw).expect_err("no children to read");
    assert!(err.contains("children"), "{err}");
}

/// A null entry in `children`: `DuckDB` dereferenced it too.
#[test]
fn a_null_child_is_refused() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let mut kids: Box<[*mut RawArrowArray; 1]> = Box::new([ptr::null_mut()]);
    let (raw, _buffers) = parent(4, 1, kids.as_mut_ptr());
    let err = import(&con, raw).expect_err("child 0 is null");
    assert!(err.contains("child 0"), "{err}");
}

/// The specification requires every child of a struct array to hold at least
/// `offset + length` of the parent's rows; `DuckDB` reads that many without
/// checking.
#[test]
fn a_child_shorter_than_its_parent_is_refused() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let mut short = int_child(2, 2);
    let mut kids = Box::new([ptr::from_mut(&mut *short.raw)]);
    let (raw, _buffers) = parent(4096, 1, kids.as_mut_ptr());
    let err = import(&con, raw).expect_err("child too short");
    assert!(err.contains("child 0"), "{err}");

    // A well-formed array of the same shape imports.
    let mut full = int_child(4, 4);
    let mut full_kids = Box::new([ptr::from_mut(&mut *full.raw)]);
    let (raw, _full_buffers) = parent(4, 1, full_kids.as_mut_ptr());
    assert_eq!(import(&con, raw), Ok(4));
}

/// `DuckDB` imports child rows `0..length` whatever the parent struct
/// array's `offset`: in the C reproducer, 3 rows at offset 2 over a child
/// `[0, 10, …, 90]` came back as `0 10 20` instead of `20 30 40`, with no
/// error (1.4.4, 1.5.0 and 1.5.5). A nonzero offset is now refused before
/// `DuckDB` sees the array.
#[test]
fn a_nonzero_parent_offset_is_refused_not_imported_as_the_wrong_rows() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let mut child = int_child(10, 10);
    let mut kids = Box::new([ptr::from_mut(&mut *child.raw)]);
    let (mut raw, _buffers) = parent(3, 1, kids.as_mut_ptr());
    raw.offset = 2;
    let err = import(&con, raw).expect_err("offset 2 is refused");
    assert!(err.contains("has offset 2"), "{err}");
}

/// Exports one value of `expr` and imports it back, returning the imported
/// column's type and the value rendered by `DuckDB` (through a table the
/// chunk is appended to).
fn round_trip(con: &OwnedConnection, expr: &str) -> (TypeId, String) {
    let mut result = con.query(&format!("SELECT {expr} AS v")).expect("query");
    let ty = result.column_logical_type(0).expect("type");
    let options = ArrowOptions::from_connection(con).expect("options");
    let mut schema = to_arrow_schema(&options, &[("v", &ty)]).expect("schema");
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    let array = data_chunk_to_arrow(&options, &chunk).expect("export");
    // SAFETY: `con` is live and `converted` came from it.
    let back = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }.expect("import");
    // SAFETY: the chunk has one column and one row.
    let vector = unsafe { back.vector(0) };
    // SAFETY: `vector` belongs to the live chunk `back`.
    let back_type =
        unsafe { LogicalType::from_raw(libduckdb_sys::duckdb_vector_get_column_type(vector)) };
    // SAFETY: `back_type` is a live, owned logical type.
    let back_id = unsafe { back_type.get_type_id() };
    con.execute("DROP TABLE IF EXISTS rt").expect("drop");
    con.execute(&format!("CREATE TABLE rt (v {})", back_id.sql_name()))
        .expect("create");
    // SAFETY: `con` is live and the table exists.
    let appender =
        unsafe { quack_rs::appender::Appender::new(con.as_raw(), None, c"rt") }.expect("appender");
    appender.append_chunk(&back).expect("append");
    appender.close().expect("close");
    drop(appender);
    let mut text = con.query("SELECT v::VARCHAR FROM rt").expect("select");
    let chunk = text.next_chunk().expect("fetch").expect("one row");
    // SAFETY: one VARCHAR column, one row.
    (back_id, unsafe { chunk.reader(0).read_str(0).to_owned() })
}

/// Two Arrow round trips are lossy, as the Arrow docs in the book say.
#[test]
fn timetz_and_bit_do_not_round_trip_exactly() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let (ty, text) = round_trip(&con, "'01:02:03+05:30'::TIMETZ");
    assert_eq!(
        (ty, text.as_str()),
        (TypeId::Time, "01:02:03"),
        "offset lost"
    );
    let (ty, text) = round_trip(&con, "'10110'::BIT");
    assert_eq!(ty, TypeId::Blob, "BIT comes back as {text}");
    // The control: a type that does round-trip.
    let (ty, text) = round_trip(&con, "TIMESTAMP '2024-02-29 12:34:56'");
    assert_eq!(
        (ty, text.as_str()),
        (TypeId::Timestamp, "2024-02-29 12:34:56")
    );
}

/// The audit's F-V9a: the docs said dictionary-encoded children were
/// rejected with `NotImplemented`. `DuckDB` converts them
/// (`ColumnArrowToDuckDBDictionary`); an `ENUM` column exports as one.
#[test]
fn dictionary_encoded_children_are_converted() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let mut result = con
        .query("SELECT (['a','b','c'][i % 3 + 1])::ENUM('a','b','c') AS e FROM range(10) t(i)")
        .expect("query");
    let ty = result.column_logical_type(0).expect("type");
    let options = ArrowOptions::from_connection(&con).expect("options");
    let mut schema = to_arrow_schema(&options, &[("e", &ty)]).expect("schema");
    let child = schema.child(0).expect("one column");
    // SAFETY: `child` borrows a live schema record.
    let dictionary = unsafe { (*child.as_ptr()).dictionary };
    assert!(!dictionary.is_null(), "an ENUM exports dictionary-encoded");
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    let array = data_chunk_to_arrow(&options, &chunk).expect("export");
    // SAFETY: `con` is live and `converted` came from it.
    let back = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }
        .expect("dictionary arrays import");
    assert_eq!(back.size(), 10);
}

/// Runs `sql`, exports its first chunk to Arrow and imports it back.
fn export_import(con: &OwnedConnection, sql: &str) -> quack_rs::query::OwnedDataChunk {
    let mut result = con.query(sql).expect("query");
    let ty = result.column_logical_type(0).expect("type");
    let options = ArrowOptions::from_connection(con).expect("options");
    let mut schema = to_arrow_schema(&options, &[("v", &ty)]).expect("schema");
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    let array = data_chunk_to_arrow(&options, &chunk).expect("export");
    // SAFETY: `con` is live and `converted` came from it.
    unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }.expect("import")
}

/// What the ENUM queries below hold at `row`: `None` every seventh row, else
/// alternately a string longer than the 12-byte inline limit and a short one.
const fn enum_expected(row: usize) -> Option<&'static str> {
    if row % 7 == 3 {
        None
    } else if row % 2 == 0 {
        Some("bb-longer-than-twelve-bytes")
    } else {
        Some("aa")
    }
}

const ENUM_ROWS: &str = "(CASE WHEN i % 7 = 3 THEN NULL WHEN i % 2 = 0 \
     THEN 'bb-longer-than-twelve-bytes' ELSE 'aa' END)::ENUM('aa', 'bb-longer-than-twelve-bytes')";

/// The fourth audit's V3: `DuckDB` imports a dictionary-encoded column as a
/// *dictionary* vector (`vector.Slice` in `ColumnArrowToDuckDBDictionary`),
/// sized for the dictionary, not the chunk. Every `VectorReader` indexes the
/// data flat, so reading row `i` read entry `i` of a 3-entry buffer: wrong
/// values for every row, and a heap read out of bounds past row 2.
/// `data_chunk_from_arrow` now flattens such columns before returning them.
#[test]
fn a_dictionary_encoded_column_reads_back_row_for_row() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let back = export_import(
        &con,
        &format!("SELECT {ENUM_ROWS} AS v FROM range(2048) t(i)"),
    );
    assert_eq!(back.size(), 2048);
    // SAFETY: column 0 exists; the chunk outlives the reader.
    let reader = unsafe { back.reader(0) };
    for row in 0..back.size() {
        // SAFETY: `row < size`; strings are only read from valid rows.
        let got = unsafe { reader.is_valid(row).then(|| reader.read_str(row)) };
        assert_eq!(got, enum_expected(row), "row {row}");
    }
}

/// V3 inside a `STRUCT`: the struct's child is imported through the same
/// dictionary path (`arrow_conversion.cpp`, the struct case).
#[test]
fn a_dictionary_encoded_struct_field_reads_back_row_for_row() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let back = export_import(
        &con,
        &format!("SELECT {{'e': {ENUM_ROWS}}} AS v FROM range(2048) t(i)"),
    );
    // SAFETY: column 0 is a one-field STRUCT; the chunk outlives the reader.
    let field = unsafe { back.struct_field_reader(0, 0) };
    for row in 0..back.size() {
        // SAFETY: `row < size`; strings are only read from valid rows.
        let got = unsafe { field.is_valid(row).then(|| field.read_str(row)) };
        assert_eq!(got, enum_expected(row), "row {row}");
    }
}

/// V3 inside a `LIST`: each row holds `[e, e]`, so the child has twice as
/// many entries as the chunk has rows — 2048 here, the most `DuckDB` can
/// import with NULLs (see the next test).
#[test]
fn a_dictionary_encoded_list_child_reads_back_row_for_row() {
    use quack_rs::vector::complex::ListVector;
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let back = export_import(
        &con,
        &format!("SELECT [{ENUM_ROWS}, {ENUM_ROWS}] AS v FROM range(1024) t(i)"),
    );
    assert_eq!(back.size(), 1024);
    // SAFETY: column 0 exists.
    let list = unsafe { back.vector(0) };
    // SAFETY: `list` is a live LIST vector of the chunk.
    let child = unsafe { ListVector::child_reader(list, ListVector::get_size(list)) };
    for row in 0..back.size() {
        // SAFETY: `row < size`.
        let entry = unsafe { ListVector::get_entry(list, row) };
        assert_eq!(entry.length, 2, "row {row}");
        for k in 0..2 {
            let at = usize::try_from(entry.offset).unwrap() + k;
            // SAFETY: `at` is inside the child; strings are only read from valid rows.
            let got = unsafe { child.is_valid(at).then(|| child.read_str(at)) };
            assert_eq!(got, enum_expected(row), "row {row} element {k}");
        }
    }
}

/// Found while fixing V3: `DuckDB` imports a dictionary-encoded array with
/// NULLs and more than `STANDARD_VECTOR_SIZE` (2048) entries by writing past a
/// heap buffer (`ColumnArrowToDuckDBDictionary` copies the indices' validity
/// into a 2048-row `ValidityMask`). A 2048-row chunk of `[e, e]` has a
/// 4096-entry dictionary child, and before the check this test's import
/// corrupted the heap (glibc: `realloc(): invalid next size`; valgrind: an
/// invalid write 0 bytes after a 256-byte block in `GetValidityMask`).
#[test]
fn a_dictionary_array_duckdb_would_overflow_on_is_refused() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let mut result = con
        .query(&format!(
            "SELECT [{ENUM_ROWS}, {ENUM_ROWS}] AS v FROM range(2048) t(i)"
        ))
        .expect("query");
    let ty = result.column_logical_type(0).expect("type");
    let options = ArrowOptions::from_connection(&con).expect("options");
    let mut schema = to_arrow_schema(&options, &[("v", &ty)]).expect("schema");
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");
    let chunk = result.next_chunk().expect("fetch").expect("one chunk");
    let array = data_chunk_to_arrow(&options, &chunk).expect("export");
    // SAFETY: `con` is live and `converted` came from it.
    let err = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }
        .expect_err("a 4096-entry dictionary child with NULLs");
    assert_eq!(err.error_type(), DuckDbErrorType::InvalidInput);
    let message = err.message().unwrap_or_default();
    assert!(
        message.contains("4096 rows that can hold NULLs"),
        "{message}"
    );
}

/// An Arrow null-type column imports as a *constant* NULL vector
/// (`vector.Reference(Value())` in `ColumnArrowToDuckDB`); it now reads as
/// NULL in every row like any flat vector. The array is built by hand: an
/// Arrow null array has no buffers at all.
#[test]
fn a_null_type_column_reads_back_null_in_every_row() {
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let options = ArrowOptions::from_connection(&con).expect("options");
    let null_type = LogicalType::new(TypeId::SqlNull);
    let mut schema = to_arrow_schema(&options, &[("v", &null_type)]).expect("schema");
    assert_eq!(schema.child(0).and_then(|c| c.format()), Some("n"));
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");

    let mut child = Box::new(RawArrowArray {
        length: 2048,
        null_count: 2048,
        offset: 0,
        n_buffers: 0,
        n_children: 0,
        buffers: ptr::null_mut(),
        children: ptr::null_mut(),
        dictionary: ptr::null_mut(),
        release: Some(release_nothing),
        private_data: ptr::null_mut(),
    });
    let mut children = [ptr::from_mut(&mut *child)];
    let (raw, _buffers) = parent(2048, 1, children.as_mut_ptr());
    // SAFETY: every pointer in `raw` refers to a local that outlives the
    // import; `release_nothing` frees nothing.
    let array = unsafe { ArrowArray::from_raw(raw) };
    // SAFETY: `con` is live, `converted` came from it, and the array
    // conforms to its one null-type column.
    let back = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }.expect("import");
    assert_eq!(back.size(), 2048);
    // SAFETY: column 0 exists; the chunk outlives the reader.
    let reader = unsafe { back.reader(0) };
    for row in 0..back.size() {
        // SAFETY: `row < size`.
        assert!(!unsafe { reader.is_valid(row) }, "row {row}");
    }
}

/// Releases of records built by `claimed_by_column`, per test.
unsafe extern "C" fn count_release(array: *mut RawArrowArray) {
    // SAFETY: `private_data` is the test's live counter.
    unsafe {
        (*(*array)
            .private_data
            .cast::<std::sync::atomic::AtomicUsize>())
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (*array).release = None;
    }
}

/// Whether the producer's release has run once the chunk is dropped while a
/// vector still references column `column` of it.
#[cfg(feature = "duckdb-1-5")]
fn released_while_column_is_referenced(column: usize) -> bool {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fx = Fixture::open();
    // SAFETY: the fixture's database outlives the connection.
    let con = unsafe { OwnedConnection::open(fx.db()) }.expect("connect");
    let options = ArrowOptions::from_connection(&con).expect("options");
    let int = LogicalType::new(TypeId::Integer);
    let mut schema = to_arrow_schema(&options, &[("a", &int), ("b", &int)]).expect("schema");
    // SAFETY: `con` is live.
    let converted = unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.expect("converted");
    let (a, b) = (int_child(2, 2), int_child(2, 2));
    let mut children = [
        ptr::from_ref(&*a.raw).cast_mut(),
        ptr::from_ref(&*b.raw).cast_mut(),
    ];
    let (mut raw, _buffers) = parent(2, 2, children.as_mut_ptr());
    let releases = AtomicUsize::new(0);
    raw.private_data = ptr::from_ref(&releases).cast_mut().cast();
    raw.release = Some(count_release);
    // SAFETY: the record's release may run once; nothing else holds it.
    let array = unsafe { ArrowArray::from_raw(raw) };
    // SAFETY: `con` is live and `converted` came from it.
    let chunk = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }.expect("import");
    let target = quack_rs::vector::OwnedVector::new(&int, 2).expect("vector");
    // SAFETY: both vectors are INTEGER; `target` is not read after this.
    unsafe { quack_rs::vector::ops::reference_vector(target.as_raw(), chunk.vector(column)) };
    drop(chunk);
    let freed = releases.load(Ordering::SeqCst) == 1;
    drop(target);
    assert_eq!(releases.load(Ordering::SeqCst), 1, "released exactly once");
    freed
}

/// `DuckDB` gives the array's `release` to column 0 alone: every column's
/// state holds a copy of the record, but `release` is nulled after the
/// first. A vector referencing column 0 keeps the producer's buffers; one
/// referencing column 1 does not.
#[cfg(feature = "duckdb-1-5")]
#[test]
fn only_the_first_column_holds_the_claim_on_the_arrow_array() {
    assert!(!released_while_column_is_referenced(0));
    assert!(released_while_column_is_referenced(1));
}
