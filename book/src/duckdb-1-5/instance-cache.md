# Instance Cache

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

An `InstanceCache` lets multiple connections share a single underlying DuckDB
*instance* for a given database path. Opening the same path twice through the
cache returns handles backed by the **same** instance, which avoids the
"database is already open in another instance" conflict and saves the cost of
re-initialising the database.

This is primarily useful for extensions or host integrations that open secondary
databases on behalf of a query.

## Opening through the cache

```rust,no_run
use quack_rs::instance_cache::InstanceCache;

# fn demo() -> Result<(), quack_rs::error::ExtensionError> {
let cache = InstanceCache::new();

// Returns a duckdb_database the caller OWNS and must close with duckdb_close.
let db = cache.get_or_create(c"analytics.db", None)?;
# let _ = db;
# Ok(())
# }
```

Pass a [`DbConfig`] to control how a *freshly created* instance is configured.
When an instance already exists for the path, the config must **match** the one it
was created with — a different one is an error ("Can't open a connection to same
database file with a different configuration than existing connections"), not
silently ignored. `None` means DuckDB's defaults, so it too conflicts with an
instance created with a custom config. An empty path (in-memory) is never cached:
each call creates a separate database.

```rust,no_run
use quack_rs::instance_cache::InstanceCache;
use quack_rs::config::DbConfig;

# fn demo() -> Result<(), quack_rs::error::ExtensionError> {
let cache = InstanceCache::new();
let config = DbConfig::new()?;
// configure `config` as needed...
let db = cache.get_or_create(c"analytics.db", Some(&config))?;
# let _ = db;
# Ok(())
# }
```

## API

| Method | Description |
|--------|-------------|
| `InstanceCache::new()` | Create a new, empty cache |
| `get_or_create(path, config)` | Open `path`, creating the instance if needed |
| `as_raw()` | The raw `duckdb_instance_cache` handle |

`get_or_create` returns `Result<duckdb_database, `[`ExtensionError`]`>`; on failure
the error carries DuckDB's message.

## Threads

`InstanceCache` is `Send + Sync`, so one cache can serve several threads: DuckDB's
`DBInstanceCache` guards its map with a mutex and serialises creation of each
database. One ordering hazard remains, and it is DuckDB's: while the **last** handle
to a cached file database is being closed, a concurrent `get_or_create` of the same
path can fail with "Unique file handle conflict" because the closing instance still
holds the file. Keep one handle open while other threads may open the path, or
retry.

## Ownership

`InstanceCache` is RAII and destroys the cache on drop. The `duckdb_database`
returned by `get_or_create` is, however, **owned by the caller** — you must close
it with `duckdb_close` when finished. The cache keeps the *underlying* instance
alive so that subsequent opens of the same path are cheap.

## Related modules

- [`config`](https://docs.rs/quack-rs/latest/quack_rs/config/struct.DbConfig.html) — `DbConfig`, the RAII configuration builder accepted here

[`DbConfig`]: https://docs.rs/quack-rs/latest/quack_rs/config/struct.DbConfig.html
[`ExtensionError`]: https://docs.rs/quack-rs/latest/quack_rs/error/struct.ExtensionError.html
