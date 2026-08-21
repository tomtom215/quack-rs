# Changelog

All notable changes to quack-rs, mirrored from
[`CHANGELOG.md`](https://github.com/tomtom215/quack-rs/blob/main/CHANGELOG.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
quack-rs adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

## [0.17.0] — 2026-08-21

### Security

- **A panicking `Drop` in extension state aborted the process.** Every FFI
  destructor quack-rs generates — `FfiState<T>::destroy_callback`,
  `FfiBindData` / `FfiInitData` / `FfiLocalInitData::destroy`,
  `replacement_scan::drop_box`, `TypedCallbacks::destroy_extra` — dropped a
  `Box<T>` of arbitrary user data directly inside an `extern "C" fn`. Since Rust
  1.81 an unwind across that boundary is a guaranteed process abort. Reproduced
  against `DuckDB` 1.5.4: an aggregate whose state type has a panicking `Drop`
  killed the process with `SIGABRT` from inside
  `duckdb::RowOperations::DestroyStates`, on a task-scheduler thread. All of them
  now run under the new `callback::catch_ffi_panic`, which is public so
  extensions writing their own `extern "C"` destructors get the same containment.

  Where `DuckDB` offers an error channel the panic is now reported instead of
  swallowed: `CAPIAggregateStateInit` checks the error flag and throws, so a
  panicking `Default::default()` becomes an ordinary SQL error rather than a
  silent NULL. The state destructor has none (`CAPIAggregateDestructor` takes no
  info and returns nothing), so there the message is discarded.

  `FfiState::init_callback` also no longer forms a `&mut Self` over the
  possibly-uninitialised allocation `DuckDB` hands it.

### Fixed

- **Pitfall L8 — `DEFAULT_NULL_HANDLING` does not propagate NULLs for scalar
  functions.** quack-rs documented that `DuckDB` "automatically returns NULL if
  any argument is NULL, without your function callback being called". For a
  scalar function registered through the C API that is false at run time:
  `CAPIScalarFunction` calls the callback for every row including NULL ones and
  never inspects the result's validity, and the only NULL check in
  `ExpressionExecutor::Execute` is `VerifyNullHandling`, whose entire body is
  inside `#ifdef DEBUG`. A callback that ignores validity therefore returns a
  non-NULL answer for a NULL input, silently, in every release build.

  `SELECT f(NULL)` still returns NULL — a literal NULL is constant-folded before
  the function is reached — which is why the bug survives review. From a column
  it does not. New `DataChunk::propagate_nulls` / `any_null` restore SQL
  semantics in one line; the new typed constructors below get it right by
  construction; the docs, the book chapter and `LESSONS.md` now state what
  `DuckDB` does, with the source quoted. A regression test pins the behaviour.
  Aggregates are unaffected — their executor really does filter NULL rows.

- **Composite `TypeId`s silently produced an invalid type.**
  `duckdb_create_logical_type` "returns an invalid logical type" for `DECIMAL`,
  `ENUM`, `LIST`, `STRUCT`, `MAP`, `ARRAY` and `UNION` — a *non-null* handle
  wrapping `LogicalTypeId::INVALID`, so the existing null check never fired.
  `.param(TypeId::Struct)` failed much later with a message that named neither
  the parameter nor the fix, and `get_type_id()` on one panicked. New
  `TypeId::is_composite` / `composite_constructor_hint`; `LogicalType::new`
  asserts, `try_new` errors, and every builder validates before allocating any
  `DuckDB` handle.

- **`extra_info` leaked when a builder was not registered.** `DuckDB` only takes
  ownership at `duckdb_*_set_extra_info`; a dropped builder dropped the pointer.
  This reached users through APIs that never mention a pointer —
  `TableFunctionBuilder::with_state` boxes two closures. Found by Miri.

- **Two stale-borrow bugs in `src/secrets.rs`'s own tests**, which took a pointer
  into a `String`, called a `&mut` method, then read through the stale pointer.
  The library's `zeroize_string` was correct throughout.

- **The reference example disabled every panic guard in the crate.**
  `examples/hello-ext/Cargo.toml` shipped `panic = "abort"` — the setting
  `validate_release_profile` rejects outright and the scaffold refuses to
  generate, because it makes every `catch_unwind` in quack-rs inert. CI built
  that example, loaded it into a real `DuckDB`, and held it up as the way to do
  this. Two new tests hold the example *and* the scaffold's generated profile to
  quack-rs's own validator, so the generator and the validator cannot drift.

- **The `ScaffoldConfig` example in the README and two book pages did not
  compile.** The struct gained three fields and the exhaustive literals were
  never updated; rustdoc examples are compiled by `cargo test` but Markdown code
  fences are not, so the copy a new user reaches for was the broken one. All now
  use `..ScaffoldConfig::default()`.

- **Registration failures now name what to check.**
  `duckdb_register_*_function` reports failure as a bare `DuckDBError` with no
  message. There are exactly three causes, and a name collision with a `DuckDB`
  built-in (`list_sum`, `array_sum`, …) looks identical to a type error. The
  message names all three and points at
  `SELECT * FROM duckdb_functions() WHERE function_name = '<name>'`.

### Added

- **Scalar functions as safe Rust closures.** `ScalarFunctionBuilder::map1` /
  `map2` / `map1_str` / `map2_str` / `map1_opt` / `map2_opt` take an ordinary
  closure; parameter and return types come from its signature, NULLs propagate
  correctly, and a panic becomes a SQL error. `VARCHAR` gets its own
  constructors so the closure can borrow a `&str` straight out of the vector.
  One indirect call per chunk, not per row.

- **`Value` gained every remaining constructor** — all scalar widths, the
  temporal family, `INTERVAL`, `BLOB`, `DECIMAL`, and the composites (`STRUCT`,
  `LIST`, `ARRAY`, `ENUM`; `MAP` and `UNION` behind `duckdb-1-5`) — plus
  `is_sql_null` (distinct from `is_null`, which asks about the handle) and
  `as_enum_index`. `struct_value` checks the field count first, because
  `duckdb_create_struct_value` takes no count and reads one value per field of
  the *type*. `list_value` / `array_value` take the **element** type: `duckdb.h`
  contradicts itself here, and the implementation settles it.

- **`PreparedStatement` gained the remaining 18 typed binds** and `bind_value`,
  the escape hatch for every composite type.

- **Cancellation and progress.** `OwnedConnection::interrupt_handle` returns a
  `Send + Sync` token, lifetime-tied to the connection, that a watchdog thread
  can use to stop a running query.

- **Streaming results.** `PreparedStatement::execute_streaming` and
  `QueryResult::is_streaming` (`duckdb-1-5`).

- **`QueryResult::column_logical_type`** keeps the nested structure that
  `column_type` collapses, and **`result_kind`** separates rows from row counts.

- **`LogicalType::register`** — `CREATE TYPE` from the C API, so an extension can
  ship a named `ENUM` or `STRUCT`. Stable-prefix; no feature needed.

- **`ScalarBindData<T>` / `ScalarLocalState<T>`** (`duckdb-1-5`) — typed bind
  data and per-thread local state for scalar functions, with the
  `duckdb_delete_callback_t` generated and panic-safe. The raw
  `set_bind_data` / `set_state` route makes the extension author write their own
  `unsafe extern "C" fn` around `Box::from_raw`, which is the abort hazard above
  relocated into user code.

- **`vector::ops`** (`duckdb-1-5`) makes `SelectionVector` usable: `copy_selected`,
  `slice`, `reference_value`, `reference_vector` and `OwnedVector`. Documents
  that `slice` produces a *dictionary* vector, after which every reader in this
  crate reads the wrong rows.

- **Arrow C Data Interface bridge** — the new `arrow` module behind a new
  **`duckdb-1-5-4`** feature, wrapping the eight-function conversion family
  DuckDB 1.5.0 added: `ArrowOptions` (from a connection or a `QueryResult`),
  owning `ArrowSchema` / `ArrowArray` / `ArrowConvertedSchema` RAII types, and
  `to_arrow_schema` / `data_chunk_to_arrow` / `schema_from_arrow` /
  `data_chunk_from_arrow`. **No `arrow` crate dependency**: the module works on
  the ABI records `libduckdb-sys` defines, which have arrow-rs's
  `FFI_ArrowSchema` / `FFI_ArrowArray` layout, so bridging is a pointer cast —
  `ArrowArray::take_from` moves a record out of a foreign wrapper and leaves a
  released placeholder, so only one side ever calls `release`.

  The ownership rules were read out of `arrow-c.cpp` and `arrow_converter.cpp`
  rather than inferred: `duckdb_data_chunk_from_arrow` sets
  `arrow_array->release = nullptr` *before* the conversion loop body, so it
  claims the array on the error path too — hence `data_chunk_from_arrow` takes
  the array **by value**, and the by-value binding still releases it in the one
  case (a zero-column schema) where the loop never runs. `to_arrow_schema` and
  `data_chunk_to_arrow` install `release` last, after everything that can throw,
  so a failed conversion leaves nothing to free.

  Two crashes DuckDB does not guard are refused here instead:
  `duckdb_data_chunk_from_arrow` indexes `arrow_array->children[i]` once per
  schema column with no bounds check and dereferences an already-released array,
  so `ArrowConvertedSchema` remembers its column count and both are checked
  first. The module documents the one case it still cannot check — a child whose
  buffers do not match the type its schema declares.

  The **`duckdb-1-5-4`** feature's floor is set by the bindings, not by DuckDB:
  all eight functions are in `duckdb_ext_api_v1` from DuckDB **1.5.0** (verified
  against the v1.5.0 `duckdb_extension.h`), but `libduckdb-sys` declared
  `ArrowSchema` / `ArrowArray` as opaque zero-sized bindgen placeholders until
  **1.10504.0**. `src/arrow.rs` carries a `const` assertion that says so.

- **`COPY … FROM`** — `CopyFunctionBuilder::copy_from` attaches a quack-rs table
  function as a format's reader, so an extension can implement loading as well
  as writing. Supporting pieces:

  - `TableFunctionBuilder::build_handle` returns a configured, unregistered
    `TableFunctionHandle` — `register` is now that plus
    `duckdb_register_table_function` — because a `COPY … FROM` reader is
    *attached to a copy function* rather than registered on its own.
  - `BindInfo::result_column_count` / `result_column_name` / `result_column_type`
    (`duckdb-1-5`) read the target table's schema, which `COPY … FROM` fixes
    before the bind callback runs. `duckdb.h` is explicit that such a bind
    "should not define its own result columns".
  - `CopyBindInfo::options` exposes the `COPY … TO` options as the STRUCT value
    DuckDB builds, and `Value::struct_field_names` walks the field names off the
    value's borrowed logical type without exposing the handle (pitfall P11).
  - `CopyFunctionBuilder::extra_info`, with the same
    ownership-until-transfer guarantee as the other builders.

  `duckdb_copy_function_set_copy_from_function` reports every rejection by doing
  nothing at all, so `copy_from` checks `duckdb.h`'s stated precondition — "the
  table function must take a single VARCHAR parameter (the file path)" — which
  DuckDB never enforces: `CCopyFromBind` builds the argument list itself and
  never consults `tf.arguments`, so a mismatch surfaces much later inside the
  reader's own bind callback.

### Changed

- **Breaking: `CopyFunctionBuilder` is no longer `Send` or `Sync`.** Supporting
  `COPY … FROM` gave it two new fields that each carry a raw pointer —
  `copy_from: Option<TableFunctionHandle>` (a `duckdb_table_function`) and
  `extra_info: Option<ExtraInfo>` (a `*mut c_void`) — and a raw pointer is
  neither `Send` nor `Sync`. `cargo-semver-checks` classifies this as
  `auto_trait_impl_removed`, a major break, which is why the crate version moves
  to **0.17.0**: for a pre-1.0 crate Cargo treats the leftmost non-zero
  component as the major, so the minor position is where a break goes
  (`RELEASING.md`, "Semantic versioning policy").

  The change aligns the type with its three siblings rather than making it an
  outlier — `ScalarFunctionBuilder`, `TableFunctionBuilder` and
  `AggregateFunctionBuilder` were already `!Send + !Sync` in 0.16.0, each
  because it holds a raw `DuckDB` handle. A builder is a short-lived object
  constructed and registered inside `duckdb_init_c_api`, and no `DuckDB` C API
  accepts one from another thread, so the traits were never usable for anything
  real. Code that moved a `CopyFunctionBuilder` between threads must now
  construct it on the thread that registers it.

- **Flaky tests: a shared counter raced across parallel tests.** The
  `extra_info` tests and the new `arrow` ones each reset one `static
  AtomicUsize` and then asserted on it, but `cargo test` runs tests in
  parallel — so one test's reset could land between another's drop and its
  assertion. `dropping_an_untransferred_extra_info_frees_it` failed in CI while
  the identical job on the identical commit passed. Each test now owns its
  counter: `extra_info` carries it in the allocation, and `arrow` carries it in
  the record's own `private_data`, which is what that field is for. Verified
  with 100 repeat runs, zero failures.

- **`data_chunk_from_arrow` aborted a debug `DuckDB` on a zero-row array.**
  `duckdb_data_chunk_from_arrow` passes `arrow_array->length` straight through
  as the chunk's *capacity* (`dchunk->Initialize(alloc, types, length)`), and
  `VectorCacheBuffer` turns a capacity of zero into
  `Allocator::AllocateData(0)`, whose `D_ASSERT(size > 0)` aborts a debug build.
  A release build allocates nothing and carries on — so whether an empty batch
  worked depended on how the engine happened to be compiled. Zero-row arrays are
  now refused with a message that says why. Caught by CI's coverage job, which
  compiles DuckDB from source in debug; every local run linked a release
  prebuilt libduckdb, where the assertion does not exist.

- **`ExtraInfo` silently removed `RefUnwindSafe` from four public builders.**
  Its `transferred` flag was a `Cell<bool>`, and `Cell` contains an
  `UnsafeCell`, which is `!RefUnwindSafe` — so `ScalarFunctionBuilder`,
  `TableFunctionBuilder`, `AggregateFunctionBuilder` and `CopyFunctionBuilder`
  all lost the auto trait. It is an `AtomicBool` now, which offers the same
  shared-reference mutation and restores the trait. (`RefUnwindSafe` only — the
  same `extra_info` field is also one of the two that make `CopyFunctionBuilder`
  `!Send + !Sync`, which is a separate and deliberate break; see the entry
  above.)

- **128-bit splitting and reassembly was open-coded at eight call sites.**
  `Value::as_i128` / `as_u128` / `as_uuid` / `as_decimal` / `uuid` and
  `PreparedStatement::bind_i128` / `bind_u128` / `bind_decimal` each did their
  own `<< 64` / `>> 64` word arithmetic. That is the one place in the crate
  where being silently wrong is easiest — a shift in the wrong direction still
  compiles, still round-trips zero, and still round-trips anything that fits in
  64 bits. They now all route through four `pub(crate)` helpers
  (`hugeint_from_i128` / `hugeint_to_i128` / `uhugeint_from_u128` /
  `uhugeint_to_u128`) that live next to each other and are unit-tested in both
  directions, at the extremes and against hand-built records. Mutation testing
  confirms all seven shift mutants across the three modules are now killed.

- **Test gaps the mutation sweep exposed, once it could see the files.** Chief
  among them the shift direction in `hugeint_from_i128` / `uhugeint_from_u128`:
  swapping `>>` for `<<` still compiles, still round-trips zero and still
  round-trips anything that fits in 64 bits, so nothing in the suite noticed.
  Also `TypeId::composite_constructor_hint`'s per-variant arms,
  `LogicalType::check_slot`'s rejection path, `composite_message`,
  `LogicalTypeError::api_func`, `secrets::parse_scope_array`, the `arrow`
  accessors against a populated record rather than only an empty one, and
  `map2_str`'s NULL propagation when just one argument is NULL.

  With the three gate defects fixed and the two FFI-wrapper modules excluded,
  the incremental sweep over this branch's 40 changed source files reports 404
  mutants — 250 caught, 154 unviable, **none missed**. What survives after that
  is annotated in the source with `#[mutants::skip]` and a reason, rather than
  filtered out of sight: the bare FFI reads (`DataChunk::size` /
  `column_count`, `ScalarBindData::set`, `ScalarLocalState::set`), two `Drop`
  impls whose effect is only visible in freed memory or to a leak checker
  (`OwnedVector`, `SecretEntry`), the deprecated `FfiBindData::get_from_bind`
  whose mutant *is* the function (it returns `None` unconditionally, because
  `DuckDB` has no `duckdb_bind_get_bind_data`), `map2` / `map2_str` whose
  per-row NULL check only runs inside `DuckDB`'s expression executor, and
  `Value::as_str_or_default`, whose null-handle answer is exactly the mutant's
  `String::new()`.

  Two of those turned into real work rather than an annotation. `SecretEntry`'s
  zeroize-on-drop is a security property the crate advertises and nothing
  asserted — its body is now `SecretEntry::zeroize_in_place`, tested directly,
  with `Drop` left as a one-line delegation. And `hugeint_to_i128` combined its
  halves with `|`; because the halves occupy disjoint bits, `|` → `^` cannot
  change the result for any input, so that mutant was unkillable *by
  construction*. The halves are added instead: identical here, incapable of
  overflowing (`i64::MIN << 64` is exactly `i128::MIN`, and the round trips at
  the extremes would panic in a debug build if that were wrong), and `-` or `*`
  in its place dies at once.

- **The mutation-testing gate always reported 100% and always passed.**
  `cargo mutants --output DIR` writes its results to `DIR/mutants.out/`, so
  `--output mutants.out` put them in `mutants.out/mutants.out/`. The report step
  counted `mutants.out/caught.txt` and friends, found nothing, computed
  `SCORE=100%` from `TOTAL=0`, and skipped its `exit 1` because `MISSED` was
  also 0. The run on this PR printed `MUTATION SCORE: 100%` and passed while
  cargo-mutants' own summary line in the same log read `229 missed, 238 caught,
  218 unviable`. Both jobs now pass `--output .`.

- **Incremental mutation testing skipped every top-level `src/*.rs`.** The job
  selected changed files with the pathspec `src/**/*.rs`, but git's default
  wildmatch lets `*` cross `/`, so that pattern requires at least one directory
  component after `src/` and matches no top-level file at all. `src/value.rs`,
  `src/query.rs`, `src/appender.rs` and every other module directly under
  `src/` were silently excluded while the job reported success. It now filters
  to `.rs` in the shell.

- **The mutation gate's Display/Debug filter matched nothing.** `mutants.toml`
  carried `exclude_re = ["^fmt::"]`, commented "Display/Debug impls".
  cargo-mutants matches that regex against the whole line it prints for a
  mutant — `src/x.rs:1: replace <impl core::fmt::Debug for T>::fmt -> … with …`
  — which begins with the file path, so a pattern anchored at `fmt::` can never
  match and seventeen unkillable `Debug::fmt -> Ok(Default::default())` mutants
  survived every sweep. cargo-mutants' own documented form, `impl Debug`, does
  not match this crate either: it writes `impl core::fmt::Debug for T`, and the
  qualified path lands in the mutant name. The pattern is now
  `impl [a-z:]*Debug for`. `Display` is deliberately not excluded — those impls
  render error text that tests assert on, so their mutants die and belong in
  the gate.

  The incremental job now also re-applies `exclude_re` from `mutants.toml` as
  CLI flags, the way it already did for `exclude_globs`: cargo-mutants only
  *combines* a CLI `--exclude-re` with the config file from 27.0.0 onwards, and
  that job passes one.

- **`src/value.rs` and `src/query.rs` are excluded from mutation testing, and
  their pure logic moved out so that it is not.** Every function left in those
  two files wraps a `DuckDB` C call, which the gate's `--cargo-arg=--lib` run —
  no live engine — structurally cannot reach; that is the same rationale
  `mutants.toml` already carried for nine sibling modules, and between them the
  two files accounted for 206 of the 220 survivors. Excluding them wholesale
  would have swallowed the pure code too, so it moved to siblings that stay in
  the gate: `src/value/hugeint.rs` (the four 128-bit word helpers),
  `src/value/defaults.rs` (the fourteen `as_*_or` accessors, as a second
  inherent `impl Value`) and `src/query/cstr.rs` (`to_c_sql`,
  `c_str_to_owned`). Seven of the `as_*_or` accessors had no unit test at all,
  and nothing exercised `c_str_to_owned`'s non-null path — all of them were
  among the 220 — so nine new tests turn those survivors into kills rather than
  hiding them. No public item moved: the re-export keeps every existing path.

- **Four CI jobs never actually ran.** `rust-toolchain.toml` pins
  `channel = "stable"`, and a rustup toolchain file overrides the default
  `dtolnay/rust-toolchain` sets — so the `miri`, `leak-check`, `fuzz` and
  `nightly` jobs all resolved a bare `cargo` to stable. Miri and LeakSanitizer
  failed loudly (`the 'miri' component ... is not available for the
  'stable-...' toolchain`; `the -Z flag is only accepted on the nightly
  channel`); the informational nightly job failed silently, re-testing stable.
  All four now invoke `cargo +nightly` explicitly, with a comment saying why.

  The `bundled-test-prebuilt` job's clippy step was also missing the `env:`
  block its sibling test step has, so `build.rs` panicked looking for
  `duckdb.hpp` before clippy ran.

- **`cargo doc` with `-D warnings` failed.** Six intra-doc links were broken or
  redundant — `QueryResult::column_logical_type`, `scalar::typed`,
  `vector::ops` (twice), `TableFunctionBuilder::build_handle` — and
  `vector::ops` carried a doc comment on both the `pub mod` declaration and the
  file's own `//!` header, so its module docs were resolved in the parent's
  scope and every link into the module failed with no source location. Three
  more links in `appender` and `table_description` pointed at `duckdb-1-5`-gated
  methods and so broke the *default*-feature doc build. `cargo doc` is now clean
  under `-D warnings` on all four feature sets.

- **Pitfall L9 — `duckdb_data_chunk_from_arrow` takes the array even when it
  fails.** `duckdb.h` reads like a success-path statement; `arrow-c.cpp` nulls
  `arrow_array->release` inside the per-column loop, before the work that can
  throw. Guessing either way gives you a bug — double release, or a leaked Arrow
  buffer tree when a zero-column schema means the loop never runs. Documented in
  `LESSONS.md` and the book, and made impossible by the by-value signature.

  The pitfall count in the README, the crate docs and the FAQ was stale at 17;
  `LESSONS.md` documents 21, and the book's catalogue was missing L8 and L9. All
  four now agree.

- **A copy function may now implement `COPY … FROM` alone.**
  `CopyFunctionBuilder::register` used to require `bind`, `sink` *and*
  `finalize`, which made a read-only format impossible even though
  `duckdb_register_copy_function` accepts one: it decides what a copy function
  supports from `info.sink != nullptr` and `copy_from_bind != nullptr`
  independently. Leaving all three unset is now valid when `copy_from` is set;
  setting only some of them is an error that says so. Existing `COPY … TO`
  functions are unaffected.

- CI gains four jobs: `miri` (546 unit tests under the interpreter),
  `leak-check` (LeakSanitizer over the end-to-end suite against a real
  `libduckdb`, now leak-clean), `fuzz` (`cargo-fuzz` over the description.yml
  parser, the `duckdb_string_t` decoder and the validators) and `semver`
  (`cargo-semver-checks`). `tests/ffi_roundtrip.rs` is now linted — it is
  feature-gated, so the plain clippy job had been compiling it away to nothing.

- `extension-load` is now a matrix over `DuckDB` v1.4.4, v1.5.0, v1.5.5 and
  `latest`, rather than one `releases/latest` run that silently retargeted
  whenever `DuckDB` shipped. That is the README's compatibility claim, proven
  rather than sampled.

- New end-to-end coverage for nested types as scalar-function *input* — `LIST`
  offsets that are cumulative rather than uniform, NULL elements inside a list,
  a `MAP` key miss, and an `ARRAY`'s fixed stride across rows. Writing them was
  already covered; reading them does raw offset arithmetic against a layout only
  `DuckDB` defines, which a mock cannot check.

- `AUDIT.md` records the full review: what was read, what was probed, what was
  verified correct, and what is still open.

## [0.16.0] — 2026-08-19

### Security

- **New `abi` module: `duckdb_ext_api_v1` layout verification.** `DuckDB` hands a
  loadable extension a struct of function pointers. Its first 357 slots — the
  "stable prefix" — have been byte-for-byte identical in every release from
  v1.2.0 through v1.5.5, but everything past that is the *unstable* region, and
  `DuckDB` inserts new entries **in the middle** of it between releases
  (`duckdb_appender_clear` at slot 410 in v1.5.0, `duckdb_geometry_type_get_crs`
  at slot 493 in v1.5.2). Every quack-rs wrapper behind the `duckdb-1-5` /
  `duckdb-1-5-3` features — 105 C API functions covering scalar bind/init, copy
  functions, catalog access, `ErrorData`, `FileSystem`, `Expression`,
  `SelectionVector`, config options, table descriptions and the client context —
  lives in that region.

  `DuckDB` does not catch this: an extension stamped `C_STRUCT` + `v1.2.0` (the
  default) is accepted by *any* `DuckDB` whose C API version is at least v1.2.0
  and then handed the whole struct, unstable region included. Loading such a
  build into a `DuckDB` with a different layout silently dispatches to the wrong
  function pointers. Verified end-to-end: an extension built against `DuckDB`
  1.5.0's headers, stamped `C_STRUCT`/`v1.2.0`, loaded into `DuckDB` 1.5.5 aborts
  the process with `double free or corruption`.

  [`abi::check`] compares the slot count of the compiled-in layout against the
  layout the running engine uses (resolved from `duckdb_library_version()`, which
  sits at stable slot 7 and is therefore always dispatched correctly).
  `init_extension` / `init_extension_v2` and the `entry_point!` /
  `entry_point_v2!` macros now run that check under the new
  [`AbiPolicy::Strict`] default whenever `duckdb-1-5` is enabled, turning the
  memory corruption above into a `LOAD` error that names the mismatch and the
  remedy. `AbiPolicy::Warn` and `AbiPolicy::Trust` opt out; `Trust` is the right
  choice for binaries stamped `C_STRUCT_UNSTABLE`, where `DuckDB` already pins
  the release. Extensions that stay on the stable prefix are unaffected and keep
  their forward compatibility.

  `scripts/check-abi-table.py` re-derives the layout table from every upstream
  release header and runs in CI, so the table cannot drift as `DuckDB` releases.

- **`DuckStringView::from_bytes` was unsound.** It was safe to call yet
  dereferenced the heap pointer embedded in bytes 8–15 of a pointer-format
  `duckdb_string_t`, so safe code holding attacker-influenced bytes could read
  arbitrary memory. Replaced by two honest constructors: `from_raw` (`unsafe`,
  honours pointer format — what callbacks want) and `inline_from_bytes` (safe,
  returns `None` for pointer-format values). `from_bytes` is deprecated and no
  longer dereferences.

- **`CopyGlobalInitInfo::get_file_path` corrupted the heap.** It called
  `duckdb_free` on the pointer from
  `duckdb_copy_function_global_init_get_file_path`, which returns
  `info_ref.file_path.c_str()` — the interior pointer of a C++ `std::string`
  `DuckDB` still owns and destroys itself. Every `COPY ... TO` through a
  quack-rs copy function handed the allocator a pointer it never issued;
  the first live test of the path aborted with
  `corrupted size vs. prev_size in fastbins`.

  Every other `duckdb_free` call site in the crate was then audited against
  `DuckDB`'s implementation, and all twelve are correct. The signature is not
  sufficient to decide: `char *` returns are owned and `const char *` returns
  are usually borrowed, but `duckdb_parameter_name` is declared `const char *`
  and returns `strdup(...)`, so it *is* owned. Recorded as `LESSONS.md` P11 with
  the full table.

#### Generated CI

- **The generated CI workflow left one action unpinned.** Three of its four
  actions were SHA-pinned; `dtolnay/rust-toolchain@stable` was not, justified by
  a comment claiming its SHA "changes with each Rust release". That is not how
  the action works — it reads the toolchain from `rust-toolchain.toml` or its
  `toolchain:` input at run time, so pinning the action's SHA does not pin the
  Rust version. quack-rs's own CI SHA-pins the same action and gets current
  stable. A branch is a moving target its owner can repoint, and a workflow step
  runs arbitrary code in the user's CI. All four are now pinned to the same SHAs
  quack-rs itself uses, and a test asserts every `uses:` in the generated
  workflow carries a 40-character hex ref.

### Fixed

#### The release-profile validator required the setting that breaks panic safety

- **`validate_release_profile` required `panic = "abort"`, which makes every one
  of quack-rs's panic guards inert.** quack-rs wraps every `extern "C"` entry
  point — the extension entry point and every scalar/table/aggregate/cast/copy
  callback macro — in `catch_unwind`, so a panic in an extension's code becomes
  a `DuckDB` error instead of a crash. `catch_unwind` catches nothing under
  `panic = "abort"`: the runtime aborts before unwinding starts. Demonstrated
  directly rather than assumed —

  ```text
  rustc -O            panic_probe.rs  →  caught, process survived,  exit 0
  rustc -O -C panic=abort  …          →  Aborted,                   exit 134
  ```

  — so the validator was telling extension authors to configure the one setting
  that turns a recoverable SQL error into a `SIGABRT` that kills the user's
  whole `DuckDB` session.

  The crate already disagreed with itself: the scaffold has generated
  `panic = "unwind"` since the panic-safety work in this release, with a comment
  explaining why. `validate_release_profile` now requires `"unwind"` and rejects
  `"abort"` with that explanation; `ReleaseProfileCheck::panic_abort` is renamed
  `panic_unwind`. A new test asserts the scaffold and the validator agree, so
  they cannot drift apart again.

  The original justification — "panics across FFI boundaries are undefined
  behavior" — is also out of date: Rust defines an unwind escaping `extern "C"`
  as an abort, and quack-rs catches panics before the boundary regardless.

  quack-rs's own `[profile.release]` also said `panic = "abort"`. Cargo ignores a
  dependency's profile so it changed nothing downstream, but it contradicted the
  crate's own advice; it now says `"unwind"`.

#### A validator made legal function names unregisterable

- **`validate_function_name` rejected mixed-case names, and it gates
  `try_new`** — so `ScalarFunctionBuilder::try_new("myFunc")` returned `Err` and
  the function could not be registered through quack-rs at all. `DuckDB` itself
  ships `formatReadableSize` and `formatReadableDecimalSize`, and registering a
  camelCase name through the C API succeeds: verified against `DuckDB` 1.5.5,
  where the function is then callable as `formatReadableThing`,
  `formatreadablething` **and** `FORMATREADABLETHING`, because `DuckDB`
  identifiers are case-insensitive.

  The rule was justified as avoiding "catalog issues"; that test disproves it.
  Letters of either case are now accepted. Everything that would genuinely break
  is still rejected — a name needing quotes in SQL (`my-func`, `my func`,
  `my.func`), one starting with a digit, one over 256 characters, one with an
  interior NUL. `snake_case` remains the right convention and is documented as
  one, rather than enforced as a rule that blocks a legal name.

  The same relaxation applies to `AggregateFunctionBuilder`,
  `TableFunctionBuilder` and `SqlMacro` parameter names, which share the
  validator.

  A regression test now runs `validate_function_name` over **every** function in
  `duckdb_functions()` (746 of them) and `validate_extension_name` over every
  entry in `duckdb_extensions()`, asserting that everything identifier-shaped is
  accepted and every operator is not. That is how the defect was found.

#### A documented convention that was not being followed

- **"Every `unsafe` block inside this crate has a `// SAFETY:` comment" was not
  true.** `clippy::undocumented_unsafe_blocks` reports 180 blocks in the library.
  Most are inside an `unsafe fn` and merely forward that function's own
  documented contract — `unsafe_op_in_unsafe_fn` is denied crate-wide, so those
  blocks are required syntax rather than new assertions — but around forty were
  in **safe** functions, where the crate rather than the caller is asserting the
  invariant, and those had nothing.

  The claim is replaced with the convention actually worth following, and that
  convention is now met: every `unsafe` block in a safe function carries a
  `// SAFETY:` comment. Auditing them also turned up three comments that
  described the wrong thing — two `duckdb_free` calls and a `duckdb_destroy_value`
  annotated as if they were uses of the enclosing handle; those now say which
  allocation they own and why, cross-referencing `LESSONS.md` P11.

#### The scaffold generated a `description.yml` that would be rejected

- **`repo.ref` was generated as `main`.** `DuckDB`'s community-extension
  documentation is explicit: "Provide the hash of the latest commit on the
  branch targeting stable as `ref`". The repository builds exactly that revision
  and signs the result, so a branch makes the build unreproducible. Of the 43
  published extensions sampled, 41 pin a full 40-character hash and two pin a
  tag; **none** uses a branch.

  `ScaffoldConfig` gains `git_ref`, defaulting to `REF_PLACEHOLDER`
  (`"REPLACE_WITH_COMMIT_HASH"`) — deliberately not a valid revision, so it
  cannot be submitted by accident the way `main` silently could. The generated
  file carries a comment saying why, and a commented-out `ref_next`.

- **`DescriptionYml` silently dropped `repo.ref_next`.** It is a documented
  field: while a new `DuckDB` release is being prepared, the community
  repository tests an extension against both the latest stable release and
  `main`, and `ref_next` names the revision compatible with `main`. Now parsed
  into `git_ref_next`, empty when absent.

- **The generated `description.yml` had no `docs:` section.** All 43 published
  extensions have one — it is what renders on the community-extensions
  documentation site. The scaffold now emits `hello_world` and
  `extended_description` stubs.

#### Two more documented behaviours that were not the real ones

- **`ClientContext::catalog` documented an empty name as "the default
  catalog"; `DuckDB` rejects it outright.**
  `duckdb_client_context_get_catalog` starts with
  `if (!context || !name || strlen(name) == 0) return nullptr;` — an empty
  string is the one value guaranteed to fail. The catalog of an in-memory
  database is named `memory`; a file database's is the file's stem. The doc now
  says so, along with the other `None` case the C API imposes and quack-rs never
  mentioned: `DuckDB` checks `transaction.HasActiveTransaction()`, so this works
  inside a callback but not on an idle auto-commit connection. Both verified
  against 1.5.5 by a live test.

- **`ClientContext::config_option` aborts the process when asked for a setting
  that does not exist — on a `DuckDB` built with debug assertions.**
  `duckdb_client_context_get_config_option` calls
  `TryGetCurrentSetting(...).GetScope()` without first checking the lookup
  succeeded, and `GetScope()` asserts `scope != SettingScope::INVALID`. A
  release `DuckDB` compiles the assertion out and the function's own `default:`
  arm returns `NULL` as documented, so this never reproduces for end users and
  always reproduces in a test suite linking a debug `DuckDB`.

  This is a `DuckDB` defect, not a quack-rs one, but it makes the obvious
  "does the user have this setting?" probe unsafe. Documented on the method with
  the source lines, recorded as `LESSONS.md` P12, and the abort-free
  alternative given: `SELECT count(*) FROM duckdb_settings() WHERE name = ?`.

#### Documentation claimed a bridge that cannot exist

- **The `secrets` module described itself as bridging into `DuckDB`'s secrets
  system. There is no such bridge, and there cannot be.** The extension C API
  has **zero** secret functions — not one `duckdb_secret_*` among the 546 slots
  of `duckdb_ext_api_v1` in `DuckDB` 1.5.5. An extension cannot ask `DuckDB` for
  a credential through the C API at all.

  The only route is the `duckdb_secrets()` table function, and `DuckDB` redacts
  sensitive fields there. Verified against 1.5.5:

  ```text
  CREATE SECRET s (TYPE s3, KEY_ID 'AKIAEXAMPLE', SECRET 'super-secret-value');
  SELECT secret_string FROM duckdb_secrets();
  -- ...;key_id=AKIAEXAMPLE;secret=redacted
  ```

  The module docs now say this plainly, and say what `SecretsManager` actually
  is: a trait over the extension's **own** credential source, carrying the
  redacting `Debug`, zeroize-on-drop and absent `PartialEq` that credential
  handling needs, rather than a route to `DuckDB`'s store.

  The zeroize claim is also narrowed to what is true: it covers the buffers a
  `SecretEntry` owns, not a `String` the caller still holds or one a `String`
  abandoned when it grew.

#### The `description.yml` validator rejected 84% of real extensions

- **`parse_description_yml` rejected 36 of the 43 published community
  extensions it was tested against.** Its entire purpose is to tell an author
  their submission is valid before they open a PR, and it told almost everyone
  they were invalid. Four independent causes:

  1. **`requires_toolchains` was treated as required.** It is not — only 14 of
     the 43 set it, and the community-extensions documentation does not list it
     as required. This alone rejected half the corpus. It is now optional;
     `validate_rust_extension` still requires `rust` in it when present.

  2. **YAML quotes were not stripped.** `parse_kv` deliberately returned quoted
     values *with* their quotes and left stripping to each caller, and only
     `excluded_platforms` did. 12 of 43 files write `version: '2025120401'`, so
     the parser saw `'2025120401'` — quotes included — and every version check
     failed on it. `parse_kv` now unquotes, with a real balanced-quote check
     rather than `trim_matches`, which would also eat `""doubled""` and a
     trailing `a"`.

  3. **`validate_extension_version` imposed a format `DuckDB` does not.** It
     accepted only semver or a git hash; 11 of 43 published extensions use a
     date-based build id (`2025120401`). `DuckDB`'s community-extension
     documentation specifies no version format at all — it says the descriptor
     carries "the version of the extension" and points at existing extensions
     as examples. The check is now what would actually break something: empty,
     over 64 characters, or containing anything outside `[A-Za-z0-9._+-]`
     (whitespace, path separators, control characters).
     `classify_extension_version` is unchanged — `DuckDB`'s three-tier
     stability scheme *is* documented and *is* strict, and that function is
     where it belongs.

  4. **`windows_amd64_rtools` was rejected.** It is the R-tools Windows build
     (`DuckDBPlatform()` emits it under `DUCKDB_PLATFORM_RTOOLS`), it is not in
     the distribution matrix, and 14 of 43 published extensions exclude it.
     `DUCKDB_PLATFORMS` now also accepts it and the four group names
     (`linux`, `osx`, `wasm`, `windows` — the top-level keys of
     `distribution_matrix.json`), while the new `DUCKDB_CI_PLATFORMS` keeps
     the matrix-derived list the guard script checks. Empty segments from a
     trailing `;` — which five real files have — are skipped rather than
     reported as a platform named `""`.

  All 43 now parse, with every name matching its directory.

- **Prose in the `docs:` section was parsed as metadata.** The scan was flat, so
  a `version:` or `license:` line inside `docs.extended_description` — free-form
  prose in 42 of the 43 files — silently overwrote the extension's real values.
  Demonstrated: a `license: FAKE-LICENSE` line inside a documentation block made
  a valid file fail validation, and the same mechanism could have made an
  invalid one pass. The parser is now section-aware (only `extension:` and
  `repo:` are read) and understands block scalars: `key: |` and `key: >` bodies
  are captured as the field's value — literal blocks keeping line breaks, folded
  blocks joined — instead of being scanned for mappings.

- **Three doc examples showed indented YAML that was not indented.** A `\`
  line-continuation in a Rust string literal eats the following line's leading
  whitespace, so `description.yml` examples in `parse_description_yml`,
  `validate_description_yml_str` and `validate_rust_extension` were parsing
  fully-unindented text. They only passed because the parser ignored
  indentation; making it section-aware exposed them. Rewritten as real
  multi-line literals.

#### Validators were giving wrong answers

- **The DuckDB platform list was stale in both directions.**
  `validate::platform` rejected `linux_amd64_musl` and `linux_arm64_musl` —
  real, currently-built targets — so an extension that legitimately cannot
  support musl could not declare it. And it accepted `linux_amd64_gcc4`, which
  `DuckDB` retired: `DuckDBPlatform()` in `duckdb/common/platform.hpp` now
  raises a compile error for the legacy CXX ABI rather than emitting a `_gcc4`
  suffix, and it is absent from the distribution matrix. Excluding it was a
  silent no-op.

  The list is now derived from `config/distribution_matrix.json` in
  `duckdb/extension-ci-tools` — the file the community-extensions build actually
  reads — and `scripts/check-platform-table.py` plus a CI job fail when the two
  diverge. Adds `DUCKDB_OPT_IN_PLATFORMS` and `is_opt_in_platform`, because
  three of the twelve (`linux_amd64_musl`, `linux_arm64_musl`, `windows_arm64`)
  are only built on request, so excluding one of those is also a no-op.
  `linux_amd64_gcc4` gets a targeted error saying what happened to it, rather
  than "not a recognized DuckDB build target".

- **`validate_spdx_license` claimed valid licenses did not exist.**
  `COMMON_SPDX_LICENSES` is a 42-entry shortlist of a 733-entry registry, but
  the rejection message read "is not a recognized SPDX identifier" — false for
  `CC0-1.0`, `Python-2.0`, `BSD-4-Clause` and roughly 690 others. It now says
  the identifier is not on quack-rs's shortlist and points at the registry.

  Every entry was checked against `spdx/license-list-data`: all 42 are real and
  none are deprecated. `scripts/check-spdx-list.py` and a CI job keep it that
  way, and flag any newly-added identifier that is not OSI-approved (`SSPL-1.0`
  is listed and deliberately is not). The list is now sorted, with a test
  keeping it so. Also fixes the module doc, which called the field
  `extension.licence`; real `description.yml` files — and quack-rs's own parser
  — use `license`.

#### Silent data corruption

- **The `UUID` accessors disagreed about which 128 bits they meant, and the
  documentation said they agreed.** A `UUID` column is physically a `HUGEINT`,
  but `DuckDB` stores it with the **top bit flipped** so that signed integer
  ordering matches UUID string ordering (`BaseUUID::FromUHugeint` in
  `src/common/types/uuid.cpp` subtracts 2^63 from the upper half). So:

  | Accessor | Returned | For `'11111111-…'::UUID` |
  |----------|----------|---------------------------|
  | `VectorReader::read_uuid` (old) | raw storage | `0x9111…` |
  | `Value::as_uuid` | textual bits | `0x1111…` |

  Both were documented as "matching" the other. Handing one to the other — the
  obvious thing to do when a table function reads a `UUID` and builds a `Value`
  from it — silently changed the UUID's first hex digit.

  `read_uuid` / `write_uuid` (on `VectorReader`, `VectorWriter`, `StructReader`,
  `StructWriter` and both mocks) now apply the flip and take/return `u128`
  **textual bits**, the same convention as `Value::uuid` / `Value::as_uuid` and
  every Rust `Uuid` type. `Value::uuid` / `as_uuid` move from `i128` to `u128`
  for the same reason. The type change is deliberate: it turns a silent
  behaviour change into a compile error at every affected call site.

  `read_i128` / `write_i128` still read and write the raw storage, and the new
  `vector::uuid_from_storage` / `vector::uuid_to_storage` convert between the
  two. Pinned by a live test that asserts the raw storage and the textual bits
  really do differ, so the conversion cannot quietly become a no-op.

#### Wrong results and unloadable builds

- **`ChunkWriter` no longer hardcodes a 2048-row capacity.** `DuckDB` can be
  built with a different `STANDARD_VECTOR_SIZE`, which is exactly why the C API
  exposes `duckdb_vector_size()`; assuming 2048 against a smaller build overruns
  the output vectors. `ChunkWriter::new` now reads the running engine's value.
  `ChunkWriter::new` and `DataChunk::into_chunk_writer` are consequently no
  longer `const fn`.

- **The scaffold produced an extension `DuckDB` refuses to load.** The generated
  `Makefile` set `DUCKDB_PLATFORM_VERSION`, which `extension-ci-tools` does not
  read, alongside `USE_UNSTABLE_C_API=1`. `TARGET_DUCKDB_VERSION` therefore fell
  back to its `v0.0.1` default and the binary was stamped
  `C_STRUCT_UNSTABLE`/`v0.0.1`, which `DuckDB` rejects with *"The file was built
  specifically for DuckDB version 'v0.0.1'"*. The generated `Makefile` now sets
  `EXTENSION_NAME` (not `EXT_NAME`, which `base.Makefile` ignores),
  `TARGET_DUCKDB_VERSION` and `USE_UNSTABLE_C_API` from the new
  `ScaffoldConfig` fields, and defines the `all`/`configure`/`debug`/`release`/
  `test`/`clean` targets its own README and CI invoke.

- **The scaffold generated `panic = "abort"`**, which makes the `catch_unwind` in
  `scalar_callback!`, `table_scan_callback!` and the extension entry point inert
  — so any panic in extension code killed the whole `DuckDB` process instead of
  surfacing as a SQL error. Now generates `panic = "unwind"`.

- **The scaffold pinned `quack-rs = "0.13"`** regardless of the generating
  crate's version. It now tracks the current major.minor.

- **A freshly scaffolded project failed its own generated CI.** `cargo clippy
  --all-targets -- -D warnings` (which the generated workflow runs) rejected the
  generated `src/lib.rs` for `clippy::redundant_closure` and `src/wasm_lib.rs`
  for `special_module_name`. Both are fixed; a new `scaffold-e2e` CI job builds
  the generated project, stamps its metadata footer, loads it into a real
  `DuckDB`, asserts the query result, and runs the generated lint gate.

- **The generated CI referenced a nonexistent action** (`duckdb/duckdb-build@v1`)
  and ran `make test` without `make configure` / `make release`, so it could not
  have passed. Replaced with a workflow that configures, builds and tests through
  `extension-ci-tools`.

- **The extension entry point ran user registration code without
  `catch_unwind`.** A panic in a registration closure unwound to the
  `extern "C"` entry point, aborting the process; it now becomes a `LOAD` error.
  An `api_version` containing an interior NUL is also rejected up front instead
  of panicking inside `libduckdb-sys`.

#### Behaviour documented after verification

- `Value::display_string` renders a SQL **literal**, not display text:
  `Value::varchar("hello")` gives `'hello'` and `Value::date(0)` gives
  `'1970-01-01'::DATE`. Now documented with a table, since silently getting
  quotes and a cast suffix in a diagnostic is surprising.
- `Value::as_str` truncates at an interior NUL, because `duckdb_get_varchar`
  returns a NUL-terminated `char *`. `DuckDB` stores the full bytes; only this
  read path is limited. Documented on both `as_str` and `Value::varchar`, and
  pinned by a test.

#### Documentation

- **The crate documented an "architectural limitation" that does not exist.**
  `Cargo.toml`, `testing::in_memory_db` and the book all stated that
  `VectorReader`, `VectorWriter` and `Connection::register_*` "cannot be called
  in `cargo test`" because they route through the dispatch table. Opening an
  `InMemoryDb` populates that table for the whole process, after which the entire
  C API — registration included — works. The new `tests/ffi_roundtrip.rs`
  registers real scalar functions and round-trips every vector type through SQL:
  every integer width at its extremes, `HUGEINT`/`UHUGEINT` at theirs, floats and
  NaN, strings across the 12-byte inline/pointer boundary and multi-byte UTF-8,
  blobs containing NUL and non-UTF-8 bytes, all temporal types cross-checked
  against `DuckDB`'s own rendering, `UUID`, `INTERVAL`'s three fields, `DECIMAL`
  at all four physical widths, NULL in and out, multi-chunk scans, and a
  panicking callback surfacing as a SQL error.

- Documentation examples pinned `quack-rs = "0.13"`.

[`abi::check`]: https://docs.rs/quack-rs/latest/quack_rs/abi/fn.check.html
[`AbiPolicy`]: https://docs.rs/quack-rs/latest/quack_rs/abi/enum.AbiPolicy.html
[`AbiPolicy::Strict`]: https://docs.rs/quack-rs/latest/quack_rs/abi/enum.AbiPolicy.html

### Added

#### Live tests for every previously untested C API path

- **Copy functions and replacement scans had no live tests at all.** Between
  them they had 19 unit tests, none of which registered anything against a
  running `DuckDB` — which is how a heap-corrupting free survived in a shipped
  API. Both now have end-to-end coverage:

  - A `COPY ... TO 'f' (FORMAT my_format)` over 5000 rows, threading bind data
    and global state through all four lifecycle phases, asserting the sink saw
    every row and that both destructors ran exactly once (a leak or a double
    free is invisible without counting).
  - A replacement scan rewriting `SELECT * FROM '10.myfmt'` into a table
    function call, plus the decline path — an identifier the callback ignores
    must still reach `DuckDB`'s own error handling — and a panicking scan
    surfacing as a SQL error.

- **Six more modules had unit tests but no live registration**: scalar
  bind/init/local state, `Expression::fold`, catalog lookup, config options,
  selection vectors and the instance cache. All now run against a real `DuckDB`,
  which turned up two more documentation defects (below) and confirmed the rest.

- **`copy_bind_callback!`, `copy_global_init_callback!`, `copy_sink_callback!`
  and `copy_finalize_callback!`.** Every other callback kind had a panic-safe
  macro; the four copy-function phases did not, so a panic in one of them had
  nothing to catch it. Each routes the message through that phase's own
  `duckdb_copy_function_*_set_error`.

- `TypeId::try_from_duckdb_type` — returns `Option<TypeId>` instead of panicking
  on a type value this build does not know. Extensions routinely meet these: a
  column of a type added in a newer `DuckDB`, or a 1.5.x type reaching a build
  without `duckdb-1-5`. `from_duckdb_type` still panics and now documents that
  callbacks should not use it.
- Fallible `LogicalType` constructors that previously panicked on an interior NUL
  in a caller-supplied name: `try_struct_type_from_logical`, `try_union_type`,
  `try_union_type_from_logical`, `try_enum_type`, `try_set_alias`.
- `entry_point!` / `entry_point_v2!` accept an optional [`AbiPolicy`] as their
  second argument; `init_extension_with_policy` /
  `init_extension_v2_with_policy` are the function-level equivalents.
- `examples/scaffold_to_dir.rs` — writes a scaffolded project to disk, used by
  the new `scaffold-e2e` CI job.

#### Panic safety

- **A panic-safe wrapper macro for every callback kind.** Only `scalar_callback!`
  and `table_scan_callback!` existed, so the other six kinds — table bind, table
  init, aggregate update/combine/finalize/destroy, cast, and replacement scan —
  were unguarded, and a panic in any of them aborted the `DuckDB` process. The
  aggregate ones are the worst case: they run on worker threads, so the abort
  comes from a thread the user never sees. New macros: `table_bind_callback!`,
  `table_init_callback!`, `aggregate_update_callback!`,
  `aggregate_combine_callback!`, `aggregate_finalize_callback!`,
  `aggregate_destroy_callback!`, `cast_callback!`, `replacement_scan_callback!`.
  Each routes the panic message to that callback kind's own `set_error`;
  `cast_callback!` also returns `false` so `TRY_CAST` yields NULL. The aggregate
  destructor has no error channel in the C API, so its panic is caught and
  dropped — leaking beats aborting during query teardown. Verified end-to-end:
  a panicking aggregate `update` and a panicking cast both surface as SQL errors
  and leave the connection usable.

- The two existing macros now share `callback::panic_message` and
  `callback::message_to_c_string` with the new ones. The latter replaces an
  interior NUL rather than dropping the diagnostic, which the old
  `if let Ok(c_msg) = CString::new(msg)` silently did.

- `TypedTableFunctionBuilder` reported every panic as the same fixed string.
  It now includes the payload, so the user learns *which* assertion failed.

- **Deprecated `FfiBindData::get_from_bind`**, which always returned `None` and
  always will: `DuckDB` exposes no `duckdb_bind_get_bind_data`. Being safe and
  returning `Option`, it silently sent `if let Some(..)` down the wrong branch.

#### Capabilities

- **`ListBuilder` for `LIST` and `MAP` output vectors.**
  `duckdb_list_vector_reserve` takes a *total* capacity and reallocates the child
  vector when it grows, so a `VectorWriter` obtained beforehand is left dangling.
  That makes the natural "reserve as you go, keep one writer" loop a
  use-after-free. `ListBuilder` re-fetches the child writer after every reserve,
  tracks the running offset, writes each parent `{offset, length}` entry, and
  grows geometrically so building a list is not quadratic. `push_map_row` does
  the same for `MAP`. It also refuses capacities above
  `MAX_LIST_CHILD_CAPACITY` (`duckdb::DConstants::MAX_VECTOR_SIZE`), above which
  `DuckDB` throws a C++ exception that its own C API does not catch — an
  exception unwinding into Rust would be undefined behaviour. Covered by tests
  building 2000 lists and 1500 maps of varying length through real SQL.

- **`Value` gained the extractors and constructors it was missing.** A table
  function declared with a `TIMESTAMP` or `LIST` parameter handed the bind
  callback a `duckdb_value` that could only be read via `as_str()` and reparsed.
  Adds `as_date`, `as_time`, `as_time_tz`, `as_timestamp`, `as_timestamp_tz`,
  `as_timestamp_s/ms/ns`, `as_interval`, `as_uuid`, `as_decimal`, `as_u128`,
  `list_len` / `list_child` / `list_items`, `struct_child`, `map_len` /
  `map_key` / `map_value`, and the constructors `boolean`, `bigint`, `double`,
  `date`, `timestamp`, `varchar`, `uuid`, `null_value`.

- **`query` module — running SQL from inside an extension.** The C API has
  everything needed (`duckdb_query`, `duckdb_prepare`, `duckdb_bind_*`,
  `duckdb_fetch_chunk`) and it is all in the stable prefix, but each handle has a
  `destroy` that must run exactly once, including on error paths. `QueryResult`,
  `OwnedDataChunk`, `PreparedStatement` and `OwnedConnection` are RAII wrappers
  for those; `Connection` gains `query`, `execute`, `prepare` and
  `open_connection`.

  `OwnedConnection` covers the case the borrowed registration connection cannot:
  a `duckdb_connection` holds its own reference to the database instance, so one
  opened during load stays valid afterwards — for a callback or a background
  thread. Verified by a test that closes the `duckdb_database` handle and keeps
  querying.

- **`datetime` module — calendar conversions.** `DATE`, `TIME` and `TIMESTAMP`
  move through vectors as raw integers; turning those into year/month/day meant
  reimplementing the proleptic Gregorian calendar and `DuckDB`'s infinity
  sentinels. `DuckDB` already exposes the conversions in the stable API, so this
  wraps them: `date_from_days`/`date_to_days`, `time_from_micros`/`time_to_micros`,
  `timestamp_from_micros`/`timestamp_to_micros`, `time_tz_bits`/`time_tz_from_bits`,
  the four `is_finite_*` predicates, and `HUGEINT`/`UHUGEINT`/`DECIMAL` ↔ `f64`.

  Also exports the exact sentinel values as constants. `-infinity` is `-i32::MAX`
  / `-i64::MAX`, **not** `i32::MIN` / `i64::MIN` — `i32::MIN` is an ordinary
  finite date, and treating it as infinity would silently drop real rows.

- **`VectorWriter` caches its validity bitmap.** `set_null` called
  `duckdb_vector_ensure_validity_writable` + `duckdb_vector_get_validity` on
  every row; both are now resolved once per vector (2 FFI calls instead of 4096
  for an all-NULL 2048-row vector). Adds `set_null_range` for the batched case.

- **Vector accessors for the remaining physical layouts**:
  `write_u128`/`read_u128` (`UHUGEINT`), `write_decimal`/`read_decimal` (which
  select `i16`/`i32`/`i64`/`i128` from the declared width the way `DuckDB` does),
  `write_time_tz`/`read_time_tz`, and `TIMESTAMPTZ` / `TIMESTAMP_S` /
  `TIMESTAMP_MS` / `TIMESTAMP_NS` accessors. `VectorReader::contains` bounds-checks
  an index against the row count.

- Callback signature aliases are re-exported at their module roots:
  `scalar::ScalarFn` (plus `ScalarBindFn` / `ScalarInitFn` under `duckdb-1-5`) and
  `aggregate::{StateSizeFn, StateInitFn, UpdateFn, CombineFn, FinalizeFn, DestroyFn}`,
  matching what `table` already did.

- The prelude re-exports `AbiPolicy`, the `datetime` types and the `query` types.

- **`Registrar::register_config_option`** — the trait already covered scalar,
  scalar set, aggregate, aggregate set, table, SQL macro, cast and copy
  functions, but not config options, so an extension registering one could not
  have its whole registration closure exercised through `MockRegistrar`. Added,
  with `config_option_names` / `has_config_option` on the mock.

- **`secrets::list_duckdb_secrets`** — reads the secret *metadata* `DuckDB` does
  expose, via `duckdb_secrets()`: name, type, provider, persistence, storage,
  scope prefixes and the redacted `secret_string`. Enough to pick a scope, warn
  that a required secret is missing, or choose a provider. It returns a
  `DuckDbSecretInfo`, deliberately not a `SecretEntry`, so nothing suggests it
  carries credentials. A live test asserts both halves: the metadata comes
  through, and the credential provably does not.

- **The appender is no longer behind `duckdb-1-5`, and gained the row-at-a-time
  API it never had.** `duckdb_appender_*` occupies slots 281–291 and 330–356 —
  the *frozen stable prefix*, unchanged since v1.2.0 — yet the whole module was
  gated on `duckdb-1-5`, whose wrappers live in the unstable region. Using the
  appender therefore forced an extension onto the version-pinned unstable ABI,
  for functionality that has been portable for four minor releases. Only three
  methods actually need 1.5 and stay gated: `error_data`, `clear` and
  `append_default_to_chunk`.

  The 24 row-at-a-time functions were wrapped for the first time:
  `append_bool` / `_i8` / `_i16` / `_i32` / `_i64` / `_i128` / `_u8` / `_u16` /
  `_u32` / `_u64` / `_u128` / `_f32` / `_f64` / `_str` / `_bytes` / `_date` /
  `_time` / `_timestamp` / `_interval` / `_value` / `_null` / `_default`,
  `end_row`, `column_count`, `column_type`, `add_column`, `clear_columns`, and a
  `row(|row| …)` helper that calls `end_row` for you. Previously the only way to
  insert a row was to build a whole `DataChunk`.

  Three details that are easy to get wrong and are handled here: `append_str`
  uses `duckdb_append_varchar_length`, so interior NUL bytes survive; that
  function narrows its length to `uint32_t` with an unchecked cast in `DuckDB`'s
  release builds, so longer strings are refused rather than truncated; and
  `duckdb_append_value` dereferences its argument with no null check, so a null
  `Value` handle is refused. Covered by live tests that append every scalar type
  at its extremes, 5000 rows across several vectors, a short row, a constraint
  violation surfacing at `close`, and a `DEFAULT`-filled column subset.

  New `appender::AppendError` is `ErrorData` with `duckdb-1-5` and
  `ExtensionError` without, so enabling the feature upgrades the error type in
  place without changing any method's shape — existing `duckdb-1-5` code is
  unaffected.

- **`table_description` is no longer behind `duckdb-1-5` either.** Slots 292–297
  are stable; only `column_count` and `column_type` are 1.5 additions and stay
  gated. Adds `TableDescription::with_catalog` (`duckdb_table_description_create_ext`,
  for tables in another catalog) and `column_has_default`
  (`duckdb_column_has_default`) — the latter being the only way to know whether
  `Appender::append_default` will succeed.

- **`FileHandle` gained the looping I/O helpers, and `size`/`tell` became
  fallible.** `duckdb_file_handle_read` and `duckdb_file_handle_write` return
  "the number of bytes **actually** read/written" — a single call can come up
  short, which over `httpfs` is routine rather than theoretical. Adds
  `read_exact`, `read_to_end` and `write_all`, which loop. `size()` and `tell()`
  changed from `i64` to `Result<u64, ErrorData>`: the C API signals failure with
  a *negative* return, and the previous signature made `handle.size().max(0) as
  usize` — silently treating an error as an empty file — the obvious thing to
  write. It was in this crate's own documentation.

- **`Value::type_id()`.** `Value` had forty `as_*` accessors and no way to ask
  what the value actually is, so reading a `VARCHAR` with `as_i64()` returned
  garbage rather than an error. Wraps `duckdb_get_value_type` (stable prefix,
  slot 137, unchanged since v1.2.0), returning `None` for a null handle or a
  type id newer than this build knows.

- **Every public type implements `Debug`.** 58 of them did not, which is Rust API
  guideline [C-DEBUG] and not cosmetic: `Result::unwrap`, `Result::expect_err`,
  `assert_eq!`, and `#[derive(Debug)]` on any downstream struct storing a
  quack-rs type all fail to compile without it. `LogicalType` and `Value` print
  decoded state (type id, alias, `DECIMAL` width/scale, `DuckDB`'s own rendering)
  rather than a pointer; builders print `set`/`unset` per callback, which is the
  question you have when `register` reports a missing function;
  `WarningCollector` uses `try_lock` so printing can neither block nor deadlock.
  `missing_debug_implementations` is now enabled crate-wide, and CI's
  `-D warnings` makes it an error. `testing::InMemoryDb` was a 59th, only
  visible once the lint ran with `bundled-test` on.

[C-DEBUG]: https://rust-lang.github.io/api-guidelines/debugging.html

### Changed

#### MSRV

- **MSRV lowered 1.87.0 → 1.86.0.** DuckDB's reusable
  `_extension_distribution.yml` — the workflow the community-extensions
  repository builds every extension with — pins
  `dtolnay/rust-toolchain@… # 1.86.0` for the WebAssembly job. quack-rs required
  1.87.0, so Cargo refused, and **no quack-rs extension could be built for
  `wasm_mvp` / `wasm_eh` / `wasm_threads`** by the official pipeline — despite
  the crate advertising `wasm32-unknown-emscripten` support since 0.14.0.

  The entire 1.87 requirement was five `const fn` accessors calling `Vec::len`
  (stabilised as const in 1.87). None can be reached in a const context —
  `MockVectorWriter`, `StructReader` and `StructWriter` are all built at runtime
  — so dropping `const` costs nothing. 1.86.0 is now the floor for the library,
  its dev-dependencies (`criterion` needs 1.86) and the `hello-ext` example, all
  verified.

  New `scripts/check-msrv-vs-duckdb-ci.py` and a CI job re-derive DuckDB's pinned
  toolchains from that workflow and fail if the MSRV creeps back above them.

- **Breaking:** `ScaffoldConfig` gains `target_duckdb_version` and
  `use_unstable_c_api`. `ScaffoldConfig` now implements `Default`, so existing
  struct literals can add `..ScaffoldConfig::default()`. `generate_scaffold`
  rejects combinations that produce an unloadable binary — a `C_STRUCT` build
  claiming a `DuckDB` release as its `-dv`, or a `C_STRUCT_UNSTABLE` build
  claiming the C API version.

### CI / tooling

- New `abi-table` job: `scripts/check-abi-table.py` verifies `src/abi.rs`'s
  layout table against every upstream `DuckDB` release header.
- New `abi-guard` job: builds an extension against `DuckDB` 1.5.0's header
  layout, stamps it `C_STRUCT`, and asserts the load is refused with a layout
  diagnostic — a regression test for the corruption described above.
- New `scaffold-e2e` job (see above).
- `extension-load` now stamps a real metadata footer and asserts query *results*
  rather than grepping the log for the word "error"; loading a bare `.so`
  bypassed `DuckDB`'s metadata validation entirely.

## [0.15.0] — 2026-07-16

### Added

- `Value::as_blob()` for copying arbitrary binary data from a `duckdb_value`.
  (Thanks @adonm.)

### Fixed

- `VectorReader::read_blob()` now preserves non-UTF-8 bytes instead of returning
  an empty slice. (Thanks @adonm.)

### Changed

- Dev/CI DuckDB bumped to **1.5.4** — `libduckdb-sys` / `duckdb` 1.10503.1 →
  1.10504.0 in the root lockfile, the `hello-ext` example lockfile, and the
  `bundled-test-prebuilt` CI download (`v1.5.3` → `v1.5.4`). 1.5.4 is a bugfix
  release in the 1.5.x line; its C extension API version is unchanged (`v1.2.0`,
  verified from `duckdb_extension.h`), so `DUCKDB_API_VERSION` is unchanged and
  the public `libduckdb-sys` dependency range (`>=1.4.4, <2`) is untouched —
  downstream consumers are unaffected.

### Security

- **`crossbeam-epoch` 0.9.18 → 0.9.20** (root lockfile), resolving
  **RUSTSEC-2026-0204** (invalid pointer dereference in the `fmt::Pointer`
  impl). Reaches the tree only as a dev-dependency via `criterion → rayon →
  crossbeam-deque`.
- **`quinn-proto` 0.11.14 → 0.11.15** (root and example lockfiles), resolving
  **RUSTSEC-2026-0185** (CVSS 7.5). Reaches the lockfiles via
  `libduckdb-sys → reqwest → quinn` (feature-union only; the loadable-extension
  build never links it).

### CI / tooling

- Refreshed SHA-pinned GitHub Actions via Dependabot: `actions/checkout`
  v6.0.2 → v7.0.0, `codecov/codecov-action` v6.0.1 → v7.0.0, `actions/cache`
  v5.0.5 → v6.1.0, and `actions/attest-build-provenance` v4.1.0 → v4.1.1. Also
  bumped the `cc` build-dependency 1.2.63 → 1.2.64.

## [0.14.0] — 2026-06-07

### Added

- **`wasm32-unknown-emscripten` support** (the DuckDB-WASM target). The crate no
  longer hard-rejects non-64-bit targets with a top-level `compile_error!`, and
  the `duckdb_string_t` pointer slot is read as a `u64` then narrowed to `usize`
  — lossless on 64-bit, and on wasm32 it yields the low 4 bytes of the 8-byte
  slot (the upper 4 are zero padding in DuckDB's 16-byte layout). The full public
  API, including the `duckdb-1-5-3` surface, `cargo check`s for
  `wasm32-unknown-emscripten`; CI now guards this. (Thanks @killzoner.)
- **`bundled-test-prebuilt` feature** — links a *pre-built* libduckdb instead of
  compiling DuckDB from C++ source, for a much faster test build. Supply the
  library via `DUCKDB_DOWNLOAD_LIB=1` (`libduckdb-sys` downloads the upstream
  release zip) or `DUCKDB_LIB_DIR=...` (a libduckdb tree you already have).
  `bundled-test` continues to compile DuckDB from source. (Thanks @killzoner.)
- `InMemoryDb::open_unsigned()` opens an in-memory database with
  `allow_unsigned_extensions=true`, allowing downstream extension crates to
  `LOAD` their own locally-built (unsigned) `.duckdb_extension` artifact for
  integration testing. (Thanks @killzoner.)

### Changed

- `duckdb` is now a purely optional dependency, activated only by `bundled-test`
  / `bundled-test-prebuilt`. It is no longer a dev-dependency, and there is no
  default `bundled` feature. As a result, a plain `cargo test` — and every
  downstream consumer's `Cargo.lock` — no longer pulls the DuckDB + arrow tree,
  and the default test build no longer compiles DuckDB.

### Security

- **`tar` 0.4.45 → 0.4.46** in both the root and example lockfiles, resolving
  **GHSA-3pv8-6f4r-ffg2** ("PAX header desynchronization", Moderate). `tar` is a
  `libduckdb-sys` build-dependency, so it appears in both `Cargo.lock` files and
  raised one Dependabot alert each — the two moderate alerts reported on `main`.
  This advisory is published in the GitHub Advisory Database (GHSA) but not the
  RustSec database, so `cargo deny` did not flag it; the new OSV scan below closes
  that gap.
- Bumped `cc` 1.2.62 → 1.2.63 (which moves `shlex` 1.3.0 → 2.0.1) and refreshed
  the `codecov/codecov-action` pin to v6.0.1.

### CI / tooling

- Added an **OSV / GHSA advisory scan** to CI (`osv-scanner`, pinned to v2.3.8 via
  a checksum-verified binary) covering both `Cargo.lock` files. `cargo deny`
  consults only the RustSec database; OSV.dev aggregates GHSA **and** RustSec, so
  GHSA-only advisories (such as the `tar` one above) now fail CI alongside the
  existing cargo-deny gate.

## [0.13.0] — 2026-05-24

### Added

New safe wrappers for the `DuckDB` 1.5.0+ C extension API, all gated behind the
`duckdb-1-5` feature, plus a new `duckdb-1-5-3` feature that surfaces the two
DuckDB 1.5.3 type-enum values. DuckDB 1.5.3's C extension *function-pointer* API
(version `v1.2.0`) is unchanged from 1.5.2; the one new C addition — the
`DUCKDB_TYPE_VARIANT` (41) type-enum value — is now exposed as `TypeId::Variant`
behind the `duckdb-1-5-3` feature (see below). So the additions below mostly
expose 1.5.x capabilities the SDK had not previously wrapped rather than anything
new to 1.5.3 specifically.

- **`error_data` module** — `ErrorData`, an RAII wrapper over
  `duckdb_error_data` (the structured error type returned by several 1.5 APIs).
  Carries a `DuckDbErrorType` category and a message, and converts into
  `ExtensionError`. Adds the free function `check_valid_utf8`, exposing
  `DuckDB`'s own UTF-8 validator.
- **`expression` module** — `Expression`, an RAII wrapper over
  `duckdb_expression`, with `return_type`, `is_foldable`, and `fold`. This
  closes a real gap: `ScalarBindInfo` already returned a raw, unusable
  `duckdb_expression` from `get_argument`; the new `ScalarBindInfo::argument`
  returns a safe `Expression`, so bind callbacks can inspect argument types and
  pre-fold constant arguments once at bind time.
- **`file_system` module** — `FileSystem`, `FileHandle`, `FileOpenOptions`, and
  `FileFlag`: read and write files through `DuckDB`'s virtual file system
  (honouring `httpfs`, in-memory files, and other registered file systems)
  instead of reaching for `std::fs`.
- **`appender` module** — `Appender`: bulk row insertion (create, append a
  `DataChunk`, flush, close) plus the 1.5 additions `clear` (revert buffered
  rows), `error_data` (structured errors), and `append_default_to_chunk`.
- **`selection_vector` module** — `SelectionVector`: allocate and fill
  zero-copy row-index selection vectors.
- **`instance_cache` module** — `InstanceCache`: share one underlying database
  instance across repeated opens of the same path.
- **`Value`** gains `display_string` (canonical string rendering of any value,
  via `duckdb_value_to_string`) and `TIME_NS` accessors `Value::time_ns` /
  `Value::as_time_ns` (pairing with the existing `TypeId::TimeNs`).
- **`Catalog`** gains `type_name` (the catalog's storage type, e.g. `"duckdb"`
  or a storage extension's name).
- All new public types are re-exported from the `prelude` behind the
  `duckdb-1-5` feature.
- **`duckdb-1-5-3` feature + `TypeId::Variant` / `TypeId::Geometry`** — a new
  feature flag (`duckdb-1-5-3`, which implies `duckdb-1-5`) exposes the
  `DUCKDB_TYPE_VARIANT` (41, added in DuckDB 1.5.3) and `DUCKDB_TYPE_GEOMETRY`
  (40) type-enum values as `TypeId::Variant` and `TypeId::Geometry`, with full
  `to_duckdb_type` / `from_duckdb_type` / `sql_name` / `Display` coverage. It is a
  separate gate because these constants postdate the `duckdb-1-5` feature's 1.5.0
  floor and require `libduckdb-sys >= 1.10503.1`; keeping them out of `duckdb-1-5`
  preserves compatibility for consumers pinned to libduckdb-sys 1.5.0–1.5.2.
- **`ErrorData` is now a first-class error type** — implements
  `std::fmt::Display` and `std::error::Error`, gains a structured `Debug` impl,
  and converts into `ExtensionError` via `From` (alongside the existing
  `into_extension_error`) so it propagates through `?`. `DuckDbErrorType` now
  implements `Display` (backed by a new `pub const fn as_str`).
- **`TableDescription::as_raw()`** — exposes the raw handle, matching the
  accessor convention of the other 1.5 wrappers.

### Changed

- **`duckdb` / `libduckdb-sys` 1.10502.0 → 1.10503.1** (DuckDB 1.5.2 → 1.5.3) in
  both the workspace and `examples/hello-ext` `Cargo.lock`. DuckDB 1.5.3 is a
  bugfix release ([announcement](https://duckdb.org/2026/05/20/announcing-duckdb-153));
  since the `>=1.4.4, <2` constraint already permitted it, the bundled fixes are
  picked up purely by the lock-file update with no source changes required for
  the bump itself.
- **`cc` → 1.2.62** in both `Cargo.lock` files — workspace (1.2.61 → 1.2.62,
  folding in Dependabot PR #89, the `patch-updates` group) and
  `examples/hello-ext` (1.2.57 → 1.2.62, re-syncing the example lock's older
  `cc`). Build-dependency; no API impact.
- **MSRV corrected to 1.87.0.** The crate declared `rust-version = "1.84.1"`,
  but `libduckdb-sys` (1.5.x line, a non-optional dependency) is
  `edition = "2024"` / `rust-version = "1.85.1"` — so quack-rs has in fact
  required Rust ≥ 1.85.1 since before this release (`cargo +1.84.1 check` cannot
  even parse the manifest). The declared MSRV, the CI `MSRV` job (now explicitly
  pinned with `toolchain: "1.87.0"` so it genuinely gates instead of silently
  falling back to the `rust-toolchain.toml` stable channel), the release matrix,
  and all docs/badges are updated to **1.87.0** — a small headroom margin above
  the 1.85.1 floor.

### Fixed

- **`TypeId::from_duckdb_type` no longer panics on the `duckdb-1-5` type-enum
  values.** It previously recognised only the base (1.4) values and `panic!`ed on
  everything else — including the `duckdb-1-5` values (`TIME_NS`, `ANY`,
  `BIGNUM`/`VARINT`, `SQLNULL`, `INTEGER_LITERAL`, `STRING_LITERAL`). Because the
  public `LogicalType::get_type_id()` calls it, inspecting such a type inside a
  bind callback could panic across the FFI boundary (Pitfall L3). It now maps
  every variant available in the active feature set (plus the `duckdb-1-5-3`
  `GEOMETRY` / `VARIANT` values when that feature is enabled).
- **`TableDescription`'s `Drop` now null-checks the handle** before destroying
  it, matching every other RAII wrapper in the crate.

### Documentation

- **New book section "DuckDB 1.5+ APIs"** — dedicated guide pages for the
  `error_data`, `expression`, `appender`, `file_system`, `selection_vector`, and
  `instance_cache` modules, wired into `SUMMARY.md`.
- Refreshed the reference docs (`docs/architecture.md`, `docs/ffi-reference.md`,
  the `TypeId` reference, `CONTRIBUTING.md`/book source trees) to cover the new
  modules, and updated the VARIANT/GEOMETRY entries in `Known Limitations`,
  `concepts/types.md`, and the `TypeId` reference to document the new
  `duckdb-1-5-3` gate (previously tracked as a follow-up).
- Added `// SAFETY:` comments to previously-undocumented `unsafe` blocks in the
  `get_client_context` accessors (`scalar`, `copy_function`) and
  `TableDescription::create`, and SPDX headers to `benches/interval_bench.rs` and
  the test submodule files.
- Corrected the README install note (it claimed v0.11.0 was the latest published
  crate; v0.12.1 was in fact already on crates.io) and bumped install-example
  version references throughout the README, book, and scaffold template to `0.13`.

### CI

- **docs.rs now builds with `duckdb-1-5-3`** (`[package.metadata.docs.rs]`), so
  the feature-gated modules and new `TypeId` variants render on docs.rs and the
  README's docs.rs links resolve (previously docs.rs built the empty default
  feature set and omitted them).
- **CI exercises the `duckdb-1-5-3` feature** — `check` / `test` / `clippy` for
  `duckdb-1-5-3` alongside `duckdb-1-5`, with the `Clippy (beta)` and `doc` jobs
  on `duckdb-1-5-3`.
- **Fixed the `Nightly` CI job silently running stable** (the SHA-pinned
  `dtolnay/rust-toolchain` step lacked `with: toolchain: nightly`).
- **Mutation testing scoped to testable code** — DuckDB FFI-wrapper modules
  whose methods require a live runtime (tests `bundled-test`-gated or absent) are
  excluded from `cargo mutants`, since their mutants can't be killed by unit
  tests. Extends the existing exclusion pattern to the 1.5.x wrappers
  (`expression`, `file_system`, `appender`, `selection_vector`, `instance_cache`,
  `table_description`, and the scalar/copy `*Info` accessors). Pure-logic code
  (e.g. `DuckDbErrorType`, `TypeId` conversions) stays in scope; the mutants
  feature set is bumped to `duckdb-1-5-3`.

## [0.12.1] — 2026-05-01

### Security

Closes nine GitHub Dependabot alerts (two High, seven Low) split across
the workspace `Cargo.lock` and `examples/hello-ext/Cargo.lock`.

- **`rustls-webpki` 0.103.10 → 0.103.13** picks up fixes for three
  RustSec advisories reachable via the `bundled` DuckDB build's transitive
  `reqwest` → `rustls` chain: [RUSTSEC-2026-0098] (URI name constraints
  silently ignored), [RUSTSEC-2026-0103] (wildcard name constraints
  accepted), [RUSTSEC-2026-0104] (DoS panic on malformed CRL `BIT STRING`).
  None of these paths are exercised by `quack-rs` itself, but the
  advisories trip `cargo deny` for downstream consumers, so the patch
  bump removes friction.
- **`rand` 0.9.2 → 0.9.4 / 0.8.5 → 0.8.6** picks up the fix for
  [RUSTSEC-2026-0097] (`ThreadRng` Stacked-Borrows UB when a custom
  global logger reentered `rand::rng()` during reseed). Patched on
  every line: 0.8.6+, 0.9.3+, 0.10.1+.

### Changed

- Workspace lockfile: `cc` 1.2.59 → 1.2.61 (build-dep), `duckdb` /
  `libduckdb-sys` 1.10501.0 → 1.10502.0, `rand` 0.8.5 → 0.8.6,
  `rand` 0.9.2 → 0.9.4.
- `examples/hello-ext` lockfile: `libduckdb-sys` 1.10501.0 → 1.10502.0,
  `rand` 0.9.2 → 0.9.4, `rustls-webpki` 0.103.10 → 0.103.13.

### CI

- GitHub Actions pin updates: `actions/cache` `v5.0.4` → `v5.0.5`,
  `actions/upload-artifact` `v7.0.0` → `v7.0.1`,
  `actions/upload-pages-artifact` `v4.0.0` → `v5.0.0` (all SHA-pinned).
- New informational `Clippy (beta)` job runs the same clippy invocation
  on the `beta` toolchain (`continue-on-error`), so lint promotions
  surface ~6 weeks before they reach `stable`.

### Fixed

- `WarningCollector::len`: rewrite `map(|w| w.len()).unwrap_or(0)` as
  `map_or(0, |w| w.len())` to satisfy `clippy::map_unwrap_or`, which
  graduated to `stable` clippy in Rust 1.95.0.
- `WarningCollector::snapshot`: same defensive rewrite for the sibling
  `map(|w| w.clone()).unwrap_or_default()` call site.

[RUSTSEC-2026-0097]: https://rustsec.org/advisories/RUSTSEC-2026-0097
[RUSTSEC-2026-0098]: https://rustsec.org/advisories/RUSTSEC-2026-0098
[RUSTSEC-2026-0103]: https://rustsec.org/advisories/RUSTSEC-2026-0103
[RUSTSEC-2026-0104]: https://rustsec.org/advisories/RUSTSEC-2026-0104

## [0.12.0] — 2026-04-09

### Added

- **`TypedTableFunctionBuilder<S>` — closure-based table functions with typed scan state**
    - Entry point: `TableFunctionBuilder::with_state::<S, _>(|bind| Ok(S { ... })).scan(|state, chunk| { ... Ok(()) }).build()?`
    - `bind` closure: `&BindInfo -> Result<S, ExtensionError>` — declares output schema, reads parameters, returns the initial scan state
    - `scan` closure: `&mut S, &DataChunk -> Result<(), ExtensionError>` — writes rows; set chunk size to zero to signal end-of-stream
    - Eliminates hand-rolled `unsafe extern "C" fn` bind/init/scan trampolines in FFI-heavy extensions
    - Panics in user closures are caught via `catch_unwind` and reported through `duckdb_*_set_error`
    - `S: Send + 'static`; scans are serialised (`set_max_threads(1)`) — use the raw builder + `local_init` for parallel scans
    - Re-exported from `quack_rs::prelude`
- **`ExtensionError` ergonomics** — `From<std::io::Error>`, `From<std::ffi::NulError>`, `From<std::fmt::Error>` for direct `?` operator usage in `register_all()`
- **`tls` module** — `TlsConfigProvider` trait for type-erased TLS client configuration injection (no external deps)
- **`warning` module** — `ExtensionWarning`, `WarningSeverity`, `WarningCollector` for structured security warnings with CWE codes
- **`secrets` module** — `SecretsManager` trait and `SecretEntry` for bridging DuckDB's native `CREATE SECRET` storage
- **`StructWriter::child_list_vector()`** — semantic alias for LIST-typed struct fields
- **Prelude additions** — `TlsConfigProvider`, `ExtensionWarning`, `WarningSeverity`, `WarningCollector`, `SecretEntry`, `SecretsManager`

## [0.11.0] — 2026-03-30

### Added

- **`StructWriter::child_vector()`** / **`StructReader::child_vector()`** — raw child vector access for nested complex types (LIST, MAP, ARRAY) inside STRUCT fields
- **`ChunkWriter::vector()`** — raw vector access for complex column types
- **`ChunkWriter::column_count()`** — column count without needing `DataChunk`
- **`VectorWriter::set_valid()`** / **`StructWriter::set_valid()`** — undo `set_null()`, mark row as non-NULL
- **`ReplacementScanInfo::add_parameter_raw()`** — non-VARCHAR replacement scan parameters
- **`ReplacementScanInfo::add_i64_parameter()`** / **`add_bool_parameter()`** — typed convenience methods

### Changed

- **`table_scan_callback!`** now reports panic messages to DuckDB via `duckdb_function_set_error` (previously silent)

## [0.10.0] — 2026-03-29

### Added

- **`StructWriter`** — batched typed writer for STRUCT output vectors; eliminates repeated `duckdb_struct_vector_get_child` calls
- **`StructReader`** — batched typed reader for STRUCT input vectors; read-side counterpart to `StructWriter`
- **`ChunkWriter`** — auto-sizing chunk writer for scan callbacks; calls `set_size` on `Drop`
- **`scalar_callback!` / `table_scan_callback!`** macros — panic-safe `extern "C"` callback wrappers using `catch_unwind`
- **`Value` integer extraction** — `as_i8()`, `as_i16()`, `as_u8()`, `as_u16()`, `as_u32()`, `as_u64()`, `as_i128()` + null-safe `_or(default)` variants for all types
- **Temporal/binary vector methods** — `read_date/write_date`, `read_timestamp/write_timestamp`, `read_time/write_time`, `read_blob/write_blob`, `read_uuid/write_uuid` on `VectorReader`/`VectorWriter`/`StructReader`/`StructWriter`
- **`DataChunk` bridges** — `struct_writer()`, `struct_reader()`, `struct_field_reader()`, `into_chunk_writer()`
- **Mock type completeness** — 8 missing `try_get_*` methods, 10 missing `from_*` constructors, `Blob` variant, uuid/date/timestamp/time aliases
- **Prelude** — `StructReader`, `StructWriter`, `ChunkWriter` re-exported

### Changed

- **`TableDescription::column_type()`** returns `Option<LogicalType>` (RAII) instead of raw handle
- Version references updated to `"0.10"`

### Fixed

- 13 `expect()` calls in FFI callback contexts replaced with non-panicking `str_to_cstring()`
- 9 non-idiomatic `&mut { expr }` patterns replaced with `&raw mut`

## [0.9.0] — 2026-03-29

### Added

- **`Value` RAII wrapper** — owned wrapper around `duckdb_value` with `as_str()`, `as_i64()`, `as_i32()`, `as_f64()`, `as_f32()`, `as_bool()` and automatic `Drop` cleanup
- **`DataChunk` wrapper** — ergonomic wrapper around `duckdb_data_chunk` with `reader(col)`, `writer(col)`, `size()`, `set_size(n)`, `column_count()`, `vector(col)`
- **`VectorWriter::write_str()`** — alias for `write_varchar` for discoverability
- **`BindInfo::get_parameter_value()`** / **`get_named_parameter_value()`** — return owned `Value` instead of raw `duckdb_value`
- **`MapVector` reader/writer helpers** — `key_writer()`, `value_writer()`, `key_reader()`, `value_reader()`
- **`MockVectorWriter::write_str()`** — alias matching `VectorWriter` API
- **Prelude additions** — `Value`, `DataChunk`, `ValidityBitmap`

### Changed

- Version references updated across all docs to `"0.9"`

## [0.8.0] — 2026-03-28

### Added

- **`LogicalType::from_raw(ptr)`** — construct from raw handle
- **Complex type constructors** — `decimal`, `array`, `array_from_logical`, `union_type`, `union_type_from_logical`, `enum_type`
- **`_from_logical` variants** — `struct_type_from_logical`, `list_from_logical`, `map_from_logical` for nested complex types
- **20 introspection methods** on `LogicalType` — `get_type_id`, `get_alias`, `set_alias`, decimal/enum/list/map/struct/union/array child access
- **`TypeId::from_duckdb_type()`** — reverse conversion from raw C enum
- **`extra_info`** on `ScalarFunctionBuilder`, `ScalarOverloadBuilder`, `AggregateFunctionBuilder`
- **`param_logical` / `named_param_logical`** on `TableFunctionBuilder`
- **`CastFunctionBuilder::new_logical()`** for complex source/target types
- **Callback info wrappers** — `ScalarFunctionInfo`, `ScalarBindInfo` (`duckdb-1-5`), `ScalarInitInfo` (`duckdb-1-5`), `AggregateFunctionInfo`, `CopyBindInfo` (`duckdb-1-5`), `CopyGlobalInitInfo` (`duckdb-1-5`), `CopySinkInfo` (`duckdb-1-5`), `CopyFinalizeInfo` (`duckdb-1-5`)
- **`get_client_context()`** on all callback info types
- **`BindInfo`** — `get_parameter`, `get_named_parameter`, `get_extra_info`, `get_client_context`
- **`InitInfo` / `FunctionInfo`** — `get_extra_info`
- **`ArrayVector`** helper with `get_child()`
- **`vector_size()`** and **`vector_get_column_type()`** utilities
- **Prelude** — `StructVector`, `ListVector`, `MapVector`, `ArrayVector`, `ScalarFunctionInfo`, `AggregateFunctionInfo`

### Changed

- **Breaking:** `CastFunctionBuilder::source()` / `target()` return `Option<TypeId>` (was `TypeId`)
- **Breaking:** `CastRecord::source` / `target` fields changed to `Option<TypeId>`

## [0.7.1] — 2026-03-27

### Added

- **`TypeId::Any`** — wildcard type for function overload resolution (`duckdb-1-5`)
- **`TypeId::Varint`** — variable-length arbitrary-precision integer (`duckdb-1-5`)
- **`TypeId::SqlNull`** — explicit SQL NULL type for bare `NULL` literals (`duckdb-1-5`)
- **`TypeId::IntegerLiteral`** — integer literal type for overload resolution (`duckdb-1-5`)
- **`TypeId::StringLiteral`** — string literal type for overload resolution (`duckdb-1-5`)
- **`MockVectorReader`/`MockVectorWriter` tests** — 12 new tests for untested constructors and getters
- **DuckDB v1.5.1 evaluation** — see `docs/duckdb-v1.5.1-evaluation.md`

### Fixed

- **ARM64 / aarch64 build** — use `c_char` instead of `i8` for cross-platform
  pointer casts

### Changed

- **DuckDB v1.5.1 compatibility** — documentation updated to explicitly cover
  v1.5.1. C API version unchanged (`v1.2.0`). Recommend upgrading DuckDB
  runtime for WAL corruption and ART index fixes.

## [0.7.0] — 2026-03-22

### Added

- **`duckdb-1-5` feature modules** — the `duckdb-1-5` feature flag is no longer a
  placeholder. When enabled, it gates five new modules wrapping DuckDB 1.5.0
  C Extension API additions:
  - **`catalog`** — catalog entry lookup (`CatalogEntry`, `Catalog`,
    `CatalogEntryType`)
  - **`client_context`** — client context access (`ClientContext`) for
    retrieving catalogs, config options, and connection IDs from within
    registered function callbacks
  - **`config_option`** — extension-defined configuration options
    (`ConfigOptionBuilder`, `ConfigOptionScope`) registered via
    `SET`/`RESET`/`current_setting()`
  - **`copy_function`** — custom `COPY TO` handlers (`CopyFunctionBuilder`)
    with bind → global init → sink → finalize lifecycle
  - **`table_description`** — table metadata queries (`TableDescription`)
    for column count, names, and logical types

- **`TypeId::TimeNs`** — new `TIME_NS` column type variant for nanosecond-
  precision time of day (DuckDB 1.5.0+, requires `duckdb-1-5` feature)

- **`ScalarFunctionBuilder::varargs()`** / **`varargs_logical()`** — mark a
  scalar function as accepting variadic arguments (requires `duckdb-1-5`)

- **`ScalarFunctionBuilder::volatile()`** — mark a scalar function as volatile
  (re-evaluated for every row even with constant arguments, requires
  `duckdb-1-5`)

- **`ScalarFunctionBuilder::bind()`** — set a bind callback invoked once during
  query planning for per-query state allocation (requires `duckdb-1-5`)

- **`ScalarFunctionBuilder::init()`** — set an init callback invoked once per
  thread for per-thread local state allocation (requires `duckdb-1-5`)

### Changed

- **DuckDB 1.5.0 support** — upgraded default `libduckdb-sys` from 1.4.4 to
  1.10500.0 (DuckDB 1.5.0) and `duckdb` from 1.4.4 to 1.10500.0. The version
  range `">=1.4.4, <2"` in `Cargo.toml` is unchanged, preserving backward
  compatibility with DuckDB 1.4.x.

- **CI action updates** — `Swatinem/rust-cache` v2.8.2→v2.9.1,
  `actions/download-artifact` v8.0.0→v8.0.1, `actions/cache` 5.0.3→5.0.4,
  `codecov/codecov-action` 5.4.3→5.5.3.

### Fixed

- **COPY format handlers** — previously listed as a known limitation (no C API
  counterpart). DuckDB 1.5.0 adds `duckdb_create_copy_function` and related
  symbols; the new `copy_function` module wraps them behind `duckdb-1-5`.

---

## [0.6.0] — 2026-03-12

### Added

- **`InMemoryDb` dispatch table initialisation** — `InMemoryDb::open()` now
  correctly initialises the `loadable-extension` dispatch table from bundled
  DuckDB symbols before opening a connection. Previously, every call panicked
  with `"DuckDB API not initialized"` when the `bundled-test` feature was
  enabled in `cargo test`. See [Pitfall P9](pitfalls.md#p9) for the full
  technical analysis.

- **`src/testing/bundled_api_init.cpp`** — thin C++ shim exposing DuckDB's
  internal `CreateAPIv1()` as a C-linkage symbol, compiled at build time via
  the `cc` crate. Populates all 459 `AtomicPtr` dispatch table slots with real
  bundled DuckDB function pointers.

- **`build.rs`** — Cargo build script that locates the `libduckdb-sys` include
  path and compiles the C++ shim when the `bundled-test` feature is active.

- **CI: `test-bundled` job** — new CI job runs
  `cargo test --all-targets --features bundled-test` on Linux, macOS, and
  Windows on every PR, closing the gap that allowed this failure to reach the
  release workflow undetected.

- **Pitfall P9 documented** — full analysis in `LESSONS.md` and the
  [Pitfall Catalog](pitfalls.md#p9): root cause, `CreateAPIv1()` solution,
  ABI compatibility details, risks of the internal C++ API, and a mitigation
  table.

### Fixed

- `InMemoryDb::open()` no longer panics under `cargo test --features
  bundled-test`. This was broken from the initial 0.5.1 release.

### Changed

- `bundled-test` feature documentation updated to describe dispatch table
  initialisation accurately.

---

## [0.5.1] — 2026-03-12

### Added

- **Testing primitives (`quack_rs::testing`)** — `MockVectorWriter`,
  `MockVectorReader`, `MockDuckValue`, `MockRegistrar`, `CastRecord`.

- **`bundled-test` Cargo feature** — enables `InMemoryDb` for SQL-level
  assertions in `cargo test`. *(Note: `InMemoryDb::open()` was broken in this
  release and fixed in 0.6.0.)*

- **`InMemoryDb`** — wraps `duckdb::Connection` for SQL-level integration
  tests; available behind the `bundled-test` feature.

- **Builder introspection accessors** — `name()` on all function builders;
  `source()`/`target()` on `CastFunctionBuilder`.

### Security

- Bump `quinn-proto` 0.11.13 → 0.11.14 (addresses RUSTSEC advisory).

---

## [0.5.0] — 2026-03-10

### Added

- **`param_logical(LogicalType)` on all builders** — register parameters with complex
  parameterized types (`LIST(BIGINT)`, `MAP(VARCHAR, INTEGER)`, `STRUCT(...)`) that `TypeId`
  alone cannot express. Available on `AggregateFunctionBuilder`,
  `AggregateFunctionSetBuilder::OverloadBuilder`, `ScalarFunctionBuilder`, and
  `ScalarOverloadBuilder`. Parameters added via `param()` and `param_logical()` are
  interleaved by position, so the order you call them is the order DuckDB sees them.

- **`returns_logical(LogicalType)` on all builders** — set a complex parameterized return
  type. When both `returns(TypeId)` and `returns_logical(LogicalType)` are called, the
  logical type takes precedence. Available on `AggregateFunctionBuilder`,
  `AggregateFunctionSetBuilder`, `ScalarFunctionBuilder`, and `ScalarOverloadBuilder`. This
  eliminates the need for raw FFI when returning `LIST(BOOLEAN)`, `LIST(TIMESTAMP)`,
  `MAP(K, V)`, or any other parameterized type.

- **`null_handling(NullHandling)` on set overload builders** — per-overload NULL handling
  configuration for `AggregateFunctionSetBuilder::OverloadBuilder` and
  `ScalarOverloadBuilder`. Previously only available on single-function builders.

### Notes

- **Upstream fix: `duckdb-loadable-macros` panic-at-FFI-boundary** — the safe entry-point
  pattern developed in `quack-rs` (using `?` / `ok_or_else` throughout instead of `.unwrap()`)
  was contributed upstream as
  [duckdb/duckdb-rs#696](https://github.com/duckdb/duckdb-rs/pull/696) and merged 2026-03-09.
  All users of the `duckdb_entrypoint_c_api!` macro from `duckdb-loadable-macros` will receive
  this fix in the next `duckdb-rs` release. `quack-rs` users have always been protected via
  the safe `entry_point!` / `entry_point_v2!` macros provided by this crate.

---

## [0.4.0] — 2026-03-09

### Added

- **`Connection` and `Registrar` trait** — version-agnostic extension registration facade.
  `Connection` wraps the `duckdb_connection` and `duckdb_database` handles provided at
  initialization time. The `Registrar` trait provides uniform methods for registering all
  extension components (scalar, scalar set, aggregate, aggregate set, table, SQL macro, cast),
  making registration code interchangeable across DuckDB 1.4.x and 1.5.x.

- **`init_extension_v2`** — new entry point helper that passes `&Connection` to the
  registration callback instead of a raw `duckdb_connection`. Prefer this over
  `init_extension` for new extensions.

- **`entry_point_v2!` macro** — companion macro to `entry_point!` that generates the
  `#[no_mangle] unsafe extern "C"` entry point using `init_extension_v2`.

- **`duckdb-1-5` cargo feature** — placeholder feature flag for DuckDB 1.5.0-specific
  C API wrappers. Currently empty; will be populated when `libduckdb-sys` 1.5.0 is
  published on crates.io.

### Changed

- **DuckDB version support broadened to 1.4.x and 1.5.x** — the `libduckdb-sys` dependency
  requirement was relaxed from an exact pin (`=1.4.4`) to a range (`>=1.4.4, <2`). DuckDB
  v1.5.0 does not change the C API version string (`v1.2.0`); the existing `DUCKDB_API_VERSION`
  constant remains correct for both releases. Extension authors can pin their own `libduckdb-sys`
  to either `=1.4.4` or `=1.5.0` and resolve cleanly against `quack-rs`. The scaffold template
  and CI workflow template were updated to default to DuckDB v1.5.0.

---

## [0.3.0] — 2026-03-08

### Added

- **`TableFunctionBuilder`** — type-safe builder for registering DuckDB table functions
  (`SELECT * FROM my_function(args)`). Covers the full bind/init/scan lifecycle with
  ergonomic callbacks; `BindInfo`, `FfiBindData<T>`, and `FfiInitData<T>` eliminate all
  raw pointer manipulation. Verified end-to-end against DuckDB 1.4.4.
  See [Table Functions](../functions/table-functions.md).

- **`ReplacementScanBuilder`** — builder for registering DuckDB replacement scans
  (`SELECT * FROM 'file.xyz'` patterns). 4-method chain handles callback registration,
  path extraction, and bind-info population.
  See [Replacement Scans](../functions/replacement-scan.md).

- **`StructVector`**, **`ListVector`**, **`MapVector`** — safe wrappers for reading and
  writing nested-type vectors. Eliminate manual offset arithmetic and raw pointer casts
  over child vector handles. Re-exported from `quack_rs::vector::complex`.
  See [Complex Types](../data/complex-types.md).

- **`CastFunctionBuilder`** — type-safe builder for registering custom type cast
  functions. Covers explicit `CAST(x AS T)` and implicit coercions (optional
  `implicit_cost`). `CastFunctionInfo` exposes `cast_mode()`, `set_error()`, and
  `set_row_error()` inside callbacks for correct `TRY_CAST` / `CAST` error handling.
  See [Cast Functions](../functions/cast-functions.md).

- **`DbConfig`** — RAII wrapper for `duckdb_config`. Builder-style `.set(name, value)?`
  chain with automatic `duckdb_destroy_config` on drop and `flag_count()` /
  `get_flag(index)` for enumerating all available options.
  See [`quack_rs::config`](https://docs.rs/quack-rs/latest/quack_rs/config/index.html).

- **`ScalarFunctionSetBuilder`** — builder for registering scalar function overload sets,
  mirroring `AggregateFunctionSetBuilder`.

- **`NullHandling` enum and `.null_handling()` builder method** — configurable NULL
  propagation for scalar and aggregate functions.

- **`TypeId` variants** — `Decimal`, `Struct`, `Map`, `UHugeInt`, `TimeTz`,
  `TimestampS`, `TimestampMs`, `TimestampNs`, `Array`, `Enum`, `Union`, `Bit`.

- **`From<TypeId> for LogicalType`** — idiomatic conversion from `TypeId`.

- **`#[must_use]` on builder structs** — compile-time warning if a builder is
  constructed but never consumed.

- **`VectorWriter::write_interval`** — writes INTERVAL values to output vectors.

- **`append_metadata` binary** — native Rust replacement for the Python metadata
  script. Install with `cargo install quack-rs --bin append_metadata`.

- **`hello-ext` cast demo** — the example extension now registers
  `CAST(VARCHAR AS INTEGER)` and `TRY_CAST(VARCHAR AS INTEGER)` using
  `CastFunctionBuilder`, demonstrating both error modes with five unit tests.

- **`prelude` additions** — `TableFunctionBuilder`, `BindInfo`, `FfiBindData`,
  `FfiInitData`, `ReplacementScanBuilder`, `StructVector`, `ListVector`, `MapVector`,
  `CastFunctionBuilder`, `CastFunctionInfo`, `CastMode` added to `quack_rs::prelude`.

### Not implemented (upstream C API gap)

- **Window functions** and **COPY format handlers** are absent from DuckDB's public
  C extension API and cannot be wrapped. See [Known Limitations](known-limitations.md).

### Fixed

- **`hello-ext` `gs_bind` callback** — replaced incorrect `duckdb_value_int64(param)`
  with `duckdb_get_int64(param)`. All 11 live SQL tests now pass against DuckDB 1.4.4.

### Changed

- Bump `criterion` dev-dependency from `0.5` to `0.8`.
- Bump `Swatinem/rust-cache` GitHub Action from `v2.7.5` to `v2.8.2`.
- Bump `dtolnay/rust-toolchain` CI pin from `v2.7.5` to latest SHA.
- Bump `actions/attest-build-provenance` from `v2` to `v4`.
- Bump `actions/configure-pages` to latest SHA (`d5606572…`).
- Bump `actions/upload-pages-artifact` from `v3.0.1` to `v4.0.0`.

---

## [0.2.0] — 2026-03-07

### Added

- **`validate::description_yml` module** — parse and validate a complete `description.yml`
  metadata file end-to-end. Includes:
  - `DescriptionYml` struct — structured representation of all required and optional fields
  - `parse_description_yml(content: &str)` — parse and validate in one step
  - `validate_description_yml_str(content: &str)` — pass/fail validation
  - `validate_rust_extension(desc: &DescriptionYml)` — enforce Rust-specific fields
    (`language: Rust`, `build: cargo`, `requires_toolchains` includes `rust`)
  - 25+ unit tests covering all required fields, optional fields, error paths, and edge cases

- **`prelude` module** — ergonomic glob-import for the most commonly used items.
  `use quack_rs::prelude::*;` brings in all builder types, state traits, vector helpers,
  types, error handling, and the API version constant. Reduces boilerplate for extension authors.

- **Scaffold: `extension_config.cmake` generation** — the scaffold generator now produces
  `extension_config.cmake`, which is referenced by the `EXT_CONFIG` variable in the Makefile
  and required by `extension-ci-tools` for CI integration.

- **Scaffold: SQLLogicTest skeleton** — `generate_scaffold` now produces
  `test/sql/{name}.test`, a ready-to-fill SQLLogicTest file with `require` directive, format
  comments, and example query/result blocks. E2E tests are required for community extension
  submission (Pitfall P3).

- **Scaffold: GitHub Actions CI workflow** — `generate_scaffold` now produces
  `.github/workflows/extension-ci.yml`, a complete cross-platform CI workflow that builds and
  tests the extension on Linux, macOS, and Windows against a real DuckDB binary.

- **`validate::validate_excluded_platforms_str`** — validates the
  `excluded_platforms` field from `description.yml` as a semicolon-delimited string
  (e.g., `"wasm_mvp;wasm_eh;wasm_threads"`). Splits on `;` and validates each token.
  An empty string is valid (no exclusions).

- **`validate::validate_excluded_platforms`** — re-exported at the `validate` module level
  (previously only accessible as `validate::platform::validate_excluded_platforms`).

- **`validate::semver::classify_extension_version`** — returns `ExtensionStability`
  (`Unstable`/`PreRelease`/`Stable`) classifying the tier a version falls into.

- **`validate::semver::ExtensionStability`** — enum for DuckDB extension version stability tiers
  (`Unstable`, `PreRelease`, `Stable`) with `Display` implementation.

- **`scalar` module** — `ScalarFunctionBuilder` for registering scalar functions with the
  DuckDB C Extension API. Includes `try_new` with name validation, `param`, `returns`,
  `function` setters, and `register`. Full unit tests included.

- **`entry_point!` macro** — generates the required `#[no_mangle] extern "C"` entry point
  with zero boilerplate from an identifier and registration closure.

- **`VectorWriter::write_varchar`** — writes VARCHAR string values to output vectors using
  `duckdb_vector_assign_string_element_len` (handles both inline and pointer formats).

- **`VectorWriter::write_bool`** — writes BOOLEAN values as a single byte.

- **`VectorWriter::write_u16`** — writes USMALLINT values.

- **`VectorWriter::write_i16`** — writes SMALLINT values.

- **`VectorReader::read_interval`** — reads INTERVAL values from input vectors via
  the correct 16-byte layout helper.

- **CI: Windows testing** — the CI matrix now includes `windows-latest` in the `test` job,
  covering all three major platforms (Linux, macOS, Windows).

- **CI: `example-check` job** — CI now checks, lints, and tests `examples/hello-ext`
  as part of every PR, ensuring the example extension always compiles and its tests pass.

- **`validate::validate_release_profile`** — checks Cargo release profile settings for
  loadable-extension correctness. Validates `panic`, `lto`, `opt-level`, and `codegen-units`.

### Fixed

- MSRV documentation now consistently states 1.84.1 across `README.md`, `CONTRIBUTING.md`,
  and `Cargo.toml` (previously `README.md` stated 1.80).

---

## [0.1.0] — 2025-05-01

### Added

- Initial release
- `entry_point` module: `init_extension` helper for correct extension initialization
- `aggregate` module: `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder`
- `aggregate::state` module: `AggregateState` trait, `FfiState<T>` wrapper
- `aggregate::callbacks` module: type aliases for all 6 aggregate callback signatures
- `vector` module: `VectorReader`, `VectorWriter`, `ValidityBitmap`, `DuckStringView`
- `types` module: `TypeId` enum (33 variants), `LogicalType` RAII wrapper
- `interval` module: `DuckInterval`, `interval_to_micros`, `read_interval_at`
- `error` module: `ExtensionError`, `ExtResult<T>`
- `testing` module: `AggregateTestHarness<S>` for pure-Rust aggregate testing
- `scaffold` module: `generate_scaffold` for generating complete extension projects
- `sql_macro` module: `SqlMacro` for registering SQL macros without FFI callbacks
- Complete `hello-ext` example extension
- Documentation of all 15 DuckDB Rust FFI pitfalls (`LESSONS.md`)
- CI pipeline: check, test, clippy, fmt, doc, msrv, bench-compile
- `SECURITY.md` vulnerability disclosure policy

---

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.17.0...HEAD
[0.17.0]: https://github.com/tomtom215/quack-rs/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/tomtom215/quack-rs/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/tomtom215/quack-rs/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/tomtom215/quack-rs/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/tomtom215/quack-rs/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/tomtom215/quack-rs/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/tomtom215/quack-rs/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/tomtom215/quack-rs/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/tomtom215/quack-rs/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/tomtom215/quack-rs/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/tomtom215/quack-rs/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/tomtom215/quack-rs/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/tomtom215/quack-rs/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/tomtom215/quack-rs/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/tomtom215/quack-rs/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/tomtom215/quack-rs/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/tomtom215/quack-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/tomtom215/quack-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tomtom215/quack-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/quack-rs/releases/tag/v0.1.0
