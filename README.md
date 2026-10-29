<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/tomtom215/quack-rs/main/assets/logos/logo1-dark-elegant.svg">
    <img src="https://raw.githubusercontent.com/tomtom215/quack-rs/main/assets/logos/logo2-light-docs.svg" alt="quack-rs — a Rust SDK for DuckDB extensions" width="600">
  </picture>
  <p><em>/ˈkwækərz/ &nbsp;·&nbsp; rhymes with <em>crackers</em> &nbsp;·&nbsp; inspired by DuckDB</em></p>
  <p>
    <a href="https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml"><img src="https://github.com/tomtom215/quack-rs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://crates.io/crates/quack-rs"><img src="https://img.shields.io/crates/v/quack-rs.svg" alt="Crates.io"></a>
    <a href="https://quack-rs.com/"><img src="https://img.shields.io/badge/docs-book-blue.svg" alt="Documentation"></a>
    <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
    <a href="https://blog.rust-lang.org/2025/04/03/Rust-1.86.0/"><img src="https://img.shields.io/badge/MSRV-1.86.0-blue.svg" alt="MSRV: 1.86.0"></a>
  </p>
</div>

**Write DuckDB loadable extensions in pure Rust — no C, no C++, no glue code.**

## What it is

`quack-rs` wraps DuckDB's [C Extension API](https://duckdb.org/community_extensions/development)
so that you can write a loadable extension entirely in Rust.

- **Every function kind the C API can register:** scalar, aggregate, table, cast,
  copy and replacement scan functions, plus SQL macros.
- **Scalar functions as safe closures.** `ScalarFunctionBuilder::map1` and its
  siblings take a plain Rust closure, derive the SQL types from its signature,
  return NULL for NULL input, and turn a panic into a SQL error.
- **Guards against documented FFI pitfalls.** Most of the 34 pitfalls in
  [`LESSONS.md`](./LESSONS.md) are prevented by a wrapper, such as NULL output
  that is silently lost, a boolean read that is undefined behaviour, or a double
  free in an aggregate destructor. The rest are documented.
- **A panic in a guarded callback becomes a SQL error.** The entry-point macros,
  the closure-based builders and the `*_callback!` macros run your code under
  `catch_unwind`. A raw `unsafe extern "C" fn` that you pass to a builder
  yourself is installed unguarded; see [Installation](https://quack-rs.com/getting-started/installation.html#why-panic--unwind-not-abort).
- **Aggregate logic you can test without DuckDB.** `AggregateTestHarness` runs
  the update, combine and finalize steps in plain Rust.
- **A load-time ABI check.** The extension compares the C API layout it was
  built against with the one the running DuckDB hands it and, by default,
  refuses to load on a mismatch.
- **Community-extension tooling.** A project scaffold, runtime validators for
  `description.yml`, names, versions and platforms, and an `append_metadata`
  binary that writes the footer DuckDB requires.

## Quick Start

**1. `Cargo.toml`.** `panic = "unwind"` is required: under `panic = "abort"`
nothing can be caught, and a panic kills the user's DuckDB process.

```toml
[package]
name = "my_extension"
version = "0.1.0"
edition = "2021"
rust-version = "1.86"       # with resolver 3, Cargo picks dependency versions that build on it
resolver = "3"

[lib]
name = "my_extension"       # must match the extension name (Pitfall P1)
crate-type = ["cdylib", "rlib"]

[dependencies]
quack-rs = "0.19"
libduckdb-sys = { version = ">=1.4.4, <2", features = ["loadable-extension"] }

[profile.release]
panic = "unwind"            # required
lto = true                  # the rest are recommended
opt-level = 3
codegen-units = 1
strip = true
```

**2. `src/lib.rs`.**

```rust
use libduckdb_sys::duckdb_connection;
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;

fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    // Parameter and return types come from the closure signatures.
    // A NULL argument gives a NULL result without calling the closure.
    // `try_map1` lets the closure fail: a plain `x * 2` wraps on overflow in a
    // release build, and `Err` becomes a SQL error instead.
    let double_it = ScalarFunctionBuilder::try_map1("double_it", |x: i64| {
        x.checked_mul(2).ok_or("double_it: BIGINT overflow")
    })?;
    let shout = ScalarFunctionBuilder::map1_str("shout", |s: &str| s.to_uppercase())?;

    // SAFETY: `con` is the live connection DuckDB passed to the entry point.
    unsafe {
        double_it.register(con)?;
        shout.register(con)?;
    }
    Ok(())
}

// The symbol must be `<extension name>_init_c_api`.
entry_point!(my_extension_init_c_api, |con| register(con));
```

**3. Build, stamp and load.** DuckDB refuses to `LOAD` a bare shared library,
so append the metadata footer first. `--duckdb-version` is the C API version
(`v1.2.0`), not a DuckDB release (Pitfall P2). Installing `append_metadata`
needs Rust 1.88 or newer, because `cargo install` resolves the newest
`libduckdb-sys`, which requires it; use `cargo +stable install` if your
project pins an older toolchain. Your extension itself still builds on 1.86.

```shell
cargo build --release
cargo install quack-rs --bin append_metadata
append_metadata target/release/libmy_extension.so my_extension.duckdb_extension \
    --abi-type C_STRUCT --extension-version v0.1.0 \
    --duckdb-version v1.2.0 --platform linux_amd64

duckdb -unsigned -c "LOAD './my_extension.duckdb_extension'; SELECT double_it(21), shout('hello');"
# ┌───────────────┬────────────────┐
# │ double_it(21) │ shout('hello') │
# │     int64     │    varchar     │
# ├───────────────┼────────────────┤
# │            42 │ HELLO          │
# └───────────────┴────────────────┘
```

On macOS the library is `libmy_extension.dylib` (platform `osx_arm64` or
`osx_amd64`; `osx_amd64` is opt-in in extension-ci-tools for DuckDB 2.0, and
DuckDB 2.1 drops it); on Windows it is `my_extension.dll` (`windows_amd64`).

To start a community extension from scratch, the
[project scaffold](https://quack-rs.com/getting-started/scaffold.html) generates
the `Cargo.toml`, entry point, `description.yml`, SQLLogicTest skeleton and CI
workflow. [Your First Extension](https://quack-rs.com/getting-started/first-extension.html)
continues with aggregates and table functions.

## Why quack-rs, and when to use something else

DuckDB 2.0 has two C extension APIs, and the Rust options divide along that
line. Checked against the published sources of `duckdb` and
`duckdb-loadable-macros` 1.20000.1, `duckdb-neo` 0.20000.1 and the
`extension-template-rs` repository's `main` on 2026-10-29:

| | quack-rs | `duckdb` + `duckdb-loadable-macros` | `extension-template-rs` + `duckdb-neo` |
|---|---|---|---|
| C API | v1, stable | v1 by default (`<name>_init_c_api`); v2 behind the macros' `neo` feature | v2, unstable (`<name>_init_c_api_v2`) |
| Binary loads in | every DuckDB from 1.4.4 through 2.0.x, with the default features | set by the C API v1 stamp, as for quack-rs | only the DuckDB release it was built for ("Binaries load only in the exact TARGET_DUCKDB_VERSION", the template's `Makefile`) |
| Wraps | scalar, aggregate, table, cast, copy and replacement-scan functions; SQL macros | scalar (`vscalar`) and table (`vtab`) functions | modules for scalar, aggregate, table, cast, copy, macro and pragma functions, replacement scans, custom types and options, and log sinks |
| Status | pre-1.0, audited (below) | the `duckdb` crate | `duckdb-neo` describes itself as experimental |

Use `duckdb-neo` if you target DuckDB 2.0 and later only and want upstream's
wrapper, or need what only the v2 API offers (pragma functions, for one).
Use quack-rs if one binary has to load across DuckDB releases, or if you need
aggregate, cast, copy or replacement-scan functions on the stable API.
quack-rs does not generate the C API v2 entry point, and will not until DuckDB
declares part of that API stable.

## Status

`quack-rs` is pre-1.0 (0.19.0). Judge it against your own requirements; the
record so far:

- **The API still changes.** A minor release before 1.0 can break it; each lists
  its breaking changes and migration notes in [`CHANGELOG.md`](./CHANGELOG.md).
- **Audits keep finding real defects.** The review released as 0.16.0 fixed
  defects in earlier releases, including two heap-corruption paths. The 0.18.0
  audits fixed further soundness holes in the safe API, process aborts and wrong
  answers, and found a serious defect in unreleased code before publication:
  aggregate functions returned wrong results in release builds with Cargo's
  default profile. [`AUDIT.md`](./AUDIT.md) records each audit, what it found,
  and whether each fix was reproduced against a real DuckDB or derived from
  DuckDB's source.
- **Some limits are DuckDB's.** Defects in DuckDB's C API that quack-rs can only
  document or work around are listed under
  [Known Limitations](https://quack-rs.com/reference/known-limitations.html).
- **What CI checks.** Every pull request runs the full test suite against a real
  DuckDB (1.5.6) on Linux, macOS and Windows; the suite again against DuckDB
  1.4.4, 1.4.5, 1.5.0, 1.5.3, 1.5.4 and 1.5.5; the unit tests on wasm32;
  the suite against DuckDB 2.0.0 with the DuckDB 2.0 bindings;
  AddressSanitizer; and a load test of a built extension into DuckDB 1.4.4,
  1.5.0, 1.5.5, 1.5.6, 2.0.0 and the latest release.

If you ship an extension built on it, test it end to end against every DuckDB
release you support.

## Supported versions

- **DuckDB:** 1.4.4 through 2.0.0. The `libduckdb-sys = ">=1.4.4, <2"` range
  covers the bindings for all of them, 1.4.4 through 1.20000.0 (DuckDB 2.0,
  published 2026-10-22). 1.20000.0 needs Rust 1.88; on 1.88 or newer
  `cargo update` selects it, and an older toolchain with MSRV-aware resolution
  stays on 1.10506.x (DuckDB 1.5.6).
- **Rust (MSRV):** 1.86.0.

DuckDB 2.0.0 (2026-10-21) is a long-term support release, supported until at
least October 2027; DuckDB supports 1.4 until late November 2026 and 1.5 until
March 2027.

With the default features, one extension binary loads into every DuckDB release
from 1.4.4 through 2.0.0, because it is stamped with C API version `v1.2.0`.
DuckDB 1.4.4 to 1.5.5 declare that version; 1.5.6 and 2.0.0 declare `v1.5.6`
and still load `v1.2.0` extensions. The `duckdb-1-5*` features use the part of
DuckDB's C API whose layout differed between releases before 1.5.6: build such
an extension against the 1.5.6 bindings and stamp it `C_STRUCT` with C API
`v1.5.6`, and it loads into DuckDB 1.5.6 and 2.0.0. See
[ABI Compatibility](https://quack-rs.com/concepts/abi.html) before you enable one.

## Cargo features

All features are off by default. The default build uses only the stable C API.

| Feature | Requires | Enables |
|---------|----------|---------|
| `duckdb-1-5` | `libduckdb-sys >= 1.10500.0` (DuckDB 1.5.0) | Wrappers for the C API added in DuckDB 1.5, among them scalar bind/init and local state, config options, copy functions, catalog, client context, structured errors, bound expressions, the virtual file system, selection vectors and the instance cache |
| `duckdb-1-5-3` | `libduckdb-sys >= 1.10503.0` (DuckDB 1.5.3); implies `duckdb-1-5` | `TypeId::Variant` and `TypeId::Geometry` |
| `duckdb-1-5-4` | `libduckdb-sys >= 1.10504.0`; implies `duckdb-1-5-3`; a DuckDB 1.5.0+ engine at run time | The `arrow` module (Arrow C Data Interface bridge, no `arrow` crate dependency) |
| `bundled-test` | A C++ toolchain (compiles DuckDB from source, about 5–10 minutes cold) | `testing::InMemoryDb`, a real in-memory DuckDB for SQL-level tests in `cargo test` |
| `bundled-test-prebuilt` | A pre-built libduckdb: `DUCKDB_DOWNLOAD_LIB=1` or `DUCKDB_LIB_DIR` (an extracted 1.x `libduckdb-<platform>.zip` or 2.0 `duckdb-shared-libs-<platform>.tar.gz`); header discovery needs `libduckdb-sys >= 1.10503` (older: set `DUCKDB_INCLUDE_DIR`) | The same `InMemoryDb`, linked against the pre-built library |

Enable the two test features as dev-dependency features only. The
`duckdb-1-5-4` floor comes from the bindings: `libduckdb-sys` shipped the Arrow
struct layouts first in 1.10504.0, although the DuckDB functions exist from
1.4.4.

## Documentation

- **The book**, [quack-rs.com](https://quack-rs.com/): guides from the
  first extension to community submission. The site root is the latest
  release; `https://quack-rs.com/vX.Y/` is each release line (for example
  [/v0.19/](https://quack-rs.com/v0.19/)), and [/dev/](https://quack-rs.com/dev/)
  is the `main` branch.
- **API reference**: [docs.rs/quack-rs](https://docs.rs/quack-rs). The crate
  root lists every module.
- [Pitfall Catalog](https://quack-rs.com/reference/pitfalls.html) and
  [`LESSONS.md`](./LESSONS.md): all 34 known pitfalls (L1–L19, P1–P15), with
  symptom, cause and fix.
- [Community Extensions](https://quack-rs.com/publishing.html): `description.yml`,
  naming, versioning, platforms and submission.
- [Testing Guide](https://quack-rs.com/testing.html): mocks, `AggregateTestHarness`
  and `InMemoryDb`.
- [`docs/architecture.md`](./docs/architecture.md): module layout, design
  principles and the architecture decision records.
- [`CHANGELOG.md`](./CHANGELOG.md): every release, with breaking changes and
  migration notes. The latest is 0.19.0 (2026-10-22).
- [`CONTRIBUTING.md`](./CONTRIBUTING.md): development setup, quality gates and
  the test strategy.
- [`SECURITY.md`](./SECURITY.md): supported versions and how to report a
  vulnerability.

## License

MIT — see [`LICENSE`](./LICENSE).

---

*DuckDB is a trademark of the DuckDB Foundation. `quack-rs` is an independent
community project; it is not affiliated with or endorsed by the DuckDB Foundation
or DuckDB Labs.*
