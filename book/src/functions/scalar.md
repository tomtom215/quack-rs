# Scalar Functions

A DuckDB scalar function returns one output value per input row, like the built-in
`length()`, `upper()` or `sin()`. This page shows how to write one in Rust with quack-rs:
as a raw callback registered through `ScalarFunctionBuilder`, as a safe closure with
`map1` / `map2`, or as a set of overloads with `ScalarFunctionSetBuilder`.

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

A panic that unwinds out of an `extern "C"` function aborts the process. Generate the
function with `scalar_callback!`, which reports a panic as a SQL error, or use the
closure constructors below ([Error Handling](../concepts/errors.md#no-panics)).

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

Before calling `duckdb_register_scalar_function`, `register` checks that `returns` and
`function` are set and that no parameter or return type is a bare composite `TypeId`
(`List`, `Decimal`, …; use the `*_logical` methods for those). It also refuses a
signature that `duckdb_functions()` already lists under the same name, built-ins
included: DuckDB would otherwise silently replace that overload for every connection,
or make every call ambiguous. If DuckDB reports failure, `register` returns `Err`.

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
at most 256 characters, and not a DuckDB keyword that cannot be called as a function
unquoted (`order`, `coalesce`). Mixed case is allowed (DuckDB itself ships
`formatReadableSize`). `new` does **not** validate: it only panics if the name
contains an interior NUL byte, so use it for names known at compile time.

### Closures

`ScalarFunctionBuilder::map1` / `map2` / `map1_str` / `map2_str` / `map1_opt` /
`map2_opt` build the whole function from a Rust closure and return a
`TypedScalarFunctionBuilder`. Its signature is fixed by the closure's types, so it
deliberately offers only `name()`, `volatile()` and `register(con)` — no `returns`,
`param`, `function`, `extra_info`, `bind` or `init`, any of which could make DuckDB
hand the closure vectors of a different type than it reads and writes. Register it
with `register(con)`, or through a `Registrar` with `register_typed_scalar`.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::map1("is_even", |x: i64| x % 2 == 0)?.register(con)?;
# Ok(())
# }
```

Arithmetic inside the closure is yours to check: a plain `x * 2` wraps on overflow in a
release build and panics (so fails the query) in a debug build. Use `try_map1` and
`checked_mul` to turn overflow into an error in both, as the
[Quick Start](../getting-started/quick-start.md) does.

A closure can also return `Option<R>`, where `None` is SQL `NULL`, and the
`try_map1` / `try_map2` / `try_map3` / `try_map1_str` / `try_map2_str`
constructors take a closure returning `Result<R, E>`. An `Err` fails the query
with the error's `Display` text, through the same channel a caught panic uses.
`map3` and `try_map3` take three arguments. NULL arguments still never reach the
closure; the `*_opt` constructors are the way to see them.

```rust
# use libduckdb_sys::duckdb_connection;
# use quack_rs::prelude::*;
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
// `None` is SQL NULL.
ScalarFunctionBuilder::map1_str("email_domain", |s: &str| {
    s.split_once('@').map(|(_, domain)| domain.to_owned())
})?
.register(con)?;

// `Err` fails the query with its message; `Ok(None)` is NULL.
ScalarFunctionBuilder::try_map1_str("parse_int", |s: &str| {
    if s.is_empty() {
        return Ok(None);
    }
    s.trim()
        .parse::<i64>()
        .map(Some)
        .map_err(|e| format!("parse_int({s:?}): {e}"))
})?
.register(con)?;

ScalarFunctionBuilder::map3("lerp", |a: f64, b: f64, t: f64| a + (b - a) * t)?.register(con)?;
# Ok(())
# }
```

---

## Complete example: `double_it(BIGINT) → BIGINT`

For a function this small the closure builder above is the better choice
(`try_map1` with `checked_mul`, as in the [Quick Start](../getting-started/quick-start.md)).
Written as a raw callback, with `scalar_callback!` for the panic guard, it looks like this.
`value * 2` would wrap silently in a release build, so the callback reports overflow
through `ScalarFunctionInfo::set_error`:

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
use quack_rs::scalar::ScalarFunctionInfo;
use quack_rs::vector::{VectorReader, VectorWriter};

quack_rs::scalar_callback!(double_it, |info, input, output| {
    // SAFETY: DuckDB provides valid chunk and vector pointers.
    let reader = unsafe { VectorReader::new(input, 0) };   // column 0
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        if unsafe { !reader.is_valid(row) } {
            // NULL input → NULL output
            // SAFETY: row < row_count, writer is valid.
            unsafe { writer.set_null(row) };
            continue;
        }
        let value = unsafe { reader.read_i64(row) };
        let Some(doubled) = value.checked_mul(2) else {
            unsafe { ScalarFunctionInfo::new(info) }.set_error("double_it: BIGINT overflow");
            return;
        };
        unsafe { writer.write_i64(row, doubled) };
    }
});
# let con = live_connection();
# unsafe {
#     ScalarFunctionBuilder::new("double_it").param(TypeId::BigInt).returns(TypeId::BigInt)
#         .function(double_it).register(con).unwrap();
# }
# assert_eq!(query_i64(con, "SELECT double_it(21)"), Some(42));
# // From a column, not a literal: `double_it(NULL::BIGINT)` is constant-folded.
# assert_eq!(query_i64(con, "SELECT double_it(i) FROM (VALUES (NULL::BIGINT)) t(i)"), None);
# let err = unsafe { quack_rs::query::query(con, "SELECT double_it(9223372036854775807)") }.err().unwrap();
# assert!(err.as_str().contains("double_it: BIGINT overflow"), "{err}");
```

---

## Multi-parameter example: `add(BIGINT, BIGINT) → BIGINT`

```rust
# use quack_rs::prelude::*;
use quack_rs::scalar::ScalarFunctionInfo;

quack_rs::scalar_callback!(add, |info, input, output| {
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
        let Some(sum) = a.checked_add(b) else {
            unsafe { ScalarFunctionInfo::new(info) }.set_error("add: BIGINT overflow");
            return;
        };
        unsafe { writer.write_i64(row, sum) };
    }
});
```

---

## VARCHAR example: `shout(VARCHAR) → VARCHAR`

```rust
# use quack_rs::prelude::*;
quack_rs::scalar_callback!(shout, |_info, input, output| {
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
});
```

---

## Overloading with function sets

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
index. It also refuses two overloads that accept the same call: the same argument
types, or, with varargs, the same types at some argument count. `f(BIGINT)` and
`f(BIGINT, BIGINT...)` both accept `f(1)`, so they cannot share a set; DuckDB would
accept the set and then fail every such call as ambiguous. An overload that matches
an existing function's signature is refused as for `ScalarFunctionBuilder`.

---

## NULL handling

Your callback receives NULL rows whatever the setting: under the default,
`DefaultNullHandling`, it *promises* NULL-in-NULL-out and must write the NULLs
itself — call `chunk.propagate_nulls(&mut writer)` at the end, or use the closure
constructors (`map1`, `map2` and their `_str` forms), which do it for you
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

With `SpecialNullHandling`, the callback must check `VectorReader::is_valid(row)`
itself and decide what each NULL input produces. The closure constructors
`map1_opt` / `map2_opt` register `SpecialNullHandling` for you and pass the
closure an `Option` (`None` for NULL); returning `None` writes NULL.

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

### `DECIMAL` parameters are not generic

A `DECIMAL(w, s)` parameter takes exactly that width and scale. DuckDB casts every argument
to the declared type before your callback runs, and the cast **rounds**: a function
declared over `DECIMAL(4,1)` that doubles its argument, called with
`0.123::DECIMAL(3,3)`, sees `0.1` and returns `0.2`, not `0.246` (measured on 1.5.6 and
2.0.0). Declaring several widths as overloads does not work either: DuckDB's overload
resolution does not tell `DECIMAL(4,1)` from `DECIMAL(9,3)` apart, so every call to such a
set fails with "Could not choose a best candidate function" (measured on 1.5.6 and 2.0.0;
the same holds for `MAP` and `ENUM` parameters of different parameterisations, and for those
nested inside a `LIST`, `ARRAY` or `STRUCT`). quack-rs 0.20 refuses such a set when you
register it; 0.19 and earlier registered it, and the first call failed. To accept any `DECIMAL`, declare the
parameter `TypeId::Any`, read the argument vector's actual type at run time
(`quack_rs::vector::vector_get_column_type`, then `LogicalType::decimal_width`,
`decimal_scale` and `decimal_internal_type`) and return a type that does not depend on it
(`DOUBLE`, `VARCHAR` or `HUGEINT`); or register one function per width under different
names.

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
- **Always check `is_valid(row)` before reading** — skipping this reads garbage for NULL rows, and for `VARCHAR`/`BLOB` can follow a stale pointer
- **`set_null` must be called for NULL outputs** — it calls `ensure_validity_writable`
  automatically ([Pitfall L4](../reference/pitfalls.md#l4-ensure_validity_writable-is-required-before-null-output))
- **`read_bool` returns `bool`** — handles DuckDB's non-0/1 boolean bytes correctly
  ([Pitfall L5](../reference/pitfalls.md#l5-boolean-reading-must-use-u8--0-not-const-bool))
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

Marks the function as volatile: DuckDB re-evaluates it for every row, even when
its arguments are constant (as for `random()`), and never merges two identical
calls in one query. Without it, DuckDB may evaluate a call with constant
arguments only once. Maps to
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

## DuckDB 1.5.0 additions (`duckdb-1-5`)

The following `ScalarFunctionBuilder` methods are available when the `duckdb-1-5`
feature is enabled:

### `bind(bind_fn)`

Sets a custom bind callback that runs at plan time. Use it to inspect the argument
types and constant arguments, to store bind data for the execute callback, and to reject
a call with `set_error`. Maps to `duckdb_scalar_function_set_bind`.

**The bind callback cannot set the return type.** The C API v1 has no such function: it
offers `duckdb_scalar_function_bind_get_argument`, `_get_argument_count`,
`_get_extra_info` and `_set_error`, and `ScalarBindInfo` wraps exactly those, plus
`set_bind_data`, `set_bind_data_copy` and `get_client_context`. The return type is the one
you declared with `returns`. When the result type must depend on the argument types,
register one overload per result type with `ScalarFunctionSetBuilder` (for example
`f(INTEGER) → INTEGER[]` and `f(VARCHAR) → STRUCT(v VARCHAR)`). Choosing the return type
at bind time exists only in the C API v2
([Known Limitations](../reference/known-limitations.md#duckdb-20s-c-api-v2-offers-what-v1-cannot)).

The callback takes a `RawScalarBindInfo` (wrap it with `ScalarBindInfo::new`),
not a bare `duckdb_bind_info`: DuckDB passes a table function's bind callback
a different, larger struct, and a callback written for one kind of function
corrupts memory on the other, so the types keep them apart. `init` takes a
`RawScalarInitInfo` for the same reason. Generate panic-safe callbacks with
`scalar_bind_callback!` and `scalar_init_callback!`; the table-function macros
do not type-check here.

```rust
# use libduckdb_sys::{duckdb_aggregate_state, duckdb_bind_info, duckdb_connection,
#     duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector, idx_t};
# use quack_rs::prelude::*;
# unsafe extern "C" fn dynamic_return_fn(_: duckdb_function_info, _: duckdb_data_chunk, _: duckdb_vector) {}
# unsafe extern "C" fn my_bind_fn(_: quack_rs::scalar::RawScalarBindInfo) {}
# unsafe fn demo(con: duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("dynamic_return")
    .varargs(TypeId::Varchar)
    .returns(TypeId::Varchar)   // fixed: bind cannot change it
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
# unsafe extern "C" fn my_init_fn(_: quack_rs::scalar::RawScalarInitInfo) {}
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
and init callbacks with a generated, panic-safe destructor, so no hand-written
`Box::from_raw` is needed. A function that multiplies its argument by a factor
fixed at bind time, and counts the rows each thread processes:

```rust
# use quack_rs::prelude::*;
use quack_rs::scalar::{
    ScalarBindData, ScalarBindInfo, ScalarFunctionInfo, ScalarInitInfo, ScalarLocalState,
};

#[derive(Clone)]
struct Factor(i64);

quack_rs::scalar_bind_callback!(scaled_bind, |info| {
    // SAFETY: `info` is the argument of the running bind callback.
    let bind = unsafe { ScalarBindInfo::new(info) };
    ScalarBindData::set(&bind, Factor(10));
});

quack_rs::scalar_init_callback!(scaled_init, |info| {
    // SAFETY: `info` is the argument of the running init callback.
    let init = unsafe { ScalarInitInfo::new(info) };
    ScalarLocalState::set(&init, 0_u64); // rows this thread has processed
});

quack_rs::scalar_callback!(scaled, |info, input, output| {
    // SAFETY: DuckDB passes valid handles to the running callback.
    let fn_info = unsafe { ScalarFunctionInfo::new(info) };
    let chunk = unsafe { DataChunk::from_raw(input) };
    // SAFETY: `scaled_bind` stored a `Factor`, and nothing else did.
    let factor = unsafe { ScalarBindData::<Factor>::get(&fn_info) }.map_or(1, |f| f.0);
    // SAFETY: `scaled_init` stored a `u64` on this thread; no other borrow is live.
    if let Some(rows) = unsafe { ScalarLocalState::<u64>::get_mut(&fn_info) } {
        *rows += chunk.size() as u64;
    }
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        // A NULL row holds an arbitrary value; `wrapping_mul` keeps it from
        // overflowing, and `propagate_nulls` below overwrites it with NULL.
        unsafe { writer.write_i64(row, reader.read_i64(row).wrapping_mul(factor)) };
    }
    unsafe { chunk.propagate_nulls(&mut writer) };
});

# unsafe fn demo(con: libduckdb_sys::duckdb_connection) -> Result<(), ExtensionError> {
ScalarFunctionBuilder::new("scaled")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .bind(scaled_bind)
    .init(scaled_init)
    .function(scaled)
    .register(con)?;
# Ok(())
# }
```

- **Bind data must be `Clone + Send + Sync`.** Every executing thread reads the
  same value concurrently, and DuckDB *copies* the bound expression whenever the
  optimizer duplicates it (filter pushdown through a projection does). Without a
  copy callback the copy has **no** bind data at all, so `set` registers one that
  clones `T`. Wrap data that is expensive or impossible to clone in an `Arc<T>`.
- **A bind-time constant cannot come from a prepared-statement parameter.** If `bind`
  rejects an argument that is not foldable (see [Bound Expressions](../duckdb-1-5/expression.md)),
  a `?` is not foldable: `PREPARE p AS SELECT f(x, ?)` fails at the `PREPARE`, with your
  message, on DuckDB 1.5.6 and 2.0.0, while a literal or a constant expression such as `1 + 1`
  folds. Say so in the error text, and tell users to pass such an argument as a literal.
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

Attach arbitrary data to a scalar function using `extra_info`, for example a locale
or a configuration struct that parameterises its behaviour. The method is available
on both `ScalarFunctionBuilder` and `ScalarOverloadBuilder`. The pointee must be
`Send + Sync`: DuckDB passes the same pointer to the callback on every thread that
runs the function, and the destructor runs on whichever thread releases it.

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
- `set_error(message)` — reports an error, which fails the query

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
- `argument(index) -> Option<Expression>` — the argument at `index` as an RAII
  `Expression`, before DuckDB casts it to the parameter type. On DuckDB before
  1.5.5 it fails the bind and returns `None`, because those releases abort the
  process when the argument is a scalar subquery
- `get_argument(index) -> duckdb_expression` — the raw handle, with that hazard left
  to the caller
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
