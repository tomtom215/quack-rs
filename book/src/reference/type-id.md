# TypeId Reference

`quack_rs::types::TypeId` is an ergonomic enum of all DuckDB column types
supported by the builder APIs. It wraps the `DUCKDB_TYPE_*` integer constants
from `libduckdb-sys` and provides safe, named variants.

---

## Full variant table

| Variant | SQL name | libduckdb-sys constant | Notes |
|---------|----------|------------------------|-------|
| `TypeId::Boolean` | `BOOLEAN` | `DUCKDB_TYPE_BOOLEAN` | true/false stored as u8 |
| `TypeId::TinyInt` | `TINYINT` | `DUCKDB_TYPE_TINYINT` | 8-bit signed |
| `TypeId::SmallInt` | `SMALLINT` | `DUCKDB_TYPE_SMALLINT` | 16-bit signed |
| `TypeId::Integer` | `INTEGER` | `DUCKDB_TYPE_INTEGER` | 32-bit signed |
| `TypeId::BigInt` | `BIGINT` | `DUCKDB_TYPE_BIGINT` | 64-bit signed |
| `TypeId::UTinyInt` | `UTINYINT` | `DUCKDB_TYPE_UTINYINT` | 8-bit unsigned |
| `TypeId::USmallInt` | `USMALLINT` | `DUCKDB_TYPE_USMALLINT` | 16-bit unsigned |
| `TypeId::UInteger` | `UINTEGER` | `DUCKDB_TYPE_UINTEGER` | 32-bit unsigned |
| `TypeId::UBigInt` | `UBIGINT` | `DUCKDB_TYPE_UBIGINT` | 64-bit unsigned |
| `TypeId::HugeInt` | `HUGEINT` | `DUCKDB_TYPE_HUGEINT` | 128-bit signed |
| `TypeId::Float` | `FLOAT` | `DUCKDB_TYPE_FLOAT` | 32-bit IEEE 754 |
| `TypeId::Double` | `DOUBLE` | `DUCKDB_TYPE_DOUBLE` | 64-bit IEEE 754 |
| `TypeId::Timestamp` | `TIMESTAMP` | `DUCKDB_TYPE_TIMESTAMP` | µs since Unix epoch |
| `TypeId::TimestampTz` | `TIMESTAMPTZ` | `DUCKDB_TYPE_TIMESTAMP_TZ` | timezone-aware timestamp |
| `TypeId::Date` | `DATE` | `DUCKDB_TYPE_DATE` | days since epoch |
| `TypeId::Time` | `TIME` | `DUCKDB_TYPE_TIME` | µs since midnight |
| `TypeId::Interval` | `INTERVAL` | `DUCKDB_TYPE_INTERVAL` | months + days + µs |
| `TypeId::Varchar` | `VARCHAR` | `DUCKDB_TYPE_VARCHAR` | UTF-8 string |
| `TypeId::Blob` | `BLOB` | `DUCKDB_TYPE_BLOB` | binary data |
| `TypeId::List` | `LIST` | `DUCKDB_TYPE_LIST` | variable-length list |
| `TypeId::Uuid` | `UUID` | `DUCKDB_TYPE_UUID` | 128-bit UUID |

---

## Methods

### `to_duckdb_type() → DUCKDB_TYPE`

Converts to the raw C API integer constant. Used internally by the builder APIs.

```rust
use quack_rs::types::TypeId;

let raw: libduckdb_sys::DUCKDB_TYPE = TypeId::BigInt.to_duckdb_type();
```

### `sql_name() → &'static str`

Returns the SQL type name as a static string.

```rust
assert_eq!(TypeId::BigInt.sql_name(), "BIGINT");
assert_eq!(TypeId::Varchar.sql_name(), "VARCHAR");
assert_eq!(TypeId::TimestampTz.sql_name(), "TIMESTAMPTZ");
```

### `Display`

`TypeId` implements `Display`, which outputs the SQL name:

```rust
println!("{}", TypeId::Interval);  // prints: INTERVAL
let s = format!("{}", TypeId::UBigInt); // "UBIGINT"
```

---

## VectorReader/VectorWriter mapping

The read and write methods on `VectorReader`/`VectorWriter` map to TypeId
variants as follows:

| TypeId | Read method | Write method | Rust type |
|--------|------------|--------------|-----------|
| `Boolean` | `read_bool` | `write_bool` | `bool` |
| `TinyInt` | `read_i8` | `write_i8` | `i8` |
| `SmallInt` | `read_i16` | `write_i16` | `i16` |
| `Integer` | `read_i32` | `write_i32` | `i32` |
| `BigInt` | `read_i64` | `write_i64` | `i64` |
| `UTinyInt` | `read_u8` | `write_u8` | `u8` |
| `USmallInt` | `read_u16` | `write_u16` | `u16` |
| `UInteger` | `read_u32` | `write_u32` | `u32` |
| `UBigInt` | `read_u64` | `write_u64` | `u64` |
| `Float` | `read_f32` | `write_f32` | `f32` |
| `Double` | `read_f64` | `write_f64` | `f64` |
| `Varchar` | `read_str` | `write_varchar` | `&str` |
| `Interval` | `read_interval` | — | `DuckInterval` |

`HugeInt`, `Blob`, `List`, `Uuid`, `Date`, `Time`, `Timestamp`, `TimestampTz`
do not yet have dedicated read/write helpers. Access these via the raw data
pointer from `duckdb_vector_get_data`.

---

## Properties

`TypeId` implements `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, and `Hash`,
making it usable as map keys, set elements, and in match expressions:

```rust
use std::collections::HashMap;
use quack_rs::types::TypeId;

let mut type_names: HashMap<TypeId, &str> = HashMap::new();
type_names.insert(TypeId::BigInt, "count");
type_names.insert(TypeId::Varchar, "label");
```

---

## `#[non_exhaustive]`

`TypeId` is marked `#[non_exhaustive]`. This means future DuckDB versions may
add new variants without it being a breaking change. If you match on `TypeId`,
include a wildcard arm:

```rust
match type_id {
    TypeId::BigInt => { /* ... */ }
    TypeId::Varchar => { /* ... */ }
    _ => { /* handle future types */ }
}
```

---

## `LogicalType`

For types that require runtime parameters (such as `DECIMAL(p, s)` or
parameterized `LIST`), use `quack_rs::types::LogicalType`:

```rust
use quack_rs::types::LogicalType;

let lt = LogicalType::new(TypeId::BigInt);
// LogicalType implements Drop → calls duckdb_destroy_logical_type automatically
```

`LogicalType` wraps `duckdb_logical_type` with RAII cleanup, preventing the
memory leak described in [Pitfall L7](pitfalls.md#l7-logicaltype-memory-leak).
