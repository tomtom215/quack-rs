# Panic-Safe Callbacks

A Rust panic that reaches a DuckDB callback boundary aborts the whole process:
the user's DuckDB session, and any application embedding it, goes with it. The
`*_callback!` macros in `quack_rs::callback` prevent that. Each one generates the
`unsafe extern "C" fn` a builder expects, runs your code inside
`std::panic::catch_unwind`, and reports a caught panic to DuckDB as an ordinary
SQL error.

```rust,no_run
use quack_rs::data_chunk::DataChunk;
use quack_rs::vector::VectorWriter;

quack_rs::scalar_callback!(double_it, |info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    // Build the reader and writer once per chunk: each costs several C API
    // calls, which is several times the work of the loop body if done per row.
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        let value = unsafe { reader.read_i64(row) };
        unsafe { writer.write_i64(row, value * 2) };
    }
});
// `double_it` is now a `pub unsafe extern "C" fn` you pass to
// `ScalarFunctionBuilder::function`.
```

> **A raw `unsafe extern "C" fn` passed to a builder is not guarded.** The
> builders install whatever function you give them, as written. If you write the
> function by hand, a panic in it aborts the process. Write it with the matching
> macro, or wrap its body in `quack_rs::callback::catch_ffi_panic` and report the
> `Err` yourself.

## The macros

There are 16 macros. Each takes the name of the function to generate and a
closure-like body whose parameters match the C callback's arguments.

| Macro | Callback | Body parameters | On a panic |
|-------|----------|-----------------|------------|
| `scalar_callback!` | Scalar function | `info, input, output` | Query error |
| `scalar_bind_callback!` | Scalar bind (`duckdb-1-5`) | `info` | Query error |
| `scalar_init_callback!` | Scalar init (`duckdb-1-5`) | `info` | Query error |
| `table_bind_callback!` | Table bind | `info` | Query error |
| `table_init_callback!` | Table init **and** local init | `info` | Query error |
| `table_scan_callback!` | Table scan | `info, output` | Query error; the scan ends |
| `aggregate_update_callback!` | Aggregate update | `info, input, states` | Query error |
| `aggregate_combine_callback!` | Aggregate combine | `info, source, target, count` | Query error |
| `aggregate_finalize_callback!` | Aggregate finalize | `info, source, result, count, offset` | Query error |
| `aggregate_destroy_callback!` | Aggregate destructor | `states, count` | Swallowed |
| `cast_callback!` | Cast | `info, count, input, output` | Cast error; returns `false` |
| `replacement_scan_callback!` | Replacement scan | `info, table_name, data` | Query error |
| `copy_bind_callback!` | `COPY … TO` bind (`duckdb-1-5`) | `info` | Query error |
| `copy_global_init_callback!` | `COPY … TO` global init (`duckdb-1-5`) | `info` | Query error |
| `copy_sink_callback!` | `COPY … TO` sink (`duckdb-1-5`) | `info, chunk` | Query error |
| `copy_finalize_callback!` | `COPY … TO` finalize (`duckdb-1-5`) | `info` | Query error |

Each macro reports the panic through the `set_error` function DuckDB provides
for that callback kind: `duckdb_scalar_function_set_error` for
`scalar_callback!`, `duckdb_bind_set_error` for `table_bind_callback!`,
`duckdb_copy_function_sink_set_error` for `copy_sink_callback!`, and so on. The
table scan also sets the output chunk's size to 0.

The macros cover every callback you write yourself except the aggregate's
`state_size` and `state_init`. You normally let `ffi_state::<T>()` install
`FfiState`'s versions of those, which already catch a panic in your `Default`
impl (except when the aggregate runs as a window function: see
[Known Limitations](../reference/known-limitations.md#a-failing-state-initialiser-in-a-window-frame-crashes-the-host-duckdb-defect)). A `COPY … FROM` reader is a table function, so it uses the `table_*`
macros.

Each page shows its macros in use: [Scalar Functions](scalar.md),
[Aggregate Functions](aggregate.md), [Table Functions](table-functions.md),
[Replacement Scans](replacement-scan.md), [Cast Functions](cast-functions.md)
and [Copy Functions](copy-functions.md).

## What the user sees

The panic message becomes the query's error message:

```rust
# use libduckdb_sys::DuckDBSuccess;
# use quack_rs::scalar::ScalarFunctionBuilder;
# use quack_rs::types::TypeId;
quack_rs::scalar_callback!(always_fails, |info, input, output| {
    let _ = (info, input, output);
    panic!("always_fails: this input is not supported");
});
# std::mem::forget(quack_rs::testing::InMemoryDb::open().unwrap());
# let (mut db, mut con) = (std::ptr::null_mut(), std::ptr::null_mut());
# unsafe {
#     assert_eq!(libduckdb_sys::duckdb_open(std::ptr::null(), &mut db), DuckDBSuccess);
#     assert_eq!(libduckdb_sys::duckdb_connect(db, &mut con), DuckDBSuccess);
#     ScalarFunctionBuilder::new("always_fails")
#         .param(TypeId::BigInt)
#         .returns(TypeId::BigInt)
#         .function(always_fails)
#         .register(con)
#         .unwrap();
# }
# let err = unsafe { quack_rs::query::query(con, "SELECT always_fails(1)") }.err().unwrap();
# assert!(err.as_str().contains("this input is not supported"), "{err}");
```

```sql
SELECT always_fails(1);
-- Error: ... always_fails: this input is not supported
```

The process keeps running, and the next query works normally. Details:

- **An empty message** (`panic!("")`) is reported as
  `callback::EMPTY_PANIC_PLACEHOLDER`, "an extension callback panicked without
  a message". DuckDB would otherwise show a bare error prefix, and a replacement
  scan would ignore the error altogether.
- **An interior NUL** in the message is replaced by `?`.
- **The panic payload** is dropped under a second guard, because a payload's own
  `Drop` is user code too.
- **The cast** returns `false`. In a `TRY_CAST` it also sets every row of the
  chunk to `NULL`, because rows written before the panic cannot be trusted. See
  [Cast Functions](cast-functions.md) for the cast's error rules.
- **The aggregate destructor** has no way to report an error: DuckDB calls it
  with no info handle. A panic there is caught and discarded. Whatever the
  destructor had not freed yet leaks, which is better than aborting during
  query cleanup.

## Writing a macro body

The body looks like a closure, but it expands to an ordinary nested `fn`. Three
rules follow from that.

**The body is not an `unsafe` context.** Wrap each unsafe call in its own
`unsafe { … }` block, as the examples do. A raw-pointer read with no `unsafe`
block does not compile.

**The body returns `()`**, except `cast_callback!`'s, which returns the cast's
`bool`. You cannot use `?` in it: report an error through the callback's
`set_error` and return.

```rust,no_run
use quack_rs::table::BindInfo;
use quack_rs::types::TypeId;

quack_rs::table_bind_callback!(parse_limit, |info| {
    let bind = unsafe { BindInfo::new(info) };
    let text = unsafe { bind.get_parameter_value(0) }.as_str_or_default();
    match text.parse::<i64>() {
        Ok(_limit) => {
            bind.add_result_column("n", TypeId::BigInt);
        }
        Err(e) => bind.set_error(&format!("limit must be a number: {e}")),
    }
});
```

**`return` leaves the callback.** `return;` (or `return false;` in a cast) ends
the body early, as in any function.

Each macro also has an argument type that matches only its own callback kind.
For example, `scalar_bind_callback!` takes a `RawScalarBindInfo`, so its output
does not compile in a table function's `bind`, and `table_bind_callback!`'s
output does not compile in a scalar function's.

## Never build with `panic = "abort"`

`catch_unwind` cannot catch anything when the crate is built with
`panic = "abort"`: the process ends at the panic site, before any guard runs.
Keep your extension on `panic = "unwind"`. The scaffold generates it, and
`validate_release_profile` rejects `"abort"`. See
[Installation](../getting-started/installation.md#why-panic--unwind-not-abort).

## Prefer the closure-based builders where they fit

For the most common shapes you do not need a raw callback at all. The
closure-based builders generate the callbacks for you, guard them, and also
handle types, NULLs and state:

- **Scalar functions**: `ScalarFunctionBuilder::map1`, `map2`, `map1_str`,
  `map2_str` and their `_opt` variants (`quack_rs::scalar::typed`). Parameter
  and return types come from the closure's signature, and a row with a NULL
  argument gets a NULL result. A panic in the closure fails the query instead of
  aborting. See [Scalar Functions](scalar.md).
- **Table functions**: `TableFunctionBuilder::with_state` and `with_bind_init`
  (`quack_rs::table::typed`). The closures receive typed state, may return
  `Err(ExtensionError)` to fail the query, and a panic in them is reported as a
  bind, init or scan error. See
  [Table Functions](table-functions.md#closure-based-typed-state-with_state).

```rust,no_run
use quack_rs::scalar::ScalarFunctionBuilder;

# fn demo(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
// No extern "C" fn, no macro: the builder generates and guards the callback.
unsafe { ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?.register(con) }
# }
```

Use a macro and the raw builder when the closure-based builders cannot express
the function:

- scalar functions with `STRUCT`, `LIST` or `MAP` arguments, variable arity,
  bind-time constant folding or per-thread local state;
- table functions with parallel scans (`set_max_threads` above 1), projection
  pushdown, or `local_init`;
- aggregates, casts, replacement scans and copy functions, which have no
  closure-based builder.

## Code quack-rs generates for you

You do not need a macro for callbacks quack-rs writes itself. These already run
your code under `catch_unwind`:

- the entry points generated by `entry_point!` and `entry_point_v2!`, which turn
  a panic in your registration function into a load error;
- the callbacks of the closure-based builders above;
- `FfiState`'s `init_callback` and `destroy_callback` (your `Default` and `Drop`);
- the destructors quack-rs registers for `FfiBindData`, `FfiInitData` and
  replacement-scan data.

For a destructor or other `extern "C" fn` of your own that has no macro, use
`quack_rs::callback::catch_ffi_panic(|| …)`. It returns `Err(message)` when the
closure panics.

## See also

- [Pitfall L3: No panic across FFI boundaries](../reference/pitfalls.md#l3-no-panic-across-ffi-boundaries)
- [`callback`](https://docs.rs/quack-rs/latest/quack_rs/callback/index.html) module documentation
