# The Entry Point

Every DuckDB extension must export a single C-callable symbol that DuckDB invokes at load time.
quack-rs provides two ways to create it.

---

## Option A: `entry_point_v2!` with `Connection` (recommended)

*Added in v0.4.0.*

The `entry_point_v2!` macro gives your closure a `&Connection` instead of a raw
`duckdb_connection`. The `Connection` type implements the `Registrar` trait, which
provides ergonomic methods for registering every function type:

```rust
use quack_rs::entry_point_v2;
use quack_rs::connection::{Connection, Registrar};
use quack_rs::error::ExtensionError;

unsafe fn register(con: &Connection) -> Result<(), ExtensionError> {
    unsafe {
        con.register_scalar(/* ScalarFunctionBuilder */)?;
        con.register_aggregate(/* AggregateFunctionBuilder */)?;
        con.register_table(/* TableFunctionBuilder */)?;
        con.register_cast(/* CastFunctionBuilder */)?;
        con.register_scalar_set(/* ScalarFunctionSetBuilder */)?;
        con.register_aggregate_set(/* AggregateFunctionSetBuilder */)?;
        con.register_sql_macro(/* SqlMacro */)?;
        con.register_replacement_scan(/* callback, data, destructor */);
    }
    Ok(())
}

entry_point_v2!(my_extension_init_c_api, |con| register(con));
```

This emits:

```rust
#[no_mangle]
pub unsafe extern "C" fn my_extension_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    unsafe {
        quack_rs::entry_point::init_extension_v2(
            info, access, quack_rs::DUCKDB_API_VERSION,
            |con| register(con),
        )
    }
}
```

Pass the **full symbol name** to the macro. The symbol `{name}_init_c_api` must match the
`name` field in `description.yml` and the `[lib] name` in `Cargo.toml`.

### Why `Connection` over raw `duckdb_connection`?

| Feature | `entry_point!` (raw) | `entry_point_v2!` (Connection) |
|---------|---------------------|-------------------------------|
| Receives | `duckdb_connection` | `&Connection` |
| Registration | Call builders' `.register(con)` | Call `con.register_*()` |
| Type safety | Raw pointer | Wrapper with lifetime |
| Future-proofing | Tied to C pointer | Can evolve without breaking extensions |

---

## Option B: The `entry_point!` macro

The original macro passes a raw `duckdb_connection` to your closure. It works
identically but requires you to pass the connection to each builder's `.register()`:

```rust
use quack_rs::entry_point;
use quack_rs::error::ExtensionError;

fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        // register your functions here
        Ok(())
    }
}

entry_point!(my_extension_init_c_api, |con| register(con));
```

---

## Option C: Manual entry point

If you need full control (e.g., multiple registration functions, conditional logic):

```rust
use quack_rs::entry_point::init_extension;
use libduckdb_sys::{duckdb_extension_info, duckdb_extension_access};

#[no_mangle]
pub unsafe extern "C" fn my_extension_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    unsafe {
        init_extension(info, access, quack_rs::DUCKDB_API_VERSION, |con| {
            register_scalar_functions(con)?;
            register_aggregate_functions(con)?;
            register_sql_macros(con)?;
            Ok(())
        })
    }
}
```

---

## What `init_extension` does

```mermaid
flowchart TD
    A["**1. duckdb_rs_extension_api_init**(info, access, version)<br/>Fills the global AtomicPtr dispatch table"]
    B["**2. access.get_database**(info)<br/>Returns the duckdb_database handle"]
    C["**3. duckdb_connect**(db, &amp;mut con)<br/>Opens a connection for function registration"]
    D["**4. register**(con) ← your closure"]
    E["**5. duckdb_disconnect**(&amp;mut con)<br/>Always runs, even if registration failed"]
    F{Error?}
    G["return **true**"]
    H["return **false**<br/>error reported via access.set_error"]

    A --> B --> C --> D --> E --> F
    F -->|no| G
    F -->|yes| H

    style G fill:#1c3b1c,stroke:#4a9e4a,color:#c8ecc8
    style H fill:#3b1c1c,stroke:#9e4a4a,color:#ecc8c8
```

Errors from step 4 are reported back to DuckDB via `access.set_error` and the function
returns `false`. DuckDB then surfaces the error message to the user.

---

## The C API version constant

```rust
pub const DUCKDB_API_VERSION: &str = "v1.2.0";
```

> **Pitfall P2**: This is the **C API version**, not the DuckDB release version.
> DuckDB 1.4.4 uses C API version `v1.2.0`. Passing the wrong string causes the metadata
> script to fail or produce incorrect metadata.
> See [Pitfall P2](../reference/pitfalls.md#p2-extension-metadata-version-is-c-api-version).

---

## No panics in the entry point

`init_extension` never panics. All error paths use `Result` and `?`. If your registration
closure returns `Err`, the error message is reported to DuckDB via `access.set_error` and
the extension fails to load gracefully.

Never use `unwrap()` or `expect()` in FFI callbacks.
See [Pitfall L3](../reference/pitfalls.md#l3-no-panic-across-ffi-boundaries).
