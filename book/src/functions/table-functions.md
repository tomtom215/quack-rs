# Table Functions

Table functions implement the `SELECT * FROM my_function(args)` pattern — they
return a result set rather than a scalar value. DuckDB table functions have three
lifecycle callbacks: **bind**, **init**, and **scan**.

`quack-rs` provides two layers for registering table functions:

1. **`TypedTableFunctionBuilder<S>`** (recommended for new extensions) — closure-based
   API that hides bind/init/scan trampolines behind two safe Rust closures and carries
   a typed scan state from `bind` into `scan` for you.
2. **`TableFunctionBuilder`** — the underlying raw builder used by `TypedTableFunctionBuilder`
   internally. Reach for it when you need fine-grained control: `local_init`-driven
   parallel scans, projection pushdown with column filtering, or callback shapes that
   don't fit the "produce state in bind, mutate it in scan" model.

Both builders are backed by the helper types `BindInfo`, `InitInfo`, `FunctionInfo`,
`FfiBindData<T>`, `FfiInitData<T>`, and `FfiLocalInitData<T>`.

## Lifecycle

| Phase | Callback | Called when | Typical work |
|-------|----------|-------------|--------------|
| **bind** | `bind_fn` | Query is planned | Extract parameters; register output columns; store config in bind data |
| **init** | `init_fn` | Execution starts | Allocate per-scan state (cursor, row index, etc.) |
| **scan** | `scan_fn` | Each output batch | Fill `duckdb_data_chunk` with rows; call `duckdb_data_chunk_set_size` |

The scan callback is called repeatedly until it writes 0 rows in a batch, signalling
end-of-results.

## Closure-based typed state (`with_state`)

For the common "take parameters at bind, stream rows until exhausted" pattern,
`TypedTableFunctionBuilder<S>` replaces all three callback trampolines with two
closures:

```rust,no_run
use quack_rs::prelude::*;

struct State {
    remaining: u64,
}

fn register(reg: &impl Registrar) -> ExtResult<()> {
    let builder = TableFunctionBuilder::new("count_down")
        .param(TypeId::BigInt)
        // 1. bind closure: declare the output schema, read parameters,
        //    return the initial scan state.
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

### What you get for free

- **No hand-written `unsafe extern "C" fn` trampolines.** `TypedTableFunctionBuilder`
  generates them internally.
- **Typed scan state.** The `bind` closure returns `S`; the `scan` closure receives
  `&mut S`. State is moved from the bind phase into init data for you — no manual
  `FfiBindData` / `FfiInitData` shuffling.
- **Panic safety.** User closures run inside `catch_unwind`. Panics surface as
  `duckdb_bind/init/function_set_error`, and the scan forces chunk size to zero so
  the query terminates cleanly instead of unwinding across the FFI boundary.
- **Error propagation.** Return `Err(ExtensionError::new("..."))` from either closure
  to report a SQL error to DuckDB.

### Trade-offs and threading

- `S` must be `Send + 'static`. `Sync` is **not** required, so
  `TypedTableFunctionBuilder` forces scans to run on a single worker by calling
  `InitInfo::set_max_threads(1)` internally.
- Extensions that need multi-worker parallelism (`local_init` + thread-local buffers)
  should use the raw [`TableFunctionBuilder`](#builder-api) directly.
- `TypedTableFunctionBuilder::build()` returns a fully configured
  `TableFunctionBuilder`, so you can still pass it through any `Registrar`
  — including `MockRegistrar` for unit tests.

## Builder API

```rust
use quack_rs::table::{TableFunctionBuilder, BindInfo, FfiBindData, FfiInitData};
use quack_rs::types::TypeId;

TableFunctionBuilder::new("my_function")
    .param(TypeId::BigInt)                 // positional parameter types
    .bind(my_bind_callback)               // declare output columns inside bind
    .init(my_init_callback)
    .scan(my_scan_callback)
    .register(con)?;
```

Output columns are declared inside the bind callback using `BindInfo::add_result_column`,
not on the builder itself.

## State management

### Bind data

Bind data persists from the bind phase through all scan batches. Use
`FfiBindData<T>` to allocate it safely:

```rust
struct MyBindData {
    limit: i64,
}

unsafe extern "C" fn my_bind(info: duckdb_bind_info) {
    let n = unsafe { duckdb_get_int64(duckdb_bind_get_parameter(info, 0)) };
    unsafe { FfiBindData::<MyBindData>::set(info, MyBindData { limit: n }) };
}
```

`FfiBindData::set` stores the value and registers a destructor so DuckDB frees
it at the right time — no `Box::into_raw` / `Box::from_raw` needed.

### Init (scan) state

Per-scan state (e.g., a current row index) uses `FfiInitData<T>`:

```rust
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
// Bind: extract `n`, register one output column
unsafe extern "C" fn gs_bind(info: duckdb_bind_info) {
    let bind_info = unsafe { BindInfo::new(info) };
    // Value is RAII — automatically destroyed when dropped
    let n = unsafe { bind_info.get_parameter_value(0) }.as_i64();

    bind_info.add_result_column("value", TypeId::BigInt);
    unsafe { FfiBindData::<GsBindData>::set(info, GsBindData { total: n }) };
}

// Init: zero-initialise the scan cursor
unsafe extern "C" fn gs_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<GsScanState>::set(info, GsScanState { pos: 0 }) };
}

// Scan: emit a batch of rows using DataChunk wrapper
unsafe extern "C" fn gs_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let bind = unsafe { FfiBindData::<GsBindData>::get_from_function(info) }.unwrap();
    let state = unsafe { FfiInitData::<GsScanState>::get_mut(info) }.unwrap();

    let remaining = bind.total - state.pos;
    let batch = remaining.min(2048).max(0) as usize;

    let chunk = unsafe { DataChunk::from_raw(output) };
    let mut writer = unsafe { chunk.writer(0) };
    for i in 0..batch {
        unsafe { writer.write_i64(i, state.pos + i as i64) };
    }
    unsafe { chunk.set_size(batch) };
    state.pos += batch as i64;
}
```

## Registration

```rust
TableFunctionBuilder::new("generate_series_ext")
    .param(TypeId::BigInt)
    .bind(gs_bind)
    .init(gs_init)
    .scan(gs_scan)
    .register(con)?;
```

## Advanced features

### Named parameters

Named parameters let callers pass optional arguments by name (e.g., `step := 10`):

```rust
TableFunctionBuilder::new("gen_series_v2")
    .param(TypeId::BigInt)                    // positional: n
    .named_param("step", TypeId::BigInt)      // named: step := <value>
    .bind(gs_v2_bind)
    .init(gs_v2_init)
    .scan(gs_v2_scan)
    .register(con)?;
```

In the bind callback, read the named parameter with
`duckdb_bind_get_named_parameter(info, c"step".as_ptr())`.

### Local init (per-thread state)

For multi-threaded table functions, use `local_init` to allocate per-thread state:

```rust
TableFunctionBuilder::new("gen_series_v2")
    .param(TypeId::BigInt)
    .bind(gs_v2_bind)
    .init(gs_v2_init)
    .local_init(gs_v2_local_init)            // per-thread state allocation
    .scan(gs_v2_scan)
    .register(con)?;
```

The local init callback receives `duckdb_init_info` and can use
`FfiLocalInitData<T>::set` to store per-thread state.

### Thread control

Use `InitInfo::set_max_threads` in the global init callback to tell DuckDB how
many threads can scan concurrently:

```rust
unsafe extern "C" fn gs_v2_init(info: duckdb_init_info) {
    let init_info = unsafe { InitInfo::new(info) };
    unsafe { init_info.set_max_threads(1) };
    unsafe { FfiInitData::<MyState>::set(info, MyState { pos: 0 }) };
}
```

### Projection pushdown

Enable projection pushdown to let DuckDB skip unrequested columns:

```rust
TableFunctionBuilder::new("my_func")
    .projection_pushdown(true)
    // ...
```

> **Caution:** When projection pushdown is enabled, your scan callback must check
> which columns DuckDB actually needs using `InitInfo::projected_column_count` and
> `InitInfo::projected_column_index`. Writing to non-projected columns causes crashes.

See `examples/hello-ext/src/lib.rs` for a complete example using `named_param`,
`local_init`, and `set_max_threads`.

### Complex parameter types

For parameterised types that `TypeId` cannot express (e.g. `LIST(BIGINT)`,
`MAP(VARCHAR, INTEGER)`, `STRUCT(...)`), use `param_logical` and
`named_param_logical`:

```rust
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
```

### BindInfo helpers

`BindInfo` wraps `duckdb_bind_info` and exposes these methods:

| Method | Description |
|--------|-------------|
| `add_result_column(name, TypeId)` | Declares an output column |
| `add_result_column_with_type(name, &LogicalType)` | Output column with complex type |
| `set_cardinality(rows, is_exact)` | Cardinality hint for the optimizer |
| `set_error(message)` | Report a bind-time error |
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
| `projected_column_index(idx)` | Output column index at projection position |
| `set_max_threads(n)` | Maximum parallel scan threads |
| `set_error(message)` | Report an init-time error |
| `get_extra_info()` | Returns the extra-info pointer set on the function |

### FunctionInfo helpers

`FunctionInfo` wraps `duckdb_function_info` (scan callbacks):

| Method | Description |
|--------|-------------|
| `set_error(message)` | Report a scan-time error |
| `get_extra_info()` | Returns the extra-info pointer set on the function |

### Extra info

Use `TableFunctionBuilder::extra_info` to attach function-level data that is
accessible from all callbacks (bind, init, and scan) via `get_extra_info()`.

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
