# NULL Handling

> **The one thing to take away:** for a **scalar** function, `DefaultNullHandling`
> does *not* make DuckDB return NULL for you. Your callback is invoked for NULL
> rows too, and if it writes a value there, that value is the answer. Call
> `DataChunk::propagate_nulls` — or use `ScalarFunctionBuilder::map1` /
> `map2`, which do it for you.

---

## What DuckDB actually does

DuckDB's `FunctionNullHandling` has two settings, and quack-rs mirrors them as
[`NullHandling`]. The names suggest that the default makes the engine handle NULL
propagation. For **aggregate** functions that is true. For **scalar** functions
registered through the C API it is not, and the difference is silent wrong
answers rather than an error.

Two pieces of DuckDB source settle it (quoted from v1.5.4):

`src/main/capi/scalar_function-c.cpp` — the C API bridge calls your callback for
the whole flattened chunk and never looks at the result's validity:

```cpp
void CAPIScalarFunction(DataChunk &input, ExpressionState &state, Vector &result) {
    ...
    input.Flatten();
    ...
    c_bind_info.info.function(c_function_info, c_input, c_result);
    if (!function_info.success) {
        throw InvalidInputException(function_info.error);
    }
    ...
}
```

`src/execution/expression_executor/execute_function.cpp` — the only NULL check is
a debug-only assertion that your function *already* did the right thing:

```cpp
static void VerifyNullHandling(const BoundFunctionExpression &expr, DataChunk &args, Vector &result) {
#ifdef DEBUG
    if (args.data.empty() || expr.function.GetNullHandling() != FunctionNullHandling::DEFAULT_NULL_HANDLING) {
        return;
    }
    // ... D_ASSERT(!result_data.validity.RowIsValid(idx));
#endif
}
```

Every DuckDB a user installs is a release build, so that assertion is compiled
out.

### Why the obvious test passes anyway

```sql
SELECT my_func(NULL);   -- NULL, even for a broken function
```

A literal `NULL` is **constant-folded** during binding: DuckDB evaluates the
expression once, sees a NULL argument to a `DEFAULT_NULL_HANDLING` function, and
substitutes NULL without calling anything. The wrong answers only appear when the
argument comes from a column:

```sql
CREATE TABLE t(i BIGINT);
INSERT INTO t VALUES (1), (NULL), (3);
SELECT i, my_func(i) FROM t;
```

Against DuckDB 1.5.4, a function that unconditionally writes `999` returns:

| i | my_func(i) |
|---|-----------|
| 1 | 999 |
| NULL | **999** ← not NULL |
| 3 | 999 |

quack-rs pins this behaviour in `tests/ffi_roundtrip.rs`, so a future DuckDB that
starts propagating will show up as a test failure rather than as a surprise.

---

## Doing it right

### The safe route: typed scalar functions

`ScalarFunctionBuilder::map1` / `map2` take an ordinary Rust closure and handle
validity for you — a NULL argument short-circuits to a NULL result without ever
calling your code:

```rust
ScalarFunctionBuilder::map1("double_it", |x: i64| x * 2)?
    .register(con)?;
```

Use `map1_opt` / `map2_opt` when the function needs to *see* NULLs; those
register `SpecialNullHandling` for you and hand the closure `Option<T>`.

### The raw route: `propagate_nulls`

When you write the `extern "C"` callback yourself, restore SQL semantics with one
call at the end:

```rust
quack_rs::scalar_callback!(double_it, |_info, input, output| {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { chunk.reader(0) };
    let mut writer = unsafe { VectorWriter::from_vector(output) };
    for row in 0..chunk.size() {
        unsafe { writer.write_i64(row, reader.read_i64(row) * 2) };
    }
    // Without this, double_it(NULL) is 0, not NULL.
    unsafe { chunk.propagate_nulls(&mut writer) };
});
```

`propagate_nulls` resolves each column's validity pointer once and marks the
output NULL wherever any input column is NULL. A column with no validity mask has
no NULLs and costs nothing. `DataChunk::any_null(row)` is the per-row form when
you need the decision inline.

---

## `NullHandling` enum

```rust
use quack_rs::types::NullHandling;

// Default: the function promises NULL in -> NULL out.
// Scalar: you must keep that promise (see above).
// Aggregate: DuckDB enforces it by filtering NULL rows before `update`.
NullHandling::DefaultNullHandling

// The function means to see NULLs and may return non-NULL for them.
NullHandling::SpecialNullHandling
```

---

## Aggregate functions

Aggregates are the case where the default behaves as its name suggests: DuckDB's
aggregate executor filters NULL rows out before calling `update`. Opt out when
the aggregate needs to count or observe them:

```rust
use quack_rs::aggregate::AggregateFunctionBuilder;
use quack_rs::types::{TypeId, NullHandling};

AggregateFunctionBuilder::new("count_with_nulls")
    .param(TypeId::BigInt)
    .returns(TypeId::BigInt)
    .null_handling(NullHandling::SpecialNullHandling)
    .state_size(my_state_size)
    .init(my_init)
    .update(my_update)   // now called for NULL rows too
    .combine(my_combine)
    .finalize(my_finalize)
    .register(con)?;
```

---

## When to use special NULL handling

| Use case | NULL handling | Who propagates |
|----------|---------------|----------------|
| Scalar function, NULL in → NULL out | `DefaultNullHandling` | **you** (`propagate_nulls`, or `map1`/`map2`) |
| Scalar function that inspects NULLs (`COALESCE`-like, `IS_NULL`-like) | `SpecialNullHandling` | you |
| Aggregate, ignore NULL rows | `DefaultNullHandling` (the default) | DuckDB |
| Aggregate that counts NULLs | `SpecialNullHandling` | you |

If you don't call `.null_handling()`, `DefaultNullHandling` is used.
