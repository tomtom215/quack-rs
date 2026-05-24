# Changelog

All notable changes to quack-rs, mirrored from
[`CHANGELOG.md`](https://github.com/tomtom215/quack-rs/blob/main/CHANGELOG.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
quack-rs adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

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

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.12.1...HEAD
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
