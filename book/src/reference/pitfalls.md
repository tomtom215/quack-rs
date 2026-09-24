# Pitfall Catalog

All known DuckDB Rust FFI pitfalls, discovered while building
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a
production DuckDB community extension. Every future developer who builds a Rust
DuckDB extension will hit the majority of these. quack-rs makes most of them
impossible.

---

## L1: COMBINE must propagate ALL config fields

**Status**: Testable with `AggregateTestHarness`.

**Symptom**: Aggregate function returns wrong results. No error, no crash.

**Root cause**: DuckDB's segment tree creates fresh target states, initialised
by `state_init` (with `FfiState<T>`, a `T::default()`), then calls `combine` to
merge source states into them. If your `combine` only propagates data fields
(`count`, `sum`) but omits configuration fields (`window_size`, `mode`), the
configuration is still its `state_init` default at `finalize` time, silently
corrupting results.

This bug passed 435 unit tests before being caught by E2E tests.

**Fix**:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# #[derive(Default)] struct MyState { window_size: i64, mode: u8, count: i64 }
# impl AggregateState for MyState {}
unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        if let (Some(src), Some(tgt)) = (
            FfiState::<MyState>::with_state(src_ptr),
            FfiState::<MyState>::with_state_mut(tgt_ptr),
        ) {
            tgt.window_size = src.window_size;  // config — MUST copy
            tgt.mode = src.mode;                // config — MUST copy
            tgt.count += src.count;             // data — accumulate
        }
    }
}
```

Test this with `AggregateTestHarness::combine` — see [Testing Guide](../testing.md).

---

## L2: State destroy double-free

**Status**: Made impossible by `FfiState<T>`.

**Symptom**: Crash or memory corruption on extension unload.

**Root cause**: If `state_destroy` frees the inner `Box` but does not null the
pointer, a second `state_destroy` call (common in error paths) frees
already-freed memory → undefined behavior.

**Fix**: `FfiState<T>::destroy_callback` nulls `inner` after freeing. Use it
instead of writing your own destructor:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# #[derive(Default)] struct MyState { window_size: i64, mode: u8, count: i64 }
# impl AggregateState for MyState {}
unsafe extern "C" fn state_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<MyState>::destroy_callback(states, count) };
}
```

---

## L3: No panic across FFI boundaries

**Status**: Made impossible by `init_extension` and the callback guards (which require `panic = "unwind"`).

**Symptom**: Extension causes DuckDB to crash or behave unpredictably.

**Root cause**: a panic cannot unwind out of an `extern "C"` function. Since
Rust 1.81 the runtime aborts the process when one tries (before 1.81 it was
undefined behaviour), so an uncaught `panic!()` or `.unwrap()` in a callback
takes down the user's whole DuckDB session.

**Fix**: Use `Result` and `?` inside `init_extension`. Never use `unwrap()` in
FFI callbacks. `FfiState::with_state_mut` returns `Option`, not `Result`, so
callers use `if let`:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# #[derive(Default)] struct MyState { window_size: i64, mode: u8, count: i64 }
# impl AggregateState for MyState {}
# unsafe fn demo(state_ptr: duckdb_aggregate_state) {
// Safe pattern — no unwrap in FFI callback
if let Some(st) = unsafe { FfiState::<MyState>::with_state_mut(state_ptr) } {
    st.count += 1;
}

// Dangerous — never do this in an FFI callback
let st = unsafe { FfiState::<MyState>::with_state_mut(state_ptr) }.unwrap(); // panics if None
# }
```

quack-rs's callback macros and typed builders catch a panic and report it as a
SQL error. That requires `panic = "unwind"` in the release profile, which is
what the scaffold generates and what `validate_release_profile` insists on:
under `panic = "abort"` nothing can be caught.

---

## L4: `ensure_validity_writable` is required before NULL output {#l4-ensure_validity_writable-is-required-before-null-output}

**Status**: Made impossible by `VectorWriter::set_null`.

**Symptom**: NULLs you write are silently lost — the row reads back as a
valid value (whatever is in the data buffer).

**Root cause**: a vector that has never held a NULL usually has no validity
mask at all, and `duckdb_vector_get_validity` then returns NULL (as `duckdb.h`
documents). `duckdb_validity_set_row_invalid` returns early on a NULL mask, so
nothing is written and nothing crashes. `duckdb_vector_ensure_validity_writable`
allocates the mask, after which `get_validity` returns it. (Dereferencing the
NULL pointer yourself, instead of going through the C API helpers, would
crash.)

**Fix**: Always call `duckdb_vector_ensure_validity_writable` before accessing
the validity bitmap on the write path. `VectorWriter::set_null` does this
automatically:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(writer: &mut VectorWriter, row: usize) {
// Correct — handled by set_null
unsafe { writer.set_null(row) };

// Wrong — validity bitmap may not be allocated yet
// let validity = duckdb_vector_get_validity(output);          // NULL
// duckdb_validity_set_row_invalid(validity, row);            // silently ignored
# }
```

For `STRUCT` and `ARRAY` outputs `set_null` also nulls the children at that
row, as DuckDB's internal `FlatVector::SetNull` does; a bare
`duckdb_validity_set_row_invalid` on the parent leaves the fields valid, and
`struct_extract` on the NULL row returns their stale values.

---

## L5: Boolean reading must use `u8 != 0`, not `*const bool`

**Status**: Made impossible by `VectorReader::read_bool`.

**Symptom**: Undefined behavior; Rust requires `bool` to be exactly 0 or 1.

**Root cause**: DuckDB's C API does not guarantee that boolean values in vectors
are exactly 0 or 1. Values of 2, 255, etc. cast to Rust `bool` is undefined
behavior.

**Fix**: Read as `u8` and compare with `!= 0`. `VectorReader::read_bool` always
does this:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(reader: &VectorReader, row: usize) {
let b: bool = unsafe { reader.read_bool(row) };  // safe: uses u8 != 0 internally
# }
```

---

## L6: Function set name must be set on EACH member

**Status**: Made impossible by `AggregateFunctionSetBuilder`.

**Symptom**: Functions are silently not registered. No error returned.

**Root cause**: When using `duckdb_register_aggregate_function_set`, the function
name must be set on EACH individual `duckdb_aggregate_function` using
`duckdb_aggregate_function_set_name`, not just on the set.

This is completely undocumented. Discovered by reading DuckDB's C++ test code
at `test/api/capi/test_capi_aggregate_functions.cpp`.

In duckdb-behavioral, 6 of 7 functions failed to register silently due to this
bug.

**Fix**: `AggregateFunctionSetBuilder` calls `duckdb_aggregate_function_set_name`
on every individual function before adding it to the set. Use it instead of
managing the set manually.

---

## L7: LogicalType memory leak

**Status**: Made impossible by `LogicalType` RAII wrapper.

**Symptom**: Memory leak proportional to number of registered functions.

**Root cause**: `duckdb_create_logical_type` allocates memory that must be freed
with `duckdb_destroy_logical_type`. Forgetting leaks memory.

**Fix**: `LogicalType` implements `Drop` and calls `duckdb_destroy_logical_type`
automatically when it goes out of scope.

---

## L8: `DEFAULT_NULL_HANDLING` does not propagate NULLs for scalar functions

**Status**: Made impossible by `ScalarFunctionBuilder::map1` / `map2` /
`map1_str` / `map2_str`. `DataChunk::propagate_nulls` fixes it in one line for
hand-written callbacks.

**Symptom**: A scalar function returns a value where SQL requires NULL — but only
for arguments that come from a column. `SELECT f(NULL)` looks correct, because a
literal NULL is constant-folded before the function is reached, so the bug
survives review and ships.

**Root cause**: The name suggests DuckDB returns NULL on your behalf. For a
scalar function registered through the C API it does not, at run time:
`CAPIScalarFunction` calls the callback for every row including NULL ones and
checks only the *error* flag, and the one NULL check in `ExpressionExecutor` —
`VerifyNullHandling` — has its entire body inside `#ifdef DEBUG`. Every DuckDB a
user installs is a release build.

**Fix**: use the typed closure constructors, which skip NULL rows and write NULL
for them, or call `DataChunk::propagate_nulls(&mut writer)` at the end of a
hand-written callback. `map1_opt` / `map2_opt` and
`NullHandling::SpecialNullHandling` are for functions that genuinely mean to see
NULLs. **Aggregates are no different**: `update` receives NULL rows under either
setting too, so check `is_valid` before reading (see L12).

---

## L9: `duckdb_data_chunk_from_arrow` takes the array even when it fails

**Status**: Made impossible by `arrow::data_chunk_from_arrow`, which takes the
`ArrowArray` **by value**.

**Symptom**: One of two opposite bugs, depending on which way you guessed. Treat
the array as still yours after a failed conversion and you double-release it.
Treat it as gone in every case and a zero-column conversion leaks the whole
Arrow buffer tree.

**Root cause**: `duckdb.h` says "Data ownership is passed on to DuckDB's
DataChunk", which reads like a success-path statement. `arrow-c.cpp` sets
`arrow_array->release = nullptr` inside the per-column loop, *before* the work
that can throw — so the array is claimed on the error path too, but only if the
loop runs at all. A zero-column converted schema leaves `release` intact and the
array still belongs to the caller.

**Fix**: own the record in a wrapper whose `Drop` releases only if `release`
survived, and consume it by value. The by-value binding drops on the way out: a
no-op when DuckDB nulled `release`, a correct release when it did not. The
mirror case is handled by the same rule — `ToArrowSchema` / `ToArrowArray`
install `release` last, so a failed *export* leaves nothing to free.

---

## L10: Scalar bind data is dropped when `DuckDB` copies the expression

**Status**: Fixable only from the extension, and now possible:
[`ScalarBindInfo::set_bind_data_copy`].

**Symptom**: A scalar function that allocates per-query state in its bind
callback reads **null** from `duckdb_scalar_function_get_bind_data` during
execution, for some queries and not others. Nothing crashes and nothing is
reported: the callback simply runs without the state it bound, so the answer is
quietly wrong.

**Root cause**: `duckdb_scalar_function_set_bind_data` registers the pointer and
its destructor, but *not* how to duplicate it. `DuckDB` copies a bound
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
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# use std::os::raw::c_void;
# use quack_rs::scalar::ScalarBindInfo;
# #[derive(Clone)] struct MyBindData;
# unsafe extern "C" fn destroy(p: *mut c_void) { drop(unsafe { Box::from_raw(p.cast::<MyBindData>()) }); }
# unsafe fn demo(bind_info: ScalarBindInfo, boxed: Box<MyBindData>) {
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
# }
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

**Status**: A `DuckDB` defect, reported upstream as
[duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Cannot be prevented or detected from an extension;
documented on `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder` and
`FfiState`.

**Symptom**: An aggregate that works under `SELECT agg(x) FROM t` and `GROUP BY`
segfaults (or corrupts memory, or returns a wrong answer) when used as a window
over a whole-partition frame — `agg(x) OVER ()`, `OVER (PARTITION BY p)` — or as
an ordered aggregate, `agg(x ORDER BY y)`.

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
`DuckDB` fixes it, document for your users that the aggregate must not be used
in those two query shapes. Frames that are not whole-partition (`ROWS BETWEEN 5
PRECEDING AND CURRENT ROW`, segment-tree windows) and `DISTINCT` windows were
checked and work.

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
this book) that DuckDB's aggregate executor filters NULL rows out before
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

A related trap under either setting: in a correlated subquery,
`(SELECT my_count(x) FROM t2 WHERE t2.k = t1.k)` is NULL, not `my_count` of an
empty input, for an outer row with no match. DuckDB rewrites that NULL to 0 only
for its own `count`. Wrap the subquery in `coalesce(..., 0)` if it matters.

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
check); CI job `test-older-engines` runs the suite against DuckDB 1.4.4 (default
features) and 1.5.0 (`duckdb-1-5`).

**Symptom**: code tested against the release `Cargo.lock` pins (1.5.5) aborts
or misbehaves in an older release the same binary loads into. A default-feature
extension loads into every release from 1.4.4; a `duckdb-1-5` one built against
the 1.5.4 bindings has the 546-slot layout of 1.5.2 to 1.5.5, so the ABI guard
rightly lets it load into all four.

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

## P1: Library name must match extension name

**Status**: Must be configured in `Cargo.toml`. Scaffold handles this.

**Symptom**: Community build fails with `FileNotFoundError`.

**Root cause**: The community build expects `lib{extension_name}.so`. If the
Cargo crate name produces a different `.so` filename, the build fails.

**Fix**: Set `name` explicitly in `[lib]`:

```toml
[lib]
name = "my_extension"   # Must match description.yml `name: my_extension`
crate-type = ["cdylib", "rlib"]
```

---

## P2: Metadata version is C API version, not DuckDB version

**Status**: `DUCKDB_API_VERSION` constant encodes the correct value.

**Symptom**: The metadata script succeeds, and `LOAD` then refuses the file:
"The file was built for DuckDB C API version 'v1.5.5', but we can only load
extensions built for DuckDB C API 'v1.2.0' and lower" (verified on DuckDB 1.4.4
and 1.5.5 with a file stamped `-dv v1.5.5`).

**Root cause**: The `-dv` flag to `append_extension_metadata.py` must be the
C API version (`v1.2.0`), not the DuckDB release version (`v1.4.4`). These are
different strings.

**Fix**: Use `quack_rs::DUCKDB_API_VERSION` (`"v1.2.0"`) in `init_extension`,
and use the same version with `append_extension_metadata.py -dv v1.2.0`.

---

## P3: E2E testing is mandatory

**Status**: Documented. See [Testing Guide](../testing.md).

**Symptom**: All unit tests pass but the extension is completely broken.

**Root cause**: Unit tests cannot detect SEGFAULTs on load, silent registration
failures, or wrong results from combine bugs.

**Fix**: Always run E2E tests using an actual DuckDB binary. The scaffold
generates a complete SQLLogicTest skeleton.

---

## P4: `extension-ci-tools` submodule must be initialized

**Status**: Build-time check.

**Symptom**: `make configure` or `make release` fails.

**Fix**: In a **new** project (for example one fresh from the scaffold) the
submodule has never been added: the scaffold writes `.gitmodules`, but a file
cannot create the gitlink git needs, so `git submodule update --init` finds
nothing to do and exits 0 without cloning anything. Add it once:

```bash
git submodule add https://github.com/duckdb/extension-ci-tools.git extension-ci-tools
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

**Symptom**: Tests fail in CI but pass locally (or vice versa).

**Root cause**: SQLLogicTest does exact string matching. Output format (decimal
places, NULL representation, column separators) must match character-for-character.

**Fix**: Generate expected values by running the SQL in DuckDB CLI and copying
the output. NULL is `NULL` (uppercase). Integers have no decimal places.

---

## P6: `duckdb_register_aggregate_function_set` silently fails

**Status**: Builder returns `Err`. Also see L6.

**Symptom**: Function appears registered but is not found in SQL.

**Root cause**: The return value of `duckdb_register_aggregate_function_set` is
often ignored. When it returns `DuckDBError`, the function set is not registered.

**Fix**: The builder checks the return value and propagates it as `Err`.

---

## P7: `duckdb_string_t` format is undocumented

**Status**: Handled by `VectorReader::read_str`, `VectorReader::read_blob`, and
`DuckStringView`.

**Symptom**: VARCHAR reading produces garbage, empty strings, or crashes; BLOB
reading silently drops bytes that are not valid UTF-8.

**Root cause**: DuckDB stores strings in a 16-byte struct with two formats
(inline ≤ 12 bytes, pointer > 12 bytes) that are not documented in
`libduckdb-sys`.

**Fix**: Use `VectorReader::read_str(row)` for UTF-8 text and
`VectorReader::read_blob(row)` for arbitrary binary data. See
[NULL Handling & Strings](../data/nulls-and-strings.md).

---

## P8: `INTERVAL` struct layout is undocumented

**Status**: Handled by `DuckInterval` and `read_interval_at`.

**Symptom**: Interval calculations produce wrong results or crashes.

**Root cause**: DuckDB's `INTERVAL` is `{ months: i32, days: i32, micros: i64 }`
(16 bytes total). This is not documented in `libduckdb-sys`. Month conversion
uses 1 month = 30 days (DuckDB's approximation).

**Fix**: Use `VectorReader::read_interval(row)` and `DuckInterval`. See
[INTERVAL Type](../data/intervals.md).

---

## P9: `loadable-extension` dispatch table uninitialised in `cargo test` {#p9}

**Status**: Fixed. `InMemoryDb::open()` initialises the dispatch table
automatically.

**Symptom**: All three `InMemoryDb` unit tests panic at runtime:

```text
thread 'testing::in_memory_db::tests::in_memory_db_opens' panicked at
'DuckDB API not initialized or DuckDB feature omitted'
```

This failure appears only when running `cargo test --features bundled-test`.
Regular `cargo test` (no feature) does not exercise this code path, so CI can
miss it entirely.

**Root cause**: Cargo's feature-unification merges `loadable-extension` (from
the main `libduckdb-sys` dependency) and `bundled-full` (pulled in by the
`duckdb` crate's `features = ["bundled"]`) into a single `libduckdb-sys` build
with **both features active**. In `loadable-extension` mode every DuckDB C API
call is routed through an `AtomicPtr<fn>` dispatch table, which is normally
populated at extension-load time when DuckDB calls
`duckdb_rs_extension_api_init`. In `cargo test`, no DuckDB host process loads
the extension, so the table stays uninitialised and every call panics.

**Discovery**: This was triggered by the crates.io release workflow (which runs
`--all-features`) failing on macOS. Regular CI (`--no-default-features`,
`--all-targets`) never compiled the `bundled-test` path, so the bug was hidden
during development and code review.

**Fix** (implemented in quack-rs 0.6.0):

1. `src/testing/bundled_api_init.cpp` — a thin C++ shim that wraps DuckDB's
   internal `CreateAPIv1()` (from `duckdb/main/capi/extension_api.hpp`) as a
   C-linkage symbol:

   ```cpp
   #include "duckdb/main/capi/extension_api.hpp"
   extern "C" duckdb_ext_api_v1 quack_rs_create_api_v1() {
       return CreateAPIv1();
   }
   ```

2. `build.rs` — compiles the shim (via the `cc` crate) only when the
   `bundled-test` feature is active, locating the DuckDB headers from the
   `libduckdb-sys` build output directory.

3. `InMemoryDb::open()` — calls `init_dispatch_table_once()` before opening
   the connection. That function calls `quack_rs_create_api_v1()` once and
   feeds the result through `duckdb_rs_extension_api_init`, populating all 459
   `AtomicPtr` slots in the dispatch table. A `std::sync::Once` guard makes it
   safe to call from any number of threads and test cases.

4. CI `test-bundled` job — runs
   `cargo test --all-targets --features bundled-test` on Linux, macOS, and
   Windows on every PR, so this class of failure is caught before release.

**ABI compatibility note**: DuckDB's `duckdb_ext_api_v1` struct is defined
identically in both the public `duckdb_extension.h` (used by `libduckdb-sys`
bindgen) and the internal `extension_api.hpp` (used by `CreateAPIv1()`). Both
include the `DUCKDB_EXTENSION_API_VERSION_UNSTABLE` fields. `CreateAPIv1()` sets
all 459 fields. The Rust and C++ structs are produced from the same DuckDB
release and therefore stay in sync.

**Risk table** (using DuckDB's internal C++ API):

| Risk | Mitigation |
|------|-----------|
| `extension_api.hpp` is renamed or moved | `build.rs` fails with a clear compile error |
| `CreateAPIv1()` is renamed | Same — C++ compile error |
| `duckdb_ext_api_v1` gains new fields | `CreateAPIv1()` fills new fields too |
| `duckdb_ext_api_v1` field order changes | Both structs from same DuckDB release, stay in sync |
| `libduckdb-sys` drops `loadable-extension` dispatch | Problem disappears; `Once` guard becomes cheap no-op |

---

## P10: The C API struct has a stable prefix and an unstable tail {#p10}

**Status**: Detected at load time by [`quack_rs::abi`](https://docs.rs/quack-rs/latest/quack_rs/abi/index.html)
(default `AbiPolicy::Strict`).

**Symptom**: An extension loads without complaint and then corrupts memory —
`double free or corruption`, a segfault, or silently wrong results — on a
`DuckDB` release other than the one it was built against. Nothing in the build
or the load warns you.

**Root cause**: `DuckDB` hands a loadable extension a pointer to a
`duckdb_ext_api_v1` struct of function pointers, and the extension calls through
it at compiled-in offsets. The struct has two regions:

| Region | Slots | Guarantee |
|--------|-------|-----------|
| Stable | 0–356 | Frozen since v1.2.0 — same slots, order and signatures in every release through v1.5.5 (two slots, 114 and 138, were renamed `varint` → `bignum` in v1.4.0 with an identical struct layout) |
| Unstable | 357+ | `DuckDB` **inserts** entries in the middle, shifting every later slot |

`duckdb_appender_clear` landed at slot 410 in v1.5.0 and
`duckdb_geometry_type_get_crs` in the middle of v1.5.2's tail; each insertion
moves everything after it. An extension compiled against one layout and loaded
by another calls the wrong function through the right offset.

**Your action**: nothing, if you use `init_extension` — it verifies the layout
and refuses a mismatch. Two knobs matter:

- `QUACK_RS_TARGET_DUCKDB_VERSION` at build time stamps the release you built
  against, so a `DuckDB` newer than quack-rs's table is still accepted when your
  build genuinely targeted it. The community-extension CI rebuilds per release,
  so this is the normal path.
- `AbiPolicy::Warn` or `Trust` if you would rather load anyway. `Trust` is the
  old behaviour, and the failure mode above is the reason it is no longer the
  default.

If your extension enables no `duckdb-1-5*` feature it only calls into the stable
prefix, and `StableOnly` accepts every release from v1.2.0 on.

---

## P11: `const char *` returns are borrowed — freeing one corrupts the heap {#p11}

**Status**: Fixed in quack-rs; documented here because extension authors calling
the C API directly hit the same trap.

**Symptom**: `corrupted size vs. prev_size in fastbins`, `free(): invalid
pointer`, or a `SIGABRT` at an unrelated later allocation. Nothing points at the
call that caused it.

**Root cause**: The C API returns strings two ways, and only one transfers
ownership.

| Return type | Typical implementation | Caller must |
|-------------|------------------------|-------------|
| `char *` | `strdup(...)` or `duckdb_malloc` + `memcpy` | `duckdb_free` it |
| `const char *` | `some_std_string.c_str()` | **not** free it |

`duckdb_copy_function_global_init_get_file_path` is the second kind: it returns
`info_ref.file_path.c_str()`, the interior pointer of a C++ `std::string`
`DuckDB` still owns and destroys itself. Calling `duckdb_free` on it hands the
allocator a pointer it never issued.

**The trap in the rule**: the signature alone is not enough.
`duckdb_parameter_name` is declared `const char *` and yet returns
`strdup(identifier.c_str())` — it *is* owned, and *not* freeing it leaks. The
only reliable check is reading the implementation in `DuckDB`'s
`src/main/capi/`.

**Your action**: before calling `duckdb_free` on anything the C API returned,
read the implementation. `const char *` is a strong hint that it is borrowed,
but `duckdb_parameter_name` proves it is only a hint. Every `duckdb_free` site
in quack-rs was audited this way — see `LESSONS.md` P11 for the full table.

**How it was found**: by writing the first live test for copy functions. The
module had 16 unit tests and none of them registered a copy function against a
real `DuckDB`, so the corruption had never had a chance to happen. Unit tests
over an FFI wrapper test the wrapper's arithmetic, not its contract with the
library.

---

## P12: `duckdb_client_context_get_config_option` aborts on a missing setting {#p12}

**Status**: A `DuckDB` defect, not a quack-rs one. Documented on
`ClientContext::config_option`, with an abort-free alternative.

**Symptom**: `Assertion 'scope != SettingScope::INVALID' failed` and a
`SIGABRT` when asking for a configuration option that does not exist — but only
against a `DuckDB` built with debug assertions. Release builds return `NULL`
exactly as documented, so this never reproduces for end users and always
reproduces in a test suite that links a debug `DuckDB`.

**Root cause** (`DuckDB` 1.5.5):

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

**Your action**: use `ClientContext::config_option` for settings you registered
or know exist. To ask *whether* a setting exists, use SQL — it has no assertion
on this path:

```sql
SELECT count(*) FROM duckdb_settings() WHERE name = 'my_setting';
```

---

## Summary

| Pitfall | SDK status | Your action |
|---------|------------|-------------|
| L1: combine config fields | Testable | Test with `AggregateTestHarness::combine` |
| L2: state double-free | Prevented | Use `FfiState::destroy_callback` |
| L3: panic across FFI | Prevented | Use `init_extension`, no `unwrap` in callbacks |
| L4: NULL silently dropped (no validity mask) | Prevented | Use `VectorWriter::set_null` |
| L5: bool UB | Prevented | Use `VectorReader::read_bool` |
| L6: function set name | Prevented | Use `AggregateFunctionSetBuilder` |
| L7: LogicalType leak | Prevented | Use `LogicalType` (RAII) |
| L8: NULLs reach the callback anyway | Prevented | Use `map1`/`map2`, or `DataChunk::propagate_nulls` |
| L9: Arrow array taken on failure | Prevented | Use `arrow::data_chunk_from_arrow` (takes by value) |
| L10: bind data lost on expression copy | Prevented | Use `ScalarBindData::set` (or pair `set_bind_data` with `set_bind_data_copy`) |
| L11: aggregate crash under `OVER ()` / `ORDER BY` | DuckDB defect | Do not use C API aggregates in those query shapes |
| L12: aggregate `update` sees NULL rows | Documented | Skip rows where `is_valid` is false |
| L13: running window without a destructor | Prevented | Every aggregate builder registers a destructor |
| L14: C API differs across releases | Prevented | Wrappers that rely on newer behaviour check the engine version or check in Rust |
| P1: lib name mismatch | Scaffold | Set `[lib] name` in `Cargo.toml` |
| P2: API version string | Constant | Use `DUCKDB_API_VERSION` |
| P3: unit tests insufficient | Documented | Write SQLLogicTest E2E tests |
| P4: submodule not initialized | Build-time | New project: `git submodule add …`; clone: `git submodule update --init` |
| P5: SQLLogicTest exact match | Documented | Copy output from DuckDB CLI |
| P6: register set silent fail | Prevented | Builder returns `Err` |
| P7: VARCHAR format undocumented | Prevented | Use `VectorReader::read_str` |
| P8: INTERVAL layout undocumented | Prevented | Use `DuckInterval` |
| P9: dispatch table uninitialised | Fixed | `InMemoryDb::open()` initialises it via C++ shim |
| P10: unstable ABI tail shifts | Prevented | Use `init_extension`; set `QUACK_RS_TARGET_DUCKDB_VERSION` when building |
| P11: freeing a borrowed `const char *` | Fixed | Read the C++ impl before `duckdb_free`; prefer quack-rs wrappers |
| P12: config-option probe aborts (debug) | Documented | Ask `duckdb_settings()` in SQL instead |
