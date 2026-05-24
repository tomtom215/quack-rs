# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `bundled-test` can now link against a pre-built libduckdb instead of
  compiling DuckDB from C++ source. Opt in with
  `default-features = false, features = ["bundled-test"]` plus either
  `DUCKDB_DOWNLOAD_LIB=1` (libduckdb-sys downloads the upstream release
  zip) or `DUCKDB_LIB_DIR=...` (use a libduckdb tree you already have).
  Default behaviour is unchanged.
- `InMemoryDb::open_unsigned()` opens an in-memory database with
  `allow_unsigned_extensions=true`, allowing downstream extension crates
  to `LOAD` their own locally-built (unsigned) `.duckdb_extension`
  artifact for integration testing.

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

[Unreleased]: https://github.com/tomtom215/quack-rs/compare/v0.13.0...HEAD
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
