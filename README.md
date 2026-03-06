# quack-rs

[![CI](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/quack-rs.svg)](https://crates.io/crates/quack-rs)
[![docs.rs](https://img.shields.io/docsrs/quack-rs)](https://docs.rs/quack-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![MSRV: 1.84.1](https://img.shields.io/badge/MSRV-1.84.1-blue.svg)](https://blog.rust-lang.org/2025/01/30/Rust-1.84.1.html)

**The Rust SDK for building DuckDB loadable extensions — no C++ required.**

`quack-rs` provides safe, production-grade wrappers for the DuckDB C Extension API, removing
every known FFI pitfall so you can focus on writing extension logic in pure Rust.

---

## Table of Contents

- [The Pure-Rust DuckDB Problem](#the-pure-rust-duckdb-problem)
- [What quack-rs Solves](#what-quack-rs-solves)
- [Quick Start](#quick-start)
- [Module Reference](#module-reference)
  - [entry\_point](#entry_point)
  - [aggregate](#aggregate)
  - [scalar](#scalar)
  - [vector](#vector)
  - [types](#types)
  - [interval](#interval)
  - [error](#error)
  - [validate](#validate)
  - [scaffold](#scaffold)
  - [testing](#testing)
- [Pitfall Reference](#pitfall-reference)
- [Community Extension Submission](#community-extension-submission)
  - [Required Files](#required-files)
  - [description.yml Reference](#descriptionyml-reference)
  - [Extension Naming](#extension-naming)
  - [Platform Targets](#platform-targets)
  - [Extension Versioning](#extension-versioning)
  - [Release Profile Requirements](#release-profile-requirements)
  - [Binary Compatibility](#binary-compatibility)
  - [SQLLogicTest Format](#sqllogictest-format)
- [Scaffold Generator](#scaffold-generator)
- [Testing Guide](#testing-guide)
- [Validation Reference](#validation-reference)
- [Design Constraints](#design-constraints)
- [FAQ](#faq)
- [Contributing](#contributing)
- [License](#license)

---

## The Pure-Rust DuckDB Problem

DuckDB's official documentation acknowledges the problem directly:

> *"Writing a Rust-based DuckDB extension requires writing glue code in C++ and will force you
> to build through DuckDB's CMake & C++ based extension template. We understand that this is not
> ideal and acknowledge the fact that Rust developers prefer to work on pure Rust codebases."*
>
> — [DuckDB Community Extensions FAQ](https://duckdb.org/community_extensions/faq#can-i-write-extensions-in-rust)

**quack-rs closes that gap.** By building on the DuckDB C Extension API (available since
DuckDB v1.1), it provides a complete Rust SDK so extension authors never write a line of C,
C++, or CMake. The scaffold generator produces a submission-ready extension project — with
all required files — from a single function call.

---

## What quack-rs Solves

DuckDB's Rust FFI surface has undocumented pitfalls — struct layouts, callback contracts, and
initialization sequences not covered in the DuckDB documentation or the `libduckdb-sys` docs.
`quack-rs` was extracted from
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral), a production DuckDB
community extension, where each of these problems was discovered and solved.

| Pitfall | Without quack-rs | With quack-rs |
|---------|-----------------|---------------|
| **P3** Entry point SEGFAULT | `extract_raw_connection` dereferences invalid pointer | `init_extension` with correct C API init sequence |
| **L6** Silent registration failure | Function set members missing names → silently not registered | `AggregateFunctionSetBuilder` sets name on every member |
| **L1** combine config loss | Zero-initialized target state → wrong results | `AggregateTestHarness` catches missing field propagation |
| **L4** NULL output SEGFAULT | Forgetting `ensure_validity_writable` → crash | `VectorWriter::set_null` calls it automatically |
| **L5** Boolean UB | `as bool` cast on non-0/1 byte → undefined behavior | `VectorReader::read_bool` uses `u8 != 0` |
| **L7** Memory leak | `duckdb_logical_type` not freed after registration | `LogicalType` RAII wrapper with `Drop` |
| **P7** `duckdb_string_t` format | Two undocumented inline/pointer layouts → garbage reads | `DuckStringView` handles both formats |
| **P8** C API version confusion | Wrong version string → metadata script failure | `DUCKDB_API_VERSION` constant documents the distinction |
| **L2** State double-free | Second `destroy` call → crash or memory corruption | `FfiState<T>` nulls pointer after freeing |

Every pitfall has a detailed write-up in [LESSONS.md](LESSONS.md) — symptoms, root cause, and
the exact fix. Reading LESSONS.md before writing a DuckDB extension will save you days.

---

## Quick Start

### 1. Create an extension project

Generate a complete, submission-ready project from code:

```rust
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};

let config = ScaffoldConfig {
    name: "my_extension".to_string(),
    description: "My DuckDB extension".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Your Name".to_string(),
    github_repo: "yourorg/duckdb-my-extension".to_string(),
    excluded_platforms: vec![],
};

let files = generate_scaffold(&config).unwrap();
for file in &files {
    std::fs::create_dir_all(std::path::Path::new(&file.path).parent().unwrap()).unwrap();
    std::fs::write(&file.path, &file.content).unwrap();
}
```

Generated files:
- `Cargo.toml` — with correct `cdylib` crate type, pinned deps, and release profile
- `Makefile` — delegates to `cargo build` and `extension-ci-tools` for CI
- `extension_config.cmake` — tells DuckDB's build system about your extension
- `src/lib.rs` — entry point with `duckdb_entrypoint_c_api` macro
- `src/wasm_lib.rs` — WASM staticlib shim
- `description.yml` — required metadata for community extension submission
- `test/sql/my_extension.test` — SQLLogicTest skeleton
- `.github/workflows/extension-ci.yml` — complete cross-platform CI for your extension
- `.gitmodules` — `extension-ci-tools` submodule reference
- `.gitignore` — sensible defaults for Rust + DuckDB
- `.cargo/config.toml` — Windows CRT static linking

### 2. Add to an existing extension

Add to your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.1"
libduckdb-sys = { version = "=1.4.4", features = ["loadable-extension"] }

[lib]
name = "my_extension"   # MUST match extension name — see Pitfall P1 / L7
crate-type = ["cdylib", "rlib"]

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"         # REQUIRED — panics across FFI are undefined behavior
strip = true
```

### 3. Write the entry point

```rust
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;

fn register_functions(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        register_word_count(con)?;
        Ok(())
    }
}

entry_point!(my_extension, |con| register_functions(con));
```

### 4. Register an aggregate function

```rust
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateState, FfiState};
use quack_rs::types::TypeId;
use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_aggregate_state,
                    duckdb_data_chunk, duckdb_vector, idx_t};

#[derive(Default)]
struct WordCountState {
    count: u64,
}
impl AggregateState for WordCountState {}

unsafe extern "C" fn update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    state: duckdb_aggregate_state,
) {
    // Read inputs, update state
    if let Some(s) = FfiState::<WordCountState>::with_state_mut(state) {
        s.count += 1;
    }
}

unsafe extern "C" fn combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        // CRITICAL: copy ALL fields (Pitfall L1)
        if let (Some(src), Some(tgt)) = (
            FfiState::<WordCountState>::with_state(*source.add(i)),
            FfiState::<WordCountState>::with_state_mut(*target.add(i)),
        ) {
            tgt.count += src.count;
        }
    }
}

unsafe extern "C" fn finalize(
    _info: duckdb_function_info,
    state: duckdb_aggregate_state,
    output: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    use quack_rs::vector::VectorWriter;
    let mut writer = VectorWriter::new(output, count as usize);
    if let Some(s) = FfiState::<WordCountState>::with_state(state) {
        writer.write_u64(offset as usize, s.count);
    }
}

fn register_word_count(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
    unsafe {
        AggregateFunctionBuilder::new("word_count")
            .param(TypeId::Varchar)
            .returns(TypeId::BigInt)
            .state_size(FfiState::<WordCountState>::size_callback)
            .init(FfiState::<WordCountState>::init_callback)
            .update(update)
            .combine(combine)
            .finalize(finalize)
            .destructor(FfiState::<WordCountState>::destroy_callback)
            .register(con)
    }
}
```

### 5. Test without DuckDB

```rust
use quack_rs::testing::AggregateTestHarness;

#[test]
fn word_count_is_correct() {
    let result = AggregateTestHarness::<WordCountState>::aggregate(
        [1, 2, 3, 4, 5],
        |state, _| state.count += 1,
    );
    assert_eq!(result.count, 5);
}
```

See [examples/hello-ext](examples/hello-ext/) for a complete, runnable extension.

---

## Module Reference

### `entry_point`

Panic-free initialization for the DuckDB C Extension API.

**Why it exists**: DuckDB's `duckdb-loadable-macros` relies on `extract_raw_connection`, which
accesses the internal `Rc<RefCell<InnerConnection>>` layout. When that layout changes, it
causes a SEGFAULT. The correct approach is to call `duckdb_rs_extension_api_init` directly,
then `get_database` + `duckdb_connect`. `init_extension` implements this correctly.

```rust
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;

// Option 1: macro (recommended — generates the correct C entry point signature)
entry_point!(my_extension, |con| {
    register_my_function(con)?;
    Ok(())
});

// Option 2: manual entry point (for full control)
#[no_mangle]
pub unsafe extern "C" fn my_extension_init_c_api(
    info: libduckdb_sys::duckdb_extension_info,
    access: *const libduckdb_sys::duckdb_extension_access,
) -> bool {
    unsafe {
        quack_rs::entry_point::init_extension(info, access, quack_rs::DUCKDB_API_VERSION, |con| {
            register_my_function(con)?;
            Ok(())
        })
    }
}
```

**Key exported items**:
- `init_extension(info, access, api_version, register) -> bool` — core init helper
- `entry_point!(name, closure)` — macro that generates the `#[no_mangle] extern "C"` function
- `DUCKDB_API_VERSION` — C API version string (`"v1.2.0"` for DuckDB v1.4.x)

---

### `aggregate`

Builders for registering DuckDB aggregate functions.

#### `AggregateFunctionBuilder` — single-signature aggregate

```rust
use quack_rs::aggregate::{AggregateFunctionBuilder, FfiState};
use quack_rs::types::TypeId;

AggregateFunctionBuilder::new("my_agg")
    .param(TypeId::Varchar)
    .param(TypeId::Integer)
    .returns(TypeId::Double)
    .state_size(FfiState::<MyState>::size_callback)
    .init(FfiState::<MyState>::init_callback)
    .update(my_update)
    .combine(my_combine)
    .finalize(my_finalize)
    .destructor(FfiState::<MyState>::destroy_callback)
    .register(con)?;
```

#### `AggregateFunctionSetBuilder` — variadic / overloaded aggregate

For aggregates that accept different numbers of arguments (e.g., `retention(c1, c2, ..., c32)`):

```rust
use quack_rs::aggregate::AggregateFunctionSetBuilder;

let mut set = AggregateFunctionSetBuilder::new("retention");
for n in 1..=32_u32 {
    set.overload(|b| {
        let mut b = b.returns(TypeId::Ubigint);
        for _ in 0..n { b = b.param(TypeId::Boolean); }
        b.state_size(FfiState::<RetentionState>::size_callback)
         .init(FfiState::<RetentionState>::init_callback)
         .update(retention_update)
         .combine(retention_combine)
         .finalize(retention_finalize)
         .destructor(FfiState::<RetentionState>::destroy_callback)
    });
}
set.register(con)?;
```

#### `AggregateState` trait

Your state type must implement `AggregateState`:

```rust
use quack_rs::aggregate::AggregateState;

#[derive(Default)]
struct MyState {
    count: u64,
    // configuration fields — MUST be copied in combine (Pitfall L1)
    window_size: usize,
}
impl AggregateState for MyState {}
```

Requirements: `Default + Send + 'static`. No FFI dependencies in your state type.

#### `FfiState<T>` — raw pointer lifecycle manager

`FfiState<T>` wraps a `Box<T>` in a `#[repr(C)]` struct that satisfies DuckDB's `duckdb_aggregate_state` layout. It provides:

- `size_callback` — returns `size_of::<FfiState<T>>()`
- `init_callback` — allocates `Box<T>` via `T::default()`
- `destroy_callback` — frees the `Box<T>` and **nulls the pointer** (prevents double-free, Pitfall L2)
- `with_state(state) -> Option<&T>` — safe immutable access
- `with_state_mut(state) -> Option<&mut T>` — safe mutable access

---

### `scalar`

Builder for registering DuckDB scalar (row-level) functions.

```rust
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::TypeId;

ScalarFunctionBuilder::new("add_one")
    .param(TypeId::Integer)
    .returns(TypeId::Integer)
    .function(my_scalar_fn)
    .register(con)?;
```

---

### `vector`

Type-safe readers and writers for DuckDB data vectors.

#### `VectorReader` — read input data

```rust
use quack_rs::vector::VectorReader;

// In your update callback:
let reader = VectorReader::new(input_chunk, row_count);

for i in 0..row_count {
    if reader.is_valid(0, i) {          // check NULL for column 0, row i
        let s = reader.read_str(0, i);  // read VARCHAR
        let n = reader.read_i64(1, i);  // read BIGINT
        let b = reader.read_bool(2, i); // read BOOLEAN (never UB, Pitfall L5)
    }
}
```

Supported read methods: `read_bool`, `read_i8`, `read_i16`, `read_i32`, `read_i64`,
`read_u8`, `read_u16`, `read_u32`, `read_u64`, `read_f32`, `read_f64`,
`read_str`, `read_interval`.

#### `VectorWriter` — write output data

```rust
use quack_rs::vector::VectorWriter;

// In your finalize callback:
let mut writer = VectorWriter::new(output_vector, row_count);

writer.write_i64(row_index, 42_i64);
writer.set_null(row_index);   // automatically calls ensure_validity_writable (Pitfall L4)
```

Supported write methods: `write_bool`, `write_i8`, `write_i16`, `write_i32`, `write_i64`,
`write_u8`, `write_u16`, `write_u32`, `write_u64`, `write_f32`, `write_f64`, `set_null`.

#### `DuckStringView` — DuckDB's 16-byte string format

DuckDB stores strings in a 16-byte struct with two undocumented layouts (Pitfall P7):
- **Inline** (≤ 12 bytes): `[ len: u32 | data: [u8; 12] ]`
- **Pointer** (> 12 bytes): `[ len: u32 | prefix: [u8; 4] | ptr: *const u8 | _: u32 ]`

`VectorReader::read_str` handles both transparently. If you need direct access:

```rust
use quack_rs::vector::DuckStringView;

let view = DuckStringView::from_bytes(&raw_16_bytes);
let s: Option<&str> = view.as_str();   // None if not valid UTF-8
let len: usize = view.len();
```

---

### `types`

DuckDB's type system, safe to use in Rust.

#### `TypeId` — all DuckDB column types

```rust
use quack_rs::types::TypeId;

let ty = TypeId::BigInt;
println!("{}", ty.sql_name());    // "BIGINT"
println!("{ty}");                  // same
let raw = ty.to_duckdb_type();    // duckdb_type for FFI calls
```

Available variants: `Boolean`, `TinyInt`, `SmallInt`, `Integer`, `BigInt`,
`UTinyInt`, `USmallInt`, `UInteger`, `UBigInt`, `HugeInt`, `Float`, `Double`,
`Timestamp`, `TimestampTz`, `Date`, `Time`, `Interval`, `Varchar`, `Blob`, `Uuid`, `List`.

#### `LogicalType` — RAII wrapper

`duckdb_create_logical_type` allocates heap memory that must be freed. Forgetting to call
`duckdb_destroy_logical_type` leaks memory proportional to the number of registered functions
(Pitfall L7). `LogicalType` implements `Drop` and frees automatically.

```rust
use quack_rs::types::{LogicalType, TypeId};

let lt = LogicalType::new(TypeId::Varchar);
// Freed automatically when lt goes out of scope
```

---

### `interval`

Checked conversion from DuckDB's `INTERVAL` type to microseconds.

DuckDB stores `INTERVAL` as `{ months: i32, days: i32, micros: i64 }` (Pitfall P8).
Month conversion uses `1 month = 30 days` (matching DuckDB's own approximation).

```rust
use quack_rs::interval::{DuckInterval, interval_to_micros, interval_to_micros_saturating};

let iv = DuckInterval { months: 1, days: 0, micros: 0 };

// Checked: returns None on overflow
let micros: Option<i64> = interval_to_micros(iv);

// Saturating: clamps to i64::MIN / i64::MAX on overflow
let micros: i64 = interval_to_micros_saturating(iv);
```

---

### `error`

`ExtensionError` — the SDK's error type, usable with `?`.

```rust
use quack_rs::error::{ExtensionError, ExtResult};

fn register(con: duckdb_connection) -> ExtResult<()> {
    if con.is_null() {
        return Err(ExtensionError::new("connection is null"));
    }
    // ...
    Ok(())
}

// From any type that implements Display:
let err: ExtensionError = format!("failed at row {}", i).into();

// For FFI: convert to CString (truncates at embedded nulls)
let c_str = err.to_c_string();
```

---

### `validate`

Runtime validators that enforce DuckDB community extension compliance.
Run these before submitting your extension to catch violations early.

```rust
use quack_rs::validate::{
    validate_extension_name,
    validate_extension_version,
    validate_function_name,
    validate_spdx_license,
    validate_platform,
    validate_release_profile,
};

// Extension name rules
validate_extension_name("my_extension").unwrap();   // ok
validate_extension_name("MyExtension").unwrap_err(); // uppercase rejected
validate_extension_name("my extension").unwrap_err(); // spaces rejected

// Function name rules (SQL-safe identifiers)
validate_function_name("word_count").unwrap();
validate_function_name("word-count").unwrap_err();   // hyphens not allowed in SQL identifiers

// Semver and git-hash versions
validate_extension_version("1.0.0").unwrap();
validate_extension_version("0.1.0").unwrap();
validate_extension_version("690bfc5").unwrap();      // unstable (git hash)
validate_extension_version("not-a-version").unwrap_err();

// SPDX license
validate_spdx_license("MIT").unwrap();
validate_spdx_license("Apache-2.0").unwrap();
validate_spdx_license("FAKE-LICENSE").unwrap_err();

// Platform targets
validate_platform("linux_amd64").unwrap();
validate_platform("wasm_mvp").unwrap();
validate_platform("freebsd_amd64").unwrap_err();

// Release profile (parse from Cargo.toml values)
let check = validate_release_profile("abort", "true", "3", "1").unwrap();
assert!(check.is_fully_optimized());
assert!(check.panic_abort);
```

See [Validation Reference](#validation-reference) for the full API.

---

### `scaffold`

Project generator for new DuckDB Rust extensions. Produces a complete, submission-ready
file set — **no C++ or CMake required**.

```rust
use quack_rs::scaffold::{ScaffoldConfig, generate_scaffold};

let config = ScaffoldConfig {
    name: "my_analytics".to_string(),
    description: "Fast analytics functions for DuckDB".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Jane Doe".to_string(),
    github_repo: "janedoe/duckdb-my-analytics".to_string(),
    excluded_platforms: vec![],
};

let files = generate_scaffold(&config)?;
// Returns Vec<GeneratedFile> — callers decide how to write to disk

for file in &files {
    println!("{}: {} bytes", file.path, file.content.len());
}
```

`generate_scaffold` validates `name`, `version`, `license`, and all `excluded_platforms`
before generating any files, so invalid configurations fail fast.

Generated files are documented in [Scaffold Generator](#scaffold-generator).

---

### `testing`

Test aggregate business logic in pure Rust — no DuckDB process required.

```rust
use quack_rs::testing::AggregateTestHarness;

// Simple aggregate test
let result = AggregateTestHarness::<SumState>::aggregate(
    [10, 20, 30, 40, 50],
    |state, value| state.total += value,
);
assert_eq!(result.total, 150);

// Test combine (catches Pitfall L1 — config field propagation)
let mut source = AggregateTestHarness::<RetentionState>::new();
source.update(|s| {
    s.n_conditions = 3;
    s.counts[0] += 100;
});

let mut target = AggregateTestHarness::<RetentionState>::new();
target.combine(&source, |src, tgt| {
    tgt.n_conditions = src.n_conditions; // MUST copy config fields
    for i in 0..src.n_conditions {
        tgt.counts[i] += src.counts[i];
    }
});

let result = target.finalize();
assert_eq!(result.n_conditions, 3);
```

`AggregateTestHarness<S>` methods:
- `new()` — creates a zero-initialized state via `S::default()`
- `update(fn)` — calls your update logic on the state
- `combine(&other, fn)` — simulates DuckDB's segment-tree combine (source → target)
- `finalize()` — returns the inner state for inspection
- `aggregate(iter, fn)` — convenience method: update with each item, return final state

---

## Pitfall Reference

All 15 known DuckDB Rust FFI pitfalls are documented in [LESSONS.md](LESSONS.md) with
symptoms, root causes, and exact fixes. The table below is a navigation aid.

| ID | Pitfall | Status |
|----|---------|--------|
| **L1** | `combine` must propagate ALL config fields | Testable via `AggregateTestHarness` |
| **L2** | State `destroy` double-free → crash | Made impossible by `FfiState<T>` |
| **L3** | No panic across FFI boundaries | Made impossible by `init_extension` |
| **L4** | `ensure_validity_writable` required before NULL output | Made impossible by `VectorWriter::set_null` |
| **L5** | Boolean reading must use `u8 != 0` | Made impossible by `VectorReader::read_bool` |
| **L6** | Function set name must be set on each member | Made impossible by `AggregateFunctionSetBuilder` |
| **L7** | `duckdb_logical_type` memory leak | Made impossible by `LogicalType` RAII wrapper |
| **P1** | Library name must match extension name | Enforced by scaffold generator |
| **P2** | Extension metadata version ≠ DuckDB release version | Documented in `DUCKDB_API_VERSION` |
| **P3** | E2E testing is mandatory — unit tests alone are insufficient | Documented; SQLLogicTest scaffold generated |
| **P4** | `extension-ci-tools` submodule must be initialized | Enforced by `.gitmodules` in scaffold |
| **P5** | SQLLogicTest expected values must match DuckDB output exactly | Test skeleton generated by scaffold |
| **P6** | `duckdb_register_aggregate_function_set` silently fails | Builder returns `Err` on failure |
| **P7** | `duckdb_string_t` format is undocumented | Handled by `DuckStringView` |
| **P8** | `INTERVAL` struct layout is undocumented | Handled by `DuckInterval` |

---

## Community Extension Submission

### Required Files

To submit to <https://community-extensions.duckdb.org/>, your extension repository must contain:

| File | Purpose |
|------|---------|
| `Cargo.toml` | `cdylib` crate type, pinned deps, release profile |
| `Makefile` | Delegates to `cargo build` + `extension-ci-tools` |
| `extension_config.cmake` | CMake hook for DuckDB's build system |
| `src/lib.rs` | Entry point using `duckdb_entrypoint_c_api` |
| `description.yml` | Extension metadata (name, version, license, maintainers) |
| `extension-ci-tools/` | Git submodule (DuckDB CI/CD pipeline) |
| `test/sql/*.test` | SQLLogicTest format integration tests |

All of these are generated by `generate_scaffold`. The `extension-ci-tools` submodule is
referenced in `.gitmodules` and must be initialized with:

```bash
git submodule update --init --recursive
```

### description.yml Reference

```yaml
extension:
  name: your_extension             # must match [lib] name in Cargo.toml
  description: One-line description
  version: 0.1.0                   # semver or git hash
  language: Rust                   # always "Rust" for quack-rs extensions
  build: cargo                     # always "cargo" for quack-rs extensions
  license: MIT                     # SPDX identifier
  requires_toolchains: rust;python3
  excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"  # optional
  maintainers:
    - Your Name

repo:
  github: yourorg/your_extension
  ref: main                        # branch or tag to build from
```

Validate with:
```rust
use quack_rs::validate::{validate_extension_name, validate_extension_version,
                          validate_spdx_license, validate_platform};

validate_extension_name("your_extension")?;
validate_extension_version("0.1.0")?;
validate_spdx_license("MIT")?;
```

### Extension Naming

DuckDB community extensions must have **globally unique names** across the entire ecosystem.

- **Check existing names** at <https://community-extensions.duckdb.org/> before choosing
- **Use vendor prefixing** if needed (e.g., `myorg_analytics` instead of `analytics`)
- Names must match `^[a-z][a-z0-9_-]*$` — validated by `validate_extension_name`
- Maximum 64 characters
- The `[lib] name` in `Cargo.toml` **must match** the name in `description.yml` (Pitfall P1)
- The DuckDB Foundation may require a rename if a naming conflict arises

```rust
use quack_rs::validate::validate_extension_name;

// Check before submitting
validate_extension_name("my_extension")?;
```

### Platform Targets

Extensions are built for all platforms by default. Exclude platforms that cannot build:

| Platform | Description |
|----------|-------------|
| `linux_amd64` | Linux x86\_64 |
| `linux_amd64_gcc4` | Linux x86\_64 (GCC 4 ABI) |
| `linux_arm64` | Linux AArch64 |
| `osx_amd64` | macOS x86\_64 |
| `osx_arm64` | macOS Apple Silicon |
| `windows_amd64` | Windows x86\_64 (MSVC) |
| `windows_amd64_mingw` | Windows x86\_64 (MinGW) |
| `windows_arm64` | Windows AArch64 |
| `wasm_mvp` | WebAssembly (MVP) |
| `wasm_eh` | WebAssembly (exception handling) |
| `wasm_threads` | WebAssembly (threads) |

Validate exclusions:
```rust
use quack_rs::validate::platform::validate_excluded_platforms;

validate_excluded_platforms(&["wasm_mvp", "wasm_eh", "wasm_threads"])?;
```

### Extension Versioning

| Tier | Format | Example | Stability |
|------|--------|---------|-----------|
| **Unstable** | Short git hash (7+ hex chars) | `690bfc5` | No stability guarantees |
| **Pre-release** | Semver `0.y.z` | `0.1.0` | API may have breaking changes in minor versions |
| **Stable** | Semver `x.y.z` (x ≥ 1) | `1.0.0` | Full semver; breaking changes require major bump |

```rust
use quack_rs::validate::{validate_extension_version, semver::classify_extension_version};

validate_extension_version("0.1.0")?;    // pre-release
validate_extension_version("690bfc5")?;  // unstable (git hash)

let tier = classify_extension_version("1.0.0"); // VersionClass::Stable
```

### Release Profile Requirements

Extensions are shared libraries loaded into DuckDB's process. The release profile must be:

```toml
[profile.release]
panic = "abort"    # REQUIRED — panics across FFI are undefined behavior
lto = true         # STRONGLY RECOMMENDED — reduces binary size, improves performance
opt-level = 3      # RECOMMENDED — maximum optimization
codegen-units = 1  # RECOMMENDED — enables cross-crate inlining
strip = true       # RECOMMENDED — strips debug symbols from release binary
```

Validate programmatically:
```rust
use quack_rs::validate::validate_release_profile;

let check = validate_release_profile("abort", "true", "3", "1")?;
assert!(check.panic_abort);         // REQUIRED
assert!(check.lto_enabled);         // recommended
assert!(check.is_fully_optimized()); // all settings optimal
```

### Binary Compatibility

- Extension binaries are tied to a specific DuckDB version and platform triple
- A binary compiled for DuckDB v1.4.4 will not load in DuckDB v1.4.3 or v1.5.0
- DuckDB verifies binary compatibility at load time and refuses mismatched binaries
- All extensions in the community registry are cryptographically signed
- Unsigned extensions require `SET allow_unsigned_extensions = true` (development only)
- Rebuild for each DuckDB release — the scaffold's CI workflow automates this

### SQLLogicTest Format

Integration tests use DuckDB's SQLLogicTest format. The scaffold generates a skeleton:

```sqllogictest
# test/sql/my_extension.test

require my_extension

query I
SELECT my_function('hello');
----
expected_output_here
```

Key rules:
- `require extension_name` — ensures the extension is loaded before tests run
- `query T` — VARCHAR result; `query I` — INTEGER; `query R` — REAL; `query B` — BOOLEAN
- Expected output must match DuckDB's exact output format (no trailing spaces, `NULL` not `null`)
- Generate expected values by running queries in the DuckDB CLI and copying the output
- See [Pitfall P5](LESSONS.md#p5-sqllogictest-expected-values-must-match-actual-duckdb-output)

---

## Scaffold Generator

`generate_scaffold` validates inputs and returns a `Vec<GeneratedFile>` — it does **not** write
to disk. Callers decide how to persist the files.

```rust
use quack_rs::scaffold::{ScaffoldConfig, GeneratedFile, generate_scaffold};

let config = ScaffoldConfig {
    name: "my_extension".to_string(),
    description: "Fast analytics for DuckDB".to_string(),
    version: "0.1.0".to_string(),
    license: "MIT".to_string(),
    maintainer: "Jane Doe".to_string(),
    github_repo: "janedoe/duckdb-my-extension".to_string(),
    excluded_platforms: vec!["wasm_mvp".to_string(), "wasm_eh".to_string()],
};

let files: Vec<GeneratedFile> = generate_scaffold(&config)?;
```

### Generated File Reference

| Path | Description |
|------|-------------|
| `Cargo.toml` | `cdylib` + `staticlib` (WASM), pinned deps, release profile |
| `Makefile` | Delegates to `cargo` + `extension-ci-tools` |
| `extension_config.cmake` | CMake hook declaring the extension to DuckDB's build system |
| `src/lib.rs` | Entry point with `duckdb_entrypoint_c_api`, example table function |
| `src/wasm_lib.rs` | WASM staticlib shim (`mod lib`) |
| `description.yml` | Community extension metadata (name, version, license, maintainers) |
| `test/sql/{name}.test` | SQLLogicTest skeleton with `require` and basic query |
| `.github/workflows/extension-ci.yml` | Cross-platform CI: build + test on all DuckDB platforms |
| `.gitmodules` | `extension-ci-tools` submodule reference |
| `.gitignore` | `/target`, `*.duckdb`, `build/`, `.env` |
| `.cargo/config.toml` | Windows MSVC CRT static linking |

### Validation on Scaffold

The generator enforces all submission requirements at generation time:

- `name` must pass `validate_extension_name` — no uppercase, no spaces, ≤ 64 chars
- `version` must pass `validate_extension_version` — semver or git hash
- `license` must pass `validate_spdx_license` — recognized SPDX identifier
- Each entry in `excluded_platforms` must pass `validate_platform` — known DuckDB target

```rust
// This fails fast — no files are generated if config is invalid
let err = generate_scaffold(&ScaffoldConfig {
    name: "Invalid Name".to_string(), // spaces → error
    ..
}).unwrap_err();
assert!(err.as_str().contains("invalid character"));
```

---

## Testing Guide

### Why unit tests alone are insufficient (Pitfall P3)

In the `duckdb-behavioral` extension (the project that produced quack-rs), **435 unit tests
passed** while the extension had three critical bugs: a SEGFAULT on load, 6 of 7 functions
silently not registered, and wrong results from a window aggregate. Unit tests cannot detect
these because they never load the compiled `.so` into a DuckDB process.

**Always run E2E tests** against a real DuckDB binary.

### Test layers

| Layer | What it tests | How to run |
|-------|--------------|------------|
| **Unit tests** (`#[cfg(test)]`) | Pure Rust logic, no DuckDB API calls | `cargo test` |
| **Integration tests** (`tests/`) | Cross-module logic, no DuckDB API calls | `cargo test --test integration_test` |
| **Property tests** (proptest) | Mathematical properties across the full input domain | `cargo test` (included) |
| **E2E tests** (SQLLogicTest) | Loaded extension behavior in real DuckDB | `make test` in extension repo |

### Constraint: the `loadable-extension` feature

`libduckdb-sys` with `features = ["loadable-extension"]` routes every DuckDB C API call
through lazy `AtomicPtr` dispatch. These pointers are only initialized when
`duckdb_rs_extension_api_init` is called during a real DuckDB extension load event. Calling
any `duckdb_*` function in a unit test panics with "DuckDB API not initialized".

This is why `AggregateTestHarness` exists — it tests your state logic without touching the
DuckDB C API.

### Testing with `AggregateTestHarness`

```rust
use quack_rs::testing::AggregateTestHarness;

#[derive(Default)]
struct SumState { total: i64 }
impl quack_rs::aggregate::AggregateState for SumState {}

#[test]
fn sum_aggregate() {
    let result = AggregateTestHarness::<SumState>::aggregate(
        [1i64, 2, 3, 4, 5],
        |state, v| state.total += v,
    );
    assert_eq!(result.total, 15);
}

#[test]
fn combine_propagates_config() {
    // Simulates DuckDB's segment-tree combine: source → fresh zero-initialized target
    let mut source = AggregateTestHarness::<SumState>::new();
    source.update(|s| s.total = 100);

    let mut target = AggregateTestHarness::<SumState>::new();
    target.combine(&source, |src, tgt| tgt.total += src.total);

    assert_eq!(target.finalize().total, 100);
}
```

### Running E2E tests

Build the extension and test against the DuckDB CLI:

```bash
# In your extension repository (generated by scaffold)
git submodule update --init --recursive
make configure
make release
make test
```

`make test` runs all `.test` files in `test/sql/` using DuckDB's SQLLogicTest runner.

---

## Validation Reference

All validators live in `quack_rs::validate`. They return `Result<_, ExtensionError>` and are
suitable for use in pre-submission checks, CI scripts, or as library functions.

| Function | What it validates | Module |
|----------|-----------------|--------|
| `validate_extension_name(s)` | `^[a-z][a-z0-9_-]*$`, ≤ 64 chars | `validate` |
| `validate_function_name(s)` | `^[a-z_][a-z0-9_]*$`, SQL-safe | `validate` |
| `validate_semver(s)` | `MAJOR.MINOR.PATCH` semver | `validate` |
| `validate_extension_version(s)` | semver or 7+ char git hash | `validate` |
| `validate_spdx_license(s)` | recognized SPDX license identifier | `validate` |
| `validate_platform(s)` | known DuckDB build target | `validate` |
| `validate_excluded_platforms(slice)` | slice of platform strings, no duplicates | `validate::platform` |
| `validate_release_profile(panic, lto, opt, cgu)` | release profile settings | `validate` |
| `classify_extension_version(s)` | returns `VersionClass` enum | `validate::semver` |

### Accepted SPDX Licenses

The most common licenses accepted by DuckDB community extensions:
`MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `MPL-2.0`, `GPL-2.0-only`, `GPL-3.0-only`,
`LGPL-2.1-only`, `LGPL-3.0-only`, `AGPL-3.0-only`, `Unlicense`, `CC0-1.0`.

See `validate::spdx` for the full list.

---

## Design Constraints

### Runtime dependency: `libduckdb-sys` only

The `duckdb` crate provides a high-level Rust API but can bundle DuckDB. Extensions must
**not** bundle DuckDB — they link against the DuckDB that loads them. `libduckdb-sys` with
`features = ["loadable-extension"]` provides lazy function pointers populated by DuckDB at
load time, which is the correct approach.

### Exact version pin (`=1.4.4`)

The DuckDB C API can change between minor releases. The `=` pin ensures that extensions built
with quack-rs link against exactly the DuckDB version they were tested against. Before
bumping the pin, audit all callback signatures against the new `bindgen.rs` output and update
`DUCKDB_API_VERSION` if the C API version string changed.

### No proc macros

The SDK uses declarative macros only (`entry_point!`, `paste!`). This keeps compile times
short and avoids proc-macro dependencies.

### No panics in FFI paths

`unwrap()`, `expect()`, and `panic!()` are forbidden in callbacks and entry points. The Rust
runtime cannot unwind panics across FFI boundaries — the result is undefined behavior.
All SDK error paths use `Result`/`Option` and `?`.

### MSRV 1.84.1

Required for `&raw mut` syntax (stable in 1.82) and `const extern fn` support.
The MSRV is enforced in CI via `cargo +1.84.1 check`.

---

## FAQ

**Q: Can I write a DuckDB extension in pure Rust without any C++?**

Yes, using quack-rs and the DuckDB C Extension API. The scaffold generator produces a
complete project with no C++, no CMakeLists.txt, and no glue code. See [Quick Start](#quick-start).

**Q: Are community extensions safe to install?**

Community extensions are not vetted for security by the DuckDB team. The community extensions
repository is a distribution mechanism, not a security guarantee. Extension authors are
responsible for the safety of their code. quack-rs enforces `panic = "abort"` and provides
safe FFI wrappers, but cannot make security guarantees about your extension's logic.

**Q: Can I expose SQL macros as an extension?**

Yes. Writing macros in SQL and packaging them as a DuckDB extension is well-established
(see `pivot_table` and `chsql`). This currently requires some C++ wrapper code, but is one
of the simplest extension patterns.

**Q: How do I handle naming collisions?**

Extension names must be globally unique. Check <https://community-extensions.duckdb.org/> first.
Use vendor prefixing: `myorg_analytics` instead of `analytics`. The DuckDB Foundation
reserves the right to require renames on a case-by-case basis.

**Q: What if a platform can't build my extension?**

Declare it in `excluded_platforms` in `description.yml`:
```yaml
excluded_platforms: "wasm_mvp;wasm_eh;wasm_threads"
```
Validate with `quack_rs::validate::platform::validate_excluded_platforms`.

**Q: The CI toolchain is missing a dependency I need. What do I do?**

Open a PR to [duckdb/extension-ci-tools](https://github.com/duckdb/extension-ci-tools) to
add it as an optional extra. Alternatively, install the dependency in your Makefile, but
this may be fragile as DuckDB toolchains are updated. See the [DuckDB Community FAQ](https://duckdb.org/community_extensions/faq).

**Q: What is the difference between C API version and DuckDB release version?**

DuckDB v1.4.4 uses C API version `"v1.2.0"`. The C API version is what you pass to
`duckdb_rs_extension_api_init` and `append_extension_metadata.py -dv`. Using the DuckDB
release version causes the metadata script to fail. See Pitfall P8 in [LESSONS.md](LESSONS.md).

**Q: How do I debug a SEGFAULT when loading my extension?**

1. Confirm your entry point is generated by `entry_point!` or calls `init_extension` correctly.
2. Check that `DUCKDB_API_VERSION` matches your DuckDB version.
3. Build with `RUSTFLAGS="-g"` and use `lldb` / `gdb` to get a stack trace.
4. Check that no `duckdb_*` functions are called before `duckdb_rs_extension_api_init`.
5. See [LESSONS.md](LESSONS.md) Pitfalls P3 and L3.

**Q: How does quack-rs stay up to date with DuckDB releases?**

When a new DuckDB version ships, we:
1. Read the DuckDB changelog for C API changes.
2. Update the `=x.y.z` pin in `Cargo.toml`.
3. Update `DUCKDB_API_VERSION` if the C API version string changed.
4. Audit all callback signatures against the new `bindgen.rs`.
5. Release a new quack-rs version.

Subscribe to [GitHub releases](https://github.com/tomtom215/quack-rs/releases) for notifications.

---

## Examples

- [examples/hello-ext](examples/hello-ext/) — complete `word_count` aggregate extension
  demonstrating: `FfiState<T>`, `VectorReader`, `AggregateTestHarness`, and `entry_point!`

---

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting PRs.

All contributions must pass:

```bash
cargo test && \
cargo clippy --all-targets -- -D warnings && \
cargo fmt -- --check && \
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

These same checks run in CI on every push and pull request.

**To document a new pitfall**: add an entry to [LESSONS.md](LESSONS.md) following the
existing format (symptom → root cause → fix → SDK status).

---

## AI-Assisted Development

This project was developed with assistance from [Claude Code](https://claude.ai/code)
(Anthropic). AI was used for code generation, documentation drafting, test authoring, and
research into DuckDB's C Extension API and community extension infrastructure. All
AI-generated code was reviewed, tested, and validated by the project maintainer.

---

## License

MIT — see [LICENSE](LICENSE).
