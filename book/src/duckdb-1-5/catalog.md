# Catalog & Client Context

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

A `ClientContext` is the connection a query is running on, seen from inside your
extension. Through it a callback can:

- read a setting's current value (`config_option`);
- look up tables, views, types and other catalog entries (`catalog`);
- open files through DuckDB's [virtual file system](file-system.md);
- learn the connection's ID (`connection_id`).

The types live in `quack_rs::client_context` and `quack_rs::catalog`.

## Getting a `ClientContext`

Most callbacks that run during planning can hand you one:

| Where | Method |
|-------|--------|
| Table function bind | `BindInfo::get_client_context()` |
| Scalar function bind / init | `ScalarBindInfo::get_client_context()` / `ScalarInitInfo::get_client_context()` |
| Copy function bind, global init, sink, finalize | `get_client_context()` on each `Copy*Info` |
| Anywhere you hold a connection | `ClientContext::from_connection(con)` |

All of them are `unsafe`. The context does **not** keep its connection alive: do
not use it, or a `FileSystem` borrowed from it, after that connection is
closed. A `ClientContext` is destroyed when it is dropped.

## Reading a setting

```rust,no_run
use quack_rs::client_context::ClientContext;

fn threads(ctx: &ClientContext) -> Option<u64> {
    ctx.config_option(c"threads")?.parse().ok()
}
```

`config_option` returns the value as text, or `None` when the setting does not
exist or is SQL `NULL`. Do not use it to probe for a setting that might not
exist: a DuckDB built with debug assertions aborts on a missing name
([Pitfall P12](../reference/pitfalls.md#p12)). [Config Options](config-options.md)
shows how to register a setting of your own.

## Looking up a catalog entry

Looking up an entry takes two steps. `ClientContext::catalog(name)` returns the
database's `Catalog`, and `Catalog::get_entry` looks up one entry in it. This
table function bind checks that a table exists before it declares its result:

```rust,no_run
use quack_rs::catalog::CatalogEntryType;
use quack_rs::table::BindInfo;
use quack_rs::types::TypeId;

quack_rs::table_bind_callback!(my_bind, |info| {
    let bind = unsafe { BindInfo::new(info) };
    let ctx = unsafe { bind.get_client_context() };
    let Some(catalog) = (unsafe { ctx.catalog(c"memory") }) else {
        bind.set_error("database 'memory' is not attached");
        return;
    };
    match unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"events", CatalogEntryType::Table) } {
        Ok(Some(_entry)) => {
            bind.add_result_column("n", TypeId::BigInt);
        }
        Ok(None) => bind.set_error("table main.events does not exist"),
        Err(e) => bind.set_error(e.as_str()),
    }
});
```

`get_entry` returns:

- `Ok(Some(entry))` when the entry exists. `entry.name()` and `entry.entry_type()`
  are copies taken at lookup time; the entry holds no DuckDB handle.
- `Ok(None)` when it does not.
- `Err(_)` when quack-rs refused the lookup, without calling DuckDB (see below).

A `Table` lookup also finds a **view** of that name. Check `entry_type()` when the
difference matters.

### Catalog names

`catalog(name)` takes the catalog's name, not a default. An in-memory
database's catalog is named `memory`; a database file's is the file's stem
(`analytics.duckdb` is `analytics`). An empty name returns `None`.

`catalog` also returns `None` when no transaction is active. Inside a function
callback a transaction is always active. On an idle connection outside a query,
there is none:

```rust
# use quack_rs::catalog::CatalogEntryType;
# use quack_rs::client_context::ClientContext;
# use quack_rs::query::OwnedConnection;
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
con.execute("CREATE TABLE events(id BIGINT)").unwrap();
con.execute("CREATE VIEW recent AS SELECT * FROM events").unwrap();
let ctx = unsafe { ClientContext::from_connection(con.as_raw()) }.unwrap();

// Idle auto-commit connection: no transaction, so no catalog.
assert!(unsafe { ctx.catalog(c"memory") }.is_none());

con.execute("BEGIN TRANSACTION").unwrap();
let catalog = unsafe { ctx.catalog(c"memory") }.expect("a transaction is active");
assert_eq!(catalog.type_name(), Some("duckdb"));

let found = unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"events", CatalogEntryType::Table) };
let entry = found.unwrap().expect("the table exists");
assert_eq!(entry.name(), Some("events"));

// A Table lookup also finds a view.
let view = unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"recent", CatalogEntryType::Table) };
assert_eq!(view.unwrap().unwrap().entry_type(), CatalogEntryType::View);

let missing = unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"nope", CatalogEntryType::Table) };
assert!(missing.unwrap().is_none());
# drop(catalog);
# con.execute("COMMIT").unwrap();
```

The `Catalog` value is valid only while the transaction that returned it is
active and its database is attached. Look it up again in each callback rather
than storing it.

## Lookups quack-rs refuses

`duckdb_catalog_get_entry` has no error handling, so some lookups would let a C++
exception cross into Rust and abort the process. `get_entry` (and the underlying
`CatalogEntry::lookup`) returns an `Err` for them instead:

- **Entry types the C API cannot look up**: `Schema`, `Database`,
  `PreparedStatement` and `Invalid`. `CatalogEntryType::is_lookup_supported()`
  tells you which types work: `Table`, `View`, `Index`, `Sequence`, `Collation`
  and `Type`.
- **A `Type` or `Collation` name that DuckDB would autoload an extension for**
  (`json`, `inet`, or an ICU collation such as `de`), while the
  `autoload_known_extensions` setting is on. Run
  `SET autoload_known_extensions = false` first to look such a name up.
  `CatalogEntryType::may_autoload_extension(name)` makes the same check.
- **A catalog DuckDB does not implement itself**, such as one a storage
  extension attaches. `Catalog::type_name()` is `"duckdb"` for DuckDB's own
  catalogs (the database's, `temp` and `system`). Query other catalogs with SQL,
  for example `duckdb_tables()`.

```rust
# use quack_rs::catalog::CatalogEntryType;
# use quack_rs::client_context::ClientContext;
# use quack_rs::query::OwnedConnection;
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# let ctx = unsafe { ClientContext::from_connection(con.as_raw()) }.unwrap();
# con.execute("BEGIN TRANSACTION").unwrap();
# let catalog = unsafe { ctx.catalog(c"memory") }.unwrap();
assert!(!CatalogEntryType::Schema.is_lookup_supported());
let refused = unsafe { catalog.get_entry(ctx.as_raw(), c"main", c"main", CatalogEntryType::Schema) };
assert!(refused.is_err());
# drop(catalog);
# con.execute("COMMIT").unwrap();
```

## API summary

| Item | Purpose |
|------|---------|
| `ClientContext::from_connection(con)` | Context for a raw `duckdb_connection` (`unsafe`) |
| `ClientContext::from_raw(ctx)` / `as_raw()` | Wrap or expose the raw handle |
| `ClientContext::config_option(name)` | A setting's value as text |
| `ClientContext::catalog(name)` | The named catalog, in an active transaction (`unsafe`) |
| `ClientContext::connection_id()` | The connection's ID |
| `Catalog::type_name()` | `"duckdb"` for DuckDB's own catalogs |
| `Catalog::get_entry(ctx, schema, name, type)` | Look up one entry (`unsafe`) |
| `CatalogEntry::name()` / `entry_type()` | The entry's name and type at lookup time |

## See also

- [Config Options](config-options.md)
- [Virtual File System](file-system.md), which takes a `ClientContext`
- [Running SQL from an Extension](../data/running-sql.md), for anything the catalog API refuses
- [`catalog`](https://docs.rs/quack-rs/latest/quack_rs/catalog/) and
  [`client_context`](https://docs.rs/quack-rs/latest/quack_rs/client_context/) module documentation
