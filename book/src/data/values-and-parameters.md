# Values & Parameter Extraction

When a table function receives bind-time parameters, DuckDB passes them as
`duckdb_value` handles. These handles are heap-allocated and must be destroyed
after use. The `Value` wrapper handles this automatically via RAII.

---

## The problem

Without `Value`, every parameter extraction requires three raw FFI calls and
careful manual cleanup:

```rust,ignore
// Before: raw FFI — easy to leak memory
let param = duckdb_bind_get_parameter(info, 0);
let n = duckdb_get_int64(param);
duckdb_destroy_value(&mut { param });  // forget this → memory leak
```

## The solution: `Value`

`Value` wraps a `duckdb_value` handle and calls `duckdb_destroy_value` on drop:

```rust
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
use quack_rs::table::BindInfo;

unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
    let bind_info = unsafe { BindInfo::new(info) };

    // Value is RAII — automatically destroyed when dropped
    let n = unsafe { bind_info.get_parameter_value(0) }.as_i64();

    // Named parameters work the same way
    let path = unsafe { bind_info.get_named_parameter_value("path") }
        .as_str()
        .unwrap_or_default();
}
```

## Typed extraction methods

| Method | DuckDB type | Rust type |
|--------|-------------|-----------|
| `as_str()` | VARCHAR | `Result<String, ExtensionError>` |
| `as_blob()` | BLOB | `Result<Vec<u8>, ExtensionError>` |
| `as_i8()` | TINYINT | `i8` |
| `as_i16()` | SMALLINT | `i16` |
| `as_i32()` | INTEGER | `i32` |
| `as_i64()` | BIGINT | `i64` |
| `as_i128()` | HUGEINT | `i128` |
| `as_u8()` | UTINYINT | `u8` |
| `as_u16()` | USMALLINT | `u16` |
| `as_u32()` | UINTEGER | `u32` |
| `as_u64()` | UBIGINT | `u64` |
| `as_f32()` | FLOAT | `f32` |
| `as_f64()` | DOUBLE | `f64` |
| `as_bool()` | BOOLEAN | `bool` |

DuckDB will attempt to cast the value to the requested type. If the cast fails,
numeric methods return `0` / `0.0` / `false`; `as_str()` and `as_blob()` return
an error if extraction fails.

`as_blob()` copies the bytes into an owned `Vec<u8>` without UTF-8 validation:

```rust,ignore
let bytes = unsafe { bind_info.get_parameter_value(0) }.as_blob()?;
```

### Null-safe convenience variants

Every extraction method has an `_or(default)` variant that returns `default`
when the value handle is null:

```rust,ignore
let timeout = val.as_i64_or(30);       // default 30 if NULL
let host = val.as_str_or("localhost");  // default "localhost" if NULL
let port = val.as_u16_or(5432);        // default 5432 if NULL
```

## Checking for NULL

```rust,ignore
let val = unsafe { bind_info.get_parameter_value(0) };
if val.is_null() {
    // parameter was NULL or not provided
}
```

## Escape hatch

If you need the raw handle for an API not yet wrapped:

```rust,ignore
let val = unsafe { bind_info.get_parameter_value(0) };
let raw: duckdb_value = val.into_raw();  // takes ownership, no auto-destroy
// ... use raw handle ...
// caller must call duckdb_destroy_value manually
```

---

## `DataChunk`

Scan callbacks receive a `duckdb_data_chunk` for output. The `DataChunk` wrapper
provides ergonomic access:

```rust
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
use quack_rs::data_chunk::DataChunk;

unsafe extern "C" fn my_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };

    // Get a writer for column 0
    let mut writer = unsafe { chunk.writer(0) };
    unsafe { writer.write_i64(0, 42) };

    // Set the output row count (0 = end of stream)
    unsafe { chunk.set_size(1) };
}
```

### Methods

| Method | Description |
|--------|-------------|
| `size()` | Current row count |
| `set_size(n)` | Set row count (0 signals end of stream) |
| `column_count()` | Number of columns |
| `vector(col)` | Raw `duckdb_vector` handle |
| `writer(col)` | `VectorWriter` for a column |
| `reader(col)` | `VectorReader` for a column |

`DataChunk` is non-owning — it does not destroy the chunk on drop. DuckDB
manages the chunk's lifetime.
