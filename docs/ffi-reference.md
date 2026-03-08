# DuckDB C API Reference for Extension Authors

This page documents the DuckDB C API surface used by `quack-rs`, including
correct callback signatures, memory ownership rules, and gotchas discovered
through production use. The [LESSONS.md](../LESSONS.md) companion file documents
each pitfall in detail; this page is the quick-reference.

## Table of Contents

- [Extension Entry Point](#extension-entry-point)
- [Scalar Function Registration](#scalar-function-registration)
- [Aggregate Function Registration](#aggregate-function-registration)
- [Aggregate Callback Signatures](#aggregate-callback-signatures)
- [Vector API](#vector-api)
- [String Format (duckdb_string_t)](#string-format-duckdb_string_t)
- [INTERVAL Layout](#interval-layout)
- [Type Constants](#type-constants)
- [Common Mistakes Quick Reference](#common-mistakes-quick-reference)

---

## Extension Entry Point

```rust
// The entry point function name must be: {extension_name}_init_c_api
// (all lowercase, underscores, no "lib" prefix)
#[no_mangle]
pub unsafe extern "C" fn my_ext_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    unsafe {
        quack_rs::entry_point::init_extension(
            info, access, quack_rs::DUCKDB_API_VERSION,
            |con| register_my_functions(con),
        )
    }
}
```

### Initialization sequence (inside `init_extension`)

```
duckdb_rs_extension_api_init(info, access, "v1.2.0")  ← C API version, NOT DuckDB version
  → fills the global AtomicPtr function table
(*access).get_database(info)
  → duckdb_database handle
duckdb_connect(db, &mut con)
  → duckdb_connection for registration
  [register functions]
duckdb_disconnect(&mut con)
```

**Critical**: The version string passed to `duckdb_rs_extension_api_init` is the
*C API version* (`"v1.2.0"` for DuckDB 1.4.x), not the DuckDB release version
(`"v1.4.4"`). See [Pitfall P2](../LESSONS.md).

---

## Scalar Function Registration

### Single function

```rust
unsafe {
    ScalarFunctionBuilder::new("double_it")
        .param(TypeId::BigInt)
        .returns(TypeId::BigInt)
        .function(double_it_fn)
        .register(con)?
}
```

### Function set (multiple overloads)

```rust
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
        .register(con)?
}
```

### NULL handling

```rust
ScalarFunctionBuilder::new("my_fn")
    .null_handling(NullHandling::SpecialNullHandling) // receive NULLs in callback
    // ...
```

---

## Aggregate Function Registration

### Single function

```rust
unsafe {
    AggregateFunctionBuilder::new("my_agg")
        .param(TypeId::Varchar)    // one call per input parameter
        .returns(TypeId::BigInt)
        .state_size(my_state_size)
        .init(my_state_init)
        .update(my_update)
        .combine(my_combine)
        .finalize(my_finalize)
        .destructor(my_destroy)    // required if state allocates heap memory
        .register(con)?
}
```

### Function set (multiple arities)

```rust
unsafe {
    AggregateFunctionSetBuilder::new("retention")
        .returns(TypeId::BigInt)
        .overloads(2..=32, |n, builder| {
            (0..n).fold(builder, |b, _| b.param(TypeId::Boolean))
                .state_size(state_size)
                .init(state_init)
                .update(update)
                .combine(combine)
                .finalize(finalize)
                .destructor(destroy)
        })
        .register(con)?
}
```

**Pitfall L6**: `duckdb_aggregate_function_set_name` must be called on each
individual function in the set, not just the set itself.
`AggregateFunctionSetBuilder` enforces this automatically.

---

## Aggregate Callback Signatures

The following are the correct Rust signatures for DuckDB aggregate callbacks.
Note that `update`, `combine`, and `finalize` take `*mut duckdb_aggregate_state`
(a pointer to an **array** of state pointers), **not** `duckdb_aggregate_state`.

```rust
// state_size: called once; returns sizeof(your FFI state struct)
unsafe extern "C" fn state_size(
    info: duckdb_function_info,
) -> idx_t;

// state_init: called once per group allocation
unsafe extern "C" fn state_init(
    info: duckdb_function_info,
    state: duckdb_aggregate_state,      // pointer to one state allocation
);

// update: called per input batch
unsafe extern "C" fn update(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state, // array: states[row] for each row in chunk
);

// combine: merge source array into target array (parallel merge phase)
unsafe extern "C" fn combine(
    info: duckdb_function_info,
    source: *mut duckdb_aggregate_state, // array of count source states
    target: *mut duckdb_aggregate_state, // array of count target states (zero-initialized!)
    count: idx_t,
);

// finalize: write results to output vector
unsafe extern "C" fn finalize(
    info: duckdb_function_info,
    source: *mut duckdb_aggregate_state, // array of count states
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,                       // write to result[offset..offset+count]
);

// destroy: free heap memory for each state
unsafe extern "C" fn destroy(
    states: *mut duckdb_aggregate_state, // array of count states
    count: idx_t,
);
```

`duckdb_aggregate_state` is defined as `*mut _duckdb_aggregate_state`, so
`*mut duckdb_aggregate_state` is a pointer-to-pointer (array of pointers).

---

## Vector API

### Reading input vectors (inside `update`)

```rust
// Get column i from the input chunk
let row_count = duckdb_data_chunk_get_size(chunk) as usize;
let vector = duckdb_data_chunk_get_vector(chunk, col_idx as idx_t);
let data = duckdb_vector_get_data(vector) as *const u8;
let validity = duckdb_vector_get_validity(vector);  // may be null (all valid)

// Check NULL
if !validity.is_null() && !duckdb_validity_row_is_valid(validity, row as idx_t) {
    // row is NULL
}

// Read typed values — use ptr::read_unaligned for safety
let val = ptr::read_unaligned(data.add(row * 8) as *const i64);

// Read booleans — always as u8, never cast to bool (Pitfall L5)
let b = *data.add(row) != 0;
```

### Writing output vectors (inside `finalize`)

```rust
let data = duckdb_vector_get_data(result) as *mut u8;

// Write a value
ptr::write_unaligned(data.add((offset + row) * 8) as *mut i64, value);

// Write NULL — must call ensure_validity_writable first (Pitfall L4)
duckdb_vector_ensure_validity_writable(result);
let validity = duckdb_vector_get_validity(result);
duckdb_validity_set_row_invalid(validity, (offset + row) as idx_t);
```

`VectorReader` and `VectorWriter` encapsulate these patterns and handle the
pitfalls automatically.

---

## String Format (duckdb_string_t)

DuckDB stores VARCHAR values as 16-byte `duckdb_string_t` structs with two layouts:

### Inline (length ≤ 12)

```
bytes 0..4   : length as u32 LE
bytes 4..16  : string data, zero-padded
```

### Pointer (length > 12)

```
bytes 0..4   : length as u32 LE
bytes 4..8   : prefix (first 4 bytes of string)
bytes 8..16  : pointer to heap-allocated string data (*const u8)
```

```rust
// Correct reading:
let bytes: [u8; 16] = ptr::read(data.add(row * 16) as *const [u8; 16]);
let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
let s: &str = if len <= 12 {
    std::str::from_utf8(&bytes[4..4 + len]).unwrap_or("")
} else {
    let ptr = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as *const u8;
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap_or("")
};
```

`DuckStringView` and `read_duck_string` handle this automatically.

---

## INTERVAL Layout

`duckdb_interval` is a 16-byte C struct:

```c
struct duckdb_interval {
    int32_t months;
    int32_t days;
    int64_t micros;
};
```

DuckDB uses a 30-day month approximation for arithmetic. The SDK provides
`interval_to_micros` (checked) and `interval_to_micros_saturating` for
conversion to microseconds.

**Note**: `months * 30 * 86_400_000_000 + days * 86_400_000_000 + micros`
can overflow `i64` for large values. Always use the checked variant or
handle the `None` case.

---

## Type Constants

`libduckdb-sys` exposes DuckDB type constants with the pattern
`DUCKDB_TYPE_DUCKDB_TYPE_*` (the double prefix is intentional):

| `TypeId` | `libduckdb-sys` constant |
|----------|--------------------------|
| `Boolean` | `DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN` |
| `TinyInt` | `DUCKDB_TYPE_DUCKDB_TYPE_TINYINT` |
| `SmallInt` | `DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT` |
| `Integer` | `DUCKDB_TYPE_DUCKDB_TYPE_INTEGER` |
| `BigInt` | `DUCKDB_TYPE_DUCKDB_TYPE_BIGINT` |
| `Float` | `DUCKDB_TYPE_DUCKDB_TYPE_FLOAT` |
| `Double` | `DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE` |
| `Varchar` | `DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR` |
| `Timestamp` | `DUCKDB_TYPE_DUCKDB_TYPE_TIMESTAMP` |
| `Interval` | `DUCKDB_TYPE_DUCKDB_TYPE_INTERVAL` |

`DUCKDB_TYPE` is a `u32` type alias, not an enum. Using `TypeId` avoids
dealing with these constants directly.

---

## Common Mistakes Quick Reference

| # | Symptom | Root Cause | Fix |
|---|---------|-----------|-----|
| L1 | Wrong results with parallel queries | `combine` doesn't copy config fields | Copy ALL fields in `combine` |
| L2 | Double-free / crash on destroy | `destroy` called twice, `inner` not nulled | Null `inner` after `Box::from_raw` |
| L4 | Segfault writing NULL | `get_validity` before `ensure_validity_writable` | Call `ensure_validity_writable` first |
| L5 | Wrong boolean values | Cast `*const u8` to `*const bool` | Use `*data.add(i) != 0` |
| L6 | Function silently not registered | Missing `set_name` on individual function in set | Use `AggregateFunctionSetBuilder` / `ScalarFunctionSetBuilder` |
| L7 | Memory leak | `duckdb_create_logical_type` without `destroy` | Use `LogicalType` RAII wrapper |
| P1 | Extension fails to load | `[lib] name` ≠ description.yml `name` | Match them exactly |
| P8 | Extension fails API init | Using DuckDB release version in `api_init` | Use C API version (`"v1.2.0"`) |

See [LESSONS.md](../LESSONS.md) for all 15 pitfalls with full analysis.
