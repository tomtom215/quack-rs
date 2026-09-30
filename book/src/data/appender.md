# Bulk Appender

`Appender` is an RAII wrapper around DuckDB's appender, the fastest way for an
extension to bulk-insert rows into an existing table: rows are buffered and
written in batches instead of going through one `INSERT` statement each. This
page covers appending row by row and chunk by chunk, error handling, and what
happens when a row fails half-way.

**No feature flag required.** DuckDB has kept `duckdb_appender_*` in the frozen
stable prefix of the extension API (slots 281–291 and 330–356) since v1.2.0, so
using the appender does not push your extension onto the version-pinned unstable
ABI. See [ABI Compatibility](../concepts/abi.md) for why that distinction
matters. Three methods are the exception and need `duckdb-1-5`: `error_data`,
`clear`, and `append_default_to_chunk`.

## Row at a time

Call one `append_*` per column, then finish the row with `end_row`. `row` calls
`end_row` for you when its closure succeeds, so a forgotten `end_row` cannot
leave the table silently short:

```rust
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
# fn live_connection() -> libduckdb_sys::duckdb_connection {
#     std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
#     let (mut db, mut con) = (std::ptr::null_mut(), std::ptr::null_mut());
#     unsafe {
#         assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess);
#         assert_eq!(libduckdb_sys::duckdb_connect(db, &mut con), libduckdb_sys::DuckDBSuccess);
#     }
#     con
# }
# /// First column of the first row, as BIGINT; `None` for NULL.
# fn query_i64(con: libduckdb_sys::duckdb_connection, sql: &str) -> Option<i64> {
#     let mut result = unsafe { quack_rs::query::query(con, sql) }.unwrap();
#     let chunk = result.next_chunk().unwrap().unwrap();
#     let reader = unsafe { chunk.reader(0) };
#     unsafe { reader.is_valid(0).then(|| reader.read_i64(0)) }
# }
# let con = live_connection();
# unsafe { quack_rs::query::execute(con, "CREATE TABLE measurements (sensor VARCHAR, reading DOUBLE)") }.unwrap();
# unsafe { demo(con) }.unwrap();
# assert_eq!(query_i64(con, "SELECT count(*) FROM measurements"), Some(2));
# assert_eq!(query_i64(con, "SELECT (sum(reading) * 10)::BIGINT FROM measurements"), Some(40));
```

`append_str` uses `duckdb_append_varchar_length`, so **interior NUL bytes
survive**; the NUL-terminated `duckdb_append_varchar` would stop at the first
one. `append_str` and `append_bytes` refuse a value longer than `u32::MAX`
bytes, which DuckDB would silently truncate.

`append_time` and `append_timestamp` refuse, before calling DuckDB, a payload
outside the range DuckDB's SQL produces (`TIME` `00:00:00`–`24:00:00`; a
`TIMESTAMP` from `290309-12-22 (BC)`, plus `-infinity`). DuckDB stores such a
value unchecked, and reading the row back later fails or crashes.

## A chunk at a time

`append_chunk` takes a whole [`DataChunk`] whose column types match the
appender's active columns. It makes fewer FFI calls than appending row by row
and suits data that is already in vectors; it is refused while a row appended
by hand is still open.

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
`DEFAULT` (or NULL). Both `add_column` and `clear_columns` flush every row
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
a default before relying on one. `append_default()` fails for a default that is
not a constant, such as `nextval('seq')` or `now()`: DuckDB's appender
evaluates defaults once, when it is created.

## Errors arrive late, and invalidate the batch

Appended rows are buffered. A constraint violation therefore surfaces at `flush`
or `close`, **not** at the `append_*` call that caused it, and it invalidates
every buffered row.

With `duckdb-1-5`, `clear` discards the offending buffer so you can carry on
without re-appending rows that were already committed:

```rust,no_run
# use quack_rs::appender::Appender;
# fn demo(appender: &Appender) {
if let Err(err) = appender.flush() {
    eprintln!("flush failed: {err}");
    let _ = appender.clear(); // drop the offending buffered rows
}
# }
```

## A row that fails half-way loses the batch

DuckDB counts the values of the current row and cannot take one back. If a
`row` closure fails or panics *after* its first value went in, the half-written
row can be neither finished nor dropped, and DuckDB's `close` then returns
success while writing **nothing**: every row buffered since the last flush is
gone. (DuckDB also flushes by itself each time 204,800 rows accumulate; rows
written by such a flush are safe.)

quack-rs does not let that pass silently. The appender becomes *poisoned*:
every later `row`, append, `end_row`, `flush` and `close` returns an error
saying how many buffered rows were not written. With `duckdb-1-5`, `clear()`
discards them and makes the appender usable again; without it, create a new
appender. `close()` with a row that was started but not ended is an error
too (finish the row and close again).

A value that fails as the *first* of its row loses nothing, and when you
append by hand you can retry a rejected value in the same column. If losing a
batch is not acceptable, `flush()` at the points you can afford to go back to.

After a successful `close()` the appender refuses all further work.

## Row order and schema changes

- Row-at-a-time rows wait in their own buffer until a full chunk (2,048 rows)
  accumulates, while
  `append_chunk` adds its rows to the table-bound buffer directly, so a chunk
  lands **ahead of** rows appended before it that are still buffered. Flush
  first if insertion order matters.
- Buffered rows are written by column **position** at flush time. If another
  connection drops a column and adds one in between, a value lands in the new
  column with no error.

## API

| Method | Description |
|--------|-------------|
| `Appender::new(con, schema, table)` (unsafe) | Create for `table` in `schema` (`None` = default) |
| `Appender::with_catalog(con, catalog, schema, table)` (unsafe) | Create fully qualified |
| `column_count()` / `column_type(i)` | The active column list |
| `add_column(name)` / `clear_columns()` | Narrow / reset the active column list |
| `row(closure)` | Append one row, calling `end_row` if the closure succeeds |
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
| `error_message()` | Message from the last failed operation (`Option<String>`) |
| `append_default_to_chunk(&chunk, col, row)` ¹ | Write a column's `DEFAULT` into a chunk cell |
| `clear()` ¹ | Discard buffered, unflushed rows (and un-poison the appender) |
| `error_data()` ¹ | Structured [`ErrorData`] from the last failed operation |

> ¹ Requires the `duckdb-1-5` feature flag.

Every fallible method returns `Result<_, AppendError>`. [`AppendError`] is
[`ErrorData`] (message **and** machine-readable category) when `duckdb-1-5` is
enabled, and [`ExtensionError`] (message only) otherwise — enabling the feature
upgrades the error type in place without changing any method's shape.

## Safety

`new` and `with_catalog` are `unsafe`: you must pass a valid, open
`duckdb_connection` (such as the one provided to your extension's entry point).

Both return `Result` for a reason: DuckDB's `duckdb_append_*` functions do not
check whether the appender was created successfully before dereferencing it,
so an appender whose creation failed must never be used. A failed create
returns `Err` and no `Appender`, so such an appender cannot be reached.

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
