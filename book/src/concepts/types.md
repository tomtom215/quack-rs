# Type System

quack-rs provides `TypeId` and `LogicalType` to bridge Rust types and DuckDB column types.

---

## `TypeId`

`TypeId` is an ergonomic enum covering all DuckDB column types:

```rust
use quack_rs::types::TypeId;

TypeId::Boolean
TypeId::TinyInt     // i8
TypeId::SmallInt    // i16
TypeId::Integer     // i32
TypeId::BigInt      // i64
TypeId::UTinyInt    // u8
TypeId::USmallInt   // u16
TypeId::UInteger    // u32
TypeId::UBigInt     // u64
TypeId::HugeInt     // i128
TypeId::Float       // f32
TypeId::Double      // f64
TypeId::Timestamp
TypeId::TimestampTz
TypeId::Date
TypeId::Time
TypeId::Interval
TypeId::Varchar
TypeId::Blob
TypeId::Uuid
TypeId::List
```

`TypeId` is `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, and `Display`.

### SQL name

```rust
assert_eq!(TypeId::BigInt.sql_name(), "BIGINT");
assert_eq!(TypeId::Varchar.sql_name(), "VARCHAR");
assert_eq!(format!("{}", TypeId::Timestamp), "TIMESTAMP");
```

### DuckDB constant

`TypeId::to_duckdb_type()` returns the `DUCKDB_TYPE_*` integer constant from `libduckdb-sys`.
You rarely need this directly — it's called internally by `LogicalType::new`.

---

## `LogicalType`

`LogicalType` is a RAII wrapper around DuckDB's `duckdb_logical_type`. It is used internally
by the function builders.

```rust
use quack_rs::types::{LogicalType, TypeId};

let lt = LogicalType::new(TypeId::Varchar);
// lt.as_raw() returns the duckdb_logical_type pointer
// Drop calls duckdb_destroy_logical_type automatically
```

> **Pitfall L7**: `duckdb_create_logical_type` allocates memory that must be freed with
> `duckdb_destroy_logical_type`. `LogicalType`'s `Drop` implementation does this automatically,
> preventing the memory leak that occurs when calling the DuckDB C API directly.
> See [Pitfall L7](../reference/pitfalls.md#l7-logicaltype-memory-leak).

You almost never need to create `LogicalType` directly. The function builders
(`ScalarFunctionBuilder`, `AggregateFunctionBuilder`) create and destroy them internally.

---

## Rust type ↔ DuckDB type mapping

When reading from or writing to vectors, use the corresponding `VectorReader`/`VectorWriter`
method:

| DuckDB type | `TypeId` | Reader method | Writer method |
|-------------|----------|---------------|---------------|
| `BOOLEAN` | `Boolean` | `read_bool` | `write_bool` |
| `TINYINT` | `TinyInt` | `read_i8` | `write_i8` |
| `SMALLINT` | `SmallInt` | `read_i16` | `write_i16` |
| `INTEGER` | `Integer` | `read_i32` | `write_i32` |
| `BIGINT` | `BigInt` | `read_i64` | `write_i64` |
| `UTINYINT` | `UTinyInt` | `read_u8` | `write_u8` |
| `USMALLINT` | `USmallInt` | `read_u16` | `write_u16` |
| `UINTEGER` | `UInteger` | `read_u32` | `write_u32` |
| `UBIGINT` | `UBigInt` | `read_u64` | `write_u64` |
| `FLOAT` | `Float` | `read_f32` | `write_f32` |
| `DOUBLE` | `Double` | `read_f64` | `write_f64` |
| `VARCHAR` | `Varchar` | `read_str` | `write_varchar` |
| `INTERVAL` | `Interval` | `read_interval` | — |

NULLs are handled separately — see [NULL Handling & Strings](../data/nulls-and-strings.md).
