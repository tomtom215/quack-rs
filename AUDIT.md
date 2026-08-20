<!-- SPDX-License-Identifier: MIT -->

# quack-rs — production-readiness audit, August 2026

Reviewed at `v0.16.0`, against **DuckDB 1.5.4** (prebuilt `libduckdb`, x86-64
Linux) with `libduckdb-sys 1.10504.0`.

The question this audit set out to answer is not "does quack-rs work" — it does,
and the existing suite proves it — but "what would stop it being the default way
to write a production DuckDB extension in Rust". Everything below was checked
against a primary source: DuckDB's own headers and `.cpp` files at the release
tag, or a program run against a real database. Nothing here is inferred from
naming or from documentation alone; where DuckDB's documentation and its
implementation disagree, that is called out as a finding.

---

## 1. What was examined

**Primary sources read at DuckDB `v1.5.4` (and `main`, for the forward-looking
section):**

| Source | Why |
|---|---|
| `src/include/duckdb.h` | The full C API surface and its contracts |
| `src/include/duckdb_extension.h` at `v1.2.0`, `v1.3.0`, `v1.4.0`, `v1.4.4`, `v1.5.0`, `v1.5.3`, `v1.5.4`, `v1.5.5`, `main` | The `duckdb_ext_api_v1` layout per release |
| `src/main/capi/scalar_function-c.cpp` | Whether scalar inputs are flattened; NULL handling |
| `src/main/capi/aggregate_function-c.cpp` | Which aggregate callbacks have an error channel |
| `src/main/capi/cast_function-c.cpp`, `copy_function-c.cpp` | Flattening in the other callback kinds |
| `src/main/capi/duckdb_value-c.cpp` | Ownership and bounds contracts of the composite `Value` constructors |
| `src/main/capi/duckdb-c.cpp`, `src/main/connection.cpp`, `src/main/client_context.cpp` | Thread-safety of `duckdb_interrupt` / `duckdb_query_progress` |
| `src/main/extension/extension_load.cpp` | How the ABI footer is validated at `LOAD` |
| `src/execution/expression_executor/execute_function.cpp` | What `DEFAULT_NULL_HANDLING` actually does |
| `src/common/types/value.cpp` | Whether `Value::LIST`/`ARRAY` take the element type or the container type |

**Mechanical checks:**

- Every one of the 546 entries in `duckdb_ext_api_v1` (357 stable + 189
  unstable, as of v1.5.2–v1.5.5) diffed against every `duckdb_*` symbol
  quack-rs references.
- quack-rs's `KNOWN_LAYOUTS` table and `STABLE_API_SLOT_COUNT` re-derived
  independently from the release headers.
- The whole suite built and run against a real DuckDB.
- Throwaway probe programs written for each hypothesis, so that behaviour was
  observed rather than assumed. Three of the eight probes disproved the
  hypothesis they were written for; those are noted below, because a review that
  only reports its confirmations is not a review.

**Tools brought in:** Miri, LeakSanitizer, `cargo-fuzz`, `cargo-semver-checks`.

---

## 2. Defects found and fixed

### D1 — A panicking `Drop` in extension state aborted the process (High)

`FfiState<T>::destroy_callback`, `FfiBindData<T>::destroy`,
`FfiInitData<T>::destroy`, `FfiLocalInitData<T>::destroy`,
`replacement_scan::drop_box` and `TypedCallbacks::destroy_extra` all did
`drop(Box::from_raw(..))` directly inside an `extern "C" fn`. `T` is arbitrary
user data. Since Rust 1.81 an unwind across `extern "C"` is a guaranteed process
abort, not merely UB.

Reproduced against DuckDB 1.5.4: an aggregate whose state type has a panicking
`Drop` killed the test binary with `SIGABRT` from inside
`duckdb::RowOperations::DestroyStates`, on a task-scheduler thread —
`panic_cannot_unwind`, with no way for the host application to recover.

This contradicted the crate's headline claim ("FFI panics → *without quack-rs*:
process abort; *with quack-rs*: caught"). Every such destructor now runs under
`callback::catch_ffi_panic`, which is also public so extensions writing their own
`extern "C"` destructors get the same containment.

Where DuckDB offers an error channel, the panic is now *reported* rather than
swallowed: `CAPIAggregateStateInit` checks the error flag and throws, so a
panicking `Default::default()` becomes an ordinary SQL error instead of a silent
NULL. The state destructor has none — `CAPIAggregateDestructor` takes no info and
returns nothing — so there the message is discarded, which is still better than
aborting.

`FfiState::init_callback` additionally no longer forms a `&mut Self` over the
possibly-uninitialised allocation DuckDB hands it; the slot is `ptr::write`-
initialised with a null inner pointer before any user code runs.

### D2 — `DEFAULT_NULL_HANDLING` does not propagate NULLs for scalar functions (High)

quack-rs documented, in the API docs and the book:

> By default, DuckDB automatically propagates NULLs: if any argument to a
> function is NULL, the result is NULL **without your function callback being
> called**.

For a scalar function registered through the C API this is false at run time.

- `CAPIScalarFunction` flattens the input chunk and calls the extension's
  callback for **every** row, NULL rows included, then checks only the error
  flag. It never inspects the result's validity.
- `ExpressionExecutor::Execute` then calls `VerifyNullHandling`, whose entire
  body is inside `#ifdef DEBUG`. It *asserts* that the function already produced
  NULL wherever an input was NULL.

Every DuckDB a user installs is a release build, so that assertion is compiled
out and a callback that ignores validity is simply believed.

Observed on DuckDB 1.5.4 with a function that writes `999` unconditionally:

```
CREATE TABLE t(i BIGINT); INSERT INTO t VALUES (1), (NULL), (3);
SELECT i, f(i) FROM t;   -- 1 -> 999,  NULL -> 999 (valid!),  3 -> 999
SELECT f(NULL::BIGINT);  -- NULL
```

The last line is why this ships: a literal `NULL` is **constant-folded** during
binding, so the obvious spot check passes for a broken function and the wrong
answers only start when the argument comes from a column.

Fixed by correcting the documentation (with the source quoted), adding
`DataChunk::propagate_nulls` / `any_null` as the one-line fix for hand-written
callbacks, adding the typed constructors in D-gap-3 below which get it right by
construction, and recording it as pitfall **L8** in `LESSONS.md`. A regression
test pins DuckDB's current behaviour, so a future DuckDB that starts propagating
shows up as a test failure rather than a surprise.

**Aggregates are not affected** — DuckDB's aggregate executor really does filter
NULL rows before `update` under the default. The correction is specific to
scalar functions, and the docs now say so.

### D3 — Composite `TypeId`s silently produced an invalid type (Medium)

`duckdb.h` on `duckdb_create_logical_type`:

> Returns an invalid logical type, if type is: `DUCKDB_TYPE_INVALID`,
> `DUCKDB_TYPE_DECIMAL`, `DUCKDB_TYPE_ENUM`, `DUCKDB_TYPE_LIST`,
> `DUCKDB_TYPE_STRUCT`, `DUCKDB_TYPE_MAP`, `DUCKDB_TYPE_ARRAY`, or
> `DUCKDB_TYPE_UNION`.

"Invalid" there means a **non-null handle** wrapping `LogicalTypeId::INVALID`, so
`LogicalType::new`'s null check never fired. `.param(TypeId::Struct)` therefore
built one, and the failure surfaced much later as
`duckdb_register_scalar_function failed for 'f'` — naming neither the offending
parameter nor the fix. `LogicalType::get_type_id()` on such a handle panicked
with `unknown DUCKDB_TYPE value`.

`TypeId::is_composite` / `composite_constructor_hint` now name the seven cases
and the constructor to use. `LogicalType::new` asserts, `try_new` returns an
error, and every builder validates **before allocating any DuckDB handle**, so a
rejected type cannot leak a half-built function. The message says which slot,
which type, and what to call instead.

> A probe written to check whether registration succeeded silently found that it
> does not — DuckDB rejects the function. The defect is diagnosability, not
> corruption, and this section says so rather than the more dramatic thing.

### D4 — `extra_info` leaked when a builder was not registered (Medium)

DuckDB takes ownership of an `extra_info` allocation at
`duckdb_*_set_extra_info`, inside `register`. Until then it belongs to the
builder, and a dropped builder dropped the pointer on the floor. Easy to dismiss
as a path nobody takes — until a typed constructor allocates on the user's
behalf: `TableFunctionBuilder::with_state` boxes two closures, so `build()`
without `register()` leaked user data through an API that never mentions a
pointer. Miri's leak checker found exactly that in the crate's own tests.

`ExtraInfo` now owns the allocation and frees it on drop unless `register` marked
it transferred; all five builders go through it.

### D5 — Stale-borrow UB in `src/secrets.rs`'s tests (Medium, test-only)

Two tests took a pointer into a `String`, called a `&mut` method
(`zeroize_string`), and then read through the stale pointer. Under Stacked
Borrows the `&mut` retag pops the earlier tag, so the read is UB and the compiler
may reorder around it — meaning the tests did not reliably prove the zeroization
they exist to prove. One of them carried a comment explaining why it was fine.

Both now derive the pointer after the mutation. The library's `zeroize_string`
itself was correct throughout: it uses `ptr::write_volatile`.

### D6 — `InMemoryDb` leaked its dispatch table (Low)

`Box::into_raw(Box::new(api))`, deliberately leaked, on the belief that the
struct had to outlive the process. It does not: libduckdb-sys's
`duckdb_rs_extension_api_init` copies every function pointer into its own atomics
and keeps no reference. It is now a stack local. This mattered because it was the
only allocation the new LeakSanitizer job had to suppress, and a leak detector
that starts with a suppression for your own code is one bad day away from hiding
a real missing destructor.

### D7 — `duckdb_create_struct_value` has no count argument (Medium, latent)

It reads `values[0 .. StructType::GetChildCount(type)]` — the count comes from
the *type*, not from the caller. A short slice reads past its end. quack-rs's
`Value::struct_value` checks the two agree before calling, and says so in the
error.

### D8 — `duckdb.h` contradicts itself on `duckdb_create_list_value` (upstream doc bug)

The prose says "Creates a list value from a **child (element) type**"; the
`@param` line on the very next line says "@param type The type of the list". The
implementation settles it: both `duckdb_create_list_value` and
`duckdb_create_array_value` forward to
`Value::LIST(const LogicalType &child_type, …)` / `Value::ARRAY(const LogicalType
&child_type, …)`, which take the **element** type — and `Value::ARRAY` derives
the array type as `ARRAY(child_type, values.size())`, so the array's size comes
from the value count too.

Passing the container type returns a bare `nullptr` with no explanation. This
review's first draft of the wrapper got it wrong in exactly that direction, and
the test caught it. quack-rs names the parameter `element_type`, documents the
contradiction, and reports the container-type mistake by name.

Worth reporting upstream.

### D9 — The reference example disabled every panic guard in the crate (High)

`examples/hello-ext/Cargo.toml` shipped with:

```toml
[profile.release]
panic = "abort"
```

quack-rs's own `validate_release_profile` **rejects** that outright, and says
why: "std::panic::catch_unwind cannot catch anything under panic = abort — the
process aborts instead, taking the user's DuckDB session with it". The scaffold
generates `panic = "unwind"` with a comment explaining the same thing. quack-rs's
own `[profile.release]` sets `unwind` with a comment saying it "should not
contradict the crate's own guidance".

The example CI builds, loads into a real DuckDB, and holds up as the way to do
this had it the other way round — so in the one artefact a new user copies,
`scalar_callback!`'s guard, `init_extension`'s guard and every destructor fix in
D1 were inert.

Nothing caught it because nothing had ever pointed the validator at the crate's
own files. Two tests now do: one holds `examples/hello-ext/Cargo.toml` to
`validate_release_profile`, the other holds the profile `generate_scaffold`
writes to the same check, so the generator and the validator cannot drift apart
either.

### D10 — The `ScaffoldConfig` example in the README and the book did not compile

`ScaffoldConfig` gained `target_duckdb_version`, `use_unstable_c_api` and
`git_ref`; the exhaustive struct literal in `README.md`,
`book/src/publishing.md` and `book/src/getting-started/scaffold.md` was never
updated. Rustdoc examples are compiled by `cargo test` and the one in
`src/scaffold/mod.rs` was correct — Markdown code fences are not, so the copy a
new user actually reaches for was the broken one.

All three now use `..ScaffoldConfig::default()`, which is also what keeps them
correct the next time the struct grows.

> Worth considering for the next breaking release: `#[non_exhaustive]` on
> `ScaffoldConfig` would make adding a field a non-breaking change permanently,
> at the cost of one breaking change now. `cargo-semver-checks` (added here)
> would surface it as exactly that.

### D11 — Registration failures named nothing actionable (Low)

`duckdb_register_*_function` reports failure as a bare `DuckDBError` with no
message attached. quack-rs's error said only "duckdb_register_scalar_function
failed for 'x'". There are exactly three causes, and this review hit the least
obvious one while writing a test: `list_sum` and `array_sum` are DuckDB
built-ins, and a name collision looks identical to a type error. The message now
names all three and points at
`SELECT * FROM duckdb_functions() WHERE function_name = '<name>'`.

---

## 3. Gaps closed

### API coverage

Measured against the 546-entry `duckdb_ext_api_v1` of DuckDB v1.5.2–v1.5.5:

| Region | Before | After |
|---|---|---|
| Stable prefix (357 slots, frozen since v1.2.0) | 270 referenced | **316** |
| Unstable tail (189 slots, `duckdb-1-5`) | 106 referenced | **130** |

The 70 newly wrapped entries are the ones an extension actually reaches for:

- **`Value`** went from 9 constructors to the full set — every scalar width, the
  temporal family, `INTERVAL`, `BLOB`, `DECIMAL`, and the composites (`STRUCT`,
  `LIST`, `ARRAY`, `ENUM`; `MAP` and `UNION` behind `duckdb-1-5`). Plus
  `is_sql_null`, which is a different question from `is_null` — the latter asks
  about the *handle* — and `as_enum_index`.
- **`PreparedStatement`** went from 6 typed binds to 24, including `bind_value`,
  which is the escape hatch for every composite and for anything a future DuckDB
  adds.
- **Cancellation.** An extension that ran its own SQL had no way to stop it.
  `OwnedConnection::interrupt_handle` hands out a `Send + Sync` token whose
  lifetime is tied to the connection, so it can be moved into a watchdog thread
  without racing `duckdb_disconnect`. The `unsafe impl`s are justified against
  DuckDB's source, not by assumption: `ClientContext::interrupted` is an
  `atomic<bool>` and every `QueryProgress` field is an atomic.
- **Streaming results.** `PreparedStatement::execute_streaming` avoids
  materialising a large result before its first row is readable.
- **Result introspection.** `column_logical_type` keeps the nested structure that
  `column_type` collapses; `result_kind` separates rows from row counts.
- **Custom types.** `LogicalType::register` — `CREATE TYPE` from the C API. It is
  in the *stable* prefix and was unwrapped, so an extension could build an `ENUM`
  but not name it.
- **`SelectionVector` is now usable.** Its own docs said to "hand it to the
  relevant DuckDB vector operations", and quack-rs wrapped none of them.
  `vector::ops` adds `copy_selected`, `slice`, `reference_value`,
  `reference_vector` and `OwnedVector` — leading with the fact that `slice`
  produces a *dictionary* vector, after which every reader in this crate silently
  reads the wrong rows.

### Typed scalar bind data and local state

`ScalarBindInfo::set_bind_data` and `ScalarInitInfo::set_state` take a raw
pointer and a `duckdb_delete_callback_t`, so every extension using scalar bind
data wrote its own `unsafe extern "C" fn drop_bind` around `Box::from_raw` —
D1's hazard, relocated into user code. `ScalarBindData<T>` and
`ScalarLocalState<T>` generate it instead, panic-safe, mirroring
`FfiState`/`FfiBindData` for aggregates and table functions. A test registers a
bind-data type whose `Drop` panics and shows the query completing.

### Ergonomics: scalar functions as closures

Scalar functions are the most common extension function and had the least
ergonomic API — `ScalarFunctionBuilder::function` takes an `unsafe extern "C"
fn`, so every one started with raw pointers, manual offsets, and the NULL
contract from D2. Table functions already had `TypedTableFunctionBuilder`; this
is the equivalent:

```rust
ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?
ScalarFunctionBuilder::map2("add", |a: i64, b: i64| a + b)?
ScalarFunctionBuilder::map1_str("shout", |s: &str| s.to_uppercase())?
ScalarFunctionBuilder::map1_opt("or_zero", |x: Option<i64>| Some(x.unwrap_or(0)))?
```

Types come from the closure's signature, NULLs propagate correctly by
construction, and a panic becomes a SQL error. `VARCHAR` needs its own
constructors because the argument borrows from the chunk, which a plain type
parameter cannot express — the closure sees a `&str` into the vector, with no
per-row allocation. Cost is one indirect call per *chunk*: the closure is
monomorphised into a per-chunk executor at build time and only that is reached
through `dyn`.

### Verification

The suite was strong on "does it give the right answer" — 900+ tests, mutation
testing, an end-to-end load into a real DuckDB CLI, and four jobs that re-derive
checked-in tables from upstream. It had nothing that answers "is it sound" or
"does it leak", which for a crate whose job is raw pointers into someone else's
memory is the harder half.

| Job | What it buys | First run |
|---|---|---|
| `miri` | Pointer provenance, aliasing, initialisation, leaks, over the pure-Rust half | Found D5 and D4; now 546 tests green in ~3 min |
| `leak-check` | LeakSanitizer over the end-to-end suite against a real libduckdb — the only way a missing `duckdb_destroy_*` is visible | Found D6; now zero leaks across 58 end-to-end tests |
| `fuzz` | `cargo-fuzz` over the description.yml parser, the `duckdb_string_t` decoder and the validators | ~32M execs, no crashes |
| `semver` | `cargo-semver-checks` against the published crate — the API *is* the product | — |

Also closed two CI gaps:

- `tests/ffi_roundtrip.rs` is `#![cfg(feature = "_duckdb-testing")]`, so the
  plain clippy job compiled the repo's largest test file away to nothing and
  never linted it.
- `extension-load` pulled `releases/latest`, sampling a single point of the
  README's "supports DuckDB 1.4.x and 1.5.x" claim and silently retargeting
  whenever DuckDB shipped. It is now a matrix over v1.4.4, v1.5.0, v1.5.5 and
  `latest`. Verified locally first: the same stamped binary loads and answers
  correctly in all three pinned releases.

### Arrow interop and `COPY … FROM`

Both were listed as open items in section 5.1 of the first pass; they are closed
here. Fourteen more unstable-region entries, all verified against DuckDB v1.5.4's
`arrow-c.cpp`, `copy_function-c.cpp`, `table_function-c.cpp` and
`arrow_converter.cpp` rather than against the header alone.

**Arrow** (`src/arrow.rs`, new `duckdb-1-5-4` feature) wraps all eight
non-deprecated Arrow entries: the four conversions, the converted-schema
destructor, and the three `duckdb_arrow_options` calls. The remaining fourteen
Arrow entries are the deprecated `duckdb_query_arrow` result API, which is
inside `#ifndef DUCKDB_API_NO_DEPRECATED` — so the *live* Arrow surface is now
fully covered.

There is no `arrow` crate dependency. The interface is an ABI, and
`libduckdb-sys` defines the two `#[repr(C)]` records with arrow-rs's layout, so
bridging is a pointer cast. `ArrowArray::take_from` moves a record out of a
foreign wrapper and leaves a released placeholder behind, so exactly one side
ever calls `release`.

Three findings drove the design, none of them stated in `duckdb.h`:

1. `duckdb_data_chunk_from_arrow` sets `arrow_array->release = nullptr` **before**
   the conversion loop body, so it claims the array on the error path too. The
   wrapper takes the array **by value**; the by-value binding is dropped on the
   way out, which releases it in the one case DuckDB does *not* claim it — a
   zero-column converted schema, where the loop never runs.
2. `ArrowConverter::ToArrowSchema` and `ToArrowArray` install `release` / assign
   `*out_array` as their **last** statement, after everything that can throw. So
   a failed conversion leaves a record with `release == NULL` and nothing to
   free, and discarding it is correct rather than a leak.
3. `duckdb_data_chunk_from_arrow` indexes `arrow_array->children[i]` once per
   schema column with no bounds check, and dereferences an already-released
   array. Both segfault. The wrapper checks both first — which is why
   `ArrowConvertedSchema` records the `n_children` of the schema it was built
   from, the C API having no accessor for it. The module also documents the case
   it *cannot* check: a child whose buffers do not match its declared type,
   because `ColumnArrowToDuckDB` reads `array.buffers[1]` without testing
   `n_buffers` or the pointer. (Its sibling `GetValidityMask` is guarded; the
   crash is the data buffer, not the validity one. Confirmed by crashing a
   deliberately malformed array during this work.)

The `duckdb-1-5-4` feature's floor is set by the bindings, not by DuckDB: all
eight functions are in `duckdb_ext_api_v1` from **1.5.0** — checked against the
v1.5.0 `duckdb_extension.h`, not assumed — but `libduckdb-sys` declared
`ArrowSchema` / `ArrowArray` as opaque zero-sized bindgen placeholders until
**1.10504.0**. `src/arrow.rs` carries a `const` assertion naming that.

**`COPY … FROM`** closes the asymmetry in a shipped feature.
`CopyFunctionBuilder::copy_from` attaches a quack-rs table function as a
format's reader; `TableFunctionBuilder::build_handle` exists because such a
reader is *attached* rather than registered, and `register` is now that plus
`duckdb_register_table_function`. `BindInfo::result_column_count` /
`result_column_name` / `result_column_type` read the target table's schema,
which `COPY … FROM` fixes before the bind callback runs.

Two findings here as well:

1. `duckdb_register_copy_function` computes `is_copy_to` from
   `info.sink != nullptr` and `is_copy_from` from `copy_from_bind != nullptr`,
   **independently**, and accepts either. quack-rs required bind + sink +
   finalize unconditionally, which made a read-only format impossible. Fixed:
   the trio is required only when any of it is set.
2. `duckdb.h` states "the table function must take a single VARCHAR parameter
   (the file path)" and nothing enforces it —
   `duckdb_copy_function_set_copy_from_function` only rejects `INVALID` types,
   and `CCopyFromBind` builds the argument vector itself without consulting
   `tf.arguments`. A mismatch therefore surfaces much later, inside the reader's
   own bind callback, reading an argument that is not what it asked for.
   `copy_from` turns it into a registration-time error.

One user-visible quirk is documented rather than worked around: a bad `COPY …
FROM` option is reported as `'X' is not a supported option for copy function
'NAME'` where `NAME` is the *table function's* name, because `CCopyFromBind`
reads `info.tf.name` and `set_copy_from_function` only substitutes the copy
function's name when the table function has none. The docs say to name the two
alike.

Verified end to end against a real DuckDB 1.5.4: eleven Arrow tests (including a
full chunk → Arrow → chunk round trip with NULLs, and a zero-row chunk) and
seven `COPY … FROM` tests (a read-only format loading a table, a COPY option
arriving as a named parameter, and every rejection path), all under Miri and
LeakSanitizer.

---

---

## 4. Verified correct — checked, and no change needed

Recording these matters as much as the defects: each was a plausible failure
mode that turned out not to be one, and re-checking them later is wasted effort
if nobody wrote down that they were checked.

- **Flat-vector reads are sound everywhere DuckDB hands an extension a vector.**
  quack-rs's readers index the data buffer directly, which is wrong for
  constant and dictionary vectors. It never sees one:
  `CAPIScalarFunction` calls `input.Flatten()`, `CAPIAggregateUpdate` calls
  `inputs[c].Flatten(count)` and `state.Flatten(count)`, the cast bridge calls
  `input.Flatten(count)`, and the copy sink calls `input.Flatten()`. Verified in
  the v1.5.4 sources. `vector::ops::slice` is now the one way an extension can
  produce a non-flat vector for itself, and it says so loudly.
- **The ABI layout table is right.** `STABLE_API_SLOT_COUNT = 357` and every row
  of `KNOWN_LAYOUTS` (408 / 428 / 459 / 545 / 546) re-derived independently from
  the release headers and matched exactly, including v1.5.5.
- **`abi.rs`'s account of the loader is right.** `GetAPI` for `C_STRUCT` parses
  the semver, checks `IsSupportedCAPIVersion`, and then returns the **whole**
  struct including the unstable tail; `C_STRUCT_UNSTABLE` skips the check
  entirely because the footer is validated against the exact engine version
  earlier. Confirmed in `extension_load.cpp`.
- **`ListBuilder` handles the reallocation hazard.** `duckdb_list_vector_reserve`
  takes a total capacity and reallocates the child buffer when it grows; the
  builder re-fetches the child writer after every reserve, and refuses to exceed
  `MAX_VECTOR_SIZE` rather than let DuckDB throw a C++ exception into Rust.
- **The appender, the virtual file system and the string decoder** are careful in
  the places that matter: short reads and writes are looped
  (`FileHandle::read_exact` / `write_all`), `duckdb_append_varchar_length`'s
  silent 32-bit narrowing is refused rather than truncated, and the 16-byte
  `duckdb_string_t` is decoded with explicit little-endian reads that work
  regardless of pointer width.
- **Every truncating cast is deliberate.** The crate is clippy-pedantic-clean
  with `-D warnings`, so each one carries an explicit `#[allow]`; all of them are
  the `i128 → {u64, i64}` hugeint split or the documented DECIMAL width
  dispatch. Every `usize → idx_t` is widening on both 64-bit and wasm32.
- **Nested-type reads are correct against a real DuckDB.** A `LIST` row is a
  `{offset, length}` into one flat child vector shared by the whole chunk, and
  those offsets are cumulative, not `row * length`; `MAP` is two parallel child
  vectors behind one offset table; `ARRAY` has a fixed stride and no offset
  table at all. New end-to-end tests read variable-length lists (including an
  empty one, one containing NULL elements, and a NULL list), a map miss, and an
  array column across rows — all through scalar functions, where a mock cannot
  catch a wrong offset. All correct.
- **`TypeId::from_duckdb_type`'s panic is documented and has a `try_` sibling**,
  and nothing inside the crate reaches it from a callback path that lacks a
  `catch_unwind`.

---

## 5. Open items

Ordered by what a production extension is most likely to want.

### 5.1 API surface still unwrapped (100 of 546 entries)

| Group | Count | Assessment |
|---|---|---|
| Deprecated per-value result accessors (`duckdb_value_int32`, `duckdb_row_count`, `duckdb_column_data`, …) | 24 | **Correctly skipped.** `duckdb.h` marks the whole group deprecated and says to use `duckdb_fetch_chunk`, which is what `QueryResult` does. |
| Deprecated Arrow result API (`duckdb_query_arrow`, `duckdb_arrow_scan`, `duckdb_destroy_arrow`, …) | 14 | **Correctly skipped.** All fourteen sit inside `#ifndef DUCKDB_API_NO_DEPRECATED`. The eight non-deprecated Arrow entries are wrapped — see section 3. |
| Task scheduler (`duckdb_execute_tasks`, `duckdb_create_task_state`, …) | 9 | Niche: for embedding DuckDB's scheduler, rarely what an extension wants. |
| Pending-result execution (`duckdb_pending_prepared`, `duckdb_pending_execute_task`, …) | 8 | Now partly redundant: `execute_streaming` + `InterruptHandle` cover the common "long query, cancellable, incremental" need. Worth revisiting for cooperative progress reporting. |
| Log storage (`duckdb_register_log_storage`, …) | 6 | New in 1.5 and genuinely useful — an extension can register a log sink. Good next addition. |
| Profiling (`duckdb_get_profiling_info`, …) | 5 | Useful for extensions that expose their own EXPLAIN-like output. |
| Extracted statements (`duckdb_extract_statements`, …) | 4 | Low value: `duckdb_query` already runs multi-statement scripts. |
| Prepared-statement result metadata (`duckdb_prepared_statement_column_*`, `duckdb_param_type`, …) | 6 | Lets an extension learn a query's shape without running it. |
| `duckdb_scalar_function_set_bind_data_copy` | 1 | Needed for scalar bind data under parallel execution. |
| Misc (`duckdb_get_table_names`, `duckdb_appender_create_query`, `duckdb_create_bit`/`get_bit`, `duckdb_get_bignum`, `duckdb_create_data_chunk`, …) | ~13 | Individually small. |

`COPY … FROM` and the table-function bind result-column accessors were rows in
this table; both are closed in section 3.

`duckdb_string_is_inlined` / `duckdb_string_t_length` / `duckdb_string_t_data`
are deliberately unused: quack-rs decodes the 16-byte layout itself, which avoids
three FFI calls per string. That is a defensible trade, but it does mean the
layout is hard-coded in Rust rather than delegated — noted so the choice is
visible.

### 5.2 wasm32 is compile-checked only

CI runs `cargo check --lib --target wasm32-unknown-emscripten`. Nothing executes.
The one place pointer width genuinely matters is the `duckdb_string_t` decoder,
where the pointer occupies bytes 8..12 of a 16-byte union and the high half is
padding; the code reads the slot as `u64` and truncates, which is correct, but
it has never been *run* on a 32-bit target. A DuckDB-Wasm smoke test would close
this; it is a real piece of work.

### 5.3 `AbiPolicy::Strict` refuses unknown engine versions — by design, with a cost

An extension built against a 546-slot layout refuses to load into any DuckDB
whose version is not in `KNOWN_LAYOUTS`. That is right when the layout actually
differs, and it will differ for 1.5.6 (548 slots). But it also means an extension
shipped today refuses a future patch release that happens to be
layout-compatible, until quack-rs cuts a release. The escape hatches
(`QUACK_RS_TARGET_DUCKDB_VERSION`, `AbiPolicy::AllowUnknownEngine`) exist and are
documented; the trade-off is sound, but it should be a conscious release-cadence
commitment rather than an implicit one.

### 5.4 Forward risk: DuckDB `main` re-versions the C extension API

This is the largest single item on the horizon, and it is not yet released.

DuckDB `main` (`v1.5.6`-dev) reworks `src/include/duckdb_extension.h`:

- `DUCKDB_EXTENSION_API_VERSION` goes from **1.2.0 to 1.5.6**. quack-rs's
  `DUCKDB_API_VERSION` constant, pitfall **P2**, the scaffold, and the
  `append_metadata` guidance all hard-code `v1.2.0`.
- The stable/unstable `#ifdef` split is replaced by per-entry version *bands*
  guarded by a new `DUCKDB_API_VERSION_AT_LEAST(major, minor, patch)` macro. An
  extension declares the API version it targets and the newer entries are
  compiled out, so the struct it sees matches what it asked for.
- The former unstable tail — everything from `duckdb_create_instance_cache`
  onward, slot 404 in the new numbering — sits behind
  `#if DUCKDB_API_VERSION_AT_LEAST(1, 5, 6)`. Two new entries
  (`duckdb_create_timestamp_tz_ns`, `duckdb_get_timestamp_tz_ns`) bring the total
  to 548.

If this lands as designed it *fixes* the problem `abi.rs` exists to guard
against, from 1.5.6 onward. Until then `abi.rs` is doing necessary work. When it
lands, quack-rs will need: a non-hard-coded API version, a layout model that
understands bands rather than slot counts, and a decision about which API version
the scaffold should target by default.

`scripts/check-abi-table.py` will flag the slot-count change; it will not flag
the *versioning* change, because that is not what it looks at. Worth watching
`duckdb/duckdb` releases directly.

### 5.5 Smaller notes

- `Value::is_null` (handle) versus `Value::is_sql_null` (value) is a real
  footgun that documentation now covers but naming does not. Renaming
  `is_null` → `is_handle_null` would be a breaking change; worth doing at the
  next major.
- `LogicalType` has `try_*` constructors for the name-bearing types but not for
  `decimal`, `array`, `list_from_logical` or `map_from_logical`, which can still
  only panic on a null return. `Value::decimal` (added here) does return a
  `Result`; the `LogicalType` side should match.

---

## 6. Bottom line

quack-rs was already unusually careful — RAII everywhere, an ABI guard most
bindings do not attempt, upstream-drift checks in CI, and a pitfall register
written from real failures. The defects found were not sloppiness; they were the
two or three places where a documented contract turns out not to be what the
implementation does, plus one class of hazard (`extern "C"` destructors running
user code) that the crate had already reasoned about for callbacks and had not
carried through to its own generated code.

After this pass: no known abort paths in generated FFI code, the NULL semantics
match what DuckDB does rather than what its setting is named, the leak-clean
claim is machine-checked against a real database, and the surface an extension
actually reaches for — values, parameters, cancellation, streaming, custom types,
selection vectors — is covered rather than adjacent to covered.

The honest remaining list is section 5. Arrow interop and `COPY … FROM` — the
two that section 5.1 called out as the ones that would most change what quack-rs
can claim — are closed in section 3, verified against DuckDB's own sources and
against a running database. What is left of 5.1 is either deprecated or niche.
The remaining item that would most change the claim is a wasm32 test that
actually runs (5.2); the one that will demand attention on someone else's
schedule is 5.4.
