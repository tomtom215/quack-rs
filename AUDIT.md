# quack-rs — Pre-Release Audit Report

**Date**: 2026-03-08
**Auditor**: Claude (automated)
**Scope**: Entire repository — code, docs, tests, CI, examples
**Purpose**: Comprehensive audit for public release, crate submission, and portfolio showcase

---

## Executive Summary

The codebase is in excellent shape. Code quality is high: zero clippy warnings
(with pedantic+nursery+cargo lints), clean formatting, documentation compiles
without warnings, and all 449 tests pass (303 unit + 20 binary + 40 integration
+ 86 doc-tests). Round 1 found 6 documentation issues, all fixed. Round 2
found 6 additional issues (1 API safety improvement, 5 documentation), all fixed.

**Severity scale**: CRITICAL > HIGH > MEDIUM > LOW > STYLE

---

## Findings — Round 1 (all fixed)

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

### F6 — book/src/reference/changelog.md out of sync with CHANGELOG.md [HIGH]

**File**: `book/src/reference/changelog.md`
**Evidence**: The book changelog had no `[0.2.0]` section. All v0.2.0 features
were still listed under `[Unreleased]`. The comparison link pointed to
`v0.1.0...HEAD` instead of `v0.2.0...HEAD`. The v0.1.0 section was also missing
several items present in the actual CHANGELOG.md (scaffold, sql_macro, validate).
**Fix**: Rewrote to mirror the actual CHANGELOG.md exactly.

- [x] Fixed

---

## Findings — Round 2 (all fixed)

### F7 — SECURITY.md: version table missing 0.3.x [MEDIUM]

**File**: `SECURITY.md:7`
**Evidence**: The supported versions table lists `0.2.x` as "Yes" but the
project is at version `0.3.0`. Version `0.3.x` should be the supported version,
and `0.2.x` should be end-of-life.
**Fix**: Updated table to list `0.3.x | Yes`, `0.2.x | No (end-of-life)`,
`0.1.x | No (end-of-life)`.

- [x] Fixed

### F8 — hello-ext/README.md code tour: stale struct names [MEDIUM]

**File**: `examples/hello-ext/README.md:183-186`
**Evidence**: The "Code tour" section references:
- `GsBindData` — does not exist; bind data is stored as `FfiBindData::<i64>`
- `GsScanState` — does not exist; the actual struct is `GenerateSeriesState`
- `gs_init — zero-initialises scan state via FfiInitData::init_callback` — the actual
  code calls `FfiInitData::<GenerateSeriesState>::set(info, ...)`, not `init_callback`
**Fix**: Updated code tour to use correct struct names and method names.

- [x] Fixed

### F9 — book first-extension.md: says "two functions" but example has four [MEDIUM]

**File**: `book/src/getting-started/first-extension.md:4`
**Evidence**: The page says hello-ext registers "**two functions**" and only lists
`word_count` and `first_word`. The actual hello-ext example now registers four
functions: `word_count`, `first_word`, `generate_series_ext`, and `CAST(VARCHAR AS INTEGER)`.
**Fix**: Updated to "**four functions**" with complete table.

- [x] Fixed

### F10 — config.rs: `DbConfig::set()` uses `.expect()` which can panic [LOW]

**File**: `src/config.rs:88-89`
**Evidence**: `CString::new(name).expect(...)` and `CString::new(value).expect(...)`
will panic if the caller passes a string containing an interior null byte. While
unlikely, this is a library API that could be called from FFI context where panics
are undefined behavior. All other SDK APIs return `Result` for error conditions.
**Fix**: Replaced `.expect()` with `.map_err()` returning `ExtensionError`.

- [x] Fixed

### F11 — DuckStringView::from_bytes: misleading "Panics" doc [STYLE]

**File**: `src/vector/string.rs:59-61`
**Evidence**: The doc comment says "# Panics — Panics if `raw` is not exactly
16 bytes" but the function signature is `fn from_bytes(raw: &'a [u8; DUCK_STRING_SIZE])`
which enforces the size at compile time via the fixed-size array reference.
**Fix**: Replaced panics section with accurate note about compile-time enforcement.

- [x] Fixed

### F12 — Cargo.toml vs lib.rs: inconsistent unsafe_op_in_unsafe_fn lint level [STYLE]

**File**: `Cargo.toml:37` and `src/lib.rs:98`
**Evidence**: `Cargo.toml` sets `unsafe_op_in_unsafe_fn = "warn"` but `src/lib.rs`
has `#![deny(unsafe_op_in_unsafe_fn)]`. The `deny` in lib.rs overrides the `warn`
in Cargo.toml, making the Cargo.toml setting misleading. SECURITY.md also
references the `warn` level, which could confuse contributors.
**Fix**: Changed Cargo.toml to `"deny"` to match lib.rs.

- [x] Fixed

---

## Verification Results

| Check | Status | Notes |
|-------|--------|-------|
| `cargo build` | PASS | Clean build, no warnings |
| `cargo build --release` | PASS | LTO + strip + abort |
| `cargo test` | PASS | 449/449 (303 unit + 20 binary + 40 integration + 86 doc) |
| `cargo clippy --all-targets -- -D warnings` | PASS | Zero warnings |
| `cargo fmt -- --check` | PASS | Perfectly formatted |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` | PASS | No broken links |
| `hello-ext` build | PASS | Compiles as cdylib |
| `hello-ext` tests | PASS | All 21 unit tests pass |
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
- **src/cast/** — `CastFunctionBuilder`, `CastFunctionInfo`, `CastMode` — clean
- **src/table/** — `TableFunctionBuilder`, `BindInfo`, `FfiBindData`, `FfiInitData` — clean
- **src/replacement_scan/** — `ReplacementScanBuilder` — clean
- **src/vector/** — Reader, writer, string, validity, complex — all clean
- **src/types/** — `TypeId`, `LogicalType` with RAII Drop — clean
- **src/validate/** — All 7 validator modules — clean, thorough test coverage
- **src/scaffold/** — Project generator — clean
- **src/testing/** — `AggregateTestHarness` — clean
- **src/prelude.rs** — Correct re-exports, documented inclusions and exclusions
- **tests/integration_test.rs** — Comprehensive pure-Rust tests
- **benches/interval_bench.rs** — Criterion benchmarks
- **examples/hello-ext/src/lib.rs** — All callbacks correct, NULL handling verified
- **LESSONS.md** — All 15 pitfalls documented correctly
- **CHANGELOG.md** — Properly formatted, versions match
- **RELEASING.md** — Complete release runbook
- **CONTRIBUTING.md** — Accurate prerequisites and workflow
- **Copyright dates** — "2026" is correct (current year)
- **MSRV** — 1.84.1 consistent across all documents
- **`&raw mut` syntax** — Valid Rust 1.82+, MSRV is 1.84.1
- **CI pipelines** — 11-job CI (16 matrix instances across Linux/macOS/Windows), 7-job release, 2-job docs — all correctly configured
- **deny.toml** — Security audits, license policy, source restrictions
- **dependabot.yml** — Weekly updates for both cargo and GitHub Actions
- **Cargo.lock** — Committed (correct for library with binary)
