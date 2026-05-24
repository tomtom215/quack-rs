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
| `TypeId::Decimal` | `DECIMAL` | `DUCKDB_TYPE_DECIMAL` | fixed-point decimal |
| `TypeId::TimestampS` | `TIMESTAMP_S` | `DUCKDB_TYPE_TIMESTAMP_S` | seconds since epoch |
| `TypeId::TimestampMs` | `TIMESTAMP_MS` | `DUCKDB_TYPE_TIMESTAMP_MS` | milliseconds since epoch |
| `TypeId::TimestampNs` | `TIMESTAMP_NS` | `DUCKDB_TYPE_TIMESTAMP_NS` | nanoseconds since epoch |
| `TypeId::Enum` | `ENUM` | `DUCKDB_TYPE_ENUM` | enumeration type |
| `TypeId::List` | `LIST` | `DUCKDB_TYPE_LIST` | variable-length list |
| `TypeId::Struct` | `STRUCT` | `DUCKDB_TYPE_STRUCT` | named fields (row type) |
| `TypeId::Map` | `MAP` | `DUCKDB_TYPE_MAP` | key-value pairs |
| `TypeId::Uuid` | `UUID` | `DUCKDB_TYPE_UUID` | 128-bit UUID |
| `TypeId::Union` | `UNION` | `DUCKDB_TYPE_UNION` | tagged union of types |
| `TypeId::Bit` | `BIT` | `DUCKDB_TYPE_BIT` | bitstring |
| `TypeId::TimeTz` | `TIMETZ` | `DUCKDB_TYPE_TIME_TZ` | timezone-aware time |
| `TypeId::UHugeInt` | `UHUGEINT` | `DUCKDB_TYPE_UHUGEINT` | 128-bit unsigned |
| `TypeId::Array` | `ARRAY` | `DUCKDB_TYPE_ARRAY` | fixed-length array |
| `TypeId::TimeNs` | `TIME_NS` | `DUCKDB_TYPE_TIME_NS` | nanosecond-precision time (`duckdb-1-5`) |
| `TypeId::Any` | `ANY` | `DUCKDB_TYPE_ANY` | wildcard for function signatures (`duckdb-1-5`) |
| `TypeId::Varint` | `VARINT` | `DUCKDB_TYPE_BIGNUM` | variable-length integer (`duckdb-1-5`) |
| `TypeId::SqlNull` | `SQLNULL` | `DUCKDB_TYPE_SQLNULL` | explicit SQL NULL type (`duckdb-1-5`) |
| `TypeId::IntegerLiteral` | `INTEGER_LITERAL` | `DUCKDB_TYPE_INTEGER_LITERAL` | unresolved integer literal (`duckdb-1-5`) |
| `TypeId::StringLiteral` | `STRING_LITERAL` | `DUCKDB_TYPE_STRING_LITERAL` | unresolved string literal (`duckdb-1-5`) |
| `TypeId::Geometry` | `GEOMETRY` | `DUCKDB_TYPE_GEOMETRY` | spatial geometry value (`duckdb-1-5-3`) |
| `TypeId::Variant` | `VARIANT` | `DUCKDB_TYPE_VARIANT` | self-describing nested value, e.g. Iceberg v3 (`duckdb-1-5-3`) |

> **Feature gate for `Geometry` / `Variant`:** `DUCKDB_TYPE_GEOMETRY` (40) and
> `DUCKDB_TYPE_VARIANT` (41) require the **`duckdb-1-5-3`** feature, which layers
> on top of `duckdb-1-5` and needs `libduckdb-sys >= 1.10503.1` (DuckDB 1.5.3).
> They sit behind a separate feature because these type-enum values postdate the
> `duckdb-1-5` feature's 1.5.0 floor (`VARIANT` was added in DuckDB 1.5.3); gating
> them this way avoids breaking consumers pinned to libduckdb-sys 1.5.0–1.5.2.
> See [Known Limitations](known-limitations.md).

---

## Methods

### `to_duckdb_type() → DUCKDB_TYPE`

Converts to the raw C API integer constant. Used internally by the builder APIs.

```rust
use quack_rs::types::TypeId;

let raw: libduckdb_sys::DUCKDB_TYPE = TypeId::BigInt.to_duckdb_type();
```

### `from_duckdb_type(raw) → TypeId`

Converts a raw `DUCKDB_TYPE` constant back into a `TypeId`. Recognizes every
variant available in the active feature set, including the `duckdb-1-5` values
(`TIME_NS`, `ANY`, `VARINT`, `SQLNULL`, `INTEGER_LITERAL`, `STRING_LITERAL`) and
the `duckdb-1-5-3` values (`GEOMETRY`, `VARIANT`) when those features are enabled.
Panics if the value does not correspond to any variant available in the current
feature configuration.

```rust
use quack_rs::types::TypeId;

let type_id = TypeId::from_duckdb_type(libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
assert_eq!(type_id, TypeId::BigInt);
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
| `Interval` | `read_interval` | `write_interval` | `DuckInterval` |

`HugeInt`, `Blob`, `List`, `Struct`, `Map`, `Uuid`, `Date`, `Time`, `Timestamp`,
`TimestampTz`, `Decimal`, `TimestampS`, `TimestampMs`, `TimestampNs`, `Enum`,
`Union`, `Bit`, `TimeTz`, `UHugeInt`, `Array`, `TimeNs`, `Any`, `Varint`, `SqlNull`,
`IntegerLiteral`, `StringLiteral`, `Geometry`, `Variant` do not yet have dedicated
read/write helpers. Access these via the raw data pointer from
`duckdb_vector_get_data`.

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
use quack_rs::types::{LogicalType, TypeId};

let lt = LogicalType::new(TypeId::BigInt);
// or use the From impl:
let lt: LogicalType = TypeId::BigInt.into();
// LogicalType implements Drop → calls duckdb_destroy_logical_type automatically
```

`LogicalType` wraps `duckdb_logical_type` with RAII cleanup, preventing the
memory leak described in [Pitfall L7](pitfalls.md#l7-logicaltype-memory-leak).

### Constructors

| Constructor | Creates |
|-------------|---------|
| `new(type_id)` | Simple type from a `TypeId` |
| `from_raw(ptr)` | Takes ownership of a raw handle (unsafe) |
| `decimal(width, scale)` | `DECIMAL(width, scale)` |
| `list(element_type)` | `LIST<T>` from a `TypeId` |
| `list_from_logical(element)` | `LIST<T>` from an existing `LogicalType` |
| `map(key, value)` | `MAP<K, V>` from `TypeId`s |
| `map_from_logical(key, value)` | `MAP<K, V>` from existing `LogicalType`s |
| `struct_type(fields)` | `STRUCT` from `&[(&str, TypeId)]` |
| `struct_type_from_logical(fields)` | `STRUCT` from `&[(&str, LogicalType)]` |
| `union_type(members)` | `UNION` from `&[(&str, TypeId)]` |
| `union_type_from_logical(members)` | `UNION` from `&[(&str, LogicalType)]` |
| `enum_type(members)` | `ENUM` from `&[&str]` |
| `array(element_type, size)` | `ARRAY<T>[size]` from a `TypeId` |
| `array_from_logical(element, size)` | `ARRAY<T>[size]` from an existing `LogicalType` |

### Introspection methods

All introspection methods are `unsafe` (require a valid DuckDB runtime handle):

`get_type_id`, `get_alias`, `set_alias`, `decimal_width`, `decimal_scale`,
`decimal_internal_type`, `enum_internal_type`, `enum_dictionary_size`,
`enum_dictionary_value`, `list_child_type`, `map_key_type`, `map_value_type`,
`struct_child_count`, `struct_child_name`, `struct_child_type`,
`union_member_count`, `union_member_name`, `union_member_type`,
`array_size`, `array_child_type`.

See [Type System](../concepts/types.md) for the full introspection table.
