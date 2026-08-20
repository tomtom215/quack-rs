# Scalar Functions

Scalar functions transform a batch of input rows into a corresponding batch of output values.
They are the most common DuckDB extension pattern — equivalent to SQL's built-in functions
like `length()`, `upper()`, or `sin()`.

---

## Function signature

DuckDB calls your scalar function once per data chunk (not once per row). The signature is:

```rust,ignore
unsafe extern "C" fn my_fn(
    info: duckdb_function_info,     // function metadata (rarely needed)
    input: duckdb_data_chunk,       // input data — one or more columns
    output: duckdb_vector,          // output vector — one value per input row
)
```

Inside the function, you:
1. Create a `VectorReader` for each input column
2. Create a `VectorWriter` for the output
3. Loop over rows, checking for NULLs and transforming values

---

## Registration

```rust,ignore
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

```rust,ignore
ScalarFunctionBuilder::try_new(name)?   // validates name before building
    .param(TypeId::Varchar)
    .returns(TypeId::Varchar)
    .function(my_fn)
    .register(con)?;
```

`try_new` validates the name against DuckDB naming rules:
`[a-z_][a-z0-9_]*`, max 256 characters. `new` panics on invalid names (suitable for
compile-time-known names only).

---

## Complete example: `double_it(BIGINT) → BIGINT`

```rust
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
```

---

## Multi-parameter example: `add(BIGINT, BIGINT) → BIGINT`

```rust
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
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
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
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

```rust,ignore
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

---

## NULL Handling

By default, DuckDB returns NULL if any argument is NULL — your function callback is
never called for those rows. If you need to handle NULLs explicitly (e.g., for a
`COALESCE`-like function), set `SpecialNullHandling`:

```rust,ignore
use quack_rs::types::NullHandling;

ScalarFunctionBuilder::new("coalesce_custom")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .null_handling(NullHandling::SpecialNullHandling)
    .function(my_coalesce_fn)
    .register(con)?;
```

With `SpecialNullHandling`, your callback must check `VectorReader::is_valid(row)`
and handle NULLs yourself.

---

## Complex parameter and return types

For scalar functions that accept or return parameterized types like `LIST(BIGINT)`,
use `param_logical` and `returns_logical`:

```rust,ignore
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{LogicalType, TypeId};

ScalarFunctionBuilder::new("flatten_list")
    .param_logical(LogicalType::list(TypeId::BigInt))  // LIST(BIGINT) input
    .returns(TypeId::BigInt)
    .function(flatten_list_fn)
    .register(con)?;
```

These methods are also available on `ScalarOverloadBuilder` for function sets:

```rust,ignore
ScalarOverloadBuilder::new()
    .param(TypeId::Varchar)
    .returns_logical(LogicalType::list(TypeId::Timestamp))  // LIST(TIMESTAMP) output
    .function(my_fn)
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

## DuckDB 1.5.0 Additions (`duckdb-1-5`)

The following `ScalarFunctionBuilder` methods are available when the `duckdb-1-5`
feature is enabled:

### `varargs(type_id: TypeId)`

Declares that the function accepts a variable number of trailing arguments, all
of the given `TypeId`. Maps to `duckdb_scalar_function_set_varargs`.

```rust,ignore
ScalarFunctionBuilder::new("concat_all")
    .varargs(TypeId::Varchar)
    .returns(TypeId::Varchar)
    .function(concat_all_fn)
    .register(con)?;
```

### `varargs_logical(logical_type: LogicalType)`

Like `varargs`, but accepts a `LogicalType` for parameterized variadic arguments.
Maps to `duckdb_scalar_function_set_varargs`.

```rust,ignore
ScalarFunctionBuilder::new("merge_lists")
    .varargs_logical(LogicalType::list(TypeId::BigInt))
    .returns_logical(LogicalType::list(TypeId::BigInt))
    .function(merge_lists_fn)
    .register(con)?;
```

### `volatile()`

Marks the function as volatile, meaning DuckDB will not cache or reuse its
results across calls with the same arguments. Maps to
`duckdb_scalar_function_set_volatile`.

```rust,ignore
ScalarFunctionBuilder::new("random_int")
    .returns(TypeId::Integer)
    .volatile()
    .function(random_int_fn)
    .register(con)?;
```

### `bind(bind_fn)`

Sets a custom bind callback that runs at plan time. Use this to inspect argument
types and set the return type dynamically. Maps to
`duckdb_scalar_function_set_bind`.

```rust,ignore
ScalarFunctionBuilder::new("dynamic_return")
    .varargs(TypeId::Varchar)
    .returns(TypeId::Varchar)   // default; overridden in bind
    .bind(my_bind_fn)
    .function(dynamic_return_fn)
    .register(con)?;
```

### `init(init_fn)`

Sets a local-init callback invoked once per thread before execution begins. Use
this to allocate per-thread state. Maps to
`duckdb_scalar_function_set_init`.

```rust,ignore
ScalarFunctionBuilder::new("stateful_fn")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .init(my_init_fn)
    .function(stateful_fn)
    .register(con)?;
```

---

## Extra info

Attach arbitrary data to a scalar function using `extra_info`. This is useful for
parameterising the function behaviour (e.g., a locale or configuration struct).
The method is available on both `ScalarFunctionBuilder` and `ScalarOverloadBuilder`.

```rust,ignore
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
# use quack_rs::prelude::*;
# use quack_rs::error::ExtensionError;
# use libduckdb_sys::*;
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
