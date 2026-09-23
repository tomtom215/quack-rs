# Installation

## Adding quack-rs to an existing extension

Add the following to your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.13"
libduckdb-sys = { version = ">=1.4.4, <2", features = ["loadable-extension"] }
```

> **Why `>=1.4.4, <2`?**
> DuckDB 1.4.x and 1.5.x expose the same C API version (`v1.2.0`), so `quack-rs` supports
> both with a single bounded range. The `<2` upper bound prevents silent adoption of a future
> major release whose C API may change in breaking ways — making any such upgrade an explicit,
> auditable decision. See [Extension Anatomy](../concepts/anatomy.md#version-support).

---

## Required Cargo.toml settings

Every DuckDB extension requires specific Cargo settings to link and behave correctly:

```toml
[lib]
name = "my_extension"       # ← must match extension name exactly (Pitfall P1)
crate-type = ["cdylib", "rlib"]
#             ^^^^^^  cdylib produces the .so/.dylib/.dll DuckDB loads
#                      rlib  allows unit tests and documentation to work

[profile.release]
panic = "unwind"            # REQUIRED — quack-rs catches panics at every FFI boundary;
                            #   "abort" makes that impossible (see below)
lto = true                  # recommended — reduces binary size, improves performance
opt-level = 3               # recommended
codegen-units = 1           # recommended — enables full LTO
strip = true                # recommended — reduces binary size
```

### Why `panic = "unwind"`, not `"abort"`?

Every callback quack-rs generates, and every entry point, runs your code inside
`std::panic::catch_unwind`, and turns a panic into an ordinary SQL error that DuckDB
reports to the user. `catch_unwind` can only catch a panic that **unwinds**: under
`panic = "abort"` the process terminates at the panic site, before any guard runs, taking
the user's whole DuckDB session with it.

(A panic that escapes an `extern "C"` function without being caught is not undefined
behaviour on Rust ≥ 1.81 — the runtime aborts the process — but that is exactly the
outcome the guards exist to prevent.)

`validate_release_profile` rejects `panic = "abort"`, and the scaffold generator emits
`panic = "unwind"`.

---

## Minimum Supported Rust Version

quack-rs requires **Rust ≥ 1.86.0**.

This MSRV is required for:
- `&raw mut expr` syntax for creating raw pointers without references (sound and stable since 1.84.0)
- `const extern fn` support

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
quack-rs = { version = "0.18", features = ["bundled-test"] }
# ...or link a prebuilt libduckdb instead (set DUCKDB_DOWNLOAD_LIB=1 or DUCKDB_LIB_DIR):
# quack-rs = { version = "0.18", features = ["bundled-test-prebuilt"] }
```

Either one initialises the `loadable-extension` dispatch table from the linked DuckDB, so
`testing::InMemoryDb` works and the whole C API — including your own registration code — can
be exercised in a test. Without them, any `duckdb_*` call in a `cargo test` process panics,
because nothing has filled the dispatch table. See the [Testing Guide](../testing.md).

---

## Starting a new extension from scratch

Use the [scaffold generator](scaffold.md) to produce a complete project with all required
files pre-configured. This is the fastest and most reliable way to start a new extension.
