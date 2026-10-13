# Config Options

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

An extension can add its own settings to DuckDB. Users then change them with
`SET`, reset them with `RESET`, and read them with `current_setting()`, exactly
like a built-in setting. Your callbacks read the current value through a
[`ClientContext`](catalog.md).

`ConfigOptionBuilder` (in `quack_rs::config_option`) describes the setting, and
`register` adds it to the database.

## Registering a setting

Call this from your entry point, with the other registrations:

```rust,no_run
use quack_rs::config_option::{ConfigOptionBuilder, ConfigOptionScope};
use quack_rs::prelude::*;

fn register(reg: &impl Registrar) -> Result<(), ExtensionError> {
    let option = ConfigOptionBuilder::try_new("my_ext_threshold")?
        .description("Maximum threshold for my_ext operations")?
        .option_type(TypeId::BigInt)
        .default_value("100")?
        .scope(ConfigOptionScope::Global);
    unsafe { reg.register_config_option(option) }
}
```

Users can then write:

```sql
SELECT current_setting('my_ext_threshold');  -- 100
SET my_ext_threshold = 5;
SELECT current_setting('my_ext_threshold');  -- 5
RESET my_ext_threshold;
```

`Registrar::register_config_option` calls `ConfigOptionBuilder::register` on
the extension's connection. You can also call `register(con)` yourself on a raw
`duckdb_connection`.

## The builder

| Method | Sets | Notes |
|--------|------|-------|
| `try_new(name)` | The setting's name | Errors if the name contains a NUL byte |
| `description(text)` | The text `duckdb_settings()` shows | Errors on a NUL byte |
| `option_type(type_id)` | The value type, a `TypeId` | Required |
| `default_value(text)` | The default, written as SQL text | Required; errors on a NUL byte |
| `scope(scope)` | `ConfigOptionScope::Global` (the default), `Session` or `Local` | |

`default_value` takes a string. `register` converts it to the option's type
with the same SQL cast `SET` uses, so `"100"` becomes the `BIGINT` 100.

## What `register` refuses

`register` checks the option before it registers anything, and returns an
`ExtensionError` if:

- **The type or the default is missing.** DuckDB treats an extension option
  without a default as unset: `current_setting` fails with "unrecognized
  configuration parameter" until someone `SET`s it.
- **The type is a pseudo-type** (`ANY`, `SQLNULL`, `INTEGER_LITERAL`,
  `STRING_LITERAL`), or a type whose default the C API cannot build safely
  (`BIGNUM`, `GEOMETRY`, `VARIANT`, …). Use a `VARCHAR` option and parse the
  text yourself.
- **The default does not convert** to the option's type.
- **The name is already a setting**: a built-in one such as `threads`, an alias
  such as `worker_threads`, or another extension's. Names are compared without
  regard to case. Prefix your settings with your extension's name.

```rust
# use quack_rs::config_option::ConfigOptionBuilder;
# use quack_rs::query::OwnedConnection;
# use quack_rs::types::TypeId;
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
// No default: refused.
let no_default = ConfigOptionBuilder::try_new("my_ext_mode")
    .unwrap()
    .option_type(TypeId::Varchar);
assert!(unsafe { no_default.register(con.as_raw()) }.is_err());

// A built-in setting's name: refused.
let taken = ConfigOptionBuilder::try_new("THREADS")
    .unwrap()
    .option_type(TypeId::BigInt)
    .default_value("4")
    .unwrap();
assert!(unsafe { taken.register(con.as_raw()) }.is_err());

// A default that is not a BIGINT: refused.
let bad_default = ConfigOptionBuilder::try_new("my_ext_limit")
    .unwrap()
    .option_type(TypeId::BigInt)
    .default_value("lots")
    .unwrap();
assert!(unsafe { bad_default.register(con.as_raw()) }.is_err());
```

## Reading the value in a callback

A callback reads the current value through a `ClientContext`.
`ClientContext::config_option` returns the value rendered as text:

```rust,no_run
use quack_rs::client_context::ClientContext;

fn threshold(ctx: &ClientContext) -> i64 {
    ctx.config_option(c"my_ext_threshold")
        .and_then(|text| text.parse().ok())
        .unwrap_or(100)
}
```

It returns `None` both when the setting does not exist and when its value is
SQL `NULL` (after `SET my_ext_threshold = NULL`). Only ask for a setting you
registered or know exists: against a DuckDB built with debug assertions, asking
for a missing setting aborts the process
([Pitfall P12](../reference/pitfalls.md#p12)). To test whether a setting exists,
use SQL:

```sql
SELECT count(*) FROM duckdb_settings() WHERE name = 'my_ext_threshold';
```

The example below registers a setting, changes it with `SET`, and reads it back
both ways:

```rust
# use quack_rs::client_context::ClientContext;
# use quack_rs::config_option::ConfigOptionBuilder;
# use quack_rs::query::OwnedConnection;
# use quack_rs::types::TypeId;
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# let setting = |sql: &str| -> String {
#     let mut result = con.query(sql).unwrap();
#     let chunk = result.next_chunk().unwrap().unwrap();
#     let reader = unsafe { chunk.reader(0) };
#     unsafe { reader.read_str(0) }.to_owned()
# };
let option = ConfigOptionBuilder::try_new("my_ext_threshold")
    .unwrap()
    .option_type(TypeId::BigInt)
    .default_value("100")
    .unwrap();
unsafe { option.register(con.as_raw()) }.unwrap();

assert_eq!(setting("SELECT current_setting('my_ext_threshold')::VARCHAR"), "100");
con.execute("SET my_ext_threshold = 5").unwrap();
assert_eq!(setting("SELECT current_setting('my_ext_threshold')::VARCHAR"), "5");

let ctx = unsafe { ClientContext::from_connection(con.as_raw()) }.unwrap();
assert_eq!(ctx.config_option(c"my_ext_threshold").as_deref(), Some("5"));
```

## Testing

`MockRegistrar` runs the checks that need no database (a type and a default
are set, and the type is not a pseudo-type), so a unit test catches a missing
default. The name and default-conversion checks need a real connection; use
`InMemoryDb` (see the [Testing Guide](../testing.md)).

## See also

- [Catalog & Client Context](catalog.md): reading settings from callbacks
- [`config_option`](https://docs.rs/quack-rs/latest/quack_rs/config_option/) module documentation
