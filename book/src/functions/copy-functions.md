# Copy Functions

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

Copy functions let you implement custom `COPY TO` file format handlers. When a
user runs `COPY table TO 'file.xyz' (FORMAT my_format)`, DuckDB invokes your
extension's bind, init, sink, and finalize callbacks.

## Lifecycle

1. **Bind** — called once. Inspect output columns, configure the export.
2. **Global init** — called once. Open the output file, allocate global state.
3. **Sink** — called once per data chunk. Write rows to the output.
4. **Finalize** — called once. Flush buffers, close the file.

## Builder API

```rust,ignore
use quack_rs::copy_function::CopyFunctionBuilder;

let builder = CopyFunctionBuilder::try_new("my_format")?
    .bind(my_bind_fn)
    .global_init(my_global_init_fn)
    .sink(my_sink_fn)
    .finalize(my_finalize_fn);

// Register on a connection (inside entry_point_v2! callback):
// unsafe { builder.register(con)?; }
# Ok::<(), quack_rs::error::ExtensionError>(())
```

## Callback signatures

| Phase | Signature |
|-------|-----------|
| Bind | `unsafe extern "C" fn(info: duckdb_copy_function_bind_info)` |
| Global init | `unsafe extern "C" fn(info: duckdb_copy_function_global_init_info)` |
| Sink | `unsafe extern "C" fn(info: duckdb_copy_function_sink_info, chunk: duckdb_data_chunk)` |
| Finalize | `unsafe extern "C" fn(info: duckdb_copy_function_finalize_info)` |

## Callback info wrappers

Each phase provides an ergonomic wrapper type around its raw info handle. Wrap
the handle at the top of your callback to access helper methods:

### `CopyBindInfo`

| Method | Description |
|--------|-------------|
| `column_count()` | Number of output columns |
| `column_type(index)` | `LogicalType` of the column at `index` |
| `get_extra_info()` | Extra-info pointer set on the copy function |
| `set_bind_data(data, destroy)` | Store bind data and its destructor |
| `set_error(message)` | Report a bind-time error |
| `get_client_context()` | Returns a `ClientContext` for catalog/config access |

### `CopyGlobalInitInfo`

| Method | Description |
|--------|-------------|
| `get_bind_data()` | Retrieve the bind data pointer |
| `get_extra_info()` | Extra-info pointer set on the copy function |
| `get_file_path()` | Output file path for the COPY operation |
| `set_global_state(state, destroy)` | Store global state and its destructor |
| `set_error(message)` | Report an init-time error |
| `get_client_context()` | Returns a `ClientContext` |

### `CopySinkInfo`

| Method | Description |
|--------|-------------|
| `get_bind_data()` | Retrieve the bind data pointer |
| `get_extra_info()` | Extra-info pointer set on the copy function |
| `get_global_state()` | Retrieve the global state pointer |
| `set_error(message)` | Report a sink-time error |
| `get_client_context()` | Returns a `ClientContext` |

### `CopyFinalizeInfo`

| Method | Description |
|--------|-------------|
| `get_bind_data()` | Retrieve the bind data pointer |
| `get_extra_info()` | Extra-info pointer set on the copy function |
| `get_global_state()` | Retrieve the global state pointer |
| `set_error(message)` | Report a finalize-time error |
| `get_client_context()` | Returns a `ClientContext` |

All four wrappers are re-exported from `quack_rs::copy_function`:

```rust
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
use quack_rs::copy_function::{CopyBindInfo, CopyGlobalInitInfo, CopySinkInfo, CopyFinalizeInfo};
```

## Related modules

- [`config_option`](https://docs.rs/quack-rs/latest/quack_rs/config_option/) — register custom settings for your format
- [`client_context`](https://docs.rs/quack-rs/latest/quack_rs/client_context/) — access the file system and catalog from callbacks
- [`table_description`](https://docs.rs/quack-rs/latest/quack_rs/table_description/) — inspect table metadata
- [`catalog`](https://docs.rs/quack-rs/latest/quack_rs/catalog/) — look up catalog entries
