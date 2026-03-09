# Installation

## Adding quack-rs to an existing extension

Add the following to your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.4"
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
panic = "abort"             # REQUIRED — panics across FFI are undefined behavior
lto = true                  # recommended — reduces binary size, improves performance
opt-level = 3               # recommended
codegen-units = 1           # recommended — enables full LTO
strip = true                # recommended — reduces binary size
```

### Why `panic = "abort"`?

Rust's default panic behavior unwinds the stack. When a panic crosses an FFI boundary into
DuckDB's C++ code, the result is **undefined behavior** — DuckDB may crash, corrupt memory,
or silently produce wrong results. The `panic = "abort"` setting converts panics into
immediate process termination, which is far safer.

`quack-rs` itself never panics in FFI callbacks, but this setting protects you if a
dependency or your own code panics.

---

## Minimum Supported Rust Version

quack-rs requires **Rust ≥ 1.84.1**.

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
rustc --version   # must be ≥ 1.84.1
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
