# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `validate` module: community extension compliance validators
  - `validate_extension_name` — enforces `^[a-z][a-z0-9_-]*$` naming rules
  - `validate_semver` — validates semantic versioning (MAJOR.MINOR.PATCH)
  - `validate_spdx_license` — validates SPDX license identifiers
  - `validate_platform` — validates DuckDB build target identifiers
  - `validate_release_profile` — checks release profile settings for loadable extensions
- `scalar` module: `ScalarFunctionBuilder` for registering scalar functions
- `entry_point!` macro for generating the extension entry point with zero boilerplate
- `VectorWriter::write_varchar` for writing VARCHAR string values to output vectors
- `VectorWriter::write_bool` for writing BOOLEAN values
- `VectorWriter::write_u16` for writing USMALLINT values
- `VectorWriter::write_i16` for writing SMALLINT values
- `VectorReader::read_interval` for reading INTERVAL values from input vectors
- `CHANGELOG.md` for tracking releases
- `SECURITY.md` for vulnerability disclosure policy

### Fixed

- MSRV documentation now consistently states 1.84.1 across README, CONTRIBUTING.md,
  and Cargo.toml (previously README said 1.80)

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
- Complete `hello-ext` example extension
- Documentation of all 15 DuckDB Rust FFI pitfalls (LESSONS.md)
- CI pipeline: check, test, clippy, fmt, doc, MSRV, bench-compile

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tomtom215/quack-rs/releases/tag/v0.1.0
