<!-- SPDX-License-Identifier: MIT -->
<!-- Copyright 2026 Tom F. <tomf@tomtomtech.net> (https://github.com/tomtom215) -->

# Contributing to quack-rs

Thank you for contributing! Please read this document before opening a PR.

Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

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
| Rust | the exact release `rust-toolchain.toml` pins (1.99.1); rustup installs it on first use | Compiler (see [The Rust toolchain](#the-rust-toolchain)) |
| Rust 1.86.0 toolchain | `rustup toolchain install 1.86.0` | MSRV check (gate 6) |
| `rustfmt`, `clippy` | the pinned release (`rust-toolchain.toml` lists both components) | Formatting and linting (gates 3, 4) |
| Python 3 | 3.11 or later (two scripts use `tomllib`) | `scripts/*.py`, including the live extension check (gates 5b, 7) |
| DuckDB CLI | 1.4.4 or later; CI also uses 2.0.0 and the latest (currently 2.0.0) | Live extension testing (gate 7, required) |
| libduckdb shared library | the release `Cargo.lock` pins (currently 1.5.6) | End-to-end tests and book doctests (gate 2c) |
| `cargo-deny` | 0.20.2 (the version CI installs) | License and advisory checks (gate 8) |
| `cargo-mutants` | 27.1.0 (the version CI installs) | Mutation testing (gate 9) |
| `mdbook` | the version in `book/mdbook-version` | Building the book (only for book changes) |
| `typos`, `taplo`, `cargo-machete`, `actionlint` | 1.46.0, 0.10.0, 0.9.2, 1.7.12 (the release binaries CI downloads) | Spelling, TOML style, unused dependencies, workflow lint (gate 8b; optional locally) |

Install the Rust toolchain via [rustup](https://rustup.rs/). `cargo-msrv` is not
needed: the MSRV is checked by compiling with the pinned 1.86.0 toolchain.

**DuckDB 2.0's CLI changes its output when it thinks an AI agent is driving
it.** With `AI_AGENT` or `CLAUDECODE` set in the environment and stdout not a
terminal, the 2.0.0 CLI prints `duckdb agent mode on: ...` and renders results
as a Markdown table with column types (`| answer:INTEGER |`) instead of its
default box layout. Anything that parses the CLI's output must choose a
format: `-csv` or `-json` (both work on every DuckDB, and override agent
mode), or `-no-agent`, which turns it off but exists only in the 2.0 CLI
(1.5.6 exits with "Unrecognized option '-no-agent'"). The scripts here that
read query results (`scripts/check-hello-ext.py`, the CI probes) pass `-csv`;
GitHub's runners set neither variable.

Install DuckDB with `scripts/fetch-duckdb.sh` (no system package manager
needed; the script fetches the linux-amd64 build). CI's `extension-load` job exercises **v1.4.4, v1.5.0, v1.5.5, v1.5.6,
v2.0.0 and `latest`** — the floor, the 1.5 floor, the newest 1.5 releases, the
current release, and an early-warning signal for a release not yet pinned
(`latest` is currently v2.0.0). The current release is v2.0.0, but `Cargo.lock`
stays on the v1.5.6 bindings, because the DuckDB 2.0 bindings (`libduckdb-sys`
1.20000.x) need Rust 1.88, above the 1.86 MSRV, and several CI jobs build one
extension against the 546-slot v1.5.6 layout and load it into both 1.5.6 and
2.0.0. CI's `test-duckdb-2-0` job runs the suite against v2.0.0 on the 2.0
bindings. Fetch the CLI of the release you develop against:

```bash
scripts/fetch-duckdb.sh v1.5.6 cli /tmp    # or v2.0.0, or latest
/tmp/duckdb --version
```

DuckDB renamed its release assets in v2.0.0 (`duckdb_cli-<platform>.zip` and
`libduckdb-<platform>.zip` up to 1.5.6; `duckdb-cli-<platform>.tar.gz` and
`duckdb-shared-libs-<platform>.tar.gz` from 2.0.0), and the script picks the
right one.

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

### The book

The book at [quack-rs.com](https://quack-rs.com/) is versioned. `/` is the
latest release, `/vX.Y/` is each supported release line built from its newest
tag, and `/dev/` is `main`. A version menu sits in the menu bar of every page,
and pages outside the latest release carry a banner saying which version they
describe. `scripts/build-versioned-site.py` builds the site and documents the
layout. CI's `book` job runs it on every pull request, and `docs.yml` deploys
it.

So a change to `book/src/` is visible at `/dev/` once merged and at `/` once
released. Preview the whole site locally (needs the release tags,
`git fetch --tags`):

```bash
cargo doc --no-deps --features duckdb-1-5-4
python3 scripts/build-versioned-site.py --out target/site --dev-api target/doc
python3 -m http.server -d target/site 8000   # http://localhost:8000/dev/
```

For a quick look at the current book alone, `mdbook build` (the version in
`book/mdbook-version`) writes it to `book/book/`.

---

## Coding Standards

### Every file starts with the SPDX header

```rust
// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!
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
violate at runtime (documented with `// SAFETY:` style comment), or the panic
is the function's documented contract: a `# Panics` section states it and a
fallible alternative exists (the builders' `new(name)` panics on an interior
NUL; `try_new(name)` returns the error instead).

---

## Quality Gates

This is the canonical list of quality gates; the README and the book link here.
**All of the following must pass before merging any pull request.** Each one
mirrors a job in `.github/workflows/ci.yml` (job names in brackets).

```bash
# 1. Tests — zero failures [test]. `--all-targets` covers the unit tests,
#    tests/integration_test.rs, examples and benches, but NOT doctests (2b).
cargo test --all-targets

# 2. Feature-gated code — the `duckdb-1-5*` modules compile only with a feature
#    [test-duckdb-1-5]
cargo test --all-targets --features duckdb-1-5-4

# 2b. Doctests — `--all-targets` does NOT include them, so they need their own
#     run. For an SDK the rustdoc examples are part of the product. [test]
cargo test --doc --features duckdb-1-5-4

# 2c. End-to-end tests and the book's code blocks, against a real libduckdb
#     [test-bundled-prebuilt]. Download the libduckdb release that matches
#     Cargo.lock (`scripts/duckdb-version-from-lock.sh` prints its tag) into
#     $DUCKDB_LIB_DIR; on linux-amd64:
#       scripts/fetch-duckdb.sh "$(scripts/duckdb-version-from-lock.sh)" lib "$DUCKDB_LIB_DIR"
export DUCKDB_LIB_DIR=/path/to/libduckdb LD_LIBRARY_PATH=/path/to/libduckdb
cargo test --all-targets --features bundled-test-prebuilt,duckdb-1-5-4
cargo test --doc --manifest-path book/doctest/Cargo.toml   # every Rust block in book/src and README.md

# 3. Linting — zero warnings (warnings are treated as errors) [clippy, test-duckdb-1-5]
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features duckdb-1-5-4 -- -D warnings

# 4. Formatting [fmt]
cargo fmt -- --check

# 5. Documentation — zero broken links or missing docs, with and without
#    the feature-gated modules [doc]
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --features duckdb-1-5-4

# 5b. The book's changelog page is GENERATED from CHANGELOG.md. Edit
#     CHANGELOG.md, then regenerate. CI runs the `--check` form and fails if
#     the two differ. [doc]
scripts/sync-book-changelog.py
scripts/sync-book-changelog.py --check

# 6. MSRV — must compile on Rust 1.86.0 [msrv]. Benches are excluded (criterion
#    needs a newer compiler). `+1.86.0` is required, not stylistic:
#    `rust-toolchain.toml` pins a newer release (`channel`) and a toolchain
#    file overrides rustup's default, so a bare `cargo check` silently runs
#    the pinned release and checks nothing.
#
#    Against what a 1.86.0 user's Cargo resolves, not the committed Cargo.lock:
#    the lock carries `libduckdb-sys` 1.10506.1, which declares rust-version
#    1.88, so a bare `cargo +1.86.0 check` fails on it. CI re-resolves with the
#    rust-version-aware fallback first; do the same on a copy, because `update`
#    rewrites Cargo.lock:
#      cp -r . /tmp/msrv && cd /tmp/msrv
#      CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo +1.86.0 update
cargo +1.86.0 check
cargo +1.86.0 check --features duckdb-1-5-4

# 7. Live extension test — build hello-ext, append the metadata footer, then
#    run every documented SQL check (examples/hello-ext/sql_checks.txt, 31
#    statements) in the DuckDB CLI. [extension-load: DuckDB 1.4.4, 1.5.0,
#    1.5.5, 1.5.6, 2.0.0 and the latest release]
cargo build --release --manifest-path examples/hello-ext/Cargo.toml
cargo run --bin append_metadata -- \
    examples/hello-ext/target/release/libhello_ext.so \
    /tmp/hello_ext.duckdb_extension \
    --abi-type C_STRUCT --extension-version v0.1.0 \
    --duckdb-version v1.2.0 --platform linux_amd64
python3 scripts/check-hello-ext.py --duckdb /tmp/duckdb \
    --extension /tmp/hello_ext.duckdb_extension
# → 31/31 checks pass with the extension loaded

# 7b. The example extension's own tests and lints [example-check]
cargo test --manifest-path examples/hello-ext/Cargo.toml
cargo clippy --manifest-path examples/hello-ext/Cargo.toml --all-targets -- -D warnings

# 8. Supply chain — license policy and advisories [security]. The example and
#    CI-only crates have their own policy file (deny.toml's license rules
#    cannot hold for them; see its header).
cargo deny check
for m in examples/hello-ext fuzz book/doctest; do
    cargo deny --manifest-path "$m/Cargo.toml" --config deny.ci-crates.toml check
done

# 8b. Hygiene and workflow lint [hygiene, workflow-lint]: spelling (typos.toml),
#     TOML style (.taplo.toml), unused dependencies, and the workflow files
#     (actionlint runs shellcheck on every `run:` block when it is installed).
typos
taplo fmt --check && taplo lint
cargo machete --with-metadata
actionlint
```

```bash
# 9. Mutation testing — zero surviving mutants in changed files
#    (`.github/workflows/mutants.yml`, a separate workflow)
cargo mutants --file <changed-files>
```

CI sets `RUSTFLAGS="-D warnings"` for every job, so a compiler warning anywhere
fails the build. CI runs these checks on every pull request to `main`, on pushes to
`main` and `claude/**`, weekly, and as the release's gate, together with jobs that
have no single local command: the other test operating systems, older DuckDB
engines (`test-older-engines`), WASM, Miri, sanitizers, semver checks, the
scaffold and ABI-guard end-to-end tests, the book build and its link check,
and several consistency scripts under `scripts/`. Coverage and mutation testing
run in separate workflows.

### The Rust toolchain

`rust-toolchain.toml` pins one exact stable release (`channel = "1.99.1"`).
rustup reads it, so a plain `cargo` here uses that release, and every CI job
that is not about another toolchain installs the same one; the release
workflow packages and publishes with it. `scripts/check-toolchain-pin.py` (in
the `doc` job) fails when a workflow names any other toolchain outside the
informational jobs. The reason: CI runs with `-D warnings` and clippy's
`pedantic`, `nursery` and `cargo` groups, so a floating `stable` let every Rust
release (one every six weeks) change rustc, rustfmt and clippy under every job
at once, with no change in this repository; 1.99.0's `clippy::assert_is_empty`
did exactly that (AUDIT.md 12.7).

The exceptions are the MSRV job (`cargo +1.86.0`), the jobs that need nightly
(Miri, the sanitizers, the fuzzer), and three informational jobs that look
ahead without blocking: `latest-stable` (the current stable: `fmt`, clippy on
both feature sets, rustdoc, the library tests), `clippy-beta` and `nightly`.
They run on every pull request and on the weekly schedule.

Move the pin by a pull request of its own, when `latest-stable` shows a newer
stable:

1. Check that the release exists: `rustup toolchain install X.Y.Z` (or read
   `https://static.rust-lang.org/dist/channel-rust-X.Y.Z.toml`).
2. Change `channel` in `rust-toolchain.toml` and every `toolchain: <old>` in
   `.github/workflows/*.yml`; `python3 scripts/check-toolchain-pin.py` lists
   any you missed.
3. Run the quality gates above (plain `cargo` now uses the new release) and fix
   what new lints and warnings report, in the same pull request.

The MSRV (`rust-version`, 1.86.0) is a separate decision with its own rules
(RELEASING.md, "Semantic versioning policy").

---

## Test Strategy

### Unit tests

Unit tests live in `#[cfg(test)]` modules alongside the code. They test
pure-Rust logic that does not require a live DuckDB instance.

**Constraint**: `libduckdb-sys` with `features = ["loadable-extension"]` makes
every DuckDB C API function go through lazy `AtomicPtr` dispatch. These pointers
are only initialized when `duckdb_rs_extension_api_init` is called from within a
real DuckDB extension load — or by `testing::InMemoryDb::open()` under the
`bundled-test` / `bundled-test-prebuilt` features. Without one of those, calling
any DuckDB API function in a unit test panics ("DuckDB API not initialized").
Put such tests in `tests/ffi_roundtrip.rs` (below).

### Integration tests (`tests/integration_test.rs`)

Pure-Rust tests that cross module boundaries — e.g., testing `interval` with
`AggregateTestHarness`, or verifying `FfiState` lifecycle across module boundaries.
These do not call `duckdb_*` functions.

### End-to-end tests (`tests/ffi_roundtrip.rs` and `tests/ffi_roundtrip/`)

Every test here opens a real in-memory DuckDB through `InMemoryDb`, registers a
function built with quack-rs, runs SQL and checks the answer. Requires
`--features bundled-test-prebuilt,duckdb-1-5-4` (with `DUCKDB_LIB_DIR` or
`DUCKDB_DOWNLOAD_LIB=1`) or `bundled-test`. CI's LeakSanitizer and
AddressSanitizer jobs run exactly this target (`--test ffi_roundtrip`), so new
end-to-end tests belong in it — as a submodule declared with
`#[path = "ffi_roundtrip/<name>.rs"] mod <name>;` when the root file would
otherwise grow further. Know what those runs cannot see (next section).

### What the sanitizer and valgrind runs can and cannot see

CI's `leak-check` (LeakSanitizer) and `asan` (AddressSanitizer) jobs, like a
local valgrind run of the end-to-end suite, use the **release** libduckdb: the
same prebuilt library `test-bundled-prebuilt` links. That library carries its
own copy of jemalloc (it exports 769 `duckdb_je_*` symbols at v1.5.6 and 768
at v2.0.0, and defines no `malloc` or `free` of its own), and DuckDB's
`Allocator` takes vector data buffers, LIST entry arrays and string heaps from
it. Neither valgrind nor AddressSanitizer tracks the blocks jemalloc hands out,
so **an out-of-bounds or use-after-free write into a vector's data buffer or a
LIST's entry array is invisible to all of these runs**: the stale writes into a
LIST entry array of the `ListBuilder` regression found before 0.20.0 drew no
report under valgrind.

What they do catch: errors in memory the Rust side allocates (Rust's heap,
boxed aggregate states, the bind and init data quack-rs owns), and in what
DuckDB allocates through C++ `new` and `malloc`, which includes validity masks
(an out-of-bounds validity write *is* reported) and the objects behind its
handles, so LeakSanitizer still sees a handle that is never destroyed. A clean
sanitizer or valgrind run therefore says nothing about the pointer arithmetic
into vector buffers; the end-to-end tests' answers, and Miri for the pure-Rust
paths, are what cover it. A sanitizer run against a DuckDB built from source
without jemalloc would close the gap; none is set up (CI compiles DuckDB from
source in `test-bundled` but does not sanitize that build).

### Property-based tests

Selected modules include `proptest`-based tests for mathematical properties:
- `interval.rs` — overflow edge cases across the full `i32`/`i64` range
- `testing/harness.rs` — sum associativity, identity element for `AggregateState`

### Example-extension tests (`examples/hello-ext/`)

The `hello-ext` example compiles as a `cdylib` and contains `#[cfg(test)]` unit
tests for all pure-Rust logic (`count_words`, `first_word`, `parse_varchar_to_int`,
aggregate state transitions). **Full end-to-end testing against a live DuckDB is
required** — not left to consumers. This means building the `.so`, appending the extension
metadata footer with `append_metadata`, and running every SQL check in
`examples/hello-ext/sql_checks.txt` (29 numbered checks, 31 statements) with
`scripts/check-hello-ext.py`. The script also fails if the statements in
`examples/hello-ext/README.md` drift from that file. CI's `extension-load` job runs it
against DuckDB 1.4.4, 1.5.0, 1.5.5, 1.5.6, 2.0.0 and the latest release. Gate 7 under
[Quality Gates](#quality-gates) gives the exact commands.

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

Configuration is in `.cargo/mutants.toml` — the one place cargo-mutants reads it
from without `--config`.

**What the gate cannot see.** cargo-mutants 27.1.0 generates no mutants inside
the body of an `unsafe fn` (free, associated, `const`, trait-default and
`unsafe extern "C"` alike; it does mutate a safe function that holds `unsafe`
blocks), never mutates integer literals (`base.add(8)`, `>> 6`, `& 63`, `* 16`
have no mutants even in a visible function), and cannot look into a macro body.
In this crate that is the pointer arithmetic: by a heuristic count
(`scripts/mutants-expose-unsafe.py` strips comments and strings and matches
braces), 506 `unsafe fn`s hold 5,127 of 22,898 non-test source lines (22.4%),
among them most of `VectorReader`, `VectorWriter`, `ListBuilder`, the Arrow
layout walker and `FfiState`'s callbacks. So a "0 missed" result covers the 2,493
mutants cargo-mutants lists with this repository's configuration (3,748 with
`--no-config`), not those lines. The live suite and the sanitizer jobs
([above](#what-the-sanitizer-and-valgrind-runs-can-and-cannot-see)) are what
stand behind them.

To mutate them in an audit, run `scripts/mutants-expose-unsafe.py` on a copy of
the tree: it drops the `unsafe` keyword from signatures (the crate denies
`unsafe_op_in_unsafe_fn`, so the copy still compiles) and, with `--literals`,
turns the integer operands of pointer arithmetic into expressions cargo-mutants
can flip. On `src/vector/reader.rs` and `writer.rs` the mutant list goes from 6
to 192 (344 with literals). The script's header gives the commands. It is not
run in CI. Moving such arithmetic into small safe functions over integers and
slices (`stride_offset(idx, width)`, `validity_bit(words, idx)`) makes it both
mutable and checkable by Miri, and is the better fix where a function allows it.

### Test naming convention

Tests follow the pattern: `{component}_{scenario}_{expected_outcome}`

Examples from the suite:
- `harness_combine_propagates_config` (`tests/integration_test.rs`)
- `extension_error_message_preserved` (`tests/integration_test.rs`)
- `default_null_handling_does_not_propagate_nulls_for_scalar_functions`
  (`tests/ffi_roundtrip.rs`)

---

## Code Standards

### Safety documentation

Every `unsafe` block must have a `// SAFETY:` comment that explains:
1. Which invariant the caller guarantees
2. Why the operation is valid given that invariant

`clippy::undocumented_unsafe_blocks` enforces this in library code (CI treats
it as an error); test code is exempt.

Example:
```rust
// SAFETY: `ffi.inner` came from `Box::into_raw` and has not been freed;
// nothing else holds it, so reclaiming and dropping the box is sound.
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

- First line: a one-sentence summary, ending with a period
- `# Safety`: mandatory on every `unsafe fn`
- `# Panics`: mandatory if the function can panic in any reachable code path
- `# Errors`: mandatory on functions returning `Result`
- `# Example`: encouraged on public types and key methods

---

## Repository Structure

```
quack-rs/
├── src/
│   ├── abi.rs                         # `DuckDB` C Extension API ABI compatibility checking
│   ├── appender.rs                    # Bulk data appending
│   ├── arrow.rs                       # Arrow C Data Interface bridge (`duckdb-1-5-4` feature; floor set by the `libduckdb-sys` 1.10504.0 bindings)
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
│   ├── entry_point/
│   │   └── registration.rs            # Runs the registration closure in one transaction, or with autocommit
│   ├── error.rs                       # Error types for `DuckDB` extension FFI error propagation
│   ├── error_data.rs                  # Structured error data (`DuckDB` 1.5.0+)
│   ├── expression.rs                  # Bound expressions (`DuckDB` 1.5.0+)
│   ├── extra_info.rs                  # Ownership of a function's `extra_info` allocation until `DuckDB` takes it
│   ├── file_system.rs                 # File system access (`DuckDB` 1.5.0+)
│   ├── instance_cache.rs              # Database instance cache (`DuckDB` 1.5.0+)
│   ├── interval.rs                    # `DuckDB` `INTERVAL` type conversion utilities
│   ├── lib.rs                         # Crate root: module list and crate-level docs
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
│   │       ├── collision.rs           # Which catalog entry holds the name a failed aggregate registration wanted
│   │       ├── mod.rs                 # Builder types for registering `DuckDB` aggregate functions
│   │       ├── overload.rs            # One overload within an [`AggregateFunctionSetBuilder`]
│   │       ├── set.rs                 # Builder for registering a `DuckDB` aggregate function set (multiple overloads)
│   │       ├── single/
│   │       │   └── register.rs        # `AggregateFunctionBuilder::register`
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
│   │   ├── export_check.rs            # Values `DuckDB` would export as different values, found before the export
│   │   ├── import_check.rs            # Structural checks on an imported array, and flattening what the readers cannot index
│   │   ├── import_layout/
│   │   │   ├── bounds.rs              # Offsets, lengths and dictionary indices `DuckDB` would read past, refused
│   │   │   └── tests.rs               # Unit tests
│   │   ├── import_layout.rs           # Arrow layouts `DuckDB` imports wrongly, found by walking the array with its schema
│   │   ├── options.rs                 # `ArrowOptions` — the Arrow production settings of a connection or a result
│   │   ├── schema.rs                  # `ArrowSchema` — an owned Arrow C Data Interface schema
│   │   └── tests.rs                   # Unit tests
│   ├── bin/
│   │   ├── append_metadata/
│   │   │   ├── cli.rs                 # Command-line parsing and validation (std-only, no clap)
│   │   │   ├── entry.rs               # The entry point DuckDB will look for, and the atomic write of the stamped file
│   │   │   ├── footer.rs              # The 512-byte `DuckDB` extension footer, and the optional 22-byte WebAssembly custom-section header that precedes it
│   │   │   ├── main.rs                # Append a DuckDB extension metadata block to a compiled .so / .dylib / .dll file,
│   │   │   ├── output.rs              # Reading the library and writing the stamped file
│   │   │   ├── tests.rs               # Unit tests
│   │   │   └── tests_output.rs        # Unit tests for reading the input and writing the output
│   │   └── quack_scaffold/
│   │       ├── cli.rs                 # Command-line parsing (std-only, no clap)
│   │       ├── main.rs                # Write a new `DuckDB` extension project to disk, from `generate_scaffold_with_warnings`
│   │       ├── tests.rs               # Unit tests
│   │       └── write.rs               # Writing the generated files: everything is checked before anything is written
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
│   │   ├── metadata.rs                # What a statement is before it runs: `StatementType`, parameter and column types, `table_names`
│   │   ├── prepared.rs                # Inspecting and executing a [`PreparedStatement`]; `bind.rs` binds its parameters
│   │   └── result.rs                  # Reading a [`QueryResult`]: its columns, its chunks and what kind of outcome it is
│   ├── replacement_scan/
│   │   └── mod.rs                     # Builder for registering `DuckDB` replacement scans
│   ├── scaffold/
│   │   ├── checks.rs                  # What `generate_scaffold` checks before it writes anything, and the warnings it gives
│   │   ├── escape.rs                  # Quoting configured free text for YAML and Rust doc comments
│   │   ├── licenses.rs                # The license text of a generated project
│   │   ├── mod.rs                     # Project scaffolding for `DuckDB` Rust extensions
│   │   ├── templates.rs               # Template generators for scaffold file content
│   │   ├── templates_ci.rs            # The generated project's CI and distribution workflows
│   │   ├── templates_docs.rs          # The generated project's `README.md`, `docs/function_descriptions.csv` and `deny.toml`
│   │   ├── templates_make.rs          # The generated `Makefile`: extension-ci-tools' variables and targets, and the skipped-test check
│   │   ├── tests.rs                   # Unit tests
│   │   ├── tests_content.rs           # What the scaffold accepts, warns about and refuses, and the files a community submission needs
│   │   ├── tests_escaping.rs          # Free text reaches the generated files intact
│   │   ├── tests_generated.rs         # Unit tests
│   │   ├── tests_make.rs              # Runs the generated `Makefile`'s own logic under `make`
│   │   └── tests_workflows.rs         # The generated workflows match the community build
│   ├── scalar/
│   │   ├── info.rs                    # Ergonomic wrapper around `duckdb_function_info` for scalar function callbacks
│   │   ├── mod.rs                     # Builder for registering `DuckDB` scalar functions
│   │   ├── state.rs                   # Typed bind data and per-thread local state for scalar functions (`DuckDB` 1.5.0+)
│   │   ├── typed.rs                   # Scalar functions written as ordinary Rust closures
│   │   ├── typed_builder.rs           # The builder the closure-based scalar constructors return, and the one `extern "C"` trampoline they all share
│   │   ├── typed/
│   │   │   ├── fallible.rs            # Closure-based scalar constructors whose closure can fail (`try_map*`)
│   │   │   ├── ternary.rs             # Three-argument closure-based scalar constructors (`map3`, `try_map3`)
│   │   │   └── tests.rs               # Unit tests
│   │   └── builder/
│   │       ├── collision.rs           # Refusing a scalar signature that would replace an existing one or make calls ambiguous
│   │       ├── mod.rs                 # Builder for registering `DuckDB` scalar functions
│   │       ├── overload.rs            # One overload within a [`ScalarFunctionSetBuilder`]
│   │       ├── set.rs                 # Builder for registering a `DuckDB` scalar function set (multiple overloads)
│   │       ├── signature.rs           # Detecting overloads that accept the same call
│   │       ├── single.rs              # Builder for registering a single-signature `DuckDB` scalar function
│   │       └── tests.rs               # Unit tests
│   ├── table/
│   │   ├── bind_data.rs               # Type-safe bind data management for table functions
│   │   ├── builder.rs                 # Builder for registering `DuckDB` table functions
│   │   ├── collision.rs               # Refusing a taken table function name, with one catalog scan per extension load
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
│   │       ├── tests_pyyaml.rs        # Accepted exactly when PyYAML accepts it, in linear time
│   │       ├── tests_yaml.rs          # Unit tests
│   │       ├── validator.rs           # Validates a `description.yml` string and returns `Ok(())` if it passes all checks
│   │       ├── yaml.rs                # A reader for the subset of YAML that `description.yml` files use
│   │       ├── yaml11.rs              # Which plain scalars PyYAML (YAML 1.1) reads as booleans, numbers, dates or null
│   │       └── yaml/
│   │           ├── block.rs           # Block mappings and sequences, and their keys
│   │           ├── flow.rs            # Flow collections, nested and spread over lines
│   │           ├── scalar.rs          # Scalar-level pieces of the `description.yml` YAML reader: decoding plain, quoted, block and flow values, and recognising keys and comments
│   │           └── value.rs           # The value after a mapping key or a sequence marker
│   ├── value/
│   │   ├── bits.rs                    # `BIT` and `BIGNUM` values: validated constructors and getters that check type and `NULL` first
│   │   ├── blob.rs                    # `Value::as_blob` — `BLOB` extraction
│   │   ├── cast.rs                    # `Value::try_cast` (exception-free, any type), `array_items` and `union_tag`
│   │   ├── checks.rs                  # Pure-Rust preconditions checked before a `Value` call reaches `DuckDB`
│   │   ├── composite.rs               # Composite constructors: `STRUCT`, `LIST`, `ARRAY`, `ENUM`, `MAP`, `UNION`
│   │   ├── defaults.rs                # The defaulting accessors — `Value::as_*_or`
│   │   ├── getters.rs                 # The typed scalar accessors — `Value::as_i64`, `as_timestamp`, `as_decimal`, …
│   │   ├── hugeint.rs                 # Conversions between Rust's 128-bit integers and `DuckDB`'s split-word `HUGEINT` / `UHUGEINT` records
│   │   ├── nested.rs                  # Reading nested values: `LIST` elements, `STRUCT` fields, `MAP` entries
│   │   ├── render_guard.rs            # Refusing to render a value `DuckDB` would throw on (an out-of-range timestamp from SQL)
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
│   ├── abi_policy_enforced.rs         # `AbiPolicy` enforced in-process against an engine whose layout differs
│   ├── aggregate_leaks.rs             # Aggregate states `DuckDB` never destroys leak no Rust heap
│   ├── append_metadata_cli.rs         # The `append_metadata` binary run end to end: exit status, output and the file it writes
│   ├── build_script_env.rs            # The build script run with a crafted `QUACK_RS_TARGET_DUCKDB_VERSION`
│   ├── entry_point_expansion.rs       # The entry-point macros compiled as edition 2024 and under `panic = "abort"`
│   ├── entry_point_lint.rs            # The entry-point macros accept an `unsafe` registration closure warning-free
│   ├── ffi_roundtrip.rs               # End-to-end FFI round-trips against a real `DuckDB`
│   ├── file_handle_close.rs           # `FileHandle`'s `Drop` when the close fails (stubbed C API)
│   ├── handle_leaks.rs                # Every RAII handle frees what `DuckDB` allocated for it (glibc)
│   ├── integration_test.rs            # Integration tests for `quack-rs`
│   ├── lsan_probe.rs                  # The `leak-check` job's canary: a handle leaked on request must fail LeakSanitizer
│   ├── property_inputs.rs             # Property tests for `parse_version`, SPDX, `description.yml` and the scaffold's escaping
│   ├── quack_scaffold_cli.rs          # The `quack_scaffold` binary run as a user runs it: exit status, output and the files it writes
│   ├── secret_memory_scan.rs          # `SecretEntry`'s zeroize, checked by scanning the process's own memory (Linux)
│   ├── secret_zeroize.rs              # `SecretEntry` never frees a buffer that still holds a secret
│   └── ffi_roundtrip/
│       ├── agg_states.rs              # Every aggregate state is dropped, including the ones `DuckDB` moves
│       ├── agg_window.rs              # Aggregates in the running-window and sorted-aggregate paths
│       ├── aggregate_arity.rs         # An aggregate with no parameter is refused at registration (DuckDB 2.0 passes `update` no row count)
│       ├── appender_api.rs            # `Appender` methods a no-op replacement survived: schemas, column types, `clear_columns`, defaults
│       ├── appender_query.rs          # `Appender::for_query` and the appender's `from_raw` / `into_raw`
│       ├── appender_rows.rs           # What happens to buffered rows when an append fails mid-row
│       ├── arrow_export.rs            # `arrow::data_chunk_to_arrow` refuses values `DuckDB` would export wrongly
│       ├── arrow_import.rs            # `arrow::data_chunk_from_arrow` checks against a live `DuckDB`
│       ├── arrow_layout.rs            # Valid Arrow layouts `DuckDB` imports wrongly, refused, and their correct neighbours
│       ├── bind_expressions.rs        # What a bind callback learns about its arguments from `Expression`
│       ├── builder_logical.rs         # `param_logical`, `varargs_logical`, `returns_logical` on the aggregate, scalar and table builders
│       ├── chunk_writer.rs            # `ChunkWriter` against a chunk `DuckDB` allocated
│       ├── chunk_writer_cols.rs       # `ChunkWriter::{vector, writer, struct_writer}` and `DataChunk::into_chunk_writer` filling a real chunk
│       ├── collision.rs               # The scalar signature-collision check, held to `DuckDB`'s own binder
│       ├── connection_registrar.rs    # Every `Registrar` method of `Connection` reaches `DuckDB`
│       ├── copy_from_columns.rs       # A typed `COPY … FROM` reader that declares a column is refused
│       ├── copy_info_errors.rs        # `set_error` from all four COPY phases, and the state passed between them (`duckdb-1-5`)
│       ├── copy_typed_state.rs        # Typed `COPY TO` state, a `FileHandle` kept across callbacks, `Send` handles (`duckdb-1-5`)
│       ├── data_chunk_helpers.rs      # `DataChunk::any_null`, `struct_writer`, `into_chunk_writer` and `ChunkWriter::is_full`
│       ├── entry_point_inprocess.rs   # `init_extension*` and the entry-point macros run in process against a fake extension access
│       ├── entry_point_txn.rs         # A failed `LOAD` rolls back and can be retried; a `LOAD` inside the host's transaction
│       ├── file_errors.rs             # `FileHandle` reports write, sync and seek failures, and refuses a seek past `i64::MAX`
│       ├── handles_api.rs             # `StructWriter` child handles and `InMemoryDb::execute`'s row count
│       ├── hostile_input.rs           # Hostile names, SQL bodies, type nesting and cardinality hints against a real engine
│       ├── inline_paths.rs            # The paths that skip a C call: `is_valid` / `set_null` bit operations and the `ListBuilder` child-writer cache, held to the C API
│       ├── lifecycle.rs               # Aggregate NULL rows, name collisions, overload builders, bind-data sharing
│       ├── lifecycle_gaps.rs          # Lifecycle paths no other test reached, found by mutation testing
│       ├── list_api.rs                # The raw `ListVector` workflow, `get_size`, `get_entry`, `child_writer`, `max_child_capacity` on live LIST and MAP vectors
│       ├── list_limits.rs             # `ListBuilder` stops at `DuckDB`'s byte ceiling, not an element count
│       ├── logical_type_accessors.rs  # Every `LogicalType` accessor, built by the constructors and read back, against the engine's own `typeof`
│       ├── map_api.rs                 # `MapVector` reserve, set_size, set_entry, key/value writers and readers on live MAP vectors
│       ├── mock_parity.rs             # `MockRegistrar` refuses a bad type with the live registration's message
│       ├── nested_reserve.rs          # which nested buffers a `LIST` reserve moves (the writer contracts)
│       ├── nested_validity.rs         # `VectorWriter::set_valid` on nested rows, against a live `DuckDB`
│       ├── oracle_temporal.rs         # Randomised cross-checks of `datetime` and `interval` against `DuckDB`'s own SQL
│       ├── overload_keys.rs           # Overloads `DuckDB` resolves by type id alone are refused; closure-built overloads; taken aggregate names
│       ├── owned_chunk.rs             # `OwnedDataChunk::new`, `capacity` and `reset` against a live `DuckDB`
│       ├── panic_guards.rs            # The panic-guard macros and `set_error` methods, against a live `DuckDB`
│       ├── query_docs.rs              # Pins the documented behaviour of `query`, `PreparedStatement`, `DbConfig`
│       ├── query_extras.rs            # `bind_str`, `bind_null`, `clear_bindings` and an idle `progress`
│       ├── query_metadata.rs          # Statement type, parameter and column metadata, `table_names`, raw-handle round trips
│       ├── query_stream.rs            # A streaming result that stops early must not look like a finished one
│       ├── replacement_scan_api.rs    # Replacement-scan `add_*_parameter`, `register_with_data` and `set_error`
│       ├── scalar_agg.rs              # Scalar and aggregate builder regressions
│       ├── scalar_bind_data_copy.rs   # `ScalarBindInfo::set_bind_data_copy` and `get_argument` (`duckdb-1-5`)
│       ├── signature_hazards.rs       # Signatures that crash or invalidate DuckDB: `VARIANT`, refused at the constructor
│       ├── string_view.rs             # `DuckStringView`, `read_duck_string` and `read_duck_blob` against `duckdb_string_t` records the engine wrote
│       ├── struct_io.rs               # Every `StructReader` / `StructWriter` field type, round-tripped through a 20-field STRUCT
│       ├── table_cast.rs              # Table function, cast, replacement scan, SQL macro and COPY regressions
│       ├── table_description.rs       # `TableDescription` accessors on an index `DuckDB` cannot hold
│       ├── table_info_api.rs          # `BindInfo` parameter and cardinality methods, `FfiInitData`, `FfiLocalInitData`, `extra_info`
│       ├── temporal_binds.rs          # Temporal and over-4-GiB values refused by `PreparedStatement` binds and the `Appender`
│       ├── timestamp_tz_ns.rs         # `TIMESTAMPTZ_NS` (DuckDB 2.0): vectors, `Value`, and every type the engine can produce
│       ├── tooling.rs                 # Checks of quack-rs's tooling tables against the linked `DuckDB`
│       ├── typed_extra_info.rs        # `extra_info_typed` / `extra_info_ref` on casts and aggregates; a wrong type reads as `None`
│       ├── typed_scalar_opt.rs        # Typed scalar closures returning `Option`, the `try_*` constructors and `map3`
│       ├── validity_api.rs            # `ValidityBitmap` against the engine's masks, at the word and vector edges
│       ├── value_bits.rs              # `BIT` / `BIGNUM` values; every input the C getters abort on reads as `None`
│       ├── value_cast.rs              # `try_cast`, `array_items`, `union_tag`: reading `ARRAY` and `UNION` values
│       ├── value_defaults.rs          # `Value::as_*_or` when a value is present
│       ├── value_getters.rs           # Each typed `Value` getter at its own type; out-of-range TIMETZ / TIME_NS are not cast
│       ├── value_nested.rs            # Nested `Value` construction and inspection against a live `DuckDB`
│       ├── value_query.rs             # `Value` getters, DECIMAL binding, `Expression::fold`
│       ├── value_render.rs            # `Value` rendering of values SQL builds and `DuckDB` cannot render
│       ├── value_temporal.rs          # Every `Value` getter against every temporal source type, at every edge
│       ├── vector_capacity_guard.rs   # `OwnedVector::new` refuses a capacity whose nested `ARRAY` child would overflow
│       ├── vector_dt.rs               # NULLs in nested output vectors; selection vectors
│       ├── vector_ops.rs              # `OwnedVector::into_raw`, `copy_selected`, `slice`, `reference_value`, `reference_vector` (`duckdb-1-5`)
│       ├── vector_support.rs          # Helpers shared by the vector-module tests
│       ├── wide_rows.rs               # Every fixed-width and string type echoed over thousands of distinct rows
│       └── writer_api.rs              # `write_timestamp_tz`, `write_str`, and the Ok path of `try_write_varchar` / `try_write_blob`
├── benches/
│   ├── interval_bench.rs          # Criterion benchmarks for interval conversion
│   └── engine/                    # Benchmark extension and A/B driver: SDK functions beside hand-written twins, timed in a real DuckDB
│       ├── Cargo.toml
│       ├── README.md
│       ├── compare.py
│       └── src/lib.rs
├── examples/
│   └── hello-ext/                 # Complete word_count aggregate extension example
│       ├── Cargo.toml
│       └── src/lib.rs
├── book/                          # mdBook documentation source
├── .github/workflows/
│   ├── ci.yml                     # CI: every quality gate (one job each; see workflows/README.md)
│   ├── release.yml                # Release pipeline: CI gate, package, publish
│   ├── docs.yml                   # Versioned book build & deploy to GitHub Pages (scripts/build-versioned-site.py)
│   ├── coverage.yml               # Test coverage (cargo-llvm-cov → Codecov)
│   ├── mutants.yml                # Mutation testing (cargo-mutants)
│   ├── benchmarks.yml             # Criterion benchmark execution
│   └── README.md                  # Workflow overview and quality gate summary
├── CONTRIBUTING.md                # This file
├── LESSONS.md                     # The DuckDB Rust FFI pitfalls (L1–L21, P1–P17), documented in full
└── README.md                      # Quick start, SDK overview, badge table
```

---

## PR Checklist

- [ ] SPDX header on every new file
- [ ] New or split files follow the 500-line guideline (exceed it only where splitting would harm cohesion)
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

This crate supports `libduckdb-sys = ">=1.4.4, <1.20100"` (the bindings of DuckDB 1.4.4
through 2.0.x). Every DuckDB from 1.4.4 through 2.0.0 loads C API `v1.2.0` extensions.
`libduckdb-sys` encodes DuckDB `vMAJOR.MINOR.PATCH` as `1.(10000·MAJOR + 100·MINOR + PATCH).N`,
so DuckDB 2.0.0 is 1.20000.0 and DuckDB 2.1.0 will be 1.20100.0; the bound admits the first
and stops before the second. The 1.20000.x releases (1.20000.0 on 2026-10-22, 1.20000.1 on
2026-10-28) declare `rust-version` 1.88, so a Rust 1.86 toolchain cannot build them;
MSRV-aware resolution (resolver 3, or `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`)
keeps a 1.86 build on 1.10506.x.

**Why there is an upper bound.** 1.20000.x keeps the root-level names quack-rs and its
exported macros use (`libduckdb_sys::duckdb_connection`, ...) behind the default feature
`capi-v1`, beside a `// TODO: Remove in the future`. With only that re-export deleted from a
copy of 1.20000.1, quack-rs failed with 214 compile errors (the eighth review reproduced
it). Without an upper bound, the day a later binding drops it, every new project resolves to
bindings this crate cannot build against. Published releases cannot be narrowed after the
fact, which is why the bound is on the newest release and is widened deliberately.

**Widening the bound for a new DuckDB minor** (a patch release of quack-rs):

1. Read the DuckDB release notes and the C API diff (`duckdb.h`, `duckdb_extension.h`):
   `scripts/check-abi-table.py` and `scripts/check-api-coverage.py` report new slots.
2. Resolve the new bindings (`cargo update -p duckdb --precise <version>` and
   `-p libduckdb-sys --precise <version>`) in a copy and run the whole suite against the new
   engine, as the `test-duckdb-2-0` job does for 2.0. Check specifically that the root-level
   re-exports still exist (`cargo check` is enough to tell).
3. Add the release to `KNOWN_LAYOUTS` / `PREFIX_LAYOUTS` in `src/abi.rs` if its slot count
   differs (the abi-table job derives them), and to the `extension-load` and older-engine
   matrices.
4. Update the `libduckdb-sys` and `duckdb` requirements in `Cargo.toml` and in every
   manifest and document that repeats them (`git grep '<1.20100'`; a test compares them).
5. Update `DUCKDB_API_VERSION` in `src/lib.rs` if the C API version changed.

Versions follow [Semantic Versioning](https://semver.org/) as Cargo applies it.
While the crate is pre-1.0, a breaking change to the public API bumps the
**minor** version (`0.17.x` → `0.18.0`) and is marked **Breaking:** in
`CHANGELOG.md`; see the semantic versioning policy in
[`RELEASING.md`](RELEASING.md).
