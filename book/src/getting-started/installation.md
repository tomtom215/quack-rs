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
panic = "unwind"            # REQUIRED — quack-rs catches panics; catching needs unwinding
lto = true                  # recommended — reduces binary size, improves performance
opt-level = 3               # recommended
codegen-units = 1           # recommended — enables full LTO
strip = true                # recommended — reduces binary size
```

### Why `panic = "unwind"`?

quack-rs wraps every `extern "C"` entry point — the extension entry point and every
scalar/table/aggregate/cast/copy callback macro — in `std::panic::catch_unwind`, so a panic
in your code becomes a DuckDB error message instead of a crash.

**`catch_unwind` cannot catch anything under `panic = "abort"`.** The runtime aborts before
unwinding starts, so the process dies with `SIGABRT` and takes the user's DuckDB session
with it. Setting `abort` therefore disables every panic guard quack-rs provides:

```text
$ rustc -O panic_probe.rs && ./panic_probe
catch_unwind returned: true
process survived the panic          exit=0

$ rustc -O -C panic=abort panic_probe.rs && ./panic_probe
Aborted                             exit=134
```

The older advice to set `abort` came from panics escaping an `extern "C"` boundary once
being undefined behavior. They no longer are — Rust defines that as an abort — and quack-rs
catches them before the boundary anyway, which is the whole point.

`validate_release_profile` enforces this; see [Publishing](../publishing.md).

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

For testing with a live DuckDB instance (example-extension tests only):

```toml
[dev-dependencies]
duckdb = { version = ">=1.4.4, <2", features = ["bundled"] }
```

> **Important**: you cannot call any `duckdb_*` function in a `cargo test` process when using
> the `loadable-extension` feature. See [Testing Guide](../testing.md) for the full explanation.

---

## Starting a new extension from scratch

Use the [scaffold generator](scaffold.md) to produce a complete project with all required
files pre-configured. This is the fastest and most reliable way to start a new extension.
