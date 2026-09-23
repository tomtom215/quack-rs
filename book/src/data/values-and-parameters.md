# Values & Parameter Extraction

When a table function receives bind-time parameters, DuckDB passes them as
`duckdb_value` handles. These handles are heap-allocated and must be destroyed
after use. The `Value` wrapper handles this automatically via RAII.

---

## The problem

Without `Value`, every parameter extraction requires three raw FFI calls and
careful manual cleanup:

```rust
// Before: raw FFI — easy to leak memory, and `duckdb_get_int64` aborts the
// process if the argument is SQL NULL (DuckDB throws a C++ exception).
let mut param = unsafe { duckdb_bind_get_parameter(info, 0) };
let n = unsafe { duckdb_get_int64(param) };
unsafe { duckdb_destroy_value(&mut param) };  // forget this → memory leak
```

## The solution: `Value`

`Value` wraps a `duckdb_value` handle and calls `duckdb_destroy_value` on drop:

```rust
use quack_rs::table::BindInfo;

unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
    let bind_info = unsafe { BindInfo::new(info) };

    // Value is RAII — automatically destroyed when dropped.
    // `as_i64` is `None` for SQL NULL; `as_i64_or` supplies a default.
    let n = unsafe { bind_info.get_parameter_value(0) }.as_i64_or(0);

    // Named parameters work the same way
    let path = unsafe { bind_info.get_named_parameter_value("path") }
        .as_str()
        .unwrap_or_default();
}
```

## Typed extraction methods

| Method | Reads as | Rust type |
|--------|----------|-----------|
| `as_str()` | any scalar, rendered as text | `Result<String, ExtensionError>` |
| `as_blob()` | BLOB only | `Result<Vec<u8>, ExtensionError>` |
| `as_i8()` | TINYINT | `Option<i8>` |
| `as_i16()` | SMALLINT | `Option<i16>` |
| `as_i32()` | INTEGER | `Option<i32>` |
| `as_i64()` | BIGINT | `Option<i64>` |
| `as_i128()` | HUGEINT | `Option<i128>` |
| `as_u8()` | UTINYINT | `Option<u8>` |
| `as_u16()` | USMALLINT | `Option<u16>` |
| `as_u32()` | UINTEGER | `Option<u32>` |
| `as_u64()` | UBIGINT | `Option<u64>` |
| `as_u128()` | UHUGEINT | `Option<u128>` |
| `as_f32()` | FLOAT | `Option<f32>` |
| `as_f64()` | DOUBLE | `Option<f64>` |
| `as_bool()` | BOOLEAN | `Option<bool>` |
| `as_date()`, `as_time()`, `as_timestamp()`, … | the temporal types | `Option<i32>` / `Option<i64>` / … |
| `as_interval()`, `as_uuid()` | INTERVAL, UUID | `Option<DuckInterval>`, `Option<u128>` |
| `as_decimal()` | DECIMAL only (no cast) | `Option<Decimal>` |
| `as_enum_index()` | ENUM only (no cast) | `Option<u64>` |

The scalar getters cast the way SQL's `TRY_CAST` does: a `VARCHAR` `'42'`
reads as `Some(42)` through `as_i64()`, a `DOUBLE` `1.5` as `Some(2)` through
`as_i32()`. They return `None` when:

- the value is SQL `NULL` (for example `my_func(n := NULL)`),
- the handle is null (a named parameter the caller did not supply),
- the type is not a scalar (`LIST`, `STRUCT`, `MAP`, `BLOB`, `ENUM`, …), or
- the cast fails (`'abc'`, or a number out of range for the target).

No getter calls into DuckDB in the first three cases — DuckDB's own
`duckdb_get_*` functions abort the process on a SQL `NULL` and crash on a null
handle — and none modifies the value it reads (DuckDB's getters cast the value
in place; quack-rs reads from a copy).

`as_blob()` copies the bytes into an owned `Vec<u8>` without UTF-8 validation.
It accepts only a `BLOB`: DuckDB's conversion of anything else to `BLOB` can
throw. Use `as_str()` for text.

```rust
let bytes = unsafe { bind_info.get_parameter_value(0) }.as_blob()?;
```

### Defaulting variants

The integer, float, bool and string getters have an `_or(default)` variant
that returns `default` wherever the plain getter returns `None` (or `Err`):

```rust
let timeout = val.as_i64_or(30);       // 30 if NULL, absent, or not a number
let host = val.as_str_or("localhost");  // "localhost" if NULL or absent
let port = val.as_u16_or(5432);        // 5432 if NULL, absent, or out of range
```

## Checking for NULL

```rust
let val = unsafe { bind_info.get_named_parameter_value("limit") };
if val.is_null() {
    // the handle is null: the named parameter was not provided
}
if val.is_sql_null() {
    // the parameter was provided as SQL NULL
}
```

## Escape hatch

If you need the raw handle for an API not yet wrapped:

```rust
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
