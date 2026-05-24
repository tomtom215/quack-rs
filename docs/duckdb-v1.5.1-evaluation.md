# DuckDB v1.5.1 Compatibility Evaluation for quack-rs

> **⚠️ Historical / superseded (as of 2026-05).** This is a point-in-time
> evaluation written for DuckDB v1.5.1 and quack-rs 0.7.0; it is retained as a
> decision record. Since then the project has moved to **DuckDB 1.5.3**
> (`libduckdb-sys` 1.10503.1) and wrapped a large part of the 1.5.x C API
> (see the [CHANGELOG](../CHANGELOG.md)). Two specifics in this document are now
> out of date: `DUCKDB_TYPE_VARIANT` **does** now exist in the C type enum (added
> in DuckDB 1.5.3, value 41), and the `TypeId` gaps it calls out (`Any`,
> `SqlNull`, `Varint`) have since been filled. The C extension function-pointer
> API version remains `v1.2.0`. For the current state, see the
> [Known Limitations](../book/src/reference/known-limitations.md) reference.

**Date:** 2026-03-27
**DuckDB Release:** v1.5.1 (2026-03-23, commit 7dbb2e6, codename "variegata")
**quack-rs Version:** 0.7.0 (2026-03-22)
**Current libduckdb-sys:** 1.10500.0 (DuckDB 1.5.0)

## Executive Summary

DuckDB v1.5.1 is a **bugfix/patch release** on the v1.5 line. The C Extension API
version remains **v1.2.0** (unchanged from v1.4.x and v1.5.0). quack-rs is
**broadly compatible** with v1.5.1 with no breaking changes required. However,
there are several areas where updates would improve compatibility, correctness,
and take advantage of upstream improvements.

### Compatibility Verdict

| Area | Status | Action Required |
|------|--------|-----------------|
| C API version | Compatible | None - remains `v1.2.0` |
| Core FFI bindings | Compatible | None |
| Storage version | Informational | Update docs only |
| TypeId enum | Gap identified | Add `Any`, `SqlNull`, `Varint` when exposed |
| VARIANT type (Iceberg v3) | Not yet in C API | Monitor - not exposed in libduckdb-sys 1.10500.0 |
| New config options | Enhancement | Document `force_download_threshold`, `allowed_configs` |
| Arrow/ADBC fixes | Transparent benefit | Users benefit by upgrading DuckDB runtime |
| Parquet improvements | Transparent benefit | No wrapper changes needed |
| Extension bumps | Transparent benefit | No wrapper changes needed |

---

## Detailed Analysis

### 1. C API & FFI Layer

**Status: No changes required**

The DuckDB C Extension API version remains `v1.2.0`. The `DUCKDB_API_VERSION`
constant in `src/lib.rs:149` is correct. No new C API functions were added in
v1.5.1 that would require new FFI bindings or wrapper functions.

Key upstream fixes that benefit quack-rs users transparently (no code changes):

| Fix | PR | Impact |
|-----|----|--------|
| Arrow dictionary buffer overread with NULLs | #21083 | Crash fix - affects Arrow result conversion |
| ADBC concurrent statements on same connection | #21415 | Correctness fix for concurrent workloads |
| `TryGetCurrentSetting` infinite recursion | #21356 | Fixes hang when querying settings |
| `FileOpenerInfo` parameter passing | #21301 | Correctness fix for file opener callbacks |
| `MbedTLS` exception throwing | #21365 | TLS errors now properly surface |

**Recommendation:** No code changes. Users benefit automatically by upgrading
their DuckDB runtime to v1.5.1.

---

### 2. Storage Format

**Status: Documentation update only**

- Storage version bumped from v1.5.0 to **v1.5.1**
- WAL corruption fix (reverted #21067, now uses `MarkBlockAsCheckpointed`)
- ART index fixes:
  - Memory error transforming to v1.0.0 ART storage (#21270)
  - Stale update read during index removal (#21427)
  - INSERT OR REPLACE on non-unique indexed columns (#20962)
- `TrimFreeBlocks` fix: prevents zeroing concurrently allocated memory (#21146)
- Lazy `mmap` in `BlockAllocator` (#21276)

**Recommendation:** Update `DUCKDB_API_VERSION` doc comment in `src/lib.rs` to
mention v1.5.1 compatibility. The blog post explicitly recommends updating to
v1.5.1 if using indexes/constraints.

---

### 3. Type System (`TypeId` Enum)

**Status: Gaps identified - future action needed**

#### Currently mapped types (34 variants)
All 34 types in the quack-rs `TypeId` enum correctly map to their
`libduckdb-sys` constants. No regressions.

#### Types in libduckdb-sys NOT yet mapped

| Type Constant | Value | Should Map? | Notes |
|---------------|-------|-------------|-------|
| `DUCKDB_TYPE_INVALID` | 0 | No | Sentinel value, not a real column type |
| `DUCKDB_TYPE_ANY` | 34 | Consider | Used internally for type resolution |
| `DUCKDB_TYPE_BIGNUM` | 35 | Consider | Big number type |
| `DUCKDB_TYPE_SQLNULL` | 36 | Consider | SQL NULL type |
| `DUCKDB_TYPE_STRING_LITERAL` | 37 | No | Internal parser type |
| `DUCKDB_TYPE_INTEGER_LITERAL` | 38 | No | Internal parser type |

#### VARIANT type (Iceberg v3)

The DuckDB v1.5.1 blog post mentions VARIANT as a new type for Iceberg v3
support. However, **`DUCKDB_TYPE_VARIANT` does NOT exist in libduckdb-sys
1.10500.0** (DuckDB 1.5.0 bindings). This type is likely:

1. Only exposed through the Iceberg extension's internal handling, OR
2. Will be added to the C API in a future release (v1.5.2 or v1.6.0)

When writing unsupported variant types to Parquet, DuckDB v1.5.1 now converts
them to INT64 (#21357) rather than failing.

#### TIMESTAMP_NS type

Already fully supported as `TypeId::TimestampNs` (mapped to
`DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP_NS`). No changes needed. Note: this is
distinct from `TypeId::TimeNs` (time of day with nanosecond precision, gated
behind `duckdb-1-5`).

**Recommendation:**
- Consider adding `TypeId::Any`, `TypeId::SqlNull`, and `TypeId::Varint` variants
  behind `#[cfg(feature = "duckdb-1-5")]` for completeness. These are edge cases
  but would make the enum exhaustive over all non-internal types.
- Monitor `DUCKDB_TYPE_VARIANT` in future libduckdb-sys releases.
- The `#[non_exhaustive]` attribute on `TypeId` already protects downstream
  users from breakage when new variants are added.

---

### 4. New Configuration Options

**Status: Enhancement opportunity**

DuckDB v1.5.1 introduces these new settings:

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `force_download_threshold` | Integer | 0 | Forces full file download for files under N bytes (httpfs) |
| `allowed_configs` | String | — | Allow-list for configs when `lock_configurations` is set |

Additionally, `AutoloadKnownExtensions` is now checked before loading libraries
in `TryAutoLoadExtension` (#21051).

**Recommendation:**
- These are standard SQL `SET` commands, not C API additions. Extensions can
  already read them via `current_setting()` or the `ConfigOptionBuilder` (v1.5+).
- Document these new options in the quack-rs book or README for extension
  authors who need to interact with httpfs or locked configurations.
- The `allowed_configs` + `lock_configurations` combination is valuable for
  sandboxed/embedded extension scenarios.

---

### 5. libduckdb-sys Version Update

**Status: Action required**

The current `Cargo.lock` resolves `libduckdb-sys = 1.10500.0` (DuckDB 1.5.0).
To fully target v1.5.1:

1. **Wait for libduckdb-sys 1.10501.0** to be published to crates.io
   (tracks DuckDB v1.5.1 release)
2. The existing version range `">=1.4.4, <2"` in `Cargo.toml` will
   automatically accept v1.10501.0 — no Cargo.toml change needed
3. Run `cargo update -p libduckdb-sys` once the new version is available
4. Run `cargo update -p duckdb` for the bundled test dependency

**Recommendation:** Once `libduckdb-sys 1.10501.0` is published:
```bash
cargo update -p libduckdb-sys -p duckdb
cargo test --all-targets
cargo test --all-targets --features duckdb-1-5
cargo test --all-targets --features bundled-test
```

---

### 6. Bug Fixes That Benefit Extension Authors

These upstream fixes are transparent to quack-rs but important for users:

#### Critical (Recommend upgrading DuckDB runtime)

| Category | Fix | PR |
|----------|-----|----|
| **WAL corruption** | Fix corruption through `MarkBlockAsCheckpointed` on WAL blocks | #21285 |
| **ART indexes** | Two correctness fixes for indexes/constraints | #21270, #21427 |
| **INSERT OR REPLACE** | Fix updates on non-unique indexed columns | #20962 |
| **Arrow** | Buffer overread crash in dictionary conversion with NULLs | #21083 |

#### Important

| Category | Fix | PR |
|----------|-----|----|
| **JSON** | Invalid JSON when casting from certain types | #21280 |
| **Parquet** | Row Group Reorderer bug | #21282 |
| **Parquet** | Define buffer corruption during skips | #21298 |
| **Parquet** | Metadata cache invalidation (local FS) | #21435 |
| **Query** | UnnestRewriter for deeply nested struct UNNEST | #21209 |
| **Query** | Window elimination optimizer | #21428 |
| **Query** | Decorrelation delim index bug | #21233 |
| **CTE** | Column pruning for CTEs | #21275 |
| **CTE** | Invalid common subplan CTE reuse | #21386 |
| **CSV** | Header detection type count expansion | #21292 |
| **View** | Restore bind_state when binding fails | #21193 |

---

### 7. Performance Improvements

These are transparent to quack-rs but relevant for extension performance:

| Area | Improvement | PR |
|------|-------------|----|
| **Parquet** | Cached metadata for cardinality estimates | #21358 |
| **Parquet** | Prefetch column range merging | #21373 |
| **Parquet** | Directory glob file sizes for cardinality | #21374 |
| **Parquet** | Init without global lock in multi-file reader | #21439 |
| **Parquet** | Batch index support for parallel metadata reads | #21314 |
| **Aggregation** | Dynamic radix bits for external aggregation | #21274 |
| **Query** | Predicate factoring optimization | #21418 |
| **Query** | Column pruning for MATERIALIZED CTEs | #21169 |
| **Query** | Row Group Pruner for NULLS_FIRST | #21399 |
| **Join** | Semi/anti/left delim join pushdown path | #21416 |
| **Bloom filter** | Atomic load in look-up | #21238 |
| **S3** | Globbing performance regression fixed | (blog) |
| **Struct** | Avoid scanning validity during pushdown extract | #21421 |

---

### 8. Extension Ecosystem Changes

DuckDB v1.5.1 bumps numerous extensions. These are independent of quack-rs
but relevant for extension authors who interact with these formats:

| Extension | Change | Relevance to quack-rs |
|-----------|--------|-----------------------|
| **lance** | New core extension (read/write Lance format) | None - DuckDB-managed extension |
| **iceberg** | v3 support (VARIANT, TIMESTAMP_NS, partitioned tables) | None - DuckDB-managed extension |
| **spatial** | Bumped (twice) | None |
| **delta** | Bumped | None |
| **httpfs** | Bumped, patches removed | `force_download_threshold` config |
| **ducklake** | Bumped | None |
| **vortex** | Bumped (multiple) | None |
| **avro** | Bumped | None |
| **sqlite** | Bumped | None |
| **postgres/mysql** | Bumped | None |

**Recommendation:** No action needed. These extensions are loaded at runtime
by DuckDB, not compiled into quack-rs extensions. Extension authors using
`INSTALL`/`LOAD` to interact with these formats will automatically get the
updates when users upgrade DuckDB.

---

### 9. CLI Changes

These do not affect quack-rs (SDK for loadable extensions), but may affect
E2E testing workflows that pipe SQL into the DuckDB CLI:

| Fix | PR | Impact on Testing |
|-----|----|-------------------|
| Non-interactive shell execution fix | (blog) | Critical if using piped SQLLogicTests |
| `-jsonlines` parameter restored | #21263 | If tests use JSON output |
| `.open` for Parquet files | #21269 | If tests open Parquet via CLI |
| Bail on error as default | #21344 | May change test error behavior |

**Recommendation:** If the E2E test suite (SQLLogicTests via `examples/hello-ext`)
pipes scripts to the DuckDB CLI, verify tests pass with v1.5.1 CLI binary.

---

### 10. Platform-Specific Changes

| Platform | Change | Impact on quack-rs |
|----------|--------|--------------------|
| z/OS | Adjusted for v1.5.0 | None - quack-rs doesn't target z/OS |
| Windows | UTF-8/UTF-16 fixes, Unicode entry point | None - CLI-only changes |
| MinGW | MSYS shell no longer used, well-defined env | None - quack-rs uses Cargo/rustc |

---

## Action Items Summary

### Immediate (v1.5.1 compatibility)

1. **Update `Cargo.lock`** — Run `cargo update` once `libduckdb-sys 1.10501.0`
   is published to crates.io
2. **Update documentation** — Mention v1.5.1 compatibility in `src/lib.rs`
   `DUCKDB_API_VERSION` doc comment
3. **Run full test suite** against v1.5.1 runtime to verify no regressions

### Short-term enhancements

4. **Add unmapped TypeId variants** — Consider `Any`, `SqlNull`, `Varint`
   (behind `duckdb-1-5` feature flag) for enum completeness
5. **Document new config options** — `force_download_threshold` and
   `allowed_configs` in the book/README
6. **Verify E2E tests** — Run SQLLogicTest suite with DuckDB v1.5.1 CLI binary

### Monitor for future releases

7. **VARIANT type** — Watch for `DUCKDB_TYPE_VARIANT` in future libduckdb-sys
   releases; add `TypeId::Variant` when it appears in the C API
8. **Lance extension API** — If Lance exposes extension-specific C APIs in
   the future, evaluate wrapping them
9. **Iceberg v3 types** — VARIANT and other Iceberg v3 types may eventually
   be exposed through the C Extension API

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Breaking C API change | Very Low | High | Version range `<2` prevents silent adoption |
| TypeId enum incomplete | Low | Low | `#[non_exhaustive]` protects downstream |
| WAL corruption in v1.5.0 | Medium | Critical | Recommend users upgrade to v1.5.1 |
| ART index bugs in v1.5.0 | Medium | High | Recommend users upgrade to v1.5.1 |
| Arrow crash with NULL dicts | Low | High | Fixed in v1.5.1, transparent to users |

---

## Conclusion

DuckDB v1.5.1 is a patch release with **no C API changes** that affect
quack-rs. The SDK is fully compatible. The primary action is upgrading
`libduckdb-sys` in `Cargo.lock` once v1.10501.0 is published, and
recommending that users upgrade their DuckDB runtime to v1.5.1 for critical
WAL corruption and ART index fixes.

The most significant forward-looking item is the VARIANT type for Iceberg v3,
which is not yet exposed in the C Extension API but may appear in a future
release. The `#[non_exhaustive]` attribute on `TypeId` ensures this can be
added as a non-breaking change.
