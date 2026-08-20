# Overloading with Function Sets

DuckDB supports multiple signatures for the same function name via **function sets**.
This is how you implement variadic aggregates like `retention(c1, c2, ..., c32)`.

> **Note**: For scalar function overloads, see [`ScalarFunctionSetBuilder`](scalar.md#overloading-with-function-sets).

---

## When to use function sets

Use `AggregateFunctionSetBuilder` when you need:
- Multiple type signatures for the same function name (e.g., `my_agg(INT)` and `my_agg(BIGINT)`)
- Variadic arity under one name (e.g., `retention(2 columns)`, `retention(3 columns)`, ...)

For a single signature, use `AggregateFunctionBuilder` directly.

---

## Registration

```rust,ignore
use quack_rs::aggregate::AggregateFunctionSetBuilder;
use quack_rs::types::TypeId;

unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        AggregateFunctionSetBuilder::new("retention")
            .returns(TypeId::Varchar)
            .overloads(2..=3, |n, builder| {
                // Each overload gets `n` BOOLEAN parameters
                let b = (0..n).fold(builder, |b, _| b.param(TypeId::Boolean));
                b.state_size(state_size)
                    .init(state_init)
                    .update(update)
                    .combine(combine)
                    .finalize(finalize)
                    .destructor(state_destroy)
            })
            .register(con)?;
    }
    Ok(())
}
```

The `overloads` method accepts a `RangeInclusive<usize>` and a closure that
receives the arity `n` and a fresh `OverloadBuilder`. The builder sets the
function name on each individual member internally.

---

## The silent name bug — solved

> **Pitfall L6**: When using a function set, the name must be set on **each individual
> `duckdb_aggregate_function`** via `duckdb_aggregate_function_set_name`, not just on the set.
> If any member lacks a name, it is **silently not registered** — no error is returned.
>
> This is completely undocumented. It was discovered by reading DuckDB's C++ test code at
> `test/api/capi/test_capi_aggregate_functions.cpp`. In `duckdb-behavioral`, 6 of 7 functions
> failed to register silently due to this bug.

`AggregateFunctionSetBuilder` enforces that each member has its name set internally
when the `overloads` closure builds each function.

See [Pitfall L6](../reference/pitfalls.md#l6-function-set-name-must-be-set-on-each-member).

---

## Complex return types

If all overloads share a complex return type, use `returns_logical` on the set builder:

```rust,ignore
use quack_rs::aggregate::AggregateFunctionSetBuilder;
use quack_rs::types::{LogicalType, TypeId};

AggregateFunctionSetBuilder::new("retention")
    .returns_logical(LogicalType::list(TypeId::Boolean))  // LIST(BOOLEAN) for all overloads
    .overloads(2..=32, |n, builder| {
        (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
            .state_size(state_size)
            .init(state_init)
            .update(update)
            .combine(combine)
            .finalize(finalize)
            .destructor(destroy)
    })
    .register(con)?;
```

Individual overloads can also use `param_logical` for complex parameter types:

```rust,ignore
.overloads(2..=8, |n, builder| {
    builder
        .param(TypeId::Interval)
        .param_logical(LogicalType::list(TypeId::Timestamp)) // LIST(TIMESTAMP) parameter
        // ...
})
```

---

## Why not varargs?

DuckDB's C API does not provide `duckdb_aggregate_function_set_varargs`. For true variadic
aggregates, you must register N overloads — one for each supported arity. Function sets make
this tractable.

> **Note**: As of DuckDB 1.5.0, **scalar** functions now support varargs directly via
> `ScalarFunctionBuilder::varargs()` (requires the `duckdb-1-5` feature). This limitation
> still applies to aggregate functions, which have no varargs counterpart in the C API.

ADR-002 in the architecture docs explains this design decision in detail.
