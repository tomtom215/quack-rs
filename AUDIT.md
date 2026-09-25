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

> **Corrected September 2026 (third pass).** The last paragraph is false.
> `CAPIAggregateUpdate` (`aggregate_function-c.cpp:92-111`, DuckDB 1.5.5)
> flattens the inputs and passes every row, NULL rows included, under either
> null-handling setting. An aggregate's setting is read only by
> `BoundAggregateExpression::PropagatesNullValues`, which only the
> correlated-subquery decorrelator consults. VALIDATED end to end by
> `aggregate_update_receives_null_rows_under_either_null_handling`
> (`tests/ffi_roundtrip/lifecycle.rs`); recorded as Pitfall L12. See section 8.

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
- **`PreparedStatement`** went from 6 binds to 23: 16 more typed binds and `bind_value`,
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
| `miri` | Pointer provenance, aliasing, initialisation, leaks, over the pure-Rust half | Found D5 and D4. Ran with default features until 2026-09, which `cfg`'d out every `duckdb-1-5*` module — including `src/arrow.rs`, the largest block of pure-Rust `unsafe` in the crate. Now runs `--features duckdb-1-5-4`, so the claim in this row is true for the first time. |
| `leak-check` | LeakSanitizer over the end-to-end suite against a real libduckdb — the only way a missing `duckdb_destroy_*` is visible | Found D6; now zero leaks across 58 end-to-end tests |
| `asan` | AddressSanitizer over the same suite — out-of-bounds writes and use-after-free, the class behind the two heap-corruption defects in v0.16.0. Added 2026-09-20, informational until it had a green run on main; blocking since 2026-09 (section 7). | Green on main (84 tests) and on the 125-test suite locally, with no suppressions |
| `fuzz` | `cargo-fuzz` over the description.yml parser, the `duckdb_string_t` decoder and the validators | ~32M execs, no crashes |
| `semver` | `cargo-semver-checks` against the published crate — the API *is* the product | — |

Mutation testing was already here, and was the part that most needed reading
rather than trusting. Three separate defects meant the gate could not fail:

1. `--output mutants.out` writes to `mutants.out/mutants.out/`, so the report
   step counted nothing, divided by a total of zero, printed `100%`, and
   skipped its `exit 1` — seventeen lines after cargo-mutants' own summary said
   `229 missed, 238 caught`.
2. The incremental job selected changed files with the pathspec `src/**/*.rs`.
   Git's default wildmatch lets `*` cross `/`, so that requires at least one
   directory component and matches **no** top-level file: `value.rs`,
   `query.rs`, `arrow.rs` and every other module directly under `src/` were
   never scanned.
3. `mutants.toml` carried `exclude_re = ["^fmt::"]`, commented "Display/Debug
   impls". cargo-mutants matches that regex against the whole line it prints
   for a mutant, which begins with the file path, so it matched nothing and
   seventeen unkillable `Debug::fmt` mutants survived every sweep.

With all three fixed the gate reports honestly, and what it then found was a
real bug class — eight call sites open-coding 128-bit word arithmetic, each
with a live `>>`/`<<` mutant — plus a set of test gaps listed in the changelog.
`src/value.rs` and `src/query.rs` are now excluded for the same reason nine
sibling modules already were (every function is a `DuckDB` C call a `--lib` run
cannot reach), with their pure logic moved into `value/hugeint.rs`,
`value/defaults.rs` and `query/cstr.rs` so it stays in the gate rather than
being excluded along with the wrappers.

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
  constant and dictionary vectors. It never sees one for its *input data*:
  `CAPIScalarFunction` calls `input.Flatten()`, `CAPIAggregateUpdate` calls
  `inputs[c].Flatten(count)`, the cast bridge calls `input.Flatten(count)`, and
  the copy sink calls `input.Flatten()`. `vector::ops::slice` is now the one way
  an extension can produce a non-flat vector for itself, and it says so loudly.

  > **Corrected September 2026.** This bullet originally also said
  > `CAPIAggregateUpdate` calls `state.Flatten(count)`. It does not — only
  > combine and finalize flatten the state vector — and the window-constant and
  > sorted-aggregate executors pass a constant state vector with `count > 1`.
  > That is Pitfall L11: every C API aggregate reads out of bounds under
  > `agg(x) OVER ()` and `agg(x ORDER BY y)`. See section 7.
  >
  > **Corrected again in the fourth audit (section 9).** `slice` is not the
  > one way: `duckdb_data_chunk_from_arrow` returns dictionary and constant
  > vectors for dictionary-encoded and null-type Arrow columns (quack-rs now
  > flattens them), and the in-place `Flatten` in `CAPIAggregateUpdate` is
  > itself what breaks a streaming window over a destructor-less aggregate
  > (Pitfall L13).
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

  > **Corrected September 2026 (third pass).** Two parts of this were wrong.
  > A refused row got no list entry, and the refusal was sticky, so every later
  > row in the chunk kept the previous chunk's `{offset, length}` and came back
  > as a valid list the callback never wrote (VALIDATED: a whole 2048-row chunk
  > of stale lists). And staying under the ceiling does not prevent an abort:
  > `duckdb_list_vector_reserve` has no try/catch, so an allocation failure
  > below it unwinds too (VALIDATED: 2^36 `BIGINT` elements). Refused rows are
  > now NULL, and `ListBuilder::with_element_limit` bounds input-driven lengths.
- **Correction (third pass): the appender was not the only 32-bit narrowing.**
  The next bullet is right about `duckdb_append_varchar_length`, but
  `VectorWriter::write_varchar` / `write_blob` reached the same narrowing in
  `StringVector::AddStringOrBlob` unguarded: a 4 GiB + 1 byte value was stored
  as a one-byte string (VALIDATED). They now refuse it (`try_write_varchar`,
  `vector::string::MAX_STRING_LEN`).
- **The appender, the virtual file system and the string decoder** are careful in
  the places that matter: short reads and writes are looped
  (`FileHandle::read_exact` / `write_all`), `duckdb_append_varchar_length`'s
  silent 32-bit narrowing is refused rather than truncated, and the 16-byte
  `duckdb_string_t` is decoded with explicit little-endian reads that work
  regardless of pointer width.

  > **Corrected in the fifth audit (section 10).** Little-endian reads were
  > the defect: `duckdb_string_t` holds its length and pointer in the target's
  > byte order, so on a big-endian target the decoder misread every string and
  > followed inlined bytes as a pointer (reproduced under Miri on s390x). It
  > now reads both natively, the pointer at the target's width.
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

### 5.1 API surface still unwrapped (99 of 546 entries)

> Re-measured 2026-09-20 against the v1.5.4 `duckdb_ext_api_v1` struct (546
> function pointers; byte-identical in v1.5.5). 99 are unreferenced in `src/`,
> of which 45 are in the header's deprecated block. The group counts below sum
> to fewer than 99 because the Misc row is approximate.

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
| ~~`duckdb_scalar_function_set_bind_data_copy`~~ | ~~1~~ | **Closed 2026-09-20** — `ScalarBindInfo::set_bind_data_copy`. This was not merely a gap: `CScalarFunctionBindData::Copy()` in DuckDB's `src/main/capi/scalar_function-c.cpp` leaves the copy's `bind_data` **null** when no copy callback is registered, so every extension using `ScalarBindInfo::set_bind_data` could see null bind data on a copied expression — a silent wrong answer, not a crash. |
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
differs. But it also means an extension shipped today refuses a future patch
release that happens to be layout-compatible, until quack-rs cuts a release —
and 1.5.6 looks like one: the `v1.5-variegata` branch head, fetched 2026-09-24,
compiles to 546 slots, the v1.5.5 layout (`sizeof(duckdb_ext_api_v1)` with the
unstable region enabled). An earlier version of this section said 1.5.6 would
have 548 slots; that is the `main` header (5.4), not the 1.5 branch. The escape hatches
(`QUACK_RS_TARGET_DUCKDB_VERSION`, `AbiPolicy::AllowUnknownEngine`) exist and are
documented; the trade-off is sound, but it should be a conscious release-cadence
commitment rather than an implicit one.

### 5.4 Forward risk: DuckDB `main` re-versions the C extension API

This is the largest single item on the horizon, and it is not yet released.

DuckDB `main`, and the `v2.0-cyanoptera` branch whose `duckdb_extension.h` is
byte-identical to it (both fetched 2026-09-24), rework
`src/include/duckdb_extension.h`:

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

- The loader routes by ABI type (`UsesCAPIV2` in
  `src/main/extension/extension_load.cpp`): **every `C_STRUCT_UNSTABLE`
  binary**, and every `C_STRUCT` binary declaring a C API v2.x.y, must export
  `<name>_init_c_api_v2` from `duckdb_extension_v2.h`. A V1 extension instead
  pins C API v1.5.6, into which that file says everything unstable in V1 was
  stabilised. quack-rs has no `_init_c_api_v2` entry point (its
  `entry_point_v2!` is the `Connection`-facade macro, unrelated), and
  `append_metadata` refuses `C_STRUCT` above `v1.2.0` — so a `duckdb-1-5`
  build stamped `C_STRUCT_UNSTABLE`, as recommended today, will fail to `LOAD`
  in a DuckDB built from that code, with DuckDB's own "did not contain function
  `<name>_init_c_api_v2`" error. Not changed yet: no released DuckDB routes this
  way (v1.5.5 is the newest tag, and the 1.5 branch has no `UsesCAPIV2`), so
  neither a V2 entry point nor a v1.5.6 pin can be tested against one.

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

---

## 7. Second audit, September 2026 (v0.17.0 → 0.18.0)

Reviewed at `6455238` (v0.17.0 plus #121/#122) against **DuckDB 1.5.5**
(prebuilt `libduckdb`, x86-64 Linux, `libduckdb-sys 1.10505.0`), with the DuckDB
v1.5.5 source tree as the reference and the 1.4.4 and 1.5.0 CLIs and libraries
for cross-version checks.

### 7.1 Method

Six independent audits — vector/chunk memory, scalar/aggregate/callback
lifecycle, table/cast/copy/macro/catalog, values/queries/Arrow/types, tooling
(ABI guard, `append_metadata`, validators, scaffold, CI, MSRV) and documentation
— each required to cite the quack-rs line, the DuckDB source line that fixes the
contract, and a probe against a real DuckDB wherever one could be written.
Probes ran against the linked `libduckdb`, under valgrind where ownership was the
question. Documentation was checked by extracting all 243 Rust blocks from the
README and book and compiling them, and every hello-ext SQL check was run on
three DuckDB releases.

Findings are labelled by how they were established:

- **VALIDATED** — reproduced by a program against a real DuckDB (or, for docs,
  a compiler) before the fix, and shown fixed after.
- **PROVEN** — derived from both sides' source; no program could trigger it.

### 7.2 A DuckDB defect every C API aggregate inherits (Pitfall L11)

`CAPIAggregateUpdate` flattens its input vectors but not the state vector; the
C API registers no `simple_update`; and `WindowConstantAggregatorLocalState` and
`SortedAggregateFunction` therefore call `update` with a **constant** state
vector and `count > 1`. Every aggregate registered through the C API reads
`states[1..]` out of bounds under `agg(x) OVER ()` (and other whole-partition
frames) and `agg(x ORDER BY y)`.

VALIDATED in plain C, with no quack-rs involved, against the official
`libduckdb` 1.4.4, 1.5.0 and 1.5.5: the control `SELECT capi_sum(i)` returns
4498500 and both shapes segfault; AddressSanitizer places the fault in the
callback, called from `CAPIAggregateUpdate` ←
`WindowConstantAggregatorLocalState::Sink` / `SortedAggregateFunction::Finalize`.
Section 4's first bullet said the opposite and has been corrected.

There is no extension-side fix: the callback cannot tell a constant vector from
a flat one, and probing `states[1]` is itself the out-of-bounds read. The one-line
upstream fix that suggests itself — `state.Flatten(count)` in
`CAPIAggregateUpdate` — is also wrong: `WindowConstantAggregatorLocalState::Sink`
caches `FlatVector::GetData(statep)` once and writes through it on every later
iteration, and an in-place `Flatten` replaces that buffer. Reported upstream,
with the C reproducer, as [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109) (2026-09-23);
the report also notes the defect is still present in `main`'s source at
`94d7b64`, where the file moved to `src/main/capi/v1/` and `simple_update`
became `cluster_update`.

### 7.3 Defects found and fixed

Safe-API soundness (safe code could cause UB or a data race):

| Defect | Evidence |
|---|---|
| `ScalarBindData<T>` / `ScalarLocalState<T>` had no `Send`/`Sync` bounds; DuckDB reads bind data from every executing thread | VALIDATED: one `Rc<Cell>` bind-data pointer used from 4 threads |
| Typed scalar constructors returned a mutable `ScalarFunctionBuilder`; `.returns(Integer)` on an `i64` closure wrote 8-byte values into a 4-byte vector | VALIDATED: truncated results; overflow past row 1024 |
| `ArrowOptions` outlived its connection (use-after-free in `data_chunk_to_arrow`); same class in `FileSystem` | VALIDATED under valgrind |
| `SelectionVector::new` returned uninitialised indices through safe `as_slice`, and a wrapped allocation size let safe code index out of bounds | VALIDATED: stale `0xDEADBEEF`; SIGSEGV in a release build |
| `decimal_to_f64` read past DuckDB's powers-of-ten tables for `scale > 38` | VALIDATED: `inf` / `NaN` at scale 40 / 60 |
| `FfiBindData` / `FfiInitData` / replacement-scan data lacked thread bounds for data DuckDB shares across threads | PROVEN |

Process aborts from ordinary input (a C++ exception unwinding into Rust):
`Value::as_*` on SQL `NULL` (e.g. `f(n := NULL)`), `Value::decimal` /
`bind_decimal` out of range, `date_to_days` on an invalid date,
`timestamp_from_micros` on infinities, `SelectionVector::new` above the
allocator limit, a config-option default that does not cast, catalog lookups for
four entry types, and a panic payload whose `Drop` panics (all callback kinds,
including the typed-table trampolines, which the first fix pass missed) — all
VALIDATED. The entry points' NULL-`duckdb_database*` dereference is PROVEN only:
nothing in a test can make DuckDB's `GetDatabase` fail.

Wrong answers with no error, all VALIDATED: scalar bind data lost on expression
copy (`sum` 90 instead of 132 under filter pushdown); NULL `STRUCT` rows keeping
valid fields; typed table functions failing on the second `EXECUTE` of a prepared
statement; typed projection pushdown returning the wrong column; `Value` getters
casting the value in place; `Value::decimal` / `bind_decimal` silently keeping
the low word; `ANY`-typed result columns silently dropped; duplicate overloads
registering and then failing every call.

Tooling, all VALIDATED: the mutation gate passed with a 100% score when
cargo-mutants had tested nothing (exit 4); `parse_description_yml` rejected or
misread 29 of the 346 published community `description.yml` files (now 346/346,
field-for-field against PyYAML); the scaffold accepted hyphenated names and
produced a project that cannot build; `append_metadata` took flags as values and
stamped C API versions DuckDB refuses; the unstable scaffold could declare a
DuckDB version its bindings were not built for, defeating the ABI guard; the
published crate's own integration test did not compile; and the book told users
`panic = "abort"` was required — the setting that disables every panic guard in
the crate.

Documentation: of 243 Rust blocks in the README and book, 140 failed to compile
as extracted. Most are deliberate fragments; the ones a user would copy — the
README quick start, the `running-sql` examples, `classify_extension_version`, a
README assertion that panicked — are fixed and were recompiled.

### 7.4 Verified correct

- The ABI slot counts and `KNOWN_LAYOUTS` (re-derived again from the headers);
  the ABI guard end to end (a 1.5.0-built binary refused by 1.5.5 and 1.4.4);
  `build.rs` under cross-compilation.
- The 512-byte footer is byte-identical to DuckDB's own tooling.
- Error strings passed to `duckdb_*_set_error` are copied by DuckDB.
- Aggregate `combine` / `finalize` ownership; segment-tree and `DISTINCT`
  windows; parallel `GROUP BY` over 4M rows and 50k groups.
- Leaks: every `Value` constructor and accessor, `LogicalType` accessors,
  appender, result and prepared-statement paths — 0 bytes lost under valgrind.
- `QueryResult` and `PreparedStatement` outliving their connection (they hold a
  `shared_ptr<ClientContext>`).
- Every integer / DECIMAL / temporal / interval / string layout in the vector
  readers and writers.
- MSRV 1.86.0; all 29 hello-ext SQL checks on DuckDB 1.4.4, 1.5.0 and 1.5.5.

### 7.5 Open items

1. **L11 upstream.** Filed as
   [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Until it is fixed, aggregate authors
   must tell their users to avoid the two query shapes; when it is, the
   limitation sections and Pitfall L11 should name the first fixed release.
2. **Config-option defaults.** The pre-check uses SQL `TRY_CAST`, which sees
   extension-registered casts; DuckDB's default-value cast does not. An extension
   that registers a more permissive `VARCHAR → T` cast *before* the option could
   still pass the check and abort. Documented on `register`.
3. **`Value` getters.** The copy-and-cast design is PROVEN not to abort for a
   cast that fails or succeeds non-NULL. A cast that *succeeds with a NULL*
   would still make DuckDB's `GetValue` throw; none was found across the 31
   value types the new test exercises against every getter, but that is testing,
   not proof.

   > **Corrected September 2026 (third pass).** "PROVEN not to abort for a
   > cast that fails" was false. Temporal casts bound through
   > `TemplatedCastLoop` throw instead of reporting through the error channel,
   > so 8 value-type/getter pairs aborted the process at both +infinity and
   > -infinity, 16 cases in all (VALIDATED, e.g. `TIMESTAMP -> as_time`,
   > `TIMESTAMP_MS -> as_date`; the DuckDB CLI's
   > `TRY_CAST('infinity'::TIMESTAMP AS TIME)` throws too). The proof missed
   > casts bound through `duckdb::Cast`, and the test used only benign values.
   > The getters now refuse those pairs first (`value::temporal_checks`). See
   > section 8.
4. ~~**The book is not compiled in CI.**~~ — fixed in the third pass:
   `book/doctest` compiles every Rust block of the book and README as a
   doctest in CI, and a `book` job builds the book and checks its links on
   every pull request (section 8).
5. ~~The book changelog's relative links~~ — fixed in this pass:
   `scripts/sync-book-changelog.py` rewrites repository-file links to GitHub
   URLs, since the mirror lives in `book/src/reference/`.
6. ~~AddressSanitizer is informational~~ — made blocking in this pass: green on
   `main` (84 tests) and locally over the merged 125-test suite, no suppressions.
7. `src/value.rs` (~1,100 lines at the time; split into `src/value/` in the
   third pass) and `src/aggregate/builder/set.rs` (~508) exceed
   the 500-line guideline.

### 7.6 How this pass was verified

- **Tests, against DuckDB 1.5.5:** 771 library tests (with `duckdb-1-5-4`), 125
  end-to-end tests in `tests/ffi_roundtrip`, 66 integration tests, 31
  `append_metadata` tests, 193 doctests; clippy `-D warnings` under default,
  `duckdb-1-5-4`, `bundled-test-prebuilt` and
  `bundled-test-prebuilt,duckdb-1-5-4`; rustdoc `-D warnings`; MSRV 1.86.0.
- **Sanitizers, locally with CI's exact flags:** Miri over the library (731
  tests at the time, exit 0, leak check on), LeakSanitizer and
  AddressSanitizer over the 125 end-to-end tests, all clean. The
  AddressSanitizer job was made blocking on that evidence and its green run on
  `main`.
- **hello-ext:** all 29 numbered README checks (31 statements) on DuckDB 1.4.4,
  1.5.0 and 1.5.5 — 31/31 on each; three of them now return different results
  without `LOAD`, so they cannot pass vacuously.
- **Mutation testing,** the exact invocation CI's incremental job would run on a
  PR from this branch (every changed `src` file, config exclusions re-applied,
  `--features duckdb-1-5-4 --lib`): 893 mutants, **0 missed**, 763 caught, 128
  unviable, and 2 timeouts — both in the SPDX expression parser's index
  cursor, which was then rewritten as a consuming slice cursor (34 mutants,
  34 caught).

**The mutation-testing configuration itself was broken** (found while doing
the above): cargo-mutants reads `.cargo/mutants.toml`, not the root
`mutants.toml`, so no job had ever read the exclusions, features or timeouts;
the file also held two keys cargo-mutants 27.1.0 rejects; and once it was
read, its `examine_globs` overrode `--file`. All three are fixed.

Two mutants are excluded with a written equivalence argument (in
`.cargo/mutants.toml`), and two were removed rather than killed —
`child_bad` became an `unsafe fn` (it reads an arbitrary raw handle through
FFI, so it should have been), and `Owned`'s hand-written `Drop` now delegates to
`LogicalType`'s. Their leak-shaped mutants are only observable with a live
engine and are covered by the end-to-end suite, not by mutation testing.

One process error is recorded so it is not repeated: the first mutation run
set `CARGO_TARGET_DIR`, which makes cargo-mutants' parallel jobs share one
target directory and test each other's binaries. It reported a mutant that
replaced `validate_extension_name` with `Ok(())` as surviving the tests that
assert it rejects hyphens — impossible, and how the problem was noticed. That
run was discarded.

---

## 8. Third audit, September 2026 (0.18.0, before release)

Reviewed on `claude/busy-lamport-19hxye`, starting from `main` at `82b96fc`
(the merged second audit), against **DuckDB 1.5.5** (prebuilt `libduckdb`,
x86-64 Linux, `libduckdb-sys 1.10505.0`) with the v1.5.5 source tree as the
reference, the 1.4.4 and 1.5.0 CLIs for cross-version checks, and DuckDB `main`
at `30c64e17` (2026-09-23, source only, not built) for the upstream checks.
Nothing from this pass has been published. 0.17.0 was prepared but never
tagged or published (crates.io's latest is 0.16.0); it has been folded into the
unreleased 0.18.0, where every entry below also lands.

### 8.1 Method

Six area audits again (vector/chunk memory, scalar/aggregate lifecycle,
table/cast/copy/macro/catalog, values/queries/Arrow, tooling, documentation),
each finding required to carry a probe against a real DuckDB or a derivation
from both sides' source, and each fix a regression test shown failing first.
Labels are as in section 7: **VALIDATED** (reproduced by a program, then shown
fixed) and **PROVEN** (derived from source; nothing could trigger it). A third
label is used here for contract fixes found by reading: **CONTRACT** — an
`unsafe fn`'s `# Safety` section omitted a requirement, so a caller following it
could still cause UB; fixed by stating the requirement, no runtime test possible.

This pass also corrected four claims earlier sections made as verified. Each
correction is inline where the claim was made, marked "Corrected September 2026
(third pass)": D2 (aggregates do receive NULL rows), two section 4 bullets
(`ListBuilder`; `VectorWriter`'s string narrowing), and section 7.5 item 3 (the
`Value` getters were not proven abort-free).

### 8.2 Defects found and fixed

Process aborts from ordinary input (a C++ exception unwinding into Rust):

| Defect | Evidence |
|---|---|
| `Value` temporal getters on ±infinity: 8 type/getter pairs, 16 cases (`TIMESTAMP -> as_time`, `TIMESTAMP_MS -> as_date`, …) | VALIDATED (exit 134); now refused first |
| `Value` temporal constructors accepted any `i64`; a later `as_str` / getter aborted (`time(i64::MAX)`, `timestamp_s(1e14)`, `timestamp(i64::MIN)`) | VALIDATED; constructors now return `Result` (**breaking**) |
| Catalog lookups of names DuckDB autoloads an extension for (e.g. type `inet`) | VALIDATED, and reproduced in plain C (8.5) |
| Config option: a default that `DefaultCastAs` cannot convert (incl. ICU-only `TIMESTAMPTZ` strings); `SET <option> = NULL` read back | VALIDATED (probes t09, t29) |
| `OwnedVector::new` capacities whose byte size wraps: a 32-byte buffer for `HUGEINT × (2^60 + 2)` | VALIDATED (16 of 16 neighbouring vectors corrupted) |
| `ListBuilder` below the 2^37 ceiling: `duckdb_list_vector_reserve` OOM | VALIDATED (2^36 `BIGINT`); `with_element_limit` added |
| `AbiPolicy::Warn`'s diagnostic could panic under the C entry point | PROVEN |

Wrong answers with no error:

| Defect | Evidence |
|---|---|
| A scalar whose signature already exists silently **replaced** it for every connection — a built-in included (`abs(BIGINT)`) | VALIDATED; now refused, 37 rendered types checked against `duckdb_functions()` |
| A table or copy function with a taken name was silently dropped | VALIDATED |
| `ListBuilder` refused rows kept the previous chunk's list entries | VALIDATED (a whole 2048-row chunk) |
| `VectorWriter::write_varchar` / `write_blob` over 4 GiB stored a truncated string | VALIDATED (strlen = 1) |
| `QueryResult::next_chunk` reported a failed stream as end-of-results | VALIDATED; now `Result<Option<_>>` (**breaking**) |
| `Appender`: a half-written row was committed on `close`, buffered rows lost | VALIDATED; the appender is poisoned instead |
| Typed table builder accepted projection pushdown it cannot honour | VALIDATED |
| A panicking `TRY_CAST` callback left rows holding stale values | VALIDATED; every row is nulled |
| `SqlMacro` bodies with `--` comments broke; `1); DROP TABLE …; SELECT (1` ran | VALIDATED |
| An error message with an interior NUL lost its tail in some paths and kept it in others; every path now replaces NUL with `?` (`ErrorData::new` was the last) | VALIDATED (e.g. `"bad\0input"` → `"bad"`) |
| A `CAST` callback returning `false` with no message: `Conversion Error: ` and nothing | VALIDATED |
| `FileHandle` outlived its `FileSystem`; `FileFlag::CreateNew` did not create | VALIDATED (valgrind "Invalid read"); `FileHandle<'fs>` (**breaking**). On Windows an existing file is still not refused (DuckDB ignores the exclusive flag there; VALIDATED by the release CI failure, documented) |

Security: `SecretEntry` freed replaced field, provider and scope values
without zeroizing them, and `Debug` printed the scope. VALIDATED by an
inspecting global allocator (`tests/secret_zeroize.rs`): 3 marked buffers freed
before the fix, 0 after, in debug and release.

Capability: `InstanceCache` is now `Send + Sync`. PROVEN from DuckDB's
`DBInstanceCache` locking (`db_instance_cache.cpp`), and exercised from four
threads.

Panics and contract gaps:

| Defect | Evidence |
|---|---|
| `varargs(TypeId::List)` panicked in the setter | VALIDATED; now an `Err` from `register` |
| `MapVector::set_size` did not require `size` ≤ entries written | CONTRACT |
| `read_duck_blob` did not require the vector to outlive an inline blob | CONTRACT |
| `FfiInitData` / `FfiLocalInitData` / `FfiBindData` getters did not require `T` to be the type `set` stored (`set::<u8>`, `get_mut::<[u64; 64]>` wrote 512 bytes through a 1-byte allocation) | CONTRACT |

Found by the final verification sweep and by CI, not by the area audits:

| Finding | Evidence |
|---|---|
| DuckDB does not destroy every aggregate state of a query whose `finalize` reports an error (ungrouped: 2 initialised, 1 destroyed; grouped: 4 and 2); anything a state owns leaks | VALIDATED by counting callbacks; documented, pinned by a test, drafted upstream |
| Two new tests leaked (an `FfiState` aggregate failing in `finalize`; Arrow records built with `Box::leak`), which would have failed CI's blocking LeakSanitizer job | VALIDATED: 280 bytes, then 16 bytes (6 of 6 runs); clean in 3 of 3 runs after |
| A new SPDX test (a million nested parentheses) cannot finish under Miri; the PyYAML test spawns a process Miri cannot | VALIDATED: over 30 minutes on one test; both now scaled or skipped under Miri only |
| The incremental mutation gate would have failed: 63 mutants survived in files this branch changed, 28 of them in code split out of `value.rs`, whose exclusion did not follow it | VALIDATED; now 0 missed (8.6). Survivors in pure logic are killed by new unit tests; FFI-bound ones are excluded with a reason, and 4 that had no end-to-end test now have one |
| This pass's own test pinned `time_from_micros(-1)` and three other out-of-range inputs as harmless arithmetic, which holds only for a release DuckDB: `Time::Convert` ends in `D_ASSERT(IsValidTime)`, so a DuckDB built with assertions aborted. CI's source-built jobs (`bundled-test` on three OSes, coverage) caught it; no local run had built DuckDB from source | VALIDATED (SIGABRT at `time.cpp:326`, locally and in CI); `time_from_micros` and `time_tz_from_bits` now return `None` outside `00:00:00`–`24:00:00` (**breaking**) |
| Two end-to-end tests could not run against a source-built DuckDB, which no local run had tried: the temporal sweep's SQL oracle ran `CAST(<infinite TIMESTAMP_MS> AS DATE)` (and `TIME`), on which DuckDB 1.5.5 itself fails `D_ASSERT(IsFinite)` in `Timestamp::FromEpochMs` (a release build answers with a Conversion Error), and the config-option test required ICU, which libduckdb-sys's `cc` build does not compile in | VALIDATED (gdb backtrace into `sql_cast`; the 1.5.5 CLI's error for all four casts). Neither is a quack-rs defect: the oracle pins the release answer (`None`) for exactly those pairs, and without ICU the zoned default is asserted to be refused rather than registered |

Tooling, all VALIDATED: the ABI table lacked DuckDB v1.4.5; `validate_spdx_license`
overflowed the stack on deep nesting and rejected `WITH` exceptions; `vX.Y.Z`
versions were misclassified; the scaffold wrote free text unquoted into YAML
and Rust, and generated a unit test and SQLLogicTest that tested nothing;
`append_metadata` rejected 32-byte fields and `--flag=value`; the test shim did
not check that its headers matched the bindings.

Documentation: every Rust block in the book and README is now compiled as a
doctest in CI (`book/doctest`); every relative and docs.rs link is checked
against the rendered book and rustdoc (`scripts/check-book-links.py`), which
found 8 broken links; all 31 hello-ext README statements run in CI
(`scripts/check-hello-ext.py`), which checks them against the fixture too; and
`docs/architecture.md`, stale in its module list and feature gates, is now held
to `src/lib.rs` by an integration test. The load recipes no longer tell readers
to set `allow_extensions_metadata_mismatch`, which the documented `C_STRUCT`
stamping never needs (VALIDATED on 1.4.4, 1.5.0 and 1.5.5).

Code health: the four files furthest over the 500-line guideline (`query.rs`,
`arrow.rs`, `appender.rs`, `testing/mock_vector.rs`) were split by pure moves.
The rustdoc page list is unchanged apart from four redirect stubs, and every
test count is identical before and after. Every `unsafe` block in library code
now carries a `// SAFETY:` note, and `clippy::undocumented_unsafe_blocks` keeps
it that way.

### 8.3 Verified correct

- `arrow::data_chunk_from_arrow` refuses a negative length or offset by name
  before DuckDB sees it (pinned by a unit test, shown red with the check
  disabled); DuckDB itself aborts on it (8.5).
- A user macro shadowing a built-in is documented on `SqlMacro` and in the
  book, and still behaves as documented (DuckDB 1.5.5 CLI: `abs(-1)` returns
  the macro's 42; `system.main.abs(-1)` returns 1).
- Identical scalar calls whose bind callbacks stored different data are merged
  (`CScalarFunctionBindData::Equals` compares only `extra_info` and the
  callback); documented on `ScalarBindData`, pinned end to end, and raised
  upstream as a feature request (8.5).

### 8.4 Not changed, deliberately

- 39 files under `src/` still exceed 500 lines, counting their unit tests;
  CONTRIBUTING's guideline says "should generally" and allows exceptions for
  cohesion. Only files with more than 800 non-test lines were split.
- `/dev/null` in the audit container was a regular file whose contents changed
  as processes wrote to it. Cargo's `rustc` probe reads it and cached the
  failure, which failed builds with "failed to run `rustc` to learn about
  target-specific information" and made cargo-mutants mark mutants unviable.
  Repairing the device was not permitted in that environment, so the final runs
  use a `RUSTC_WRAPPER` that gives `rustc` an empty stdin in its place; no
  result below depends on the corrupted file. This is an environment fault,
  not a repository one.
- `undocumented_unsafe_blocks` is allowed in test code (`cfg_attr(test)` in
  `src/lib.rs`, and the two integration test crates that need it): 632 blocks
  in `tests/` alone, plus those in `src`'s unit-test modules, whose invariants
  are the test's own setup.

### 8.5 Upstream

Four DuckDB C API functions let a C++ exception escape an `extern "C"`
function, each reproduced in plain C against the v1.5.5 library (exit 134) and
unchanged in `main`'s source at `30c64e17`: `duckdb_list_vector_reserve`,
`duckdb_catalog_get_entry` (failed autoload), `duckdb_config_option_set_default_value`
(`DefaultCastAs`) and `duckdb_data_chunk_from_arrow` (`NumericCast` and
`Initialize` before its `try`). A fifth report asks for a way to compare scalar
and aggregate bind data, and a sixth reports the aggregate states a failed
`finalize` leaves undestroyed. Drafted, not yet filed; DuckDB's tracker was not
searched for duplicates. The drafts, reproducers and a status table are in
`docs/upstream-duckdb-reports.md`.

Not drafted: in DuckDB 1.5.5, `CastTimestampMsToDate` and `CastTimestampMsToTime`
pass an infinite value to `Timestamp::FromEpochMs`, whose `D_ASSERT` aborts a
debug build from plain SQL (8.2). `main` at `30c64e17` casts through `TryCast`
instead and no longer calls `FromEpochMs` outside the infinity-guarded C API
converter (derived from source; not built).

### 8.6 How this pass was verified

All at `fcebab5` against DuckDB 1.5.5 unless stated, x86-64 Linux. `fcebab5` and
`3ddd9b8` (below) are commits of the branch that was squash-merged as `52dc2fd`,
so they are not in this repository's history and these figures cannot be
re-run at them. At `52dc2fd` itself the library has 761 tests with default
features and 916 with `bundled-test-prebuilt,duckdb-1-5-4` (counted in the
fourth audit, section 9), not 759 and 913:

- **Tests:** 759 library tests with default features; with
  `bundled-test-prebuilt,duckdb-1-5-4`, 913 library, 187 end-to-end
  (`tests/ffi_roundtrip`), 70 integration and 34 `append_metadata` tests;
  198 doctests; 239 book and README blocks compiled by `book/doctest`
  (4 ignored, each saying why).
- **Lints and docs:** `cargo fmt --check`; clippy `-D warnings` with default
  features, `duckdb-1-5`, `duckdb-1-5-3` and `bundled-test-prebuilt,duckdb-1-5-4`;
  rustdoc `-D warnings` with default features and with `duckdb-1-5-4`;
  `mdbook build`; `scripts/check-book-links.py`; the sitemap and changelog-mirror
  checks. MSRV `cargo +1.86.0 check`. Dependency floor (`duckdb` and
  `libduckdb-sys` pinned to 1.4.4): 759 library tests pass.
- **Sanitizers, with CI's exact flags:** Miri over the library, 841 passed,
  1 ignored (the subprocess test), no undefined behaviour; LeakSanitizer and
  AddressSanitizer over the 187 end-to-end tests, no reports.
- **hello-ext:** built from this tree, stamped `C_STRUCT` / `v1.2.0`, all 31
  README statements pass on DuckDB 1.4.4, 1.5.0 and 1.5.5; 2 of them
  (T25b1, T25b2) also pass without the extension, and T25 shows its cast is
  the one running.
- **Mutation testing,** the invocation CI's incremental job runs on this branch
  (116 changed `src` files, config exclusions re-applied, `--features
  duckdb-1-5-4 --lib`): 994 mutants, **0 missed**, 745 caught, 249 unviable,
  0 timeouts. No mutant log contains the `rustc` probe failure of 8.4.
  CI's own run of the same invocation (on `3ddd9b8`) missed one mutant the
  local run had caught: `days * MICROS_PER_DAY` -> `+` in
  `interval_to_micros_saturating`, which only a randomized proptest could
  reach. A deterministic test now kills it (shown failing with the mutation
  applied), and `src/interval.rs` under `PROPTEST_CASES=1` gives 28 mutants,
  27 caught, 1 unviable, 0 missed; the only other proptests in the library
  (`src/testing/harness.rs`) exercise the harness and their own test states.
- **DuckDB built from source, assertions on,** after the `time_from_micros`
  fix (8.2), with CI's two commands: `cargo test --all-targets --features
  bundled-test` passes 816 library, 34 `append_metadata`, 119 end-to-end,
  70 integration and 1 `secret_zeroize` test; `--all-features` (the coverage
  job) passes 915, 34, 187, 70 and 1. The unfixed code aborts the first of
  these at `time.cpp:326`, in the test named in 8.2. CI's mutation invocation
  on the two files that fix changed: 128 mutants, 124 caught, 4 unviable,
  **0 missed**.

## 9. Fourth audit, September 2026 (0.18.0, before release)

### 9.1 Method

Six areas were audited in parallel — vectors and Arrow; functions and
aggregates; tables, catalog and copy functions; values, queries and the
appender; tooling and CI; documentation — each finding re-run by hand before
it was accepted, then fixed with a regression test shown failing
without the fix (by reverting or disabling the fix and re-running). DuckDB
behaviour was reproduced in plain C against the prebuilt v1.4.4, v1.5.0 and
v1.5.5 libraries, and against v1.5.5 built from source with assertions and
ASan where a build-dependent answer mattered. Late in the pass the whole suite
was run against DuckDB 1.4.4 (default features) and 1.5.0 (`duckdb-1-5`), the
oldest release each build can load into; that found four more defects (9.2,
"Engine-specific") and is now a CI job.

Severity, assigned in this write-up by one rule: **Critical** — a process
abort or memory corruption reachable from SQL; **High** — the same not
reachable from SQL alone, or a silent wrong answer; **Medium** — a wrong
answer in a narrow case, or a failure with a misleading message; **Low** —
tooling, validation and documentation. Labels: VALIDATED (reproduced and
pinned by a test), PROVEN (derived from DuckDB's source, cited), CONTRACT (a
Safety or documentation obligation).

### 9.2 Defects found and fixed

**Critical**

- *Arrow import heap overflow* (VALIDATED). A dictionary array with NULLs and
  more than 2048 entries made `DuckDB` write past a validity mask
  (upstream item 9); refused before the call.
- *Arrow import out-of-bounds read* (VALIDATED). Dictionary and constant
  vectors came back from `duckdb_data_chunk_from_arrow` and were read as flat
  (upstream item 8); they are flattened on import.
- *Rendering an SQL-built timestamp aborted* (VALIDATED). `Value::as_str`,
  `display_string`, `Debug` on `make_timestamp(-9223372036854775808)` and the
  like, also nested (upstream item 13); every payload is checked first. The
  converting getters no longer pass such a payload to `DuckDB`'s cast.
- *Subquery argument at bind time aborted on 1.5.0–1.5.4* (VALIDATED against
  1.5.0; PROVEN for 1.5.1–1.5.4 from the source at each tag).
  `ScalarBindInfo::argument` asks for nothing before 1.5.5 (upstream item 12).
- *Out-of-range TIME crashed rendering* (VALIDATED). Bind and append refuse
  it; `VectorWriter::write_time` has a Safety clause (upstream item 14).
- *Literal types invalidated the database* (VALIDATED). Refused by
  `LogicalType::try_new` (upstream item 17).
- *Panics in scalar bind/init callbacks aborted* (VALIDATED). New
  `scalar_bind_callback!` / `scalar_init_callback!` macros.

**High**

- *Running window over a destructor-less aggregate* returned wrong values
  (VALIDATED; upstream item 7, Pitfall L13). Every aggregate registers a
  destructor.
- *Nonzero Arrow parent offset* imported the wrong rows (VALIDATED; upstream
  item 15). Refused.
- *`destroy` on never-initialised states* (VALIDATED; upstream item 10).
  `FfiState` carries an address-derived tag.
- *4 GiB and over* stored modulo 2^32 by `append_bytes`, `bind_str`,
  `bind_blob` (VALIDATED for append; bind source-traced). Refused.
- *Catalog entry handle lifetime* (VALIDATED). `CatalogEntry` owns its data.

**Medium**

- Scalar collision check: aliases, macro shadowing (`system.main.`
  qualification), varargs overlap (VALIDATED against `DuckDB`'s binder).
- `ListBuilder` over existing list entries; `set_valid` and nested children;
  secret scopes; `get_file_path` at a NUL; `interval_to_micros` intermediate
  overflow; the appender's false "poisoned" after a failed automatic flush;
  `LogicalType::register`'s message for `ANY`; `SecretEntry` spare capacity
  (all VALIDATED).
- Zero-size ARRAY / empty UNION built by release `DuckDB` only (VALIDATED on
  release and assertion builds; upstream item 18). Refused.
- `MockRegistrar` accepted builders `LOAD` refuses (VALIDATED); it runs the
  same checks.

**Engine-specific** (found by running the suite against 1.4.4 and 1.5.0)

- `duckdb_create_decimal_type` validates nothing before 1.5.4 (PROVEN from the
  source at each tag; VALIDATED on 1.4.4 and 1.5.0): `try_decimal` checks
  itself.
- Before 1.5.0 a scalar cannot take an existing name (no `ALTER_ON_CONFLICT`;
  PROVEN; VALIDATED on 1.4.4): refused first, with the reason.
- 1.4.x's C API reports `TIME_NS` as `INVALID` (VALIDATED): `try_new` refuses
  a type the engine hands back changed.
- The subquery-argument abort above.

These were present at `52dc2fd` (run against 1.4.4 there: four of the same
tests failed) and invisible to CI, which ran the end-to-end suite only against
the pinned 1.5.5. Pitfall L14.

**Low (tooling, validation, documentation)**

- CI's "refused extension registered nothing" checks could not fail (`-c`
  stops at the failed `LOAD`); fixed and run locally end to end.
- `AbiPolicy` branches untested: pure `policy_verdict`, full matrix test,
  shown to catch a mutated arm.
- Generated project: `lib.rs` not `rustfmt`-stable for names of 1–4 or 40–64
  characters (checked for all 64 lengths); Linux SQLLogicTest skipped by
  extension-ci-tools; `description.yml` fields `PyYAML` retypes (a matcher for
  its YAML 1.1 resolvers, checked against `PyYAML` 6.0.1 on 231,758 strings
  with 0 differences; 3 of the 346 published descriptors are warned about).
- `description.yml` reader kept the quotes of a value on the line after its
  key.
- `append_metadata`: platform groups, `wasm_*` without `--wasm`, empty-ABI
  footers. `validate_semver`: leading zeros (0 differences from the `semver`
  crate over 232,615 strings but one above `u64::MAX`). SPDX message and doc
  claim. Keyword names. Dev-engine ABI message and `build.rs` warning. Null
  `get_api`. `check-abi-table.py` fingerprints signatures, not names.
- A Stacked Borrows violation Miri found in one of this pass's own new
  `FfiState` unit tests (test code only): the tag is now written through the
  pointer the destructor uses, not the local.
- `description.yml`'s "text after a comment" error named the line the value
  started on, not the comment's (VALIDATED; the regression test fails on the
  old code with "line 2" for a comment on line 3).
- 72 mutants the library tests did not kill (9.6), all in code this pass
  added or changed. 55 were pure logic tested only end to end or not at all:
  the `FfiState` tag (`^` vs `|` was indistinguishable for two stack slots
  16 bytes apart), the collision check, `refuse_any_return`,
  `rendered_signature`, a set-level return type, `render_guard`'s type
  predicates, `is_plain` and the YAML 1.1 matcher (its 231,758-string
  comparison with `PyYAML` ran outside the suite). 53 now fail a unit test;
  the decimal and array-size checks moved into pure functions for it. The
  other two went with the code: `is_time_ns`'s cfg'd-out twin (no build
  that runs it was mutated) is one function, and `parse_value`'s arm for a
  plain value on the next line, identical to its recursion once the line
  fix above was in, is gone. Of the 17 that need a live `DuckDB`, run end to
  end, one survived: no test rendered a temporal `NULL`, which the mutant
  refused. One now does.
- Documentation corrections listed in 9.4.

### 9.3 Not changed, deliberately

- *Arrow export of large INTERVAL / UHUGEINT* (upstream item 16): documented,
  not checked at run time. A check covering nested children means walking
  every column; one covering only top-level columns would miss cases while
  implying it catches them.
- *ARRAY and UNION of a temporal type* cannot be rendered by `as_str` even in
  range: the C API cannot read their elements, so the payload cannot be
  checked, and the rendering is refused rather than risked.
- *Argument inspection on 1.5.0–1.5.4* is refused outright. Rust cannot catch
  the exception, and no C API call tells a copyable argument from one that is
  not. `get_argument` stays available, with the requirement in its Safety
  section.
- *DuckDB's `C_STRUCT_UNSTABLE` → V2 entry point* (5.4): not implemented; no
  released `DuckDB` has it, so nothing could test it.
- *`interval_to_micros`* returns totals `epoch_us` fails on (upstream item
  19); documented.

### 9.4 Corrections to earlier sections

- 4, flat-vector bullet: `slice` is not the only source of non-flat vectors
  (annotated in place).
- 5.3: 1.5.6 is not 548 slots; the 1.5 branch head is 546 (annotated in place).
- 5.4: `main` is 2.0 development, with the V2 entry-point routing (updated in
  place).
- 3 (API coverage): `PreparedStatement` went from 6 binds to 23, not 24.
- 7.5 item 7: `src/value.rs` has since been split.
- 8.6: `fcebab5` / `3ddd9b8` are pre-squash commits; the library counts at
  `52dc2fd` are 761 / 916.
- `CHANGELOG`: the "AUDIT.md does not cover" line, 47 (not 42) checkouts,
  16 (not 18) new typed binds, the ASAN job's status.

### 9.5 Upstream

`docs/upstream-duckdb-reports.md` items 7 to 19, each with a plain-C
reproducer and the output observed on the stated releases. None filed. Two
DuckDB changes found on the way are already fixed upstream and so are not
reports: the `try` in `duckdb_scalar_function_bind_get_argument` (1.5.5) and
the width/scale check in `duckdb_create_decimal_type` (1.5.4).

### 9.6 How this pass was verified

The code is the eight commits from `49c669a` to `409dfc4` on top of `52dc2fd`;
the commit after them adds this section. Local runs are x86-64 Linux against
the prebuilt DuckDB 1.5.5 unless stated.

- **Tests, at `409dfc4`:** with `bundled-test-prebuilt,duckdb-1-5-4`
  (`--all-targets`), 955 library, 37 `append_metadata`, 225 end-to-end
  (`tests/ffi_roundtrip`), 70 integration and 1 `secret_zeroize` test; 199
  doctests with `duckdb-1-5-4` (3 ignored); with default features, 796 library
  tests and 173 doctests (1 ignored).
- **Lints:** `cargo fmt --check`; clippy `-D warnings` on default features,
  `duckdb-1-5`, `duckdb-1-5-3`, `bundled-test-prebuilt`,
  `bundled-test-prebuilt,duckdb-1-5` and `bundled-test-prebuilt,duckdb-1-5-4`;
  beta clippy (1.99) on CI's two configurations. The first push (`49c669a`)
  failed CI on three clippy lints and one new beta lint: the local clippy run
  on record predated the last edits. `1278c56` and `44701bb` fix them.
- **CI:** all 47 jobs of the CI workflow passed at `409dfc4` (run 563), as
  they had at `963fd9e`; the three commits after it change only test code,
  `.cargo/mutants.toml` and this file. The 47 jobs include the end-to-end
  suite against DuckDB 1.4.4 (146 tests) and 1.5.0 (202), each with 70
  integration tests; DuckDB built from source with `bundled-test` on Linux,
  macOS and Windows; Miri over the library (882 passed, 1 ignored);
  LeakSanitizer and AddressSanitizer; hello-ext on three platforms and its
  load tests on 1.4.4, 1.5.0, 1.5.5 and the latest release; the scaffold
  end-to-end job; both ABI guards; MSRV; the dependency floor; the book;
  clippy on beta.
- **Mutation testing,** CI's incremental invocation (`--features duckdb-1-5-4
  --cargo-arg=--lib`, config exclusions re-applied, `--timeout 120 --jobs 4`,
  cargo-mutants 27.1.0). On the 80 changed `src` files of `49c669a` (run on
  its tree less one comment edit): 975 mutants, 732 caught, 171 unviable, 0
  timeouts, **72 missed** (9.2). After the fixes, on the ten files involved:
  356 mutants, 286 caught, 51 unviable, 19 missed: two date checks the new
  table did not yet reach (rows were added, and the test was shown to fail
  under each mutant) and the 17 that need a live `DuckDB`. Those 17 run
  against `tests/ffi_roundtrip` with libduckdb 1.5.5: 16 caught (10 by the
  abort the guard prevents, 6 by assertions), 1 missed and then killed by the
  `NULL` rendering test; `.cargo/mutants.toml` excludes them from the `--lib`
  gate with that evidence. Finally the whole incremental job replayed at
  `ac13445` (source identical at `409dfc4`), changed-file detection and
  `scripts/mutants-report.sh` included: 82 files, 1,032 mutants, 858 caught,
  174 unviable, **0 missed**, 0 timeouts, score 100%. No mutant log contains
  the `rustc` probe failure of 8.4.
- **`description.yml` reader:** the YAML 1.1 matcher agreed with `PyYAML`
  6.0.1 on 231,758 generated strings (0 differences); 123 values are now
  pinned in the suite. The reader's output for 169 generated documents and the
  346 published descriptors is byte-identical before and after the
  `parse_value` change (`6d0c062` vs `963fd9e`), and all 346 agree with
  `PyYAML`.
- **Earlier in the pass, on the fixes themselves:** each regression test was
  shown failing without its fix; hello-ext passed all 31 README statements on
  DuckDB 1.4.4, 1.5.0 and 1.5.5; the scaffold end-to-end job and both ABI
  guard jobs were run locally in full; `scripts/check-abi-table.py` over 18
  releases; the book built with mdBook 0.4.40, its links checked, 239 blocks
  compiled (4 ignored). Miri over the library found the test-code violation in
  9.2; after its fix the `aggregate::state` tests, the new adjacent-slot test
  included, pass under Miri.

---

## 10. Fifth audit, September 2026 (0.18.0, before release)

### 10.1 Method

A pre-release gate, run in parallel over four areas — soundness of the safe
API, callbacks and builders, the Arrow bridge, and every documentation claim —
with each finding re-run by hand before it was accepted. Every code fix has a
regression test shown failing without the fix (by reverting it, or by
running the test on the parent commit); where DuckDB was involved, the defect
was first reproduced against a real engine, in plain C where the defect is
DuckDB's. Claims with "all", "every", "never" or a count were re-derived by a
command. The whole suite ran against every release a build can load into
(1.4.4, 1.4.5, 1.5.0 to 1.5.5), and against v1.5.5 built from source with
assertions and AddressSanitizer.

Severity and labels as in 9.1: **Critical** — a process abort or memory
corruption reachable from SQL; **High** — the same not reachable from SQL
alone, or a silent wrong answer; **Medium** — a wrong answer in a narrow case,
or a failure with a misleading message; **Low** — tooling, validation and
documentation. VALIDATED (reproduced and pinned by a test), PROVEN (derived from
DuckDB's source, cited), CONTRACT (a Safety or documentation obligation).

### 10.2 Defects found and fixed

**Critical**

- *Rendering a value aborted* (VALIDATED). A `VARIANT` holding an out-of-range
  timestamp, a `DECIMAL(38, 0)` holding `i128::MIN` from `sum` over two
  in-range values, and a `GEOMETRY` from malformed WKB each made
  `duckdb_get_varchar` throw through the C API (upstream item 22). The render
  guard is now an allow-list.
- *`ListBuilder` aborted on a large list of a wide type* (VALIDATED). DuckDB's
  ceiling is 2^37 bytes per child buffer after rounding to a power of two, not
  2^37 elements (item 23). The limit is computed from the child type, and
  `with_element_limit` after the first row can no longer raise it.
- *`TableDescription` accessors aborted on index `u64::MAX` from 1.5.0*
  (VALIDATED on all eight releases; item 30). Refused before the call.
- *Arrow import heap overflows and out-of-bounds reads* (VALIDATED, under
  valgrind or with glibc heap-corruption aborts or SIGSEGV): dictionaries
  under lists, or under struct NULLs past 2048 rows (a fixed-size list's
  broadcast NULLs included), overlapping list views, `geoarrow.wkb` past
  2048 rows, and run-end arrays below zero-row nodes (items 25, 27 and 33;
  item 24's run-end case). Refused by the layout walk in
  `src/arrow/import_layout.rs`, which also refuses the layouts that import
  wrong values (High, below).
- *Two more Arrow-import crashes a differential fuzzer found after the pass's
  own fixes* (VALIDATED): a run-end-encoded child of a list `DuckDB` converts
  as zero rows when the list's offset is not 0 (SIGSEGV on 1.5.5 and in plain
  C on all eight releases), and a dictionary-encoded child of a fixed-size
  list that is a `MAP` value, where the map's validity check flattens the
  fixed-size list over its over-allocated capacity and reads the dictionary's
  selection vector out of bounds (SIGSEGV on 1.5.0-1.5.2, an AddressSanitizer
  `heap-buffer-overflow` on 1.5.5). Both are item 24; the walk now judges a
  zero-row list's child as `DuckDB` does (by row count, not offset) and
  refuses a dictionary fixed-size-list child under a map.
- *That walk let a run-end-encoded child of a zero-row list through when the
  list's offset was not 0* (VALIDATED: SIGSEGV on 1.5.5 through
  `data_chunk_from_arrow`, and on all eight releases in plain C; item 24's
  second site). DuckDB takes a list it converts as zero rows for empty
  whatever its offsets say and reads its child as a plain array; the walk
  judged emptiness by the offsets. Found by a fresh review after the fixes
  above; the walk now judges by the row count.

**High**

- *Arrow import returned wrong values from valid arrays* (VALIDATED on all
  eight releases): offsets below the top level, nested dictionaries, recoded
  union codes, and `null_count = -1` dictionaries and unions (items 24, 26,
  28, 31 and 32). Refused.

- *Moved aggregate states were never dropped* (VALIDATED): the fourth audit's
  address-derived tag failed for every state radix repartitioning moved. The
  tag now follows the slot's contents.
- *Abandoned grouped-aggregate states leaked a box each* (VALIDATED; item 20):
  a `T` of at most 256 bytes is stored inline in DuckDB's state bytes.
- *Arrow export returned values that differ, or an array that contradicts its
  schema* (VALIDATED): large INTERVAL / UHUGEINT / 39-digit HUGEINT (item 16,
  now refused at any depth) and BIGNUM / GEOMETRY under
  `arrow_output_version = '1.4'` before 1.5.5 (item 34; DuckDB's re-import
  crashed).
- *A typed table function answered from the wrong column under projection
  pushdown*, and *its callbacks could be replaced after `build()`*, reading one
  another's data as the wrong type (VALIDATED). Both refused.
- *Scalar bind/init callbacks shared the table callbacks' argument types*, so
  a table macro's output compiled on a scalar function and wrote past the
  scalar bind info (VALIDATED as compiling; the SIGSEGV is the crate's own
  earlier record). Distinct `RawScalarBindInfo` / `RawScalarInitInfo` types;
  `compile_fail` doctests pin it.
- *A typed `COPY … FROM` reader could invalidate the database* (VALIDATED on a
  1.5.5 assertion build; item 37). Refused at bind.
- *Arrow import: `+w:2x`-style fixed-size list formats bypassed the layout
  checks* (VALIDATED: SIGSEGV before). Upgraded from the reviewer's Low: the
  size is parsed as DuckDB's `std::stoi` parses it.
- *`VARCHAR` / `BLOB` decoding assumed little-endian* (VALIDATED under Miri
  with `--target s390x-unknown-linux-gnu`: undefined behaviour, a dangling
  pointer built from an inline string's bytes). `duckdb_string_t` holds its
  length and pointer in the target's byte order. On a big-endian target a
  string of 1 to 12 bytes read its length as `length << 24` and had its
  inlined bytes followed as a pointer, and a longer one read a byte-swapped
  length and pointer. That would be Critical, but it is limited to
  big-endian targets, and none of the 18 platform names in
  `validate::platform::DUCKDB_PLATFORMS` is one. Both fields are now read
  natively, the pointer as a pointer, which also clears Miri's one
  provenance warning (10.8). Found by reading the code behind that warning.
- *The Arrow export check read `HUGEINT` / `UHUGEINT` rows with their halves
  swapped on a big-endian target* (VALIDATED under Miri on s390x: the test
  reads 2^64 as 1). DuckDB stores `{lower, upper}`; the check read a native
  128-bit integer, so on s390x `10^38` passed into a lossy export and `2^63`
  was refused. It reads DuckDB's struct now.
- *On a 32-bit target, `ListBuilder`, `OwnedVector::new` and
  `data_chunk_from_arrow` could make DuckDB allocate a buffer whose size
  wraps* (PROVEN from DuckDB's source and computed; not run, as no wasm32
  DuckDB was available). DuckDB sizes buffers as a 64-bit `idx_t`, checks
  them only against 2^48 (`Allocator::AllocateData`), and hands them to
  `malloc`, which narrows them to a 32-bit `size_t`. The limits bounded
  element counts, not bytes: a `BIGINT` or `VARCHAR` list child could
  reserve 2^32 elements (`malloc(0)`), `OwnedVector::new(HUGEINT, 2^28)`
  succeeded, and a run-end-encoded Arrow column, which declares any length
  with a few bytes of buffers, made the chunk itself wrap, after which the
  import wrote past it. Critical on wasm32, where it is reachable from SQL
  when an extension imports Arrow data, but limited to 32-bit targets. The
  limits now bound bytes, `MAX_CAPACITY` is 2^28 - 1 there, and the Arrow
  layout walk refuses a row count above it at any node. On 64-bit that
  also refuses a length past 2^37, which threw through the C API before
  (item 4's case).

**Medium**

- A `row` closure panicking mid-row committed a half row (VALIDATED; the
  appender is poisoned). `FileHandle` drop could abort when the close threw,
  and `seek` past `i64::MAX` returned `Ok` on tmpfs (VALIDATED). `CombineFn`'s
  documentation advised a `combine` that gave 4985 of 5000 window rows wrong
  (VALIDATED; Pitfall L15). Catalog lookups outside DuckDB's own catalog type
  (PROVEN from `catalog.cpp`; no offline reproducer). `MockRegistrar` accepted
  builders whose types registration refuses (VALIDATED; messages compared with
  a live registration for ten cases). `entry_point!` aborted when an argument
  expression panicked (VALIDATED: SIGABRT). A callback macro body that used
  `?` dropped its error silently (VALIDATED as compiling; now a type error).
  Arrow offsets below the top level were not checked for sign and their sums
  could overflow (a debug-build panic, VALIDATED). `ArrowArray::release` could
  call a non-conforming producer's callback twice (VALIDATED).
- *A callback macro's body was an `unsafe` context* (VALIDATED: two
  `compile_fail` doctests compile against the old macros). The body was a
  closure inside the generated `unsafe extern "C" fn`, which it inherits, so
  a raw-pointer dereference in a "safe" callback compiled with no `unsafe`
  and, on edition 2021 (the scaffold's), no warning. It is a nested `fn` now.
- `ffi_state::<T>()` installs an aggregate's three state callbacks for one
  type; the raw setters remain, with the pairing a stated Safety obligation
  (CONTRACT). This mitigates, not eliminates, a mismatched pairing.

**Low (contracts, tooling, documentation)**

- Contracts widened to what DuckDB does (each measured or source-read):
  writers under a `LIST` child (a reserve moves every buffer below it,
  measured; Pitfall L18); which callback `FfiInitData` / `FfiLocalInitData::set`
  belong to; `FfiBindData::set` in a typed bind; a replacement scan's
  `delete_callback` runs with null data (observed); only column 0 of an
  imported Arrow chunk holds the producer's buffers (observed);
  `data_chunk_from_arrow` needs one element of dictionary padding (item 29);
  `DestroyFn` is not called for every state DuckDB creates (items 20, 35);
  `duckdb_prepare`'s allocation-failure path (item 36); why
  `config_option`'s missing `try` is not reachable; `OwnedConnection`'s `Send`
  reasoning.
- `append_metadata` named the wrong default platform on OpenHarmony.
- `StructWriter::field_mut` lets safe code swap two field writers, after
  which the `unsafe` `write_*` methods write to the wrong child: their Safety
  sections now require that no writer was replaced (CONTRACT).
- Mutation testing found untested behaviour (10.8), now pinned: `FfiState`'s
  tag structure, a release profile without `panic = "unwind"`, an overload's
  own return type, `ArrowSchema::dictionary`, validity under a list child
  starting past zero, every typed `Value` getter, the `TIMETZ` / `TIME_NS`
  range guards, four `Appender` methods, `StructWriter`'s child vectors,
  `InMemoryDb::execute`, `ResultKind::Nothing`, `ErrorData`,
  `MockVectorWriter::len`, seven `append_metadata` argument, version and
  footer checks, and every RAII handle's `Drop` (`tests/handle_leaks.rs`).
- Documentation claims corrected (10.4).
- `cargo doc --document-private-items` warned 8 times with `duckdb-1-5-4`
  (redundant link targets, from before this pass) and twice on default
  features (links to feature-gated items); 0 on all four feature sets now.

### 10.3 Not changed, deliberately

Decisions on 9.3's open items:

- *Arrow export of large INTERVAL / UHUGEINT*: superseded. This pass checks
  such values at any depth (10.2, High); the cost is one walk over the
  columns being exported.
- *ARRAY and UNION of a temporal type* still refuse `as_str`: the C API still
  cannot read their elements, and the render guard's allow-list keeps it so.
- *Argument inspection on 1.5.0–1.5.4* stays refused (item 12).
- *DuckDB's V2 entry point*: still not implemented; no released DuckDB has it.
- *`interval_to_micros` vs `epoch_us`* (item 19): stays documented.

New this pass:

- *EXCLUDE window frames* (item 35) and *abandoned grouped states* (item 20):
  a slot `init` receives again cannot be told from reused memory holding a
  stale copy of a moved state, so it cannot be dropped safely; small states
  leak nothing, large ones leak their box. Documented.
- *`duckdb_prepare` on allocation failure* (item 36): undefined behaviour no
  caller can detect. Documented.
- *Raw aggregate setters* kept (maintainer's choice); see 10.2.
- *Empty-list dictionary children* stay refused: DuckDB converts that child as
  a plain array, and the refusal is the conservative side.
- *`SelectionVector::new`* accepts up to 2^32 indices (16 GiB); an allocation
  the system cannot satisfy throws through the C API, as its documentation
  says. Real selections are vector-sized; the cap stays at `sel_t`'s range.
- *Open, Low, CONJECTURED*: the union export check reads unselected members;
  DuckDB nulls them, so a spurious refusal is the most it could cause.

### 10.4 Corrections to earlier sections

Each re-derived by a command, in `6a249a7` and the commits before it except
where a commit is named:

- The Arrow conversion functions are in DuckDB 1.4.4's C API, not added in
  1.5.0; the unstable region did not change in every recent release; it had
  middle insertions in two of the last four, not four.
- CHANGELOG: 36 CI jobs (not 35), 48 checkout steps (not 47); a test-gap
  list named a function removed before release.
- Platform counts: 84 of 346 published descriptors exclude
  `windows_amd64_rtools`, 3 exclude `windows_arm64_mingw` (community-extensions
  `5ae7df8`).
- The dispatch table has one slot per API field (546 with the pinned
  bindings), not 459; the unified `libduckdb-sys` feature is `bundled`.
- "Unit tests in every source file" (105 of 156), "every `unsafe fn` has
  `# Safety`" (four did not), "every unsafe block has a SAFETY comment" (two
  macro expansions did not), a header example no file uses, `Registrar`'s
  coverage, the `ExtraInfo` holder count (seven, not four), a nonexistent C
  function name, the macro-parameter validator, `destroy_callback`'s
  behaviour.
- The pitfall summary tables stopped at L14; they list all 30 now.
- 4, the appender/VFS/decoder bullet (`08311ce`): the string decoder's
  "explicit little-endian reads" were the defect in 10.2 (annotated in
  place). P7's
  layout, in `LESSONS.md`, the book and `docs/ffi-reference.md`, listed an
  `unused: u32` after the pointer, which only exists on 32-bit targets.
- README (`08311ce`): "three further audits (sections 7–9)", and "each reproduced
  against a real DuckDB or derived from its source", which the pure-Rust
  fixes were not.
- 9.6 recorded CI's doc build as clean; five intra-doc links added in this
  pass broke it on the default feature set until `ed5e381`.

### 10.5 Upstream

`docs/upstream-duckdb-reports.md` items 20 to 37, each with a plain-C
reproducer (item 32 reuses item 31's program) and the output observed on the
releases it names. None filed.

### 10.6 Exception escapes

Every C API call quack-rs makes where a C++ exception can leave the C
function, which aborts the host process ("Rust cannot catch foreign
exceptions"):

| C API call | Trigger | quack-rs | Evidence |
|---|---|---|---|
| `duckdb_list_vector_reserve` | capacity past `MAX_VECTOR_SIZE` or a child buffer past 2^37 bytes | `ListBuilder` caps both, from the child type; `with_element_limit` cannot raise the cap | items 1, 23; `list_limits.rs` |
| `duckdb_get_varchar`, `duckdb_value_to_string` | temporal, DECIMAL, VARIANT, GEOMETRY payloads SQL builds | render guard allow-list | items 13, 22; `value_render.rs`; 4.0 M fuzz runs |
| `duckdb_table_description_get_column_name` / `_type`, `duckdb_column_has_default` | index `idx_t(-1)`, from 1.5.0 | refused | item 30 |
| `duckdb_scalar_function_bind_get_argument` | an uncopyable argument, before 1.5.5 | refused before 1.5.5 | item 12 |
| `duckdb_catalog_get_entry` | a failed autoload; a catalog that is not DuckDB's own | autoloading names and non-`duckdb` catalogs refused | item 2; PROVEN |
| `duckdb_config_option_set_default_value` | a default that does not cast | converted by SQL first | item 3 |
| `duckdb_data_chunk_from_arrow` | chunk allocation before its `try` | negative lengths, and row counts above `MAX_CAPACITY` at any node, refused; an allocation below that the system cannot satisfy is documented | item 4 |
| `duckdb_destroy_file_handle` | a `Close()` that throws | closes through `duckdb_file_handle_close` first; leaks on failure | `file_errors.rs` |
| `duckdb_client_context_get_config_option` | a missing setting (debug builds); a throwing getter | documented; no getter throws for a stored state (source, 1.5.5) | P12; 150 / 157 settings read |
| any allocating call | allocation failure | documented; `duckdb_prepare` leaves a freed statement (item 36) | known limitations |

### 10.7 Versions

| DuckDB | Version feature (with `bundled-test-prebuilt`) | `cargo test --all-targets` at `42f894c` | Version-specific behaviour found |
|---|---|---|---|
| 1.4.4, 1.4.5 | none | 1,164 passed each | items 30 (returns NULL), 33 (imports as BLOB), 34 |
| 1.5.0, 1.5.1 | `duckdb-1-5` | 1,318 passed each | items 12, 30, 33 (abort in `malloc` on 1.5.0), 34; union and run-end layouts differ from 1.5.2+ |
| 1.5.2 | `duckdb-1-5` | 1,318 passed | items 12, 30, 33, 34 |
| 1.5.3 | `duckdb-1-5-3` | 1,322 passed | items 12, 30, 33, 34 |
| 1.5.4 | `duckdb-1-5-4` | 1,412 passed | items 12, 30, 33, 34 |
| 1.5.5 | `duckdb-1-5-4` | 1,412 passed | items 30, 33; the assertion build (item 37) |

Items 20, 24 to 29, 31, 32, 35 and 36 hold on all eight releases; item 21
was run, and holds, on 1.4.4, 1.5.0 and 1.5.5.

### 10.8 How this pass was verified

The code is the 44 commits from `16f7c3b` to `42f894c` on top of `6382d0a`;
the commit after them adds this section.
Local runs are x86-64 Linux against the prebuilt DuckDB 1.5.5 unless stated.

- **Tests, at `42f894c`:** with `bundled-test-prebuilt,duckdb-1-5-4`, 1,013
  library, 42 `append_metadata`, 279 end-to-end (`tests/ffi_roundtrip`), 71
  integration, 4 `append_metadata_cli`, and one each in `aggregate_leaks`,
  `handle_leaks` and `secret_zeroize`; 207 doctests (3 ignored). With default
  features, 820 library tests and 174 doctests (1 ignored).
- **Lints, before every push:** `cargo fmt --check` (added to the local gate
  late in the pass, after a formatting slip was caught before a push);
  clippy `-D warnings` on default features, `duckdb-1-5`, `duckdb-1-5-3`,
  `duckdb-1-5-4`, `bundled-test-prebuilt`, `bundled-test-prebuilt,duckdb-1-5`
  and `bundled-test-prebuilt,duckdb-1-5-4`; beta clippy on CI's two
  configurations; hello-ext's clippy; `RUSTDOCFLAGS="-D warnings" cargo doc`
  on default features, `duckdb-1-5`, `duckdb-1-5-3` and `duckdb-1-5-4`.
  The doc builds were added after CI's doc job failed at `f16c26b` (fixed in
  `ed5e381`). `cb02176` was pushed with a clippy error, because the gate and
  the commit ran in one command; `f79704c` fixed it, and from then on the gate
  ran as its own step.
- **Every release:** `cargo test --all-targets` with CI's pinning of
  `duckdb` / `libduckdb-sys`, at `42f894c` (table in 10.7): 0 failures on
  any of the eight releases. The same matrix passed at earlier commits.
- **Assertions and AddressSanitizer:** DuckDB v1.5.5 built from source with
  `FORCE_ASSERT` and ASan (core functions, ICU and Parquet linked; not
  httpfs), each end-to-end test in its own process at `f16c26b`: 262 of 263
  pass. The one failure needs httpfs ("Secret type 's3' does not exist").
  The tests added afterwards were run the same way and all pass. The
  integration tests pass except the scaffold test, which hangs because
  `rustc -vV` inherits `LD_PRELOAD`: a harness artefact, not run.
  `handle_leaks`, `aggregate_leaks` and `secret_zeroize` pass. 0 ASan reports.
  This build found item 37.
- **Miri:** the library at `f79704c`, 925 passed, 1 ignored, no undefined
  behaviour, and one warning: an integer-to-pointer cast in the string
  decoder. Reading the code behind it found the byte-order defect (10.2,
  High). At `08311ce` the library also runs under Miri on s390x: 934
  passed, 1 ignored, 0 undefined behaviour, no warning. CI's `miri` job now
  runs both targets. CI's Miri failed once in this pass, at `ed5e381`, on
  one of this pass's own tests, which compared function addresses; Rust
  guarantees no address identity for functions. The test now runs the
  callbacks instead (`cb02176`).
- **Fuzzing,** libFuzzer via cargo-fuzz: `value_render` against a live DuckDB
  1.5.5, 4,047,724 runs in 10,801 s; `description_yml`, 107,229,512 runs in
  7,201 s; `duck_string`, 5,275,667,662 runs in 7,201 s (built before the
  byte-order fix; on x86-64 its oracle's `from_le_bytes` and the fixed
  `from_ne_bytes` are the same function); `validators`, 435,075,141 runs in 7,201 s. No crashes, no
  artifacts.
- **A structure-aware Arrow-import differential fuzzer** (`scratchpad`,
  kept): a seeded generator of valid nested Arrow arrays (struct, list, list
  view, fixed-size list, map, dictionary, run-end, sparse union; depths 1-3;
  offsets, null counts including -1, lengths spanning the 2048-row vector),
  each run through `data_chunk_from_arrow` and dropped, checked against
  DuckDB 1.5.5 and its AddressSanitizer build. It found the two crashes
  above. After they were fixed: 0 crashers over 200,000 arrays, and 0 ASan
  errors over ~4,600 (windows 0-3500, 8000-8699, 50400-50799). A separate
  heap-dependent report it made against the pre-fix code (a run-end array
  under a union) did not reproduce under AddressSanitizer once the map case
  was fixed.
- **Mutation testing** (cargo-mutants 27.1.0):
  - Full sweep with the repository's configuration (`--lib`,
    `duckdb-1-5-4`): 1,810 mutants, 1,496 caught, 292 unviable, 0 timeouts,
    22 missed. 7 now fail new tests: 3 in `FfiState::tag_for`, killed after
    its salt became a pure function, and 4 pure-logic gaps. 3 are equivalent
    and excluded with the argument. 2 had fallen out of the render guard's
    exclusion through a rename. 10 need a live DuckDB and are excluded;
    their end-to-end run is below.
  - Then every mutant the configuration excludes, end to end (`--no-config`,
    `bundled-test-prebuilt,duckdb-1-5-4`, libduckdb 1.5.5), in four groups:
    - The excluded FFI files, against `ffi_roundtrip`: 684 mutants, 538
      caught, 73 unviable, 2 timeouts, 71 missed. The two timeouts are
      detections: `FileHandle::read -> Ok(1)` loops, and a no-op
      `InterruptHandle::cancel` hangs its test.
    - The mock vectors, against the integration tests: 197 mutants, 178
      caught, 17 unviable, 2 missed.
    - `append_metadata`: 121 mutants, 113 caught, 1 unviable, 7 missed.
    - The `exclude_re` patterns: 84 mutants, 48 caught, 3 unviable, 33
      missed.
  - Of the 113 missed, 29 are `Debug` impls, excluded by design. 10 are the
    documented equivalents in `.cargo/mutants.toml`. Two, in `ErrorData`, were
    wrongly recorded there as killed end to end (`89f8bf2`); they now fail
    tests. `tests/handle_leaks.rs`'s header likewise named two `Drop`
    survivors that `ffi_roundtrip` had caught, and missed the one it had not;
    corrected in `7242606`. 24 are `Drop` impls, and the free in `parameter_name`, that nothing
    functional observes; `tests/handle_leaks.rs` kills them. The other 48
    were re-run through cargo-mutants after tests were added, together with
    the 6 mutants of the refactored `append_bytes_as` / `fits_append_length`
    (3 of the 48 no longer exist after that refactor): 9 of 9 for
    `append_metadata` (its patterns matched 2 same-named siblings too), 26 of
    26 for the `Drop` and `ErrorData` mutants, and 44 tested for the rest,
    with 32 caught and 12 missed.
  - The 12 are equivalent, each argued from DuckDB's source:
    - `runtime_vector_size`'s `==` (`duckdb_vector_size()` is 2048 in every
      official build, as is the fallback).
    - `FileSystem::open`'s and `InstanceCache::get_or_create`'s `&&`: DuckDB
      writes the out-handle only on success (`file_system-c.cpp`,
      `duckdb_open_internal`).
    - `Expression::fold`'s error-path `!` (`expression-c.cpp` writes the
      value only on success).
    - The read chunk size, and `read_to_end`'s size-hint arm: buffer sizing
      only.
    - `SelectionVector`'s length limit and its two slice accessors (equal
      limits; `new` refuses a null handle).
    - `parameter_name`'s `||` (DuckDB returns null for index 0 and past the
      end itself).
    - `clear_bindings -> Ok` (it fails only for a statement
      `PreparedStatement` cannot hold).
    - `FileHandle::close -> Ok` (the local file system's `Close` never
      throws: `local_file_system.cpp`).
  - The functions added after those runs (`capacity_within`,
    `max_capacity_within`, `fits_one_vector`, `hugeint_at`, `uhugeint_at`):
    every mutant caught but two `<` → `<=` in minimum computations, which
    are equivalent (equal operands give the same value) and excluded.
  - The workflow's `sed`/`grep` extraction of `.cargo/mutants.toml` (34
    globs, 18 patterns) was replayed after every edit and matches a TOML
    parse.
- **Release mechanics:**
  - `cargo semver-checks` 0.50.0 against 0.16.0 runs no checks for a 0.x
    minor bump (254 skipped), so it was run as a patch release: 223 checks,
    8 failures with `duckdb-1-5-3` (7 on default features). All are listed
    as breaking in the CHANGELOG; three were not until this pass.
  - `cargo deny` 0.19.0: advisories, bans, licences and sources pass for the
    crate. For `fuzz/Cargo.lock` everything passes except the licence check,
    on `libfuzzer-sys`'s NCSA term; that crate is fuzz-only and never
    published.
  - A docs.rs-equivalent build (`--cfg docsrs`, nightly, `-D warnings`)
    finishes with 0 warnings. `cargo publish --dry-run`
    packages 227 files (873,740 bytes compressed) and builds them; it warns
    that the two examples are not packaged, which `Cargo.toml`'s `exclude`
    intends. semver-checks, the docs.rs build and the dry run were run at
    `08311ce`; no manifest, lock file or `deny.toml` has changed since
    cargo-deny ran on `bc94b7a`.
- **CI:** all 50 jobs of the workflow passed at `42f894c` (run 610). They
  include the end-to-end suite against DuckDB 1.4.4, 1.4.5, 1.5.0, 1.5.3 and
  1.5.4 as well as the pinned 1.5.5; DuckDB built from source on Linux, macOS
  and Windows; Miri over the library on x86-64 and, added this pass, on
  big-endian s390x; LeakSanitizer and AddressSanitizer; the incremental
  mutation gate; both ABI guards; the book and README code blocks; the
  scaffold end-to-end job; MSRV and the dependency floor; `semver-checks`;
  and the publish dry run.
