# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/tomtom215/quack-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/tomtom215/quack-rs/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tomtom215/quack-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/quack-rs/releases/tag/v0.1.0
