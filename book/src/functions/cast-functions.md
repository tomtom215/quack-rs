# Cast Functions

Cast functions let your extension define how DuckDB converts values from one type to
another. Once registered, both explicit `CAST(x AS T)` syntax and (optionally) implicit
coercions will use your callback.

## When to use cast functions

- Your extension introduces a new logical type and needs `CAST` to/from standard types.
- You want to override DuckDB's built-in cast behaviour for a specific type pair.
- You need to control implicit cast priority relative to other registered casts.

## Registering a cast

```rust,no_run
use quack_rs::cast::{CastFunctionBuilder, CastFunctionInfo, CastMode};
use quack_rs::types::TypeId;
use quack_rs::vector::{VectorReader, VectorWriter};
use libduckdb_sys::{duckdb_function_info, duckdb_vector, idx_t};

unsafe extern "C" fn varchar_to_int(
    info: duckdb_function_info,
    count: idx_t,
    input: duckdb_vector,
    output: duckdb_vector,
) -> bool {
    let cast_info = unsafe { CastFunctionInfo::new(info) };
    let reader = unsafe { VectorReader::from_vector(input, count as usize) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..count as usize {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        match s.parse::<i32>() {
            Ok(v) => unsafe { writer.write_i32(row, v) },
            Err(e) => {
                let msg = format!("cannot cast {:?} to INTEGER: {e}", s);
                if cast_info.cast_mode() == CastMode::Try {
                    // TRY_CAST: write NULL and record a per-row error
                    unsafe { cast_info.set_row_error(&msg, row as idx_t, output) };
                    unsafe { writer.set_null(row) };
                } else {
                    // Regular CAST: abort the whole query
                    unsafe { cast_info.set_error(&msg) };
                    return false;
                }
            }
        }
    }
    true
}

fn register(con: libduckdb_sys::duckdb_connection)
    -> Result<(), quack_rs::error::ExtensionError>
{
    unsafe {
        CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
            .function(varchar_to_int)
            .register(con)
    }
}
```

## Implicit casts

Provide an `implicit_cost` to allow DuckDB to use the cast automatically in
expressions where the types do not match:

```rust,no_run
# use quack_rs::cast::CastFunctionBuilder;
# use quack_rs::types::TypeId;
# use libduckdb_sys::{duckdb_function_info, duckdb_vector, idx_t};
# unsafe extern "C" fn my_cast(_: duckdb_function_info, _: idx_t, _: duckdb_vector, _: duckdb_vector) -> bool { true }
# fn register(con: libduckdb_sys::duckdb_connection) -> Result<(), quack_rs::error::ExtensionError> {
unsafe {
    CastFunctionBuilder::new(TypeId::Varchar, TypeId::Integer)
        .function(my_cast)
        .implicit_cost(100) // lower = higher priority
        .register(con)
}
# }
```

## TRY_CAST vs CAST

Inside your callback, check [`CastFunctionInfo::cast_mode()`] to distinguish between
the two modes:

| Mode | User wrote | Expected behaviour on error |
|------|------------|-----------------------------|
| `CastMode::Normal` | `CAST(x AS T)` | Call `set_error` and return `false` |
| `CastMode::Try` | `TRY_CAST(x AS T)` | Call `set_row_error`, write `NULL`, continue |

## API reference

- [`CastFunctionBuilder`][quack_rs::cast::CastFunctionBuilder] — the main builder
- [`CastFunctionInfo`][quack_rs::cast::CastFunctionInfo] — info handle inside callbacks
- [`CastMode`][quack_rs::cast::CastMode] — `Normal` vs `Try` cast mode
