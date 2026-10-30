# Installation

This page covers the `Cargo.toml` dependencies and release-profile settings a DuckDB
loadable extension built with quack-rs needs, the minimum Rust version, and the optional
test features.

## Adding quack-rs to an existing extension

Add the following to your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.20"

[dependencies.libduckdb-sys]
version = ">=1.4.4, <1.20100"
default-features = false
features = ["loadable-extension"]
```

Copy this block rather than running `cargo add libduckdb-sys`: `cargo add` writes the
newest version, with the default features on and without `loadable-extension`.

> **Why `>=1.4.4, <1.20100`?**
> DuckDB 1.4.4 through 2.0.x load extensions built for C API version `v1.2.0` (1.5.6 and
> 2.0.x declare `v1.5.6` and accept every earlier one), so `quack-rs` supports them all with
> a single bounded range. Releases before 1.4.4 are not supported. `libduckdb-sys` numbers
> DuckDB `vMAJOR.MINOR.PATCH` as `1.(10000·MAJOR + 100·MINOR + PATCH).N`, so the range runs
> from the DuckDB 1.4.4 bindings (`1.10404.x`) through the DuckDB 2.0 bindings
> (`1.20000.x`) and stops before DuckDB 2.1 (`1.20100.0`).
>
> **The upper bound.** quack-rs, and every extension that uses its macros, names the C API
> through the crate root: `libduckdb_sys::duckdb_connection`,
> `libduckdb_sys::duckdb_register_scalar_function`, and so on. In the 1.20000 bindings
> those root names exist only as a compatibility re-export of the crate's `v1` module,
> marked `// TODO: Remove in the future` upstream. If a later release removes it, quack-rs
> and your extension stop compiling. So quack-rs widens the range one DuckDB minor at a time,
> after running its tests against the new bindings and the new engine.
>
> **When DuckDB 2.1 bindings (`1.20100.x`) are published**, nothing breaks: Cargo keeps
> resolving `1.20000.x`. An extension built that way keeps the default stamp (`C_STRUCT`,
> `v1.2.0`), which DuckDB's development branch, the 2.1 line, still loads as 2.0 does (its
> C API v1 loader was unchanged from v2.0.0 to `main` when this was checked; a release of 2.1
> could still change that). If you ask for a newer `libduckdb-sys` yourself, Cargo
> reports a conflict with this range. There is nothing to configure on your side (a
> `[patch]` entry does not change a version requirement): wait for the quack-rs release that
> widens the range; its changelog entry says so.
>
> **Rust 1.88.** The 1.20000.x releases need Rust 1.88. On Rust 1.86 or 1.87, Cargo's
> default (edition 2021, resolver 2) ignores `rust-version` and selects them anyway, so the
> build fails: set `rust-version = "1.86"` and `resolver = "3"` in your `[package]` (as the
> [Quick Start](quick-start.md) does), or resolve with
> `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`. Either keeps the build on
> `1.10506.0`, the newest release that builds there.
> See [Extension Anatomy](../concepts/anatomy.md#version-support).
>
> **`default-features = false`** is optional. With the 1.20000 bindings it leaves out the
> HTTP, TLS and download tooling their default features compile: `cargo tree` lists 65 crates
> for a bare `libduckdb-sys` dependency with the defaults and 30 without them. With the
> 1.10506 bindings the difference is two crates, because that tooling is not optional
> there. Write it in your own manifest: a `features = [...]` list elsewhere in the graph
> turns the defaults back on. `cargo test` with `bundled-test` or `bundled-test-prebuilt` does
> so too, because the `duckdb` crate asks for them; a plain `cargo build --release` of the
> extension does not.

### Build requirements

| What you build | What it needs |
|----------------|---------------|
| The extension (`cargo build --release`) | Rust 1.86 or newer. A C compiler may be needed: the 1.10506 bindings, and the 1.20000 bindings with their default features, have a build dependency (`ring`, through the download tooling) that compiles C. `default-features = false` with the 1.20000 bindings removes it. |
| `append_metadata` (`cargo install quack-rs --bin append_metadata`) | Rust 1.88 or newer, because `cargo install` resolves the newest `libduckdb-sys`. |
| Tests with `bundled-test` | A C++ toolchain; DuckDB compiles from source, about 5 to 10 minutes cold. |
| Tests with `bundled-test-prebuilt` | A prebuilt libduckdb (`DUCKDB_DOWNLOAD_LIB=1` or `DUCKDB_LIB_DIR`) and a C compiler for the small shim that fills the dispatch table (a C++ compiler when the library comes from a DuckDB 1.4 release zip). |
| The community-extension flow (`make configure release test`, [scaffold](scaffold.md)) | Python 3 with `venv`, `pip` and network access to PyPI and GitHub, `git` (with a user identity set, for the first commit), and `make`. |

Upgrading an extension from quack-rs 0.16? 0.18 has breaking changes; see
[Upgrading from 0.16 to 0.18](upgrading-0-18.md) for what to change. From 0.18 or 0.19,
see [Upgrading from 0.18 to 0.20](upgrading-0-20.md).

---

## Cargo features

The default build uses only the C API's stable prefix and works with every DuckDB
release in the `>=1.4.4, <1.20100` range. Each feature below unlocks more API and raises
the `libduckdb-sys` version Cargo must resolve. The floors are the ones in the feature
documentation in quack-rs's `Cargo.toml`.

| Feature | Requires | Enables |
|---------|----------|---------|
| `duckdb-1-5` | `libduckdb-sys >= 1.10500.0` (DuckDB 1.5.0+) | Scalar bind/init callbacks, [config options](../duckdb-1-5/config-options.md), [copy functions](../functions/copy-functions.md), [catalog and client context](../duckdb-1-5/catalog.md), structured errors, bound expressions, the virtual file system, selection vectors and [vector operations](../duckdb-1-5/vector-ops.md), the instance cache, and more |
| `duckdb-1-5-3` | `libduckdb-sys >= 1.10503.0` (DuckDB 1.5.3) | `TypeId::Variant` and `TypeId::Geometry`. Implies `duckdb-1-5` |
| `duckdb-1-5-4` | `libduckdb-sys >= 1.10504.0`; a DuckDB 1.5.0+ engine at run time | The [Arrow interop](../duckdb-1-5/arrow.md) bridge. Implies `duckdb-1-5-3` |
| `bundled-test` | No floor beyond the crate's own range | `testing::InMemoryDb` for `cargo test`, by compiling DuckDB from C++ source (the `duckdb` crate's `bundled` feature) |
| `bundled-test-prebuilt` | `libduckdb-sys >= 1.10503` to find the header directory by itself; older versions need `DUCKDB_INCLUDE_DIR`. The headers must include `duckdb_extension.h` or `duckdb.hpp` | The same as `bundled-test`, but links a prebuilt libduckdb (set `DUCKDB_DOWNLOAD_LIB=1` or `DUCKDB_LIB_DIR`; a 1.x `libduckdb-<platform>.zip` or a 2.0 `duckdb-shared-libs-<platform>.tar.gz`, extracted) |

The DuckDB version features are cumulative: `duckdb-1-5-4` turns on the other two. There
is no `duckdb-1-5-5` or `duckdb-1-5-6` feature, because neither release adds C API
functions. Enabling a version feature against an older `libduckdb-sys` is a compile error,
by design.

Use the test features only under `[dev-dependencies]`; see
[Development dependencies](#development-dependencies) below. If both are enabled, the
source build (`bundled-test`) wins.

---

## Required Cargo.toml settings

Every DuckDB extension requires specific Cargo settings to link and behave correctly:

```toml
[lib]
name = "my_extension"       # ← must match extension name exactly (Pitfall P1)
crate-type = ["cdylib", "rlib"]
#             ^^^^^^  cdylib produces the .so/.dylib/.dll DuckDB loads
#                      rlib   optional: lets doctests, examples and tests/ link the crate

[profile.release]
panic = "unwind"            # REQUIRED — quack-rs catches panics at every FFI boundary;
                            #   "abort" makes that impossible (see below)
lto = true                  # recommended — reduces binary size, improves performance
opt-level = 3               # recommended
codegen-units = 1           # recommended — better optimisation, slower build
strip = true                # recommended — reduces binary size
```

Only `cdylib` is required. Keep `rlib` in the list if your integration tests in `tests/`,
your examples or your doctests call the crate's own functions; without it they cannot
link it. If a generated project lists only `cdylib`, add `rlib` when you need it.

### Why `panic = "unwind"`, not `"abort"`?

Every callback quack-rs generates (the `*_callback!` macros, the closure-based builders,
`FfiState`'s callbacks), and every entry point, runs your code inside
`std::panic::catch_unwind` and turns a panic into an ordinary SQL error that DuckDB
reports to the user. `catch_unwind` can only catch a panic that **unwinds**: under
`panic = "abort"` the process terminates at the panic site, before any guard runs, taking
the user's whole DuckDB session with it.

A raw `unsafe extern "C" fn` that you pass to a builder yourself (such as the callbacks in
[Scalar Functions](../functions/scalar.md)) is installed as written, with no guard. Define it with the
matching macro (`scalar_callback!`, `aggregate_update_callback!`, …), which reports a panic
as a SQL error, or wrap its body in `quack_rs::callback::catch_ffi_panic` and report the
`Err` it returns yourself.

(A panic that escapes an `extern "C"` function without being caught is not undefined
behaviour on Rust ≥ 1.81 — the runtime aborts the process — but that is exactly the
outcome the guards exist to prevent.)

`validate_release_profile` rejects `panic = "abort"`, and the scaffold generator emits
`panic = "unwind"`.

---

## Minimum Supported Rust Version

quack-rs requires **Rust ≥ 1.86.0**.

While DuckDB 1.x is built, 1.86.0 is a ceiling as much as a floor. The
community-extension build workflow (`_extension_distribution.yml` in
`duckdb/extension-ci-tools`) pins Rust **1.86.0** for its WebAssembly jobs on the
`v1.4-andium` and `v1.5-variegata` branches, so an extension, and therefore
quack-rs, must build on it; CI's `msrv-vs-duckdb-ci` job re-derives that pin
and fails if the MSRV rises above it. (The 2.0 branch, `v2.0-cyanoptera`, pins
1.97.1 for Windows and WebAssembly.) The `msrv` job checks the crate with
`cargo +1.86.0 check`, and the benchmark dev-dependency (`criterion`) needs
1.86 as well.

Install or update via:

```bash
rustup update stable
rustup default stable
```

Verify:

```bash
rustc --version   # must be ≥ 1.86.0
```

---

## Development dependencies

To run SQL against your functions inside `cargo test`, enable one of quack-rs's test
features as a dev-dependency:

```toml
[dev-dependencies]
# Compiles DuckDB from C++ source: no setup, slow cold build.
quack-rs = { version = "0.20", features = ["bundled-test"] }
# ...or link a prebuilt libduckdb instead (set DUCKDB_DOWNLOAD_LIB=1 or DUCKDB_LIB_DIR):
# quack-rs = { version = "0.20", features = ["bundled-test-prebuilt"] }
```

Either one initialises the `loadable-extension` dispatch table from the linked DuckDB, so
`testing::InMemoryDb` works and the whole C API — including your own registration code — can
be exercised in a test. Without them, any `duckdb_*` call in a `cargo test` process panics,
because nothing has filled the dispatch table. See the [Testing Guide](../testing.md).

---

## Starting a new extension from scratch

Use the [scaffold generator](scaffold.md) to produce a complete project with these settings,
the build files and CI already in place.
