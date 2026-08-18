# Table Metadata

`TableDescription` answers questions about an existing table from inside your
extension: what its columns are called, and whether a column has a `DEFAULT`.
Useful for replacement scans, table functions, and copy functions that need to
inspect a table before deciding what to do.

**No feature flag required** for creating a description or reading column names
and defaults — `duckdb_table_description_*` has been in the frozen stable prefix
of the extension API (slots 292–297) since v1.2.0. Two accessors are DuckDB 1.5
additions living in the unstable region and need `duckdb-1-5`: `column_count`
and `column_type`.

```rust,no_run
use quack_rs::table_description::TableDescription;
# use libduckdb_sys::duckdb_connection;

# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
// SAFETY: `con` is a valid, open connection.
let desc = unsafe { TableDescription::create(con, "main", "events") }?;

assert_eq!(desc.column_name(0).as_deref(), Some("id"));
assert_eq!(desc.column_has_default(0), Some(false));
# Ok(())
# }
```

`with_catalog` addresses a table in another catalog, and takes `None` to mean
"the default" for either the catalog or the schema:

```rust,no_run
# use quack_rs::table_description::TableDescription;
# use libduckdb_sys::duckdb_connection;
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
let desc = unsafe { TableDescription::with_catalog(con, Some("mydb"), None, "events") }?;
# let _ = desc;
# Ok(())
# }
```

## API

| Method | Description |
|--------|-------------|
| `TableDescription::create(con, schema, table)` (unsafe) | Describe `schema.table` |
| `TableDescription::with_catalog(con, catalog, schema, table)` (unsafe) | Describe a fully-qualified table (`None` = default) |
| `column_name(i)` | Column name, or `None` if `i` is out of range |
| `column_has_default(i)` | Whether the column has a `DEFAULT`, or `None` if `i` is out of range |
| `column_count()` ¹ | Number of columns |
| `column_type(i)` ¹ | Column [`LogicalType`], or `None` if `i` is out of range |

> ¹ Requires the `duckdb-1-5` feature flag.

Out-of-range indices return `None` rather than panicking, so a description can
be walked without knowing the width up front when `duckdb-1-5` is off.

## Related chapters

- [Bulk Appender](appender.md) — `column_has_default` tells you when
  `append_default` is safe to call
- [Type System](../concepts/types.md) — what a [`LogicalType`] describes

[`LogicalType`]: https://docs.rs/quack-rs/latest/quack_rs/types/struct.LogicalType.html
