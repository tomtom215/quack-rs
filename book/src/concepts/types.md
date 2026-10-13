# Type System

quack-rs describes DuckDB column types with two types: `TypeId`, a plain enum of DuckDB's
type ids, and `LogicalType`, an owned handle for full type descriptions such as
`DECIMAL(18, 3)`, `LIST(VARCHAR)` or a `STRUCT`. This page also maps each DuckDB type to its
Rust type and vector read/write methods.

---

## `TypeId`

`TypeId` is an enum covering DuckDB's column types (the `GEOMETRY` and
`VARIANT` types added in DuckDB 1.5.x are exposed behind the `duckdb-1-5-3`
feature — see [Known Limitations](../reference/known-limitations.md)). The list below
names the variants; it is a listing, not compilable code:

```rust,ignore
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
TypeId::TimeNs
TypeId::Any
TypeId::Varint           // SQL name BIGNUM (VARINT before DuckDB 1.4)
TypeId::SqlNull
TypeId::IntegerLiteral
TypeId::StringLiteral
TypeId::Geometry         // duckdb-1-5-3
TypeId::Variant          // duckdb-1-5-3
```

`TypeId` implements `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash` and `Display`.

### SQL name

```rust
# use quack_rs::types::TypeId;
assert_eq!(TypeId::BigInt.sql_name(), "BIGINT");
assert_eq!(TypeId::Varchar.sql_name(), "VARCHAR");
assert_eq!(format!("{}", TypeId::Timestamp), "TIMESTAMP");
```

### DuckDB constant

`TypeId::to_duckdb_type()` returns the `DUCKDB_TYPE_*` integer constant from `libduckdb-sys`.
You rarely need this directly — it's called internally by `LogicalType::new`.

### Reverse conversion

`TypeId::from_duckdb_type(raw)` converts a raw `DUCKDB_TYPE` constant back into a `TypeId`.
It panics if the value does not match any known constant; `TypeId::try_from_duckdb_type(raw)`
returns `None` instead.

```rust
use quack_rs::types::TypeId;

let type_id = TypeId::from_duckdb_type(libduckdb_sys::DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
assert_eq!(type_id, TypeId::BigInt);
```

---

## `LogicalType`

`LogicalType` is an RAII wrapper around DuckDB's `duckdb_logical_type`. Creating one calls
into DuckDB, so this block is compiled but not run:

```rust,no_run
use quack_rs::types::{LogicalType, TypeId};

let lt = LogicalType::new(TypeId::Varchar);
// lt.as_raw() returns the duckdb_logical_type pointer
// Drop calls duckdb_destroy_logical_type automatically
```

> **Pitfall L7**: `duckdb_create_logical_type` allocates memory that must be freed with
> `duckdb_destroy_logical_type`. `LogicalType`'s `Drop` implementation does this automatically,
> preventing the memory leak that occurs when calling the DuckDB C API directly.
> See [Pitfall L7](../reference/pitfalls.md#l7-logicaltype-memory-leak).

For a type a `TypeId` fully describes, pass the `TypeId` to a builder's `param` or `returns`
method; the builder creates and destroys the `LogicalType` internally. For a parameterized
type (`DECIMAL`, `LIST`, `MAP`, `STRUCT`, `UNION`, `ENUM`, `ARRAY`), build a `LogicalType` and
pass it to `param_logical` or `returns_logical`.

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

Every constructor except `from_raw` panics on invalid input (for example a composite
`TypeId` passed to `new`, or a `DECIMAL` width outside 1–38) and has a `try_*` counterpart
(`try_new`, `try_decimal`, `try_struct_type`, …) that returns
`Result<LogicalType, LogicalTypeError>` instead. `UNION` types accept at most
`MAX_UNION_MEMBERS` (255) members.

### Introspection methods

All introspection methods are `unsafe`: they call into DuckDB, so the handle must be valid
and the C API initialized.

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

### Custom types: `set_alias` and `register`

An extension can add a named type to SQL, as `CREATE TYPE` does. Build the
underlying `LogicalType`, give it a name with `set_alias`, then call
`register(con)` from your entry point. After that, users can write the name
anywhere a type name goes:

```sql
SELECT 'happy'::mood;
CREATE TABLE diary(day DATE, m mood);
```

```rust
use quack_rs::types::LogicalType;
# use quack_rs::query::OwnedConnection;
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();

let mood = LogicalType::enum_type(&["sad", "ok", "happy"]);
unsafe {
    mood.set_alias("mood");
    mood.register(con.as_raw()).unwrap();
}

// Registering the same name again fails: the name is taken.
assert!(unsafe { mood.register(con.as_raw()) }.is_err());
# drop(mood); // DuckDB keeps its own copy
# con.execute("CREATE TABLE diary(m mood)").unwrap();
# con.execute("INSERT INTO diary VALUES ('happy')").unwrap();
# let mut result = con.query("SELECT m::VARCHAR FROM diary").unwrap();
# let chunk = result.next_chunk().unwrap().unwrap();
# let reader = unsafe { chunk.reader(0) };
# assert_eq!(unsafe { reader.read_str(0) }, "happy");
```

- `set_alias(name)` panics if `name` contains a NUL byte; `try_set_alias(name)`
  returns an error instead. `get_alias()` reads the alias back.
- `register(con)` returns an error if the type has no alias, if the name already
  exists in the catalog, or if the type is or contains `ANY` or `INVALID`.
- All three are `unsafe`: they call into DuckDB, and `register` needs a valid,
  open connection.
- The `LogicalType` is still yours after `register`, and is destroyed when it is
  dropped. DuckDB keeps its own copy.

---

## Rust type ↔ DuckDB type mapping

When reading from or writing to vectors, use the corresponding `VectorReader`/`VectorWriter`
method. The most common types:

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
| `HUGEINT` | `HugeInt` | `read_i128` | `write_i128` |
| `UHUGEINT` | `UHugeInt` | `read_u128` | `write_u128` |
| `VARCHAR` | `Varchar` | `read_str` | `write_varchar` |
| `BLOB` | `Blob` | `read_blob` | `write_blob` |
| `UUID` | `Uuid` | `read_uuid` | `write_uuid` |
| `DATE` | `Date` | `read_date` | `write_date` |
| `TIME` | `Time` | `read_time` | `write_time` |
| `TIMESTAMP` | `Timestamp` | `read_timestamp` | `write_timestamp` |
| `INTERVAL` | `Interval` | `read_interval` | `write_interval` |

The timestamp variants (`TIMESTAMP_S`, `_MS`, `_NS`, `TIMESTAMPTZ`), `TIMETZ` and `DECIMAL`
have matching `read_*`/`write_*` methods as well.

NULLs are handled separately — see [NULL Handling & Strings](../data/nulls-and-strings.md).
