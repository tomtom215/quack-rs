# Changelog

All notable changes to quack-rs, mirrored from
[`CHANGELOG.md`](https://github.com/tomtom215/quack-rs/blob/main/CHANGELOG.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
quack-rs adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

- **`CastFunctionBuilder`** — type-safe builder for registering custom type cast
  functions. Covers explicit `CAST(x AS T)` and implicit coercions (optional
  `implicit_cost`). `CastFunctionInfo` exposes `cast_mode()`, `set_error()`, and
  `set_row_error()` inside callbacks for correct `TRY_CAST` / `CAST` error handling.
  See [Cast Functions](../functions/cast-functions.md).

- **`DbConfig`** — RAII wrapper for `duckdb_config`. Builder-style `.set(name, value)?`
  chain with automatic `duckdb_destroy_config` on drop and `flag_count()` /
  `get_flag(index)` for enumerating all available options.
  See [`quack_rs::config`](https://docs.rs/quack-rs/latest/quack_rs/config/index.html).

- **`hello-ext` cast demo** — the example extension now registers
  `CAST(VARCHAR AS INTEGER)` and `TRY_CAST(VARCHAR AS INTEGER)` using
  `CastFunctionBuilder`, demonstrating both error modes with five unit tests.

- **`ScalarFunctionSetBuilder`**, additional `TypeId` variants (`Decimal`, `Struct`,
  `Map`, `UHugeInt`, `TimeTz`, `TimestampS`, `TimestampMs`, `TimestampNs`, `Array`,
  `Enum`, `Union`, `Bit`), `NullHandling` enum, `From<TypeId> for LogicalType`,
  `#[must_use]` on builder structs, `VectorWriter::write_interval`, and
  `append_metadata` native binary (replaces Python metadata script).

### Not implemented (upstream C API gap)

- **Window functions** and **COPY format handlers** are absent from DuckDB's public
  C extension API and cannot be wrapped. See [Known Limitations](known-limitations.md).

### Changed

- Bump `criterion` dev-dependency from `0.5` to `0.8`.
- Bump `Swatinem/rust-cache` GitHub Action from `v2.7.5` to `v2.8.2`.
- Bump `dtolnay/rust-toolchain` CI pin from `1.84.1` to `1.100.0`.
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
- `types` module: `TypeId` enum (21 variants), `LogicalType` RAII wrapper
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

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/tomtom215/quack-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/quack-rs/releases/tag/v0.1.0
