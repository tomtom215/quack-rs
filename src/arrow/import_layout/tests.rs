// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use std::ffi::c_void;

/// An Arrow array under construction that owns its buffers and children, so
/// the raw pointers it hands out stay valid while it lives. Boxed so that
/// moving a node does not move the records its parent points at.
#[allow(
    clippy::vec_box,
    reason = "each node is built boxed, and its parent's pointers are taken before the move into this vector"
)]
struct Node {
    raw: RawArrowArray,
    data: Vec<Vec<u8>>,
    buffers: Vec<*const c_void>,
    children: Vec<Box<Self>>,
    child_ptrs: Vec<*mut RawArrowArray>,
    dictionary: Option<Box<Self>>,
}

impl Node {
    fn new(length: i64, offset: i64, null_count: i64, data: Vec<Vec<u8>>) -> Box<Self> {
        let mut raw = RawArrowArray::empty();
        raw.length = length;
        raw.offset = offset;
        raw.null_count = null_count;
        let mut node = Box::new(Self {
            raw,
            data,
            buffers: Vec::new(),
            children: Vec::new(),
            child_ptrs: Vec::new(),
            dictionary: None,
        });
        node.buffers = node
            .data
            .iter()
            .map(|b| {
                if b.is_empty() {
                    std::ptr::null()
                } else {
                    b.as_ptr().cast()
                }
            })
            .collect();
        node.raw.n_buffers = i64::try_from(node.buffers.len()).expect("buffer count");
        node.raw.buffers = node.buffers.as_mut_ptr();
        node
    }

    #[allow(clippy::vec_box, reason = "as on `Node`")]
    fn with_children(mut self: Box<Self>, children: Vec<Box<Self>>) -> Box<Self> {
        self.children = children;
        self.child_ptrs = self.children.iter_mut().map(|c| &raw mut c.raw).collect();
        self.raw.n_children = i64::try_from(self.child_ptrs.len()).expect("child count");
        self.raw.children = self.child_ptrs.as_mut_ptr();
        self
    }

    fn with_dictionary(mut self: Box<Self>, dictionary: Box<Self>) -> Box<Self> {
        self.dictionary = Some(dictionary);
        self.raw.dictionary = &raw mut self.dictionary.as_mut().expect("just set").raw;
        self
    }
}

fn i32s(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// An all-valid validity bitmap (the null count decides whether it is read).
fn bits(rows: usize) -> Vec<u8> {
    vec![0xFF; rows.div_ceil(8).max(1)]
}

fn leaf() -> Shape {
    Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: None,
    }
}

fn of(kind: Kind, children: Vec<Shape>) -> Shape {
    Shape {
        kind,
        children,
        dictionary: None,
    }
}

fn ints(length: i64, offset: i64) -> Box<Node> {
    let n = usize::try_from(length + offset).unwrap_or(0);
    Node::new(length, offset, 0, vec![vec![], i32s(&vec![7; n])])
}

/// One column, checked as the child of a record batch of `rows` rows.
fn check_column(rows: i64, column: Box<Node>, shape: Shape) -> Result<(), String> {
    let batch = Node::new(rows, 0, 0, vec![vec![]]).with_children(vec![column]);
    // SAFETY: `batch` owns every buffer and child it points at, and each
    // node matches `shape`.
    unsafe { check(&batch.raw, &[shape], 2048) }
}

#[test]
fn formats_map_to_the_kinds_duckdb_reads() {
    assert_eq!(kind_of("i", 0, None), Kind::Leaf);
    assert_eq!(kind_of("n", 0, None), Kind::Null);
    assert_eq!(kind_of("+s", 2, None), Kind::Struct);
    assert_eq!(kind_of("+l", 1, None), Kind::List { wide: false });
    assert_eq!(kind_of("+m", 1, None), Kind::List { wide: false });
    assert_eq!(kind_of("+L", 1, None), Kind::List { wide: true });
    assert_eq!(kind_of("+vl", 1, None), Kind::ListView { wide: false });
    assert_eq!(kind_of("+vL", 1, None), Kind::ListView { wide: true });
    assert_eq!(kind_of("+w:4", 1, None), Kind::FixedList(4));
    assert_eq!(kind_of("+w:x", 1, None), Kind::Leaf);
    // `DuckDB` parses the size with `std::stoi`.
    for size in ["2x", " 2", "\t+2", "+2", "02", "2 "] {
        assert_eq!(
            kind_of(&format!("+w:{size}"), 1, None),
            Kind::FixedList(2),
            "{size:?}"
        );
    }
    for size in ["", "-2", "x2", "+-2", "99999999999"] {
        assert_eq!(
            kind_of(&format!("+w:{size}"), 1, None),
            Kind::Leaf,
            "{size:?}"
        );
    }
    assert_eq!(kind_of("+r", 2, None), Kind::RunEnd);
    assert_eq!(kind_of("+us:0,1", 2, None), Kind::SparseUnion);
    assert_eq!(kind_of("+us:1,0", 2, None), Kind::RecodedUnion);
    assert_eq!(
        kind_of("+us:0,1", 3, None),
        Kind::RecodedUnion,
        "fewer codes than members"
    );
    assert_eq!(kind_of("+us:0,2", 2, None), Kind::RecodedUnion);
}

#[test]
fn a_struct_inside_a_struct_with_an_offset_is_refused() {
    let shape = of(Kind::Struct, vec![of(Kind::Struct, vec![leaf()])]);
    let outer = |offset| {
        Node::new(3, offset, 0, vec![vec![]]).with_children(vec![
            Node::new(5, 0, 0, vec![vec![]]).with_children(vec![ints(5, 0)])
        ])
    };
    assert_eq!(check_column(3, outer(0), shape.clone()), Ok(()));
    let err = check_column(3, outer(2), shape).expect_err("the outer offset is dropped");
    assert!(
        err.contains("column 0: field 0: field 0: rows would be read"),
        "{err}"
    );
}

#[test]
fn a_struct_with_an_offset_inside_a_list_is_refused() {
    let shape = of(
        Kind::List { wide: false },
        vec![of(Kind::Struct, vec![leaf()])],
    );
    let list = |offset| {
        Node::new(2, 0, 0, vec![vec![], i32s(&[0, 2, 4])]).with_children(vec![Node::new(
            4,
            offset,
            0,
            vec![vec![]],
        )
        .with_children(vec![ints(7, 0)])])
    };
    assert_eq!(check_column(2, list(0), shape.clone()), Ok(()));
    let err = check_column(2, list(3), shape).expect_err("the struct's offset is dropped");
    assert!(err.contains("list child: field 0"), "{err}");
}

#[test]
fn a_union_with_an_offset_or_recoded_type_ids_is_refused() {
    let shape = |kind| of(kind, vec![leaf(), leaf()]);
    let union = |offset| {
        Node::new(3, offset, 0, vec![vec![0, 1, 0, 1]]).with_children(vec![ints(4, 0), ints(4, 0)])
    };
    assert_eq!(check_column(3, union(0), shape(Kind::SparseUnion)), Ok(()));
    let err = check_column(3, union(1), shape(Kind::SparseUnion)).expect_err("offset");
    assert!(err.contains("member 0"), "{err}");
    let err = check_column(3, union(0), shape(Kind::RecodedUnion)).expect_err("codes");
    assert!(err.contains("type codes"), "{err}");
}

#[test]
fn overlapping_or_gapped_list_views_are_refused() {
    let shape = of(Kind::ListView { wide: false }, vec![leaf()]);
    let view = |offsets: &[i32], sizes: &[i32], child: i64| {
        Node::new(2, 0, 0, vec![vec![], i32s(offsets), i32s(sizes)])
            .with_children(vec![ints(child, 0)])
    };
    assert_eq!(
        check_column(2, view(&[0, 2], &[2, 1], 3), shape.clone()),
        Ok(())
    );
    let err = check_column(2, view(&[0, 0], &[3, 3], 3), shape.clone()).expect_err("overlap");
    assert!(err.contains("past its child"), "{err}");
    let err = check_column(2, view(&[0, 2], &[1, 1], 3), shape).expect_err("gap");
    assert!(err.contains("overlap or leave gaps"), "{err}");
}

#[test]
fn dictionaries_duckdb_imports_wrongly_are_refused() {
    let dict_shape = Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: Some(Box::new(leaf())),
    };
    let dict = |length, offset, nulls| {
        Node::new(
            length,
            offset,
            nulls,
            vec![bits(4200), i32s(&vec![0; 4200])],
        )
        .with_dictionary(ints(1, 0))
    };
    // Under a list starting past element 0, with NULLs: validity offset lost.
    let in_list = of(Kind::List { wide: false }, vec![dict_shape.clone()]);
    let list = |nulls| {
        Node::new(1, 0, 0, vec![vec![], i32s(&[2, 4])]).with_children(vec![dict(4, 0, nulls)])
    };
    assert_eq!(check_column(1, list(0), in_list.clone()), Ok(()));
    let err = check_column(1, list(1), in_list).expect_err("validity offset");
    assert!(err.contains("validity of dictionary indices"), "{err}");
    // More rows than the mask holds, with NULLs of its own or its struct's.
    assert_eq!(
        check_column(4096, dict(4096, 0, 0), dict_shape.clone()),
        Ok(())
    );
    let err = check_column(4096, dict(4096, 0, 1), dict_shape.clone()).expect_err("own NULLs");
    assert!(err.contains("2048-row mask"), "{err}");
    let in_struct = of(Kind::Struct, vec![dict_shape.clone()]);
    let strukt = Node::new(4096, 0, 1, vec![bits(4096)]).with_children(vec![dict(4096, 0, 0)]);
    let err = check_column(4096, strukt, in_struct).expect_err("struct NULLs");
    assert!(err.contains("enclosing struct"), "{err}");
    // A dictionary of dictionaries.
    let nested = Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: Some(Box::new(dict_shape)),
    };
    let node = Node::new(1, 0, 0, vec![vec![], i32s(&[0])]).with_dictionary(dict(1, 0, 0));
    let err = check_column(1, node, nested).expect_err("nested dictionary");
    assert!(err.contains("shares one dictionary cache"), "{err}");
}

#[test]
fn run_end_encoding_duckdb_reads_wrongly_is_refused() {
    let ree_shape = of(Kind::RunEnd, vec![leaf(), leaf()]);
    let ree = |value_nulls| {
        Node::new(3, 0, 0, vec![]).with_children(vec![
            Node::new(1, 0, 0, vec![vec![], i32s(&[3])]),
            Node::new(1, 0, value_nulls, vec![bits(1), i32s(&[20])]),
        ])
    };
    assert_eq!(check_column(3, ree(1), ree_shape.clone()), Ok(()));
    // Under a list starting past element 0, the values' validity is misread.
    let in_list = of(Kind::List { wide: false }, vec![ree_shape.clone()]);
    let list =
        |nulls| Node::new(1, 0, 0, vec![vec![], i32s(&[1, 3])]).with_children(vec![ree(nulls)]);
    assert_eq!(check_column(1, list(0), in_list.clone()), Ok(()));
    let err = check_column(1, list(1), in_list).expect_err("values' validity");
    assert!(err.contains("run-end-encoded array's values"), "{err}");
    // Under a fixed-size list, it is read as a plain array.
    let in_array = of(Kind::FixedList(3), vec![ree_shape]);
    let array = Node::new(1, 0, 0, vec![vec![]]).with_children(vec![ree(0)]);
    let err = check_column(1, array, in_array).expect_err("plain read");
    assert!(err.contains("buffers it does not have"), "{err}");
}

#[test]
fn a_child_count_or_dictionary_mismatch_is_refused() {
    let err = check_column(1, ints(1, 0), of(Kind::Struct, vec![leaf()])).expect_err("children");
    assert!(
        err.contains("0 child array(s) where its schema has 1"),
        "{err}"
    );
    let dict_shape = Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: Some(Box::new(leaf())),
    };
    let err = check_column(1, ints(1, 0), dict_shape).expect_err("dictionary");
    assert!(err.contains("disagree on dictionary"), "{err}");
}

#[test]
fn a_list_or_run_end_array_without_its_children_is_refused() {
    let list = Node::new(1, 0, 0, vec![vec![], i32s(&[0, 1])]);
    let err = check_column(1, list, of(Kind::List { wide: false }, vec![])).expect_err("list");
    assert!(err.contains("child 0 is missing"), "{err}");
    let ree = Node::new(1, 0, 0, vec![]).with_children(vec![ints(1, 0)]);
    let err = check_column(1, ree, of(Kind::RunEnd, vec![leaf()])).expect_err("run end");
    assert!(err.contains("child 1 is missing"), "{err}");
}

#[test]
fn a_shape_count_that_differs_from_the_column_count_is_refused() {
    let batch = Node::new(1, 0, 0, vec![vec![]]).with_children(vec![ints(1, 0)]);
    // SAFETY: `batch` owns every buffer and child it points at.
    let err = unsafe { check(&batch.raw, &[], 2048) }.expect_err("no shapes");
    assert!(err.contains("1 column(s) but 0 schema shape(s)"), "{err}");
}

/// A fixed-size list's NULLs (its own, or an enclosing struct's) are
/// broadcast into its child's validity (`ArrowToDuckDBArray`); a `STRUCT`
/// child passes that validity on to its fields as `parent_mask`, so a
/// dictionary field of more than 2048 rows overflows the mask even though
/// neither the struct nor the dictionary has a NULL of its own. A dictionary
/// directly under the fixed-size list gets no `parent_mask` and is fine.
#[test]
fn fixed_list_nulls_reach_a_dictionary_through_a_struct() {
    let dict_shape = Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: Some(Box::new(leaf())),
    };
    let dict =
        || Node::new(2200, 0, 0, vec![vec![], i32s(&vec![0; 2200])]).with_dictionary(ints(1, 0));
    let fixed = |nulls, child: Box<Node>| {
        let valid = if nulls == 0 { vec![] } else { bits(1100) };
        Node::new(1100, 0, nulls, vec![valid]).with_children(vec![child])
    };
    let through_struct = of(
        Kind::FixedList(2),
        vec![of(Kind::Struct, vec![dict_shape.clone()])],
    );
    let strukt = || Node::new(2200, 0, 0, vec![vec![]]).with_children(vec![dict()]);
    assert_eq!(
        check_column(1100, fixed(0, strukt()), through_struct.clone()),
        Ok(())
    );
    let err = check_column(1100, fixed(1, strukt()), through_struct).expect_err("broadcast");
    assert!(err.contains("2048-row mask"), "{err}");
    let direct = of(Kind::FixedList(2), vec![dict_shape]);
    assert_eq!(check_column(1100, fixed(1, dict()), direct), Ok(()));
}

/// `null_count = -1` ("not computed") makes `GetValidityMask` copy the
/// bitmap (it tests `!= 0`) while the dictionary path's `CanContainNull`
/// tests `> 0` and ignores it: the NULL rows' indices are used as values.
/// A union has no validity bitmap, so any nonzero `null_count` makes `DuckDB`
/// read its type ids as one.
#[test]
fn an_unknown_null_count_where_duckdb_disagrees_with_itself_is_refused() {
    let dict_shape = Shape {
        kind: Kind::Leaf,
        children: vec![],
        dictionary: Some(Box::new(leaf())),
    };
    let dict = |nulls| {
        Node::new(3, 0, nulls, vec![vec![0b101], i32s(&[0, 7, 1])]).with_dictionary(ints(2, 0))
    };
    assert_eq!(check_column(3, dict(1), dict_shape.clone()), Ok(()));
    let err = check_column(3, dict(-1), dict_shape).expect_err("unknown dictionary nulls");
    assert!(err.contains("null_count -1"), "{err}");

    let shape = of(Kind::SparseUnion, vec![leaf(), leaf()]);
    let union = |nulls| {
        Node::new(2, 0, nulls, vec![vec![0, 0]]).with_children(vec![ints(2, 0), ints(2, 0)])
    };
    assert_eq!(check_column(2, union(0), shape.clone()), Ok(()));
    for nulls in [-1, 1] {
        let err = check_column(2, union(nulls), shape.clone()).expect_err("union null count");
        assert!(err.contains("type ids"), "{nulls}: {err}");
    }
}

/// Arrow metadata in the C Data Interface encoding: an `i32` pair count,
/// then for each pair an `i32` length and the key bytes, an `i32` length and
/// the value bytes.
fn metadata(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut out = i32::try_from(pairs.len())
        .expect("count")
        .to_ne_bytes()
        .to_vec();
    for (key, value) in pairs {
        for part in [key, value] {
            out.extend_from_slice(&i32::try_from(part.len()).expect("len").to_ne_bytes());
            out.extend_from_slice(part.as_bytes());
        }
    }
    out
}

#[test]
fn the_extension_name_is_read_from_arrow_metadata() {
    let geo = metadata(&[
        ("ARROW:extension:metadata", "{}"),
        ("ARROW:extension:name", "geoarrow.wkb"),
    ]);
    // SAFETY: each buffer is well-formed metadata that outlives the call.
    unsafe {
        assert_eq!(
            extension_name(geo.as_ptr().cast()).as_deref(),
            Some("geoarrow.wkb")
        );
        let other = metadata(&[("ARROW:extension:name", "arrow.uuid")]);
        assert_eq!(
            extension_name(other.as_ptr().cast()).as_deref(),
            Some("arrow.uuid")
        );
        let none = metadata(&[("key", "value")]);
        assert_eq!(extension_name(none.as_ptr().cast()), None);
        assert_eq!(extension_name(metadata(&[]).as_ptr().cast()), None);
        assert_eq!(extension_name(std::ptr::null()), None);
    }
    assert_eq!(
        kind_of("z", 0, Some("geoarrow.wkb")),
        Kind::CopiedIntoOneVector
    );
    assert_eq!(kind_of("z", 0, Some("arrow.json")), Kind::Leaf);
    assert_eq!(kind_of("z", 0, None), Kind::Leaf);
}

/// `geoarrow.wkb` storage is converted into a vector of `DuckDB`'s standard
/// size (2048) before the cast, so more rows than that write past it.
#[test]
fn a_geoarrow_column_of_more_than_2048_rows_is_refused() {
    let geo = of(Kind::CopiedIntoOneVector, vec![]);
    let blobs = |rows: usize| {
        let offsets: Vec<i32> = (0..=rows)
            .map(|i| i32::try_from(i).expect("fits"))
            .collect();
        Node::new(
            i64::try_from(rows).expect("fits"),
            0,
            0,
            vec![vec![], i32s(&offsets), vec![0; rows]],
        )
    };
    assert_eq!(check_column(2048, blobs(2048), geo.clone()), Ok(()));
    let err = check_column(4096, blobs(4096), geo).expect_err("past the vector");
    assert!(err.contains("geoarrow.wkb"), "{err}");
}

/// A node `DuckDB` converts as zero rows still has its descendants converted:
/// a run-end-encoded array below it is expanded at its full length, and its
/// values' validity read at the inherited offset. The walk used to stop at
/// the first zero-row node.
#[test]
fn the_walk_continues_below_a_node_of_zero_rows() {
    let ree_shape = of(Kind::RunEnd, vec![leaf(), leaf()]);
    let shape = of(
        Kind::List { wide: false },
        vec![of(Kind::Struct, vec![ree_shape])],
    );
    let column = |value_nulls| {
        let ree = Node::new(100, 0, 0, vec![]).with_children(vec![
            Node::new(1, 0, 0, vec![vec![], i32s(&[100])]),
            Node::new(1, 0, value_nulls, vec![bits(1), i32s(&[20])]),
        ]);
        let strukt = Node::new(100, 0, 0, vec![vec![]]).with_children(vec![ree]);
        // One empty list whose offsets start at 100: zero child rows, read
        // from element 100 on.
        Node::new(1, 0, 0, vec![vec![], i32s(&[100, 100])]).with_children(vec![strukt])
    };
    assert_eq!(check_column(1, column(0), shape.clone()), Ok(()));
    let err = check_column(1, column(1), shape).expect_err("below a zero-row struct");
    assert!(err.contains("run-end-encoded array's values"), "{err}");
}

/// Offsets are summed down the tree; a sum past `i64::MAX` is refused rather
/// than overflowing (a panic in a debug build, a wrong comparison in release).
#[test]
fn an_offset_sum_that_overflows_is_refused() {
    let shape = of(Kind::Struct, vec![leaf()]);
    // No data buffers: the check reads none for these nodes.
    let column = Node::new(1, 1, 0, vec![vec![]]).with_children(vec![Node::new(
        1,
        i64::MAX,
        0,
        vec![vec![], vec![]],
    )]);
    let err = check_column(1, column, shape).expect_err("the sum overflows");
    assert!(
        err.contains("field 0: ") && err.contains("overflow"),
        "{err}"
    );
}

/// Only the record batch's own length and offset were checked for being
/// negative; every node's are now.
#[test]
fn a_negative_offset_or_length_below_the_top_is_refused() {
    let shape = of(Kind::Struct, vec![leaf()]);
    for (length, offset) in [(1, -1), (-1, 0)] {
        let column = Node::new(1, 0, 0, vec![vec![]]).with_children(vec![Node::new(
            length,
            offset,
            0,
            vec![vec![], vec![]],
        )]);
        let err = check_column(1, column, shape.clone()).expect_err("negative");
        assert!(
            err.contains(&format!("negative length ({length}) or offset ({offset})")),
            "{err}"
        );
    }
}

/// A list's child is read from the list's first element for its values and
/// its validity alike, so a child with NULLs under a list starting at element
/// 2 imports: `DuckDB` reads row 2 of the bitmap, where Arrow puts it.
#[test]
fn validity_under_a_list_starting_past_zero_is_accepted() {
    let shape = of(Kind::List { wide: false }, vec![leaf()]);
    let child = Node::new(4, 0, 1, vec![bits(4), i32s(&[1, 2, 3, 4])]);
    let list = Node::new(1, 0, 0, vec![vec![], i32s(&[2, 4])]).with_children(vec![child]);
    assert_eq!(check_column(1, list, shape), Ok(()));
}
