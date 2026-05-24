# Type System

quack-rs provides `TypeId` and `LogicalType` to bridge Rust types and DuckDB column types.

---

## `TypeId`

`TypeId` is an ergonomic enum covering DuckDB's column types (the `GEOMETRY` and
`VARIANT` types added in DuckDB 1.5.x are exposed behind the `duckdb-1-5-3`
feature — see [Known Limitations](../reference/known-limitations.md)):

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
TypeId::UHugeInt    // u128
TypeId::Float       // f32
TypeId::Double      // f64
TypeId::Timestamp
TypeId::TimestampTz
TypeId::TimestampS
TypeId::TimestampMs
TypeId::TimestampNs
TypeId::Date
TypeId::Time
TypeId::TimeTz
TypeId::Interval
TypeId::Varchar
TypeId::Blob
TypeId::Decimal
TypeId::Enum
TypeId::List
TypeId::Struct
TypeId::Map
TypeId::Uuid
TypeId::Union
TypeId::Bit
TypeId::Array
TypeId::TimeNs      // duckdb-1-5
TypeId::Any              // duckdb-1-5
TypeId::Varint           // duckdb-1-5
TypeId::SqlNull          // duckdb-1-5
TypeId::IntegerLiteral   // duckdb-1-5
TypeId::StringLiteral    // duckdb-1-5
TypeId::Geometry         // duckdb-1-5-3
TypeId::Variant          // duckdb-1-5-3
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

### Reverse conversion

`TypeId::from_duckdb_type(raw)` converts a raw `DUCKDB_TYPE` constant back into a `TypeId`.
Panics if the value does not match any known constant.

```rust
use quack_rs::types::TypeId;

let type_id = TypeId::from_duckdb_type(libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
assert_eq!(type_id, TypeId::BigInt);
```

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

### Constructors

| Constructor | Creates |
|-------------|---------|
| `LogicalType::new(type_id)` | Simple type from a `TypeId` |
| `LogicalType::from_raw(ptr)` | Takes ownership of a raw `duckdb_logical_type` handle (unsafe) |
| `LogicalType::decimal(width, scale)` | `DECIMAL(width, scale)` |
| `LogicalType::list(element_type)` | `LIST<element_type>` from a `TypeId` |
| `LogicalType::list_from_logical(element)` | `LIST<element>` from an existing `LogicalType` |
| `LogicalType::map(key, value)` | `MAP<key, value>` from `TypeId`s |
| `LogicalType::map_from_logical(key, value)` | `MAP<key, value>` from existing `LogicalType`s |
| `LogicalType::struct_type(fields)` | `STRUCT` from `&[(&str, TypeId)]` |
| `LogicalType::struct_type_from_logical(fields)` | `STRUCT` from `&[(&str, LogicalType)]` |
| `LogicalType::union_type(members)` | `UNION` from `&[(&str, TypeId)]` |
| `LogicalType::union_type_from_logical(members)` | `UNION` from `&[(&str, LogicalType)]` |
| `LogicalType::enum_type(members)` | `ENUM` from `&[&str]` |
| `LogicalType::array(element_type, size)` | `ARRAY<element_type>[size]` from a `TypeId` |
| `LogicalType::array_from_logical(element, size)` | `ARRAY<element>[size]` from an existing `LogicalType` |

### Introspection methods

All introspection methods are `unsafe` (require a valid DuckDB runtime handle).

| Method | Returns | Applicable to |
|--------|---------|---------------|
| `get_type_id()` | `TypeId` | Any |
| `get_alias()` | `Option<String>` | Any |
| `set_alias(alias)` | `()` | Any |
| `decimal_width()` | `u8` | `DECIMAL` |
| `decimal_scale()` | `u8` | `DECIMAL` |
| `decimal_internal_type()` | `TypeId` | `DECIMAL` |
| `enum_internal_type()` | `TypeId` | `ENUM` |
| `enum_dictionary_size()` | `u32` | `ENUM` |
| `enum_dictionary_value(index)` | `String` | `ENUM` |
| `list_child_type()` | `LogicalType` | `LIST` |
| `map_key_type()` | `LogicalType` | `MAP` |
| `map_value_type()` | `LogicalType` | `MAP` |
| `struct_child_count()` | `u64` | `STRUCT` |
| `struct_child_name(index)` | `String` | `STRUCT` |
| `struct_child_type(index)` | `LogicalType` | `STRUCT` |
| `union_member_count()` | `u64` | `UNION` |
| `union_member_name(index)` | `String` | `UNION` |
| `union_member_type(index)` | `LogicalType` | `UNION` |
| `array_size()` | `u64` | `ARRAY` |
| `array_child_type()` | `LogicalType` | `ARRAY` |

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
| `INTERVAL` | `Interval` | `read_interval` | `write_interval` |

NULLs are handled separately — see [NULL Handling & Strings](../data/nulls-and-strings.md).
