# Bulk Appender

`Appender` is an RAII wrapper around DuckDB's appender — the fastest way to
bulk-insert rows into an existing table, and considerably faster than issuing
`INSERT` statements.

**No feature flag required.** DuckDB has kept `duckdb_appender_*` in the frozen
stable prefix of the extension API (slots 281–291 and 330–356) since v1.2.0, so
using the appender does not push your extension onto the version-pinned unstable
ABI. See [ABI Compatibility](../concepts/abi.md) for why that distinction
matters. Three methods are the exception and need `duckdb-1-5`: `error_data`,
`clear`, and `append_default_to_chunk`.

## Row at a time

Call one `append_*` per column, then finish the row. `row` calls `end_row` for
you, which is the difference between a forgotten `end_row` being a non-issue and
a silently short table:

```rust,no_run
use quack_rs::appender::{AppendError, Appender};
# use libduckdb_sys::duckdb_connection;

# unsafe fn demo(con: duckdb_connection) -> Result<(), AppendError> {
// SAFETY: `con` is a valid, open connection (e.g. from an entry point).
let appender = unsafe { Appender::new(con, None, c"measurements") }?;

for (sensor, reading) in [("a", 1.5_f64), ("b", 2.5)] {
    appender.row(|row| {
        row.append_str(sensor)?;
        row.append_f64(reading)
    })?;
}

appender.close()?;
# Ok(())
# }
```

`append_str` uses `duckdb_append_varchar_length`, so **interior NUL bytes
survive** — the NUL-terminated alternative would stop at the first one.

## A chunk at a time

Fewer FFI crossings, and the natural fit when the data already lives in vectors:

```rust,no_run
use quack_rs::appender::{AppendError, Appender};
use quack_rs::data_chunk::DataChunk;
# use libduckdb_sys::duckdb_connection;

# unsafe fn load(con: duckdb_connection, chunks: &[DataChunk]) -> Result<(), AppendError> {
// SAFETY: `con` is a valid, open connection.
let appender = unsafe { Appender::new(con, None, c"events") }?;
for chunk in chunks {
    appender.append_chunk(chunk)?;
}
appender.close()?;
# Ok(())
# }
```

Pass a schema (or a fully-qualified catalog + schema) when the default schema is
not what you want:

```rust,no_run
use quack_rs::appender::{AppendError, Appender};
# use libduckdb_sys::duckdb_connection;
# unsafe fn demo(con: duckdb_connection) -> Result<(), AppendError> {
let a = unsafe { Appender::new(con, Some(c"main"), c"events") }?;
let b = unsafe { Appender::with_catalog(con, Some(c"mydb"), Some(c"main"), c"events") }?;
# let _ = (a, b);
# Ok(())
# }
```

## Appending a subset of columns

`add_column` narrows the active column list; the omitted columns take their
`DEFAULT` (or NULL). Both `add_column` and `clear_columns` flush everything
appended so far.

```rust,no_run
# use quack_rs::appender::{AppendError, Appender};
# unsafe fn demo(appender: &Appender) -> Result<(), AppendError> {
appender.add_column(c"id")?;              // now only `id` is expected
appender.row(|row| row.append_i32(7))?;
appender.clear_columns()?;                // back to every column
# Ok(())
# }
```

Use [`TableDescription::column_has_default`] to find out whether a column *has*
a default before relying on one.

## Errors arrive late, and invalidate the batch

Appended rows are buffered. A constraint violation therefore surfaces at `flush`
or `close`, **not** at the `append_*` call that caused it, and it invalidates
every buffered row.

With `duckdb-1-5`, `clear` discards the offending buffer so you can carry on
without re-appending rows that were already committed:

```rust,no_run
# use quack_rs::appender::Appender;
# #[cfg(feature = "duckdb-1-5")]
# fn demo(appender: &Appender) {
if let Err(err) = appender.flush() {
    eprintln!("flush failed: {err}");
    let _ = appender.clear(); // drop the offending buffered rows
}
# }
```

## API

| Method | Description |
|--------|-------------|
| `Appender::new(con, schema, table)` (unsafe) | Create for `table` in `schema` (`None` = default) |
| `Appender::with_catalog(con, catalog, schema, table)` (unsafe) | Create fully qualified |
| `column_count()` / `column_type(i)` | The active column list |
| `add_column(name)` / `clear_columns()` | Narrow / reset the active column list |
| `row(|row| ...)` | Append one row, calling `end_row` on success |
| `end_row()` | Finish the current row explicitly |
| `append_bool/_i8/_i16/_i32/_i64/_i128` | Signed integers and `BOOLEAN` |
| `append_u8/_u16/_u32/_u64/_u128` | Unsigned integers |
| `append_f32/_f64` | `FLOAT` / `DOUBLE` |
| `append_str(&str)` / `append_bytes(&[u8])` | `VARCHAR` (NUL-safe) / `BLOB` |
| `append_date/_time/_timestamp/_interval` | Temporal types |
| `append_value(&Value)` | Anything else — `LIST`, `STRUCT`, `MAP`, `UUID`, `DECIMAL`, `ENUM` |
| `append_null()` / `append_default()` | SQL `NULL` / the column's `DEFAULT` |
| `append_chunk(&chunk)` | Append an entire [`DataChunk`] |
| `flush()` / `close()` | Flush buffered rows / flush and close |
| `error_message()` | Message from the last failed operation |
| `append_default_to_chunk(&chunk, col, row)` ¹ | Write a column's `DEFAULT` into a chunk cell |
| `clear()` ¹ | Discard buffered, unflushed rows |
| `error_data()` ¹ | Structured [`ErrorData`] from the last failed operation |

> ¹ Requires the `duckdb-1-5` feature flag.

Every fallible method returns `Result<_, AppendError>`. [`AppendError`] is
[`ErrorData`] (message **and** machine-readable category) when `duckdb-1-5` is
enabled, and [`ExtensionError`] (message only) otherwise — enabling the feature
upgrades the error type in place without changing any method's shape.

## Safety

`new` and `with_catalog` are `unsafe`: you must pass a valid, open
`duckdb_connection` (such as the one provided to your extension's entry point).

Note that both return `Result`. That is not decoration: DuckDB's
`duckdb_append_*` functions do not check whether the appender was successfully
created before dereferencing it, so an appender whose creation failed must never
be used. Because a failed create yields `Err` and no `Appender`, quack-rs makes
that unreachable.

## Drop

Dropping an `Appender` closes (and so flushes) it, but the result is **ignored**
— DuckDB's own header notes that after destruction "it is no longer possible to
obtain the specific error message". Call `close()` explicitly whenever the
outcome matters.

## Related chapters

- [Reading & Writing Vectors](vectors.md) — building the [`DataChunk`]s you append
- [Table Metadata](table-description.md) — column names and `DEFAULT`s
- [Structured Errors](../duckdb-1-5/error-data.md) — the [`ErrorData`] returned with `duckdb-1-5`
- [The Entry Point](../concepts/entry-point.md) — where you obtain a connection

[`DataChunk`]: https://docs.rs/quack-rs/latest/quack_rs/data_chunk/struct.DataChunk.html
[`ErrorData`]: https://docs.rs/quack-rs/latest/quack_rs/error_data/struct.ErrorData.html
[`ExtensionError`]: https://docs.rs/quack-rs/latest/quack_rs/error/struct.ExtensionError.html
[`AppendError`]: https://docs.rs/quack-rs/latest/quack_rs/appender/type.AppendError.html
[`TableDescription::column_has_default`]: https://docs.rs/quack-rs/latest/quack_rs/table_description/struct.TableDescription.html#method.column_has_default
