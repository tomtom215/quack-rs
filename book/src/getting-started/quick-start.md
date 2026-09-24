# Quick Start

This page gets you from zero to a working DuckDB extension in three steps.

---

## Prerequisites

- Rust ≥ 1.86.0 (MSRV) — install via [rustup](https://rustup.rs/)
- DuckDB CLI (for testing the built extension) — [download](https://duckdb.org/docs/installation/)

---

## Step 1 — Add quack-rs to your extension

In your extension's `Cargo.toml`:

```toml
[dependencies]
quack-rs = "0.18"
libduckdb-sys = { version = ">=1.4.4, <2", features = ["loadable-extension"] }

[lib]
name = "my_extension"       # must match your extension name — see Pitfall P1
crate-type = ["cdylib", "rlib"]

[profile.release]
panic = "unwind"            # required — lets quack-rs turn panics into SQL errors
lto = true
opt-level = 3
codegen-units = 1
strip = true
```

> **Start fresh?** Use the [scaffold generator](scaffold.md) to generate a complete,
> submission-ready project from code.

---

## Step 2 — Write the extension

```rust
// src/lib.rs
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};
use libduckdb_sys::{duckdb_connection, duckdb_function_info, duckdb_data_chunk, duckdb_vector};

/// Scalar function: double_it(BIGINT) → BIGINT
unsafe extern "C" fn double_it(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    // SAFETY: input is a valid data chunk provided by DuckDB.
    let reader = unsafe { VectorReader::new(input, 0) };
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader.row_count();

    for row in 0..row_count {
        if unsafe { !reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let value = unsafe { reader.read_i64(row) };
        unsafe { writer.write_i64(row, value * 2) };
    }
}

fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        ScalarFunctionBuilder::new("double_it")
            .param(TypeId::BigInt)
            .returns(TypeId::BigInt)
            .function(double_it)
            .register(con)?;
    }
    Ok(())
}

entry_point!(my_extension_init_c_api, |con| register(con));
```

---

## Step 3 — Build and test

```bash
# Build the extension
cargo build --release

# DuckDB refuses to LOAD a bare .so: append the metadata footer first
# (`append_metadata` is a binary in the quack-rs repository; see Publishing).
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
> `osx_arm64` (or `osx_amd64`). **Windows**: `my_extension.dll` and
> `windows_amd64`.

---

## What's next?

- Learn how DuckDB calls your extension: [Extension Anatomy](../concepts/anatomy.md)
- Add an aggregate function: [Aggregate Functions](../functions/aggregate.md)
- Add SQL macros without any callbacks: [SQL Macros](../functions/sql-macros.md)
- Generate a complete community extension project: [Project Scaffold](scaffold.md)
