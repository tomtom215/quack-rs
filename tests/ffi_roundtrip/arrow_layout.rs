// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Valid Arrow layouts `duckdbdata_chunk_from_arrow` imports wrongly,
//! refused by `data_chunk_from_arrow` (`src/arrow/import_layout.rs`;
//! `docs/upstream-duckdb-reports.md`, items 9 and 24 to 28), each next to a
//! neighbouring layout that `DuckDB` imports correctly and that must still be
//! accepted, with the values the Arrow specification gives.
//!
//! The arrays are built by hand: `DuckDB`'s own export never produces these
//! layouts, but other producers do (a slice of a nested array keeps its
//! offsets below the top level).

use std::any::Any;
use std::ffi::{c_void, CString};
use std::ptr;

use quack_rs::arrow::{
    data_chunk_from_arrow, schema_from_arrow, ArrowArray, ArrowSchema, RawArrowArray,
    RawArrowSchema,
};
use quack_rs::error_data::DuckDbErrorType;
use quack_rs::query::{OwnedConnection, OwnedDataChunk};

use super::Fixture;

unsafe extern "C" fn release_array(array: *mut RawArrowArray) {
    // SAFETY: called with a valid pointer; the specification requires the
    // callback to null its own `release`. The test owns every buffer.
    unsafe { (*array).release = None };
}

unsafe extern "C" fn release_schema(schema: *mut RawArrowSchema) {
    // SAFETY: as above.
    unsafe { (*schema).release = None };
}

/// A schema node that owns its strings and children.
#[allow(
    clippy::vec_box,
    reason = "each node is built boxed, and its parent's pointers are taken before the move into this vector"
)]
struct Sch {
    raw: RawArrowSchema,
    strings: [CString; 2],
    children: Vec<Box<Self>>,
    child_ptrs: Vec<*mut RawArrowSchema>,
    dictionary: Option<Box<Self>>,
}

#[allow(clippy::vec_box, reason = "as on `Sch`")]
fn sch(format: &str, children: Vec<Box<Sch>>) -> Box<Sch> {
    let strings = [
        CString::new(format).expect("format"),
        CString::new(format!("f{}", children.len())).expect("name"),
    ];
    let mut raw = RawArrowSchema::empty();
    raw.format = strings[0].as_ptr();
    raw.name = strings[1].as_ptr();
    raw.flags = 2; // nullable
    raw.release = Some(release_schema);
    let mut node = Box::new(Sch {
        raw,
        strings,
        children,
        child_ptrs: Vec::new(),
        dictionary: None,
    });
    node.child_ptrs = node.children.iter_mut().map(|c| &raw mut c.raw).collect();
    node.raw.n_children = node.child_ptrs.len() as i64;
    node.raw.children = if node.child_ptrs.is_empty() {
        ptr::null_mut()
    } else {
        node.child_ptrs.as_mut_ptr()
    };
    node
}

/// `node` with the field name `name`.
fn named(name: &str, mut node: Box<Sch>) -> Box<Sch> {
    node.strings[1] = CString::new(name).expect("name");
    node.raw.name = node.strings[1].as_ptr();
    node
}

fn dict_sch(indices: &str, values: Box<Sch>) -> Box<Sch> {
    let mut node = sch(indices, vec![]);
    node.dictionary = Some(values);
    node.raw.dictionary = &raw mut node.dictionary.as_mut().expect("just set").raw;
    node
}

/// An array node that owns its buffers and children.
#[allow(
    clippy::vec_box,
    reason = "each node is built boxed, and its parent's pointers are taken before the move into this vector"
)]
struct Arr {
    raw: RawArrowArray,
    data: Vec<Vec<u8>>,
    buffers: Vec<*const c_void>,
    children: Vec<Box<Self>>,
    child_ptrs: Vec<*mut RawArrowArray>,
    dictionary: Option<Box<Self>>,
}

/// `buffers[i]` empty means a null buffer.
#[allow(clippy::vec_box, reason = "as on `Arr`")]
fn arr(
    length: i64,
    offset: i64,
    null_count: i64,
    data: Vec<Vec<u8>>,
    children: Vec<Box<Arr>>,
) -> Box<Arr> {
    let mut raw = RawArrowArray::empty();
    raw.length = length;
    raw.offset = offset;
    raw.null_count = null_count;
    raw.release = Some(release_array);
    let mut node = Box::new(Arr {
        raw,
        data,
        buffers: Vec::new(),
        children,
        child_ptrs: Vec::new(),
        dictionary: None,
    });
    node.buffers = node
        .data
        .iter()
        .map(|b| {
            if b.is_empty() {
                ptr::null()
            } else {
                b.as_ptr().cast()
            }
        })
        .collect();
    node.raw.n_buffers = node.buffers.len() as i64;
    node.raw.buffers = node.buffers.as_mut_ptr();
    node.child_ptrs = node.children.iter_mut().map(|c| &raw mut c.raw).collect();
    node.raw.n_children = node.child_ptrs.len() as i64;
    node.raw.children = if node.child_ptrs.is_empty() {
        ptr::null_mut()
    } else {
        node.child_ptrs.as_mut_ptr()
    };
    node
}

fn with_dict(mut node: Box<Arr>, values: Box<Arr>) -> Box<Arr> {
    node.dictionary = Some(values);
    node.raw.dictionary = &raw mut node.dictionary.as_mut().expect("just set").raw;
    node
}

fn bytes<T: Copy>(v: &[T]) -> Vec<u8> {
    // SAFETY: `T` is a plain integer type here; its bytes are initialised.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
        .to_vec()
}

/// A validity bitmap with bit `i` set for `valid[i]`.
fn bitmap(valid: &[bool]) -> Vec<u8> {
    let mut out = vec![0_u8; valid.len().div_ceil(8).max(1)];
    for (i, &v) in valid.iter().enumerate() {
        if v {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// An `int32` array of `values` at `offset` (the values include the rows
/// before the offset).
fn ints(values: &[i32], offset: i64) -> Box<Arr> {
    arr(
        values.len() as i64 - offset,
        offset,
        0,
        vec![vec![], bytes(values)],
        vec![],
    )
}

fn connect(fx: &Fixture) -> OwnedConnection {
    // SAFETY: the fixture's database outlives the connection.
    unsafe { OwnedConnection::open(fx.db()) }.expect("connect")
}

/// Imports a one-column batch of `rows` rows whose column has `schema` and
/// `column`. The array records are returned with the chunk: the chunk reads
/// their buffers, so they must outlive it.
fn import_one(
    con: &OwnedConnection,
    schema: Box<Sch>,
    column: Box<Arr>,
    rows: i64,
) -> Result<(OwnedDataChunk, Vec<Box<dyn Any>>), String> {
    let root_schema = sch("+s", vec![schema]);
    // SAFETY: a bitwise copy of the record: `release_schema` frees nothing, so
    // the copy and the original may both be released, and `root_schema` owns
    // everything the record points at and is kept alive below.
    let mut schema = unsafe { ArrowSchema::from_raw(ptr::read(&raw const root_schema.raw)) };
    // SAFETY: `con` is live.
    let converted =
        unsafe { schema_from_arrow(con.as_raw(), &mut schema) }.map_err(|e| e.to_string())?;
    let root = arr(rows, 0, 0, vec![vec![]], vec![column]);
    // SAFETY: a bitwise copy of the record, as above: `release_array` frees
    // nothing, and `root` owns everything the record points at and is returned
    // with the chunk.
    let array = unsafe { ArrowArray::from_raw(ptr::read(&raw const root.raw)) };
    // SAFETY: `con` is live, `converted` came from it, and the array conforms
    // to the schema it was built with.
    let chunk = unsafe { data_chunk_from_arrow(con.as_raw(), array, &converted) }.map_err(|e| {
        assert_eq!(e.error_type(), DuckDbErrorType::InvalidInput, "{e}");
        e.to_string()
    })?;
    Ok((chunk, vec![Box::new(root), Box::new(root_schema)]))
}

/// The imported column's rows as text, through a table of `sql_type`.
fn render(con: &OwnedConnection, chunk: &OwnedDataChunk, sql_type: &str) -> Vec<String> {
    con.execute("DROP TABLE IF EXISTS layout").expect("drop");
    con.execute(&format!("CREATE TABLE layout (v {sql_type})"))
        .expect("create");
    // SAFETY: `con` is live and the table exists.
    let appender = unsafe { quack_rs::appender::Appender::new(con.as_raw(), None, c"layout") }
        .expect("appender");
    appender.append_chunk(chunk).expect("append");
    appender.close().expect("close");
    drop(appender);
    let mut result = con
        .query("SELECT COALESCE(v::VARCHAR, 'NULL') FROM layout ORDER BY rowid")
        .expect("select");
    let mut out = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("fetch") {
        // SAFETY: one VARCHAR column.
        let reader = unsafe { chunk.reader(0) };
        for row in 0..chunk.size() {
            // SAFETY: `row < size`, and COALESCE made every row valid.
            out.push(unsafe { reader.read_str(row) }.to_owned());
        }
    }
    out
}

fn refused(result: Result<(OwnedDataChunk, Vec<Box<dyn Any>>), String>, what: &str) {
    match result {
        Ok(_) => panic!("imported: {what}"),
        Err(message) => assert!(message.contains(what), "{message}"),
    }
}

fn imports(
    con: &OwnedConnection,
    schema: Box<Sch>,
    column: Box<Arr>,
    rows: i64,
    sql_type: &str,
) -> Vec<String> {
    let (chunk, _keep) =
        import_one(con, schema, column, rows).expect("a layout DuckDB imports correctly");
    render(con, &chunk, sql_type)
}

/// A struct with an offset inside a list: `DuckDB` drops the struct's offset.
/// The same rows with the offset on the struct's field instead import right.
#[test]
fn a_struct_with_an_offset_inside_a_list_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || sch("+l", vec![sch("+s", vec![named("a", sch("i", vec![]))])]);
    let data = [0, 1, 2, 30, 40, 50, 60];
    let list = |struct_offset, int_offset| {
        arr(
            2,
            0,
            0,
            vec![vec![], bytes(&[0_i32, 2, 4])],
            vec![arr(
                4,
                struct_offset,
                0,
                vec![vec![]],
                vec![ints(&data, int_offset)],
            )],
        )
    };
    refused(
        import_one(&con, schema(), list(3, 0), 2),
        "list child: field 0",
    );
    assert_eq!(
        imports(&con, schema(), list(0, 3), 2, "STRUCT(a INTEGER)[]"),
        ["[{'a': 30}, {'a': 40}]", "[{'a': 50}, {'a': 60}]"]
    );
}

/// A struct inside a struct whose offset is not zero: the inner struct's
/// fields get only the inner offset.
#[test]
fn a_struct_inside_a_struct_with_an_offset_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || {
        sch(
            "+s",
            vec![named("a", sch("+s", vec![named("b", sch("i", vec![]))]))],
        )
    };
    let data = [10, 20, 30, 40, 50];
    let column = |outer, inner| {
        arr(
            3,
            outer,
            0,
            vec![vec![]],
            vec![arr(5 - inner, inner, 0, vec![vec![]], vec![ints(&data, 0)])],
        )
    };
    refused(
        import_one(&con, schema(), column(2, 0), 3),
        "field 0: field 0",
    );
    assert_eq!(
        imports(
            &con,
            schema(),
            column(0, 2),
            3,
            "STRUCT(a STRUCT(b INTEGER))"
        ),
        ["{'a': {'b': 30}}", "{'a': {'b': 40}}", "{'a': {'b': 50}}"]
    );
}

/// A sparse union with an offset: its members are converted from row 0. The
/// same rows with the offset on the members import right.
#[test]
fn a_union_with_an_offset_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || {
        sch(
            "+us:0,1",
            vec![named("a", sch("i", vec![])), named("b", sch("i", vec![]))],
        )
    };
    let members = |offset| {
        vec![
            ints(&[0, 100, 0, 102], offset),
            ints(&[0, 0, 11, 0], offset),
        ]
    };
    let union = |union_offset, member_offset, ids: &[i8]| {
        arr(3, union_offset, 0, vec![bytes(ids)], members(member_offset))
    };
    refused(
        import_one(&con, schema(), union(1, 0, &[0, 0, 1, 0]), 3),
        "member 0",
    );
    assert_eq!(
        imports(
            &con,
            schema(),
            union(0, 1, &[0, 1, 0]),
            3,
            "UNION(a INTEGER, b INTEGER)"
        ),
        ["100", "11", "102"]
    );
}

/// A sparse union whose `+us:` codes are not `0, 1`: `DuckDB` uses the codes
/// as member indices.
#[test]
fn a_union_with_recoded_type_ids_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let union = || {
        arr(
            2,
            0,
            0,
            vec![bytes(&[1_i8, 0])],
            vec![ints(&[1, 2], 0), ints(&[3, 4], 0)],
        )
    };
    refused(
        import_one(
            &con,
            sch(
                "+us:1,0",
                vec![named("a", sch("i", vec![])), named("b", sch("i", vec![]))],
            ),
            union(),
            2,
        ),
        "type codes",
    );
    assert_eq!(
        imports(
            &con,
            sch(
                "+us:0,1",
                vec![named("a", sch("i", vec![])), named("b", sch("i", vec![]))]
            ),
            union(),
            2,
            "UNION(a INTEGER, b INTEGER)"
        ),
        ["3", "2"]
    );
}

/// A dictionary with NULLs under a list that starts past element 0: its
/// validity is read without the list's offset.
#[test]
fn a_dictionary_with_nulls_under_a_list_starting_past_zero_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || sch("+l", vec![dict_sch("i", sch("i", vec![]))]);
    let list = |nulls: bool| {
        let valid = [true, true, true, !nulls];
        let dict = arr(
            4,
            0,
            i64::from(nulls),
            vec![bitmap(&valid), bytes(&[0_i32, 0, 1, 1])],
            vec![],
        );
        arr(
            1,
            0,
            0,
            vec![vec![], bytes(&[2_i32, 4])],
            vec![with_dict(dict, ints(&[5, 6], 0))],
        )
    };
    refused(
        import_one(&con, schema(), list(true), 1),
        "validity of dictionary indices",
    );
    assert_eq!(
        imports(&con, schema(), list(false), 1, "INTEGER[]"),
        ["[6, 6]"]
    );
}

/// A run-end-encoded array with NULL values under an offset struct: the
/// values' validity is read from the struct's offset, not run 0.
#[test]
fn a_run_end_encoded_array_below_an_offset_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || {
        sch(
            "+s",
            vec![sch("+r", vec![sch("i", vec![]), sch("i", vec![])])],
        )
    };
    let column = |offset| {
        let values = arr(
            2,
            0,
            1,
            vec![bitmap(&[false, true]), bytes(&[0_i32, 20])],
            vec![],
        );
        let ree = arr(4, 0, 0, vec![], vec![ints(&[2, 4], 0), values]);
        arr(2, offset, 0, vec![vec![]], vec![ree])
    };
    refused(
        import_one(&con, schema(), column(2), 2),
        "run-end-encoded array's values",
    );
    assert_eq!(
        imports(&con, schema(), column(0), 2, "STRUCT(f2 INTEGER)"),
        ["{'f2': NULL}", "{'f2': NULL}"]
    );
}

/// List views that overlap: `DuckDB` scans `sum(sizes)` elements from the
/// lowest offset, past the child. Views that leave no gap import right.
#[test]
fn overlapping_list_views_are_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || sch("+vl", vec![sch("i", vec![])]);
    let view = |offsets: &[i32], sizes: &[i32]| {
        arr(
            2,
            0,
            0,
            vec![vec![], bytes(offsets), bytes(sizes)],
            vec![ints(&[1, 2, 3], 0)],
        )
    };
    refused(
        import_one(&con, schema(), view(&[0, 0], &[3, 3]), 2),
        "past its child",
    );
    refused(
        import_one(&con, schema(), view(&[0, 2], &[1, 1]), 2),
        "overlap or leave gaps",
    );
    assert_eq!(
        imports(&con, schema(), view(&[2, 0], &[1, 2]), 2, "INTEGER[]"),
        ["[3]", "[1, 2]"]
    );
}

/// A dictionary of more than 2048 rows under a struct with NULL rows: the
/// struct's NULLs are copied into a 2048-row mask past a heap buffer.
#[test]
fn a_dictionary_under_null_struct_rows_past_2048_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let schema = || sch("+s", vec![dict_sch("i", sch("i", vec![]))]);
    let column = |rows: usize| {
        let valid: Vec<bool> = (0..rows).map(|i| i % 3 != 0).collect();
        let dict = arr(
            rows as i64,
            0,
            0,
            vec![vec![], bytes(&vec![1_i32; rows])],
            vec![],
        );
        arr(
            rows as i64,
            0,
            rows.div_ceil(3) as i64,
            vec![bitmap(&valid)],
            vec![with_dict(dict, ints(&[5, 6], 0))],
        )
    };
    refused(
        import_one(&con, schema(), column(4096), 4096),
        "2048-row mask",
    );
    let rendered = imports(&con, schema(), column(2048), 2048, "STRUCT(f0 INTEGER)");
    assert_eq!(rendered.len(), 2048);
    assert_eq!(rendered[0], "NULL");
    assert_eq!(rendered[1], "{'f0': 6}");
}

/// A dictionary whose values are dictionary-encoded: both share one cache.
#[test]
fn a_nested_dictionary_is_refused() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let inner = with_dict(
        arr(2, 0, 0, vec![vec![], bytes(&[1_i32, 0])], vec![]),
        ints(&[5, 6], 0),
    );
    let outer = with_dict(
        arr(2, 0, 0, vec![vec![], bytes(&[0_i32, 1])], vec![]),
        inner,
    );
    refused(
        import_one(
            &con,
            dict_sch("i", dict_sch("i", sch("i", vec![]))),
            outer,
            2,
        ),
        "shares one dictionary cache",
    );
}

/// A null-type field inside a struct imports as a constant vector, which the
/// readers would index as flat; the column is flattened, so every row reads
/// NULL.
#[test]
fn null_type_children_read_null_in_every_row() {
    let fx = Fixture::open();
    let con = connect(&fx);
    let null_field = arr(3, 0, 3, vec![], vec![]);
    let column = arr(3, 0, 0, vec![vec![]], vec![null_field, ints(&[1, 2, 3], 0)]);
    let (chunk, _keep) = import_one(
        &con,
        sch("+s", vec![sch("n", vec![]), sch("i", vec![])]),
        column,
        3,
    )
    .expect("import");
    // SAFETY: column 0 is a two-field struct; the chunk outlives the reader.
    let field = unsafe { chunk.struct_field_reader(0, 0) };
    for row in 0..3 {
        // SAFETY: `row < 3`.
        assert!(!unsafe { field.is_valid(row) }, "row {row}");
    }
}
