// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

use super::*;
use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

// These tests exercise the ownership bookkeeping of the wrappers, which is
// pure Rust: they run without a DuckDB dispatch table. The end-to-end
// conversions live in `tests/ffi_roundtrip.rs`, where a real database is
// available.

// Each test owns its counter and hands it to the release callback through
// the record's own `private_data`, which is exactly what that field is for.
// A shared `static` would race: `cargo test` runs these in parallel, so one
// test's reset could land between another's drop and its assertion.

/// Increments the counter in `private_data`, if the test installed one.
///
/// # Safety
///
/// `private_data` must be null or a pointer to a live `AtomicUsize`.
unsafe fn bump(private_data: *mut std::os::raw::c_void) {
    // SAFETY: forwarded from this function's own contract.
    if let Some(counter) = unsafe { private_data.cast::<AtomicUsize>().as_ref() } {
        counter.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe extern "C" fn count_schema_release(schema: *mut RawArrowSchema) {
    // SAFETY: DuckDB and every other Arrow producer pass a valid pointer,
    // and every schema built here stores its counter in `private_data`.
    unsafe {
        bump((*schema).private_data);
        // The Arrow specification requires the callback to null its own
        // `release`, which is how "already released" is observable.
        (*schema).release = None;
    }
}

unsafe extern "C" fn count_array_release(array: *mut RawArrowArray) {
    // SAFETY: as above.
    unsafe {
        bump((*array).private_data);
        (*array).release = None;
    }
}

fn live_schema(counter: &'static AtomicUsize) -> ArrowSchema {
    let mut raw = RawArrowSchema::empty();
    raw.private_data = std::ptr::from_ref(counter).cast_mut().cast();
    raw.release = Some(count_schema_release);
    // SAFETY: `count_schema_release` frees nothing and nulls itself.
    unsafe { ArrowSchema::from_raw(raw) }
}

fn live_array(counter: &'static AtomicUsize) -> ArrowArray {
    let mut raw = RawArrowArray::empty();
    raw.private_data = std::ptr::from_ref(counter).cast_mut().cast();
    raw.release = Some(count_array_release);
    // SAFETY: `count_array_release` frees nothing and nulls itself.
    unsafe { ArrowArray::from_raw(raw) }
}

/// The accessors have to be exercised against a record that actually
/// carries values: reading only `empty()` proves the released guards work
/// and nothing else, so "always return None" would go unnoticed.
///
/// Every owner below is a **local**, and the pointers into them are derived
/// after the last move. This used to be a `populated_schema()` helper that
/// built the owners and then moved them into a returned struct; under
/// Stacked Borrows that move retags the `Box`, which pops the tag already
/// derived from it, so `child()` was reading through a dead tag. Miri only
/// caught it once the job started running with `--features duckdb-1-5-4`,
/// because this whole module is gated on it.
#[test]
fn a_populated_schema_reports_its_format_name_flags_and_children() {
    let strings: Vec<CString> = ["+s", "duckdb_query_result", "i", "id", "u", "label"]
        .iter()
        .map(|s| CString::new(*s).expect("no interior NUL"))
        .collect();

    let mut children = [RawArrowSchema::empty(), RawArrowSchema::empty()];
    children[0].format = strings[2].as_ptr();
    children[0].name = strings[3].as_ptr();
    children[0].release = Some(count_schema_release);
    children[1].format = strings[4].as_ptr();
    children[1].name = strings[5].as_ptr();
    children[1].release = Some(count_schema_release);

    // Disjoint sub-places of one array, so the second retag leaves the
    // first pointer's tag intact.
    let mut child_ptrs = [
        std::ptr::from_mut(&mut children[0]),
        std::ptr::from_mut(&mut children[1]),
    ];

    let mut raw = RawArrowSchema::empty();
    raw.format = strings[0].as_ptr();
    raw.name = strings[1].as_ptr();
    raw.flags = 2; // ARROW_FLAG_NULLABLE
    raw.n_children = 2;
    raw.children = child_ptrs.as_mut_ptr();
    raw.release = Some(count_schema_release);

    // SAFETY: `count_schema_release` frees nothing and nulls itself, and
    // every pointer above targets a local declared before `root`, so all of
    // them outlive it (locals drop in reverse declaration order).
    let root = unsafe { ArrowSchema::from_raw(raw) };

    assert!(!root.is_released());
    assert_eq!(root.format(), Some("+s"));
    assert_eq!(root.name(), Some("duckdb_query_result"));
    assert_eq!(root.flags(), 2);
    assert_eq!(root.child_count(), 2);

    let id = root.child(0).expect("child 0");
    assert_eq!(id.format(), Some("i"));
    assert_eq!(id.name(), Some("id"));

    let label = root.child(1).expect("child 1");
    assert_eq!(label.format(), Some("u"));
    assert_eq!(label.name(), Some("label"));

    assert!(root.child(2).is_none(), "past the end");
}

/// A populated array with two children. Same shape, and same Stacked
/// Borrows reasoning, as the schema test above: the owners are locals and
/// nothing moves after a pointer is taken.
#[test]
fn a_populated_array_reports_its_length_nulls_offset_and_children() {
    let mut children = [RawArrowArray::empty(), RawArrowArray::empty()];
    for (i, child) in children.iter_mut().enumerate() {
        child.length = 7;
        child.null_count = i64::try_from(i).expect("small");
        child.release = Some(count_array_release);
    }

    let mut child_ptrs = [
        std::ptr::from_mut(&mut children[0]),
        std::ptr::from_mut(&mut children[1]),
    ];

    let mut raw = RawArrowArray::empty();
    raw.length = 7;
    raw.null_count = 3;
    raw.offset = 2;
    raw.n_children = 2;
    raw.children = child_ptrs.as_mut_ptr();
    raw.release = Some(count_array_release);

    // SAFETY: as in the schema test -- `count_array_release` frees nothing
    // and nulls itself, and every pointer targets a local declared before
    // `root`.
    let root = unsafe { ArrowArray::from_raw(raw) };

    assert!(!root.is_released());
    assert_eq!(root.len(), 7);
    assert!(!root.is_empty());
    assert_eq!(root.null_count(), 3);
    assert_eq!(root.offset(), 2);
    assert_eq!(root.child_count(), 2);

    assert_eq!(root.child(0).expect("child 0").len(), 7);
    assert_eq!(root.child(0).expect("child 0").null_count(), 0);
    assert_eq!(root.child(1).expect("child 1").null_count(), 1);
    assert!(root.child(2).is_none(), "past the end");
}

#[test]
fn a_length_zero_array_is_empty_but_not_released() {
    let mut raw = RawArrowArray::empty();
    raw.release = Some(count_array_release);
    // SAFETY: `count_array_release` frees nothing and nulls itself.
    let array = unsafe { ArrowArray::from_raw(raw) };
    assert!(!array.is_released(), "a live producer set `release`");
    assert_eq!(array.len(), 0);
    assert!(array.is_empty(), "zero rows is empty");
}

#[test]
fn an_empty_schema_is_already_released_and_frees_nothing() {
    let schema = ArrowSchema::empty();
    assert!(schema.is_released());
    assert_eq!(schema.format(), None);
    assert_eq!(schema.name(), None);
    assert_eq!(schema.child_count(), 0);
    assert!(schema.child(0).is_none());
    drop(schema);
}

#[test]
fn an_empty_array_is_already_released_and_frees_nothing() {
    let array = ArrowArray::empty();
    assert!(array.is_released());
    assert_eq!(array.len(), 0);
    assert!(array.is_empty());
    assert_eq!(array.child_count(), 0);
    assert!(array.child(0).is_none());
    drop(array);
}

#[test]
fn dropping_a_live_schema_releases_it_once() {
    static RELEASES: AtomicUsize = AtomicUsize::new(0);
    drop(live_schema(&RELEASES));
    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_live_array_releases_it_once() {
    static RELEASES: AtomicUsize = AtomicUsize::new(0);
    drop(live_array(&RELEASES));
    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn releasing_explicitly_is_idempotent_and_drop_adds_nothing() {
    static SCHEMA: AtomicUsize = AtomicUsize::new(0);
    static ARRAY: AtomicUsize = AtomicUsize::new(0);

    let mut schema = live_schema(&SCHEMA);
    schema.release();
    schema.release();
    assert!(schema.is_released());
    drop(schema);
    assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

    let mut array = live_array(&ARRAY);
    array.release();
    array.release();
    assert!(array.is_released());
    drop(array);
    assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
}

#[test]
fn into_raw_hands_the_release_callback_to_the_caller() {
    static SCHEMA: AtomicUsize = AtomicUsize::new(0);
    static ARRAY: AtomicUsize = AtomicUsize::new(0);

    let mut raw = live_schema(&SCHEMA).into_raw();
    assert_eq!(
        SCHEMA.load(Ordering::SeqCst),
        0,
        "into_raw must not release"
    );
    let release = raw.release.expect("release survived into_raw");
    // SAFETY: the caller now owns the record; releasing it once is correct.
    unsafe { release(&raw mut raw) };
    assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

    let mut raw = live_array(&ARRAY).into_raw();
    assert_eq!(ARRAY.load(Ordering::SeqCst), 0);
    let release = raw.release.expect("release survived into_raw");
    // SAFETY: as above.
    unsafe { release(&raw mut raw) };
    assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
}

#[test]
fn take_from_neutralises_the_source_so_only_one_side_releases() {
    static SCHEMA: AtomicUsize = AtomicUsize::new(0);
    static ARRAY: AtomicUsize = AtomicUsize::new(0);

    let mut source = RawArrowSchema::empty();
    source.private_data = std::ptr::from_ref(&SCHEMA).cast_mut().cast();
    source.release = Some(count_schema_release);
    // SAFETY: `source` is a live, uniquely-owned record.
    let taken = unsafe { ArrowSchema::take_from(&raw mut source) };
    assert!(!taken.is_released(), "the callback moved to the wrapper");
    assert!(
        source.release.is_none(),
        "the source must be left released so its own Drop is a no-op"
    );
    drop(taken);
    assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

    let mut source = RawArrowArray::empty();
    source.private_data = std::ptr::from_ref(&ARRAY).cast_mut().cast();
    source.release = Some(count_array_release);
    // SAFETY: as above.
    let taken = unsafe { ArrowArray::take_from(&raw mut source) };
    assert!(!taken.is_released());
    assert!(source.release.is_none());
    drop(taken);
    assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
}

#[test]
fn a_released_record_reports_neutral_values_instead_of_dangling_ones() {
    // After release the Arrow specification leaves every other field
    // undefined, so the accessors must not read them. Simulate a producer
    // that released without clearing the stale pointers.
    let mut raw = RawArrowSchema::empty();
    raw.format = c"+s".as_ptr();
    raw.n_children = 7;
    raw.children = ptr::dangling_mut();
    // SAFETY: `release` is None, so nothing is called and nothing is freed.
    let schema = unsafe { ArrowSchema::from_raw(raw) };
    assert!(schema.is_released());
    assert_eq!(schema.format(), None, "must not read a stale format");
    assert_eq!(schema.child_count(), 0, "must not trust a stale n_children");
    assert!(schema.child(0).is_none(), "must not walk stale children");

    let mut raw = RawArrowArray::empty();
    raw.length = 42;
    raw.n_children = 7;
    raw.children = ptr::dangling_mut();
    // SAFETY: as above.
    let array = unsafe { ArrowArray::from_raw(raw) };
    assert_eq!(array.len(), 0);
    assert_eq!(array.child_count(), 0);
    assert!(array.child(0).is_none());
}

#[test]
fn debug_says_released_without_touching_the_other_fields() {
    static SCHEMA: AtomicUsize = AtomicUsize::new(0);
    static ARRAY: AtomicUsize = AtomicUsize::new(0);

    assert!(format!("{:?}", ArrowSchema::empty()).contains("released"));
    assert!(format!("{:?}", ArrowArray::empty()).contains("released"));

    let schema = live_schema(&SCHEMA);
    let rendered = format!("{schema:?}");
    assert!(rendered.contains("n_children"), "{rendered}");
    let array = live_array(&ARRAY);
    let rendered = format!("{array:?}");
    assert!(rendered.contains("length"), "{rendered}");
}

#[test]
fn a_converted_schema_remembers_its_column_count_and_shapes() {
    use super::import_layout::Kind;

    let strings: Vec<CString> = ["+s", "i", "+l", "+us:1,0", "u"]
        .iter()
        .map(|s| CString::new(*s).expect("no interior NUL"))
        .collect();
    let mut leaves = [
        RawArrowSchema::empty(),
        RawArrowSchema::empty(),
        RawArrowSchema::empty(),
    ];
    leaves[0].format = strings[1].as_ptr();
    leaves[1].format = strings[1].as_ptr();
    leaves[2].format = strings[4].as_ptr();
    // Every record in a live schema carries a release callback; one without
    // reads as released.
    for leaf in &mut leaves {
        leaf.release = Some(count_schema_release);
    }
    let [item, member_a, member_b] = &mut leaves;
    let mut list_children = [std::ptr::from_mut(item)];
    let mut union_children = [std::ptr::from_mut(member_a), std::ptr::from_mut(member_b)];
    let mut columns = [
        RawArrowSchema::empty(),
        RawArrowSchema::empty(),
        RawArrowSchema::empty(),
    ];
    columns[0].format = strings[1].as_ptr();
    columns[1].format = strings[2].as_ptr();
    columns[1].n_children = 1;
    columns[1].children = list_children.as_mut_ptr();
    columns[2].format = strings[3].as_ptr();
    columns[2].n_children = 2;
    columns[2].children = union_children.as_mut_ptr();
    for column in &mut columns {
        column.release = Some(count_schema_release);
    }
    let [c0, c1, c2] = &mut columns;
    let mut column_ptrs = [
        std::ptr::from_mut(c0),
        std::ptr::from_mut(c1),
        std::ptr::from_mut(c2),
    ];
    let mut raw = RawArrowSchema::empty();
    raw.format = strings[0].as_ptr();
    raw.n_children = 3;
    raw.children = column_ptrs.as_mut_ptr();
    raw.release = Some(count_schema_release);
    // SAFETY: `count_schema_release` frees nothing and nulls itself, and every
    // pointer above targets a local declared before `root`.
    let root = unsafe { ArrowSchema::from_raw(raw) };

    // SAFETY: a null handle is what `duckdb_destroy_arrow_converted_schema`
    // ignores, so this never calls into DuckDB.
    let converted = unsafe { ArrowConvertedSchema::from_raw(ptr::null_mut(), &root) };
    assert_eq!(converted.column_count(), 3);
    let kinds: Vec<Kind> = converted.shapes().iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [Kind::Leaf, Kind::List { wide: false }, Kind::RecodedUnion]
    );
    assert_eq!(converted.shapes()[1].children[0].kind, Kind::Leaf);
    assert_eq!(converted.shapes()[2].children.len(), 2);
    assert!(format!("{converted:?}").contains("column_count"));
}

// A producer that breaks the rule above: its callback does not null `release`.
unsafe extern "C" fn count_schema_release_leaving_it_set(schema: *mut RawArrowSchema) {
    // SAFETY: as in `count_schema_release`.
    unsafe { bump((*schema).private_data) };
}

unsafe extern "C" fn count_array_release_leaving_it_set(array: *mut RawArrowArray) {
    // SAFETY: as in `count_array_release`.
    unsafe { bump((*array).private_data) };
}

/// `release` runs the producer's callback once even when the callback does
/// not null itself, so a later `release` or the drop cannot free twice.
#[test]
fn release_runs_once_even_if_the_callback_leaves_itself_set() {
    static SCHEMA: AtomicUsize = AtomicUsize::new(0);
    static ARRAY: AtomicUsize = AtomicUsize::new(0);
    let mut raw = RawArrowSchema::empty();
    raw.private_data = std::ptr::from_ref(&SCHEMA).cast_mut().cast();
    raw.release = Some(count_schema_release_leaving_it_set);
    // SAFETY: the callback frees nothing.
    let mut schema = unsafe { ArrowSchema::from_raw(raw) };
    schema.release();
    assert!(schema.is_released());
    schema.release();
    drop(schema);
    assert_eq!(SCHEMA.load(Ordering::SeqCst), 1);

    let mut raw = RawArrowArray::empty();
    raw.private_data = std::ptr::from_ref(&ARRAY).cast_mut().cast();
    raw.release = Some(count_array_release_leaving_it_set);
    // SAFETY: the callback frees nothing.
    let mut array = unsafe { ArrowArray::from_raw(raw) };
    array.release();
    assert!(array.is_released());
    drop(array);
    assert_eq!(ARRAY.load(Ordering::SeqCst), 1);
}

/// `dictionary` borrows a dictionary-encoded schema's value schema, and is
/// `None` for a schema without one.
#[test]
fn dictionary_borrows_the_value_schema() {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let values_format = CString::new("u").expect("no NUL");
    let mut values = RawArrowSchema::empty();
    values.format = values_format.as_ptr();
    // Live, as a producer's is; no counter, so its release counts nothing.
    values.release = Some(count_schema_release);
    let indices_format = CString::new("i").expect("no NUL");
    let mut raw = RawArrowSchema::empty();
    raw.format = indices_format.as_ptr();
    raw.dictionary = &raw mut values;
    raw.private_data = std::ptr::from_ref(&COUNTER).cast_mut().cast();
    raw.release = Some(count_schema_release);
    // SAFETY: `count_schema_release` frees nothing and nulls itself; `values`
    // and the format strings outlive the schema.
    let schema = unsafe { ArrowSchema::from_raw(raw) };
    assert_eq!(schema.dictionary().and_then(ArrowSchema::format), Some("u"));
    assert!(live_schema(&COUNTER).dictionary().is_none());
}
