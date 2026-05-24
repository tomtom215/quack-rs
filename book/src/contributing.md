# Contributing

quack-rs is an open source project. Contributions of all kinds are welcome:
bug reports, documentation improvements, new pitfall discoveries, and code.

---

## Development prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | ≥ 1.87.0 (MSRV) | Compiler |
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

# MSRV — must compile on Rust 1.87.0 (excludes benches; matches CI)
cargo +1.87.0 check
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

```
quack-rs/
├── src/
│   ├── lib.rs                     # Crate root; module declarations; DUCKDB_API_VERSION
│   ├── entry_point.rs             # init_extension() / init_extension_v2() + entry_point! / entry_point_v2!
│   ├── connection.rs              # Connection facade + Registrar trait (version-agnostic registration)
│   ├── config.rs                  # DbConfig — RAII wrapper for duckdb_config
│   ├── error.rs                   # ExtensionError, ExtResult<T>
│   ├── interval.rs                # DuckInterval, interval_to_micros
│   ├── sql_macro.rs               # SqlMacro — CREATE MACRO without FFI callbacks
│   ├── aggregate/
│   │   ├── mod.rs
│   │   ├── builder/               # Builder types for aggregate function registration
│   │   │   ├── mod.rs             # Module doc + re-exports
│   │   │   ├── single.rs          # AggregateFunctionBuilder (single-signature)
│   │   │   ├── set.rs             # AggregateFunctionSetBuilder, OverloadBuilder
│   │   │   └── tests.rs           # Unit tests
│   │   ├── info.rs                # AggregateFunctionInfo
│   │   ├── callbacks.rs           # Callback type aliases
│   │   └── state.rs               # AggregateState trait, FfiState<T>
│   ├── scalar/
│   │   ├── mod.rs
│   │   ├── info.rs                # ScalarFunctionInfo, ScalarBindInfo, ScalarInitInfo
│   │   └── builder/               # Builder types for scalar function registration
│   │       ├── mod.rs             # Module doc + re-exports
│   │       ├── single.rs          # ScalarFn type alias, ScalarFunctionBuilder
│   │       ├── set.rs             # ScalarFunctionSetBuilder, ScalarOverloadBuilder
│   │       └── tests.rs           # Unit tests
│   ├── catalog.rs                 # Catalog access helpers (requires `duckdb-1-5`)
│   ├── cast/
│   │   ├── mod.rs                 # Re-exports
│   │   └── builder.rs             # CastFunctionBuilder, CastFunctionInfo, CastMode
│   ├── client_context.rs          # ClientContext wrapper (requires `duckdb-1-5`)
│   ├── config_option.rs           # ConfigOption registration (requires `duckdb-1-5`)
│   ├── copy_function/
│   │   ├── mod.rs                 # CopyFunctionBuilder (requires `duckdb-1-5`)
│   │   └── info.rs                # CopyBindInfo, CopySinkInfo, etc.
│   ├── appender.rs                # Appender — bulk row insertion (requires `duckdb-1-5`)
│   ├── error_data.rs              # ErrorData, DuckDbErrorType — structured errors (requires `duckdb-1-5`)
│   ├── expression.rs              # Expression — bound expr inspection/folding (requires `duckdb-1-5`)
│   ├── file_system.rs             # FileSystem, FileHandle — DuckDB virtual file system (requires `duckdb-1-5`)
│   ├── instance_cache.rs          # InstanceCache — shared DB instance cache (requires `duckdb-1-5`)
│   ├── selection_vector.rs        # SelectionVector — zero-copy row-index vectors (requires `duckdb-1-5`)
│   ├── replacement_scan/
│   │   └── mod.rs                 # ReplacementScanBuilder — SELECT * FROM 'file.xyz' patterns
│   ├── types/
│   │   ├── mod.rs
│   │   ├── type_id.rs             # TypeId enum (33 base + 6 with duckdb-1-5)
│   │   └── logical_type.rs        # LogicalType RAII wrapper
│   ├── vector/
│   │   ├── mod.rs
│   │   ├── reader.rs              # VectorReader
│   │   ├── writer.rs              # VectorWriter
│   │   ├── validity.rs            # ValidityBitmap
│   │   ├── string.rs              # DuckStringView, read_duck_string
│   │   └── complex.rs             # StructVector, ListVector, MapVector, ArrayVector
│   ├── validate/
│   │   ├── mod.rs
│   │   ├── description_yml/       # Parse and validate description.yml metadata
│   │   │   ├── mod.rs             # Module doc + re-exports
│   │   │   ├── model.rs           # DescriptionYml struct
│   │   │   ├── parser.rs          # parse_description_yml and helpers
│   │   │   ├── validator.rs       # validate_description_yml_str, validate_rust_extension
│   │   │   └── tests.rs           # Unit tests
│   │   ├── extension_name.rs
│   │   ├── function_name.rs
│   │   ├── platform.rs
│   │   ├── release_profile.rs
│   │   ├── semver.rs
│   │   └── spdx.rs
│   ├── scaffold/
│   │   ├── mod.rs                 # ScaffoldConfig, GeneratedFile, generate_scaffold
│   │   ├── templates.rs           # Template generators for scaffold files (pub(super))
│   │   └── tests.rs               # Unit tests
│   ├── table_description.rs       # TableDescription wrapper (requires `duckdb-1-5`)
│   ├── table/
│   │   ├── mod.rs
│   │   ├── builder.rs             # TableFunctionBuilder, BindFn/InitFn/ScanFn aliases
│   │   ├── info.rs                # BindInfo, InitInfo, FunctionInfo
│   │   ├── bind_data.rs           # FfiBindData<T>
│   │   └── init_data.rs           # FfiInitData<T>, FfiLocalInitData<T>
│   └── testing/
│       ├── mod.rs
│       ├── harness.rs             # AggregateTestHarness<S>
│       ├── mock_vector.rs         # MockVectorReader, MockVectorWriter, MockDuckValue
│       ├── mock_registrar.rs      # MockRegistrar, CastRecord
│       └── in_memory_db.rs        # InMemoryDb (requires `bundled-test`)
├── tests/
│   └── integration_test.rs
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
├── LESSONS.md                     # The 16 DuckDB Rust FFI pitfalls
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
