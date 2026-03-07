# Overloading with Function Sets

DuckDB supports multiple signatures for the same function name via **function sets**.
This is how you implement variadic aggregates like `retention(c1, c2, ..., c32)`.

---

## When to use function sets

Use `AggregateFunctionSetBuilder` when you need:
- Multiple type signatures for the same function name (e.g., `my_agg(INT)` and `my_agg(BIGINT)`)
- Variadic arity under one name (e.g., `retention(2 columns)`, `retention(3 columns)`, ...)

For a single signature, use `AggregateFunctionBuilder` directly.

---

## Registration

```rust
use quack_rs::aggregate::{AggregateFunctionBuilder, AggregateFunctionSetBuilder};
use quack_rs::types::TypeId;

unsafe fn register(con: duckdb_connection) -> Result<(), ExtensionError> {
    unsafe {
        let mut set = AggregateFunctionSetBuilder::new("retention");

        // 2-column overload: retention(c1 BOOLEAN, c2 BOOLEAN)
        let f2 = AggregateFunctionBuilder::new("retention")  // name required on each member
            .param(TypeId::Boolean)
            .param(TypeId::Boolean)
            .returns(TypeId::Varchar)
            .state_size(state_size)
            .init(state_init)
            .update(update_2)
            .combine(combine)
            .finalize(finalize)
            .destructor(state_destroy)
            .build()?;
        set.add(f2)?;

        // 3-column overload: retention(c1 BOOLEAN, c2 BOOLEAN, c3 BOOLEAN)
        let f3 = AggregateFunctionBuilder::new("retention")  // same name
            .param(TypeId::Boolean)
            .param(TypeId::Boolean)
            .param(TypeId::Boolean)
            .returns(TypeId::Varchar)
            .state_size(state_size)
            .init(state_init)
            .update(update_3)
            .combine(combine)
            .finalize(finalize)
            .destructor(state_destroy)
            .build()?;
        set.add(f3)?;

        set.register(con)?;
    }
    Ok(())
}
```

---

## The silent name bug — solved

> **Pitfall L6**: When using a function set, the name must be set on **each individual
> `duckdb_aggregate_function`** via `duckdb_aggregate_function_set_name`, not just on the set.
> If any member lacks a name, it is **silently not registered** — no error is returned.
>
> This is completely undocumented. It was discovered by reading DuckDB's C++ test code at
> `test/api/capi/test_capi_aggregate_functions.cpp`. In `duckdb-behavioral`, 6 of 7 functions
> failed to register silently due to this bug.

`AggregateFunctionBuilder` sets the name on the individual function when you call `.build()`.
`AggregateFunctionSetBuilder` enforces that each member has a name before it can be added.

See [Pitfall L6](../reference/pitfalls.md#l6-function-set-name-must-be-set-on-each-member).

---

## Why not varargs?

DuckDB's C API does not provide `duckdb_aggregate_function_set_varargs`. For true variadic
aggregates, you must register N overloads — one for each supported arity. Function sets make
this tractable.

ADR-002 in the architecture docs explains this design decision in detail.
