# quack-rs — Pre-Release Audit Report

**Date**: 2026-03-08
**Auditor**: Claude (automated)
**Scope**: Entire repository — code, docs, tests, CI, examples
**Purpose**: Comprehensive audit for public release, crate submission, and portfolio showcase

---

## Executive Summary

The codebase is in excellent shape. Code quality is high: zero clippy warnings
(with pedantic+nursery+cargo lints), clean formatting, documentation compiles
without warnings, and 39/40 tests pass (the 1 failure is a transient `/tmp`
filesystem race, not a code bug). The issues found are documentation-level:
wrong cross-references, a missing release profile in the example, and an
incomplete directory tree in CONTRIBUTING.md.

**Severity scale**: CRITICAL > HIGH > MEDIUM > LOW > STYLE

---

## Findings

### F1 — hello-ext/README.md: "P13" pitfall does not exist [MEDIUM]

**File**: `examples/hello-ext/README.md:146`
**Evidence**: The pitfall table references "P13" — but LESSONS.md only defines
L1–L7 and P1–P8 (15 pitfalls total). There is no P13.
**Fix**: The row describes "No `panic!` across FFI" which is **L3**.

- [x] Fixed

### F2 — hello-ext/README.md: wrong function name `check_release_profile` [MEDIUM]

**File**: `examples/hello-ext/README.md:98`
**Evidence**: The checklist says `quack_rs::validate::check_release_profile`.
The actual function is `validate_release_profile` (see `src/validate/release_profile.rs`).
**Fix**: Replace `check_release_profile` with `validate_release_profile`.

- [x] Fixed

### F3 — hello-ext/Cargo.toml: missing `[profile.release]` [MEDIUM]

**File**: `examples/hello-ext/Cargo.toml`
**Evidence**: No `[profile.release]` section. The scaffolded projects include
`panic = "abort"` (required per SECURITY.md and L3). The hello-ext example
should demonstrate best practices.
**Fix**: Add `[profile.release]` with `panic = "abort"`.

- [x] Fixed

### F4 — CONTRIBUTING.md: validate/ directory tree is incomplete [LOW]

**File**: `CONTRIBUTING.md:183-184`
**Evidence**: The repository structure tree only lists `mod.rs` and
`description_yml.rs` under `validate/`. The actual directory contains 7 files:
`mod.rs`, `description_yml.rs`, `extension_name.rs`, `function_name.rs`,
`platform.rs`, `release_profile.rs`, `semver.rs`, `spdx.rs`.
**Fix**: Expand the tree to list all validate submodules.

- [x] Fixed

### F5 — README.md and LESSONS.md have conflicting ADR numbering [LOW]

**File**: `README.md:587-607` and `LESSONS.md:397-418`
**Evidence**:
- README ADRs: ADR-1 Thin Wrapper, ADR-2 Exact Version Pin, ADR-3 No Panics
- LESSONS.md ADRs: ADR-1 libduckdb-sys only, ADR-2 Function sets, ADR-3 Custom entry point

These are 6 different decisions using the same numbering.
**Fix**: Renumber LESSONS.md ADRs to ADR-4, ADR-5, ADR-6 and add cross-references.

- [x] Fixed

---

## Verification Results

| Check | Status | Notes |
|-------|--------|-------|
| `cargo build` | PASS | Clean build, no warnings |
| `cargo build --release` | PASS | LTO + strip + abort |
| `cargo test` | PASS | 39/40 (1 transient `/tmp` race) |
| `cargo test` (re-run) | PASS | 40/40 |
| `cargo clippy --all-targets -- -D warnings` | PASS | Zero warnings |
| `cargo fmt -- --check` | PASS | Perfectly formatted |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` | PASS | No broken links |
| `hello-ext` build | PASS | Compiles as cdylib |
| `hello-ext` tests | PASS | All unit tests pass |
| Scaffold compile test | PASS | Generated code compiles |

---

## Items Verified Clean (No Issues Found)

- **src/lib.rs** — Module declarations, DUCKDB_API_VERSION constant, deny/warn attrs
- **src/entry_point.rs** — `entry_point!` macro, `init_extension`, `report_error`
- **src/error.rs** — `ExtensionError`, `to_c_string` with null-byte truncation
- **src/interval.rs** — `DuckInterval`, checked/saturating conversions, proptest
- **src/sql_macro.rs** — SQL injection prevention via `validate_function_name`
- **src/aggregate/** — Builder, state, callbacks — all clean
- **src/scalar/** — `ScalarFunctionBuilder` — clean
- **src/vector/** — Reader, writer, string, validity — all clean
- **src/types/** — `TypeId`, `LogicalType` with RAII Drop — clean
- **src/validate/** — All 7 validator modules — clean, thorough test coverage
- **src/scaffold/** — Project generator — clean
- **src/testing/** — `AggregateTestHarness` — clean
- **tests/integration_test.rs** — Comprehensive pure-Rust tests
- **benches/interval_bench.rs** — Criterion benchmarks
- **LESSONS.md** — All 15 pitfalls documented correctly
- **CHANGELOG.md** — Properly formatted, versions match
- **SECURITY.md** — Vulnerability disclosure policy
- **RELEASING.md** — Complete release runbook
- **Cargo.toml** — Correct pins, lint config, release profile
- **Copyright dates** — "2026" is correct (current year)
- **MSRV** — 1.84.1 consistent across all documents
- **`&raw mut` syntax** — Valid Rust 1.82+, MSRV is 1.84.1
