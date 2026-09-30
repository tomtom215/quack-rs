# Running SQL from an Extension

Extensions often need to run SQL against the database that is loading them:
checking whether a table exists before registering a replacement scan, creating a
helper view, reading a setting, or looking up a credential through
`duckdb_secrets()`. This page covers queries, prepared statements with bound
parameters, keeping a connection for use after loading, and cancelling a
running query.

The C API has everything for that — `duckdb_query`, `duckdb_prepare`,
`duckdb_bind_*`, `duckdb_fetch_chunk` — and all of it is in the [stable
prefix](../concepts/abi.md), so it needs no feature flag. What it does not have
is any help releasing the handles: every one of them has a matching `destroy`
that must run exactly once, including on the error paths, which is where
hand-written FFI usually leaks.

`quack_rs::query` wraps them:

| Type | Owns | Released by |
|------|------|-------------|
| `QueryResult` | `duckdb_result` | `duckdb_destroy_result` |
| `OwnedDataChunk` | `duckdb_data_chunk` | `duckdb_destroy_data_chunk` |
| `PreparedStatement` | `duckdb_prepared_statement` | `duckdb_destroy_prepare` |
| `OwnedConnection` | `duckdb_connection` | `duckdb_disconnect` |

## During registration

`Connection` (from `entry_point_v2!`) can run SQL directly:

```rust
use quack_rs::connection::Connection;
use quack_rs::error::ExtensionError;

fn register(con: &Connection) -> Result<(), ExtensionError> {
    // Create a helper view the extension's functions rely on.
    unsafe { con.execute("CREATE OR REPLACE VIEW my_ext_config AS SELECT 1 AS version") }?;

    // Read something back. The cast makes the column VARCHAR, which read_str requires.
    let mut result = unsafe { con.query("SELECT current_setting('threads')::VARCHAR") }?;
    if let Some(chunk) = result.next_chunk()? {
        // The reader must outlive the `&str` it hands out, so bind it first.
        let reader = unsafe { chunk.reader(0) };
        let threads = unsafe { reader.read_str(0) };
        eprintln!("DuckDB is using {threads} threads");
    }
    Ok(())
}
```

Results arrive a chunk at a time — at most `duckdb_vector_size()` rows each — so
call `next_chunk` until it returns `Ok(None)`:

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::OwnedConnection;
# // `Connection` (from entry_point_v2!) only exists during an extension load. An
# // `OwnedConnection` has the same query / execute / prepare methods.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# let run = || -> Result<(), ExtensionError> {
let mut result = unsafe { con.query("SELECT i FROM range(10000) t(i)") }?;
let mut total: i64 = 0;
while let Some(chunk) = result.next_chunk()? {
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        total += unsafe { reader.read_i64(row) };
    }
}
# assert_eq!(total, 49_995_000);
# Ok(()) };
# run().unwrap();
```

`next_chunk` returns `Result<Option<OwnedDataChunk>, ExtensionError>` because a
result can stop early. A **streaming** result
(`PreparedStatement::execute_streaming`, with the `duckdb-1-5` feature) produces
rows as it runs, so a runtime error part-way through, an interrupt, or another
statement run on the same connection (which invalidates the stream) surfaces at
`next_chunk`. The C API reports that the same way as the end of the rows (a
null chunk); quack-rs reads the error DuckDB recorded and returns it, so a
partial result cannot pass for a complete one. The `?` above is what keeps it
from being silently truncated. After `Ok(None)` or an error, later calls return
the same thing.

### Inspecting a result

| Method | Returns |
|--------|---------|
| `column_count()` | Number of columns |
| `column_name(i)` | Name of column `i` (`Option<String>`) |
| `column_type(i)` | Top-level `TypeId` of column `i` |
| `column_logical_type(i)` | Full `LogicalType` of column `i`, keeping `STRUCT` fields, `LIST` element type, `DECIMAL` width and scale |
| `result_kind()` | `ResultKind::Rows`, `ChangedRows`, `Nothing` or `Invalid` |
| `rows_changed()` | Rows changed by an `INSERT` / `UPDATE` / `DELETE`; 0 for other statements |
| `is_streaming()` | Whether the result is streaming (`duckdb-1-5`) |

### Several statements in one string

`query` and `execute` accept several `;`-separated statements, and DuckDB runs
**every one**, in order. The result you get back is the first statement that
produces rows — or, when none does, the last statement's; the results of later
row-producing statements are discarded. So `"SELECT 1; INSERT …"` runs the
`INSERT` but `execute` reports `0` rows changed. The first failing statement
fails the call, after the ones before it have run (and, outside an explicit
transaction, committed). An empty string, or just `;`, succeeds with an empty
result. `prepare` takes exactly one statement.

## Bind values, do not interpolate them

Anything that did not come from your own source text — a table name from a
function argument, a path from a config option — goes through a parameter.
Parameters are 1-indexed, matching the C API.

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::OwnedConnection;
# // `Connection` (from entry_point_v2!) only exists during an extension load. An
# // `OwnedConnection` has the same query / execute / prepare methods.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# con.execute("CREATE TABLE audit (name VARCHAR, n BIGINT)").unwrap();
# let user_supplied_name = "O'Brien'); DROP TABLE audit; --";
# let run = || -> Result<(), ExtensionError> {
let stmt = unsafe { con.prepare("INSERT INTO audit VALUES (?, ?)") }?;
stmt.bind_str(1, user_supplied_name)?;   // safe even if it contains quotes
stmt.bind_i64(2, 42)?;
stmt.execute()?;
# let mut r = con.query("SELECT count(*) FROM audit WHERE name LIKE 'O''Brien%'")?;
# assert_eq!(unsafe { r.next_chunk()?.unwrap().reader(0).read_i64(0) }, 1);
# Ok(()) };
# run().unwrap();
```

`bind_str` passes the length explicitly, so embedded NUL bytes are preserved and
no `CString` conversion can fail.

There is a typed bind for every integer width (`bind_i8` … `bind_u128`),
`bind_f32` / `bind_f64`, `bind_bool`, `bind_blob`, `bind_null`, `bind_decimal`,
`bind_date`, `bind_time`, `bind_timestamp`, `bind_timestamp_tz` and
`bind_interval`. `bind_value` takes any [`Value`](values-and-parameters.md),
which covers the composite types. Like the `Value` constructors, `bind_decimal`
validates width, scale and digit count, and `bind_time` and the timestamp
binds refuse a payload outside the range DuckDB's SQL produces; the C API's
`duckdb_bind_*` functions check nothing.

Named parameters resolve by name:

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::OwnedConnection;
# // `Connection` (from entry_point_v2!) only exists during an extension load. An
# // `OwnedConnection` has the same query / execute / prepare methods.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# con.execute("CREATE TABLE t AS SELECT range AS id FROM range(10)").unwrap();
# let id = 7;
# let run = || -> Result<(), ExtensionError> {
let stmt = unsafe { con.prepare("SELECT * FROM t WHERE id = $needle") }?;
let index = stmt.parameter_index("needle").expect("named parameter");
stmt.bind_i64(index, id)?;
# let mut r = stmt.execute()?;
# assert_eq!(unsafe { r.next_chunk()?.unwrap().reader(0).read_i64(0) }, 7);
# Ok(()) };
# run().unwrap();
```

Reuse a statement by clearing its bindings between executions:

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::OwnedConnection;
# // `Connection` (from entry_point_v2!) only exists during an extension load. An
# // `OwnedConnection` has the same query / execute / prepare methods.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
# let run = || -> Result<(), ExtensionError> {
# let stmt = con.prepare("SELECT ?::BIGINT * 2")?;
# let ids = [1_i64, 2, 3];
for id in ids {
    stmt.clear_bindings()?;
    stmt.bind_i64(1, id)?;
    let mut result = stmt.execute()?;
    // …
}
# Ok(()) };
# run().unwrap();
```

## After registration

The connection DuckDB passes to your entry point is **borrowed** — the entry
point disconnects it when your closure returns. Inside a scalar, table or
aggregate callback you have no connection at all: the C API gives you a
`duckdb_client_context`, and there is no `duckdb_client_context_get_connection`.

If a callback or a background thread needs to run SQL, open your own connection
during registration and keep it. A `duckdb_connection` holds its own reference to
the database instance, so it stays valid after loading finishes:

```rust
use quack_rs::connection::Connection;
use quack_rs::error::ExtensionError;
use quack_rs::query::OwnedConnection;
use std::sync::{Mutex, OnceLock};

// `OwnedConnection` is `Send` but not `Sync` (see below), so a `static` must
// hold it behind a `Mutex`.
static CONN: OnceLock<Mutex<OwnedConnection>> = OnceLock::new();

fn register(con: &Connection) -> Result<(), ExtensionError> {
    let owned = unsafe { con.open_connection() }?;
    let _ = CONN.set(Mutex::new(owned));
    Ok(())
}
```

`OwnedConnection` is `Send` but deliberately not `Sync`: DuckDB permits moving a
connection between threads, not using one concurrently. Open one connection per
thread, or guard it with a mutex.

## Cancelling a query and reading its progress

`OwnedConnection::interrupt_handle` returns an `InterruptHandle`, which is
`Send + Sync` and borrows the connection, so it cannot outlive it. Another
thread can call its `cancel()` to stop the running query, which then fails with
an interrupt error at DuckDB's next check, or its `progress()` to read a
`QueryProgress` (`percentage`, `rows_processed`, `total_rows_to_process`).
`percentage` is `-1.0` when DuckDB cannot report progress, for example when
the progress bar is disabled (`SET enable_progress_bar = true`).
`OwnedConnection::interrupt` and `progress` do the same on the calling thread.

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::{OwnedConnection, QueryResult};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

fn query_with_timeout(con: &OwnedConnection, sql: &str) -> Result<QueryResult, ExtensionError> {
    let watchdog = con.interrupt_handle();
    let (done, finished) = channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            // Cancel the query if it has not finished within 30 seconds.
            if let Err(RecvTimeoutError::Timeout) = finished.recv_timeout(Duration::from_secs(30)) {
                watchdog.cancel();
            }
        });
        let result = con.query(sql);
        drop(done); // wakes the watchdog
        result
    })
}
```

## Errors

Failures carry DuckDB's own message:

```rust
# use quack_rs::error::ExtensionError;
# use quack_rs::query::OwnedConnection;
# // `Connection` (from entry_point_v2!) only exists during an extension load. An
# // `OwnedConnection` has the same query / execute / prepare methods.
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let mut db = std::ptr::null_mut();
# unsafe { assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), libduckdb_sys::DuckDBSuccess); }
# let con = unsafe { OwnedConnection::open(db) }.unwrap();
let err = unsafe { con.query("SELECT * FROM no_such_table") }.unwrap_err();
assert!(err.as_str().contains("no_such_table"));
# assert_eq!(con.execute("SELECT 1").unwrap(), 0);
```

The connection stays usable afterwards.
