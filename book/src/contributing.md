# Contributing

quack-rs is an open source project. Contributions of all kinds are welcome:
bug reports, documentation improvements, new pitfall discoveries, and code.

---

## Development prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | ≥ 1.84.1 (MSRV) | Compiler |
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

# MSRV — must compile on Rust 1.84.1
cargo +1.84.1 check
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
│   ├── entry_point.rs             # init_extension()
│   ├── error.rs                   # ExtensionError, ExtResult<T>
│   ├── interval.rs                # DuckInterval, interval_to_micros
│   ├── sql_macro.rs               # SqlMacro — CREATE MACRO without FFI callbacks
│   ├── aggregate/
│   │   ├── mod.rs
│   │   ├── builder.rs             # AggregateFunctionBuilder, AggregateFunctionSetBuilder
│   │   ├── callbacks.rs           # Callback type aliases
│   │   └── state.rs               # AggregateState trait, FfiState<T>
│   ├── scalar/
│   │   ├── mod.rs
│   │   └── builder.rs             # ScalarFunctionBuilder
│   ├── types/
│   │   ├── mod.rs
│   │   ├── type_id.rs             # TypeId enum (21 variants)
│   │   └── logical_type.rs        # LogicalType RAII wrapper
│   ├── vector/
│   │   ├── mod.rs
│   │   ├── reader.rs              # VectorReader
│   │   ├── writer.rs              # VectorWriter
│   │   ├── validity.rs            # ValidityBitmap
│   │   └── string.rs              # DuckStringView, read_duck_string
│   ├── validate/
│   │   ├── mod.rs
│   │   ├── extension_name.rs
│   │   ├── function_name.rs
│   │   ├── platform.rs
│   │   ├── release_profile.rs
│   │   ├── semver.rs
│   │   └── spdx.rs
│   ├── scaffold/
│   │   └── mod.rs                 # generate_scaffold, ScaffoldConfig
│   └── testing/
│       ├── mod.rs
│       └── harness.rs             # AggregateTestHarness<S>
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
├── LESSONS.md                     # The 15 DuckDB Rust FFI pitfalls
├── CHANGELOG.md
└── README.md
```

---

## Releasing

quack-rs pins `libduckdb-sys` with `=` (exact version) because the DuckDB C API
can change between minor releases. Before bumping the pin:

1. Read the DuckDB changelog for C API changes
2. Check the new C API version string (used in `duckdb_rs_extension_api_init`)
3. Update `DUCKDB_API_VERSION` in `src/lib.rs` if the C API version changed
4. Audit all callback signatures against the new `bindgen.rs` output
5. Update all `=x.x.x` pins in `Cargo.toml` (runtime and dev-deps)

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
