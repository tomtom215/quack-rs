# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

This release comes out of a second production-readiness audit (see `AUDIT.md`,
"September 2026"). Each code defect below was either reproduced against a
real DuckDB before it was fixed or, where nothing could trigger it, derived
from DuckDB's source; `AUDIT.md` records which. Each code fix has a regression
test; the trait-bound fixes are pinned by `compile_fail` doctests.
Several fixes close holes in the *safe* API — places where safe code could
cause undefined behaviour, a data race or a process abort — and those needed
signature or trait-bound changes, so this is a breaking release; it follows
0.16.0. Each such entry is marked **Breaking:**.

0.17.0 was prepared but never published; its changes are included here, and
the entries below describe the change from 0.16.0.

A third pass followed before anything was published (`AUDIT.md` section 8).
It fixed further defects, reproducing them against a real DuckDB wherever that
was possible, added CI gates that compile the Rust examples in the book and the
README and check the book's links, and corrected the documentation's claim that
an aggregate's `update` never sees NULL rows (see Fixed).

A fourth pass followed that (`AUDIT.md` section 9). It fixed process aborts,
out-of-bounds reads and writes, and wrong answers. Each was reproduced against
a real DuckDB before it was fixed, and its regression test was shown failing
without the fix; `AUDIT.md` names the DuckDB versions per finding. The pass
also documented thirteen DuckDB defects in `docs/upstream-duckdb-reports.md`, each
with a plain-C reproducer. Its entries are grouped under **Fourth audit** in
each section below.

A fifth pass followed (`AUDIT.md` section 10). Each code fix was reproduced
against a real DuckDB, and its regression test shown failing without the
fix. DuckDB defects it found are added to `docs/upstream-duckdb-reports.md`,
each with a plain-C reproducer. Its entries are grouped under **Fifth audit**
in each section below.

### Added

#### Fifth audit

- `tests/aggregate_leaks.rs`, a test binary with a counting global allocator,
  and `tests/handle_leaks.rs`, which bounds the C heap (glibc `mallinfo2`)
  across many create-and-drop rounds of every RAII handle.
- `vector::max_child_capacity`: the most elements `DuckDB` can hold in a list
  vector's child buffer, which `ListBuilder` now respects.
- A `value_render` fuzz target (`fuzz/`, feature `live`) that renders
  arbitrary temporal payloads, alone and nested in lists, through a real
  `DuckDB`.

#### Fourth audit

- `scalar_bind_callback!` / `scalar_init_callback!` (`duckdb-1-5`).
- `validate_parameter_name`, `DUCKDB_UNCALLABLE_KEYWORDS`,
  `DUCKDB_UNREFERENCEABLE_PARAMETER_KEYWORDS`.
- `value::UNRENDERABLE`, `callback::EMPTY_PANIC_PLACEHOLDER`,
  `callback::panic_c_message`, `CopyGlobalInitInfo::get_file_path_bytes`.
- CI: `test-older-engines` runs the suite against DuckDB 1.4.4 (default
  features) and 1.5.0 (`duckdb-1-5`), the oldest release each can load into;
  every engine-specific defect below went unnoticed without it. A weekly
  scheduled run; the scaffold job runs the generated project's `make configure release
  test`; `check-abi-table.py` fingerprints whole signatures, not names.

- **Scalar functions as safe Rust closures.** `ScalarFunctionBuilder::map1` /
  `map2` / `map1_str` / `map2_str` / `map1_opt` / `map2_opt` take an ordinary
  closure; parameter and return types come from its signature, NULLs propagate
  correctly, and a panic becomes a SQL error. `VARCHAR` gets its own
  constructors so the closure can borrow a `&str` straight out of the vector.
  One indirect call per chunk, not per row. Each returns
  `Result<TypedScalarFunctionBuilder, _>`, which offers `name()`, `volatile()`
  and `register(con)` but cannot change the signature (code that passes it to a
  `Registrar` calls `Registrar::register_typed_scalar`), and the trampoline
  re-checks each chunk's vector types, so the declared result width always
  matches the closure's. Building one does not call DuckDB, so it works in a
  unit test with `MockRegistrar`; `register` checks the types.

- **`Value` gained every remaining constructor** — all scalar widths, the
  temporal family, `INTERVAL`, `BLOB`, `DECIMAL`, and the composites (`STRUCT`,
  `LIST`, `ARRAY`, `ENUM`; `MAP` and `UNION` behind `duckdb-1-5`) — plus
  `is_sql_null` (distinct from `is_null`, which asks about the handle) and
  `as_enum_index`. `struct_value` checks the field count first, because
  `duckdb_create_struct_value` takes no count and reads one value per field of
  the *type*. `list_value` / `array_value` take the **element** type: `duckdb.h`
  contradicts itself here, and the implementation settles it. The element type
  may itself be a `LIST` or `ARRAY` (lists of lists, arrays of arrays); the
  error explaining the element-type rule appears only when DuckDB itself refuses
  the value. `decimal` checks width (`1..=38`), scale and the unscaled value's
  digit count before calling DuckDB, which would otherwise abort the process or
  store a different number. The temporal constructors return `Result` and
  refuse a payload DuckDB cannot render (see the `Value::time_ns` /
  `Value::timestamp` entry under Changed).

- **`PreparedStatement` gained 16 more typed binds** and `bind_value`,
  the escape hatch for every composite type. `bind_decimal` validates like
  `Value::decimal`: `duckdb_bind_decimal` checks nothing, and for
  `width <= 18` keeps only the low 64 bits of the unscaled value.

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
  `unsafe extern "C" fn` around `Box::from_raw`, which is the abort hazard under Security
  relocated into user code. DuckDB reads scalar bind data from every executing
  thread at once, so `ScalarBindData<T>` requires `T: Send + Sync + 'static` and
  `ScalarLocalState<T>` `T: Send + 'static`. `ScalarBindData::set` also
  requires `T: Clone` (wrap other data in `Arc<T>`): it registers a generated,
  panic-safe copy callback, without which the bind data is lost whenever the
  optimizer copies the bound expression — a wrong answer, not an error
  (Pitfall L10; see `ScalarBindInfo::set_bind_data_copy` under Fixed). A second
  `set` leaks, rather than drops, the first value.

- **`vector::ops`** (`duckdb-1-5`) makes `SelectionVector` usable: `copy_selected`,
  `slice`, `reference_value`, `reference_vector` and `OwnedVector`. Documents
  that `slice` produces a *dictionary* vector, after which every reader in this
  crate reads the wrong rows. `OwnedVector::new` walks the type, child vectors
  included, with checked arithmetic and refuses a capacity above
  `vector::ops::MAX_CAPACITY` before DuckDB allocates: DuckDB computes the
  buffer size with an unchecked multiply, so `(HUGEINT, 2^60 + 2)` would get a
  32-byte buffer.

- **Arrow C Data Interface bridge** — the new `arrow` module behind a new
  **`duckdb-1-5-4`** feature, wrapping the eight-function conversion family
  DuckDB 1.5.0 added: `ArrowOptions<'conn>`, which borrows its connection so
  that a conversion cannot read freed memory after the connection closes (the
  safe `from_connection(&'conn OwnedConnection)`, and the `unsafe`
  `from_raw_connection`, `from_result` and `QueryResult::arrow_options`),
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
  first. `data_chunk_from_arrow` also returns `InvalidInput` for a negative
  length or offset, a null `children` pointer, a null child, and a child
  shorter than its parent's offset + length, each of which DuckDB would
  dereference or read out of bounds, and for a zero-row array, which DuckDB
  passes on as a zero-byte allocation that a debug build asserts against. Its Safety section requires that the array
  conform to the schema — nothing in an Arrow array records its type, so a child
  whose buffers do not match the type its schema declares cannot be checked —
  and documents that an absurd array length still aborts the process, because
  DuckDB allocates the chunk before its `try` block. DuckDB converts
  dictionary- and run-end-encoded children rather than rejecting them; TIMETZ
  comes back as TIME without its offset and BIT as BLOB.

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
    value's borrowed logical type without exposing the handle (pitfall P11);
    for a UNION it returns an empty name for the tag, then the member names.
  - `CopyFunctionBuilder::extra_info`, with the same
    ownership-until-transfer guarantee as the other builders.

  `duckdb_copy_function_set_copy_from_function` reports every rejection by doing
  nothing at all, so `copy_from` checks `duckdb.h`'s stated precondition — "the
  table function must take a single VARCHAR parameter (the file path)" — which
  DuckDB never enforces: `CCopyFromBind` builds the argument list itself and
  never consults `tf.arguments`, so a mismatch surfaces much later inside the
  reader's own bind callback.

- `TableFunctionBuilder::with_bind_init(bind, init)`: immutable bind data
  `B: Send + Sync` and a fresh scan state `S: Send` per execution, no `Clone`
  needed.
- `TypedScalarFunctionBuilder`; `Registrar::register_typed_scalar` (with a
  default implementation, so existing `Registrar`s compile).
- `StructWriter::set_row_null`.
- `callback::drop_panic_payload` and `callback::take_panic_message`.
- `selection_vector::MAX_LEN`; `datetime::is_valid_date`,
  `MICROS_PER_DAY`, `TIME_TZ_MAX_OFFSET_SECONDS`, `DECIMAL_MAX_WIDTH`.
- `CatalogEntryType::is_lookup_supported`;
  `ReplacementScanInfo::EMPTY_ERROR_PLACEHOLDER`.

- **Aggregate function sets support a different return type per overload**
  ([#121](https://github.com/tomtom215/quack-rs/issues/121)). `DuckDB` resolves
  an aggregate overload from its **parameter types and arity only** — the return
  type takes no part in resolution — so members of one set are free to return
  different types, which is how `DuckDB`'s own `arg_max(ANY, ANY) -> ANY` and
  `arg_max(ANY, ANY, ANY) -> ANY[]` coexist. quack-rs had the return type on
  `AggregateFunctionSetBuilder` alone, which made that impossible to express.

  `AggregateOverloadBuilder` now carries `returns` / `returns_logical`, and
  `AggregateFunctionSetBuilder::overload(..)` takes one fully-configured
  overload — mirroring `ScalarFunctionSetBuilder::overload` /
  `ScalarOverloadBuilder`, which already worked this way:

  ```rust
  AggregateFunctionSetBuilder::new("my_agg")
      .overload(
          AggregateOverloadBuilder::new()
              .param(TypeId::Integer)
              .returns(TypeId::Integer)
              // ... callbacks
      )
      .overload(
          AggregateOverloadBuilder::new()
              .param(TypeId::Varchar)
              .returns(TypeId::Varchar)
              // ... callbacks
      )
      .register(con)?;
  ```

  This is **additive**. `returns` / `returns_logical` on the *set* now act as a
  default for every overload that does not set its own, so existing
  `returns(..).overloads(range, ..)` code keeps working unchanged. Registration
  fails, naming the overload index, only when an overload has neither its own
  return type nor a set-level default.

- Unit tests asserting that every `AggregateOverloadBuilder` callback setter
  stores into its own field. These setters are `const fn`, which makes their
  cargo-mutants mutants **unviable** rather than caught — `Default::default()`
  cannot be called in a const context, so the replacement fails to compile and
  the mutation gate is structurally silent about them. That is a property of
  the gate, not evidence the setters work.

- **An AddressSanitizer job** (`ci.yml`; blocking — it was informational
  until its first green run). `leak-check` answers "did we forget a
  destructor"; ASAN answers "did we write outside an allocation, or use one
  after free" — the class behind the two
  heap-corruption defects fixed in v0.16.0, and the one a crate doing raw
  pointer arithmetic into DuckDB's memory is most exposed to. Miri cannot reach
  those paths: they call foreign functions.
- **`scripts/duckdb-version-from-lock.sh`** — derives the DuckDB release tag
  from the `libduckdb-sys` pin in `Cargo.lock` (1.10505.0 → v1.5.5). Two CI jobs
  hard-coded `v1.5.4` next to a comment asking the next person to keep it in
  sync with the dependency; bumping the lockfile would have left both linking a
  libduckdb one release older than the bindings being generated against it.
  Self-tested against all six known version mappings, including the pre-1.5
  scheme.
- **`scripts/sync-book-changelog.py`** — the book's changelog page is now
  generated from `CHANGELOG.md`, with a `--check` mode wired into the `doc` job.
  Kept by hand, the mirror had fallen ~19 KB behind and the *published* 0.16.0
  entry was missing its entire "Portability and feature-combination breakage"
  subsection. The only deliberate difference, an em dash in release headings,
  is applied by the script.
- `timeout-minutes` on every `ci.yml` job (35 at this release). The default is
  360 per job, so a hang in `test-bundled` (which compiles DuckDB from C++
  source) or
  `leak-check` (`-Zbuild-std`) burned six hours of runner time.

- **First end-to-end coverage of the aggregate function-set registration path**
  (`tests/ffi_roundtrip.rs`). Neither the aggregate nor the scalar set builder
  had an E2E test, despite Pitfall L6 — a set member whose name is unset is
  dropped *silently*. Four new tests register a real three-overload set
  (`BIGINT -> BIGINT`, `VARCHAR -> VARCHAR`, `(BIGINT, BIGINT) -> DECIMAL(18,2)`
  via `returns_logical`), assert `typeof(..)` per overload, check the computed
  values across multiple chunks and under `GROUP BY`, and cover the set-level
  default and both rejection paths.

- `LogicalType::try_get_type_id`, which returns `None` for a type id this crate
  does not know; `get_type_id` panics there, and now documents that it does.
- A fallible form of every `LogicalType` constructor: the new `try_decimal`,
  `try_array`, `try_array_from_logical`, `try_list_from_logical` and
  `try_map_from_logical` join the existing `try_*` functions. Also
  `types::logical_type::MAX_UNION_MEMBERS` (256).
- `VectorWriter::try_write_varchar` / `try_write_blob`, which return an error
  and write nothing for a value longer than `vector::string::MAX_STRING_LEN`
  (`u32::MAX` bytes, DuckDB's string length limit); also
  `vector::string::check_string_len`.
- `ListBuilder::with_element_limit`, to cap list lengths that come from input
  at what fits in memory. Staying under `MAX_LIST_CHILD_CAPACITY` is not enough:
  `duckdb_list_vector_reserve` has no `try`/`catch`, so a failed allocation
  below that ceiling also aborts the process. `ListVector::reserve` now
  documents this.
- `vector::ops::MAX_CAPACITY`, the largest capacity `OwnedVector::new` accepts
  (2^37 elements, DuckDB's `MAX_VECTOR_SIZE`).
- `MockVectorWriter::set_valid` and `MockVectorWriter::is_written` (see the
  `MockVectorWriter` entry under Changed).
- `CatalogEntryType::may_autoload_extension`, the name check behind the new
  catalog-lookup refusal (see Changed).
- An `EMPTY_ERROR_PLACEHOLDER` for table functions
  (`table::info::EMPTY_ERROR_PLACEHOLDER`), copy functions
  (`copy_function::info::EMPTY_ERROR_PLACEHOLDER`) and casts
  (`CastFunctionInfo::EMPTY_ERROR_PLACEHOLDER`), and
  `callback::CAST_FAILED_WITHOUT_MESSAGE` (see Fixed).
- `InstanceCache` is `Send + Sync`. DuckDB's instance cache guards its own state
  with a mutex, so the wrapper was `!Send` only because it holds a raw pointer.
- `WarningSeverity` implements `PartialOrd` and `Ord` in the order
  `Info < Low < Medium < High < Critical`, so `severity >= WarningSeverity::High`
  selects the warnings that need attention.
- `ScalarOverloadBuilder` gains `volatile`, `varargs`, `varargs_logical` and,
  with `duckdb-1-5`, `bind` / `init`; `AggregateOverloadBuilder` gains
  `extra_info`. Each behaves as it does on the single-function builder,
  including who frees `extra_info` when registration fails.
- The prelude exports `ScalarBindData` and `ScalarLocalState` (with
  `duckdb-1-5`, beside `ScalarBindInfo` / `ScalarInitInfo`). Its documentation
  now lists every re-export, and a unit test fails when one is missing.
- `validate_spdx_license` accepts `<license> WITH <exception>` (for example
  `Apache-2.0 WITH LLVM-exception`), which it used to reject. The exception must
  be on the SPDX exception list, now public as
  `validate::spdx::SPDX_LICENSE_EXCEPTIONS`, or an `AdditionRef-` id.
- `classify_extension_version` accepts one leading `v` on the semantic-version
  forms (`v1.0.0`): the spelling DuckDB's versioning documentation uses, and
  what extension-ci-tools stamps when `HEAD` carries a `vX.Y.Z` tag.
- **Pitfall L12** (`LESSONS.md`, `book/src/reference/pitfalls.md`): an
  aggregate's `update` receives NULL rows under the default NULL handling too.
  See Fixed.
- **Projects generated by `generate_scaffold` test something.** The generated
  project's `cargo test` ran zero tests and its `test/sql/<name>.test`
  held only `require` and commented-out examples, so both CI steps passed
  whatever the extension did. The project now has a unit test and a
  SQLLogicTest that queries `<name>_hello` and checks its output. The generated
  `Makefile` checks that `extension-ci-tools` is checked out and names both
  commands: `git submodule add` in a new repository (where the documented
  `git submodule update --init` clones nothing) and `update --init` in a clone
  (Pitfall P4).
- **The book and the README are compiled as doctests.** `book/doctest` is a
  standalone crate that turns every page under `book/src` (except the changelog)
  and `README.md` into rustdoc input, so every Rust block not marked `ignore`
  compiles against the working copy, and runs unless it is marked `no_run`;
  `test-bundled-prebuilt` runs it. Each remaining `ignore` block says why. `mdbook test` cannot do this: it passes no `--extern quack_rs` and
  defaults to edition 2015. The blocks that failed, and the content errors that
  turned up, are listed under Fixed. `release.yml`'s gate also runs the crate's
  own doctests now (`cargo test --doc --features duckdb-1-5-4`).
- New CI jobs and checks (`ci.yml`):
  - `book` builds the book with mdBook on every pull request, not only after a
    merge to `main`, then runs `scripts/check-book-links.py`, which checks every
    relative link and anchor in the book and README and every docs.rs link
    against this checkout's rustdoc. mdBook checks neither.
  - `autoload-entries` runs `scripts/check-autoload-entries.py`, which fails
    when a DuckDB release from v1.5.0 on autoloads an extension for a type or
    collation name missing from the lists behind the catalog-lookup refusal.
  - `abi-guard-layout` exercises the real `LayoutMismatch` path: a `duckdb-1-5`
    build stamped `C_STRUCT` must be refused by DuckDB v1.5.0 and v1.4.4 and
    must load into the release its bindings come from. The existing `abi-guard`
    job only ever reached the declared-version check.
  - `dependency-floor` resolves `duckdb` / `libduckdb-sys` to the declared 1.4.4
    floor and runs `cargo test --lib`, on stable: at that floor the dependency
    tree needs a newer rustc than the 1.86.0 MSRV.
  - `extension-load` runs `scripts/check-hello-ext.py`, which executes every
    statement documented in `examples/hello-ext/README.md` against the loaded
    extension and compares the output with `examples/hello-ext/sql_checks.txt`.
  - `semver` fails when cargo-semver-checks ran 0 checks against the published
    baseline for a bump that is not breaking; before, 0 checks passed. For a
    breaking bump it runs none by design, so a new informational step compares
    against `origin/main` with `--release-type patch` and lists every breaking
    change on the branch in the log and the job summary.
  - Integration tests fail when a documented "N pitfalls" count differs from
    `LESSONS.md`, when the source trees in `CONTRIBUTING.md` and the book
    miss a file under `src/` or `tests/` or list one that does not exist, and
    when a documentation paragraph mentions `panic = "abort"` without warning
    against it.

### Changed

#### Fifth audit

- **Breaking: `FfiState<T>` stores a small `T` in `DuckDB`'s state bytes.** A
  `T` of at most 256 bytes, aligned no more strictly than `usize`, is kept
  inline; a larger one is still boxed. `FfiState<T>` is no longer a two-word
  struct with a public `inner` field; use its callbacks and `with_state` /
  `with_state_mut` as before. See Fixed.
- **Breaking: `AggregateState` requires `Sync`.** A window's segment tree
  shares its states between threads as `combine` sources.
- Documentation: bind-time arguments are seen before the cast to the
  parameter type (`ScalarBindInfo::argument`); a Safety clause on
  `data_chunk_from_arrow` (validity bitmaps are read one byte past their
  rows); Known Limitations entries for aggregate states `DuckDB` never
  destroys, abandoned streams and out-of-memory aborts.
- `Value::as_str`, `display_string` and `Debug` render only types whose every
  payload `DuckDB` can render; everything else (`VARIANT`, `GEOMETRY`, a type
  quack-rs does not know, and `ARRAY` / `UNION` values that could hold one)
  gets `UNRENDERABLE`. See Fixed.
- `docs/upstream-duckdb-reports.md` gained items 20 to 23.

#### Fourth audit

- CI's "the refused extension registered nothing" checks could not fail
  (`duckdb -c` stops at the failed `LOAD`); they feed the statements on stdin
  and require a marker row.
- `AbiPolicy` handling is a pure `policy_verdict`, tested against every check
  result. The docs of `enforce_abi_policy` no longer say `Warn` goes through
  `set_error`.
- Arrow export documents the INTERVAL and UHUGEINT values `DuckDB` corrupts.
- `docs/upstream-duckdb-reports.md` gained items 7 to 19.

- **Breaking: `CopyFunctionBuilder` is no longer `Send` or `Sync`.** Supporting
  `COPY … FROM` gave it two new fields that each carry a raw pointer —
  `copy_from: Option<TableFunctionHandle>` (a `duckdb_table_function`) and
  `extra_info: Option<ExtraInfo>` (a `*mut c_void`) — and a raw pointer is
  neither `Send` nor `Sync`. `cargo-semver-checks` classifies this as
  `auto_trait_impl_removed`, a major break, one of the reasons this release
  bumps the minor version: for a pre-1.0 crate Cargo treats the leftmost
  non-zero component as the major, so the minor position is where a break goes
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

  The pitfall count in the README, the crate docs and the FAQ was stale at 17,
  and the book's catalogue was missing L8 and L9. All four now agree with
  `LESSONS.md`, and an integration test fails when a documented count differs
  from it (see Added).

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

- CI: the **AddressSanitizer job is now blocking**, as planned when it was
  added. It passed on `main` and over the merged 125-test end-to-end suite with
  no reports and no suppressions.
- CI: the informational **beta clippy job now also lints the end-to-end
  tests**, with `bundled-test-prebuilt,duckdb-1-5-4` against a pre-built
  libduckdb. Those tests compile only with `bundled-test-prebuilt`, so beta's
  new `assert_is_empty` lint fired on four of them while the job stayed green.
- Mutation testing: the configuration moved to `.cargo/mutants.toml`. At the
  repository root cargo-mutants never read it — so the full sweep ran without
  its exclusions or features (2,164 mutants listed instead of 1,348) — and it
  carried two keys cargo-mutants 27.1.0 rejects (`cap_timeout`, `jobs`). Its
  `examine_globs` is gone too: once the file was read, that key overrode the
  incremental job's `--file` flags instead of being narrowed by them.

- **Breaking:** `TableFunctionBuilder::with_state` requires `S: Clone + Send`:
  the state `bind` returns is a template and every execution scans a fresh
  clone. Use the new `with_bind_init` for state that cannot be cloned.
- **Breaking:** `TypedTableFunctionBuilder::projection_pushdown` is removed —
  the typed scan closure cannot learn the projection, so enabling it returned
  the wrong columns. Use the raw `TableFunctionBuilder` for pushdown.
- **Breaking:** `FfiBindData::set` / `FfiInitData::set` require `T: Send + Sync`,
  `FfiLocalInitData::set` `T: Send`; `ReplacementScanBuilder::register_with_data`
  and `Connection::register_replacement_scan_with_data` require `T: Send + Sync`.
- **Breaking:** every scalar `Value` getter returns `Option<T>` (`as_i8` …
  `as_u128`, `as_f32`, `as_f64`, `as_bool`, the date/time/timestamp family,
  `as_interval`, `as_uuid`, `as_decimal`; the new `as_enum_index` does too):
  `None` for a null handle, SQL `NULL`, a non-scalar value or a failed cast. A
  failed cast used to return a sentinel (`T::MIN`, `NaN`) indistinguishable
  from a real value. The `as_*_or(default)` forms keep their signatures and now
  cover all four cases.
- **Breaking:** `Value::as_blob` accepts only a `BLOB` (DuckDB's cast of
  anything else to `BLOB` could throw) and errors on SQL `NULL`;
  `Value::as_str` errors on SQL `NULL`.
- **Breaking:** `FileSystem<'ctx>` borrows its `ClientContext`.
- **Breaking:** `DuckDbErrorType` gains `Autoload`, `Sequence` and
  `InvalidConfiguration` (40–42), which used to map to `Invalid`.
- **Breaking:** `SelectionVector::new` returns `Result<Self, ExtensionError>`.
- **Breaking:** `datetime::date_to_days`, `time_from_micros`,
  `time_tz_from_bits`, `timestamp_from_micros`, `timestamp_to_micros`,
  `time_tz_bits` and `decimal_to_f64` return `Option`.
- **Breaking:** `VectorWriter::set_null` / `set_null_range` (and so
  `DataChunk::propagate_nulls`) on a `STRUCT` or `ARRAY` vector also null the
  row's fields / elements, recursively, as DuckDB's `FlatVector::SetNull` does.
- **Breaking:** `SqlMacro::to_sql` emits double-quoted identifiers
  (`CREATE OR REPLACE MACRO "add"("a", "b") AS (a + b)`). Calling the macro is
  unchanged: DuckDB resolves quoted identifiers case-insensitively.
- `ScalarFunctionBuilder::varargs`, `varargs_logical` and `volatile` no longer
  require `duckdb-1-5`: both C functions are in the stable v1.2.0 API.
- `Appender` is re-exported from the prelude without a feature, matching the
  module, and `use quack_rs::prelude::*` now brings `entry_point!` /
  `entry_point_v2!` into scope, as the prelude's own documentation said.

- `AggregateFunctionSetBuilder::overloads` is unchanged, but the builder its
  closure receives is now named `AggregateOverloadBuilder`, for symmetry with
  `ScalarOverloadBuilder`. The old name remains as a deprecated type alias
  (`quack_rs::aggregate::builder::OverloadBuilder`) and still compiles.
- `AggregateOverloadBuilder` is exported from `quack_rs::aggregate` and from the
  prelude. The old `OverloadBuilder` was reachable only at
  `quack_rs::aggregate::builder::`, and had no public constructor, so a caller
  could not build one outside an `overloads` closure.
- `AggregateOverloadBuilder` moved to `src/aggregate/builder/overload.rs`,
  keeping both it and `set.rs` inside the 500-line guideline in
  `CONTRIBUTING.md`.

- **Breaking:** `QueryResult::next_chunk` returns
  `Result<Option<OwnedDataChunk>, ExtensionError>`. `duckdb_fetch_chunk`
  returns null both at the end of the rows and when the fetch fails, and
  `next_chunk` returned `None` for both, so a streaming result that failed part
  way — a runtime error in the query, or another statement run on the same
  connection — read as a complete, shorter result. The error DuckDB recorded is
  now returned as `Err`, and keeps being returned on later calls; a clean end
  stays `Ok(None)`.
- **Breaking:** `Value::time_ns` and `Value::timestamp` return
  `Result<Value, ExtensionError>` and refuse a payload outside the range
  DuckDB's own SQL produces; so do the new `Value::time`, `time_tz`,
  `timestamp_tz`, `timestamp_s`, `timestamp_ms` and `timestamp_ns` (see
  Added). DuckDB stores any 64-bit payload unchecked, and rendering an
  out-of-range one crashed or aborted the process (`Value::time_ns(i64::MAX)`,
  `Value::timestamp(i64::MIN)`) or printed garbage. `Value::date` and the new
  `Value::interval` are infallible: DuckDB renders every value of those types.
- **Breaking:** `CatalogEntry::lookup` and `Catalog::get_entry` return
  `Result<Option<_>, ExtensionError>`: `Ok(None)` is "not found", `Err` is
  "refused". A `Type` or `Collation` lookup of a name that makes DuckDB
  autoload an extension (`inet`, `json`, an ICU collation name) is refused
  without calling DuckDB while the `autoload_known_extensions` setting is on: a
  failed autoload inside `duckdb_catalog_get_entry` aborted the process, and a
  successful one loaded an extension as a side effect of a lookup. The entry
  types that were already refused (`Schema`, `Database`, `PreparedStatement`,
  `Invalid`) are now an `Err` too, instead of a `None` that looked like "not
  found".
- **Breaking:** `FileSystem::open` returns `FileHandle<'_>`, which borrows the
  `FileSystem`. A `FileHandle` kept after its database was closed read freed
  memory (valgrind: an invalid read in `duckdb::FileHandle::Read`).
  `FileHandle::from_raw` returns a handle whose lifetime the caller chooses.
- **Breaking:** `InitInfo::projected_column_index` returns `Option<usize>`,
  `None` past the end of the projection, where DuckDB returns 0 — a real column
  index. `CopyBindInfo::column_type` returns `Option<LogicalType>`,
  bounds-checked like `BindInfo::result_column_type`; it used to wrap the null
  handle DuckDB returns for an out-of-range index.
- **Breaking:** `TypedTableFunctionBuilder::build` returns an error when
  `projection_pushdown(true)` was set on the `TableFunctionBuilder` before
  `with_state` / `with_bind_init`. That sequence bypassed the typed builder's
  no-pushdown rule, and the scan returned the wrong columns (`SELECT b` got
  column `a`'s values).
- **Breaking:** `ConfigOptionBuilder::register` refuses an option with no
  default value (DuckDB then reports it as an unrecognized configuration
  parameter), an option of a pseudo-type (`ANY`, `SQLNULL`, `INTEGER_LITERAL`,
  `STRING_LITERAL`) or of a type the typed `Value` constructors cannot build
  (`BIGNUM`, `GEOMETRY`, `VARIANT`, …), and a name that `duckdb_settings()`
  already lists, compared case-insensitively and including aliases: an option
  named `threads` used to register and shadow the built-in setting. The default
  is now handed to DuckDB already typed (see Security).
- **Breaking:** `TableFunctionBuilder::register` refuses a name that already
  belongs to a table function or table macro, and
  `CopyFunctionBuilder::register` a format name that already exists (`csv`,
  `parquet`, …). DuckDB dropped such a registration while reporting success, so
  the function was never called. The C API cannot overload table functions.
- **Breaking:** `ScalarFunctionBuilder::register` and
  `ScalarFunctionSetBuilder::register` (and so `map1`, `map2` and the other
  typed constructors) refuse a parameter signature that `duckdb_functions()`
  already lists under that name, built-ins included. DuckDB merges a scalar
  registration into the existing entry with override on, so the new overload
  silently replaced the old one for every query in the database — after
  registering `abs(BIGINT)`, `abs(-5::BIGINT)` returned 995 — or, with a
  different return type, made every call ambiguous. Signatures containing
  `STRUCT`, `UNION`, `ENUM` or a literal pseudo-type are registered unchecked,
  as documented on both builders.
- **Breaking:** `SqlMacro::register` refuses a body that holds more than one
  statement, counted with DuckDB's own parser, and executes nothing. The body
  `1); DROP TABLE t; SELECT (1` used to register and drop the table. A scalar
  body that ends in a `--` comment, which used to comment out the closing
  parenthesis, now works.
- **Breaking:** `Appender` no longer loses rows silently. DuckDB's appender
  cannot take back a value, so a `row()` closure that failed after appending
  part of a row left the row half-written, and `close()` then returned `Ok` and
  wrote none of the buffered rows. Such a `row()` now poisons the appender:
  every later `append_*`, `row`, `end_row`, `flush` and `close` returns an
  error saying how many buffered rows were not written. `close()` with a row
  started but not ended is an error; every mutating method is an error after a
  successful `close()` (DuckDB accepted appends after close and wrote them at
  the next flush); `append_chunk` in the middle of a row is refused. With
  `duckdb-1-5`, `clear()` resets DuckDB's state and clears the poison. A value
  rejected first in its row loses nothing and does not poison.
- **Breaking:** the `LogicalType` STRUCT and UNION constructors
  (`struct_type`, `struct_type_from_logical`, `union_type`,
  `union_type_from_logical` and their `try_` forms) apply the rules DuckDB's
  binder applies to the same types in SQL: names unique ignoring ASCII case
  (empty names exempt) and at most `MAX_UNION_MEMBERS` union members. The C
  API checks nothing, so a scalar returning such a type registered and then
  failed every call with "duplicate name in struct". The `try_` forms return
  an error; the others panic.
- **Breaking:** `VectorWriter::write_varchar` / `write_blob` and
  `StructWriter::write_varchar` / `write_blob` panic for a value longer than
  `MAX_STRING_LEN` instead of storing a truncated one (see Fixed). Inside
  `scalar_callback!` and the typed scalar constructors the panic becomes a SQL
  error; `try_write_varchar` / `try_write_blob` return it instead.
- **Breaking:** `MockVectorWriter` behaves like a real output vector, so a test
  that passed for a callback that is wrong in DuckDB now fails. `set_null`
  followed by a `write_*` leaves the row NULL (`set_valid` undoes a NULL); a row
  never written is valid, not NULL (`is_written` tells a test whether the loop
  wrote it); a write past capacity panics instead of growing the mock; a string
  over `MAX_STRING_LEN` panics, as `VectorWriter` now does. The docs no longer
  claim one function can be called with both the mock and the real writer: the
  types differ.
- **Breaking:** `generate_scaffold` validates the free text it writes into the
  generated files: `description` and `maintainer` must be non-empty, must not
  start or end with whitespace and must not contain control or bidirectional
  characters (`maintainer` must also be one line), `github_repo` must be
  `owner/repo`, and `git_ref` a commit hash or tag. Those fields went into
  `description.yml` unquoted, so `Fast: analytics`, `Analytics #1` or a
  newline produced a file that parsed to something else or not at all; they
  are now YAML double-quoted scalars. Each line of a multi-line description
  gets its own `//!` prefix; the second and later lines used to fall outside
  the doc comment and fail to compile.
- **Breaking:** `validate_spdx_license` rejects nesting deeper than 64
  parentheses. It recursed once per `(` with no limit, so a license field of a
  million `(` overflowed the stack and aborted the process, reachable from
  `parse_description_yml` on untrusted input.
- `TypeId::TimeNs`, `Any`, `Varint`, `SqlNull`, `IntegerLiteral` and
  `StringLiteral` no longer require `duckdb-1-5`. All six exist in DuckDB 1.4.4,
  this crate's floor, and 1.4.4 produces `TIME_NS` and `BIGNUM` columns, so with
  default features `LogicalType::get_type_id` panicked on those columns.
- **Breaking:** `TypeId::Varint.sql_name()` returns `"BIGNUM"`, DuckDB's name
  for the type since 1.4, instead of `"VARINT"`. Both names parse as SQL, but
  code that compares the returned string sees a different value.
- `SecretEntry`'s `Debug` output shows `[REDACTED]` for a non-empty scope: the
  scope (a bucket or URL prefix) is zeroized on drop as sensitive, but `Debug`
  printed it.
- `InMemoryDb` checks, before first use, that the `bundled-test-prebuilt` C++
  shim was compiled against headers whose `duckdb_ext_api_v1` has the size the
  `libduckdb-sys` bindings expect, and panics naming both slot counts,
  `DUCKDB_LIB_DIR` and `cargo tree -i libduckdb-sys` if not. With a DuckDB 1.5.0
  library under 1.10505 bindings, a slot the older headers lack was left as
  stack garbage and the tests ran on; larger headers would overrun the buffer.
- `scripts/check-abi-table.py` lists every upstream `vX.Y.Z` tag from v1.2.0 on
  instead of a hard-coded list, which is how v1.4.5 went missing from the
  layout table (see Security).
- `RELEASING.md`: a release is done only once the tag is on `origin`,
  crates.io's newest version is the `Cargo.toml` version and docs.rs has built
  it, each with a command to check; version snippets are bumped in the release
  pull request itself.
- Internal layout, with no change to any public path: `src/value.rs` is split
  into `value/composite.rs`, `value/nested.rs`, `value/scalars.rs`,
  `value/temporal.rs` and `value/temporal_checks.rs`; the `LogicalType`
  constructors moved to `types/logical_type/construct.rs`; and
  `ScalarOverloadBuilder` moved to `scalar/builder/overload.rs`. `src/query.rs`,
  `src/appender.rs`, `src/arrow.rs` and `src/testing/mock_vector.rs` are split
  the same way into private submodules. This keeps those files inside the
  500-line guideline in `CONTRIBUTING.md`.
- Every `unsafe` block in library code now states, in a `// SAFETY:` comment,
  the invariant it relies on and why it holds; 149 did not.
  `clippy::undocumented_unsafe_blocks` is enabled so CI keeps it that way (test
  code is exempt).
- `docs/architecture.md` matches the crate again: its module table listed
  neither `abi`, `arrow`, `callback`, `chunk_writer`, `datetime`, `query`,
  `secrets`, `tls` nor `warning`, and said `appender`, `table_description`,
  `ScalarFunctionBuilder::varargs` and `volatile` need `duckdb-1-5` (they do
  not). An integration test now fails when the table and `src/lib.rs` disagree.

### Fixed

#### Fifth audit

- **Rendering a value aborted the process** for values ordinary SQL builds:
  a `VARIANT` holding an out-of-range timestamp, a `DECIMAL(38, 0)` holding
  `i128::MIN` (from `sum` over two in-range values), and a `GEOMETRY` built
  from malformed WKB each made `DuckDB`'s cast to text throw through the C
  API; a `DECIMAL(38, 38)` holding 1.2 rendered with an unwritten first byte,
  and aborted too when that byte was not valid UTF-8. The render guard is now
  an allow-list, and `DECIMAL` payloads are checked against their width.
- **`ListBuilder` aborted the process on a large list of a wide type.**
  `DuckDB`'s ceiling is 2^37 *bytes* per child buffer, not elements, checked
  after it rounds the reservation up to a power of two, so a `BIGINT` list of
  2^34 + 1 elements, or an `INTEGER[1000]` list of 2^25 + 1, reached a reserve
  that throws through the C API. The builder respects the rounded byte
  ceiling; a row past it is NULL.
- **Aggregate states `DuckDB` moved were never dropped.** The fourth audit's
  `FfiState` tag was derived from the slot's address, and radix
  repartitioning copies states to new rows, so every moved state was skipped
  and its `T` leaked (8 of them after one ungrouped query on eight threads in
  the regression test). The tag follows the slot's contents.
- **Aggregate states `DuckDB` never destroys leaked a box each.** A grouped
  aggregate's states that a stopped scan never reached (a `LIMIT` above it,
  an error, an interrupt) are never destroyed by `DuckDB` 1.4.4 to 1.5.5;
  under `LIMIT 10` over 300,000 groups, 297,952 boxed `T`s leaked. A small
  `T` is now stored in `DuckDB`'s own state bytes, so it leaks nothing unless
  it owns heap memory itself.

#### Fourth audit

- **A C API aggregate in a running window returned the wrong answer.** Without
  a destructor `DuckDB` streams it and re-reads the first row
  (`sum`-like: `1 2 3 4 5` for `1 3 6 10 15`). Every aggregate now registers
  one (a no-op when none is given).
- **Arrow import with a nonzero parent offset imported the wrong rows**
  (`DuckDB` ignores the offset for values); it is refused.
- **`ListBuilder` overwrote rows already in the list vector** when it started
  on a non-empty one; it appends after them.
- **`VectorWriter::set_valid` left a nested row's children NULL**; it restores
  them when the row was NULL.
- **Scalar collision check.** It missed signatures containing a type alias,
  could be shadowed by a user macro named like the catalog functions it
  queries (it now qualifies them with `system.main.`), and accepted overloads
  that `DuckDB`'s binder finds ambiguous with varargs. **Breaking:** such
  overlapping overloads are refused at registration. `Connection` keeps a
  snapshot of existing scalars, so 300 registrations through `Registrar`
  take 18.6–35.6 ms instead of 5.2–6.3 s (release build, three runs each).
- **Keyword names.** **Breaking:** `validate_function_name` refuses the 53
  keywords (`DUCKDB_UNCALLABLE_KEYWORDS`) that `DuckDB` cannot call unquoted —
  `coalesce(x)` silently ran the built-in — and `SqlMacro` parameters use the
  new `validate_parameter_name` (79 keywords).
- **Catalog entries outlived their handle.** **Breaking:** `CatalogEntry`
  copies the name and type at lookup and owns nothing afterwards.
- **Secret scopes were parsed from a string**; they are read as a list,
  NULL-safely, from `system.main.duckdb_secrets()`.
- **`CopyGlobalInitInfo::get_file_path` truncated at a NUL.** **Breaking:**
  it returns `Result<String>`; `get_file_path_bytes` returns the raw bytes.
- **`interval_to_micros` reported overflow for totals that fit** (an
  intermediate sum overflowed); it computes the exact total in `i128`.
- **`Appender`:** a failed automatic flush (every 204,800 rows) poisoned the
  appender with a false "half-written row" message; the row counts as ended
  and the constraint error is reported as it is.
- **On DuckDB 1.4.x**, registering a scalar under any existing name (a new
  overload of `abs`, say) failed with no reason, because the C API registers
  with `CREATE` before 1.5.0; quack-rs refuses it first and says why. And
  `LogicalType::try_new(TypeId::TimeNs)` returned an `INVALID` type there (the
  1.4.x C API does not know `TIME_NS`); it is an error now, as is any type id
  the running engine hands back changed.
- **`LogicalType::register`** blamed a taken name when the type contained
  `ANY`; it names the cause. `try_decimal` validates width and scale itself
  (`DuckDB` does only from 1.5.4). **Breaking:** `try_array(_, 0)`, an empty
  `union_type` and an empty `Value::array_value` are refused, as in SQL.
- **`MockRegistrar` accepted builders the real registration refuses.**
  **Breaking:** it runs the same checks (missing callback or return type,
  empty function set, incomplete copy function, config option without type or
  default) and records nothing on failure.
- **`description.yml`:** a quoted, flow or block value on the line after its
  key kept its quotes; values that `PyYAML` (YAML 1.1) reads as a boolean,
  number, date or null draw a warning; the scaffold quotes `name`, `github`
  and `ref`. The "text after a comment" error named the line the value
  started on rather than the line of the comment that ended it.
- **Scaffold:** the generated `lib.rs` failed `cargo fmt --check` for names of
  4 characters or fewer or 40 or more; the generated CI's Linux SQLLogicTest
  step was skipped by extension-ci-tools.
- **`append_metadata`** refuses a platform group (`linux`) and a `wasm_*`
  platform without `--wasm`, and recognises a footer with an empty ABI field.
- **`validate_semver`** refuses numeric pre-release identifiers with leading
  zeros; **`validate_spdx_license`** names the canonical spelling for a
  case-only difference.
- **ABI refusal for a development engine** no longer suggests a declaration
  the build ignores; `build.rs` warns about a malformed
  `QUACK_RS_TARGET_DUCKDB_VERSION`.

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
  semantics in one line; the new typed constructors (see Added) get it right by
  construction; the docs, the book chapter and `LESSONS.md` now state what
  `DuckDB` does, with the source quoted. A regression test pins the behaviour.
  Aggregates are no different: their `update` receives NULL rows under either
  setting too (Pitfall L12; see Added and the aggregate entry below).

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

- **Wrong answers, no error:**
  - A NULL row of a `STRUCT` result kept its fields valid, so `(f(x)).a`
    returned the stale field value instead of NULL.
  - A typed table function failed the second time a plan ran
    (`PREPARE … ; EXECUTE p; EXECUTE p;`, or a recursive CTE): `init` moved the
    state out of bind data DuckDB reuses for every execution.
  - `Value` getters cast the value in place: `Value::double(1.5).as_i32()` turned
    the value into `DOUBLE 2.0`. Getters now read a private copy.
  - `BindInfo::add_result_column` with a type containing `ANY`/`INVALID` was
    dropped by DuckDB, shifting every later column; it is now a bind error.
  - `datetime::time_tz_bits` silently corrupted an out-of-range offset.
- Duplicate overload signatures in a scalar or aggregate function set are
  rejected at `register`, naming both overloads, instead of registering and then
  failing every call with "Could not choose a best candidate function".
- `Expression::fold` returns `Err` for a non-foldable expression instead of
  `Ok` with a null-handle `Value`.
- `CastFunctionBuilder::register` leaked `extra_info` when DuckDB rejected an
  `ANY`/`INVALID` type; those are now rejected before anything is handed over.
- `ReplacementScanInfo::set_error("")` was ignored by DuckDB, so the query fell
  through to "table does not exist".
- `SqlMacro` with a SQL keyword as a name or parameter produced a parser error.
- Documentation that was false against the code or DuckDB: `set_max_threads`
  (it does not need `local_init`); `DbConfig::set` accepts unknown option names; Pitfall L4 (a skipped
  `ensure_validity_writable` silently drops the NULL rather than segfaulting);
  the book's `panic = "abort"` advice (it must be `"unwind"`, which the crate
  itself enforces); several README and book examples that did not compile; and
  the stable ABI prefix, which is ABI-identical since v1.2.0 but not
  byte-identical (two slots were renamed `varint` → `bignum` in v1.4.0).
- `ClientContext`'s constructors now state that the context must not outlive its
  connection: DuckDB's wrapper holds a reference, not an owner.

- **Pitfall L10** (`LESSONS.md`, `book/src/reference/pitfalls.md`) — scalar bind
  data is dropped when `DuckDB` copies a bound expression. The book's pitfall
  summary table was also missing L8 and L9; all three rows are now there.
- **`ScalarBindInfo::set_bind_data_copy`** — scalar bind data was silently lost
  whenever `DuckDB` copied a bound expression. `CScalarFunctionBindData::Copy()`
  populates the copy's bind data only if a copy callback is registered, and
  quack-rs never exposed the setter, so `get_bind_data` could return null on a
  copied expression: a wrong answer, not a crash.
- **An invalid `mutants.yml` that silently disabled the mutation gate.** A
  shell comment added earlier in this branch wrote out an empty workflow
  expression while explaining not to pass values that way. GitHub evaluates
  expressions anywhere in the file, including inside shell comments, so the
  empty one invalidated the whole workflow:

  ```
  Invalid workflow file: .github/workflows/mutants.yml
  (Line: 203, Col: 14): An expression was expected
  ```

  This fails silently by design: GitHub records a run with **zero jobs** and the
  workflow stops running. `mutants-incremental` therefore stopped executing on
  pull requests while every other check stayed green — the same "a gate that is
  not actually running" failure this release is otherwise about, introduced by
  this branch rather than found in it.

  `scripts/check-workflow-expressions.py` now runs in the `doc` job and rejects
  this class. It fails on the exact commit that broke and passes on the fix.
  Neither `yaml.safe_load` nor a JSON-Schema check catches it, because both
  treat the `run:` block as an opaque string.
- **22 `assert!(x.is_empty())` / `assert!(!x.is_empty())` assertions** that beta
  clippy's new `assert_is_empty` / `assert_is_not_empty` lints reject, across 10
  files. These are not style noise: `clippy-beta` was running *stable* clippy
  before this release, so it had never reported them, and when beta promotes to
  stable the **blocking** `clippy` job inherits every one. Each now uses
  `assert_eq!` / `assert_ne!` against an empty value, which is what the lint
  asks for and prints the actual value on failure.
- **Four CI quality gates were testing nothing**, each verified against the
  files rather than inferred:
  - The MSRV job (`ci.yml`) and the release gate's MSRV entry ran a bare
    `cargo check` after selecting 1.86.0. `rust-toolchain.toml` pins
    `channel = "stable"` and a toolchain file overrides the rustup default, so
    both ran stable. Now `cargo +1.86.0 check`.
  - `clippy-beta` ran stable clippy for the same reason.
  - Miri ran with default features, `cfg`-ing out every `duckdb-1-5*` module —
    including `src/arrow.rs`, the largest block of pure-Rust `unsafe` here.
  - Doctests were never compiled: every invocation used `--all-targets`, which
    excludes them. All 183 pass.
- `release.yml` still used `fail-fast: true`, the setting `LESSONS.md` blames
  for a release that shipped with two platforms broken.
- `MUTANTS_EXIT` captured `tee`'s status rather than cargo-mutants'.
- `SECURITY.md` recommended `panic = "abort"`; that makes `catch_unwind` inert
  and disables the crate's entire panic-containment mechanism. `Cargo.toml` has
  always set `unwind`, and `validate_release_profile` rejects `abort`.
- `RELEASING.md` Step 1 listed 9 check names against 30 CI jobs, and
  `.github/workflows/README.md` listed 14. A maintainer following either could
  tag with Miri, LeakSanitizer, `osv-scan` or `semver` red. The workflow README
  now carries a table generated from `ci.yml`, with the generator inline.

- **An aggregate's `update` receives NULL rows; the documentation said it
  did not.** `NullHandling`, the `null_handling` setter on both aggregate
  builders, the null-handling book page and the first-extension tutorial said
  that DuckDB skips NULL rows before an aggregate's `update` unless
  `SpecialNullHandling` is set. DuckDB's `CAPIAggregateUpdate` passes every row of
  the chunk to `update`, NULL rows included, under `DefaultNullHandling` and
  `SpecialNullHandling` alike; for an aggregate, DuckDB reads the setting only
  when decorrelating a correlated subquery. An `update` that reads a row without
  checking its validity reads a meaningless value for every NULL row. The docs
  now say so, the new Pitfall L12 gives the symptom and the fix (skip rows whose
  `is_valid` is false), and the end-to-end test
  `aggregate_update_receives_null_rows_under_either_null_handling` pins the
  behaviour. L12 also records that, under either setting, a count-like
  aggregate in a correlated subquery returns NULL, not 0, for an outer row with
  no match; DuckDB rewrites that NULL to 0 only for its own `count`.
- **Wrong answers, no error, third pass:**
  - Once a row exceeded `ListBuilder`'s child-capacity ceiling, `push_row` /
    `push_map_row` wrote no list entry for it or for any later row of the chunk.
    DuckDB reuses output vectors, so those rows returned the previous chunk's
    lists as valid values. A refused row is now NULL.
  - `VectorWriter::write_varchar` / `write_blob` stored a value longer than
    `u32::MAX` bytes as a truncated string, for `VARCHAR` possibly cut inside a
    UTF-8 sequence: a 4 GiB + 1 byte value came back one byte long.
  - A panic in a `cast_callback!` body under `TRY_CAST` left the output vector
    as it was, so `TRY_CAST` returned zeros or a previous query's values rather
    than NULL: DuckDB ignores a cast's return value in `TRY` mode and nulls only
    the rows passed to `set_row_error`. The macro now marks every row of the
    chunk as an error.
  - Two overloads of a scalar set that differ only in their varargs type, such
    as `f(BIGINT)` and `f(BIGINT, BIGINT...)`, were rejected as duplicates. The
    varargs type is now part of the signature, as it is in DuckDB.
- A `cast_callback!` body that returned `false` without calling `set_error`
  failed a regular `CAST` with `Conversion Error: ` and no text. The macro now
  sets `callback::CAST_FAILED_WITHOUT_MESSAGE` before running the body; the
  body's own `set_error` replaces it. Hand-written cast callbacks are unchanged.
- `set_error("")` on a table function, cast or copy function reached the user
  as an error with no text after the prefix (`Binder Error: `,
  `Conversion Error: `); the new `EMPTY_ERROR_PLACEHOLDER` is reported instead.
- An error message containing a NUL byte lost everything after it on some paths
  and kept it on others. Every path now replaces the NUL with `?`: `set_error` on
  scalar, aggregate, table, cast, copy and replacement-scan functions,
  `ExtensionError::to_c_string`, `ErrorData::new`, and the entry point's report
  of a failed registration.
- `Expression::fold` errors carried DuckDB's exception serialized as JSON
  (`{"exception_type":"Conversion","exception_message":…}`) with the type
  `InvalidInput`. They now carry the plain message and the matching
  `DuckDbErrorType` (`Conversion`, `OutOfRange`, …).
- With `duckdb-1-5`, a typed table function whose bind declares no column fails
  with an ordinary bind error; DuckDB raised an `INTERNAL Error` with a C++
  stack trace.
- `FileFlag::CreateNew` did not create a file: it set only DuckDB's exclusive
  flag, which is ignored without the create flag, so an existing file opened and
  a missing one failed. `set_flag(FileFlag::CreateNew, true)` now also sets
  `Create`.
- `WarningCollector` dropped every warning, silently, once a panic had poisoned
  its lock. It now recovers the lock and keeps working.
- `ScalarFunctionBuilder::varargs` / `ScalarOverloadBuilder::varargs` with a
  composite `TypeId` (`TypeId::List`, …) panicked inside the setter. `register`
  now refuses it with an error naming the varargs slot.
- `ScalarFunctionSetBuilder` and `AggregateFunctionSetBuilder` check every
  overload for a return type and its required callbacks before creating any
  DuckDB handle, and the error names the overload index; the scalar set used to
  report "overload missing function callback" with no index.
- `AbiPolicy::Strict`'s refusal of an unknown engine recommended
  `AbiPolicy::AllowUnknownEngine`; it now recommends a build that uses only the
  stable C API. The layout-mismatch message no longer advises rebuilding a
  `duckdb-1-5` extension against a pre-1.5 engine.
- `append_metadata` rejected a 32-byte footer value, which DuckDB reads in full
  without a terminating NUL, and it now also accepts `--option=value`.
- `examples/parse_descriptions.rs`, pointed at a community-extensions checkout
  (`extensions/<name>/description.yml`), found no files and reported
  "0 parsed, 0 rejected" with exit status 0. It now reads that layout, fails
  when it finds no files, and exits non-zero when any file is rejected.
- **Documentation that was false against the code or DuckDB, third pass:**
  - Aggregates: the `state_size` callback runs whenever an operator sizes a
    state buffer, not once at registration; `finalize` runs once per result
    batch; `destroy` also runs on the source states of `combine`; `combine`
    targets are initialised by the `init` callback (`StateInitFn`;
    `T::default()` for `FfiState<T>`), not zeroed;
    `update` receives one state pointer per input row, not per group.
  - Scalars: the `null_handling` setters repeated the claim Pitfall L8
    disproved (that DuckDB skips the callback for NULL arguments). Identical
    calls share bind data: for `SELECT f(i), f(i)` the bind callback runs
    twice, but both columns use the data from its first run, so a bind that
    reads a counter, clock or RNG needs `volatile`. The `register` methods now
    state DuckDB's collision rules: an aggregate cannot take a name already
    used by a scalar function, aggregate or macro, while a scalar merges into
    an existing function of the same name (an identical signature is now
    refused, see Changed). The entry-point
    docs say that registration is not transactional: functions registered
    before the registration closure fails stay registered.
  - Table and copy functions: `set_cardinality`'s `is_exact` works the other
    way round from `duckdb.h`; `local_init` does not enable parallelism, only
    `set_max_threads` does; table `extra_info` must point to `Send + Sync` data;
    `CopyBindInfo::options`' shape is now described.
  - Casts: `set_row_error` requires `row < count`, because DuckDB checks that
    bound only in debug builds; the cast docs said `false` becomes NULL under
    `TRY_CAST`, which holds only for rows passed to `set_row_error`.
  - Queries and values: a multi-statement string passed to `query()` /
    `execute()` runs every statement and returns the first row-producing
    statement's result; every prepared parameter has a name (a positional `?`
    is named by its position); `DbConfig::get_flag` returns the extension's
    name, not a description, for an extension setting; `check_valid_utf8`
    agrees with `std::str::from_utf8` rather than being stricter.
  - Dates and intervals: `date_from_days` of `infinity` gives 5881580-07-11;
    DuckDB's 30-day month applies to
    interval comparison and `epoch_us`, not to interval arithmetic or `epoch`;
    `DuckInterval`'s `Eq` compares fields, so `1 month` differs from `30 days`
    here although SQL calls them equal.
  - `InstanceCache::get_or_create` returns an error for a different config on
    an already-open database (the docs said the config was ignored), and never
    caches in-memory paths. `FileSystem`'s `set_flag(flag, false)` does not
    clear a flag. `SqlMacro` documents where the macro is created, that it
    persists in a database file and can replace a user's macro or shadow a
    built-in; its parameter errors say "parameter name".
  - Table macros read a table parameter through `query_table(tbl)`: the
    `SqlMacro` examples wrote `FROM tbl`, which DuckDB binds at creation time
    to a table literally named `tbl`, so the module example failed to register.
  - Loading: the getting-started pages and Pitfall P3 said to `LOAD` a bare
    `.so`, which every supported DuckDB refuses; they now show the
    `append_metadata` step and `duckdb -unsigned`, and no recipe sets
    `allow_extensions_metadata_mismatch` any more, since a correctly stamped
    `C_STRUCT` build loads without it. Pitfall P2's symptom was wrong: a bad
    `-dv` is stamped without complaint and `LOAD` refuses the file.
  - The book's Known Limitations page suggested approximating window semantics
    with aggregate functions, the exact shape that crashes every C API
    aggregate (Pitfall L11); that page, the README and the hello-ext README now
    warn about it. The hello-ext README also stopped telling readers to verify
    `panic = "abort"` and to add `duckdb` with `bundled` as a dev-dependency
    (Pitfall P9).
  - The installation page's MSRV rationale, `SECURITY.md`'s claim that
    `TlsConfigProvider` enforces TLS 1.2+ by default (`min_tls_version` is a
    required method), the entry-point page's macro expansion, the crate's and
    FAQ's pitfall counts, and the README's validated-fields table (an unlisted
    SPDX id is a warning, not a rejection).
  - Book examples that did not compile or did not do what they said: among
    them a `DuckStringView` example using the deprecated `from_bytes` (it
    returns `None` for any string over 12 bytes), a `my_bind` that leaked the
    `duckdb_value` from `duckdb_bind_get_parameter`, a README `description.yml`
    example that did not compile, and scaffold examples that, run as written,
    overwrote the current package's `Cargo.toml` and `src/lib.rs`. Broken links
    in the book are fixed, and hand-kept test counts, several of them wrong, are
    removed from the repository trees.
- **`# Safety` contracts that allowed undefined behaviour** (found while
  documenting every `unsafe` block; contract text only, no signature or
  behaviour change):
  - `FfiLocalInitData::get` / `get_mut`, `FfiInitData::get_mut` and
    `FfiBindData::get_from_init` / `get_from_function` did not require `T` to
    be the type passed to `set`; `set::<u8>` then `get_mut::<[u64; 64]>` met
    every stated clause and wrote 512 bytes through a 1-byte allocation. The
    init-data getters also did not bound the returned lifetime by the scan
    call. Both requirements are now stated.
  - `MapVector::set_size` did not require the size to equal the entries
    written, so a larger size made DuckDB read past the child vectors.
  - `read_duck_blob` did not require the vector to outlive the returned slice,
    which borrows the vector's own buffer for a blob of 12 bytes or fewer.
- Documented: when an aggregate's `finalize` reports an error, DuckDB 1.5.5
  does not destroy every state the query created (ungrouped: 2 initialised, 1
  destroyed; grouped: 4 and 2), so whatever those states own leaks — an
  `FfiState<T>` box per abandoned state. Nothing in an extension can detect it;
  Known Limitations and `AggregateFunctionInfo::set_error` now say so, and an
  end-to-end test pins it. Two of this release's own tests leaked on these
  paths and made the LeakSanitizer job fail; both now use states that own
  nothing.
- `cargo doc` failed with default features on a link to
  `PreparedStatement::execute_streaming`, which needs `duckdb-1-5`. CI now also
  builds the docs with default features, the set a dependent crate documents.
- `Cargo.toml`'s `duckdb-1-5` description gave the requirement as
  `libduckdb-sys >= 1.5.0` (the crate is versioned 1.10500.0) and claimed the
  feature has no effect against 1.4.x, which was never checked; the hello-ext
  README said `varargs` and `volatile` need `duckdb-1-5`.

### Security

#### Fourth audit

- **An Arrow import with a large dictionary wrote past a heap buffer.**
  `data_chunk_from_arrow` on a dictionary-encoded array with NULLs and more
  than 2048 entries — a `LIST` of 1025+ two-element dictionary-encoded lists
  is enough — made `DuckDB` overflow a validity mask (valgrind: invalid write
  in `GetValidityMask`; SIGSEGV or SIGABRT on 1.4.4, 1.5.0 and 1.5.5). Such
  arrays are refused before `DuckDB` is called.
- **An Arrow import read out of bounds.** A dictionary-encoded or null-type
  column came back as a dictionary or constant vector, which every quack-rs
  reader reads as flat: wrong values, then reads past the buffer. Those
  columns are copied into flat vectors before `data_chunk_from_arrow` returns.
- **Rendering a timestamp from SQL aborted the process.**
  `make_timestamp(-9223372036854775808)` passed to a table function, then
  `Value::as_str`, `display_string` or `{:?}`: `DuckDB`'s rendering threw
  through the C API (exit 134, 1.4.4 to 1.5.5), also inside a LIST, STRUCT or
  MAP. Every temporal payload is checked first; `as_str` returns
  `Err(UNRENDERABLE)`. `as_time` and the other converting getters no longer
  hand such a payload to `DuckDB`'s cast, which overflowed a signed multiply.
- **A scalar bind callback inspecting a subquery argument aborted the process
  on DuckDB 1.5.0 to 1.5.4.** Those releases copy the argument outside any
  `try`; `SELECT f((SELECT 1))` threw a C++ exception through the callback.
  **Breaking:** `ScalarBindInfo::argument` now asks for nothing on those
  releases and fails the bind with an explanation; `get_argument`'s Safety
  section states the requirement.
- **An out-of-range TIME or timestamp crashed `DuckDB` later.**
  `PreparedStatement::bind_time` / `bind_timestamp` / `bind_timestamp_tz` and
  `Appender::append_time` / `append_timestamp` accepted any payload;
  `i64::MIN` as a TIME segfaulted when rendered, and at the append itself
  into a VARCHAR column. They are refused (**Breaking**). `bind_str`,
  `bind_blob` and `append_bytes` refuse more than 4 GiB, which `DuckDB`
  stored modulo 2^32 (a 4 GiB + 3 byte blob became 3 bytes).
- **A panic in a scalar function's bind or init callback aborted the
  process.** New `scalar_bind_callback!` / `scalar_init_callback!` macros
  (`duckdb-1-5`) catch it and fail the query with its message; a panic with
  an empty message reports `EMPTY_PANIC_PLACEHOLDER`.
- **A null `get_api` or `access` from `DuckDB` panicked or crashed the entry
  point.** `init_extension` checks both before `libduckdb-sys` unwraps them.
- **A literal type in a registration invalidated the database.**
  `TypeId::StringLiteral` / `IntegerLiteral` as a parameter, return or
  registered type: the first query that used it raised an internal error and
  every later query failed. `LogicalType::try_new` refuses both.
- **`FfiState` destroyed states `init` never ran on.** After a `state_init`
  error, `DuckDB` passes never-initialised states to `destroy`; each state now
  carries an address-derived tag that `destroy` checks (**Breaking:**
  `FfiState<T>` is two words).
- **`SecretEntry` left secret bytes in spare capacity** after truncation;
  zeroisation now covers the whole allocation.

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

- **Soundness: safe code could corrupt memory, race, or read freed memory.**
  - `FileSystem` held a raw `ClientContext` pointer with no lifetime, so it
    could be used after its connection closed and read freed memory (confirmed
    under valgrind). **Breaking:** `FileSystem<'ctx>`.
  - `SelectionVector::new` exposed uninitialised memory through the safe
    `as_slice()` (stale `0xDEADBEEF` observed), and a large length made DuckDB
    compute a wrapped allocation size, so safe indexing segfaulted.
    **Breaking:** it returns `Result`, rejects lengths above `MAX_LEN` before
    DuckDB is called, and zeroes the buffer.
  - `datetime::decimal_to_f64` read past DuckDB's powers-of-ten tables for
    `scale > 38`.
  - `FfiBindData`, `FfiInitData` and replacement-scan data are shared across
    threads by DuckDB. **Breaking:** `FfiBindData::set` / `FfiInitData::set` /
    `register_with_data` require `T: Send + Sync`, `FfiLocalInitData::set`
    requires `T: Send`.
- **Process aborts from ordinary input.** Each of these let a DuckDB C++
  exception unwind into Rust ("Rust cannot catch foreign exceptions"), killing
  the host process; each is now validated in Rust first and reported as an
  error or `None`:
  - every `Value::as_*` getter (and the `_or` forms) on a SQL `NULL` — e.g. a
    table function called with `f(n := NULL)`; a null handle was dereferenced;
  - `datetime::date_to_days` on an invalid date, `timestamp_from_micros` /
    `timestamp_to_micros` on infinities and the far-negative range;
  - `datetime::time_from_micros` / `time_tz_from_bits` on a time outside
    `00:00:00`–`24:00:00`, in a DuckDB built with assertions (a debug build,
    as the `bundled-test` feature compiles), which fails `Time::Convert`'s
    `D_ASSERT`; a release build returned out-of-range fields;
  - `SelectionVector::new` above DuckDB's allocation limit;
  - a config option whose default does not cast to its type;
  - catalog lookups for `Schema`, `Database`, `PreparedStatement` and
    `Invalid` entry types;
  - a panic whose payload's own `Drop` panics (`panic_any(value)`), in every
    callback macro, `catch_ffi_panic`, the typed scalar and typed table
    trampolines and the entry point's registration guard;
  - the entry points dereferenced a NULL `duckdb_database*` when DuckDB's
    `get_database` failed.
- **Documented, not fixable here: C API aggregates crash under
  `agg(x) OVER ()` and `agg(x ORDER BY y)`.** DuckDB's `CAPIAggregateUpdate`
  does not flatten the state vector, and the window-constant and
  sorted-aggregate executors pass a one-element state array with `count > 1`,
  so every aggregate registered through the C API — not only quack-rs's —
  reads out of bounds. Reproduced in plain C against DuckDB 1.4.4, 1.5.0 and
  1.5.5; reported upstream as
  [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109). Documented on `AggregateFunctionBuilder`,
  `AggregateFunctionSetBuilder`, `FfiState`, the aggregate book pages and as
  Pitfall L11.

- **The one active advisory suppression is gone, because the crate behind it
  is.** RUSTSEC-2026-0235 (`rkyv` 0.7.46) was suppressed in `osv-scanner.toml`,
  reachable only as quack-rs → `duckdb` → `rust_decimal` → `rkyv`. In `duckdb`
  1.10505.0 `rust_decimal` became an **optional** dependency (it was required
  in 1.10504.0), and quack-rs does not enable it — so neither crate is in
  `Cargo.lock` any more. Both the suppression and the CI step that re-proved it
  have been removed.
- **`cargo deny` was scanning the wrong dependency graph.** `deny.toml` had
  `graph.all-features = false`; since `duckdb` is optional and the crate
  declares no `default` feature, neither the advisory scan nor the license scan
  ever evaluated `duckdb`, `arrow`, `chrono` or `rust_decimal`. Now
  `all-features = true`.
- `ci.yml` and `mutants.yml` — the two workflows that build and execute
  pull-request code — had no `permissions:` block, so the token inherited the
  repository default. Both now take `contents: read`.
- `mutants.yml` interpolated a PR-derived file list straight into a `run:`
  block, where `$(...)` expands before bash parses the script. Moved to `env:`.
- `persist-credentials: false` on all 47 `actions/checkout` steps.

- **Soundness, third pass: more ways safe code, or a malformed input, could
  corrupt or read freed memory.**
  - A `FileHandle` could outlive its database and read freed memory.
    **Breaking:** `FileHandle<'fs>` (see Changed).
  - DuckDB v1.4.5 was missing from the ABI layout table, so a `duckdb-1-5`
    build loaded into it was treated as an unknown engine instead of a layout
    mismatch, and under `AbiPolicy::AllowUnknownEngine` it loaded and
    segfaulted. v1.4.5 is now in the table, so such a build is reported as a
    layout mismatch there, as it is on v1.4.4.
- **Process aborts, third pass.** Each of these killed the host process:
  - `ClientContext::config_option` on an option whose value is NULL:
    `enable_profiling` before it is set, or any option after
    `SET <option> = NULL`. It now returns `None`.
  - A catalog `Type` or `Collation` lookup of a name that makes DuckDB autoload
    an extension, when the autoload fails. **Breaking:** now refused (see
    Changed).
  - A config option default that SQL's `TRY_CAST` accepts but DuckDB's built-in
    cast does not, such as a `TIMESTAMPTZ` default with a time-zone name while
    ICU is loaded. `ConfigOptionBuilder::register` now converts the default
    through the connection's own `TRY_CAST` and hands DuckDB a `Value` of the
    option's type, so no cast runs inside the C API.
  - `Value` temporal getters whose cast DuckDB implements by throwing: for
    example `as_time()` of `'infinity'::TIMESTAMP` (reachable from a table
    function's named parameter) or `as_timestamp_ns()` of any `TIMESTAMP` after
    2262. They now return `None` where DuckDB would throw, and also for a result
    outside the target type's range, which DuckDB could produce but not render.
  - Rendering a temporal `Value` built from an out-of-range payload.
    **Breaking:** the constructors now validate (see Changed).
  - The `AbiPolicy::Warn` diagnostic used `eprintln!`, which panics when
    writing to stderr fails, outside the entry point's panic guard. A failed
    write now loses the warning instead.
  - `validate_spdx_license` on deeply nested parentheses (stack overflow).
    **Breaking:** nesting is capped at 64 (see Changed).
- **`SqlMacro::register` ran every statement in a body.** A body of
  `1); DROP TABLE t; SELECT (1` registered and dropped the table. **Breaking:**
  a body with more than one statement is now refused (see Changed).
- `SecretEntry::with_field` on an existing key, `with_provider` and
  `with_scope` freed the value they replaced without zeroizing it (and
  `with_field` also the duplicate key), although `Drop` zeroizes those same
  values. Each now zeroizes before replacing, verified by a test that inspects
  every buffer as it is freed.

### Dependencies

- `libduckdb-sys` / `duckdb` 1.10504.0 → **1.10505.0** (DuckDB 1.5.4 → 1.5.5),
  `cc` 1.2.64 → 1.4.7, `arrow` 58.1.0 → 58.4.0. The relock removed **100
  packages net**: `libduckdb-sys` 1.10505.0 swapped its `reqwest`
  build-dependency for `ureq`, taking the hyper/tokio/quinn/rustls trees with
  it. MSRV is unchanged at 1.86.0.
- **No `duckdb-1-5-5` feature was added, deliberately.** DuckDB 1.5.5 adds no C
  Extension API surface: `extension_api.hpp` is byte-identical between v1.5.4
  and v1.5.5 (sha256 `0232a22a…3017031`, 89456 bytes, 546 function pointers in
  each). There would be nothing to gate. See the note in `Cargo.toml`.
- GitHub Actions, each SHA resolved against the upstream tag: `actions/checkout`
  v7.0.0 → v7.0.1, `Swatinem/rust-cache` v2.9.1 → v2.9.2,
  `codecov/codecov-action` v7.0.0 → v7.1.1,
  `actions/attest-build-provenance` v4.1.1 → v4.2.2, `actions/deploy-pages`
  v5.0.0 → v5.0.1. The `actions/configure-pages` pin was already v6.0.0; only
  its comment said v5.0.0.
- Pinned the four CI tools installed unpinned (`cargo-mutants` 27.1.0,
  `cargo-semver-checks` 0.50.0, `cargo-llvm-cov` 0.9.1, `cargo-fuzz` 0.13.2).
  `mutants.yml` reasons about behaviour introduced in cargo-mutants 27.0.0,
  which held only by luck of whatever `cargo install` fetched. Every
  `cargo install` step now also clears the workflow-wide
  `RUSTFLAGS: "-D warnings"`, which was compiling third-party trees with
  warnings-as-errors.
- `dtolnay/rust-toolchain` is pinned to a commit on the action's `master`
  branch in every workflow and in the workflow `generate_scaffold` writes. The
  old pin was a commit of its regenerated `stable` branch that no ref reaches
  any more, so GitHub may garbage-collect it. Every use now passes `toolchain:`
  explicitly, as `master`'s `action.yml` requires; the pin does not pin the Rust
  version.

## [0.16.0] - 2026-08-19

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

#### Portability and feature-combination breakage

- **`MAX_LIST_CHILD_CAPACITY` was typed `usize`, making the crate fail to
  compile for `wasm32`.** `DuckDB`'s `DConstants::MAX_VECTOR_SIZE` is
  `1ULL << 37ULL` — an `idx_t`, not a pointer-sized value. As a `usize` const,
  `1 << 37` is a const-eval overflow wherever pointers are 32 bits, which is
  every `wasm32` target — and `DuckDB`'s own extension CI builds three of them.
  Now typed `u64`, with a separate `usize`-clamped constant for the capacity
  arithmetic; on a 32-bit target the ceiling is larger than any allocation
  `usize` can describe, so `usize::MAX` is the real limit.

- **Three unit tests called `duckdb-1-5`-gated methods without a `cfg` gate**,
  breaking `--features bundled-test` on its own — a combination that builds the
  live-DuckDB tests but not the 1.5 wrappers. `Value::display_string` and
  `TableDescription::column_count` / `column_type` are the gated methods; the
  assertions around them are now gated too.

  Both defects compiled cleanly under every other feature combination.
  `scripts/check-matrix.sh` now runs the combinations CI runs — including
  `bundled-test` alone and the `wasm32` legs — in one command.

#### Security scanning

- **`Security (OSV / GHSA)` failed on `RUSTSEC-2026-0235` (`rkyv` 0.7.46), an
  advisory that no build of this crate can reach.** `osv-scanner` reads
  `Cargo.lock`, and Cargo pins optional dependencies there whether or not their
  feature is enabled, so the job flagged a crate that is never compiled. The
  chain is quack-rs → `duckdb` (optional dependency, enabled by `bundled-test`)
  → `rust_decimal` 1.40.0 → `rkyv` (optional feature of `rust_decimal`, not
  enabled). `rust_decimal` itself *is* built; `rkyv` is not. It is also not
  fixable here — `rust_decimal` 1.40 constrains `rkyv` to `^0.7`, and the fixed
  version is 0.8.17. The advisory is present on `main` as well.

  Suppressed via a new `osv-scanner.toml` carrying the full reachability
  argument. The suppression does not rest on that comment staying true: the
  `osv-scan` job now re-derives it on every run, failing the build if an `rkyv`
  node ever appears in `cargo tree --all-features --target all`. The check
  tests the *forward* tree, because `cargo tree -i` exits 0 with "nothing to
  print" for a lockfile-only package and so cannot tell "not built" from
  "built" by exit code; it also anchors on `rust_decimal` being present, so a
  truncated or failed tree reports as unverified rather than as clean.

#### CI guards

- **`scripts/check-abi-table.py` treated an unreachable release header as
  proof the release did not exist.** Its `fetch` swallowed every exception —
  404, timeout, DNS, 5xx alike — and returned `None`, which the caller printed
  as "not published (skipped)" and dropped from the derivation. One transient
  failure on `v1.4.4` therefore narrowed the derived range from `v1.4.0–v1.4.4`
  to `v1.4.0–v1.4.3` and failed CI reporting `src/abi.rs` as stale. Following
  that advice would have **shrunk the layout table and made the runtime guard
  refuse `DuckDB` versions it should accept** — the exact failure the table
  exists to prevent.

  A definite 404 is now distinguished from every other failure, transient
  errors are retried, and a tag that could not be downloaded suspends the
  staleness comparison (exit 2, "could not check") instead of failing it. The
  other three guards each fetch a single file, so an empty fetch already meant
  no data rather than partial data.

- **All four guard jobs treated exit 2 as a failure.** The scripts document it
  as "upstream unreachable, could not check", but the workflow ran them bare,
  so any non-zero failed the job — making every guard a network-flake away from
  a red build. They now surface exit 2 as a warning and fail only on exit 1.

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

## [0.15.0] - 2026-07-16

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

## [0.14.0] - 2026-06-07

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

## [0.13.0] - 2026-05-24

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
  (40) type-enum values as `TypeId::Variant` and `TypeId::Geometry`, with the
  matching `to_duckdb_type` / `from_duckdb_type` / `sql_name` / `Display`
  coverage. It is a separate gate because these constants postdate the
  `duckdb-1-5` feature's 1.5.0 floor and require `libduckdb-sys >= 1.10503.1`;
  keeping them out of `duckdb-1-5` preserves compatibility for consumers pinned
  to libduckdb-sys 1.5.0–1.5.2.
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
  the test submodule files — closing the last gaps against the crate's own
  "every file / every `unsafe` block" conventions.
- Corrected the README install note (it claimed v0.11.0 was the latest published
  crate; v0.12.1 was in fact already on crates.io) and bumped install-example
  version references throughout the README, book, and scaffold template to `0.13`.

### CI

- **docs.rs now builds with `duckdb-1-5-3`** (`[package.metadata.docs.rs]`), so
  the feature-gated modules (`appender`, `error_data`, `file_system`, …) and the
  new `TypeId` variants render on docs.rs and the README's docs.rs links resolve.
  Previously docs.rs built the empty default feature set and omitted them.
- **CI exercises the `duckdb-1-5-3` feature** — the feature job now runs
  `check` / `test` / `clippy` for `duckdb-1-5-3` alongside `duckdb-1-5`, and the
  `Clippy (beta)` and `doc` jobs use `duckdb-1-5-3`.
- **Fixed the `Nightly` CI job silently running stable** — the SHA-pinned
  `dtolnay/rust-toolchain` step lacked `with: toolchain: nightly`, so it fell
  back to the `rust-toolchain.toml` stable channel (the same class of bug
  previously fixed for the MSRV job).
- **Mutation testing scoped to testable code** — DuckDB FFI-wrapper modules
  whose methods require a live runtime (and whose tests are `bundled-test`-gated
  or absent) are excluded from `cargo mutants`, since their mutants can't be
  killed by unit tests. This extends the existing exclusion pattern to the 1.5.x
  wrappers — `expression`, `file_system`, `appender`, `selection_vector`,
  `instance_cache`, `table_description`, and the scalar/copy `*Info` accessors.
  Pure-logic code (e.g. `DuckDbErrorType`, the `TypeId` conversions) stays in
  scope. The mutants feature set is bumped to `duckdb-1-5-3`.

## [0.12.1] - 2026-05-01

### Security

Closes nine GitHub Dependabot alerts (two High, seven Low) split across
the workspace `Cargo.lock` and `examples/hello-ext/Cargo.lock`.

- **`rustls-webpki` 0.103.10 → 0.103.13** — picks up the fix for three
  RustSec advisories reachable via the `bundled` DuckDB build's transitive
  `reqwest` → `rustls` chain:
    - **[RUSTSEC-2026-0098]** ([GHSA-965h-392x-2mh5]) — `nameConstraints`
      with URI name restrictions were silently ignored instead of enforced.
      Patched in 0.103.12+; the URI-name path is not on the public Web PKI,
      so impact is limited to private-PKI consumers.
    - **[RUSTSEC-2026-0103]** ([GHSA-xgp8-3hg3-c2mh]) — name-constraint
      enforcement accepted certificates asserting a wildcard subject name.
      Reachable only after signature verification and requires misissuance.
      Patched in 0.103.12+.
    - **[RUSTSEC-2026-0104]** — reachable panic when parsing certificate
      revocation lists with a syntactically valid empty `BIT STRING` in
      the `onlySomeReasons` element of an `IssuingDistributionPoint` CRL
      extension. Affects only applications that use CRLs. Patched in
      0.103.13+.

  Neither path is exercised by `quack-rs` itself, but the advisories trip
  `cargo deny` for any downstream consumer that has not yet bumped, so
  shipping a release that resolves them is the path of least friction.

- **`rand` 0.9.2 → 0.9.4 / 0.8.5 → 0.8.6** — picks up the fix for
  **[RUSTSEC-2026-0097]** ([GHSA-cq8v-f236-94qc]) — `ThreadRng` could
  produce an aliased `&mut BlockRng<ReseedingCore>` (Stacked-Borrows UB)
  when a custom logger reentered `rand::rng()` from inside a reseed at
  trace-level logging. Triggering the unsoundness requires a custom
  global logger that pulls from `rand::rng()` while reseeding, which is
  not a pattern `quack-rs` uses, but the advisory matches by version
  range so resolving it removes the alert noise. Patched on every
  affected line: 0.8.6+, 0.9.3+, 0.10.1+.

### Changed

- **Workspace `Cargo.lock` bumps** —
    - `cc` 1.2.59 → 1.2.61 (build-dep; no API impact)
    - `duckdb` / `libduckdb-sys` 1.10501.0 → 1.10502.0 (latest patch
      release; no API impact for `quack-rs`)
    - `rand` 0.8.5 → 0.8.6 (transitive via `rust_decimal`; security)
    - `rand` 0.9.2 → 0.9.4 (transitive via `proptest` dev-dep; security)
- **`examples/hello-ext` `Cargo.lock` bumps** —
    - `libduckdb-sys` 1.10501.0 → 1.10502.0
    - `rand` 0.9.2 → 0.9.4 (security)
    - `rustls-webpki` 0.103.10 → 0.103.13 (security; matches workspace)

### CI

- **GitHub Actions pin updates** —
    - `actions/cache` `v5.0.4` → `v5.0.5`
    - `actions/upload-artifact` `v7.0.0` → `v7.0.1`
    - `actions/upload-pages-artifact` `v4.0.0` → `v5.0.0`

  All updates retain SHA-pinned references for supply-chain integrity.

- **New informational `Clippy (beta)` job** — runs the same
  `cargo clippy --all-targets --features duckdb-1-5 -- -D warnings`
  invocation on the `beta` Rust toolchain. Marked `continue-on-error`
  so a beta-only lint regression does not block the merge queue, but
  surfaces six weeks before the lint reaches `stable`. Originally added
  in response to `clippy::map_unwrap_or` graduating to `stable` in
  Rust 1.95.0 and biting `src/warning.rs` after the toolchain rolled
  forward.

### Fixed

- **`clippy::map_unwrap_or` on `WarningCollector::len`** —
  `self.warnings.lock().map(|w| w.len()).unwrap_or(0)` rewritten as
  `self.warnings.lock().map_or(0, |w| w.len())`. Behaviour-preserving;
  fixes `Clippy` and `Test duckdb-1-5 feature` jobs under Rust 1.95.0.
- **`clippy::map_unwrap_or_default` on `WarningCollector::snapshot`**
  (defensive) — same rewrite for the sibling
  `map(|w| w.clone()).unwrap_or_default()` call. Caught proactively
  alongside the above; otherwise would have surfaced the next time
  the lint promotion round-trips through `pedantic` or `nursery`.

[RUSTSEC-2026-0097]: https://rustsec.org/advisories/RUSTSEC-2026-0097
[RUSTSEC-2026-0098]: https://rustsec.org/advisories/RUSTSEC-2026-0098
[RUSTSEC-2026-0103]: https://rustsec.org/advisories/RUSTSEC-2026-0103
[RUSTSEC-2026-0104]: https://rustsec.org/advisories/RUSTSEC-2026-0104
[GHSA-965h-392x-2mh5]: https://github.com/advisories/GHSA-965h-392x-2mh5
[GHSA-cq8v-f236-94qc]: https://github.com/advisories/GHSA-cq8v-f236-94qc
[GHSA-xgp8-3hg3-c2mh]: https://github.com/rustls/webpki/security/advisories/GHSA-xgp8-3hg3-c2mh

## [0.12.0] - 2026-04-09

### Added

- **`TypedTableFunctionBuilder<S>` with closure-based `bind`/`scan`** —
  new high-level layer on top of `TableFunctionBuilder` that lets extensions
  register table functions via two safe Rust closures instead of hand-rolled
  `unsafe extern "C" fn` trampolines. Entry point is
  `TableFunctionBuilder::with_state::<S, _>(|bind| Ok(S { ... }))`, followed by
  `.scan(|state, chunk| { ... Ok(()) })` and `.build()?` to recover a fully
  configured `TableFunctionBuilder` usable with any `Registrar`. Highlights:
    - The `bind` closure receives `&BindInfo`, declares the output schema,
      reads parameters, and returns the typed scan state `S: Send + 'static`.
    - The `scan` closure receives `&mut S` and a `DataChunk` for the output
      chunk. Returning with chunk size zero signals end-of-stream.
    - Panics in user closures are caught via `std::panic::catch_unwind` and
      surfaced through `duckdb_bind/init/function_set_error`; the scan
      forces chunk size to zero on panic so the query terminates safely.
    - Scan state is carried from `bind` through `init` into `init_data` so
      the scan callback can hold `&mut S` without extra ceremony.
    - Because `S` is only required to be `Send`, scans are serialised by
      calling `set_max_threads(1)`. Extensions that need true multi-worker
      parallelism should continue to use the raw `TableFunctionBuilder` with
      `local_init`.
    - Re-exported from the prelude as `TypedTableFunctionBuilder`.

  This is proposal A from the duck_net "quack-rs enhancements" list and
  eliminates the raw bind/init/scan trampolines that every FFI-heavy
  extension would otherwise write by hand.

- **`ExtensionError`: additional `From` impls** — `From<std::io::Error>`,
  `From<std::ffi::NulError>`, and `From<std::fmt::Error>` allow the `?` operator
  to propagate common error types directly in `register_all()` without
  `.map_err()`. This eliminates the need for `panic!()` when operations like
  tokio runtime allocation fail during extension initialization.

- **`tls` module** — `TlsConfigProvider` trait for type-erased TLS client
  configuration injection. HTTP-capable extensions (e.g., `duck_net`) implement
  this trait to supply custom CA bundles, client certificates for mTLS, or
  restricted cipher suites through a uniform interface. Uses `std::any::Any`
  so `quack-rs` has no dependency on any specific TLS library. Security
  hardened: `client_config()` returns `Result` for fallible config creation,
  `accepts_invalid_certs()` and `min_tls_version()` enable security auditing,
  `config_type_name()` allows safe pre-downcast verification, and
  `audit_tls_provider()` integrates with the `warning` module to automatically
  flag CWE-295 (cert validation bypass) and CWE-327 (deprecated TLS versions).
  Includes `TlsVersion` enum with `is_deprecated()` and `Ord` ordering.

- **`warning` module** — structured security warning API with
  `ExtensionWarning`, `WarningSeverity` (Info/Low/Medium/High/Critical), and
  `WarningCollector`. Extensions that touch external resources emit warnings
  with machine-readable codes and optional CWE identifiers. `WarningCollector`
  is thread-safe (`Mutex`-backed) and supports `emit()`, `snapshot()`,
  `drain()`, and `clear()`.

- **`secrets` module** — `SecretsManager` trait and `SecretEntry` type for
  bridging into DuckDB's native `CREATE SECRET` storage. Extensions implement
  `SecretsManager` to provide `get_secret()`, `list_secrets()`, and
  `remove_secret()` through a safe Rust interface. `SecretEntry` uses a builder
  pattern with `with_provider()`, `with_scope()`, and `with_field()`.
  Security hardened: `Debug` redacts field values, `Drop` zeroizes sensitive
  data via `write_volatile`, `PartialEq` intentionally omitted to prevent
  timing side-channels, and fields are private with accessor methods.

- **`StructWriter::child_list_vector(field_idx)`** — semantic alias for
  `child_vector()` that makes the intent clear when a struct field has LIST
  type. Returns the raw `duckdb_vector` handle for use with `ListVector`
  methods (`reserve`, `set_entry`, `set_size`, `child_writer`, etc.).

- **Prelude additions** — `TlsConfigProvider`, `ExtensionWarning`,
  `WarningSeverity`, `WarningCollector`, `SecretEntry`, `SecretsManager`
  re-exported from `quack_rs::prelude`.

## [0.11.0] - 2026-03-30

### Added

- **`StructWriter::child_vector(field_idx)`** — returns the raw `duckdb_vector`
  handle for a struct field, enabling `ListVector`/`MapVector`/`ArrayVector`
  operations on nested complex types without raw FFI calls.

- **`StructReader::child_vector(field_idx)`** — read-side counterpart for
  accessing nested complex type fields within STRUCT input vectors.

- **`ChunkWriter::vector(col_idx)`** — raw `duckdb_vector` access for complex
  column types (LIST, MAP, ARRAY) from within a `ChunkWriter`.

- **`ChunkWriter::column_count()`** — returns the number of columns in the
  chunk without needing a separate `DataChunk`.

- **`VectorWriter::set_valid(row)`** — marks a row as non-NULL, undoing a
  previous `set_null()` call. Calls `ensure_validity_writable` automatically.

- **`StructWriter::set_valid(row, field_idx)`** — batched version of
  `VectorWriter::set_valid()` for STRUCT fields.

- **`ReplacementScanInfo::add_parameter_raw(value)`** — adds any `duckdb_value`
  as a parameter to a replacement scan redirect, enabling non-VARCHAR parameter
  types (INTEGER, BIGINT, BOOLEAN, etc.).

- **`ReplacementScanInfo::add_i64_parameter(value)`** — convenience method for
  adding BIGINT parameters to replacement scan redirects.

- **`ReplacementScanInfo::add_bool_parameter(value)`** — convenience method for
  adding BOOLEAN parameters to replacement scan redirects.

### Changed

- **`table_scan_callback!` error reporting** — the macro now extracts the panic
  message and reports it to DuckDB via `duckdb_function_set_error` before setting
  chunk size to 0. Previously, panics silently ended the stream with no error
  message visible to the user.

## [0.10.0] - 2026-03-29

### Added

- **`StructWriter`** (`vector::struct_writer` module) — batched, typed writer for
  STRUCT output vectors. Pre-creates `VectorWriter`s for all fields at construction,
  then exposes `write_bool`, `write_varchar`, `write_i64`, `write_date`,
  `write_timestamp`, `write_time`, `write_blob`, `write_uuid`, `set_null`, etc.
  Eliminates ~120 raw `duckdb_struct_vector_get_child` calls across typical extensions.

- **`StructReader`** (`vector::struct_reader` module) — batched, typed reader for
  STRUCT input vectors. Read-side counterpart to `StructWriter` with `read_bool`,
  `read_str`, `read_i64`, `read_date`, `read_timestamp`, `read_blob`, `read_uuid`,
  `is_valid`, etc.

- **`ChunkWriter`** (`chunk_writer` module) — auto-sizing chunk writer for table
  function scan callbacks. Tracks rows via `next_row()` and automatically calls
  `duckdb_data_chunk_set_size` on `Drop`, preventing forgotten-set-size bugs.

- **`scalar_callback!`** / **`table_scan_callback!`** macros (`callback` module) —
  wrap `unsafe extern "C"` callbacks with `std::panic::catch_unwind`, preventing
  undefined behaviour from panics unwinding across the FFI boundary. Scalar errors
  are reported via `duckdb_scalar_function_set_error`; table scan panics set chunk
  size to 0 (end of stream).

- **`Value` extraction methods** — `as_i8()`, `as_i16()`, `as_u8()`, `as_u16()`,
  `as_u32()`, `as_u64()`, `as_i128()` covering every DuckDB integer type via
  `duckdb_get_int8/int16/uint8/uint16/uint32/uint64/hugeint`. Plus `as_str_or()`,
  `as_str_or_default()`, and `_or(default)` null-safe variants for all types.

- **`VectorReader`** — `read_date()`, `read_timestamp()`, `read_time()`,
  `read_blob()`, `read_uuid()` semantic methods for DATE, TIMESTAMP, TIME,
  BLOB, and UUID column types.

- **`VectorWriter`** — `write_date()`, `write_timestamp()`, `write_time()`,
  `write_blob()`, `write_uuid()` semantic methods matching reader additions.

- **`DataChunk` convenience methods** — `struct_writer(col, fields)`,
  `struct_reader(col, fields)`, `struct_field_reader(col, field)`,
  `into_chunk_writer()` bridging to the new `StructWriter`, `StructReader`,
  and `ChunkWriter` types.

- **`ChunkWriter::struct_writer(col, fields)`** — convenience bridge to
  `StructWriter` from within a `ChunkWriter`.

- **`MockVectorWriter`** — `write_blob()`, `write_date()`, `write_timestamp()`,
  `write_time()`, `write_uuid()` matching real `VectorWriter` additions.

- **`MockVectorWriter` / `MockVectorReader`** — `try_get_i8()`, `try_get_i16()`,
  `try_get_u8()`, `try_get_u16()`, `try_get_u32()`, `try_get_u64()`,
  `try_get_f32()`, `try_get_i128()`, `try_get_blob()`, `try_get_uuid()` closing
  the type coverage asymmetry between mock and real vector types.

- **`MockVectorReader` constructors** — `from_i8s()`, `from_i16s()`, `from_u8s()`,
  `from_u16s()`, `from_u32s()`, `from_u64s()`, `from_f32s()`, `from_i128s()`,
  `from_intervals()`, `from_blobs()` for every `MockDuckValue` variant.

- **`MockDuckValue::Blob(Vec<u8>)`** — new variant for BLOB testing.

- **Prelude additions** — `StructReader`, `StructWriter`, `ChunkWriter` re-exported.

### Changed

- **`TableDescription::column_type()`** now returns `Option<LogicalType>` (RAII)
  instead of raw `duckdb_logical_type`, eliminating manual destroy calls by callers.

- **Version references updated** — all documentation, examples, scaffold templates,
  and book pages now reference `quack-rs = "0.10"` (was `"0.9"`).

### Fixed

- **FFI callback panic safety** — replaced 13 `CString::new(...).expect(...)` calls
  in FFI callback contexts (table/info, scalar/info, cast/builder, aggregate/info,
  copy_function/info, replacement_scan) with non-panicking `str_to_cstring()` that
  truncates at interior null bytes. Fully honours the "no panics across FFI" design
  principle (Pitfall L3).

- **Non-idiomatic `&mut { expr }` syntax** — replaced 8 instances in builder
  `register()` methods and 1 in replacement scan with idiomatic `&raw mut`.

## [0.9.0] - 2026-03-29

### Added

- **`Value` RAII wrapper** (`value` module) — owned wrapper around `duckdb_value`
  with automatic cleanup via `Drop`. Typed extraction methods: `as_str()`,
  `as_i64()`, `as_i32()`, `as_f64()`, `as_f32()`, `as_bool()`. Eliminates
  manual `duckdb_destroy_value` calls and prevents memory leaks in bind
  parameter extraction.

- **`DataChunk` wrapper** (`data_chunk` module) — ergonomic non-owning wrapper
  around `duckdb_data_chunk` with `reader(col)`, `writer(col)`, `size()`,
  `set_size(n)`, `column_count()`, and `vector(col)` methods. Eliminates raw
  `duckdb_data_chunk_get_vector` / `duckdb_data_chunk_set_size` calls in scan
  callbacks.

- **`VectorWriter::write_str(idx, value)`** — alias for `write_varchar` for
  discoverability. Extension authors searching for `write_str` now find it
  immediately.

- **`BindInfo::get_parameter_value(index)`** — returns an owned `Value` instead
  of a raw `duckdb_value`, preventing memory leaks.

- **`BindInfo::get_named_parameter_value(name)`** — same for named parameters.

- **`MapVector::key_writer(vector)`** / **`value_writer(vector)`** — create
  `VectorWriter` instances for MAP key and value child vectors directly.

- **`MapVector::key_reader(vector, count)`** / **`value_reader(vector, count)`**
  — create `VectorReader` instances for MAP key and value child vectors.

- **`MockVectorWriter::write_str(idx, value)`** — alias for `write_varchar`
  matching the `VectorWriter` API addition.

- **Prelude additions** — `Value`, `DataChunk`, and `ValidityBitmap` are now
  re-exported from `quack_rs::prelude`.

### Changed

- **Version references updated** — all documentation, examples, scaffold
  templates, and book pages now reference `quack-rs = "0.9"` (was `"0.7"`).

## [0.8.0] - 2026-03-28

### Added

- **`LogicalType::from_raw(ptr)`** — construct a `LogicalType` from an existing
  raw `duckdb_logical_type` handle, taking ownership.

- **`LogicalType` complex type constructors** — `decimal(width, scale)`,
  `array(element, size)`, `array_from_logical(element, size)`,
  `union_type(members)`, `union_type_from_logical(members)`, `enum_type(members)`.

- **`LogicalType` `_from_logical` variants** — `struct_type_from_logical`,
  `list_from_logical`, `map_from_logical` accept `LogicalType` values for
  nested complex types that cannot be expressed as simple `TypeId`.

- **`LogicalType` introspection methods** (20 methods) — `get_type_id`,
  `get_alias`, `set_alias`, `decimal_width`, `decimal_scale`,
  `decimal_internal_type`, `enum_internal_type`, `enum_dictionary_size`,
  `enum_dictionary_value`, `list_child_type`, `map_key_type`, `map_value_type`,
  `struct_child_count`, `struct_child_name`, `struct_child_type`,
  `union_member_count`, `union_member_name`, `union_member_type`,
  `array_size`, `array_child_type`.

- **`TypeId::from_duckdb_type(raw)`** — reverse conversion from raw
  `DUCKDB_TYPE` C enum to `TypeId`.

- **`ScalarFunctionBuilder::extra_info(data, destroy)`** — attach arbitrary
  data to a scalar function, accessible via `duckdb_function_get_extra_info`
  in callbacks.

- **`ScalarOverloadBuilder::extra_info(data, destroy)`** — same for scalar
  function set overloads.

- **`AggregateFunctionBuilder::extra_info(data, destroy)`** — attach arbitrary
  data to an aggregate function.

- **`TableFunctionBuilder::param_logical(logical_type)`** — add a positional
  parameter with a complex `LogicalType`.

- **`TableFunctionBuilder::named_param_logical(name, logical_type)`** — add a
  named parameter with a complex `LogicalType`.

- **`CastFunctionBuilder::new_logical(source, target)`** — construct a cast
  builder using `LogicalType` values for complex source/target types.

- **`ScalarFunctionInfo`** — callback wrapper with `get_extra_info()`,
  `set_error()`, and (`duckdb-1-5`) `get_bind_data()`, `get_state()`.

- **`ScalarBindInfo`** (`duckdb-1-5`) — scalar bind callback wrapper with
  `argument_count()`, `get_argument()`, `get_extra_info()`, `set_bind_data()`,
  `set_error()`, `get_client_context()`.

- **`ScalarInitInfo`** (`duckdb-1-5`) — scalar init callback wrapper with
  `get_extra_info()`, `get_bind_data()`, `set_state()`, `set_error()`,
  `get_client_context()`.

- **`AggregateFunctionInfo`** — aggregate callback wrapper with
  `get_extra_info()` and `set_error()`.

- **`CopyBindInfo`** (`duckdb-1-5`) — copy bind callback wrapper with
  `column_count()`, `column_type()`, `get_extra_info()`, `set_bind_data()`,
  `set_error()`, `get_client_context()`.

- **`CopyGlobalInitInfo`** (`duckdb-1-5`) — copy global init callback wrapper
  with `get_bind_data()`, `get_extra_info()`, `get_file_path()`,
  `set_global_state()`, `set_error()`, `get_client_context()`.

- **`CopySinkInfo`** (`duckdb-1-5`) — copy sink callback wrapper with
  `get_bind_data()`, `get_extra_info()`, `get_global_state()`, `set_error()`,
  `get_client_context()`.

- **`CopyFinalizeInfo`** (`duckdb-1-5`) — copy finalize callback wrapper with
  `get_bind_data()`, `get_extra_info()`, `get_global_state()`, `set_error()`,
  `get_client_context()`.

- **`BindInfo::get_parameter(index)`** — retrieve positional parameter value
  in table function bind callbacks.

- **`BindInfo::get_named_parameter(name)`** — retrieve named parameter value
  in table function bind callbacks.

- **`BindInfo::get_extra_info()`**, **`InitInfo::get_extra_info()`**,
  **`FunctionInfo::get_extra_info()`** — access extra info from table function
  callbacks.

- **`get_client_context()`** — available on `BindInfo` (table), `ScalarBindInfo`,
  `ScalarInitInfo`, `CopyBindInfo`, `CopyGlobalInitInfo`, `CopySinkInfo`,
  `CopyFinalizeInfo`. Returns a `ClientContext` RAII wrapper.

- **`ArrayVector`** — helper for fixed-size array vectors with `get_child()`.

- **`vector_size()`** — returns the default DuckDB vector size (typically 2048).

- **`vector_get_column_type(vector)`** — returns the `LogicalType` of a vector.

- **Prelude additions** — `StructVector`, `ListVector`, `MapVector`,
  `ArrayVector`, `ScalarFunctionInfo`, `AggregateFunctionInfo` now re-exported
  from `quack_rs::prelude`.

### Changed

- **`CastFunctionBuilder::source()` / `target()`** now return `Option<TypeId>`
  instead of `TypeId`, returning `None` when the builder was created via
  `new_logical()`. **This is a breaking change.**

- **`CastRecord::source` / `target`** fields changed from `TypeId` to
  `Option<TypeId>` to match the builder change.

## [0.7.1] - 2026-03-27

### Added

- **`TypeId::Any`** — wildcard type for function overload resolution. Maps to
  `DUCKDB_TYPE_ANY` in the C API. Requires `duckdb-1-5` feature.

- **`TypeId::Varint`** — variable-length arbitrary-precision integer. Maps to
  `DUCKDB_TYPE_BIGNUM` in the C API, exposed as `VARINT` in SQL. Requires
  `duckdb-1-5` feature.

- **`TypeId::SqlNull`** — explicit SQL NULL type representing the type of a
  bare `NULL` literal before type resolution. Maps to `DUCKDB_TYPE_SQLNULL`
  in the C API. Requires `duckdb-1-5` feature.

- **`TypeId::IntegerLiteral`** — internal type for unresolved integer literals
  during overload resolution. Maps to `DUCKDB_TYPE_INTEGER_LITERAL`. Requires
  `duckdb-1-5` feature.

- **`TypeId::StringLiteral`** — internal type for unresolved string literals
  during overload resolution. Maps to `DUCKDB_TYPE_STRING_LITERAL`. Requires
  `duckdb-1-5` feature.

- **`MockVectorReader`/`MockVectorWriter` tests** — 12 new tests covering
  `from_i32s`, `from_f64s`, `from_bools` constructors, typed getters
  (`i32`, `f64`, `bool`), `u16`/`i128`/`interval` round-trips, wrong-type
  returns None, and `is_empty`.

- **DuckDB v1.5.1 compatibility evaluation** — comprehensive analysis of all
  80+ changes in DuckDB v1.5.1 against quack-rs. See
  `docs/duckdb-v1.5.1-evaluation.md`.

### Fixed

- **ARM64 / aarch64 build** — replaced all `.cast::<i8>()` and `*const i8`
  pointer casts with `std::os::raw::c_char`, which resolves to `i8` on
  x86-64 and `u8` on ARM64 (where C `char` is unsigned). Eliminates
  `E0308`/`E0277` mismatched-types errors when cross-compiling or building
  natively on aarch64. Affected files: `replacement_scan/mod.rs`,
  `types/logical_type.rs`, `vector/writer.rs`.

### Changed

- **DuckDB v1.5.1 compatibility** — updated `DUCKDB_API_VERSION` doc comment
  and version range documentation to explicitly cover v1.5.1. The C API
  version remains `"v1.2.0"` (unchanged from v1.5.0). Users are strongly
  recommended to upgrade their DuckDB runtime to v1.5.1 for critical WAL
  corruption and ART index correctness fixes.

### Internal

- **CI action update** — `dtolnay/rust-toolchain` pinned to
  `631a55b12751854ce901bb631d5902ceb48146f7` (PR #59).

- **Mutation testing** — `mutants.toml` now sets `features = ["duckdb-1-5"]`
  so that `cargo mutants` compiles and tests feature-gated code paths.
  Previously, four mutants in `MockRegistrar::copy_function_names`,
  `has_copy_function`, and `total_registrations` were unreachable because
  their tests were also feature-gated. Added
  `mock_registrar_total_registrations_scalar_plus_copy_function` to
  robustly kill the `+ with -` mutation in `total_registrations` by using
  a non-zero `base` count.

## [0.7.0] - 2026-03-22

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

- **Transitive dependency updates** — `cc` 1.2.56→1.2.57, `tar` 0.4.44→0.4.45,
  `rustls-webpki` 0.103.9→0.103.10, `arrow` 56.2.0→57.3.0, `clap` 4.5.60→4.6.0,
  `tempfile` 3.14.0→3.27.0, plus ~30 other minor/patch updates.

- **CI action updates** — `Swatinem/rust-cache` v2.8.2→v2.9.1,
  `actions/download-artifact` v8.0.0→v8.0.1, `actions/cache` 5.0.3→5.0.4,
  `codecov/codecov-action` 5.4.3→5.5.3.

### Fixed

- **COPY format handlers** — previously listed as a known limitation (no C API
  counterpart). DuckDB 1.5.0 adds `duckdb_create_copy_function` and related
  symbols; the new `copy_function` module wraps them behind `duckdb-1-5`.

## [0.6.0] - 2026-03-12

### Added

- **`InMemoryDb` dispatch table initialisation** — `InMemoryDb::open()` now
  correctly initialises the `loadable-extension` dispatch table from bundled
  DuckDB symbols before opening a connection, allowing all three `InMemoryDb`
  unit tests to pass under `cargo test --features bundled-test`. Previously
  every call to `InMemoryDb::open()` panicked with
  `"DuckDB API not initialized or DuckDB feature omitted"` because the
  `loadable-extension` dispatch table was never populated in `cargo test`.

- **`src/testing/bundled_api_init.cpp`** — thin C++ shim that wraps DuckDB's
  internal `CreateAPIv1()` function (from `duckdb/main/capi/extension_api.hpp`)
  as a C-linkage symbol (`quack_rs_create_api_v1`). Called once at test startup
  to populate all 459 `AtomicPtr` slots in the dispatch table with real bundled
  DuckDB function pointers.

- **`build.rs`** — Cargo build script that, when the `bundled-test` feature is
  active, locates the `libduckdb-sys` build output directory, finds the bundled
  DuckDB include path, and compiles `bundled_api_init.cpp` via the `cc` crate.

- **CI: `test-bundled` job** — new CI job runs
  `cargo test --all-targets --features bundled-test` on all three platforms
  (Linux, macOS, Windows) on every push and pull request, closing the gap that
  allowed this failure to reach the release workflow undetected.

- **Pitfall P9 documented** — `LESSONS.md` now contains a full analysis of the
  `loadable-extension` dispatch table failure mode: root cause, the
  `CreateAPIv1()` solution, ABI compatibility details, risks of relying on
  DuckDB's internal C++ header, and a mitigation table.

### Fixed

- `InMemoryDb::open()` no longer panics when called in `cargo test` with the
  `bundled-test` feature enabled. This was a regression introduced when
  `InMemoryDb` was first shipped in 0.5.1 without the dispatch table
  initialisation step.

### Changed

- `bundled-test` feature documentation updated to accurately describe the
  dispatch table initialisation behaviour (previously claimed to "bypass" the
  dispatch mechanism; it now correctly initialises it).

## [0.5.1] - 2026-03-12

### Added

- **Testing primitives (`quack_rs::testing`)** — new mock types for unit-testing
  extension logic without a live DuckDB process:
  - `MockVectorWriter` — in-memory output buffer matching the `VectorWriter` API;
    use to test scalar/aggregate finalize/scan callbacks
  - `MockVectorReader` — in-memory input buffer with convenience constructors
    (`from_i64s`, `from_strs`, `from_bools`, `from_f64s`, `from_i32s`)
  - `MockDuckValue` — typed enum covering all DuckDB scalar types
  - `MockRegistrar` — implements the `Registrar` trait using interior mutability;
    records registered functions without any C API call
  - `CastRecord` — records source/target types for cast registrations

- **`bundled-test` Cargo feature** — links the bundled DuckDB static library via
  the `duckdb` crate and enables `InMemoryDb::open()` for SQL-level assertions in
  `cargo test`. Does not initialize the `loadable-extension` dispatch table.

- **`InMemoryDb`** — wraps `duckdb::Connection` for SQL-level integration tests;
  available behind the `bundled-test` feature.

- **Builder introspection accessors** — `pub fn name(&self) -> &str` added to
  `ScalarFunctionBuilder`, `ScalarFunctionSetBuilder`, `AggregateFunctionBuilder`,
  `AggregateFunctionSetBuilder`, and `TableFunctionBuilder`. `pub fn source(&self)
  -> Option<TypeId>` and `pub fn target(&self) -> Option<TypeId>` added to `CastFunctionBuilder`.

### Security

- Bump `quinn-proto` 0.11.13 → 0.11.14 in root and `examples/hello-ext`
  `Cargo.lock` files (addresses RUSTSEC advisory).

## [0.5.0] - 2026-03-10

### Added

- **`param_logical(LogicalType)` on all builders** — register parameters with
  complex parameterized types (`LIST(BIGINT)`, `MAP(VARCHAR, INTEGER)`,
  `STRUCT(...)`) that `TypeId` alone cannot express. Available on
  `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder::OverloadBuilder`,
  `ScalarFunctionBuilder`, and `ScalarOverloadBuilder`. Parameters added via
  `param()` and `param_logical()` are interleaved by position, so the order
  you call them is the order DuckDB sees them.

- **`returns_logical(LogicalType)` on all builders** — set a complex
  parameterized return type. When both `returns(TypeId)` and
  `returns_logical(LogicalType)` are called, the logical type takes precedence.
  Available on `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder`,
  `ScalarFunctionBuilder`, and `ScalarOverloadBuilder`. This eliminates the
  need for raw FFI when returning `LIST(BOOLEAN)`, `LIST(TIMESTAMP)`,
  `MAP(K, V)`, or any other parameterized type.

- **`null_handling(NullHandling)` on set overload builders** — per-overload
  NULL handling configuration for `AggregateFunctionSetBuilder::OverloadBuilder`
  and `ScalarOverloadBuilder`. Previously only available on single-function
  builders.

### Notes

- **Upstream fix: `duckdb-loadable-macros` panic-at-FFI-boundary** — the safe
  entry-point pattern developed in `quack-rs` (using `?` / `ok_or_else` throughout
  instead of `.unwrap()`) was contributed upstream as
  [duckdb/duckdb-rs#696](https://github.com/duckdb/duckdb-rs/pull/696) and merged
  2026-03-09. All users of the `duckdb_entrypoint_c_api!` macro from
  `duckdb-loadable-macros` will receive this fix in the next `duckdb-rs` release.
  `quack-rs` users have always been protected via the safe `entry_point!` /
  `entry_point_v2!` macros provided by this crate.

## [0.4.0] - 2026-03-09

### Added

- **`Connection` and `Registrar` trait** — version-agnostic extension registration
  facade (`src/connection.rs`). `Connection` wraps the `duckdb_connection` and
  `duckdb_database` handles provided at initialization time. The `Registrar` trait
  provides uniform methods for registering all extension components (scalar, scalar
  set, aggregate, aggregate set, table, SQL macro, cast), making registration code
  interchangeable across DuckDB 1.4.x and 1.5.x. Replacement scans are exposed as
  direct methods on `Connection` since they require `duckdb_database`, not the
  connection handle.

- **`init_extension_v2`** — new entry point helper that passes `&Connection` to the
  registration callback instead of a raw `duckdb_connection`. Prefer this over
  `init_extension` for new extensions.

- **`entry_point_v2!` macro** — companion macro to `entry_point!` that generates
  the `#[no_mangle] unsafe extern "C"` entry point using `init_extension_v2`.

- **`duckdb-1-5` cargo feature** — placeholder feature flag for DuckDB 1.5.0-specific
  C API wrappers. Currently empty; will be populated when `libduckdb-sys` 1.5.0 is
  published on crates.io.

### Changed

- **DuckDB version support broadened to 1.4.x and 1.5.x** — the `libduckdb-sys`
  dependency requirement was relaxed from an exact pin (`=1.4.4`) to a range
  (`>=1.4.4, <2`). DuckDB v1.5.0 (released 2026-03-09) does not change the C API
  version string (`v1.2.0`) used in `duckdb_rs_extension_api_init`; the existing
  `DUCKDB_API_VERSION` constant remains correct for both releases. Extension authors
  can now pin their own `libduckdb-sys` to either `=1.4.4` or `=1.5.0` and resolve
  cleanly against `quack-rs`. The scaffold template and CI workflow template were
  updated to default to DuckDB v1.5.0.

## [0.3.0] - 2026-03-08

### Added

- **`TableFunctionBuilder`** — type-safe builder for registering DuckDB table functions
  (the `SELECT * FROM my_function(args)` pattern). Covers the full bind/init/scan
  lifecycle with ergonomic callbacks, eliminating ~100 lines of raw FFI boilerplate.
  Helper types `BindInfo`, `FfiBindData<T>`, and `FfiInitData<T>` manage parameter
  extraction and per-scan state with zero raw pointer manipulation. See
  [`table`](src/table/mod.rs) and `examples/hello-ext` (`generate_series_ext`) for
  a fully-tested end-to-end example verified against DuckDB 1.4.4.

- **`ReplacementScanBuilder`** — builder for registering DuckDB replacement scans
  (the `SELECT * FROM 'file.xyz'` pattern where a file path triggers a table-valued
  scan). The builder handles callback registration, path extraction, and bind-info
  population through a 4-method chain. See [`replacement_scan`](src/replacement_scan/).

- **`StructVector`** — safe wrapper for reading and writing STRUCT child vectors.
  `get_child(vec, idx)`, `field_reader(vec, idx, row_count)`, and
  `field_writer(vec, idx)` replace manual offset arithmetic over child vector handles.

- **`ListVector`** — safe wrapper for reading and writing LIST child vectors.
  `get_child`, `get_entry`, `set_entry`, `reserve`, `set_size`, `child_reader`, and
  `child_writer` cover the complete LIST read/write workflow without raw pointer casts.

- **`MapVector`** — safe wrapper for DuckDB MAP vectors (stored as
  `LIST<STRUCT{key, value}>`). `keys(vec)`, `values(vec)`, `struct_child(vec)`,
  `reserve`, `set_size`, `set_entry`, and `get_entry` expose the full MAP interface.

- **`vector::complex` module** — re-exports `StructVector`, `ListVector`, `MapVector`
  at `quack_rs::vector::complex` and documents the read-vs-write workflow for nested
  types with working code examples in the module doc.

- **`prelude` additions** — `TableFunctionBuilder`, `BindInfo`, `FfiBindData`,
  `FfiInitData`, `ReplacementScanBuilder`, `StructVector`, `ListVector`, `MapVector`,
  `CastFunctionBuilder`, `CastFunctionInfo`, `CastMode`
  are now all re-exported from `quack_rs::prelude`.

- **`CastFunctionBuilder`** — type-safe builder for registering custom type cast
  functions via `duckdb_cast_function_*`. Covers both explicit `CAST(x AS T)` and
  implicit coercions (with optional `implicit_cost`). The companion `CastFunctionInfo`
  wrapper exposes `cast_mode()`, `set_error()`, and `set_row_error()` inside callbacks,
  giving correct `TRY_CAST` / `CAST` error handling with zero raw pointer boilerplate.
  See [`cast`](src/cast/) for the full API.

- **`DbConfig`** — RAII wrapper for `duckdb_config` (extension configuration
  parameters). Builder-style `.set(name, value)?` chain, automatic `duckdb_destroy_config`
  on drop, and `flag_count()` / `get_flag(index)` for enumerating all available options.
  Useful when an extension needs to open a secondary `DuckDB` database from within its
  callbacks. See [`config`](src/config.rs).

- **`ScalarFunctionSetBuilder`** — builder for registering scalar function sets
  (multiple overloads under one name), mirroring `AggregateFunctionSetBuilder`.

- **`TypeId` variants** — `Decimal`, `Struct`, `Map`, `UHugeInt`, `TimeTz`,
  `TimestampS`, `TimestampMs`, `TimestampNs`, `Array`, `Enum`, `Union`, `Bit`.

- **`From<TypeId> for LogicalType`** — idiomatic conversion from `TypeId`.

- **`#[must_use]` on builder structs** — `ScalarFunctionBuilder`,
  `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder`, and `OverloadBuilder`
  now warn at compile time if constructed but never consumed.

- **`NullHandling` enum and `.null_handling()` builder method** — configurable
  NULL propagation for scalar and aggregate functions via
  `duckdb_scalar_function_set_special_handling` /
  `duckdb_aggregate_function_set_special_handling`.

- **`VectorWriter::write_interval`** — writes INTERVAL values to output vectors using
  the correct 16-byte `{ months: i32, days: i32, micros: i64 }` layout.

- **`append_metadata` binary** — native Rust replacement for the Python
  `append_extension_metadata.py` script, now shipping with the crate.
  Install with `cargo install quack-rs --bin append_metadata`.

- **`hello-ext` cast function demo** — `examples/hello-ext` now registers a
  `CAST(VARCHAR AS INTEGER)` cast function using `CastFunctionBuilder`,
  demonstrating both `CAST` (abort-on-error) and `TRY_CAST` (NULL-on-error)
  code paths. Five unit tests cover `parse_varchar_to_int`, including
  boundary values and overflow.

### Not implemented (upstream C API gap)

- **Window functions** — `duckdb_create_window_function` and related symbols do
  not exist in DuckDB's public C extension API.  They are implemented only in the
  C++ layer and are therefore not wrappable by `quack-rs` or any other C-API
  binding.  Verified against the
  [DuckDB stable C API reference](https://duckdb.org/docs/stable/clients/c/api)
  and `libduckdb-sys` 1.4.4 bindings.

- **COPY format handlers** — `duckdb_create_copy_function` and related symbols are
  similarly absent from the C extension API for the same reason.

### Fixed

- **`hello-ext` `gs_bind` callback** — replaced incorrect `duckdb_value_int64(param)`
  (wrong arity: takes 3 arguments) with `duckdb_get_int64(param)` (correct 1-argument
  form). The extension now builds cleanly and all 11 live SQL tests pass against
  DuckDB 1.4.4.

### Changed

- Bump `criterion` dev-dependency from `0.5` to `0.8`.
- Bump `Swatinem/rust-cache` GitHub Action from `v2.7.5` to `v2.8.2`.
- Bump `dtolnay/rust-toolchain` CI pin from `v2.7.5` to latest SHA.
- Bump `actions/attest-build-provenance` from `v2` to `v4`.
- Bump `actions/configure-pages` to latest SHA (`d5606572…`).
- Bump `actions/upload-pages-artifact` from `v3.0.1` to `v4.0.0`.

---

## [0.2.0] - 2026-03-07

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

## [0.1.0] - 2025-05-01

### Added

- Initial release
- `entry_point` module: `init_extension` helper for correct extension initialization
- `aggregate` module: `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder`
- `aggregate::state` module: `AggregateState` trait, `FfiState<T>` wrapper
- `aggregate::callbacks` module: type aliases for all 6 callback signatures
- `vector` module: `VectorReader`, `VectorWriter`, `ValidityBitmap`, `DuckStringView`
- `types` module: `TypeId` enum, `LogicalType` RAII wrapper
- `interval` module: `DuckInterval`, `interval_to_micros`, `read_interval_at`
- `error` module: `ExtensionError`, `ExtResult<T>`
- `testing` module: `AggregateTestHarness<S>` for pure-Rust aggregate testing
- `validate` module: `validate_extension_name`, `validate_function_name`,
  `validate_semver`, `validate_extension_version`, `validate_spdx_license`,
  `validate_platform`, `validate_release_profile`
- `scaffold` module: `generate_scaffold` for generating complete extension projects
- `sql_macro` module: `SqlMacro` for registering SQL macros without FFI callbacks
- Complete `hello-ext` example extension
- Documentation of all 15 DuckDB Rust FFI pitfalls (`LESSONS.md`)
- CI pipeline: check, test, clippy, fmt, doc, MSRV, bench-compile
- `SECURITY.md` vulnerability disclosure policy

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.16.0...HEAD
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
