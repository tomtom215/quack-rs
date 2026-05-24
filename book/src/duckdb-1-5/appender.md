# Bulk Appender

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

`Appender` is an RAII wrapper around DuckDB's appender — the fastest way to
bulk-insert rows into an existing table. It pairs the core appender lifecycle
(create, append a [`DataChunk`], flush, close) with the DuckDB 1.5 additions:
structured [`ErrorData`](error-data.md) reporting, reverting buffered rows, and
appending a column's `DEFAULT` value.

## Appending data chunks

```rust,no_run
use quack_rs::appender::Appender;
use quack_rs::data_chunk::DataChunk;
use libduckdb_sys::duckdb_connection;

# unsafe fn load(con: duckdb_connection, chunks: &[DataChunk]) -> Result<(), quack_rs::error_data::ErrorData> {
// SAFETY: `con` is a valid, open connection (e.g. from an entry point).
let appender = unsafe { Appender::new(con, None, c"events") }?;

for chunk in chunks {
    appender.append_chunk(chunk)?;
}

// Flush and surface any final error explicitly (see "Drop" below).
appender.close()?;
# Ok(())
# }
```

Pass a schema (or fully-qualified catalog + schema) when the default schema is
not what you want:

```rust,no_run
use quack_rs::appender::Appender;
use libduckdb_sys::duckdb_connection;

# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error_data::ErrorData> {
let a = unsafe { Appender::new(con, Some(c"main"), c"events") }?;
let b = unsafe { Appender::with_catalog(con, Some(c"mydb"), Some(c"main"), c"events") }?;
# let _ = (a, b);
# Ok(())
# }
```

## Recovering from a failed flush

If a `flush` fails (for example a constraint violation), the rows buffered since
the last successful flush can be discarded with `clear`, letting you continue
without re-appending already-committed rows:

```rust,no_run
use quack_rs::appender::Appender;
# use libduckdb_sys::duckdb_connection;
# unsafe fn demo(appender: &Appender) {
if let Err(err) = appender.flush() {
    eprintln!("flush failed: {}", err.message().unwrap_or_default());
    let _ = appender.clear(); // drop the offending buffered rows
}
# }
```

## API

| Method | Description |
|--------|-------------|
| `Appender::new(con, schema, table)` (unsafe) | Create for `table` in `schema` (`None` = default) |
| `Appender::with_catalog(con, catalog, schema, table)` (unsafe) | Create fully qualified |
| `append_chunk(&chunk)` | Append an entire [`DataChunk`] |
| `append_default_to_chunk(&chunk, col, row)` | Write a column's `DEFAULT` into a chunk cell |
| `flush()` | Flush buffered rows without closing |
| `close()` | Flush and close (no further appends) |
| `clear()` | Discard buffered, unflushed rows |
| `error_data()` | Structured [`ErrorData`] from the last failed operation |

Every fallible method returns `Result<(), `[`ErrorData`]`>` (or `Result<Self, ErrorData>`
for the constructors).

## Safety

`new` and `with_catalog` are `unsafe`: you must pass a valid, open
`duckdb_connection` (such as the one provided to your extension's entry point).

## Drop

Dropping an `Appender` flushes and destroys it, but the final flush's result is
**ignored**. To observe an error from the last batch, call `close()` (or a final
`flush()`) explicitly before the appender goes out of scope.

## Related modules

- [Reading & Writing Vectors](../data/vectors.md) — building the [`DataChunk`]s you append
- [Structured Errors](error-data.md) — the [`ErrorData`] returned on failure
- [The Entry Point](../concepts/entry-point.md) — where you obtain a connection

[`DataChunk`]: https://docs.rs/quack-rs/latest/quack_rs/data_chunk/struct.DataChunk.html
[`ErrorData`]: https://docs.rs/quack-rs/latest/quack_rs/error_data/struct.ErrorData.html
