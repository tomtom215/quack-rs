# Quick Start

This page takes you from an empty crate to a DuckDB loadable extension written in Rust,
loaded into the DuckDB CLI, in three steps.

---

## Prerequisites

- Rust ≥ 1.86.0 (MSRV) — install via [rustup](https://rustup.rs/)
- DuckDB CLI (for testing the built extension) — [download](https://duckdb.org/docs/installation/)

---

## Step 1 — Add quack-rs to your extension

In your extension's `Cargo.toml`:

```toml
[package]
name = "my_extension"
version = "0.1.0"
edition = "2021"
rust-version = "1.86"       # with resolver 3, Cargo picks dependency versions that build on it
resolver = "3"

[dependencies]
quack-rs = "0.19"
libduckdb-sys = { version = ">=1.4.4, <2", features = ["loadable-extension"] }

[lib]
name = "my_extension"       # must match your extension name — see Pitfall P1
crate-type = ["cdylib", "rlib"]

[profile.release]
panic = "unwind"            # required — quack-rs's panic guards need unwinding (see Installation)
lto = true
opt-level = 3
codegen-units = 1
strip = true
```

> **Starting from scratch?** The [scaffold generator](scaffold.md) generates a complete
> community extension project, including this `Cargo.toml`.

---

## Step 2 — Write the extension

```rust
// src/lib.rs
use libduckdb_sys::duckdb_connection;
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;

fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    // Parameter and return types come from the closure signature, and a NULL
    // argument gives a NULL result without calling the closure. `try_map1`
    // lets the closure fail: a plain `x * 2` wraps on overflow in a release
    // build, and an `Err` here becomes a SQL error instead.
    let double_it = ScalarFunctionBuilder::try_map1("double_it", |x: i64| {
        x.checked_mul(2).ok_or("double_it: BIGINT overflow")
    })?;

    // SAFETY: `con` is the live connection DuckDB passed to the entry point.
    unsafe { double_it.register(con) }?;
    Ok(())
}

// The symbol must be `<extension name>_init_c_api`.
entry_point!(my_extension_init_c_api, |con| register(con));
```

The closure is guarded: a panic in it becomes a SQL error, not a crashed DuckDB
process. When a function needs more than a closure gives it — several
overloads, bind-time state, nested types — write the callback yourself, as
[Scalar Functions](../functions/scalar.md) shows, and guard it with
[a panic-safe callback macro](../functions/panic-safe-callbacks.md).

---

## Step 3 — Build and test

```bash
# Build the extension
cargo build --release

# DuckDB refuses to LOAD a bare .so: append the metadata footer first.
# quack-rs ships the `append_metadata` binary for this step. Installing it needs
# Rust 1.88 or newer (it resolves the newest libduckdb-sys); use
# `cargo +stable install` if this project pins an older toolchain.
cargo install quack-rs --bin append_metadata
append_metadata target/release/libmy_extension.so my_extension.duckdb_extension \
    --abi-type C_STRUCT --extension-version v0.1.0 \
    --duckdb-version v1.2.0 --platform linux_amd64

# Load it; -unsigned allows a locally built, unsigned extension.
duckdb -unsigned -c "LOAD './my_extension.duckdb_extension'; SELECT double_it(21);"
# ┌───────────────┐
# │ double_it(21) │
# │     int64     │
# ├───────────────┤
# │            42 │
# └───────────────┘
```

> **macOS**: the library is `libmy_extension.dylib` and the platform
> `osx_arm64` (or `osx_amd64`, opt-in in extension-ci-tools for DuckDB 2.0 and dropped in
> DuckDB 2.1). **Windows**: `my_extension.dll` and
> `windows_amd64`.

---

## What's next?

- Learn how DuckDB calls your extension: [Extension Anatomy](../concepts/anatomy.md)
- Add an aggregate function: [Aggregate Functions](../functions/aggregate.md)
- Add SQL macros without any callbacks: [SQL Macros](../functions/sql-macros.md)
- Generate a complete community extension project: [Project Scaffold](scaffold.md)
