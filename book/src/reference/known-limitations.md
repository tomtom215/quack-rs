# Known Limitations

## Window functions are not available

DuckDB **window functions** (`OVER (...)` clauses) are implemented entirely in
DuckDB's C++ layer and have **no counterpart in the public C extension API**.

This is not a gap in `quack-rs` or in `libduckdb-sys` — the relevant symbol
(`duckdb_create_window_function`) simply does not exist in the C API:

| Symbol | C API (1.4.x)? | C API (1.5.0+)? | C++ API? |
|--------|----------------|-----------------|----------|
| `duckdb_create_window_function` | **No** | **No** | Yes |
| `duckdb_create_copy_function`   | **No** | **Yes** | Yes |
| `duckdb_create_scalar_function` | Yes    | Yes     | Yes |
| `duckdb_create_aggregate_function` | Yes | Yes     | Yes |
| `duckdb_create_table_function`  | Yes    | Yes     | Yes |
| `duckdb_create_cast_function`   | Yes    | Yes     | Yes |

**What this means for your extension:**

If your extension needs window-function semantics, you can approximate them with
aggregate functions in most cases (DuckDB will push down the window logic). True
custom window operator registration requires writing a C++ extension.

If DuckDB exposes window registration in a future C API version, `quack-rs`
will add wrappers in the corresponding release.

## COPY functions (resolved in DuckDB 1.5.0)

DuckDB 1.5.0 added `duckdb_create_copy_function` and related symbols to the public
C extension API. quack-rs wraps these in the `copy_function` module behind the
`duckdb-1-5` feature flag. See `CopyFunctionBuilder` for usage.

This was previously listed as a known limitation (no C API counterpart prior to 1.5.0).

## Callback accessor wrappers (resolved)

quack-rs now wraps all major **callback accessor** functions — the C API
functions used *inside* your callbacks to retrieve arguments, set errors,
access bind data, etc.

| Category | Wrapper type | Available |
|----------|-------------|-----------|
| **Scalar function execution** | `ScalarFunctionInfo` | Always |
| **Scalar function bind** | `ScalarBindInfo` | `duckdb-1-5` |
| **Scalar function init** | `ScalarInitInfo` | `duckdb-1-5` |
| **Aggregate function callbacks** | `AggregateFunctionInfo` | Always |
| **Table function bind** | `BindInfo` | Always |
| **Table function init** | `InitInfo` | Always |
| **Table function scan** | `FunctionInfo` | Always |
| **Cast function callbacks** | `CastFunctionInfo` | Always |
| **Copy function bind** | `CopyBindInfo` | `duckdb-1-5` |
| **Copy function global init** | `CopyGlobalInitInfo` | `duckdb-1-5` |
| **Copy function sink** | `CopySinkInfo` | `duckdb-1-5` |
| **Copy function finalize** | `CopyFinalizeInfo` | `duckdb-1-5` |

All callback accessor functions are now wrapped, including `get_client_context`
on all callback types (returns a [`ClientContext`][crate::client_context::ClientContext]).

## Complex type creation (resolved)

`LogicalType` now provides constructors for all complex parameterized types:

| Method | Type created |
|--------|-------------|
| `LogicalType::decimal(width, scale)` | `DECIMAL(p, s)` |
| `LogicalType::enum_type(members)` | `ENUM('a', 'b', ...)` |
| `LogicalType::array(child, size)` | `type[N]` |
| `LogicalType::union_type(members)` | `UNION(a INT, b VARCHAR)` |
| `LogicalType::list(child)` | `LIST(type)` |
| `LogicalType::struct_type(fields)` | `STRUCT(...)` |
| `LogicalType::map(key, value)` | `MAP(K, V)` |

All constructors have `_from_logical` variants for nested complex types.
Introspection methods (`get_type_id`, `list_child_type`, `struct_child_count`,
`decimal_width`, etc.) are also available.

## VARIANT and GEOMETRY types (resolved — exposed behind `duckdb-1-5-3`)

DuckDB v1.5.1 introduced the `VARIANT` type for Iceberg v3 support. As of
**DuckDB 1.5.3** it is present in the C type enum as `DUCKDB_TYPE_VARIANT` (41),
and the `GEOMETRY` type (`DUCKDB_TYPE_GEOMETRY`, 40) is present as well.

quack-rs exposes these as `TypeId::Variant` and `TypeId::Geometry`, gated behind
the **`duckdb-1-5-3`** feature. That feature layers on top of `duckdb-1-5` and
requires `libduckdb-sys >= 1.10503.1` (DuckDB 1.5.3). The separate gate exists
because these type-enum values postdate the `duckdb-1-5` feature's 1.5.0 floor
(`VARIANT` only landed in 1.5.3); keeping them out of `duckdb-1-5` preserves
compatibility for consumers pinned to libduckdb-sys 1.5.0–1.5.2.

```toml
[dependencies]
quack-rs = { version = "0.13", features = ["duckdb-1-5-3"] }
```

Neither type yet has dedicated `VectorReader`/`VectorWriter` helpers; access
their data via the raw pointer from `duckdb_vector_get_data` when needed.
