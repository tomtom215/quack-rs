# Table Functions

Table functions implement the `SELECT * FROM my_function(args)` pattern — they
return a result set rather than a scalar value. DuckDB table functions have three
lifecycle callbacks: **bind**, **init**, and **scan**.

`quack-rs` provides `TableFunctionBuilder` plus the helper types `BindInfo`,
`FfiBindData<T>`, and `FfiInitData<T>` to eliminate the raw FFI boilerplate.

## Lifecycle

| Phase | Callback | Called when | Typical work |
|-------|----------|-------------|--------------|
| **bind** | `bind_fn` | Query is planned | Extract parameters; register output columns; store config in bind data |
| **init** | `init_fn` | Execution starts | Allocate per-scan state (cursor, row index, etc.) |
| **scan** | `scan_fn` | Each output batch | Fill `duckdb_data_chunk` with rows; call `duckdb_data_chunk_set_size` |

The scan callback is called repeatedly until it writes 0 rows in a batch, signalling
end-of-results.

## Builder API

```rust
use quack_rs::table::{TableFunctionBuilder, BindInfo, FfiBindData, FfiInitData};
use quack_rs::types::TypeId;

TableFunctionBuilder::new("my_function")
    .add_parameter(TypeId::BigInt)          // positional parameter types
    .add_result_column("value", TypeId::BigInt) // output columns
    .bind(my_bind_callback)
    .init(my_init_callback)
    .scan(my_scan_callback)
    .register(con)?;
```

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
    let param = unsafe { duckdb_bind_get_parameter(info, 0) };
    let n = unsafe { duckdb_get_int64(param) };
    unsafe { duckdb_destroy_value(&mut { param }) };

    let out_type = LogicalType::new(TypeId::BigInt);
    unsafe { duckdb_bind_add_result_column(info, c"value".as_ptr(), out_type.as_raw()) };

    unsafe { FfiBindData::<GsBindData>::set(info, GsBindData { total: n }) };
}

// Init: zero-initialise the scan cursor
unsafe extern "C" fn gs_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<GsScanState>::set(info, GsScanState { pos: 0 }) };
}

// Scan: emit a batch of rows
unsafe extern "C" fn gs_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let bind = unsafe { FfiBindData::<GsBindData>::get(duckdb_function_get_bind_data(info)) };
    let state = unsafe { FfiInitData::<GsScanState>::get_mut(duckdb_function_get_init_data(info)) };

    let remaining = bind.total - state.pos;
    let batch = remaining.min(2048).max(0) as usize;

    let mut writer = unsafe { VectorWriter::new(duckdb_data_chunk_get_vector(output, 0)) };
    for i in 0..batch {
        unsafe { writer.write_i64(i, state.pos + i as i64) };
    }
    unsafe { duckdb_data_chunk_set_size(output, batch as idx_t) };
    state.pos += batch as i64;
}
```

## Registration

```rust
TableFunctionBuilder::new("generate_series_ext")
    .add_parameter(TypeId::BigInt)
    .add_result_column("value", TypeId::BigInt)
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

- [`table`](../../src/table/mod.rs) module documentation
- [`replacement_scan`](replacement-scan.md) — for file-path-triggered table scans
- [`hello-ext` README](../../examples/hello-ext/README.md)
