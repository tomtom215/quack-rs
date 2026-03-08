# Contributing to quack-rs

## Table of Contents

- [Development Prerequisites](#development-prerequisites)
- [Building](#building)
- [Quality Gates](#quality-gates)
- [Test Strategy](#test-strategy)
- [Code Standards](#code-standards)
- [Repository Structure](#repository-structure)
- [Releasing](#releasing)

---

## Development Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | ≥ 1.84.1 (MSRV) | Compiler |
| `rustfmt` | stable | Formatting |
| `clippy` | stable | Linting |
| `cargo-deny` | latest | License/advisory checks |

Install the Rust toolchain via [rustup](https://rustup.rs/).

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

## Quality Gates

**All of the following must pass before merging any pull request:**

```bash
# 1. Tests — zero failures, zero ignored
cargo test

# 2. Integration tests
cargo test --test integration_test

# 3. Linting — zero warnings (warnings are treated as errors)
cargo clippy --all-targets -- -D warnings

# 4. Formatting
cargo fmt -- --check

# 5. Documentation — zero broken links or missing docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# 6. MSRV — must compile on Rust 1.84.1
cargo +1.84.1 check --all-targets
```

These same checks run in CI (`.github/workflows/ci.yml`) on every push and pull request.

---

## Test Strategy

### Unit tests

Unit tests live in `#[cfg(test)]` modules within each source file. They test
pure-Rust logic that does not require a live DuckDB instance.

**Constraint**: `libduckdb-sys` with `features = ["loadable-extension"]` makes
every DuckDB C API function go through lazy `AtomicPtr` dispatch. These pointers
are only initialized when `duckdb_rs_extension_api_init` is called from within a
real DuckDB extension load. Calling any DuckDB API function in a unit test will
panic. Move such tests to integration tests or example-extension tests.

### Integration tests (`tests/integration_test.rs`)

Pure-Rust tests that cross module boundaries — e.g., testing `interval` with
`AggregateTestHarness`, or verifying `FfiState` lifecycle across module boundaries.
These still cannot call `duckdb_*` functions, for the same reason as unit tests.

### Property-based tests

Selected modules include `proptest`-based tests for mathematical properties:
- `interval.rs` — overflow edge cases across the full `i32`/`i64` range
- `testing/harness.rs` — sum associativity, identity element for `AggregateState`

### Example-extension tests (`examples/hello-ext/`)

The `hello-ext` example compiles as a `cdylib` and contains `#[cfg(test)]` unit
tests for the pure logic (`count_words`). Full end-to-end testing (loading the
`.so` into a DuckDB CLI or embedding in a test binary) is left to consumers.

---

## Code Standards

### Safety documentation

Every `unsafe` block must have a `// SAFETY:` comment that explains:
1. Which invariant the caller guarantees
2. Why the operation is valid given that invariant

Example:
```rust
// SAFETY: `states` is a valid array of `count` pointers, each initialized
// by `init_callback`. We are the only owner of `inner` at this point.
unsafe { drop(Box::from_raw(ffi.inner)) };
```

### No panics across FFI

`unwrap()`, `expect()`, and `panic!()` are forbidden inside any function that
may be called by DuckDB (callbacks and entry points). Use `Option`/`Result` and
the `?` operator throughout. See `entry_point::init_extension` for the canonical
pattern.

### Clippy lint policy

The crate enables `pedantic`, `nursery`, and `cargo` lint groups. Specific lints
are suppressed only where they produce false positives for SDK API patterns:

```toml
[lints.clippy]
module_name_repetitions = "allow"  # e.g., AggregateFunctionBuilder
must_use_candidate = "allow"       # builder methods
missing_errors_doc = "allow"       # unsafe extern "C" callbacks
return_self_not_must_use = "allow" # builder pattern
```

All other warnings are errors in CI.

### Documentation

Every public item must have a doc comment. Private items with non-obvious
semantics should also be documented. Doc comments follow these conventions:

- First line: short summary (noun phrase, no trailing period)
- `# Safety`: mandatory on every `unsafe fn`
- `# Panics`: mandatory if the function can panic in any reachable code path
- `# Errors`: mandatory on functions returning `Result`
- `# Example`: encouraged on public types and key methods

---

## Repository Structure

```
quack-rs/
├── src/
│   ├── lib.rs                     # Crate root; module declarations; DUCKDB_API_VERSION
│   ├── entry_point.rs             # init_extension() + entry_point! macro
│   ├── error.rs                   # ExtensionError, ExtResult<T>
│   ├── interval.rs                # DuckInterval, interval_to_micros (checked + saturating)
│   ├── prelude.rs                 # Convenience re-exports for extension authors
│   ├── sql_macro.rs               # SQL macro registration (CREATE MACRO, no FFI)
│   ├── aggregate/
│   │   ├── mod.rs                 # Re-exports
│   │   ├── builder.rs             # AggregateFunctionBuilder, AggregateFunctionSetBuilder
│   │   ├── callbacks.rs           # Type aliases for the 6 callback signatures
│   │   └── state.rs               # AggregateState trait, FfiState<T>
│   ├── scalar/
│   │   └── builder.rs             # ScalarFunctionBuilder
│   ├── types/
│   │   ├── mod.rs
│   │   ├── type_id.rs             # TypeId enum (all DuckDB column types)
│   │   └── logical_type.rs        # LogicalType — RAII wrapper for duckdb_logical_type
│   ├── vector/
│   │   ├── mod.rs
│   │   ├── reader.rs              # VectorReader — typed reads from a DuckDB data chunk
│   │   ├── writer.rs              # VectorWriter — typed writes to a DuckDB result vector
│   │   ├── validity.rs            # ValidityBitmap — NULL flag management
│   │   └── string.rs              # DuckStringView, read_duck_string (16-byte string format)
│   ├── validate/
│   │   ├── mod.rs                 # Extension compliance validators
│   │   └── description_yml.rs     # Parse and validate description.yml metadata
│   ├── scaffold/
│   │   └── mod.rs                 # Project generator for new extensions
│   └── testing/
│       ├── mod.rs
│       └── harness.rs             # AggregateTestHarness<S> — unit-test aggregate logic
├── tests/
│   └── integration_test.rs        # Cross-module pure-Rust integration tests
├── benches/
│   └── interval_bench.rs          # Criterion benchmarks for interval conversion
├── examples/
│   └── hello-ext/                 # Complete word_count aggregate extension example
│       ├── Cargo.toml
│       └── src/lib.rs
├── book/                          # mdBook documentation source
├── .github/workflows/
│   ├── ci.yml                     # CI: check, test, clippy, fmt, doc, msrv, bench-compile
│   └── release.yml                # Release pipeline: CI gate, package, publish
├── CONTRIBUTING.md                # This file
├── LESSONS.md                     # The 15 DuckDB Rust FFI pitfalls, documented in full
└── README.md                      # Quick start, SDK overview, badge table
```

---

## Releasing

This crate is pinned to `libduckdb-sys = "=1.4.4"` because the DuckDB C API
can change between minor releases. Before bumping the pin:

1. Read the DuckDB changelog for C API changes.
2. Check the new C API version string (used in `duckdb_rs_extension_api_init`).
3. Update `DUCKDB_API_VERSION` in `src/lib.rs` if the C API version changed.
4. Audit all callback signatures against the new `bindgen.rs` output.
5. Update all `=1.x.x` pins in `Cargo.toml` (both runtime and dev-deps).

Versions follow [Semantic Versioning](https://semver.org/). Breaking changes to
public API require a major version bump.
