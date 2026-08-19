# Running SQL from an Extension

Extensions routinely need to talk SQL to the database that is loading them:
checking whether a table exists before registering a replacement scan, creating a
helper view, reading a setting, or looking up a credential through
`duckdb_secrets()`.

The C API has everything for that — `duckdb_query`, `duckdb_prepare`,
`duckdb_bind_*`, `duckdb_fetch_chunk` — and all of it is in the [stable
prefix](../concepts/abi.md), so it needs no feature flag. What it does not have
is any help releasing the handles: every one of them has a matching `destroy`
that must run exactly once, including on the error paths, which is where
hand-written FFI leaks.

`quack_rs::query` wraps them:

| Type | Owns | Released by |
|------|------|-------------|
| `QueryResult` | `duckdb_result` | `duckdb_destroy_result` |
| `OwnedDataChunk` | `duckdb_data_chunk` | `duckdb_destroy_data_chunk` |
| `PreparedStatement` | `duckdb_prepared_statement` | `duckdb_destroy_prepare` |
| `OwnedConnection` | `duckdb_connection` | `duckdb_disconnect` |

## During registration

`Connection` (from `entry_point_v2!`) can run SQL directly:

```rust,ignore
use quack_rs::connection::Connection;
use quack_rs::error::ExtensionError;

fn register(con: &Connection) -> Result<(), ExtensionError> {
    // Create a helper view the extension's functions rely on.
    unsafe { con.execute("CREATE OR REPLACE VIEW my_ext_config AS SELECT 1 AS version") }?;

    // Read something back.
    let mut result = unsafe { con.query("SELECT current_setting('threads')") }?;
    if let Some(chunk) = result.next_chunk() {
        let threads = unsafe { chunk.reader(0).read_str(0) };
        eprintln!("DuckDB is using {threads} threads");
    }
    Ok(())
}
```

Results arrive a chunk at a time — at most `duckdb_vector_size()` rows each — so
call `next_chunk` until it returns `None`:

```rust,ignore
let mut result = unsafe { con.query("SELECT i FROM range(10000) t(i)") }?;
let mut total: i64 = 0;
while let Some(chunk) = result.next_chunk() {
    let reader = unsafe { chunk.reader(0) };
    for row in 0..chunk.size() {
        total += unsafe { reader.read_i64(row) };
    }
}
```

## Bind values, do not interpolate them

Anything that did not come from your own source text — a table name from a
function argument, a path from a config option — goes through a parameter.
Parameters are 1-indexed, matching the C API.

```rust,ignore
let stmt = unsafe { con.prepare("INSERT INTO audit VALUES (?, ?)") }?;
stmt.bind_str(1, user_supplied_name)?;   // safe even if it contains quotes
stmt.bind_i64(2, 42)?;
stmt.execute()?;
```

`bind_str` passes the length explicitly, so embedded NUL bytes are preserved and
no `CString` conversion can fail. Named parameters resolve by name:

```rust,ignore
let stmt = unsafe { con.prepare("SELECT * FROM t WHERE id = $needle") }?;
let index = stmt.parameter_index("needle").expect("named parameter");
stmt.bind_i64(index, id)?;
```

Reuse a statement by clearing its bindings between executions:

```rust,ignore
for id in ids {
    stmt.clear_bindings()?;
    stmt.bind_i64(1, id)?;
    let mut result = stmt.execute()?;
    // …
}
```

## After registration

The connection DuckDB passes to your entry point is **borrowed** — the entry
point disconnects it when your closure returns. Inside a scalar, table or
aggregate callback you have no connection at all: the C API gives you a
`duckdb_client_context`, and there is no `duckdb_client_context_get_connection`.

If a callback or a background thread needs to run SQL, open your own connection
during registration and keep it. A `duckdb_connection` holds its own reference to
the database instance, so it stays valid after loading finishes:

```rust,ignore
use quack_rs::query::OwnedConnection;
use std::sync::OnceLock;

static CONN: OnceLock<OwnedConnection> = OnceLock::new();

fn register(con: &Connection) -> Result<(), ExtensionError> {
    let owned = unsafe { con.open_connection() }?;
    let _ = CONN.set(owned);
    Ok(())
}
```

`OwnedConnection` is `Send` but deliberately not `Sync`: DuckDB permits moving a
connection between threads, not using one concurrently. Open one connection per
thread, or guard it with a mutex.

## Errors

Failures carry DuckDB's own message:

```rust,ignore
let err = unsafe { con.query("SELECT * FROM no_such_table") }.unwrap_err();
assert!(err.as_str().contains("no_such_table"));
```

The connection stays usable afterwards.
