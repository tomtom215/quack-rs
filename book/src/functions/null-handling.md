# NULL Handling

By default, DuckDB automatically propagates NULLs: if any argument to a function is
NULL, the result is NULL without your function callback being called. This matches the
SQL standard and works well for most functions.

However, some functions need to handle NULLs explicitly. For example:

- `COALESCE` — returns the first non-NULL argument
- `IS_NULL` / `IS_NOT_NULL` — tests whether the value is NULL
- Custom aggregates that need to count NULLs

---

## `NullHandling` enum

```rust
use quack_rs::types::NullHandling;

// Default: DuckDB auto-returns NULL for any NULL input
NullHandling::DefaultNullHandling

// Special: DuckDB passes NULLs to your callback
NullHandling::SpecialNullHandling
```

---

## Scalar functions

```rust
use quack_rs::scalar::ScalarFunctionBuilder;
use quack_rs::types::{TypeId, NullHandling};

ScalarFunctionBuilder::new("my_coalesce")
    .param(TypeId::BigInt)
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .null_handling(NullHandling::SpecialNullHandling)
    .function(my_coalesce_fn)
    .register(con)?;
```

With `SpecialNullHandling`, your callback must check `VectorReader::is_valid(row)` for
each input column and handle NULLs yourself.

---

## Aggregate functions

```rust
use quack_rs::aggregate::AggregateFunctionBuilder;
use quack_rs::types::{TypeId, NullHandling};

AggregateFunctionBuilder::new("count_with_nulls")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .null_handling(NullHandling::SpecialNullHandling)
    .state_size(my_state_size)
    .init(my_init)
    .update(my_update)   // will be called even for NULL rows
    .combine(my_combine)
    .finalize(my_finalize)
    .register(con)?;
```

---

## When to use special NULL handling

| Use case | NULL handling |
|----------|-------------|
| Most scalar/aggregate functions | `DefaultNullHandling` (the default) |
| Functions that need to see NULLs | `SpecialNullHandling` |
| `COALESCE`-like functions | `SpecialNullHandling` |
| NULL-counting aggregates | `SpecialNullHandling` |

If you don't call `.null_handling()`, the default (`DefaultNullHandling`) is used
automatically.
