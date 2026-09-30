# Values & Parameter Extraction

DuckDB hands an extension single values — a table function's bind-time
parameters, the options of a `COPY` statement, a folded constant expression — as
`duckdb_value` handles, which are heap-allocated and must be destroyed after
use. quack-rs wraps them in `Value`, which destroys the handle on drop and reads
it through typed getters that return `Option` instead of aborting on SQL `NULL`.
This page covers reading parameters with `Value`, building values, and nested
(`LIST`, `STRUCT`, `MAP`) values.

---

## The problem

Without `Value`, every parameter extraction requires three raw FFI calls and
careful manual cleanup:

```rust
# use libduckdb_sys::*;
# unsafe fn demo(info: duckdb_bind_info) {
// Before: raw FFI — easy to leak memory, and `duckdb_get_int64` aborts the
// process if the argument is SQL NULL (DuckDB throws a C++ exception).
let mut param = unsafe { duckdb_bind_get_parameter(info, 0) };
let n = unsafe { duckdb_get_int64(param) };
unsafe { duckdb_destroy_value(&mut param) };  // forget this → memory leak
# }
```

## The solution: `Value`

`Value` wraps a `duckdb_value` handle and calls `duckdb_destroy_value` on drop:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
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
| `as_str()` | any value, cast to text | `Result<String, ExtensionError>` |
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
| `as_date()`, `as_time()`, `as_time_tz()`, `as_timestamp()`, … | the temporal types | `Option<i32>` / `Option<i64>` / `Option<u64>` |
| `as_interval()`, `as_uuid()` | INTERVAL, UUID | `Option<DuckInterval>`, `Option<u128>` |
| `as_decimal()` | DECIMAL only (no cast) | `Option<Decimal>` |
| `as_enum_index()` | ENUM only (no cast) | `Option<u64>` |

The scalar getters cast the way SQL's `TRY_CAST` does: a `VARCHAR` `'42'`
reads as `Some(42)` through `as_i64()`, a `DOUBLE` `1.5` as `Some(2)` through
`as_i32()`. They return `None` when:

- the value is SQL `NULL` (for example `my_func(n := NULL)`),
- the handle is null (a named parameter the caller did not supply),
- the type is not a scalar (`LIST`, `STRUCT`, `MAP`, `BLOB`, `ENUM`, …), or
- the cast fails (`'abc'`, or a number out of range for the target), or
- for the temporal getters, SQL would refuse the conversion: the time of an
  infinite timestamp (`as_time()` of `'infinity'::TIMESTAMP`), a `TIMESTAMP`
  outside `TIMESTAMP_NS`'s 1677–2262 range read with `as_timestamp_ns()`, or
  a result outside the target type's range.

No getter calls a `duckdb_get_*` function in the first three cases — those
functions abort the process on a SQL `NULL` and crash on a null handle — and
none modifies the value it reads (DuckDB's getters cast the value in place;
quack-rs reads from a copy). The temporal cases are checked before
the call too: DuckDB converts those pairs with a cast that throws a C++
exception instead of failing, which aborts the process from Rust.

### Building values

`Value::bigint`, `Value::varchar`, `Value::date`, `Value::interval` and the
other scalar constructors are infallible, with two exceptions that return
`Result<Value, ExtensionError>`:

- The temporal constructors that take a raw 64-bit payload — `time`,
  `time_tz`, `time_ns` (with `duckdb-1-5`), `timestamp`, `timestamp_tz`,
  `timestamp_s`, `timestamp_ms` and `timestamp_ns`. DuckDB stores any payload
  unchecked, and rendering or casting an out-of-range one aborts, crashes or
  prints garbage, so quack-rs accepts exactly the range DuckDB's SQL produces
  (`TIME` `00:00:00`–`24:00:00`, the `TIMESTAMP` span
  `290309-12-22 (BC)`–`294247-01-10` plus `±infinity`, and so on) and returns
  an error otherwise.
- `Value::decimal(width, scale, unscaled)`, which checks that `width` is
  `1..=38`, `scale <= width` and `unscaled` has at most `width` digits.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# // Values are DuckDB objects: fill the dispatch table first.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
let noon = Value::time(12 * 3_600 * 1_000_000)?;
assert!(Value::time(-1).is_err());
# assert_eq!(noon.as_time(), Some(12 * 3_600 * 1_000_000));
# Ok::<(), ExtensionError>(())
```

`as_blob()` copies the bytes into an owned `Vec<u8>` without UTF-8 validation.
It accepts only a `BLOB`: DuckDB's conversion of anything else to `BLOB` can
throw. Use `as_str()` for text.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(bind_info: BindInfo) -> Result<(), ExtensionError> {
let bytes = unsafe { bind_info.get_parameter_value(0) }.as_blob()?;
# Ok(())
# }
```

### Defaulting variants

The integer (except `as_u128`), float, bool and string getters have an
`_or(default)` variant that returns `default` wherever the plain getter returns
`None` (or `Err`); `as_str_or_default()` returns an empty string:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# // Values are DuckDB objects: fill the dispatch table first.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let val = Value::null_value();
let timeout = val.as_i64_or(30);       // 30 if NULL, absent, or not a number
let host = val.as_str_or("localhost");  // "localhost" if NULL or absent
let port = val.as_u16_or(5432);        // 5432 if NULL, absent, or out of range
# assert_eq!((timeout, host.as_str(), port), (30, "localhost", 5432));
```

## Checking for NULL

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(bind_info: BindInfo) {
let val = unsafe { bind_info.get_named_parameter_value("limit") };
if val.is_null() {
    // the handle is null: the named parameter was not provided
}
if val.is_sql_null() {
    // the parameter was provided as SQL NULL
}
# }
```

## Escape hatch

If you need the raw handle for an API quack-rs does not wrap, `as_raw()` borrows
it and `into_raw()` takes ownership:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# use libduckdb_sys::duckdb_value;
# unsafe fn demo(bind_info: BindInfo) {
let val = unsafe { bind_info.get_parameter_value(0) };
let raw: duckdb_value = val.into_raw();  // takes ownership, no auto-destroy
// ... use raw handle ...
// caller must call duckdb_destroy_value manually
# }
```

## Nested values

A parameter of type `LIST`, `STRUCT` or `MAP` is read element by element; each
accessor that returns a `Value` returns an owned one.

| Method | Returns |
|--------|---------|
| `list_len()` | Number of elements of a `LIST` (0 for any other type) |
| `list_child(i)` / `list_items()` | Element `i` (`Option<Value>`) / all elements (`Vec<Value>`) |
| `struct_field_names()` | Field names of a `STRUCT`, in order |
| `struct_child(i)` | Field `i` of a `STRUCT` (`Option<Value>`); fields are positional |
| `map_len()`, `map_key(i)`, `map_value(i)` | Number of pairs; key / value of pair `i` |

```rust
# use quack_rs::value::Value;
# fn demo(options: &Value) -> Option<String> {
let names = options.struct_field_names();
let idx = names.iter().position(|n| n == "compression")?;
options.struct_child(idx)?.as_str().ok()
# }
```

To build one, `Value::list_value` and `Value::array_value` take the **element**
type and the items, `Value::struct_value` the `STRUCT` type and one value per
field, and `Value::enum_value` the `ENUM` type and an index; `Value::map` and
`Value::union_value` need `duckdb-1-5`. All return `Result<Value, ExtensionError>`.

`DataChunk`, which wraps the chunk a scan callback writes its output to, is
described in [Reading & Writing Vectors](vectors.md#datachunk).
