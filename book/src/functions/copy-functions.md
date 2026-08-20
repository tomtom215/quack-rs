# Copy Functions

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

Copy functions let you implement a custom file format for `COPY`. A format can
support writing, reading, or both:

| Direction | You supply | DuckDB calls |
|---|---|---|
| `COPY t TO 'f' (FORMAT my_format)` | `bind` + `sink` + `finalize` (and optionally `global_init`) | your four callbacks |
| `COPY t FROM 'f' (FORMAT my_format)` | `copy_from(table_function)` | your **table function**'s bind, init and scan |

`duckdb_register_copy_function` decides which directions a format supports by
looking at the sink and the reader independently, so a read-only format leaves
the writing callbacks unset entirely.

## Lifecycle (`COPY … TO`)

1. **Bind** — called once. Inspect output columns, configure the export.
2. **Global init** — called once. Open the output file, allocate global state.
3. **Sink** — called once per data chunk. Write rows to the output.
4. **Finalize** — called once. Flush buffers, close the file.

## Builder API

```rust,no_run
use quack_rs::copy_function::CopyFunctionBuilder;

# fn demo(
#     my_bind_fn: quack_rs::copy_function::CopyBindFn,
#     my_global_init_fn: quack_rs::copy_function::CopyGlobalInitFn,
#     my_sink_fn: quack_rs::copy_function::CopySinkFn,
#     my_finalize_fn: quack_rs::copy_function::CopyFinalizeFn,
# ) -> Result<(), quack_rs::error::ExtensionError> {
let builder = CopyFunctionBuilder::try_new("my_format")?
    .bind(my_bind_fn)
    .global_init(my_global_init_fn)
    .sink(my_sink_fn)
    .finalize(my_finalize_fn);

// Register on a connection (inside entry_point_v2! callback):
// unsafe { builder.register(con)?; }
# Ok(())
# }
```

## `COPY … FROM`

Reading is a **table function**, attached to the copy function rather than
registered on its own. Build it with
[`TableFunctionBuilder::build_handle`](https://docs.rs/quack-rs/latest/quack_rs/table/struct.TableFunctionBuilder.html#method.build_handle),
then hand it to `copy_from`:

```rust,no_run
use quack_rs::copy_function::CopyFunctionBuilder;
use quack_rs::table::TableFunctionBuilder;
use quack_rs::types::TypeId;

# fn demo(
#     bind: quack_rs::table::BindFn,
#     init: quack_rs::table::InitFn,
#     scan: quack_rs::table::ScanFn,
# ) -> Result<(), quack_rs::error::ExtensionError> {
// SAFETY: the callbacks match their declared signatures.
let reader = unsafe {
    TableFunctionBuilder::new("my_format")   // name it after the format
        .param(TypeId::Varchar)              // the file path — exactly one
        .named_param("skip_rows", TypeId::BigInt) // a COPY option
        .bind(bind)
        .init(init)
        .scan(scan)
        .build_handle()
}?;

let format = CopyFunctionBuilder::try_new("my_format")?.copy_from(reader)?;
// unsafe { format.register(con)?; }
# Ok(())
# }
```

Three things about the reader are not like an ordinary table function:

- **The file path is positional parameter 0**, always `VARCHAR`. `duckdb.h`
  requires the function to declare exactly one parameter, and DuckDB does not
  check it — `copy_from` does, and returns an error naming the mismatch.
- **COPY options are named parameters.** `(FORMAT my_format, SKIP_ROWS 1)`
  arrives as `skip_rows`; matching is case-insensitive. An option the function
  never declared is a binder error before your bind callback runs — and the
  message names the *table function*, which is why the example gives it the
  format's name.
- **The schema is already fixed**, because `COPY … FROM` loads into an existing
  table. The bind callback must **not** call `add_result_column`. Read the
  target's schema instead:

```rust,no_run
# use quack_rs::table::BindInfo;
# fn demo(bind: &BindInfo) {
for i in 0..bind.result_column_count() {
    let name = bind.result_column_name(i);
    // SAFETY: `i` is in range, and this runs during the bind callback.
    let ty = unsafe { bind.result_column_type(i) };
    let _ = (name, ty);
}
# }
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
| `options()` | The `COPY … TO` options, as one `STRUCT` `Value` |
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
use quack_rs::copy_function::{CopyBindInfo, CopyGlobalInitInfo, CopySinkInfo, CopyFinalizeInfo};
```

## Related modules

- [`config_option`](https://docs.rs/quack-rs/latest/quack_rs/config_option/) — register custom settings for your format
- [`client_context`](https://docs.rs/quack-rs/latest/quack_rs/client_context/) — access the file system and catalog from callbacks
- [`table_description`](https://docs.rs/quack-rs/latest/quack_rs/table_description/) — inspect table metadata
- [`catalog`](https://docs.rs/quack-rs/latest/quack_rs/catalog/) — look up catalog entries
- [`table`](https://docs.rs/quack-rs/latest/quack_rs/table/) — build the table function a `COPY … FROM` needs
