# quack-rs — Pitfall Reference

All known DuckDB Rust FFI pitfalls, with symptoms, root causes, and fixes.

These were discovered building [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral),
a production DuckDB community extension. Every future developer who builds a Rust DuckDB extension
will hit every one of these problems. This SDK makes most of them impossible.

---

## L1: COMBINE must propagate ALL config fields

**Status**: Can be tested with `AggregateTestHarness`.

**Symptom**: Aggregate function returns wrong results. No error, no crash.

**Root cause**: DuckDB's segment tree creates fresh target states with `state_init` (for
`FfiState<T>`, `T::default()`), then calls `combine` to merge source states into them. If your
`combine` only propagates data fields (e.g., `count`, `sum`) but forgets configuration fields
(e.g., `window_size`, `mode`), the configuration is still at its default at finalize time,
silently corrupting results.

**Fix**:
```rust
unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        if let (Some(src), Some(tgt)) = (
            FfiState::<MyState>::with_state(*source.add(i)),
            FfiState::<MyState>::with_state_mut(*target.add(i)),
        ) {
            // MUST copy ALL fields, including configuration fields
            tgt.window_size = src.window_size;  // config field
            tgt.mode = src.mode;                // config field
            tgt.count += src.count;             // data field
        }
    }
}
```

**SDK status**: `AggregateTestHarness::combine` lets you test this without DuckDB.
The combine-propagates-config bug passed 435 unit tests before being caught by E2E tests.

---

## L2: State destroy double-free

**Status**: Made impossible by `FfiState<T>`.

**Symptom**: Crash or memory corruption on extension unload.

**Root cause**: If `state_destroy` frees the inner Box but doesn't null out the pointer,
a second call to `state_destroy` (e.g., in error paths) will free already-freed memory.

**Fix**: `FfiState<T>::destroy_callback` clears the slot's tag before dropping the `T`, and
drops only a slot whose tag matches, so a second call is a no-op. Use it instead of
writing your own destructor.

---

## L3: No panic across FFI boundaries

**Status**: Prevented for the entry point and for every callback defined with a
`*_callback!` macro or a closure-based builder; these guards require
`panic = "unwind"`. A raw `unsafe extern "C" fn` that you pass to a builder
yourself is not guarded.

**Symptom**: The whole DuckDB process aborts when an extension callback panics.

**Root cause**: a panic cannot unwind out of an `extern "C"` function. Since Rust 1.81 the
runtime aborts the process when one tries (before 1.81 it was undefined behaviour), so an
uncaught `panic!()` or `.unwrap()` in a callback takes down the user's session.

**Fix**: Use `Result` and `?` inside `init_extension`. Never use `unwrap()` in FFI callbacks.
`FfiState::with_state_mut` returns `Option`, not `Result`, so callers use `if let`.

The entry-point macros, the `*_callback!` macros in `quack_rs::callback`, the
closure-based builders (`ScalarFunctionBuilder::map1` and its siblings,
`TypedTableFunctionBuilder`) and `FfiState`'s callbacks catch a panic and report
it as a SQL error. A raw `unsafe extern "C" fn` that you pass to a builder
yourself is installed as written, with no guard: define it with the matching
`*_callback!` macro, or wrap its body in `quack_rs::callback::catch_ffi_panic`
and report the `Err` it returns yourself.

All of these guards require `panic = "unwind"` in the release profile, which is
what the scaffold generates and what `validate_release_profile` insists on:
under `panic = "abort"` nothing can be caught.

---

## L4: `ensure_validity_writable` is required before NULL output

**Status**: Made impossible by `VectorWriter::set_null`.

**Symptom**: NULLs written to an output vector are silently lost; the row reads back as a
valid value. (It only crashes if you dereference the mask pointer yourself.)

**Root cause**: a vector that has never held a NULL usually has no validity mask, so
`duckdb_vector_get_validity` returns NULL — as `duckdb.h` documents — and
`duckdb_validity_set_row_invalid` / `_valid` return early on a NULL mask
(`src/main/capi/data_chunk-c.cpp`). Nothing is written and nothing reports an error.

**Fix**: Always call `duckdb_vector_ensure_validity_writable` before `duckdb_vector_get_validity`
when writing NULLs. `VectorWriter::set_null` does this automatically.

---

## L5: Boolean reading must use `u8 != 0`, not `*const bool`

**Status**: Made impossible by `VectorReader::read_bool`.

**Symptom**: Undefined behavior; Rust requires `bool` to be exactly 0 or 1.

**Root cause**: DuckDB's C API does not guarantee that boolean values in vectors are exactly
0 or 1. Casting a byte with value 2, 255, etc. to Rust `bool` is undefined behavior.

**Fix**: Read boolean data as `*const u8` and compare with `!= 0`.
`VectorReader::read_bool` always does this.

---

## L6: Function set name must be set on EACH member

**Status**: Made impossible by `AggregateFunctionSetBuilder`.

**Symptom**: Function is silently not registered. No error returned.

**Root cause**: When using `duckdb_register_aggregate_function_set`, the function name must
be set on EACH individual `duckdb_aggregate_function` added to the set using
`duckdb_aggregate_function_set_name`, not just on the set itself.

This is completely undocumented. Discovered by reading DuckDB's C++ test code at
`test/api/capi/test_capi_aggregate_functions.cpp`.

In duckdb-behavioral, 6 of 7 functions failed to register silently due to this bug.

**Fix**: `AggregateFunctionSetBuilder` calls `duckdb_aggregate_function_set_name` on every
individual function before adding it to the set.

---

## L7: LogicalType memory leak

**Status**: Made impossible by `LogicalType` RAII wrapper.

**Symptom**: Memory leak proportional to number of registered functions.

**Root cause**: `duckdb_create_logical_type` allocates memory that must be freed with
`duckdb_destroy_logical_type`. Forgetting to call the destructor leaks memory.

**Fix**: `LogicalType` implements `Drop` and calls `duckdb_destroy_logical_type` automatically.

---

## L8: `DEFAULT_NULL_HANDLING` does not propagate NULLs for scalar functions

**Status**: Documented, and made impossible by `ScalarFunctionBuilder::map1` /
`map2` / `map1_str` / `map2_str`. `DataChunk::propagate_nulls` fixes it in one
line for hand-written callbacks.

**Symptom**: A scalar function returns a value where SQL requires NULL — but only
for arguments that come from a column. `SELECT f(NULL)` looks correct, so the bug
survives review and ships.

**Root cause**: The name suggests DuckDB returns NULL on your behalf. For a
**scalar** function registered through the C API it does not, at run time:

- `src/main/capi/scalar_function-c.cpp` — `CAPIScalarFunction` flattens the input
  and calls the extension's callback for the whole chunk, NULL rows included,
  then checks only the *error* flag. It never inspects the result's validity.
- `src/execution/expression_executor/execute_function.cpp` — the only NULL check
  is `VerifyNullHandling`, whose entire body is inside `#ifdef DEBUG` through
  DuckDB 1.5.6. It *asserts* that the function already produced NULL wherever an
  input was NULL.

Every DuckDB a user installs is a release build, so through 1.5.6 that assertion is
compiled out. DuckDB 2.0.0 compiles it into release builds but runs it only after
`SET debug_verification_mode = 'VERIFY_FUNCTIONS'` (the default is `NONE`; measured:
with the setting on, `INTERNAL Error: VerifyNullHandling failed for scalar function`).
With the default, a callback that ignores validity is simply believed.

A literal `NULL` argument is **constant-folded** during binding — DuckDB
substitutes NULL without calling the function at all — which is why the obvious
spot check passes.

**Reproduction** (DuckDB 1.5.4):

```sql
CREATE TABLE t(i BIGINT);
INSERT INTO t VALUES (1), (NULL), (3);
SELECT i, always_999(i) FROM t;   -- always_999 writes 999 unconditionally
-- 1    -> 999
-- NULL -> 999   <- not NULL
-- 3    -> 999
SELECT always_999(NULL::BIGINT);  -- NULL, because of constant folding
```

**Fix**:

- `ScalarFunctionBuilder::map1` / `map2` / `map1_str` / `map2_str` skip NULL rows
  and write NULL for them, so the closure never sees one.
- `DataChunk::propagate_nulls(&mut writer)` at the end of a hand-written callback
  marks the output NULL wherever any input column is NULL.
- `map1_opt` / `map2_opt`, and `NullHandling::SpecialNullHandling`, are for
  functions that genuinely mean to see NULLs.

**Aggregates are no different**: `update` receives NULL rows under either
setting too — see L12. (Until September 2026 this paragraph said the opposite;
it was never checked, and it was wrong.)

Pinned by `default_null_handling_does_not_propagate_nulls_for_scalar_functions`
in `tests/ffi_roundtrip.rs`, so a future DuckDB that changes this shows up as a
test failure rather than a surprise.

---

## L9: `duckdb_data_chunk_from_arrow` takes the array even when it fails

**Status**: Made impossible by `arrow::data_chunk_from_arrow`, which takes the
`ArrowArray` **by value**.

**Symptom**: One of two opposite bugs, depending on which way you guessed. Treat
the array as still yours after a failed conversion and you double-release it —
DuckDB already stored a copy of the record in the chunk's owned data, so its
`release` runs twice. Treat it as gone in *every* case and a zero-column
conversion leaks the whole Arrow buffer tree.

**Root cause**: `duckdb.h` says "Data ownership is passed on to DuckDB's
DataChunk", which reads like a success-path statement. `src/main/capi/arrow-c.cpp`
is more specific — the transfer happens inside the per-column loop, before any of
the work that can throw:

```cpp
for (idx_t i = 0; i < dchunk->ColumnCount(); i++) {
    ...
    array_state->owned_data->arrow_array = *arrow_array;
    // We set it to nullptr to effectively transfer the ownership
    arrow_array->release = nullptr;
    try { ... } catch (...) { return duckdb_create_error_data(...); }
}
```

So the array is claimed on the error path too — but **only if the loop runs at
all**. A converted schema with zero columns leaves `release` intact and the array
still belongs to the caller. The early return for null arguments is the same.

**Fix**: own the record in a wrapper whose `Drop` releases *only if `release`
survived*, and consume it by value:

```rust
let chunk = unsafe { data_chunk_from_arrow(con, array, &converted) }?;
// `array` is gone from the caller's view either way. Inside, the by-value
// binding drops on the way out: a no-op when DuckDB nulled `release`, a
// correct release when it did not.
```

The same wrapper handles the mirror-image case. `ArrowConverter::ToArrowSchema`
and `ToArrowArray` install `release` / assign `*out_array` as their **last**
statement, so a *failed* export leaves `release == NULL` and nothing to free —
discarding the record is correct there, not a leak.

**Related**: `duckdb_data_chunk_from_arrow` also indexes
`arrow_array->children[i]` once per schema column with no bounds check, and
dereferences an already-released array. Both are segfaults rather than errors;
`data_chunk_from_arrow` checks them first, which is why `ArrowConvertedSchema`
records the column count of the schema it was built from.

---

## L10: Scalar bind data is dropped when DuckDB copies the expression

**Status**: Fixable only from the extension, and now possible:
[`ScalarBindInfo::set_bind_data_copy`].

**Symptom**: A scalar function that allocates per-query state in its bind
callback reads **null** from `duckdb_scalar_function_get_bind_data` during
execution, for some queries and not others. Nothing crashes and nothing is
reported: the callback simply runs without the state it bound, so the answer is
quietly wrong.

**Root cause**: `duckdb_scalar_function_set_bind_data` registers the pointer and
its destructor, but *not* how to duplicate it. DuckDB copies a bound
expression whenever it duplicates a plan, and `CScalarFunctionBindData::Copy()`
in `src/main/capi/scalar_function-c.cpp` (read at `v1.5.5`; byte-identical in
`v1.5.4`) only fills the copy in when a copy callback exists:

```cpp
unique_ptr<FunctionData> Copy() const override {
    auto copy = make_uniq<CScalarFunctionBindData>(info);
    if (copy_callback) {
        copy->bind_data = copy_callback(bind_data);
        copy->delete_callback = delete_callback;
        copy->copy_callback = copy_callback;
    }
    return std::move(copy);   // bind_data stays null without a callback
}
```

With no callback the copy carries `bind_data = nullptr`, and the original is
untouched — which is why the failure is intermittent rather than total, and why
it survives a test suite that only ever executes the first-bound expression.

**Fix**: use `ScalarBindData::set`, which registers a generated, panic-safe
copy callback (it requires `T: Clone + Send + Sync`). With the raw API, register
a copy callback alongside the bind data, in the same bind callback and after
`set_bind_data`:

```rust
unsafe extern "C" fn copy(data: *mut c_void) -> *mut c_void {
    if data.is_null() {
        return std::ptr::null_mut();
    }
    let src = unsafe { &*data.cast::<MyBindData>() };
    Box::into_raw(Box::new(src.clone())).cast()
}

unsafe {
    bind_info.set_bind_data(Box::into_raw(boxed).cast(), Some(destroy));
    bind_info.set_bind_data_copy(Some(copy));
}
```

The duplicate is freed with the **same** destructor as the original, so `copy`
must return an independently owned allocation — returning the pointer it was
given is a double free.

**Related**: the copy callback runs across the FFI boundary like any other, so
it must not unwind. Wrap anything that can panic in
[`callback::catch_ffi_panic`] and return null.

[`ScalarBindInfo::set_bind_data_copy`]: https://docs.rs/quack-rs/latest/quack_rs/scalar/info/struct.ScalarBindInfo.html#method.set_bind_data_copy
[`callback::catch_ffi_panic`]: https://docs.rs/quack-rs/latest/quack_rs/callback/fn.catch_ffi_panic.html

---

## L11: C API aggregates crash under `agg(x) OVER ()` and `agg(x ORDER BY y)`

**Status**: A DuckDB defect in 1.4.x and 1.5.x, reported upstream as
[duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109) and fixed in
DuckDB 2.0.0 (see **Fix**). On older releases it cannot be prevented or detected from an extension;
documented on `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder` and
`FfiState`.

**Symptom**: An aggregate that works under `SELECT agg(x) FROM t` and `GROUP BY`
segfaults (or corrupts memory, or returns a wrong answer) when used as a window
over a whole-partition frame — `agg(x) OVER ()`, or an explicit `ROWS` or `RANGE
BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING` frame, with or without a
`PARTITION BY` — or as an ordered aggregate, `agg(x ORDER BY y)`. On 1.4.4 `OVER
(PARTITION BY p)` without an `ORDER BY` crashes too; on 1.5.6 it did not in the shapes
measured (5,000 and 300,000 rows). Whether that is a different code path or luck was not
established, so do not rely on it.

**Root cause**: `CAPIAggregateUpdate` (`src/main/capi/aggregate_function-c.cpp`)
flattens the input vectors but not the state vector, then hands the callback
`FlatVector::GetDataUnsafe(state)`. The C API registers no `simple_update`, so
two executors fall back to calling `update` with a **constant** state vector and
`count > 1`: `WindowConstantAggregatorLocalState` (`statep(Value::POINTER(0))`)
and `SortedAggregateFunction` (`agg_state_vec.SetVectorType(CONSTANT_VECTOR)`).
The callback reads `states[i]` for every row, as the C API contract says it
may; only `states[0]` exists. Reproduced with a plain C aggregate (no quack-rs)
against DuckDB 1.4.4, 1.5.0 and 1.5.5; AddressSanitizer places the fault in the
callback, called from `CAPIAggregateUpdate`.

**Fix**: none on the extension side — the callback receives a raw
`duckdb_aggregate_state *` and cannot tell a constant vector from a flat one,
and reading `states[1]` to find out is itself the out-of-bounds read. Until
you require DuckDB 2.0.0, document for your users that the aggregate must not be used
in those query shapes. Frames that are not whole-partition (the default frame of
`OVER (ORDER BY i)`, `ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`, `ROWS BETWEEN 5
PRECEDING AND CURRENT ROW`, `EXCLUDE`) and `DISTINCT` and `FILTER` were measured and work
on 1.4.4, 1.5.6 and 2.0.0 ([Known
Limitations](https://quack-rs.com/reference/known-limitations.html#aggregates-crash-under-over--and-order-by-duckdb-defect-pitfall-l11)
has the table).

DuckDB 2.0.0 fixes it in `CAPIAggregateUpdate`
(`src/main/capi/v1/aggregate_function-c.cpp`, lines 104-108): a state vector
that is not flat is flattened into one state pointer per row before `update`
is called. Checked live: a plain C aggregate (no quack-rs) answers both shapes
correctly on 2.0.0 and runs clean under Valgrind; the same program segfaults on
1.5.6, whose `CAPIAggregateUpdate` (`src/main/capi/aggregate_function-c.cpp`,
lines 92-111) still passes the state vector's data pointer as it is.

---

## L12: Aggregate `update` receives NULL rows under `DEFAULT_NULL_HANDLING`

**Status**: Documented on `NullHandling`, `UpdateFn` and both aggregate builders'
`null_handling`. Pinned by
`aggregate_update_receives_null_rows_under_either_null_handling` in
`tests/ffi_roundtrip/lifecycle.rs`.

**Symptom**: An aggregate that reads every row — `state.sum += reader.read_i64(row)`
— returns a wrong answer, with no error, as soon as its input column contains a
NULL. The value read for a NULL row is whatever the data buffer happens to hold.

**Root cause**: quack-rs used to document (in `NullHandling`, the builders and
the book) that DuckDB's aggregate executor filters NULL rows out before
`update` unless `SpecialNullHandling` is set. It does not. `CAPIAggregateUpdate`
(`src/main/capi/aggregate_function-c.cpp`) flattens each input vector and
passes the whole chunk, validity and all. For an aggregate the setting is
read in one place, `BoundAggregateExpression::PropagatesNullValues`, which only
the correlated-subquery decorrelator (`flatten_dependent_join.cpp`) consults to
pick an `INNER` or `LEFT` join; the aggregate `VerifyNullHandling` check is
compiled only under `#ifdef DEBUG`. Checked against DuckDB 1.5.5: `update` saw
every NULL row, ungrouped and under `GROUP BY`, under both settings, and no
correlated subquery tried answered differently under the two.

**Fix**: in `update`, skip rows where `VectorReader::is_valid(row)` is false,
whatever the null handling. Use `SpecialNullHandling` to declare that the
aggregate returns non-NULL for NULL input; it does not change which rows arrive.

A related trap: in a correlated subquery,
`(SELECT my_count(x) FROM t2 WHERE t2.k = t1.k)` is NULL, not `my_count` of an
empty input, for an outer row with no match. Before DuckDB 2.0.0 that holds
under either setting, and DuckDB rewrites the NULL to 0 only for its own
`count`. From 2.0.0 the decorrelator (`FlattenDependentJoins::PushDownAggregate`,
`flatten_dependent_join.cpp` lines 912-950) finalizes an empty state of every
`SpecialNullHandling` aggregate and uses it for such rows, so only a
`DefaultNullHandling` aggregate still answers NULL there. Wrap the subquery in
`coalesce(..., 0)` if it matters.
Pinned by `a_count_like_aggregate_in_a_correlated_subquery_is_null_for_an_unmatched_row`.

---

## L13: A C API aggregate without a destructor is wrong in a running window

**Status**: Fixed in quack-rs: every aggregate builder registers a destructor,
a no-op when none is given. Pinned by
`an_aggregate_without_a_destructor_is_right_in_a_running_window` in
`tests/ffi_roundtrip/agg_window.rs`. Reported in
`docs/upstream-duckdb-reports.md`, item 7.

**Symptom**: `agg(x) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)`
with no `PARTITION BY` or `ORDER BY` returns the wrong running value, with no
error: a sum over `1..5` reads `1 2 3 4 5`. The same aggregate is right with
`ORDER BY`, ungrouped, grouped, and in DuckDB's own `sum`.

**Root cause**: DuckDB streams such a window only for an aggregate with no
destructor (`PhysicalStreamingWindow::IsStreamingFunction`). Streaming calls
`update` once per row, count 1, on a one-row dictionary slice it moves along,
and `CAPIAggregateUpdate` flattens that input vector in place, so the slice
becomes row 0's value for the rest of the chunk. Checked against 1.4.4, 1.5.0
and 1.5.5.

**Fix**: register a destructor, even an empty one. quack-rs does this for you;
with the raw C API, call `duckdb_aggregate_function_set_destructor`.

---

## L14: The C API behaves differently across the releases one build loads into

**Status**: Four cases fixed in quack-rs (`ScalarBindInfo::argument`,
`LogicalType::try_decimal`, `LogicalType::try_new`, the scalar collision
check); CI job `test-older-engines` runs the suite against DuckDB 1.4.4 and
1.4.5 (default features), 1.5.0 (`duckdb-1-5`), 1.5.3 (`duckdb-1-5-3`) and
1.5.4 (`duckdb-1-5-4`).

**Symptom**: code tested against the release `Cargo.lock` pins (1.5.5) aborts
or misbehaves in an older release the same binary loads into. A default-feature
extension loads into every release from 1.4.4; a `duckdb-1-5` one built against
the 1.5.4 bindings has the 546-slot layout of 1.5.2 to 1.5.6, so the ABI guard
rightly lets it load into all five.

**Root cause**: a C function's *contract* can change in a release while its
slot stays put. `duckdb_scalar_function_bind_get_argument` gained its `try`
only in 1.5.5 (before it, a subquery argument throws through the extension's
callback: an abort in Rust); `duckdb_create_decimal_type` its width/scale
check only in 1.5.4 (before it, `DECIMAL(0, 0)` comes back as a type);
`duckdb_register_scalar_function` its `ALTER_ON_CONFLICT` only in 1.5.0 (before
it, no existing name can take another overload); and 1.4.x's C API reports
`TIME_NS`, which its SQL has, as `INVALID`. All four were found only by running
the whole suite against the oldest release of each range.

**Fix**: when a wrapper relies on C API behaviour, find the release that
introduced it (the source at each tag answers that), and either check the
engine version at run time (`abi::engine_version`) or make the check in Rust.
Test against the oldest release a build can load into, not only the pinned one.

---

## L15: `combine` must leave its source states unchanged

**Status**: Documented on `CombineFn` and in the aggregate chapter; pinned by
`combine_must_leave_its_source_unchanged` (`tests/ffi_roundtrip/agg_window.rs`).
Not preventable by the SDK: `combine` receives raw state pointers.

**Symptom**: an aggregate is right in `GROUP BY` and wrong in a sliding window
(`ROWS BETWEEN n PRECEDING AND CURRENT ROW`): in the regression test a sum
whose `combine` moved its value out of the source gave 4985 of 5000 rows wrong.

**Root cause**: a window's segment tree keeps one state per tree node and
combines each node's state into every frame that covers it, from several
threads at once (`WindowSegmentTreePart::WindowSegmentValue`,
`window_segment_tree.cpp`). A `combine` that consumes its source (`mem::take`,
zeroing a counter) is right for the first frame and wrong for the rest, and a
write to a source another thread reads is a data race.

**Fix**: read the source; copy or clone what the target needs. `AggregateState`
requires `Sync` for the same reason.

---

## L16: A valid Arrow array is not always one DuckDB imports correctly

**Status**: Fixed in quack-rs: `data_chunk_from_arrow` walks the array with its
schema and refuses the layouts DuckDB 1.4.4 to 1.5.5 mishandles
(`src/arrow/import_layout.rs`, `tests/ffi_roundtrip/arrow_layout.rs`;
`docs/upstream-duckdb-reports.md`, items 9, 24 to 29 and 31 to 33).

**Symptom**: an array that arrow-rs or another producer built, valid by the
Arrow specification, imports with values from the wrong rows, reads past a
buffer, or corrupts the heap. Arrays DuckDB exported itself never show it,
which is why round-trip tests pass.

**Root cause**: DuckDB's importer tracks where a node's rows start with two
parameters, `parent_offset` and `nested_offset`, and some paths pass the wrong
one: a struct gives its children only its own offset, union members start at
row 0, a dictionary's validity ignores the list's offset. List views, recoded
union type ids and nested dictionaries are mishandled too.

**Fix**: never assume a producer's layout matches the one DuckDB writes.
Test an importer with hand-built arrays that put offsets at every level, and
refuse what the engine cannot import rather than return wrong values.

---

## L17: A `COPY … FROM` reader must not declare result columns

**Status**: Refused for typed table functions (their bind fails with a
message); documented for raw ones on `CopyFunctionBuilder::copy_from` and
`BindInfo::add_result_column`. Pinned by
`tests/ffi_roundtrip/copy_from_columns.rs`;
`docs/upstream-duckdb-reports.md`, item 37.

**Symptom**: on a DuckDB built with assertions, `COPY t FROM …` fails with
`chunk.ColumnCount() == types.size()` and the database is invalidated. A
release build silently drops the extra column, so the bug hides in testing.

**Root cause**: `CCopyFromBind` hands the reader's bind the `INSERT`'s own list
of expected types as its result types, and `duckdb_bind_add_result_column`
appends to that list, so every chunk the `INSERT` receives is wider than the
table. `duckdb.h` says the reader "should not" declare columns; nothing
enforces it.

**Fix**: in a `COPY … FROM` reader's bind, read the target's columns with
`BindInfo::result_column_count` and its siblings, and declare none.

---

## L18: A `LIST` reserve moves every buffer below its child

**Status**: Documented in the `# Safety` sections of `VectorWriter::from_vector`,
`StructWriter::new`, `StructVector::field_writer` and
`ValidityBitmap::ensure_writable`; measured by
`tests/ffi_roundtrip/nested_reserve.rs`.

**Symptom**: a writer on a STRUCT field of a list's elements writes into freed
memory after the list is grown, although it was never a direct child of the
list.

**Root cause**: `duckdb_list_vector_reserve` resizes the child with
`Vector::Resize`, which reallocates the data and validity buffers of the child
and of every STRUCT field and ARRAY element vector below it, down to the next
`LIST` (whose child has its own buffer). Writers cache both pointers.

**Fix**: fetch every writer and bitmap below a list's child again after each
`reserve` on that list (a `ListBuilder` row that grows it counts).

---

## L19: Addresses of constants are not identities

**Status**: Fixed in `FfiState` (0.18.0): its per-type tag salt is a hash of
`TypeId::of::<T>()`, not the address of `type_name::<T>()`. AUDIT.md 10.2
(High) and 10.8.

**Symptom**: a per-type tag compared across callbacks mismatches only in
release builds of the user's crate — `FfiState::with_state` returns `None`,
`destroy_callback` skips states — while debug and fat-LTO builds hide it.

**Root cause**: `core::any::type_name::<T>().as_ptr()` (and a function
pointer, and the address of any `&'static` constant) can differ between
codegen units: `rustc` emits a private copy of a constant in each codegen unit
that uses it, so with `codegen-units > 1` and no fat LTO (Cargo's default
release profile) two uses of the same constant can have different addresses.
Rust makes no address-identity guarantee for functions or constants.

**Fix**: derive identity from a value, e.g. hash `TypeId::of::<T>()`.

**Evidence**: a standalone crate taking `type_name::<T>().as_ptr()` for one
`T` in 8 modules: release profile (`codegen-units = 16`, `lto = false`) → 6
distinct addresses; dev profile → 1; `codegen-units = 1`, `lto = true` → 1.
In quack-rs itself, `cargo test --release --lib aggregate::` with
`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_LTO=false`
failed 5 tests on the address salt and passes on the `TypeId` one; the
end-to-end suite under the same profile failed 10 of 279 (every aggregate:
NULL or garbage results) and passes all 279. Every other
CI build is debug or uses the repository's `codegen-units = 1`, fat-LTO
release profile, which is why the bug got past them; CI's `test` job now
runs this build.

---

## L20: `duckdb_register_log_storage` frees the extra data twice when the name is taken

**Status**: Documented. quack-rs does not wrap the log-storage functions because of it.
This is a DuckDB defect, present since 1.5.0 and still on `main` (2026-10-30), not a
quack-rs one.

**Symptom**: registering a log storage under a name that is already taken returns
`DuckDBError`, and the extra-data destructor you gave DuckDB has already run. The
`duckdb_destroy_log_storage` you must still call runs it again on the same pointer.
Observed on DuckDB 1.5.6 and 2.0.0 with a probe extension that registers the same name
twice: the destructor ran twice on one address. With a destructor that frees its box this
is a heap double free (`free(): double free detected in tcache 2`, seen in the eighth
review). The same happens for a name that a built-in storage already holds.

**Root cause** (`src/main/capi/v1/logging-c.cpp`, identical at 1.5.6, 2.0.0 and `main`):

```cpp
auto shared_storage_ptr = make_shared_ptr<CallbackLogStorage>(
    wrapper->name, wrapper->write_log_entry, wrapper->extra_data, wrapper->delete_callback);
const auto success = RegisterLogStorage(wrapper->name, storage_ptr);
if (!success) {
    return DuckDBError;           // shared_storage_ptr dies here and calls delete_callback(extra_data)
}
wrapper->extra_data = nullptr;    // only cleared on success
wrapper->delete_callback = nullptr;
```

The early failures (a null argument, an empty name, no write callback) return before the
`CallbackLogStorage` exists and do not consume the data, so a caller cannot treat every
`DuckDBError` alike.

**Workaround**: never register a name that can already exist. After a failure that got
past argument validation, clear the extra data with
`duckdb_log_storage_set_extra_data(ls, NULL, NULL)` before destroying the wrapper (proposed,
not tested).

**Rule**: a destructor you hand to DuckDB may already have run when the call that took
it reports failure; read the failure path before you free again.

---

## L21: A chunk row you claim is published, written or not

**Status**: Documented. Since 0.20.0 `ChunkWriter::release_row` gives a claimed row back;
before 0.20.0 every claimed row was published. The rule is about the chunk the
scan callback returns.

**Symptom**: a table function whose scan claims a row from the writer and then finds
nothing to write (its source ended) still returns that row to DuckDB, holding whatever the
reused output buffer held last. Measured in the eighth review on DuckDB 1.5.6: a scan that
wrote 2 rows returned 3, and the third was the `7` an earlier chunk had left in that
position. For a fixed-width column that is a wrong answer, not undefined behaviour
(valgrind-clean). For a `LIST` or `MAP` column written by hand, a row never written keeps the
`{offset, length}` entry of an earlier chunk, which can point past the child data this chunk
filled (CONJECTURED, not reproduced).

**Root cause**: the writer counts the rows you *claimed* and sets the chunk's size from that
count when it is dropped. DuckDB reuses output vectors across chunks, so a claimed row you
did not write is published with stale contents.

**Fix**: claim a row only when you are going to write it: check that the source has another
value first, then claim the row and write it in the same step. A row you have already
claimed can be given back with `ChunkWriter::release_row(row)` before the writer is dropped.

**Rule**: an output chunk holds the rows you wrote, not the rows you asked for.

---

## P1: Library name must match extension name

**Status**: Must be configured manually in `Cargo.toml`.

**Symptom**: Community build fails with `FileNotFoundError` when building the extension.

**Root cause**: The community extension Makefile expects `lib{extension_name}.so`. If your
Cargo crate is named `duckdb-my-ext` (producing `libduckdb_my_ext.so`) but `description.yml`
says `name: my-ext`, the build fails.

**Fix**: Add `name = "extension_name"` to `[lib]` in `Cargo.toml`:
```toml
[lib]
name = "my_extension"   # Must match description.yml's `name: my_extension`
crate-type = ["cdylib", "rlib"]
```

---

## P2: Metadata version is C API version, not DuckDB version

**Status**: Must be handled manually when using `append_extension_metadata.py`.

**Symptom**: The metadata script succeeds, and `LOAD` then refuses the file:
"The file was built for DuckDB C API version 'v1.5.5', but we can only load
extensions built for DuckDB C API 'v1.2.0' and lower" (verified on DuckDB 1.4.4
and 1.5.5 with a file stamped `-dv v1.5.5`).

**Root cause**: The `-dv` flag to `append_extension_metadata.py` must be the C API version
(e.g., `"v1.2.0"`), NOT the DuckDB release version (e.g., `"v1.4.4"` / `"v1.5.0"` / `"v1.5.1"`).
DuckDB v1.4.x and v1.5.0 – v1.5.5 all declare C API version v1.2.0 (confirmed by E2E tests); v1.5.6
and v2.0.0 declare v1.5.6 and still load v1.2.0 extensions. A `C_STRUCT` binary stamped v1.5.6 is
refused by v1.5.5 ("The file was built for DuckDB C API version 'v1.5.6'...") and loads in v1.5.6
and v2.0.0; that is the stamp for an extension using quack-rs's `duckdb-1-5` features
(`quack_rs::DUCKDB_API_VERSION_1_5_6`, see P10).

**Fix**: Use `quack_rs::DUCKDB_API_VERSION` constant for the init call, and use the same
version string with `append_extension_metadata.py -dv v1.2.0`.

**Caveat — this only holds for the `C_STRUCT` ABI type.** `-dv` means the *C API* version for
`C_STRUCT`, but the *exact DuckDB release* for `C_STRUCT_UNSTABLE` and `CPP`. If you set
`USE_UNSTABLE_C_API=1`, `TARGET_DUCKDB_VERSION` must be a real release like `v1.5.5`, and
`v1.2.0` would pin the binary to DuckDB v1.2.0. `ScaffoldConfig` validates this pairing. Until
DuckDB 2.0 this was how quack-rs advised stamping a `duckdb-1-5` build; DuckDB 2.0 cannot load a
`C_STRUCT_UNSTABLE` binary (see P10), so it now applies only to pinning one 1.x release.

---

## P3: E2E testing is mandatory

**Status**: Documented. See testing guide.

**Symptom**: All unit tests pass but the extension is completely broken when loaded.

**Root cause**: Unit tests test Rust logic in isolation. They cannot detect:
- SEGFAULTs on extension load
- Functions failing to register silently
- Wrong results due to combine not propagating config

In duckdb-behavioral, 435 unit tests passed while the extension had three critical bugs:
1. SEGFAULT on load (wrong entry point)
2. 6 of 7 functions not registered (function set name bug)
3. Wrong results from window_funnel (combine not propagating config)

**Fix**: Always run E2E tests using the actual DuckDB CLI, loading the
packaged `.duckdb_extension`. DuckDB refuses a bare `.so` ("DuckDB extensions are
files ending with '.duckdb_extension'"), and refuses a renamed `.so` that has no metadata
footer ("The metadata at the end of the file is invalid"), so append the footer with
`append_metadata` and start the CLI with `-unsigned`:
```sql
LOAD './my_extension.duckdb_extension';
SELECT my_function(col) FROM ...;
```

---

## P4: extension-ci-tools submodule must be initialized

**Status**: Build-time check, no SDK fix needed.

**Symptom**: `make configure` or `make release` fails.

**Root cause**: The community extension CI uses `extension-ci-tools` as a git submodule.
If it is not checked out, the Makefile cannot find the build scripts.

**Fix**: In a **new** project (for example one fresh from the scaffold) the
submodule has never been added: the scaffold writes `.gitmodules`, but a file
cannot create the gitlink git needs, so `git submodule update --init` finds
nothing to do and exits 0 without cloning anything. Add it once, on the branch
the generated `Makefile` names (`v2.0-cyanoptera` for DuckDB 2.0 — extension-ci-tools
has no `v2.0.0` branch — or a 1.x release's own branch, such as `v1.5.5`, for a
`C_STRUCT_UNSTABLE` build):

```bash
git submodule add -b v2.0-cyanoptera https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
```

In a **clone** of a repository that already has the submodule:

```bash
git submodule update --init --recursive
```

The generated `Makefile` checks for the checkout before it includes anything
from it and prints both commands if it is missing.

---

## P5: SQLLogicTest expected values must match exactly

**Status**: Test-authoring care required.

**Symptom**: Tests fail in CI but pass locally (or vice versa) due to output format differences.

**Root cause**: SQLLogicTest format is exact-match. Output formatting (decimal places, NULL
representation, etc.) must match exactly.

**Fix**: Generate expected values by running the actual SQL in DuckDB CLI and copying the output.

---

## P6: `duckdb_register_aggregate_function_set` silently fails

**Status**: Made impossible by builder (returns `Err`). Also see L6.

**Symptom**: Function appears to be registered but is not found when called in SQL.

**Root cause**: `duckdb_register_aggregate_function_set` returns `DuckDBError` silently when
the function set name is not set on individual members (see L6). The return value is often
ignored by extension authors.

**Fix**: The builder checks the return value and returns `Err` on failure.
Additionally, use `duckdb_get_function` to verify registration in development.

---

## P7: `duckdb_string_t` format is undocumented

**Status**: Handled by `VectorReader::read_str`, `VectorReader::read_blob`,
`read_duck_string`, and `read_duck_blob`.

**Symptom**: VARCHAR reading produces garbage, empty strings, or crashes; BLOB
reading silently drops bytes that are not valid UTF-8.

**Root cause**: DuckDB stores strings in a 16-byte struct with two formats:
- **Inline** (≤ 12 bytes): `[ len: u32 | data: [u8; 12] ]`
- **Pointer** (> 12 bytes): `[ len: u32 | prefix: [u8; 4] | ptr: *const u8 ]`
  (on a 32-bit target the pointer is 4 bytes and the last 4 are unused, and
  not necessarily zero)

The length and the pointer are in the target's own byte order: decoding them
as little-endian misreads every string on a big-endian target (quack-rs did,
until the fifth audit). This is not documented in `libduckdb-sys`.

**Fix**: Use `VectorReader::read_str` or `read_duck_string` for UTF-8 text. Use
`VectorReader::read_blob` or `read_duck_blob` for arbitrary binary data. All four
handle both storage formats.

---

## P8: `INTERVAL` struct layout is undocumented

**Status**: Handled by `DuckInterval` and `read_interval_at`.

**Symptom**: Interval calculations produce wrong results or crashes.

**Root cause**: DuckDB's `INTERVAL` is a 16-byte struct: `{ months: i32, days: i32, micros: i64 }`.
This layout is not documented in the Rust bindings.
Month conversion uses the approximation: **1 month = 30 days** (matching DuckDB's behavior).

**Fix**: Use `DuckInterval`, `read_interval_at`, and `interval_to_micros` from the `interval` module.

---

## P9: `loadable-extension` dispatch table uninitialised in `cargo test`

**Status**: Fixed in quack-rs v0.6.0. `InMemoryDb::open()` now initialises the dispatch
table automatically. No user action required.

**Symptom**: Any test that calls `InMemoryDb::open()` (or otherwise exercises the DuckDB C
API through `libduckdb-sys`) panics with:

```
DuckDB API not initialized or DuckDB feature omitted
```

The panic occurs in the generated `bindgen.rs` file at a function-pointer assertion, not in
user code. All other tests (pure-Rust logic, `AggregateTestHarness`, mock utilities) pass.

**Affected configuration**: `cargo test --features bundled-test` (or `--all-features`) on
any platform. The panic was first observed on macOS only because the regular CI job used
`cargo test --all-targets` without `--features bundled-test`, so the `in_memory_db` module
was never compiled or exercised in CI runs.

**Root cause** (deep):

quack-rs requires `libduckdb-sys` with `features = ["loadable-extension"]` so that, at
runtime, DuckDB can load the compiled `.so`/`.dylib` and populate a lazy function-pointer
dispatch table.  In `loadable-extension` mode, *every* DuckDB C API call—including
`duckdb_open`, `duckdb_query`, etc.—is routed through a global `AtomicPtr` per function.
The atomics start as null (panic stubs) and are initialised when DuckDB calls the
extension's `init_c_api` entry-point.

The `bundled-test` feature adds `duckdb = { features = ["bundled"] }` as a dependency.
`duckdb` in turn depends on `libduckdb-sys`.  Cargo's **feature-unification** rule merges
features across all uses of the same crate, so the single `libduckdb-sys` that ends up in
the test binary carries *both* `loadable-extension` AND `bundled`.

In this combined configuration the bundled DuckDB static library is linked into the binary,
but `loadable-extension` still intercepts every call through the uninitialized dispatch
table.  Since `cargo test` never goes through DuckDB's extension-loading path, the atomics
are never populated, and any DuckDB API call panics.

**Why macOS looked different**: The regular CI job ran `cargo test --all-targets` (no
`--all-features`), so `bundled-test` was off and the `in_memory_db` module was never
compiled.  The release workflow ran `cargo test --all-targets --all-features`, which did
enable `bundled-test`.  With `fail-fast: true`, macOS happened to fail first; the Linux and
Windows jobs were also broken but were cancelled before reporting.

**It happened again at v0.18.0** (2026-09): the release workflow failed on Windows in a
`FileFlag::CreateNew` test while main was green. DuckDB's Windows file system ignores the
exclusive-create flag (`O_EXCL` semantics), so an existing file opened instead of being refused,
and PR CI ran no `duckdb-1-5*` tests on Windows or macOS. CI's `test-bundled` job now also runs
the release command, `cargo test --all-targets --all-features`, on all three platforms.

**The fix**: DuckDB's own C++ codebase contains an internal inline function `CreateAPIv1()`
(in `duckdb/main/capi/extension_api.hpp`) that constructs the complete `duckdb_ext_api_v1`
struct, setting every one of its function-pointer fields (546 with the 1.5.2–1.5.6 headers, 548 with 2.0.0) to
the matching bundled DuckDB C symbol.  This is exactly the same struct that DuckDB would send to an extension's
`init_c_api` callback.

quack-rs compiled a tiny C++ shim (`src/testing/bundled_api_init.cpp`) that wraps
`CreateAPIv1()` as a C-linkage symbol `quack_rs_create_api_v1()`. (Since the DuckDB 2.0
update, `build.rs` instead generates a C shim from `duckdb_extension.h` that fills the struct
field by field, as `CreateAPIv1()` does — byte-for-byte equal to it on 1.5.6 and 2.0.0 — and
needs only C headers, so a 2.0 `duckdb-shared-libs` archive, which ships no `duckdb.hpp`,
works. The C++ shim, now C++17, remains for header directories without `duckdb_extension.h`,
such as the 1.4 zips.)  In `InMemoryDb::open()`,
a `std::sync::Once` guard calls this shim and feeds the result to
`libduckdb_sys::duckdb_rs_extension_api_init`, which populates all the atomics in one pass.
After that, the `duckdb` crate can open connections and run queries normally.

The header path is resolved at build time by `build.rs`, which searches for
`libduckdb-sys-*/out/duckdb/src/include` in Cargo's shared `build/` directory — the same
location where `libduckdb-sys` extracts the bundled DuckDB tarball.

**Risk and mitigation**: `extension_api.hpp` and `CreateAPIv1()` are **internal DuckDB
implementation details**, not part of the stable public C API.  They live in the DuckDB
source tree and can change between releases.  Known mitigations:

| Risk | Mitigation |
|------|-----------|
| `extension_api.hpp` is renamed or moved | `build.rs` will fail to compile with a clear error pointing at the missing header |
| `CreateAPIv1()` function is renamed | Same — C++ compile error |
| `duckdb_ext_api_v1` gains new fields in a future DuckDB version | `CreateAPIv1()` will fill all new fields too; the Rust struct (generated by `libduckdb-sys` bindgen from the same header set) will also have them; ABI stays consistent |
| `duckdb_ext_api_v1` field order changes | Within one DuckDB release both the C++ struct (`extension_api.hpp`) and the Rust struct (`libduckdb-sys` bindgen from `duckdb_extension.h`) are generated from the same source, so they stay in sync. **Across releases they do not** — see P10, which is about the loaded-extension case rather than this bundled-test one |
| libduckdb-sys switches away from `loadable-extension` dispatch | The problem disappears entirely; the `Once` guard becomes a cheap no-op |

The two header files that define `duckdb_ext_api_v1` are `duckdb_extension.h` (public,
used by extension authors and by `libduckdb-sys` bindgen) and `extension_api.hpp` (internal,
used by DuckDB's C++ extension-loader).  Both are maintained in the same DuckDB repository
release and always have identical field counts and field *order* — verified by
`scripts/check-abi-table.py`, which re-derives both from every release tag (459 entries for
DuckDB 1.4.x, 545 for 1.5.0–1.5.1, 546 for 1.5.2–1.5.6, 548 for 2.0.0).

**This is a genuine DuckDB ecosystem discovery**: the combination of `loadable-extension`
dispatch and bundled DuckDB is not documented anywhere in the `duckdb-rs` or `libduckdb-sys`
repositories.  The fix relies on `CreateAPIv1()` — an implementation detail that the DuckDB
team could consider promoting to a stable C API (`duckdb_get_api_v1()` or similar) to make
this class of workaround unnecessary.

---

## P10: The C API struct has a stable prefix and an unstable tail

**Status**: Detected at load time by `quack_rs::abi` (default `AbiPolicy::Strict`).

**Symptom**: An extension loads without complaint and then corrupts memory —
`double free or corruption`, a segfault, or silently wrong results — on a DuckDB
release other than the one it was built against. Nothing in the build or the load
warns you.

**Root cause**: DuckDB hands a loadable extension a pointer to a
`duckdb_ext_api_v1` struct of function pointers, and the extension calls through
it at compiled-in offsets. The struct has two regions:

| Region | Slots | Guarantee |
|--------|-------|-----------|
| Stable | 0–356 | Frozen since v1.2.0 — same slots, order and signatures in every release through v2.0.0 (two slots, 114 and 138, were renamed `varint` → `bignum` in v1.4.0 with an identical struct layout) |
| Unstable | 357+ | Before v1.5.6, DuckDB twice **inserted** entries in the middle, shifting every later slot; v2.0.0 only appended |

| DuckDB | Total slots | What moved |
|--------|-------------|------------|
| v1.2.0 – v1.2.2 | 408 | baseline |
| v1.3.0 – v1.3.2 | 428 | appended |
| v1.4.0 – v1.4.5 | 459 | `duckdb_create_varint` renamed to `duckdb_create_bignum`; appended |
| v1.5.0 – v1.5.1 | 545 | `duckdb_appender_clear` **inserted** at slot 410 |
| v1.5.2 – v1.5.6 | 546 | `duckdb_geometry_type_get_crs` **inserted** at slot 493 |
| v2.0.0 | 548 | `duckdb_create_timestamp_tz_ns`, `duckdb_get_timestamp_tz_ns` appended |

DuckDB v1.5.6 declares all 546 slots stable for an extension that targets C API v1.5.6.
v2.0.0 still declares C API v1.5.6 for this struct, and its first 546 declarations are
textually identical to v1.5.6's, so a build against the v1.5.2 – v1.5.6 bindings dispatches
correctly on v2.0.0 (`PREFIX_LAYOUTS` in `src/abi.rs`, derived by `scripts/check-abi-table.py`).

Everything behind quack-rs's `duckdb-1-5` / `duckdb-1-5-3` / `duckdb-1-5-4` features — 130 C API
functions (of the tail's 189 slots) covering scalar bind/init, copy functions, the Arrow
bridge, catalog access, `ErrorData`,
`FileSystem`, `Expression`, `SelectionVector`, config options, table descriptions
and the client context — sits in the unstable region.

DuckDB will not catch this for you. Its loader validates the ABI metadata in the
extension footer:

- `C_STRUCT` + a C API version (`v1.2.0`) — accepted by **any** DuckDB whose C API
  version is at least that, then handed the *whole* struct, unstable region
  included. There is no engine-version check.
  DuckDB v1.2.0 – v1.5.5 declare C API v1.2.0; v1.5.6 and v2.0.0 declare v1.5.6.
- `C_STRUCT_UNSTABLE` + a DuckDB release version — accepted only by that exact
  1.x release. DuckDB 2.0 routes every `C_STRUCT_UNSTABLE` binary to the C API v2
  entry point `<name>_init_c_api_v2` (`duckdb_extension_v2.h`), which quack-rs does
  not generate, so such a binary fails there with: did not contain function
  "<name>_init_c_api_v2".

So a `C_STRUCT` `v1.2.0` binary that touches the unstable region loads happily into a
DuckDB with a different layout and calls the wrong function pointers. Built
against v1.5.0 and loaded into v1.5.2+, for example,
`duckdb_scalar_function_set_bind_data(info, data, destroy)` actually invokes
`duckdb_scalar_function_get_client_context(info)`, and
`duckdb_connection_get_client_context` invokes `duckdb_destroy_client_context`.

DuckDB's own `extension-template-c` says as much:

> WARNING: When set to 1, the `duckdb_extension.h` from the `TARGET_DUCKDB_VERSION`
> must be used, using any other version of the header is unsafe.

**Reproduction** (DuckDB 1.5.5, extension built against libduckdb-sys `=1.10500.0`
and stamped `C_STRUCT`/`v1.2.0`):

```text
double free or corruption (out)
Aborted (core dumped)
```

**Fix**:

1. If you use `duckdb-1-5` / `duckdb-1-5-3` / `duckdb-1-5-4`, build against the
   v1.5.2 – v1.5.6 bindings (`libduckdb-sys` `~1.10506.0`) and stamp the binary
   `C_STRUCT` with C API v1.5.6 (`TARGET_DUCKDB_VERSION=v1.5.6` with
   `USE_UNSTABLE_C_API` unset, or `append_metadata --abi-type C_STRUCT
   --duckdb-version v1.5.6`). DuckDB before v1.5.6 then refuses it at `LOAD`, and
   v1.5.6 and v2.0.0 load it. Until DuckDB 2.0 this step said to stamp
   `C_STRUCT_UNSTABLE` with the exact release (`USE_UNSTABLE_C_API=1`); that still
   pins a binary to one 1.x release, but DuckDB 2.0 cannot load it, and
   extension-ci-tools `v2.0-cyanoptera` still stamps `C_STRUCT_UNSTABLE` when
   `USE_UNSTABLE_C_API=1`.
2. Leave `AbiPolicy::Strict` on. `quack_rs::abi::check()` compares the compiled-in
   slot count against the layout the running engine uses — resolved from
   `duckdb_library_version()`, which lives at stable slot 7 and is therefore always
   dispatched correctly — and the entry point turns a mismatch into a `LOAD` error
   naming both layouts and the remedy. Use `AbiPolicy::Trust` only when the binary's
   metadata already makes DuckDB refuse the wrong release. quack-rs 0.18.0 refused
   a `duckdb-1-5` build on DuckDB 2.0.0 ("layout cannot be verified"); since the
   DuckDB 2.0 update the guard accepts a 546-slot build on v2.0.0, still refuses a
   545-slot (1.5.0/1.5.1) build there, and treats v2.0.1 and v2.1.0 as unknown.
3. If you stay off those features, nothing changes: the stable prefix is frozen, so
   `C_STRUCT` + `v1.2.0` remains portable across every DuckDB from v1.2.0 through
   v2.0.0.

`scripts/check-abi-table.py` re-derives the layout table from every upstream
release header and runs in CI, so the table cannot silently go stale.

---

## P11: `const char *` returns are borrowed — freeing one corrupts the heap

**Status**: Fixed in `CopyGlobalInitInfo::get_file_path`; every other
`duckdb_free` call site in the crate audited against DuckDB's implementation.

**Symptom**: `corrupted size vs. prev_size in fastbins`, `free(): invalid
pointer`, or a `SIGABRT` at an unrelated later allocation. Nothing points at the
call that caused it.

**Root cause**: The C API returns strings two ways, and only one of them
transfers ownership.

| Return type | Typical implementation | Caller must |
|-------------|------------------------|-------------|
| `char *` | `strdup(...)` or `duckdb_malloc` + `memcpy` | `duckdb_free` it |
| `const char *` | `some_std_string.c_str()` | **not** free it |

`duckdb_copy_function_global_init_get_file_path` is the second kind — it returns
`info_ref.file_path.c_str()`, the interior pointer of a C++ `std::string` that
DuckDB still owns and will destroy itself. quack-rs called `duckdb_free` on it,
handing the allocator a pointer it never issued.

**The trap in the rule**: the signature alone is not enough.
`duckdb_parameter_name` is declared `const char *` and yet returns
`strdup(identifier.c_str())` — it *is* owned, and *not* freeing it leaks. The
only reliable check is reading the implementation in DuckDB's `src/main/capi/`.

**Audit performed** (DuckDB 1.5.5, every `duckdb_free` site in quack-rs):

| Function | Implementation | quack-rs |
|----------|----------------|----------|
| `duckdb_logical_type_get_alias` | `strdup` | frees ✓ |
| `duckdb_enum_dictionary_value` | `strdup` | frees ✓ |
| `duckdb_struct_type_child_name` | `strdup` | frees ✓ |
| `duckdb_union_type_member_name` | `strdup` | frees ✓ |
| `duckdb_get_varchar` | `duckdb_malloc` + `memcpy` | frees ✓ |
| `duckdb_value_to_string` | `duckdb_malloc` + `memcpy` | frees ✓ |
| `duckdb_get_blob` | `malloc` + `memcpy` | frees ✓ |
| `duckdb_table_description_get_column_name` | `malloc` + `memcpy` | frees ✓ |
| `duckdb_parameter_name` | `strdup` (despite `const char *`) | frees ✓ |
| `duckdb_get_or_create_from_cache` (`out_error`) | `strdup` | frees ✓ |
| `duckdb_table_description_error` | `wrapper->error.c_str()` | does not free ✓ |
| `duckdb_appender_error` | `error_data.RawMessage().c_str()` | does not free ✓ |
| `duckdb_copy_function_global_init_get_file_path` | `file_path.c_str()` | **was freeing ✗** |

**How it was found**: by writing the first live test for copy functions. The
module had 16 unit tests and not one of them registered a copy function against
a real DuckDB, so the corruption had never had a chance to happen. Unit tests
over an FFI wrapper test the wrapper's arithmetic, not its contract with the
library.

**Rule**: before calling `duckdb_free` on anything the C API returned, read the
implementation. `const char *` is a strong hint that it is borrowed, but
`duckdb_parameter_name` proves it is only a hint.


## P12: `duckdb_client_context_get_config_option` aborts on a missing setting

**Status**: Documented on `ClientContext::config_option`; the abort-free
alternative is SQL. This is a DuckDB defect, not a quack-rs one.

**Symptom**: `Assertion 'scope != SettingScope::INVALID' failed` and a
`SIGABRT`, when asking for a configuration option that does not exist — but only
against a DuckDB built with debug assertions. Release builds return `NULL`
exactly as documented, so this never reproduces for end users and always
reproduces in a test suite that links a debug DuckDB.

**Root cause** (DuckDB 1.5.5):

```cpp
// src/main/capi/config_options-c.cpp
switch (ctx.TryGetCurrentSetting(option_name, result).GetScope()) {
  ...
  default:                                    // <- INVALID is handled here
    res_scope = DUCKDB_CONFIG_OPTION_SCOPE_INVALID;

// src/include/duckdb/main/setting_info.hpp
SettingScope GetScope() {
    D_ASSERT(scope != SettingScope::INVALID); // <- but never reached in debug
    return scope;
}
```

The `default:` arm shows the not-found case is *meant* to be tolerated; the code
just calls `GetScope()` before checking `operator bool()`.

**Workaround**: ask SQL instead — it has no assertion on this path.

```sql
SELECT count(*) FROM duckdb_settings() WHERE name = 'my_setting';
```

**Rule**: `ClientContext::config_option` is for settings you registered or know
exist. Do not use it as an existence probe.


## P13: A `VARIANT` in a function signature crashes DuckDB

**Status**: Guarded. `LogicalType::try_new` refuses `TypeId::Variant` with an
error that names the workaround. This is a DuckDB defect, not a quack-rs one.

**Symptom**: the function registers without error. The first query that gives it
a `VARIANT` argument or result either segfaults (DuckDB 2.0.0, reproduced with
the guard bypassed) or fails with an internal error that invalidates the
database (1.5.x).

**Root cause**: DuckDB stores a `VARIANT` as a `STRUCT`, and the type that
`duckdb_create_logical_type` returns for the id alone has no such layout.

**Workaround**: declare the parameter as `ANY` (`TypeId::Any`), which accepts a
`VARIANT`.

**Rule**: a type that DuckDB builds from more than its id cannot be made from
the id alone.


## P14: An aggregate with no parameter counts no rows on DuckDB 2.0

**Status**: Guarded. Registering an aggregate, or a set overload, with no
parameter is an error that names the workaround. This is a DuckDB behaviour
change in 2.0, not a quack-rs defect.

**Symptom**: `SELECT my_count() FROM t` over 100 rows answers 100 on DuckDB
1.5.6 and 0 on 2.0.0 (measured against both). A windowed call of such an
aggregate never reaches `update` on either.

**Root cause**: the C API gives `update` a chunk and the state array and no row
count; the rows are `chunk.size()`. DuckDB 2.0 stopped setting the chunk's
cardinality (`CAPIAggregateUpdate`, `aggregate_function-c.cpp`), and a chunk with
no column reports a size of 0.

**Workaround**: give the aggregate one parameter, and for a zero-argument SQL
spelling put a macro in front of it: `CREATE MACRO my_count() AS my_count_1(1)`.

**Rule**: an aggregate needs at least one column to have a row count.


## P15: The entry of a NULL `LIST` or `MAP` row holds stale bytes

**Status**: Prevented by `ListVector::get_entry_checked` and
`MapVector::get_entry_checked`. `get_entry` keeps its contract and its docs now
state the hazard.

**Symptom**: reading `offset` and `length` of a NULL row and indexing the child
with them reads out of bounds. On DuckDB 2.0.0 lengths around 7·10¹³ were seen,
and a segmentation fault in a minority of runs.

**Root cause**: DuckDB writes no entry for a NULL row, so the entry array holds
whatever bytes the allocation had.

**Fix**: use `get_entry_checked`, which returns `None` for a NULL row or an entry
that does not fit inside the child, or test the row's validity before reading
the entry.

**Rule**: never read a list entry before the row's validity.


## P16: `duckdb_get_bit` and `duckdb_get_bignum` abort on NULL and on values that do not cast

**Status**: Documented. quack-rs has no `as_bit` or `as_bignum` wrapper; one would have to
make the checks below first. This is a DuckDB defect, not a quack-rs one.

**Symptom**: the process aborts with `fatal runtime error: Rust cannot catch foreign
exceptions` (`SIGABRT`, exit code 134) when `duckdb_get_bit` is called on a SQL `NULL`
value or on a value that does not cast to `BIT` (such as the string `'abc'`). Reproduced on
DuckDB 1.5.6 and 2.0.0. `duckdb_get_bignum` is built the same way (read in the 2.0.0
source, not run).

**Root cause**: both functions cast the value (`DefaultCastAs(BIT)` or `BIGNUM`) and read
the result with no `try`, so the cast's C++ exception leaves an `extern "C"` function.

**Fix**: before calling either, check `Value::type_id()` for the type you expect and
`Value::is_sql_null()`; both are plain accessors that cannot throw.

**Rule**: a C API getter that casts can throw; check the type and NULL-ness first, because
Rust cannot catch a C++ exception.


## P17: `std::fs` bypasses DuckDB's permission settings

**Status**: Documented in the [Virtual File
System](https://quack-rs.com/duckdb-1-5/file-system.html#security-only-filesystem-honours-duckdbs-permission-settings)
chapter, in the table-function, replacement-scan and `COPY` chapters, in [Known
Limitations](https://quack-rs.com/reference/known-limitations.html) and in the FAQ. quack-rs's `FileSystem` honours the
settings; nothing can make `std::fs` do so.

**Symptom**: with `SET enable_external_access = false` (or `allowed_directories` set),
`read_csv('data.csv')` fails with "Permission Error: Cannot access file … file system
operations are disabled by configuration", and so does a table function that opens the same
path through `FileSystem`. A table function that reads it with `std::fs::read` returns the
file. Measured on DuckDB 1.5.6 and 2.0.0.

**Root cause**: DuckDB enforces the settings in its own file system layer. `std::fs` calls
the operating system directly and never consults it.

**Fix**: open every path that comes from SQL (arguments, options, `COPY` targets,
replacement-scan names) with `FileSystem::from_client_context(...)` (feature `duckdb-1-5`).
Without that feature, take the data as a `VARCHAR` or `BLOB` argument.

**Rule**: an extension runs with the process's full privileges; the only restrictions it
inherits are the ones it asks DuckDB to apply.


## Community Extension Submission

### Build System Requirements

DuckDB's **C Extension API** now allows pure-Rust extensions **without any C++ glue**.
The official [extension-template-rs](https://github.com/duckdb/extension-template-rs) demonstrates
this approach. A pure-Rust extension needs:

1. **Cargo.toml**: `cdylib` crate type, pinned `duckdb` + `libduckdb-sys` deps, release profile
2. **Makefile**: Delegates to `cargo build` + metadata scripts from `extension-ci-tools`
3. **extension-ci-tools**: Git submodule for the DuckDB extension CI/CD pipeline
4. **src/lib.rs**: Entry point using `duckdb_entrypoint_c_api` macro + function registration
5. **description.yml**: Extension metadata (`language: Rust` and `build: cargo` for Rust extensions)
6. **test/sql/*.test**: SQLLogicTest format integration tests

Use `quack_rs::scaffold::generate_scaffold` to auto-generate all of these files from a
[`ScaffoldConfig`](https://docs.rs/quack-rs/latest/quack_rs/scaffold/struct.ScaffoldConfig.html).

> **Note**: The C Extension API has a stable and unstable part. The official template enables
> the unstable API via `USE_UNSTABLE_C_API=1` in the Makefile. As of 2026-10-29 its `main`
> targets DuckDB v2.0.0 on the C API v2 (`CAPI_VERSION=2`, `duckdb-neo`), builds binaries
> that "load only in the exact TARGET_DUCKDB_VERSION", and so needs a
> `<name>_init_c_api_v2` entry point, which quack-rs does not generate. For a quack-rs
> extension, including one using the `duckdb-1-5` features, stamp the build `C_STRUCT`
> and use `TARGET_DUCKDB_VERSION=v1.5.6` without `USE_UNSTABLE_C_API` instead (P10).
> extension-ci-tools `v2.0-cyanoptera` stamps an unstable build `C_STRUCT_UNSTABLE`, which
> DuckDB 2.0 sends to the v2 entry point. See
> [extension-template-rs](https://github.com/duckdb/extension-template-rs) for details.

### description.yml

Required fields:
```yaml
extension:
  name: your_extension
  description: One-line description
  version: 0.1.0
  language: Rust
  build: cargo
  license: MIT
  requires_toolchains: rust;python3
  excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"  # optional
  maintainers:
    - Your Name

repo:
  github: yourorg/your_extension
  ref: 0123456789abcdef0123456789abcdef01234567  # the commit to build, never a branch
```

`repo.ref` is the commit the community build checks out; a branch name would
build whatever the branch points at on the day. The book's
[Publishing page](https://quack-rs.com/publishing.html) shows the exact file the
scaffold generates.

Use `quack_rs::validate` to check name, version, and license before submission,
or use `quack_rs::scaffold::generate_scaffold` to auto-generate all project files.

### Naming Rules

- Extension names must be globally unique across the entire DuckDB community extensions ecosystem
- Check existing names at https://community-extensions.duckdb.org/ before choosing
- Use vendor prefixing to avoid collisions (e.g., `myorg_analytics` instead of `analytics`)
- Names must match `^[a-z][a-z0-9_]*$` and not exceed 64 characters (no hyphens: DuckDB
  looks up the entry point as `<name>_init_c_api`, which cannot contain one)
- The `[lib] name` in `Cargo.toml` MUST match the extension name (Pitfall P1)

### Platform Targets

Community extensions are built for the targets in `config/distribution_matrix.json` of
`duckdb/extension-ci-tools`. The authoritative copy in this repository is the table in
`book/src/publishing.md` ("Platform targets"), which `scripts/check-platform-table.py`
checks against upstream in CI; it is not repeated here so there is one list to keep
current. (`linux_amd64_gcc4`, which older guides list, has been retired upstream.)

Use `excluded_platforms` in `description.yml` to skip platforms your extension cannot support.
Validate with `quack_rs::validate::validate_platform` and `validate_excluded_platforms`.

### Security Disclaimer

Community extensions are NOT vetted for security by the DuckDB team. The community extensions
repository is a distribution mechanism, not a security guarantee. As an extension author:

- Never let a panic escape an FFI boundary (`quack-rs` catches them in its callbacks,
  which requires `panic = "unwind"`; `validate_release_profile` rejects `abort`)
- Validate all user inputs at system boundaries
- Do not include secrets, credentials, or API keys in your extension binary
- Follow the OWASP top 10 where applicable (SQL injection via dynamic SQL, etc.)

### Extension Versioning

DuckDB core extensions use a three-tier versioning scheme. Community extensions should follow
the same convention:

| Level | Format | Example | Meaning |
|-------|--------|---------|---------|
| **Unstable** | Short git hash (7+ hex chars) | `690bfc5` | No stability guarantees |
| **Pre-release** | Semver `0.y.z` | `0.1.0` | Working toward stability |
| **Stable** | Semver `x.y.z` (x>0) | `1.0.0` | Full semver, stable API |

Key points:

- **Unstable** extensions may change or remove functionality at any time
- **Pre-release** extensions follow semver but the API may still have breaking changes in minor versions
- **Stable** extensions guarantee backwards-compatible APIs; breaking changes require a major version bump
- `quack_rs::validate::validate_extension_version` accepts these three and anything
  else the community build accepts: up to 64 characters from `[A-Za-z0-9._+-]`
- Use `quack_rs::validate::semver::classify_extension_version` to determine the stability tier

### Extension Binary Compatibility

Extension binaries are tied to a platform, and to DuckDB versions according to
the ABI type stamped in their metadata footer:

- A `C_STRUCT` extension declares a C extension API version (quack-rs stamps
  `v1.2.0`) and loads into every DuckDB release that provides that API — every
  release from 1.4.4 through 2.0.x, for quack-rs's stable-API surface. One
  binary serves them all. An extension using the `duckdb-1-5` features is stamped
  C API `v1.5.6` and loads into 1.5.6 and 2.0.x.
- A `C_STRUCT_UNSTABLE` or `CPP` extension declares an exact DuckDB release and
  loads only into that release, so it is rebuilt for each one. DuckDB 2.0 loads a
  `C_STRUCT_UNSTABLE` binary only through the C API v2 entry point, which quack-rs
  does not generate.
- DuckDB verifies the footer before loading and refuses a mismatched binary
- All official extensions are cryptographically signed by the DuckDB team
- Unsigned extensions require `allow_unsigned_extensions` to load (development only)
- The DuckDB extension template provides CI workflows for automated cross-platform builds

### CI Toolchain Notes

The community extension CI uses specific compiler versions and system libraries. Common issues:

- Rust toolchain must be available in CI (add `rustup` setup to your CI workflow)
- Cross-compilation for `linux_arm64` from `linux_amd64` requires the appropriate target
- WASM targets (`wasm_mvp`, `wasm_eh`, `wasm_threads`) may not work with all Rust crates
- For DuckDB 2.0, extension-ci-tools builds every platform, WASM included, with Rust
  1.97.1 (before: stable, and 1.86.0 for WASM) and emsdk 6.0.9 (before: 3.1.71);
  `osx_amd64` is opt-in there, and DuckDB 2.1 drops the platform
- Use `excluded_platforms` to skip targets that cannot be built

---

## Architecture Decision Records

All six ADRs (the design principles ADR-1 to ADR-3, and the FFI-integration
decisions ADR-4 to ADR-6) are in
[docs/architecture.md](docs/architecture.md#architecture-decision-records).
