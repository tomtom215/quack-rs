# Overloading with Function Sets

DuckDB supports multiple signatures for the same function name via **function sets**.
This is how you implement variadic aggregates like `retention(c1, c2, ..., c32)`.

> **Known DuckDB limitation.** C-API aggregates — including every overload in a
> set — read out of bounds under `agg(x) OVER ()` (whole-partition window frames)
> and `agg(x ORDER BY y)`. This is a DuckDB C API defect; see
> [Aggregate Functions](aggregate.md#known-duckdb-limitation) for the details and
> DuckDB source lines. Do not use C-API aggregates in those two query shapes.
>
> Reported upstream as [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109).

> **Note**: For scalar function overloads, see [`ScalarFunctionSetBuilder`](scalar.md#overloading-with-function-sets).

---

## When to use function sets

Use `AggregateFunctionSetBuilder` when you need:
- Multiple type signatures for the same function name (e.g., `my_agg(INT)` and `my_agg(BIGINT)`)
- Variadic arity under one name (e.g., `retention(2 columns)`, `retention(3 columns)`, ...)
- Overloads that return **different** types (see [Per-overload return types](#per-overload-return-types))

For a single signature, use `AggregateFunctionBuilder` directly.

---

## Registration

```rust
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
receives the arity `n` and a fresh `AggregateOverloadBuilder`. The builder sets
the function name on each individual member internally.

---

## Per-overload return types

DuckDB resolves an aggregate overload from its **parameter types and arity
alone** — the return type plays no part in resolution. Members of one set are
therefore free to return different types. This is how DuckDB's own `arg_max`
works:

```text
arg_max(ANY, ANY)      -> ANY
arg_max(ANY, ANY, ANY) -> ANY[]
```

Set the return type on the overload with `AggregateOverloadBuilder::returns` (or
`returns_logical`), and add each one with `overload`:

```rust
use quack_rs::aggregate::{AggregateFunctionSetBuilder, AggregateOverloadBuilder};
use quack_rs::types::TypeId;

AggregateFunctionSetBuilder::new("my_agg")
    .overload(
        AggregateOverloadBuilder::new()
            .param(TypeId::Integer)
            .returns(TypeId::Integer)      // my_agg(INTEGER) -> INTEGER
            .state_size(int_state_size)
            .init(int_init)
            .update(int_update)
            .combine(int_combine)
            .finalize(int_finalize),
    )
    .overload(
        AggregateOverloadBuilder::new()
            .param(TypeId::Varchar)
            .returns(TypeId::Varchar)      // my_agg(VARCHAR) -> VARCHAR
            .state_size(str_state_size)
            .init(str_init)
            .update(str_update)
            .combine(str_combine)
            .finalize(str_finalize),
    )
    .register(con)?;
```

Each overload carries its own callbacks, so overloads with different parameter
types can use different state types.

### Which return type wins

For each overload, in order:

1. `AggregateOverloadBuilder::returns_logical`
2. `AggregateOverloadBuilder::returns`
3. `AggregateFunctionSetBuilder::returns_logical` (the set-level default)
4. `AggregateFunctionSetBuilder::returns` (the set-level default)

Registration fails, naming the overload index, if an overload reaches the end of
that list with nothing set.

`overload` and `overloads` may be mixed on one builder; overloads register in
the order they were added.

> **Note**: `AggregateOverloadBuilder` was called `OverloadBuilder` before
> v0.18.0. The old name is still exported as a deprecated alias at
> `quack_rs::aggregate::builder::OverloadBuilder`.

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

If **all** overloads share one complex return type, set it once on the set
builder as a default, rather than repeating it on every overload:

```rust
use quack_rs::aggregate::AggregateFunctionSetBuilder;
use quack_rs::types::{LogicalType, TypeId};

AggregateFunctionSetBuilder::new("retention")
    .returns_logical(LogicalType::list(TypeId::Boolean))  // default for every overload
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

```rust
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

> **Note**: **Scalar** functions support varargs directly via
> `ScalarFunctionBuilder::varargs()` (stable C API, no feature flag needed). This limitation
> still applies to aggregate functions, which have no varargs counterpart in the C API.

ADR-002 in the architecture docs explains this design decision in detail.
