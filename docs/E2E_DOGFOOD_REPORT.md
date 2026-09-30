# quack-rs E2E Dogfood Report

**Date:** 2026-03-21
**Crate version:** 0.6.0
**Rust toolchain:** 1.93.1 (stable)
**Platform:** Linux x86_64

> **Note (2026-09-30, before 0.18.0):** This is a historical report of the 0.6.0
> codebase and is kept as written. Its CI findings no longer describe `ci.yml`:
> the `test-duckdb-1-5` job tests the `duckdb-1-5`, `duckdb-1-5-3` and
> `duckdb-1-5-4` features on every PR (ISSUE-2), `test-bundled` also runs
> `cargo test --all-targets --all-features` on Linux, macOS and Windows,
> `test-bundled-prebuilt` tests against a prebuilt `libduckdb`, and the
> `extension-load` job loads a built extension into live DuckDB CLIs.

---

## Executive Summary

Comprehensive end-to-end testing of every build configuration, all tests, linting, documentation, CI, packaging, and deep source code review. The codebase is **production-grade** with excellent safety practices. All 105 tests pass, rustdoc builds clean, and formatting is correct. Below are all findings organized by severity.

---

## 1. Build Matrix Results

| Configuration | Result |
|---|---|
| Default features (`cargo check`) | PASS |
| `duckdb-1-5` only | PASS |
| `bundled-test` only | PASS |
| `--all-features` | PASS |
| `--no-default-features` | PASS |
| Release profile (`cargo build --release`) | PASS |
| `examples/hello-ext` | PASS |
| `cargo package --allow-dirty` | PASS (83 files, 723.8 KiB) |

**Note:** `cargo check --features bundled-test` fails because `check` skips full dep compilation — the C++ shim in `build.rs` needs `libduckdb-sys` header files that only exist after a full `cargo build`. This is a known limitation of `cargo check` with build scripts that depend on other crates' build artifacts. Not a bug — but worth documenting.

---

## 2. Test Results

| Suite | Result |
|---|---|
| Unit tests (`--all-features`) | **105 passed, 0 failed** |
| Integration tests | PASS (included in above) |
| Doc tests | PASS (all compile and run) |
| Scaffold compile test | PASS |
| Benchmark compilation | PASS |

---

## 3. Linting & Formatting

| Check | Result |
|---|---|
| `cargo fmt -- --check` | PASS |
| `RUSTDOCFLAGS="-D warnings" cargo doc` | PASS (0 warnings) |
| `cargo clippy --all-features` | 43 warnings (see below) |

### Clippy Warnings Breakdown

| Category | Count | Severity | Action |
|---|---|---|---|
| `must_use_candidate` | 35 | Low | Already allowed in `Cargo.toml` — only appears with explicit `-W` override |
| `missing_errors_doc` | 8 | Low | On `Registrar` trait methods — these are `unsafe` FFI wrappers where doc burden is already met by Safety section |

**Assessment:** All 43 warnings are on lint categories already configured as `allow` in `Cargo.toml`. They only appear when running clippy with explicit `-W clippy::pedantic` override beyond the configured level.

---

## 4. Issues Fixed in This Report

### FIX-1: Scaffold template generates stale version `"0.5"` (CRITICAL)

**Files changed:**
- `src/scaffold/templates.rs:34` — `quack-rs = "0.5"` → `"0.6"`
- `src/testing/in_memory_db.rs:37` — doc example version `"0.5"` → `"0.6"`
- `tests/integration_test.rs` — scaffold compile test version reference

**Impact:** Any user running `generate_scaffold()` would get a `Cargo.toml` that pins to the previous version, missing all 0.6.0 features including DuckDB 1.5.0 support.

### FIX-2: Book documentation references stale version `"0.5"`

**Files changed:**
- `book/src/getting-started/quick-start.md:20` — `"0.5"` → `"0.6"`
- `book/src/getting-started/installation.md:9` — `"0.5"` → `"0.6"`
- `book/src/publishing.md:204` — `"0.5"` → `"0.6"`

### FIX-3: Book pitfall count outdated (15 → 16)

`LESSONS.md` documents 16 pitfalls (L1-L7, P1-P9). README correctly says 16, but the book still says 15.

**Files changed:**
- `book/src/introduction.md:86` — "15 pitfalls" → "16 pitfalls"
- `book/src/faq.md:21` — "15 known pitfalls" → "16 known pitfalls"
- `book/src/faq.md:41` — "15 pitfalls" → "16 pitfalls"

### FIX-4: `ConfigOptionBuilder::description()` and `default_value()` silently swallow errors

**File:** `src/config_option.rs:97-111`

Previously, these methods used `.ok()` to silently discard CString conversion errors when the input contained null bytes. The user would get no feedback that their configuration was silently dropped.

Changed both methods from `fn(self, &str) -> Self` to `fn(self, &str) -> Result<Self, ExtensionError>`, with clear error messages. Updated doc example to use `?`.

---

## 5. Remaining Issues (Not Fixed — Require Design Decision)

### ISSUE-1: `LogicalType` constructors panic on null pointer (MEDIUM)

**Location:** `src/types/logical_type.rs:59-107`

`LogicalType::new()`, `list()`, `map()`, `struct_type()` all assert/panic if the underlying DuckDB C API returns null. While extremely rare (only on allocation failure), this violates the "no panic across FFI" principle.

**Recommendation:** Add `try_new()`, `try_list()`, etc. returning `Result`. Keep current `new()` as convenience that panics.

### ISSUE-2: CI doesn't test `duckdb-1-5` feature in PR pipeline (HIGH)

**Location:** `.github/workflows/ci.yml`

The main CI pipeline only tests default features and `bundled-test`. The `duckdb-1-5` feature is only tested during release (`release.yml` with `--all-features`). A breaking change to `duckdb-1-5` code could merge without detection.

**Recommendation:** Add `cargo test --all-features` job to `ci.yml`.

### ISSUE-3: `dtolnay/rust-toolchain` not pinned to SHA (MEDIUM)

**Location:** All 21 instances across `ci.yml`, `release.yml`, `coverage.yml`, `mutants.yml`, `benchmarks.yml`, `docs.yml`

The project's own `.github/workflows/README.md` states: "Pin all third-party actions to their full commit SHA." But `dtolnay/rust-toolchain` is referenced by tag (`@stable`, `@nightly`, `@1.84.1`, `@master`). Especially concerning: `release.yml:156` uses `@master`.

**Recommendation:** Pin all to commit SHA, or document an explicit exception for this action.

### ISSUE-4: No live DuckDB extension load test in CI (HIGH)

**Location:** `CONTRIBUTING.md:119-135` describes Quality Gate 7 (live extension test), but CI doesn't implement it.

The example extension is compiled and symbol-checked, but never actually loaded into a DuckDB instance. A registration signature change could break compatibility silently.

**Recommendation:** Add a CI job that builds the extension, runs `append_metadata`, and loads it in DuckDB CLI with `-unsigned`.

### ISSUE-5: Coverage and doc jobs don't use `--all-features` (MEDIUM)

**Location:** `coverage.yml:44-45`, `ci.yml:88-97`

Code behind `duckdb-1-5` is excluded from coverage reports and doc generation in PRs.

### ISSUE-6: `cargo install cargo-mutants` not pinned with `--locked` (LOW)

**Location:** `mutants.yml:37, 141`

Inconsistent with `coverage.yml:42` which correctly uses `--locked`.

### ISSUE-7: `RELEASING.md` checklist incomplete (LOW)

**Location:** `RELEASING.md:59-73`

The release checklist lists 7 CI jobs but omits 7 others (`test-bundled`, `bench-compile`, `example-check`, `scaffold-compile`, `symbol-check`, `publish-dry-run`, `nightly`).

### ISSUE-8: `cargo package` warns about excluded benchmark (LOW)

```
warning: ignoring benchmark `interval_bench` as `benches/interval_bench.rs` is not included in the published package
```

The `benches/` directory is in `Cargo.toml`'s `exclude` list. Cargo warns because `[[bench]]` is auto-detected from the file but then excluded. Harmless but noisy.

---

## 6. Code Quality Assessment

### Unsafe Code: EXCELLENT
- 249 `unsafe` blocks, 100% have `// SAFETY:` comments
- All invariants correctly documented
- No soundness issues identified

### Error Handling: EXCELLENT
- Consistent `ExtensionError` / `ExtResult<T>` throughout
- Proper `?` propagation
- Defensive fallback chains in `error.rs`

### API Design: EXCELLENT
- Uniform builder patterns across all function types
- `Registrar` trait enables testability via `MockRegistrar`
- Clean feature gating for `duckdb-1-5`

### Test Coverage: VERY GOOD
- 105 tests covering all major code paths
- Property-based testing with `proptest` for intervals
- Integration test validates scaffold compilation
- Mock infrastructure (`MockVectorReader`, `MockVectorWriter`, `MockRegistrar`)
- Gap: DuckDB 1.5.0+ modules (`catalog`, `client_context`, `copy_function`, `table_description`) lack unit tests (requires live DuckDB)

### Documentation: VERY GOOD
- `#![warn(missing_docs)]` enforced
- All public items documented
- Comprehensive book with 30+ pages
- Minor staleness issues (fixed above)

---

## 7. Architecture Observations

1. **Clean layered design:** FFI → Safe wrappers → Builders → User API
2. **Feature gating is correct:** No cross-feature leaks between default and `duckdb-1-5`
3. **Scaffold is complete:** Generates 11 files covering Cargo.toml, Makefile, CI, tests, WASM shim
4. **Validation is thorough:** Extension names, SPDX licenses, semver, platforms, description.yml
5. **The `Registrar` trait is well-designed:** Enables both `Connection` (real) and `MockRegistrar` (test) implementations

---

## 8. Recommendations for Next Release

| Priority | Item | Effort |
|---|---|---|
| HIGH | Add `--all-features` test job to CI | Small |
| HIGH | Add live DuckDB extension load test to CI | Medium |
| MEDIUM | Pin `dtolnay/rust-toolchain` to SHA or document exception | Small |
| MEDIUM | Add `try_new()` to `LogicalType` | Small |
| MEDIUM | Run coverage + docs with `--all-features` | Small |
| LOW | Pin `cargo-mutants` install with `--locked --version` | Trivial |
| LOW | Update `RELEASING.md` checklist to match all CI jobs | Trivial |
| LOW | Consider making `Cargo.toml` `exclude` for benches consistent | Trivial |
