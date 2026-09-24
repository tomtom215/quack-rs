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
    assert_eq!(kind_of("i", 0), Kind::Leaf);
    assert_eq!(kind_of("n", 0), Kind::Null);
    assert_eq!(kind_of("+s", 2), Kind::Struct);
    assert_eq!(kind_of("+l", 1), Kind::List { wide: false });
    assert_eq!(kind_of("+m", 1), Kind::List { wide: false });
    assert_eq!(kind_of("+L", 1), Kind::List { wide: true });
    assert_eq!(kind_of("+vl", 1), Kind::ListView { wide: false });
    assert_eq!(kind_of("+vL", 1), Kind::ListView { wide: true });
    assert_eq!(kind_of("+w:4", 1), Kind::FixedList(4));
    assert_eq!(kind_of("+w:x", 1), Kind::Leaf);
    assert_eq!(kind_of("+r", 2), Kind::RunEnd);
    assert_eq!(kind_of("+us:0,1", 2), Kind::SparseUnion);
    assert_eq!(kind_of("+us:1,0", 2), Kind::RecodedUnion);
    assert_eq!(
        kind_of("+us:0,1", 3),
        Kind::RecodedUnion,
        "fewer codes than members"
    );
    assert_eq!(kind_of("+us:0,2", 2), Kind::RecodedUnion);
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
