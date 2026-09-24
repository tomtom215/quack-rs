# Bound Expressions

> **Requires the `duckdb-1-5` feature flag** (DuckDB 1.5.0+).

`Expression` is an RAII wrapper around DuckDB's `duckdb_expression` handle. You
obtain one from a **scalar function's bind callback** via
[`ScalarBindInfo::argument`], which lets the bind phase inspect each argument's
static type and — when the argument is a constant — *fold* it to a concrete
[`Value`].

This is the canonical way to write scalar functions whose behaviour depends on a
constant argument (a format string, a precision, a regex) that should be
validated or pre-computed **once at bind time** rather than on every row.

## Folding a constant argument at bind time

```rust,no_run
use quack_rs::scalar::{RawScalarBindInfo, ScalarBindInfo};

unsafe extern "C" fn my_bind(info: RawScalarBindInfo) {
    let bind = unsafe { ScalarBindInfo::new(info) };

    if let Some(arg) = unsafe { bind.argument(0) } {
        // Inspect the argument's static return type.
        let _ty = arg.return_type();

        // If the argument is constant, evaluate it once here instead of
        // recomputing it for every row in the execute callback.
        if arg.is_foldable() {
            let ctx = unsafe { bind.get_client_context() };
            match arg.fold(&ctx) {
                Ok(value) => {
                    // Stash `value` as bind data for the execute phase.
                    let _ = value;
                }
                Err(err) => bind.set_error(&err.message().unwrap_or_default()),
            }
        }
    }
}
```

## API

| Method | Description |
|--------|-------------|
| `return_type()` | `Option<`[`LogicalType`]`>` — the expression's static type |
| `is_foldable()` | `true` if the expression is constant and can be `fold`ed |
| `fold(&client_context)` | `Result<`[`Value`]`, `[`ErrorData`]`>` — evaluate a constant expression |
| `from_raw(raw)` (unsafe) / `as_raw()` / `is_null()` | Handle inspection / escape hatches |

`fold` only succeeds when [`is_foldable`][is_foldable] returns `true`; otherwise it
returns a structured [`ErrorData`].

When evaluation itself fails, DuckDB 1.5.5 hands the C API the exception's JSON
form (`{"exception_type":"Conversion","exception_message":"...",...}`) and always
tags it `INVALID_INPUT`. `fold` unpacks it: `err.message()` is the plain message
and `err.error_type()` the type the exception named (`Conversion`, `OutOfRange`,
...). Text that is not that JSON is passed through unchanged.

## Obtaining an `Expression`

`ScalarBindInfo` (the wrapper around a scalar bind callback's `duckdb_bind_info`)
provides two accessors:

| Method | Returns |
|--------|---------|
| `argument(index)` (unsafe) | `Option<Expression>` — RAII, the ergonomic path |
| `get_argument(index)` (unsafe) | raw `duckdb_expression` — escape hatch |

Use `argument_count()` to bound the index.

## Ownership

`Expression` calls `duckdb_destroy_expression` on drop. The handle returned by
`argument()` is owned by the caller, so the wrapper cleans it up automatically.

## Related modules

- [Scalar Functions](../functions/scalar.md) — registering the function whose
  bind callback yields these expressions
- [Values & Parameter Extraction](../data/values-and-parameters.md) — working
  with the [`Value`] produced by `fold`
- [Structured Errors](error-data.md) — the [`ErrorData`] returned on failure

[`ScalarBindInfo::argument`]: https://docs.rs/quack-rs/latest/quack_rs/scalar/info/struct.ScalarBindInfo.html
[`Value`]: https://docs.rs/quack-rs/latest/quack_rs/value/struct.Value.html
[`LogicalType`]: https://docs.rs/quack-rs/latest/quack_rs/types/logical_type/struct.LogicalType.html
[`ErrorData`]: https://docs.rs/quack-rs/latest/quack_rs/error_data/struct.ErrorData.html
[is_foldable]: https://docs.rs/quack-rs/latest/quack_rs/expression/struct.Expression.html#method.is_foldable
