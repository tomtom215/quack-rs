# Table Functions

Table functions implement the `SELECT * FROM my_function(args)` pattern — they
return a result set rather than a scalar value. DuckDB table functions have three
lifecycle callbacks: **bind**, **init**, and **scan**.

`quack-rs` provides two layers for registering table functions:

1. **`TypedTableFunctionBuilder<S>`** (recommended for new extensions) — closure-based
   API that hides bind/init/scan trampolines behind safe Rust closures and gives every
   execution a fresh, typed scan state built from what `bind` produced.
2. **`TableFunctionBuilder`** — the underlying raw builder used by `TypedTableFunctionBuilder`
   internally. Reach for it when you need fine-grained control: parallel scans
   (`InitInfo::set_max_threads` above 1, usually with `local_init` for per-thread
   state), projection pushdown with column filtering, or callback shapes that
   don't fit the "produce state in bind, mutate it in scan" model.

Both builders are backed by the helper types `BindInfo`, `InitInfo`, `FunctionInfo`,
`FfiBindData<T>`, `FfiInitData<T>`, and `FfiLocalInitData<T>`.

## Lifecycle

| Phase | Callback | Called when | Typical work |
|-------|----------|-------------|--------------|
| **bind** | `bind_fn` | Query is planned (once per plan) | Extract parameters; register output columns; store config in bind data |
| **init** | `init_fn` | Each execution of the plan starts | Allocate per-scan state (cursor, row index, etc.) |
| **scan** | `scan_fn` | Each output batch | Fill `duckdb_data_chunk` with rows; call `duckdb_data_chunk_set_size` |

The scan callback is called repeatedly until it writes 0 rows in a batch, signalling
end-of-results.

> **Bind once, init many times.** DuckDB keeps the bind data for as long as the
> bound plan lives and runs `init` against it on **every** execution: each
> `EXECUTE` of a prepared statement, each iteration of a recursive CTE that
> references the function. Treat bind data as immutable after bind and build
> anything a scan consumes (cursors, open files) in `init`.

## Closure-based typed state (`with_state`)

For the common "take parameters at bind, stream rows until exhausted" pattern,
`TypedTableFunctionBuilder<S>` replaces all three callback trampolines with two
closures. With `with_state`, the state returned by `bind` is a **template**: every
execution of the plan scans a fresh `clone()` of it, so `S` must be `Clone`.

```rust,no_run
use quack_rs::prelude::*;

#[derive(Clone)]
struct State {
    remaining: u64,
}

fn register(reg: &impl Registrar) -> ExtResult<()> {
    let builder = TableFunctionBuilder::new("count_down")
        .param(TypeId::BigInt)
        // 1. bind closure: declare the output schema, read parameters,
        //    return the template scan state (cloned for every execution).
        .with_state::<State, _>(|bind| {
            bind.add_result_column("n", TypeId::BigInt);
            let raw = unsafe { bind.get_parameter_value(0) };
            Ok(State { remaining: raw.as_i64_or(0).max(0) as u64 })
        })
        // 2. scan closure: mutate state, write rows, set chunk size.
        .scan(|state, chunk| {
            if state.remaining == 0 {
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            let mut writer = unsafe { chunk.writer(0) };
            unsafe { writer.write_i64(0, state.remaining as i64) };
            state.remaining -= 1;
            unsafe { chunk.set_size(1) };
            Ok(())
        })
        .build()?;
    unsafe { reg.register_table(builder) }
}
```

## Separate bind data and scan state (`with_bind_init`)

When the scan state is expensive or impossible to clone (it owns a file handle,
a large buffer, a connection), or the parameters and the cursor are naturally
separate, use `with_bind_init`. `bind` returns immutable bind data `B`
(`Send + Sync`); `init` builds a fresh scan state `S` from `&B` for every
execution:

```rust,no_run
use quack_rs::prelude::*;

struct Params { n: i64 }
struct Cursor { next: i64, end: i64 }   // no Clone needed

fn register(reg: &impl Registrar) -> ExtResult<()> {
    let builder = TableFunctionBuilder::new("count_up")
        .param(TypeId::BigInt)
        .with_bind_init(
            |bind| {
                bind.add_result_column("n", TypeId::BigInt);
                let n = unsafe { bind.get_parameter_value(0) }.as_i64_or(0);
                Ok(Params { n })
            },
            |params: &Params| Ok(Cursor { next: 1, end: params.n }),
        )
        .scan(|cursor, chunk| {
            if cursor.next > cursor.end {
                unsafe { chunk.set_size(0) };
                return Ok(());
            }
            unsafe {
                chunk.writer(0).write_i64(0, cursor.next);
                chunk.set_size(1);
            }
            cursor.next += 1;
            Ok(())
        })
        .build()?;
    unsafe { reg.register_table(builder) }
}
```

### What you get for free

- **No hand-written `unsafe extern "C" fn` trampolines.** `TypedTableFunctionBuilder`
  generates them internally.
- **Typed scan state.** The `scan` closure receives `&mut S`, freshly built for
  each execution (a clone of the `with_state` template, or `init(&B)` for
  `with_bind_init`) — no manual `FfiBindData` / `FfiInitData` shuffling, and a
  prepared statement can be executed any number of times.
- **Panic safety.** User closures run inside `catch_unwind`. Panics surface as
  `duckdb_bind/init/function_set_error`, and the scan forces chunk size to zero so
  the query terminates cleanly instead of unwinding across the FFI boundary.
- **Error propagation.** Return `Err(ExtensionError::new("..."))` from any closure
  to report a SQL error to DuckDB.

### Trade-offs and threading

- `S` must be `Send + 'static` (plus `Clone` for `with_state`). `Sync` is **not**
  required, so `TypedTableFunctionBuilder` forces scans to run on a single worker by
  calling `InitInfo::set_max_threads(1)` internally.
- The typed builder does **not** offer projection pushdown: with pushdown on, the
  scan's chunk holds only the projected columns and a closure written against the
  declared schema would write the wrong column. `build()` returns an error if
  `projection_pushdown(true)` was set on the raw builder before `with_state` /
  `with_bind_init`. Use the raw builder for pushdown.
- The bind closure must declare at least one column. With `duckdb-1-5`, a bind
  that declares none is reported as an ordinary bind error; DuckDB itself raises an
  `INTERNAL Error` with a C++ stack trace for it (and does so for a raw bind
  callback, which quack-rs cannot check after it returns — call `set_error`
  yourself).
- Extensions that need multi-worker parallelism (`set_max_threads` above 1, with
  `local_init` + thread-local buffers) should use the raw
  [`TableFunctionBuilder`](#builder-api) directly.
- `TypedTableFunctionBuilder::build()` returns a fully configured
  `TableFunctionBuilder`, so you can still pass it through any `Registrar`
  — including `MockRegistrar` for unit tests.

## Builder API

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info};
# unsafe extern "C" fn my_bind_callback(_: duckdb_bind_info) {}
# unsafe extern "C" fn my_init_callback(_: duckdb_init_info) {}
# unsafe extern "C" fn my_scan_callback(_: duckdb_function_info, _: duckdb_data_chunk) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
use quack_rs::table::{TableFunctionBuilder, BindInfo, FfiBindData, FfiInitData};
use quack_rs::types::TypeId;

TableFunctionBuilder::new("my_function")
    .param(TypeId::BigInt)                 // positional parameter types
    .bind(my_bind_callback)               // declare output columns inside bind
    .init(my_init_callback)
    .scan(my_scan_callback)
    .register(con)?;
# Ok(())
# }
```

Output columns are declared inside the bind callback using `BindInfo::add_result_column`,
not on the builder itself.

## State management

### Bind data

Bind data persists from the bind phase through all scan batches — and through every
later execution of the same plan (see *Bind once, init many times* above), possibly
read from several threads at once, so `FfiBindData::set` requires `T: Send + Sync`.
Use `FfiBindData<T>` to allocate it safely:

```rust
# use libduckdb_sys::duckdb_bind_info;
# use quack_rs::table::{BindInfo, FfiBindData};
struct MyBindData {
    limit: i64,
}

unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
    // `get_parameter_value` returns an RAII `Value`; a NULL argument reads as the default.
    let n = unsafe { BindInfo::new(info).get_parameter_value(0) }.as_i64_or(0);
    unsafe { FfiBindData::<MyBindData>::set(info, MyBindData { limit: n }) };
}
```

`FfiBindData::set` stores the value and registers a destructor so DuckDB frees
it at the right time — no `Box::into_raw` / `Box::from_raw` needed.

### Init (scan) state

Per-scan state (e.g., a current row index) uses `FfiInitData<T>` (`T: Send + Sync`,
since concurrent scan threads share it):

```rust
# use libduckdb_sys::duckdb_init_info;
# use quack_rs::table::FfiInitData;
struct MyScanState {
    pos: i64,
}

unsafe extern "C" fn my_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<MyScanState>::set(info, MyScanState { pos: 0 }) };
}
```

## Complete example: `generate_series_ext`

The `hello-ext` example registers `generate_series_ext(n BIGINT)` which emits
integers `0 .. n-1`. See `examples/hello-ext/src/lib.rs` for the full source.

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info, DuckDBSuccess};
# use quack_rs::data_chunk::DataChunk;
# use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, TableFunctionBuilder};
# use quack_rs::types::TypeId;
# struct GsBindData { total: i64 }
# struct GsScanState { pos: i64 }
// Bind: extract `n`, register one output column
unsafe extern "C" fn gs_bind(info: duckdb_bind_info) {
    let bind_info = unsafe { BindInfo::new(info) };
    // Value is RAII — automatically destroyed when dropped.
    // A NULL argument reads as the default rather than aborting.
    let n = unsafe { bind_info.get_parameter_value(0) }.as_i64_or(0);

    bind_info.add_result_column("value", TypeId::BigInt);
    unsafe { FfiBindData::<GsBindData>::set(info, GsBindData { total: n }) };
}

// Init: zero-initialise the scan cursor
unsafe extern "C" fn gs_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<GsScanState>::set(info, GsScanState { pos: 0 }) };
}

// Scan: emit a batch of rows using DataChunk wrapper
unsafe extern "C" fn gs_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    // Never unwrap in a callback: a missing state ends the scan instead.
    let bind = unsafe { FfiBindData::<GsBindData>::get_from_function(info) };
    let state = unsafe { FfiInitData::<GsScanState>::get_mut(info) };
    let (Some(bind), Some(state)) = (bind, state) else {
        unsafe { chunk.set_size(0) };
        return;
    };

    let remaining = bind.total - state.pos;
    let batch = remaining.min(2048).max(0) as usize;

    let mut writer = unsafe { chunk.writer(0) };
    for i in 0..batch {
        unsafe { writer.write_i64(i, state.pos + i as i64) };
    }
    unsafe { chunk.set_size(batch) };
    state.pos += batch as i64;
}
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let (mut db, mut con) = (std::ptr::null_mut(), std::ptr::null_mut());
# unsafe {
#     assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), DuckDBSuccess);
#     assert_eq!(libduckdb_sys::duckdb_connect(db, &mut con), DuckDBSuccess);
#     TableFunctionBuilder::new("generate_series_ext")
#         .param(TypeId::BigInt)
#         .bind(gs_bind)
#         .init(gs_init)
#         .scan(gs_scan)
#         .register(con)
#         .unwrap();
# }
# let sum = |sql: &str| -> i64 {
#     let mut result = unsafe { quack_rs::query::query(con, sql) }.unwrap();
#     let chunk = result.next_chunk().unwrap();
#     unsafe { chunk.reader(0).read_i64(0) }
# };
# assert_eq!(sum("SELECT sum(value)::BIGINT FROM generate_series_ext(5)"), 10);
# assert_eq!(sum("SELECT count(*) FROM generate_series_ext(5000)"), 5000);
```

## Registration

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info};
# use quack_rs::table::TableFunctionBuilder;
# use quack_rs::types::TypeId;
# unsafe extern "C" fn gs_bind(_: duckdb_bind_info) {}
# unsafe extern "C" fn gs_init(_: duckdb_init_info) {}
# unsafe extern "C" fn gs_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
TableFunctionBuilder::new("generate_series_ext")
    .param(TypeId::BigInt)
    .bind(gs_bind)
    .init(gs_init)
    .scan(gs_scan)
    .register(con)?;
# Ok(())
# }
```

## Advanced features

### Named parameters

Named parameters let callers pass optional arguments by name (e.g., `step := 10`):

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info};
# use quack_rs::table::TableFunctionBuilder;
# use quack_rs::types::TypeId;
# unsafe extern "C" fn gs_v2_bind(_: duckdb_bind_info) {}
# unsafe extern "C" fn gs_v2_init(_: duckdb_init_info) {}
# unsafe extern "C" fn gs_v2_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
TableFunctionBuilder::new("gen_series_v2")
    .param(TypeId::BigInt)                    // positional: n
    .named_param("step", TypeId::BigInt)      // named: step := <value>
    .bind(gs_v2_bind)
    .init(gs_v2_init)
    .scan(gs_v2_scan)
    .register(con)?;
# Ok(())
# }
```

In the bind callback, read the named parameter with
`BindInfo::get_named_parameter_value("step")`. Named parameters are optional: if the
query omits `step := …`, the returned `Value` wraps a null handle (`is_null()` is
`true`), so use a defaulting accessor such as `as_i64_or(1)`.

### Registering a name twice

The C API has no table function *sets*: a second registration under a name that
already exists — your own earlier one, another extension's, or a built-in such as
`range` — is dropped by DuckDB while `duckdb_register_table_function` still
reports success, and the old function keeps answering. `register` therefore checks
`duckdb_functions()` first and returns an error naming the conflict. Table
functions registered through the C API live in the in-memory system catalog and
are never persisted, so reloading an extension into a database file never trips
this check.

### Local init (per-thread state)

`local_init` allocates per-thread state for a scan that runs on several threads.
It does **not** make the scan parallel by itself — that is
`InitInfo::set_max_threads` (see [Thread control](#thread-control)):

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info};
# use quack_rs::table::TableFunctionBuilder;
# use quack_rs::types::TypeId;
# unsafe extern "C" fn gs_v2_bind(_: duckdb_bind_info) {}
# unsafe extern "C" fn gs_v2_init(_: duckdb_init_info) {}
# unsafe extern "C" fn gs_v2_local_init(_: duckdb_init_info) {}
# unsafe extern "C" fn gs_v2_scan(_: duckdb_function_info, _: duckdb_data_chunk) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
TableFunctionBuilder::new("gen_series_v2")
    .param(TypeId::BigInt)
    .bind(gs_v2_bind)
    .init(gs_v2_init)
    .local_init(gs_v2_local_init)            // per-thread state allocation
    .scan(gs_v2_scan)
    .register(con)?;
# Ok(())
# }
```

The local init callback receives `duckdb_init_info` and can use
`FfiLocalInitData<T>::set` to store per-thread state.

### Thread control

Use `InitInfo::set_max_threads` in the global init callback to tell DuckDB how
many threads can scan concurrently. The default is 1. Above 1, DuckDB calls the
scan from that many threads **at the same time whether or not `local_init` is
set** — and all of them share the one global init data and bind data. Do not use
`FfiInitData::get_mut` then; keep shared mutable state behind a `Mutex` or
atomics and read it with `FfiInitData::get`:

```rust
# use libduckdb_sys::duckdb_init_info;
# use quack_rs::table::{FfiInitData, InitInfo};
# struct MyState { pos: i64 }
unsafe extern "C" fn gs_v2_init(info: duckdb_init_info) {
    let init_info = unsafe { InitInfo::new(info) };
    init_info.set_max_threads(1);
    unsafe { FfiInitData::<MyState>::set(info, MyState { pos: 0 }) };
}
```

### Projection pushdown

Enable projection pushdown to let DuckDB skip unrequested columns:

```rust
# use quack_rs::table::TableFunctionBuilder;
# fn demo() {
# let _ =
TableFunctionBuilder::new("my_func")
    .projection_pushdown(true)
    // ...
# ;
# }
```

> **Caution:** When projection pushdown is enabled, your scan callback must check
> which columns DuckDB actually needs using `InitInfo::projected_column_count` and
> `InitInfo::projected_column_index`. Writing to non-projected columns causes crashes.
> `projected_column_index` returns `None` past the end of the projection (the C API
> itself answers `0` there, which is indistinguishable from the first column).

See `examples/hello-ext/src/lib.rs` for a complete example using `named_param`,
`local_init`, and `set_max_threads`.

### Complex parameter types

For parameterised types that `TypeId` cannot express (e.g. `LIST(BIGINT)`,
`MAP(VARCHAR, INTEGER)`, `STRUCT(...)`), use `param_logical` and
`named_param_logical`:

```rust
# use libduckdb_sys::{duckdb_bind_info, duckdb_connection, duckdb_data_chunk,
#     duckdb_function_info, duckdb_init_info};
# use quack_rs::table::TableFunctionBuilder;
# use quack_rs::types::TypeId;
# unsafe extern "C" fn bind_fn(_: duckdb_bind_info) {}
# unsafe extern "C" fn init_fn(_: duckdb_init_info) {}
# unsafe extern "C" fn scan_fn(_: duckdb_function_info, _: duckdb_data_chunk) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
use quack_rs::types::LogicalType;

TableFunctionBuilder::new("read_data")
    .param_logical(LogicalType::list(TypeId::Varchar))        // positional LIST param
    .named_param_logical("options", LogicalType::map(          // named MAP param
        TypeId::Varchar, TypeId::Varchar,
    ))
    .bind(bind_fn)
    .init(init_fn)
    .scan(scan_fn)
    .register(con)?;
# Ok(())
# }
```

### BindInfo helpers

`BindInfo` wraps `duckdb_bind_info` and exposes these methods:

| Method | Description |
|--------|-------------|
| `add_result_column(name, TypeId)` | Declares an output column (a type DuckDB would silently drop, like `ANY`, is a bind error instead) |
| `add_result_column_with_type(name, &LogicalType)` | Output column with complex type (same check, including nested `ANY`/`INVALID`) |
| `set_cardinality(rows, is_exact)` | Cardinality hint for the optimizer — DuckDB 1.5.5 records `is_exact = false` as estimate **and** upper bound, `true` as estimate only (the reverse of its header); see the rustdoc |
| `set_error(message)` | Report a bind-time error (an empty message is replaced by a placeholder) |
| `parameter_count()` | Number of positional parameters |
| `get_parameter(index)` | Returns a positional parameter value (`duckdb_value`) |
| `get_named_parameter(name)` | Returns a named parameter value (`duckdb_value`) |
| `get_extra_info()` | Returns the extra-info pointer set on the function |
| `get_client_context()` | Returns a `ClientContext` (requires `duckdb-1-5` feature) |

### InitInfo helpers

`InitInfo` wraps `duckdb_init_info`:

| Method | Description |
|--------|-------------|
| `projected_column_count()` | Number of projected columns (with pushdown) |
| `projected_column_index(idx)` | Declared column index at projection position; `None` when `idx` is out of range |
| `set_max_threads(n)` | Maximum concurrent scan threads (default 1; shared global state above 1) |
| `set_error(message)` | Report an init-time error (an empty message is replaced by a placeholder) |
| `get_extra_info()` | Returns the extra-info pointer set on the function |

### FunctionInfo helpers

`FunctionInfo` wraps `duckdb_function_info` (scan callbacks):

| Method | Description |
|--------|-------------|
| `set_error(message)` | Report a scan-time error (an empty message is replaced by a placeholder) |
| `get_extra_info()` | Returns the extra-info pointer set on the function |

### Extra info

Use `TableFunctionBuilder::extra_info` to attach function-level data that is
accessible from all callbacks (bind, init, and scan) via `get_extra_info()`. The
pointee must be `Send + Sync`: DuckDB passes the same pointer to callbacks running
on several threads at once, and frees it on whichever thread releases the function.

## Verified output (DuckDB 1.4.4 and 1.5.0)

```sql
SELECT * FROM generate_series_ext(5);
-- 0
-- 1
-- 2
-- 3
-- 4

SELECT value * value AS sq FROM generate_series_ext(4);
-- 0
-- 1
-- 4
-- 9
```

## See also

- [`table`](https://docs.rs/quack-rs/latest/quack_rs/table/index.html) module documentation
- [`replacement_scan`](replacement-scan.md) — for file-path-triggered table scans
- [`hello-ext` README](https://github.com/tomtom215/quack-rs/blob/main/examples/hello-ext/README.md)
