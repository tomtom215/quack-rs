# Scalar Functions

Scalar functions transform a batch of input rows into a corresponding batch of output values.
They are the most common DuckDB extension pattern — equivalent to SQL's built-in functions
like `length()`, `upper()`, or `sin()`.

---

## Function signature

DuckDB calls your scalar function once per data chunk (not once per row). The signature is:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
unsafe extern "C" fn my_fn(
    info: duckdb_function_info,     // function metadata (rarely needed)
    input: duckdb_data_chunk,       // input data — one or more columns
    output: duckdb_vector,          // output vector — one value per input row
)
# {}
```

Inside the function, you:
1. Create a `VectorReader` for each input column
2. Create a `VectorWriter` for the output
3. Loop over rows, checking for NULLs and transforming values

---

## Registration

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn my_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::TypeId;

unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        ScalarFunctionBuilder::new("my_fn")
            .param(TypeId::BigInt)      // first parameter type
            .param(TypeId::BigInt)      // second parameter type (if any)
            .returns(TypeId::BigInt)    // return type
            .function(my_fn)            // callback
            .register(con)?;
    }
    Ok(())
}
```

The builder validates that `returns` and `function` are set before calling
`duckdb_register_scalar_function`. If DuckDB reports failure, `register` returns `Err`.

### Validated registration

For user-configurable function names (e.g., from a config file), use `try_new`:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn my_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection, name: &str) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::try_new(name)?   // validates name before building
    .param(TypeId::Varchar)
    .returns(TypeId::Varchar)
    .function(my_fn)
    .register(con)?;
# Ok(())
# }
```

`try_new` validates the name as an unquoted SQL identifier: `[A-Za-z_][A-Za-z0-9_]*`,
at most 256 characters. Mixed case is allowed (DuckDB itself ships
`formatReadableSize`). `new` does **not** validate: it only panics if the name
contains an interior NUL byte, so use it for compile-time-known names only.

### Closures

`ScalarFunctionBuilder::map1` / `map2` / `map1_str` / `map2_str` / `map1_opt` /
`map2_opt` build the whole function from a Rust closure and return a
`TypedScalarFunctionBuilder`. Its signature is fixed by the closure's types, so it
deliberately offers only `name()`, `volatile()` and `register(con)` — no `returns`,
`param`, `function` or `extra_info`, any of which would make DuckDB hand the
closure vectors of a different width than it reads and writes. Register it through
a `Registrar` with `register_typed_scalar`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?.register(con)?;
# Ok(())
# }
```

---

## Complete example: `double_it(BIGINT) → BIGINT`

```rust
# use quack_rs::prelude::*;
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
use quack_rs::vector::{VectorReader, VectorWriter};
use libduckdb_sys::{duckdb_function_info, duckdb_data_chunk, duckdb_vector};

unsafe extern "C" fn double_it(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    // SAFETY: DuckDB provides valid chunk and vector pointers.
    let reader = unsafe { VectorReader::new(input, 0) };   // column 0
    let mut writer = unsafe { VectorWriter::new(output) };
    let row_count = reader.row_count();

    for row in 0..row_count {
        if unsafe { !reader.is_valid(row) } {
            // NULL input → NULL output
            // SAFETY: row < row_count, writer is valid.
            unsafe { writer.set_null(row) };
            continue;
        }
        let value = unsafe { reader.read_i64(row) };
        unsafe { writer.write_i64(row, value * 2) };
    }
}
# let con = live_connection();
# unsafe {
#     ScalarFunctionBuilder::new("double_it").param(TypeId::BigInt).returns(TypeId::BigInt)
#         .function(double_it).register(con).unwrap();
# }
# assert_eq!(query_i64(con, "SELECT double_it(21)"), Some(42));
# assert_eq!(query_i64(con, "SELECT double_it(NULL::BIGINT)"), None);
```

---

## Multi-parameter example: `add(BIGINT, BIGINT) → BIGINT`

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
unsafe extern "C" fn add(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let col0 = unsafe { VectorReader::new(input, 0) };  // first param
    let col1 = unsafe { VectorReader::new(input, 1) };  // second param
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..col0.row_count() {
        if unsafe { !col0.is_valid(row) || !col1.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let a = unsafe { col0.read_i64(row) };
        let b = unsafe { col1.read_i64(row) };
        unsafe { writer.write_i64(row, a + b) };
    }
}
```

---

## VARCHAR example: `shout(VARCHAR) → VARCHAR`

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
unsafe extern "C" fn shout(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let reader = unsafe { VectorReader::new(input, 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        if unsafe { !reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        let upper = s.to_uppercase();
        unsafe { writer.write_varchar(row, &upper) };
    }
}
```

---

## Overloading with Function Sets

If your function accepts different parameter types or arities, use `ScalarFunctionSetBuilder`
to register multiple overloads under a single name:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn add_ints(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe extern "C" fn add_doubles(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
use quack_rs::scalar::{ScalarFunctionSetBuilder, ScalarOverloadBuilder};
use quack_rs::types::TypeId;

unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        ScalarFunctionSetBuilder::new("my_add")
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Integer).param(TypeId::Integer)
                    .returns(TypeId::Integer)
                    .function(add_ints)
            )
            .overload(
                ScalarOverloadBuilder::new()
                    .param(TypeId::Double).param(TypeId::Double)
                    .returns(TypeId::Double)
                    .function(add_doubles)
            )
            .register(con)?;
    }
    Ok(())
}
```

Like `AggregateFunctionSetBuilder`, this builder calls `duckdb_scalar_function_set_name`
on every individual function before adding it to the set
([Pitfall L6](../reference/pitfalls.md#l6-function-set-name-must-be-set-on-each-member)).

`ScalarOverloadBuilder` has the same per-function settings as
`ScalarFunctionBuilder`, applied to that overload only: `null_handling`,
`extra_info`, `varargs` / `varargs_logical`, `volatile`, and (DuckDB 1.5+)
`bind` / `init`. `register` checks every overload for a return type and a
callback before it creates any DuckDB handle, and the error names the overload's
index. A varargs type counts as part of an overload's signature, so `f(BIGINT)`
and `f(BIGINT, BIGINT...)` may share a set.

---

## NULL Handling

Your callback receives NULL rows whatever the setting: under the default,
`DefaultNullHandling`, it *promises* NULL-in-NULL-out and must write the NULLs
itself — call `chunk.propagate_nulls(&mut writer)` at the end, or use the typed
`map1` / `map2` constructors, which do it for you
([Pitfall L8](../reference/pitfalls.md#l8-default_null_handling-does-not-propagate-nulls-for-scalar-functions),
[NULL handling](null-handling.md)). A function that means to return non-NULL for
NULL input (e.g., a `COALESCE`-like function) sets `SpecialNullHandling`:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn my_coalesce_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
use quack_rs::types::NullHandling;

ScalarFunctionBuilder::new("coalesce_custom")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .null_handling(NullHandling::SpecialNullHandling)
    .function(my_coalesce_fn)
    .register(con)?;
# Ok(())
# }
```

With `SpecialNullHandling`, your callback must check `VectorReader::is_valid(row)`
and handle NULLs yourself.

---

## Complex parameter and return types

For scalar functions that accept or return parameterized types like `LIST(BIGINT)`,
use `param_logical` and `returns_logical`:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn flatten_list_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};

ScalarFunctionBuilder::new("flatten_list")
    .param_logical(LogicalType::list(TypeId::BigInt))  // LIST(BIGINT) input
    .returns(TypeId::BigInt)
    .function(flatten_list_fn)
    .register(con)?;
# Ok(())
# }
```

These methods are also available on `ScalarOverloadBuilder` for function sets:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn my_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# fn demo() {
# let _ =
ScalarOverloadBuilder::new()
    .param(TypeId::Varchar)
    .returns_logical(LogicalType::list(TypeId::Timestamp))  // LIST(TIMESTAMP) output
    .function(my_fn)
# ;
# }
```

---

## Key points

- **`VectorReader::new(input, column_index)`** — the column index is zero-based
- **Always check `is_valid(row)` before reading** — skipping this reads garbage for NULL rows
- **`set_null` must be called for NULL outputs** — it calls `ensure_validity_writable`
  automatically ([Pitfall L4](../reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output))
- **`read_bool` returns `bool`** — handles DuckDB's non-0/1 boolean bytes correctly
  ([Pitfall L5](../reference/pitfalls.md#l5-boolean-reading-must-use-u8--0))
- **`read_str` handles both inline and pointer string formats** automatically
  ([Pitfall P7](../reference/pitfalls.md#p7-duckdb_string_t-format-is-undocumented))

---

## Varargs and volatility

These `ScalarFunctionBuilder` methods map to functions in DuckDB's **stable** C
API (v1.2.0), so they need no feature flag and work on DuckDB 1.4.x and 1.5.x
alike:

### `varargs(type_id: TypeId)`

Declares that the function accepts a variable number of trailing arguments, all
of the given `TypeId`. Maps to `duckdb_scalar_function_set_varargs`. A composite
`TypeId` such as `List` or `Decimal` makes `register` return an error naming the
varargs slot; use `varargs_logical` for those.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn concat_all_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("concat_all")
    .varargs(TypeId::Varchar)
    .returns(TypeId::Varchar)
    .function(concat_all_fn)
    .register(con)?;
# Ok(())
# }
```

### `varargs_logical(logical_type: LogicalType)`

Like `varargs`, but accepts a `LogicalType` for parameterized variadic arguments.
Maps to `duckdb_scalar_function_set_varargs`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn merge_lists_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("merge_lists")
    .varargs_logical(LogicalType::list(TypeId::BigInt))
    .returns_logical(LogicalType::list(TypeId::BigInt))
    .function(merge_lists_fn)
    .register(con)?;
# Ok(())
# }
```

### `volatile()`

Marks the function as volatile, meaning DuckDB will not cache or reuse its
results across calls with the same arguments. Maps to
`duckdb_scalar_function_set_volatile`. Also available on the closure-built
`TypedScalarFunctionBuilder`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn random_int_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("random_int")
    .returns(TypeId::Integer)
    .volatile()
    .function(random_int_fn)
    .register(con)?;
# Ok(())
# }
```

---

## DuckDB 1.5.0 Additions (`duckdb-1-5`)

The following `ScalarFunctionBuilder` methods are available when the `duckdb-1-5`
feature is enabled:

### `bind(bind_fn)`

Sets a custom bind callback that runs at plan time. Use this to inspect argument
types and set the return type dynamically. Maps to
`duckdb_scalar_function_set_bind`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn dynamic_return_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe extern "C" fn my_bind_fn(_: duckdb_bind_info) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("dynamic_return")
    .varargs(TypeId::Varchar)
    .returns(TypeId::Varchar)   // default; overridden in bind
    .bind(my_bind_fn)
    .function(dynamic_return_fn)
    .register(con)?;
# Ok(())
# }
```

### `init(init_fn)`

Sets a local-init callback invoked once per thread before execution begins. Use
this to allocate per-thread state. Maps to
`duckdb_scalar_function_set_init`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn stateful_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe extern "C" fn my_init_fn(_: duckdb_init_info) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("stateful_fn")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .init(my_init_fn)
    .function(stateful_fn)
    .register(con)?;
# Ok(())
# }
```

### Typed bind data and local state

`ScalarBindData<T>` and `ScalarLocalState<T>` store a Rust value from the bind
and init callbacks with a generated, panic-safe destructor:

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# use quack_rs::scalar::{ScalarBindData, ScalarBindInfo, ScalarFunctionInfo};
# unsafe fn demo(bind_info: ScalarBindInfo, fn_info: ScalarFunctionInfo) {
#[derive(Clone)]
struct Factor(i64);

// in the bind callback
ScalarBindData::set(&bind_info, Factor(10));
// in the function callback
let factor = unsafe { ScalarBindData::<Factor>::get(&fn_info) };
# }
```

- **Bind data must be `Clone + Send + Sync`.** Every executing thread reads the
  same value concurrently, and DuckDB *copies* the bound expression whenever the
  optimizer duplicates it (filter pushdown through a projection does). Without a
  copy callback the copy has **no** bind data at all, so `set` registers one that
  clones `T`. Wrap data that is expensive or impossible to clone in an `Arc<T>`.
- **Local state must be `Send`**: it is per thread, but may be freed on another.
- **Call `set` at most once per callback.** DuckDB overwrites the stored pointer
  on a second call without freeing the first value, so that value is leaked
  (never dropped).
- **Bind data must depend only on the call's arguments** (their values when
  constant, their types) and `extra_info`. DuckDB's
  `CScalarFunctionBindData::Equals` ignores bind data, so two calls with the same
  arguments — `SELECT f(i), f(i)` — are merged and share the first call's bind
  data. A bind callback that reads a counter, a clock or a random source needs
  the function marked `volatile()`, which stops the merge.

---

## Extra info

Attach arbitrary data to a scalar function using `extra_info`. This is useful for
parameterising the function behaviour (e.g., a locale or configuration struct).
The method is available on both `ScalarFunctionBuilder` and `ScalarOverloadBuilder`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn locale_upper_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe extern "C" fn my_destroy(p: *mut std::os::raw::c_void) {
#     drop(unsafe { Box::from_raw(p.cast::<String>()) });
# }
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
use std::os::raw::c_void;

let config = Box::into_raw(Box::new("en_US".to_string())).cast::<c_void>();
unsafe {
    ScalarFunctionBuilder::new("locale_upper")
        .param(TypeId::Varchar)
        .returns(TypeId::Varchar)
        .extra_info(config, Some(my_destroy))
        .function(locale_upper_fn)
        .register(con)?;
}
# Ok(())
# }
```

Inside the callback, retrieve the extra info with `ScalarFunctionInfo::get_extra_info()`.

---

## `ScalarFunctionInfo`

`ScalarFunctionInfo` wraps the `duckdb_function_info` handle provided to a scalar
function callback. It exposes:

- `get_extra_info() -> *mut c_void` — retrieves the extra-info pointer set during
  registration
- `set_error(message)` — reports an error, causing DuckDB to abort the query

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
use quack_rs::scalar::ScalarFunctionInfo;

unsafe extern "C" fn my_fn(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let info = unsafe { ScalarFunctionInfo::new(info) };
    let extra = unsafe { info.get_extra_info() };
    // ... use extra info, or report errors via info.set_error("...") ...
}
```

With the `duckdb-1-5` feature, `ScalarFunctionInfo` also provides:

- `get_bind_data() -> *mut c_void` — retrieves bind data set during the bind callback
- `get_state() -> *mut c_void` — retrieves per-thread state set during the init callback

### `ScalarBindInfo` (`duckdb-1-5`)

`ScalarBindInfo` wraps the `duckdb_bind_info` handle provided to a scalar function
bind callback. It exposes:

- `argument_count() -> u64` — number of arguments
- `get_argument(index) -> duckdb_expression` — argument expression at `index`
- `get_extra_info() -> *mut c_void` — the extra-info pointer from registration
- `set_bind_data(data, destroy)` — stores per-query data retrievable during execution
- `set_bind_data_copy(copy)` — the callback DuckDB uses to duplicate that data when it
  copies the bound expression; without one the copy's bind data is NULL
- `set_error(message)` — reports an error
- `get_client_context() -> ClientContext` — access to the connection's catalog and config

### `ScalarInitInfo` (`duckdb-1-5`)

`ScalarInitInfo` wraps the `duckdb_init_info` handle provided to a scalar function
init callback. It exposes:

- `get_extra_info() -> *mut c_void` — the extra-info pointer from registration
- `get_bind_data() -> *mut c_void` — the bind data from the bind callback
- `set_state(state, destroy)` — stores per-thread state retrievable during execution
- `set_error(message)` — reports an error
- `get_client_context() -> ClientContext` — access to the connection's catalog and config
