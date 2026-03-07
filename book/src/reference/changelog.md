# Changelog

All notable changes to quack-rs, mirrored from
[`CHANGELOG.md`](https://github.com/tomtom215/quack-rs/blob/main/CHANGELOG.md).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
quack-rs adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

- **`sql_macro` module** — Register SQL macros (scalar and table) directly from
  Rust via `CREATE OR REPLACE MACRO`. No C++ wrapper or custom FFI callbacks
  needed. Closes the FAQ gap: "Can I expose SQL macros as an extension?"

- **`SqlMacro::scalar`** — Create a scalar macro with named parameters and a SQL
  expression body.

- **`SqlMacro::table`** — Create a table macro with named parameters and a SQL
  `SELECT` query body.

- **`SqlMacro::to_sql`** — Generate the `CREATE OR REPLACE MACRO` statement as a
  `String` for testing or inspection without a DuckDB connection.

- **`SqlMacro::register`** — Execute the generated SQL against a live
  `duckdb_connection` at extension initialization time.

- **Scaffold: `extension_config.cmake` generation** — The scaffold generator now
  produces `extension_config.cmake`, required by `extension-ci-tools` for CI.

- **Scaffold: SQLLogicTest skeleton** — `generate_scaffold` produces
  `test/sql/{name}.test` with `require` directive and example blocks.

- **Scaffold: GitHub Actions CI workflow** — `generate_scaffold` produces
  `.github/workflows/extension-ci.yml` for cross-platform CI.

- **`validate::validate_excluded_platforms_str`** — Validates the
  `excluded_platforms` field as a semicolon-delimited string (e.g.,
  `"wasm_mvp;wasm_eh;wasm_threads"`).

- **`validate::validate_excluded_platforms`** — Re-exported at the `validate`
  module level.

- **CI: Windows testing** — The CI matrix now includes `windows-latest`.

- **CI: `example-check` job** — CI now checks, lints, and tests
  `examples/hello-ext` on every PR.

- **`validate` module** — Community extension compliance validators:
  - `validate_extension_name`
  - `validate_function_name`
  - `validate_semver`
  - `validate_extension_version`
  - `validate_spdx_license`
  - `validate_platform`
  - `validate_release_profile`
  - `semver::classify_extension_version`

- **`scalar` module** — `ScalarFunctionBuilder` for registering scalar functions.

- **`entry_point!` macro** — Generates the extension entry point with zero boilerplate.

- **`VectorWriter::write_varchar`** — Write VARCHAR string values to output vectors.

- **`VectorWriter::write_bool`** — Write BOOLEAN values.

- **`VectorWriter::write_u16`** — Write USMALLINT values.

- **`VectorWriter::write_i16`** — Write SMALLINT values.

- **`VectorReader::read_interval`** — Read INTERVAL values from input vectors.

- **`CHANGELOG.md`** and **`SECURITY.md`**.

### Fixed

- MSRV documentation consistently states 1.84.1 across README, CONTRIBUTING.md,
  and Cargo.toml (previously README said 1.80).

---

## [0.1.0] — 2025-05-01

### Added

- `entry_point` module: `init_extension` helper for correct extension initialization
- `aggregate` module: `AggregateFunctionBuilder`, `AggregateFunctionSetBuilder`
- `aggregate::state` module: `AggregateState` trait, `FfiState<T>` wrapper
- `aggregate::callbacks` module: type aliases for all 6 aggregate callback signatures
- `vector` module: `VectorReader`, `VectorWriter`, `ValidityBitmap`, `DuckStringView`
- `types` module: `TypeId` enum (21 variants), `LogicalType` RAII wrapper
- `interval` module: `DuckInterval`, `interval_to_micros`, `read_interval_at`
- `error` module: `ExtensionError`, `ExtResult<T>`
- `testing` module: `AggregateTestHarness<S>` for pure-Rust aggregate testing
- Complete `hello-ext` example extension (word count aggregate)
- Documentation of all 15 DuckDB Rust FFI pitfalls (`LESSONS.md`)
- CI pipeline: check, test, clippy, fmt, doc, MSRV, bench-compile

---

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tomtom215/quack-rs/releases/tag/v0.1.0
