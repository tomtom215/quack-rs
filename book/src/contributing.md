# Contributing

quack-rs is an open source project. Contributions of all kinds are welcome:
bug reports, documentation improvements, new pitfall discoveries, and code.

---

## Development prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | ≥ 1.86.0 (MSRV) | Compiler |
| `rustfmt` | stable | Formatting |
| `clippy` | stable | Linting |
| `cargo-msrv` | latest | MSRV verification |

Install the Rust toolchain via [rustup.rs](https://rustup.rs/).

---

## Building

```bash
# Build the library
cargo build

# Build in release mode (enables LTO + strip)
cargo build --release

# Build the hello-ext example extension
cargo build --release --manifest-path examples/hello-ext/Cargo.toml
```

---

## Quality gates

**All of the following must pass before merging any pull request:**

```bash
# Tests — zero failures, zero ignored
cargo test

# Integration tests
cargo test --test integration_test

# Linting — zero warnings (warnings are errors)
cargo clippy --all-targets -- -D warnings

# Formatting
cargo fmt -- --check

# Documentation — zero broken links or missing docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# MSRV — must compile on Rust 1.86.0 (excludes benches; matches CI)
cargo +1.86.0 check
```

These same checks run in CI on every push and pull request.

---

## Test strategy

### Unit tests

Unit tests live in `#[cfg(test)]` modules within each source file. They test
pure-Rust logic that does not require a live DuckDB instance.

**Important constraint**: `libduckdb-sys` with `features = ["loadable-extension"]`
makes all DuckDB C API functions go through lazy `AtomicPtr` dispatch. These
pointers are only populated when `duckdb_rs_extension_api_init` is called from
within a real DuckDB extension load. Calling any `duckdb_*` function in a unit
test will panic. Move such tests to integration tests or example-extension tests.

### Integration tests

`tests/integration_test.rs` contains pure-Rust tests that cross module
boundaries — testing `interval` with `AggregateTestHarness`, verifying `FfiState`
lifecycle, and so on. These still cannot call `duckdb_*` functions.

### Property-based tests

Selected modules include `proptest`-based tests:
- `interval.rs` — overflow edge cases across the full `i32`/`i64` range
- `testing/harness.rs` — sum associativity, identity element for `AggregateState`

### Example-extension tests

`examples/hello-ext/` contains `#[cfg(test)]` unit tests for the pure logic
(`count_words`). Full E2E testing (loading the `.so` into DuckDB) is left to
consumers.

---

## Code standards

### Safety documentation

Every `unsafe` block must have a `// SAFETY:` comment explaining:

1. Which invariant the caller guarantees
2. Why the operation is valid given that invariant

```rust
# struct Ffi { inner: *mut u64 }
# let ffi = Ffi { inner: Box::into_raw(Box::new(0_u64)) };
// SAFETY: `states` is a valid array of `count` pointers, each initialized
// by `init_callback`. We are the only owner of `inner` at this point.
unsafe { drop(Box::from_raw(ffi.inner)) };
```

### No panics across FFI

`unwrap()`, `expect()`, and `panic!()` are forbidden in any function that may
be called by DuckDB (callbacks and entry points). Use `Option`/`Result` and `?`
throughout.

### Clippy lint policy

The crate enables `pedantic`, `nursery`, and `cargo` lint groups. All warnings
are treated as errors in CI. Lints are suppressed only where they produce
false positives for SDK API patterns:

```toml
[lints.clippy]
module_name_repetitions = "allow"  # e.g., AggregateFunctionBuilder
must_use_candidate = "allow"       # builder methods
missing_errors_doc = "allow"       # unsafe extern "C" callbacks
return_self_not_must_use = "allow" # builder pattern
```

### Documentation

Every public item must have a doc comment. Follow these conventions:

- First line: short summary (noun phrase, no trailing period)
- `# Safety`: mandatory on every `unsafe fn`
- `# Panics`: mandatory if the function can panic
- `# Errors`: mandatory on functions returning `Result`
- `# Example`: encouraged on public types and key methods

---

## Repository structure

```text
quack-rs/
├── src/
│   ├── abi.rs                         # `DuckDB` C Extension API ABI compatibility checking
│   ├── appender.rs                    # Bulk data appending
│   ├── arrow.rs                       # Arrow C Data Interface bridge (`DuckDB` 1.5.0+, `duckdb-1-5-4` feature)
│   ├── callback.rs                    # Panic-safe callback wrapper macros for `DuckDB` extension callbacks
│   ├── catalog.rs                     # Catalog entry lookup (`DuckDB` 1.5.0+)
│   ├── chunk_writer.rs                # Auto-sizing chunk writer for table function scan callbacks
│   ├── client_context.rs              # Client context access (`DuckDB` 1.5.0+)
│   ├── config.rs                      # RAII wrapper for `DuckDB` database configuration
│   ├── config_option.rs               # Extension-defined configuration options (`DuckDB` 1.5.0+)
│   ├── connection.rs                  # [`Connection`] — version-agnostic extension registration facade
│   ├── data_chunk.rs                  # Ergonomic wrapper around `DuckDB` data chunks
│   ├── debug_repr.rs                  # Internal helpers for the crate's `Debug` implementations
│   ├── entry_point.rs                 # Extension entry point helper
│   ├── error.rs                       # Error types for `DuckDB` extension FFI error propagation
│   ├── error_data.rs                  # Structured error data (`DuckDB` 1.5.0+)
│   ├── expression.rs                  # Bound expressions (`DuckDB` 1.5.0+)
│   ├── extra_info.rs                  # Ownership of a function's `extra_info` allocation until `DuckDB` takes it
│   ├── file_system.rs                 # File system access (`DuckDB` 1.5.0+)
│   ├── instance_cache.rs              # Database instance cache (`DuckDB` 1.5.0+)
│   ├── interval.rs                    # `DuckDB` `INTERVAL` type conversion utilities
│   ├── lib.rs                         # A production-grade Rust SDK for building `DuckDB` loadable extensions
│   ├── prelude.rs                     # Convenience re-exports for the most commonly used `quack-rs` items
│   ├── query.rs                       # Running SQL from inside an extension
│   ├── secrets.rs                     # Credential handling for extensions
│   ├── selection_vector.rs            # Selection vectors (`DuckDB` 1.5.0+)
│   ├── sql_macro.rs                   # SQL macro registration for `DuckDB` extensions
│   ├── table_description.rs           # Table description metadata
│   ├── tls.rs                         # Type-erased TLS configuration provider for HTTP-capable extensions
│   ├── value.rs                       # RAII wrapper around `DuckDB` values (`duckdb_value`)
│   ├── warning.rs                     # Structured security warning API for extensions
│   ├── aggregate/
│   │   ├── callbacks.rs               # Type aliases for the five required `DuckDB` aggregate callback signatures
│   │   ├── info.rs                    # Ergonomic wrapper around `duckdb_function_info` for aggregate function callbacks
│   │   ├── mod.rs                     # Builders for registering `DuckDB` aggregate functions
│   │   ├── state.rs                   # Generic `FfiState<T>` wrapper for safe aggregate state management
│   │   └── builder/
│   │       ├── mod.rs                 # Builder types for registering `DuckDB` aggregate functions
│   │       ├── overload.rs            # One overload within an [`AggregateFunctionSetBuilder`]
│   │       ├── set.rs                 # Builder for registering a `DuckDB` aggregate function set (multiple overloads)
│   │       ├── single.rs              # Builder for registering a single-signature `DuckDB` aggregate function
│   │       └── tests.rs               # Unit tests
│   ├── appender/
│   │   ├── chunk.rs                   # Chunk-at-a-time appends: handing the [`Appender`] a whole [`DataChunk`]
│   │   ├── construct.rs               # Creating an [`Appender`] and choosing the columns it appends to
│   │   ├── lifecycle.rs               # Flushing, closing and (with `duckdb-1-5`) clearing an [`Appender`]
│   │   ├── rows.rs                    # Row-at-a-time appends: `row`, `end_row` and the non-numeric `append_*` methods
│   │   └── scalars.rs                 # The fixed-width numeric `append_*` methods, `append_bool` through `append_u128`
│   ├── arrow/
│   │   ├── array.rs                   # `ArrowArray` — an owned Arrow C Data Interface array
│   │   ├── convert.rs                 # The four conversions between `DuckDB` data chunks and the Arrow C Data Interface
│   │   ├── converted.rs               # `ArrowConvertedSchema` — an Arrow schema translated into `DuckDB`'s own type descriptors
│   │   ├── options.rs                 # `ArrowOptions` — the Arrow production settings of a connection or a result
│   │   ├── schema.rs                  # `ArrowSchema` — an owned Arrow C Data Interface schema
│   │   └── tests.rs                   # Unit tests
│   ├── bin/
│   │   └── append_metadata/
│   │       ├── cli.rs                 # Command-line parsing and validation (std-only, no clap)
│   │       ├── footer.rs              # The 512-byte `DuckDB` extension footer, and the optional 22-byte WebAssembly custom-section header that precedes it
│   │       ├── main.rs                # Append a DuckDB extension metadata block to a compiled .so / .dylib / .dll file,
│   │       └── tests.rs               # Unit tests
│   ├── callback/
│   │   └── payload.rs                 # Disposing of a caught panic payload without re-entering the unwinder
│   ├── cast/
│   │   ├── builder.rs                 # Builder for registering custom `DuckDB` cast functions
│   │   └── mod.rs                     # Builder for registering `DuckDB` custom cast functions
│   ├── copy_function/
│   │   ├── info.rs                    # Callback info wrappers for copy function callbacks
│   │   └── mod.rs                     # Copy function registration (`DuckDB` 1.5.0+)
│   ├── datetime/
│   │   ├── checks.rs                  # Pure-Rust mirrors of the checks `DuckDB` makes before it throws
│   │   ├── mod.rs                     # Calendar conversions for `DuckDB`'s temporal types
│   │   └── tests.rs                   # Unit tests
│   ├── query/
│   │   ├── bind.rs                    # Binding a [`PreparedStatement`]'s parameters: the typed `bind_*` methods and `bind_value`
│   │   ├── chunk.rs                   # [`OwnedDataChunk`]: a `duckdb_data_chunk` destroyed on drop
│   │   ├── connection.rs              # [`OwnedConnection`] and its cross-thread [`InterruptHandle`]
│   │   ├── cstr.rs                    # The two C-string conversions the `query` module runs everything through
│   │   ├── live_tests.rs              # Tests that need a live `DuckDB`
│   │   ├── prepared.rs                # Inspecting and executing a [`PreparedStatement`]; `bind.rs` binds its parameters
│   │   └── result.rs                  # Reading a [`QueryResult`]: its columns, its chunks and what kind of outcome it is
│   ├── replacement_scan/
│   │   └── mod.rs                     # Builder for registering `DuckDB` replacement scans
│   ├── scaffold/
│   │   ├── escape.rs                  # Quoting configured free text for YAML and Rust doc comments
│   │   ├── mod.rs                     # Project scaffolding for `DuckDB` Rust extensions
│   │   ├── templates.rs               # Template generators for scaffold file content
│   │   ├── tests.rs                   # Unit tests
│   │   ├── tests_escaping.rs          # Free text reaches the generated files intact
│   │   └── tests_generated.rs         # Unit tests
│   ├── scalar/
│   │   ├── info.rs                    # Ergonomic wrapper around `duckdb_function_info` for scalar function callbacks
│   │   ├── mod.rs                     # Builder for registering `DuckDB` scalar functions
│   │   ├── state.rs                   # Typed bind data and per-thread local state for scalar functions (`DuckDB` 1.5.0+)
│   │   ├── typed.rs                   # Scalar functions written as ordinary Rust closures
│   │   ├── typed_builder.rs           # The builder the closure-based scalar constructors return, and the one `extern "C"` trampoline they all share
│   │   └── builder/
│   │       ├── collision.rs           # Refusing a scalar signature that would silently replace an existing one
│   │       ├── mod.rs                 # Builder for registering `DuckDB` scalar functions
│   │       ├── overload.rs            # One overload within a [`ScalarFunctionSetBuilder`]
│   │       ├── set.rs                 # Builder for registering a `DuckDB` scalar function set (multiple overloads)
│   │       ├── signature.rs           # Detecting overloads that declare the same argument types
│   │       ├── single.rs              # Builder for registering a single-signature `DuckDB` scalar function
│   │       └── tests.rs               # Unit tests
│   ├── table/
│   │   ├── bind_data.rs               # Type-safe bind data management for table functions
│   │   ├── builder.rs                 # Builder for registering `DuckDB` table functions
│   │   ├── cstr.rs                    # Panic-free `&str` → `CString` conversion for the callback info wrappers
│   │   ├── info.rs                    # Ergonomic wrappers around `DuckDB` callback info handles
│   │   ├── init_data.rs               # Type-safe init data management for table functions
│   │   ├── mod.rs                     # Builder for registering `DuckDB` table functions
│   │   ├── type_check.rs              # Detects logical types that `DuckDB` refuses without saying so
│   │   ├── typed.rs                   # Closure-based table functions with typed scan state
│   │   └── typed/
│   │       └── trampolines.rs         # `extern "C"` trampolines behind [`TypedTableFunctionBuilder`][super::TypedTableFunctionBuilder]
│   ├── testing/
│   │   ├── bundled_api_init.cpp       # Compiled only when the `bundled-test` Cargo feature is active
│   │   ├── harness.rs                 # [`AggregateTestHarness`] — test aggregate logic without `DuckDB`
│   │   ├── in_memory_db.rs            # In-memory `DuckDB` helper for integration tests
│   │   ├── mock_registrar.rs          # [`MockRegistrar`] — a [`Registrar`] implementation for testing
│   │   ├── mock_vector.rs             # In-memory mock types for `DuckDB` vectors
│   │   ├── mod.rs                     # Test utilities for `DuckDB` extension development
│   │   └── mock_vector/
│   │       ├── reader.rs              # `MockVectorReader` — an in-memory mock input vector
│   │       ├── tests.rs               # Unit tests
│   │       └── writer.rs              # `MockVectorWriter` — an in-memory mock output vector
│   ├── types/
│   │   ├── logical_type.rs            # RAII wrapper for `duckdb_logical_type`
│   │   ├── mod.rs                     # `DuckDB` type system wrappers
│   │   ├── null_handling.rs           # NULL propagation behaviour for `DuckDB` functions
│   │   ├── type_id.rs                 # Ergonomic enum of all `DuckDB` column types
│   │   └── logical_type/
│   │       └── construct.rs           # Every `LogicalType` constructor, as `try_*` plus a panicking wrapper
│   ├── validate/
│   │   ├── extension_name.rs          # Extension name validation per `DuckDB` community extension rules
│   │   ├── function_name.rs           # SQL function name validation for `DuckDB` extensions
│   │   ├── mod.rs                     # Validation utilities for `DuckDB` community extension compliance
│   │   ├── platform.rs                # `DuckDB` build platform validation
│   │   ├── release_profile.rs         # Release profile validation for `DuckDB` loadable extensions
│   │   ├── semver.rs                  # Semantic versioning validation for `DuckDB` community extensions
│   │   ├── spdx.rs                    # SPDX license identifier validation for `DuckDB` community extensions
│   │   ├── spdx_exceptions.rs         # The SPDX license-exception identifiers accepted after `WITH`
│   │   └── description_yml/
│   │       ├── mod.rs                 # Validation of `DuckDB` community extension `description.yml` files
│   │       ├── model.rs               # A validated representation of a `DuckDB` community extension `description.yml`
│   │       ├── parser.rs              # Parses and validates a `description.yml` string
│   │       ├── tests.rs               # Unit tests
│   │       ├── tests_corpus.rs        # Unit tests
│   │       ├── tests_yaml.rs          # Unit tests
│   │       ├── validator.rs           # Validates a `description.yml` string and returns `Ok(())` if it passes all checks
│   │       ├── yaml.rs                # A reader for the subset of YAML that `description.yml` files use
│   │       └── yaml/
│   │           └── scalar.rs          # Scalar-level pieces of the `description.yml` YAML reader: decoding plain, quoted, block and flow values, and recognising keys and comments
│   ├── value/
│   │   ├── blob.rs                    # `Value::as_blob` — `BLOB` extraction
│   │   ├── checks.rs                  # Pure-Rust preconditions checked before a `Value` call reaches `DuckDB`
│   │   ├── composite.rs               # Composite constructors: `STRUCT`, `LIST`, `ARRAY`, `ENUM`, `MAP`, `UNION`
│   │   ├── defaults.rs                # The defaulting accessors — `Value::as_*_or`
│   │   ├── getters.rs                 # The typed scalar accessors — `Value::as_i64`, `as_timestamp`, `as_decimal`, …
│   │   ├── hugeint.rs                 # Conversions between Rust's 128-bit integers and `DuckDB`'s split-word `HUGEINT` / `UHUGEINT` records
│   │   ├── nested.rs                  # Reading nested values: `LIST` elements, `STRUCT` fields, `MAP` entries
│   │   ├── scalars.rs                 # The non-temporal scalar constructors
│   │   ├── temporal.rs                # Temporal constructors, validated against `DuckDB`'s ranges
│   │   └── temporal_checks.rs         # Pure-Rust range checks for the temporal types, derived from `DuckDB`'s source
│   └── vector/
│       ├── complex.rs                 # Complex type vector operations: STRUCT fields, LIST elements, MAP entries
│       ├── list_builder.rs            # Safe construction of `LIST` and `MAP` output vectors
│       ├── mod.rs                     # Safe helpers for reading from and writing to `DuckDB` data vectors
│       ├── nested_null.rs             # The child validity masks a NULL in a nested vector must also clear
│       ├── ops.rs                     # Whole-vector operations (`DuckDB` 1.5.0+)
│       ├── reader.rs                  # Safe typed reading from `DuckDB` data vectors
│       ├── string.rs                  # `DuckDB` `VARCHAR` and `BLOB` (`duckdb_string_t`) reading utilities
│       ├── struct_reader.rs           # Batched, typed reader for STRUCT input vectors
│       ├── struct_writer.rs           # Batched, typed writer for STRUCT output vectors
│       ├── uuid.rs                    # Converting between a `UUID`'s textual bits and `DuckDB`'s vector storage
│       ├── validity.rs                # Validity bitmap helpers for `DuckDB` NULL tracking
│       └── writer.rs                  # Safe typed writing to `DuckDB` result vectors
├── tests/
│   ├── ffi_roundtrip.rs               # End-to-end FFI round-trips against a real `DuckDB`
│   ├── integration_test.rs            # Integration tests for `quack-rs`
│   ├── secret_zeroize.rs              # `SecretEntry` never frees a buffer that still holds a secret
│   └── ffi_roundtrip/
│       ├── appender_rows.rs           # What happens to buffered rows when an append fails mid-row
│       ├── arrow_import.rs            # `arrow::data_chunk_from_arrow` checks against a live `DuckDB`
│       ├── lifecycle.rs               # Aggregate NULL rows, name collisions, overload builders, bind-data sharing
│       ├── query_docs.rs              # Pins the documented behaviour of `query`, `PreparedStatement`, `DbConfig`
│       ├── query_stream.rs            # A streaming result that stops early must not look like a finished one
│       ├── scalar_agg.rs              # Scalar and aggregate builder regressions
│       ├── table_cast.rs              # Table function, cast, replacement scan, SQL macro and COPY regressions
│       ├── tooling.rs                 # Checks of quack-rs's tooling tables against the linked `DuckDB`
│       ├── value_nested.rs            # Nested `Value` construction and inspection against a live `DuckDB`
│       ├── value_query.rs             # `Value` getters, DECIMAL binding, `Expression::fold`
│       ├── value_temporal.rs          # Every `Value` getter against every temporal source type, at every edge
│       └── vector_dt.rs               # NULLs in nested output vectors; selection vectors
├── benches/
│   └── interval_bench.rs          # Criterion benchmarks
├── examples/
│   └── hello-ext/                 # Reference example: word_count (aggregate) + first_word (scalar)
├── book/                          # mdBook documentation source
│   ├── src/                       # Markdown pages (this site)
│   └── theme/custom.css
├── .github/workflows/ci.yml       # CI pipeline
├── .github/workflows/docs.yml     # GitHub Pages deployment
├── CONTRIBUTING.md
├── LESSONS.md                     # The DuckDB Rust FFI pitfalls (L1–L11, P1–P12)
├── CHANGELOG.md
└── README.md
```

---

## Releasing

quack-rs uses `libduckdb-sys = ">=1.4.4, <2"` — a bounded range covering DuckDB 1.4.x
and 1.5.x, whose C API (`v1.2.0`) is stable across both releases. The `<2` upper bound
prevents silent adoption of a future major release that may change the C API.
Before broadening the range to a new major band:

1. Read the DuckDB changelog for C API changes
2. Check the new C API version string (used in `duckdb_rs_extension_api_init`)
3. Update `DUCKDB_API_VERSION` in `src/lib.rs` if the C API version changed
4. Audit all callback signatures against the new `bindgen.rs` output
5. Update the range bounds in `Cargo.toml` (runtime and dev-deps)

Versions follow [Semantic Versioning](https://semver.org/). Breaking changes
to the public API require a major version bump.

---

## Reporting issues

Use [GitHub Issues](https://github.com/tomtom215/quack-rs/issues). For security
vulnerabilities, see [`SECURITY.md`](https://github.com/tomtom215/quack-rs/blob/main/SECURITY.md)
for responsible disclosure policy.

---

## License

quack-rs is licensed under the [MIT License](https://github.com/tomtom215/quack-rs/blob/main/LICENSE).
Contributions are accepted under the same license. By submitting a pull request,
you agree to license your contribution under MIT.
