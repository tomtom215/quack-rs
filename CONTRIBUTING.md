<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# Contributing to quack-rs

Thank you for contributing! Please read this document before opening a PR.

## Table of Contents

- [Development Prerequisites](#development-prerequisites)
- [Building](#building)
- [Coding Standards](#coding-standards)
- [Quality Gates](#quality-gates)
- [Test Strategy](#test-strategy)
- [Mutation Testing](#mutation-testing)
- [Code Standards](#code-standards)
- [Repository Structure](#repository-structure)
- [PR Checklist](#pr-checklist)
- [Releasing](#releasing)

---

## Development Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | ≥ 1.87.0 (MSRV) | Compiler |
| `rustfmt` | stable | Formatting |
| `clippy` | stable | Linting |
| `cargo-deny` | latest | License/advisory checks |
| DuckDB CLI | 1.4.4, 1.5.0, or 1.5.1 | Live extension testing (required) |

Install the Rust toolchain via [rustup](https://rustup.rs/).

Install DuckDB 1.5.1 (or 1.4.4/1.5.0) via `curl` (no system package manager needed):

```bash
curl -fsSL https://github.com/duckdb/duckdb/releases/download/v1.5.1/duckdb_cli-linux-amd64.zip \
    -o /tmp/duckdb.zip \
    && unzip -o /tmp/duckdb.zip -d /tmp/ \
    && chmod +x /tmp/duckdb \
    && /tmp/duckdb --version
# → v1.5.1
```

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

## Coding Standards

### Every file starts with the SPDX header

```rust
// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215)
```

Markdown / TOML / YAML files use the appropriate comment syntax.

### 500-line maximum per file

Source files (`.rs`) should generally stay under 500 lines. If your implementation
is growing beyond this limit, consider splitting it into focused sub-modules with
a thin `mod.rs` that only re-exports. Some files exceed this guideline where
splitting would harm cohesion.

### Thin `mod.rs` files

`mod.rs` files should primarily contain `mod` declarations and `pub use`
re-exports. Shared types that are tightly coupled to a module's children
may live in the parent `mod.rs` when splitting them out would add indirection
without value.

### No `unwrap()` in library code

Use `?`, `map_err`, `ok_or_else`, or explicit `match`. `expect()` is also
forbidden unless the message explains an invariant that is *impossible* to
violate at runtime (documented with `// SAFETY:` style comment).

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

# 6. MSRV — must compile on Rust 1.87.0 (matches CI; excludes benches which use criterion >=1.86)
cargo +1.87.0 check

# 7. Live extension test — build hello-ext, package it, load in DuckDB 1.4.4 or 1.5.0
cargo build --release --manifest-path examples/hello-ext/Cargo.toml
cargo run --bin append_metadata -- \
    examples/hello-ext/target/release/libhello_ext.so \
    /tmp/hello_ext.duckdb_extension \
    --abi-type C_STRUCT --extension-version v0.1.0 \
    --duckdb-version v1.2.0 --platform linux_amd64
/tmp/duckdb -unsigned -c "
SET allow_extensions_metadata_mismatch=true;
LOAD '/tmp/hello_ext.duckdb_extension';
SELECT word_count('hello world foo');   -- 3
SELECT first_word('hello world');       -- hello
SELECT list(value ORDER BY value) FROM generate_series_ext(5);  -- [0,1,2,3,4]
SELECT CAST('42' AS INTEGER);           -- 42
SELECT TRY_CAST('bad' AS INTEGER);      -- NULL
"
```

```bash
# 8. Mutation testing — zero surviving mutants in changed files
cargo mutants --file <changed-files>
```

These same checks run in CI (`.github/workflows/ci.yml`) on every push and pull request.
Coverage and mutation testing run in separate workflows.

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
tests for all pure-Rust logic (`count_words`, `first_word`, `parse_varchar_to_int`,
aggregate state transitions). **Full end-to-end testing against a live DuckDB 1.4.4 or 1.5.0
instance is required** — not left to consumers. This means building the `.so`,
appending the extension metadata footer with `append_metadata`, and running all 19
SQL tests via the DuckDB CLI. See the [Quality Gates](#quality-gates) section for
the exact commands and `examples/hello-ext/README.md` for the full test listing.

### Mutation testing

Mutation testing verifies that your tests actually detect code changes. A mutant
is a small, deliberate modification to the source (e.g., replacing `+` with `-`,
flipping a boolean, returning a default value). If a mutant compiles and all
tests still pass, the test suite has a gap.

```bash
# Install cargo-mutants
cargo install cargo-mutants

# Run mutation tests on all library source
cargo mutants

# Run on a specific file
cargo mutants --file src/interval.rs

# List mutants without running (dry-run)
cargo mutants --list
```

Configuration is in `mutants.toml` at the repository root.

### Test naming convention

Tests follow the pattern: `{component}_{scenario}_{expected_outcome}`

Examples:
- `interval_to_micros_overflow_saturates`
- `error_from_string_preserves_message`
- `aggregate_state_combine_propagates_config`

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
│   ├── entry_point.rs             # init_extension() / init_extension_v2() + entry_point! / entry_point_v2! macros
│   ├── connection.rs              # Connection facade + Registrar trait (version-agnostic registration)
│   ├── config.rs                  # DbConfig — RAII wrapper for duckdb_config
│   ├── error.rs                   # ExtensionError, ExtResult<T>
│   ├── interval.rs                # DuckInterval, interval_to_micros (checked + saturating)
│   ├── prelude.rs                 # Convenience re-exports for extension authors
│   ├── sql_macro.rs               # SQL macro registration (CREATE MACRO, no FFI)
│   ├── aggregate/
│   │   ├── mod.rs                 # Re-exports
│   │   ├── builder/               # Builder types for aggregate function registration
│   │   │   ├── mod.rs             # Module doc + re-exports
│   │   │   ├── single.rs          # AggregateFunctionBuilder (single-signature)
│   │   │   ├── set.rs             # AggregateFunctionSetBuilder, OverloadBuilder
│   │   │   └── tests.rs           # Unit tests (14 tests)
│   │   ├── callbacks.rs           # Type aliases for the 6 callback signatures
│   │   └── state.rs               # AggregateState trait, FfiState<T>
│   ├── scalar/
│   │   ├── mod.rs                 # Re-exports
│   │   └── builder/               # Builder types for scalar function registration
│   │       ├── mod.rs             # Module doc + re-exports
│   │       ├── single.rs          # ScalarFn type alias, ScalarFunctionBuilder
│   │       ├── set.rs             # ScalarFunctionSetBuilder, ScalarOverloadBuilder
│   │       └── tests.rs           # Unit tests (13 tests)
│   ├── catalog.rs                 # Catalog access helpers (requires `duckdb-1-5`)
│   ├── cast/
│   │   ├── mod.rs                 # Re-exports
│   │   └── builder.rs             # CastFunctionBuilder, CastFunctionInfo, CastMode
│   ├── client_context.rs          # ClientContext wrapper (requires `duckdb-1-5`)
│   ├── config_option.rs           # ConfigOption registration (requires `duckdb-1-5`)
│   ├── copy_function/
│   │   ├── mod.rs                 # CopyFunctionBuilder (requires `duckdb-1-5`)
│   │   └── info.rs                # CopyBindInfo, CopySinkInfo, CopyGlobalInitInfo, CopyFinalizeInfo
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
│   │   ├── type_id.rs             # TypeId enum (all DuckDB column types)
│   │   └── logical_type.rs        # LogicalType — RAII wrapper for duckdb_logical_type
│   ├── vector/
│   │   ├── mod.rs
│   │   ├── reader.rs              # VectorReader — typed reads from a DuckDB data chunk
│   │   ├── writer.rs              # VectorWriter — typed writes to a DuckDB result vector
│   │   ├── validity.rs            # ValidityBitmap — NULL flag management
│   │   └── string.rs              # DuckStringView, read_duck_string (16-byte string format)
│   ├── validate/
│   │   ├── mod.rs                 # Extension compliance validators + re-exports
│   │   ├── description_yml/       # Parse and validate description.yml metadata
│   │   │   ├── mod.rs             # Module doc + re-exports
│   │   │   ├── model.rs           # DescriptionYml struct (11 fields)
│   │   │   ├── parser.rs          # parse_description_yml, parse_kv, strip_inline_comment
│   │   │   ├── validator.rs       # validate_description_yml_str, validate_rust_extension
│   │   │   └── tests.rs           # Unit tests (20 tests)
│   │   ├── extension_name.rs      # Extension name validation (^[a-z][a-z0-9_-]*$)
│   │   ├── function_name.rs       # SQL function name validation
│   │   ├── platform.rs            # DuckDB build platform validation
│   │   ├── release_profile.rs     # Cargo release profile validation
│   │   ├── semver.rs              # Semantic versioning + extension version tiers
│   │   └── spdx.rs                # SPDX license identifier validation
│   ├── scaffold/
│   │   ├── mod.rs                 # ScaffoldConfig, GeneratedFile, generate_scaffold
│   │   ├── templates.rs           # Template generators for all 11 scaffold files (pub(super))
│   │   └── tests.rs               # Unit tests (29 tests)
│   ├── table_description.rs       # TableDescription wrapper (requires `duckdb-1-5`)
│   ├── table/
│   │   ├── mod.rs                 # Re-exports
│   │   ├── builder.rs             # TableFunctionBuilder, type aliases (BindFn, InitFn, ScanFn)
│   │   ├── info.rs                # BindInfo, InitInfo, FunctionInfo — callback info wrappers
│   │   ├── bind_data.rs           # FfiBindData<T> — type-safe bind-phase data
│   │   └── init_data.rs           # FfiInitData<T>, FfiLocalInitData<T>
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
│   ├── ci.yml                     # CI: check, test, clippy, fmt, doc, msrv, bench-compile, nightly
│   ├── release.yml                # Release pipeline: CI gate, package, publish
│   ├── docs.yml                   # mdBook build & deploy to GitHub Pages
│   ├── coverage.yml               # Test coverage (cargo-llvm-cov → Codecov)
│   ├── mutants.yml                # Mutation testing (cargo-mutants)
│   ├── benchmarks.yml             # Criterion benchmark execution
│   └── README.md                  # Workflow overview and quality gate summary
├── CONTRIBUTING.md                # This file
├── LESSONS.md                     # The 16 DuckDB Rust FFI pitfalls, documented in full
└── README.md                      # Quick start, SDK overview, badge table
```

---

## PR Checklist

- [ ] SPDX header on every new file
- [ ] No file exceeds 500 lines
- [ ] `cargo fmt` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test --all-targets` passes
- [ ] `cargo doc --no-deps` passes without warnings
- [ ] New public types/functions have doc comments
- [ ] New code has tests
- [ ] All `unsafe` blocks have a `// SAFETY:` comment
- [ ] `CHANGELOG.md` updated under `[Unreleased]` (for user-facing changes)
- [ ] Book (`book/src/`) updated if the change affects extension authors
- [ ] New FFI pitfall discovered → added to `LESSONS.md` and `book/src/reference/pitfalls.md`
- [ ] `cargo mutants --file <changed-files>` shows zero surviving mutants for changed files

---

## Releasing

This crate supports `libduckdb-sys = ">=1.4.4, <2"` (DuckDB 1.4.x and 1.5.x).
The bounded range is intentional: the C API (`v1.2.0`) is stable across these releases,
and the `<2` upper bound prevents silent adoption of a future major band.
Before broadening the range to a new major band:

1. Read the DuckDB changelog for C API changes.
2. Check the new C API version string (used in `duckdb_rs_extension_api_init`).
3. Update `DUCKDB_API_VERSION` in `src/lib.rs` if the C API version changed.
4. Audit all callback signatures against the new `bindgen.rs` output.
5. Update the range bounds in `Cargo.toml` (both runtime and dev-deps).

Versions follow [Semantic Versioning](https://semver.org/). Breaking changes to
public API require a major version bump.
